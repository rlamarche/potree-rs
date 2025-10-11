use crate::hierarchy::HierarchyNodeEntry;
use crate::metadata::Metadata;
use crate::octree::aabb::create_child_aabb;
use crate::octree::node::OctreeNode;
use crate::octree::snapshot::OctreeNodeSnapshot;
use crate::octree::{FlatOctree, NodeId};
use crate::point::PointData;
use crate::resource::{ResourceError, ResourceLoader};
use binrw::BinReaderExt;
use byteorder::{ByteOrder, LittleEndian};
use glam::{DVec3, U8Vec3, U16Vec3, UVec3, Vec3A};
use std::io::{Cursor, Read};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LoadPotreePointCloudError {
    #[error("Error loading metadatas: {0}")]
    LoadMetadataError(ResourceError),

    #[error("Error loading hierarchy: {0}")]
    ReadHierarchyError(#[from] ReadHierarchyError),

    #[error("Error loading resource: {0}")]
    ResourceError(#[from] ResourceError),
}

#[derive(Error, Debug)]
pub enum ReadHierarchyError {
    #[error("Hierarchy is already loaded")]
    AlreadyLoaded,

    #[error("Invalid json: {0}")]
    JsonError(#[from] serde_json::error::Error),

    #[error("IO Error")]
    Io(#[from] std::io::Error),

    #[error("Resource error: {0}")]
    Resource(#[from] ResourceError),

    #[error("Invalid binary data")]
    InvalidBinaryData(#[from] binrw::error::Error),
}

#[derive(Error, Debug)]
pub enum LoadPointsError {
    #[error("Node does not exists")]
    NodeNotFound,

    #[error("Resource error: {0}")]
    Resource(#[from] ResourceError),

    #[error("Encoding not implemented: {0}")]
    EncodingUnimplemented(String),

    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Debug)]
pub struct PotreePointCloud {
    metadata: Metadata,
    hierarchy_url: String,
    octree_url: String,
    octree: FlatOctree<OctreeNode>,
    resource_loader: ResourceLoader,
}

impl PotreePointCloud {
    /// Load a Potree point cloud from a URL.
    /// Relatives urls works only if the provided client supports it.
    /// Metadatas, hierarchy and octree are supposed to be accessible relatively to the provided url:
    ///  - Metadata: `<url>/metadata.json`
    ///  - Hierarchy: `<url>/hierarchy.bin`
    ///  - Octree: `<url>/octree.bin`
    pub async fn from_url(
        url: &str,
        resource_loader: ResourceLoader,
    ) -> Result<PotreePointCloud, LoadPotreePointCloudError> {
        let octree = FlatOctree::new();

        let metadata_url = format!("{}/metadata.json", url).to_string();
        let hierarchy_url = format!("{}/hierarchy.bin", url).to_string();
        let metadata = resource_loader
            .get_json(&metadata_url, None)
            .await
            .map_err(|error| LoadPotreePointCloudError::ResourceError(error))?;

        let mut this = Self {
            metadata,
            hierarchy_url,
            octree_url: format!("{}/octree.bin", url).to_string(),
            octree,
            resource_loader,
        };

        this.load_initial_hierarchy().await?;

        Ok(this)
    }

    async fn load_initial_hierarchy(&mut self) -> Result<(), ReadHierarchyError> {
        let root_id = self.octree.root_id();
        // get the root node
        let root = self.octree.root_mut();

        // load root node metadatas
        *root = self.metadata.create_root_node();

        // set its id
        root.id = Some(root_id);

        // load its hierarchy
        self.load_hierarchy(root_id).await?;

        Ok(())
    }

    pub async fn load_hierarchy(&mut self, node_id: NodeId) -> Result<(), ReadHierarchyError> {
        // get the root node
        let node = self.octree.node(node_id).unwrap();

        if node.node_type == 2 {
            let data = self
                .resource_loader
                .get_range(
                    &self.hierarchy_url,
                    node.hierarchy_byte_offset,
                    node.hierarchy_byte_size as usize,
                    None,
                )
                .await?;

            self.parse_hierarchy(node_id, &data)?;
        }

        Ok(())
    }

    pub async fn load_entire_hierarchy(&mut self) -> Result<(), ReadHierarchyError> {
        // get the root node
        let root = self.octree.root();
        let children = root.children.clone();

        for child in children {
            self.parse_entire_hierarchy(child).await?;
        }

        Ok(())
    }

    async fn parse_entire_hierarchy(&mut self, node_id: NodeId) -> Result<(), ReadHierarchyError> {
        // load the node's hierarchy
        self.load_hierarchy(node_id).await?;

        // get the node's children
        let node = self
            .octree
            .node(node_id)
            .expect("parse_entire_hierarchy: invalid node_id");
        let children = node.children.clone();

        // load childrens hierarchy
        for child in children {
            Box::pin(self.parse_entire_hierarchy(child)).await?;
        }

        Ok(())
    }

    fn parse_hierarchy(&mut self, node_id: NodeId, buf: &[u8]) -> binrw::BinResult<()> {
        const BYTES_PER_NODE: usize = 22;
        let mut cursor = Cursor::new(buf);
        let num_nodes = buf.len() / BYTES_PER_NODE;

        // reserve additional nodes
        self.octree.reserve(num_nodes - 1);
        // allocate additional nodes
        let nodes = vec![OctreeNode::default(); num_nodes - 1];
        // store all node ids
        let mut node_ids = Vec::with_capacity(num_nodes);
        node_ids.push(node_id);

        // insert these nodes
        for node in nodes {
            let node_id = self.octree.insert(node);
            self.octree.node_mut(node_id).unwrap().id = Some(node_id);
            node_ids.push(node_id);
        }

        // position of the next node to write in
        let mut node_pos = 1;

        // the first node is always the root of the (sub-)hierarchy we are loading
        for i in 0..num_nodes {
            let current_id = node_ids[i];
            let current = self.octree.node_mut(current_id).unwrap();

            let header: HierarchyNodeEntry = cursor.read_le()?;

            if current.node_type == 2 {
                current.byte_offset = header.byte_offset;
                current.byte_size = header.byte_size;
                current.num_points = header.num_points;
            } else if header.r#type == 2 {
                current.hierarchy_byte_offset = header.byte_offset;
                current.hierarchy_byte_size = header.byte_size;
                current.num_points = header.num_points;
            } else {
                current.byte_offset = header.byte_offset;
                current.byte_size = header.byte_size;
                current.num_points = header.num_points;
            }

            if current.byte_size == 0 {
                // workaround for issue https://github.com/potree/potree/issues/1125
                // some inner nodes erroneously report >0 points even though have 0 points
                // however, they still report a ByteSize of 0, so based on that we now set node.NumPoints to 0
                current.num_points = 0;
            }

            current.node_type = header.r#type;

            if current.node_type == 2 {
                continue;
            }

            let mut children = Vec::with_capacity(8);

            // clone/copy just what we need
            let (current_name, current_bounding_box, current_spacing, current_level) = (
                current.name.clone(),
                current.bounding_box.clone(),
                current.spacing,
                current.level,
            );

            for child_index in 0..8 {
                let child_exists = ((1 << child_index) & header.child_mask) != 0;
                if !child_exists {
                    continue;
                }

                // get the next child id
                let child_id = node_ids[node_pos];

                // get mutable access to the pre-allocated child
                let child = self.octree.node_mut(child_id).unwrap();
                child.name.clear();
                child
                    .name
                    .push_str(&format!("{}{}", current_name, child_index));
                child.bounding_box = create_child_aabb(&current_bounding_box, child_index);
                child.spacing = current_spacing / 2.0;
                child.level = current_level + 1;
                child.parent = Some(current_id);

                children.push(child_id);

                // increment node_pos for the next child
                node_pos += 1;
            }

            // finally, append the children to the parent
            let current = self.octree.node_mut(current_id).unwrap();
            current.children = children;
        }

        Ok(())
    }

    /// Takes a snapshot of the current loaded hierarchy and return it
    pub fn hierarchy_snapshot(&self) -> Vec<OctreeNodeSnapshot> {
        self.hierarchy_snaphot_from_node(self.octree.root())
    }

    fn hierarchy_snaphot_from_node(&self, node: &OctreeNode) -> Vec<OctreeNodeSnapshot> {
        let mut stack = vec![(0_usize, node)];
        let mut nodes = Vec::new();

        while let Some((parent_index, node)) = stack.pop() {
            // get the current node future index
            let current_index = nodes.len();

            // process children
            for child in &node.children {
                let child = self
                    .octree
                    .node(*child)
                    .expect("missing node in hierarchy, shouldn't happen");
                stack.push((current_index, child));
            }

            // add the current node to the nodes array
            let mut node_snapshot: OctreeNodeSnapshot = node.into();
            node_snapshot.index = current_index;
            nodes.push(node_snapshot);

            // if there is a parent, add it to the children array on an empty space
            if parent_index < current_index {
                let parent_node = &mut nodes[parent_index];
                *parent_node
                    .children
                    .iter_mut()
                    .find(|child| **child == 0)
                    .expect("no empty child space available, there might be a problem") =
                    current_index;
            }
        }

        nodes
    }

    // Functions to load points
    pub async fn load_points(&self, node_id: NodeId) -> Result<Vec<PointData>, LoadPointsError> {
        let node = self
            .octree
            .node(node_id)
            .ok_or(LoadPointsError::NodeNotFound)?;

        self.load_points_for_node(node).await
    }

    pub async fn load_points_for_node(
        &self,
        node: &OctreeNode,
    ) -> Result<Vec<PointData>, LoadPointsError> {
        let buffer = self
            .resource_loader
            .get_range(
                &self.octree_url,
                node.byte_offset,
                node.byte_size as usize,
                None,
            )
            .await?;

        let points = match self.metadata.encoding.as_str() {
            "BROTLI" => self.parse_points_brotli(node, &buffer)?,
            _ => {
                return Err(LoadPointsError::EncodingUnimplemented(
                    self.metadata.encoding.clone(),
                ));
            }
        };

        Ok(points)
    }

    fn parse_points_brotli(
        &self,
        node: &OctreeNode,
        buffer: &[u8],
    ) -> Result<Vec<PointData>, LoadPointsError> {
        let mut cursor = Cursor::new(buffer);
        let mut input = brotli_decompressor::Decompressor::new(&mut cursor, 4096);
        let mut decompressed_buffer = Vec::new();
        let size = input.read_to_end(&mut decompressed_buffer)?;

        let mut byte_offset: usize = 0;

        let mut points = vec![PointData::default(); node.num_points as usize];

        for point_attribute in &self.metadata.attributes {
            let point_data = PointData::default();
            points.push(point_data);

            match point_attribute.name.as_str() {
                "POSITION_CARTESIAN" | "position" => {
                    let scale = &self.metadata.scale;
                    let offset = &self.metadata.offset;

                    for j in 0..node.num_points {
                        let bytes = &decompressed_buffer[byte_offset..byte_offset + 16];
                        let (x, y, z) = read_morton_128(bytes);

                        points[j as usize].position = node.bounding_box.min
                            + DVec3::new(
                                x as f64 * scale[0] + offset[0] - node.bounding_box.min.x,
                                y as f64 * scale[1] + offset[1] - node.bounding_box.min.y,
                                z as f64 * scale[2] + offset[2] - node.bounding_box.min.z,
                            );

                        byte_offset += 16;
                    }
                }
                "RGBA" | "rgba" | "RGB" | "rgb" => {
                    for j in 0..node.num_points {
                        let bytes = &decompressed_buffer[byte_offset..byte_offset + 8];
                        let (r, g, b) = read_morton_64(bytes);

                        points[j as usize].color = U8Vec3::new(
                            if r > 255 { r / 256 } else { r } as u8,
                            if g > 255 { g / 256 } else { g } as u8,
                            if b > 255 { b / 256 } else { b } as u8,
                        );

                        byte_offset += 8;
                    }
                }
                _ => {
                    for j in 0..node.num_points {
                        let bytes = &decompressed_buffer
                            [byte_offset..byte_offset + point_attribute.size as usize];

                        byte_offset += point_attribute.size as usize;
                    }
                }
            }
        }

        // println!("Final offset: {}, size: {}", byte_offset, size);

        Ok(points)
    }

    // Functions to access the octree
    pub fn octree(&self) -> &FlatOctree<OctreeNode> {
        &self.octree
    }
}

fn read_morton_64(bytes: &[u8]) -> (u16, u16, u16) {
    let mc_0 = LittleEndian::read_u32(&bytes[4..8]);
    let mc_1 = LittleEndian::read_u32(&bytes[0..4]);

    decode_morton_64(mc_0, mc_1)
}

fn read_morton_128(bytes: &[u8]) -> (u32, u32, u32) {
    let mc_0 = LittleEndian::read_u32(&bytes[4..8]);
    let mc_1 = LittleEndian::read_u32(&bytes[0..4]);
    let mc_2 = LittleEndian::read_u32(&bytes[12..16]);
    let mc_3 = LittleEndian::read_u32(&bytes[8..12]);

    decode_morton_128(mc_0, mc_1, mc_2, mc_3)
}

fn dealign_24b(mut morton: u32) -> u32 {
    // Garde seulement chaque 3ème bit
    morton &= 0x09249249; // 0b001001001001001001001001001001

    morton = (morton | (morton >> 2)) & 0x030c30c3;
    morton = (morton | (morton >> 4)) & 0x0300f00f;
    morton = (morton | (morton >> 8)) & 0x030000ff;
    morton = (morton | (morton >> 16)) & 0x000003ff;

    morton
}

fn decode_morton_64(mc_0: u32, mc_1: u32) -> (u16, u16, u16) {
    let r = dealign_24b((mc_1 & 0x00FFFFFF) >> 0)
        | (dealign_24b(((mc_1 >> 24) | (mc_0 << 8)) >> 0) << 8);

    let g = dealign_24b((mc_1 & 0x00FFFFFF) >> 1)
        | (dealign_24b(((mc_1 >> 24) | (mc_0 << 8)) >> 1) << 8);

    let b = dealign_24b((mc_1 & 0x00FFFFFF) >> 2)
        | (dealign_24b(((mc_1 >> 24) | (mc_0 << 8)) >> 2) << 8);

    (r as u16, g as u16, b as u16)
}

fn decode_morton_128(mc_0: u32, mc_1: u32, mc_2: u32, mc_3: u32) -> (u32, u32, u32) {
    // First part (lower bits)
    let mut x = dealign_24b((mc_3 & 0x00FFFFFF) >> 0)
        | (dealign_24b(((mc_3 >> 24) | (mc_2 << 8)) >> 0) << 8);

    let mut y = dealign_24b((mc_3 & 0x00FFFFFF) >> 1)
        | (dealign_24b(((mc_3 >> 24) | (mc_2 << 8)) >> 1) << 8);

    let mut z = dealign_24b((mc_3 & 0x00FFFFFF) >> 2)
        | (dealign_24b(((mc_3 >> 24) | (mc_2 << 8)) >> 2) << 8);

    // Second part (upper bits) - only if needed
    if mc_1 != 0 || mc_2 != 0 {
        x |= (dealign_24b((mc_1 & 0x00FFFFFF) >> 0) << 16)
            | (dealign_24b(((mc_1 >> 24) | (mc_0 << 8)) >> 0) << 24);

        y |= (dealign_24b((mc_1 & 0x00FFFFFF) >> 1) << 16)
            | (dealign_24b(((mc_1 >> 24) | (mc_0 << 8)) >> 1) << 24);

        z |= (dealign_24b((mc_1 & 0x00FFFFFF) >> 2) << 16)
            | (dealign_24b(((mc_1 >> 24) | (mc_0 << 8)) >> 2) << 24);
    }

    (x, y, z)
}
