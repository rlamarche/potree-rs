#[cfg(feature = "fs")]
use crate::asset::fs::PotreeFsAsset;
#[cfg(any(feature = "reqwest", feature = "ehttp"))]
use crate::asset::http::PotreeHttpAsset;
use crate::asset::url::PotreeUrlAsset;
use crate::asset::PotreeAsset;
use crate::hierarchy::{Hierarchy, HierarchyAsync, PotreeHierarchyError};
use crate::metadata::Points;
use crate::octree::node::{iter_one_bits, NodeType, OctreeNode};
use crate::octree::snapshot::OctreeNodeSnapshot;
use crate::octree::{NodeId, Octree};
use async_trait::async_trait;
use binrw::prelude::*;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PotreePointCloudError<ReadError: std::error::Error> {
    #[error("Error loading hierarchy: {0}")]
    Hierarchy(#[from] PotreeHierarchyError<ReadError>),

    #[error("Node not found: {0}")]
    NodeNotFound(NodeId),
}

#[derive(Clone, Debug)]
pub struct PointCloud<T> {
    pub(crate) hierarchy: Hierarchy<T>,
    pub(crate) octree: Octree<OctreeNode>,
}

#[async_trait]
pub trait PointCloudAsync<T: PotreeAsset> {
    async fn load_initial_hierarchy(&mut self) -> Result<(), PotreeHierarchyError<T::Error>>;

    async fn load_hierarchy(
        &mut self,
        node_id: NodeId,
    ) -> Result<(), PotreePointCloudError<T::Error>>;

    async fn load_entire_hierarchy(&mut self) -> Result<(), PotreePointCloudError<T::Error>>;

    async fn load_entire_hierarchy_recursive(
        &mut self,
        node_id: NodeId,
    ) -> Result<(), PotreePointCloudError<T::Error>>;

    // Functions to load points
    async fn load_points(&self, node_id: NodeId)
        -> Result<Points, PotreePointCloudError<T::Error>>;
}

#[async_trait]
impl<T: PotreeAsset> PointCloudAsync<T> for PointCloud<T> {
    async fn load_initial_hierarchy(&mut self) -> Result<(), PotreeHierarchyError<T::Error>> {
        // load root node metadatas
        let initial_hierarchy = self.hierarchy.load_initial_hierarchy().await?;

        // this vec will store each inserted nodes
        let mut parents = Vec::with_capacity(initial_hierarchy.len());
        for mut node in initial_hierarchy {
            let mut parent_id = None;
            if let Some(parent) = node.parent {
                parent_id = Some(parents[parent]);
            }

            // insert the new node
            let vacant_entry = self.octree.vacant_entry();
            let current_id = vacant_entry.key();
            parents.push(current_id);

            let child_index = node.child_index;

            node.parent = parent_id;
            node.children = [0; 8];
            vacant_entry.insert(node);

            // update parent children / children_mask
            if let Some(parent_id) = &parent_id {
                // infallible because it was inserted upper
                let parent = self.octree.node_mut(*parent_id).unwrap();

                // add to children array and update mask
                parent.children[child_index as usize] = current_id;
                parent.children_mask |= 1u8 << child_index;
            }
        }

        Ok(())
    }

    async fn load_hierarchy(
        &mut self,
        node_id: NodeId,
    ) -> Result<(), PotreePointCloudError<T::Error>> {
        // get the root node
        let node = self
            .octree
            .node(node_id)
            .ok_or_else(|| PotreePointCloudError::NodeNotFound(node_id))?;

        if matches!(node.node_type, NodeType::Proxy) {
            let nodes = self.hierarchy.load_hierarchy(node).await?;

            // this vec will store each inserted nodes
            let mut parents = Vec::with_capacity(nodes.len());

            // insert each new hierarchy node in the asset
            for mut node in nodes {
                if let Some(parent) = node.parent {
                    // get the corresponding parent's node id from the indexed vec
                    let parent_id = Some(parents[parent]);

                    // insert the new node
                    let vacant_entry = self.octree.vacant_entry();
                    let current_id = vacant_entry.key();
                    parents.push(current_id);

                    let child_index = node.child_index;

                    node.parent = parent_id;
                    node.children = [0; 8];
                    vacant_entry.insert(node);

                    // update parent children / children_mask
                    if let Some(parent_id) = &parent_id {
                        // infallible because it was inserted upper
                        let parent = self.octree.node_mut(*parent_id).unwrap();

                        // add to children array and update mask
                        parent.children[child_index as usize] = current_id;
                        parent.children_mask |= 1u8 << child_index;
                    }
                } else {
                    parents.push(node_id);
                    // this is the first node, it exists already, just update it
                    let existing_node = self
                        .octree
                        .node_mut(node_id)
                        .ok_or_else(|| PotreePointCloudError::NodeNotFound(node_id))?;
                    existing_node.node_type = node.node_type;
                    existing_node.num_points = node.num_points;
                    existing_node.byte_offset = node.byte_offset;
                    existing_node.byte_size = node.byte_size;
                    existing_node.hierarchy_byte_offset = node.hierarchy_byte_offset;
                    existing_node.hierarchy_byte_size = node.hierarchy_byte_size;
                }
            }
        }

        Ok(())
    }

    async fn load_entire_hierarchy(&mut self) -> Result<(), PotreePointCloudError<T::Error>> {
        // get the root node
        let root = self
            .octree
            .root()
            .ok_or_else(|| PotreePointCloudError::NodeNotFound(self.octree.root_id()))?;
        let children = root.children;

        for i in iter_one_bits(root.children_mask) {
            let child = children[i as usize];
            self.load_entire_hierarchy_recursive(child).await?;
        }

        Ok(())
    }

    async fn load_entire_hierarchy_recursive(
        &mut self,
        node_id: NodeId,
    ) -> Result<(), PotreePointCloudError<T::Error>> {
        // load node's hierarchy if needed
        self.load_hierarchy(node_id).await?;

        // get the node
        let node = self
            .octree
            .node(node_id)
            .ok_or_else(|| PotreePointCloudError::NodeNotFound(node_id))?;

        let children = node.children;

        for i in iter_one_bits(node.children_mask) {
            let child = children[i as usize];
            Box::pin(self.load_entire_hierarchy_recursive(child)).await?;
        }

        Ok(())
    }

    // Functions to load points
    async fn load_points(
        &self,
        node_id: NodeId,
    ) -> Result<Points, PotreePointCloudError<T::Error>> {
        let node = self
            .octree
            .node(node_id)
            .ok_or(PotreePointCloudError::NodeNotFound(node_id))?;

        Ok(self.hierarchy.load_points(node).await?)
    }
}

impl PointCloud<PotreeUrlAsset> {
    /// Load a Potree point cloud from a URL.
    /// Relatives urls works only if the provided client supports it.
    /// Metadatas, hierarchy and octree are supposed to be accessible relatively to the provided url:
    ///  - Metadata: `<url>/metadata.json`
    ///  - Hierarchy: `<url>/hierarchy.bin`
    ///  - Octree: `<url>/octree.bin`
    pub async fn from_url(
        url: &str,
    ) -> Result<
        PointCloud<PotreeUrlAsset>,
        PotreeHierarchyError<<PotreeUrlAsset as PotreeAsset>::Error>,
    > {
        let hierarchy = Hierarchy::try_from_url(url)
            .await
            .map_err(PotreeHierarchyError::Read)?;
        let octree = Octree::new();

        let mut this = Self { hierarchy, octree };

        this.load_initial_hierarchy().await?;

        Ok(this)
    }
}

#[cfg(any(feature = "reqwest", feature = "ehttp"))]
impl PointCloud<PotreeHttpAsset> {
    /// Load a Potree point cloud from a URL.
    /// Relatives urls works only if the provided client supports it.
    /// Metadatas, hierarchy and octree are supposed to be accessible relatively to the provided url:
    ///  - Metadata: `<url>/metadata.json`
    ///  - Hierarchy: `<url>/hierarchy.bin`
    ///  - Octree: `<url>/octree.bin`
    pub async fn from_http_url(
        url: &str,
    ) -> Result<
        PointCloud<PotreeHttpAsset>,
        PotreeHierarchyError<<PotreeHttpAsset as PotreeAsset>::Error>,
    > {
        let hierarchy = Hierarchy::from_http_url(url)
            .await
            .map_err(PotreeHierarchyError::Read)?;
        let octree = Octree::new();

        let mut this = Self { hierarchy, octree };

        this.load_initial_hierarchy().await?;

        Ok(this)
    }
}

#[cfg(feature = "fs")]
impl PointCloud<PotreeFsAsset> {
    /// Load a Potree point cloud from a URL.
    /// Relatives urls works only if the provided client supports it.
    /// Metadatas, hierarchy and octree are supposed to be accessible relatively to the provided url:
    ///  - Metadata: `<url>/metadata.json`
    ///  - Hierarchy: `<url>/hierarchy.bin`
    ///  - Octree: `<url>/octree.bin`
    pub async fn from_path(
        url: &str,
    ) -> Result<
        PointCloud<PotreeFsAsset>,
        PotreeHierarchyError<<PotreeFsAsset as PotreeAsset>::Error>,
    > {
        let hierarchy = Hierarchy::from_path(url)
            .await
            .map_err(PotreeHierarchyError::Read)?;
        let octree = Octree::new();

        let mut this = Self { hierarchy, octree };

        this.load_initial_hierarchy().await?;

        Ok(this)
    }
}

#[cfg(feature = "convert")]
impl PointCloud<crate::convert::buffer::PotreeBufferAsset> {
    /// Build a point cloud from in-memory Potree buffers (e.g. produced by the
    /// converter), without touching the filesystem.
    pub async fn from_buffers(
        metadata_json: Vec<u8>,
        hierarchy_bin: Vec<u8>,
        octree_bin: Vec<u8>,
    ) -> Result<
        PointCloud<crate::convert::buffer::PotreeBufferAsset>,
        PotreeHierarchyError<<crate::convert::buffer::PotreeBufferAsset as PotreeAsset>::Error>,
    > {
        let asset =
            crate::convert::buffer::PotreeBufferAsset::new(metadata_json, hierarchy_bin, octree_bin);
        let hierarchy = Hierarchy::new(asset)
            .await
            .map_err(PotreeHierarchyError::Read)?;
        let octree = Octree::new();

        let mut this = Self { hierarchy, octree };

        this.load_initial_hierarchy().await?;

        Ok(this)
    }
}

impl<T> PointCloud<T> {
    /// Takes a snapshot of the current loaded hierarchy and return it
    pub fn hierarchy_snapshot(&self) -> Vec<OctreeNodeSnapshot> {
        let Some(root) = self.octree.root() else {
            return Vec::new();
        };
        self.hierarchy_snaphot_from_node(root)
    }

    fn hierarchy_snaphot_from_node(&self, node: &OctreeNode) -> Vec<OctreeNodeSnapshot> {
        let mut stack = vec![(0_usize, node)];
        let mut nodes = Vec::new();

        while let Some((parent_index, node)) = stack.pop() {
            // get the current node future index
            let current_index = nodes.len();

            // process children
            for i in iter_one_bits(node.children_mask) {
                let child = &node.children[i as usize];

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

    // Functions to access the octree
    pub fn octree(&self) -> &Octree<OctreeNode> {
        &self.octree
    }
}
