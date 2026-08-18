#[cfg(feature = "blocking_reqwest")]
use crate::blocking::asset::http::BlockingPotreeHttpAsset;
use crate::blocking::asset::BlockingPotreeAsset;
use crate::blocking::hierarchy::HierarchySync;
#[cfg(feature = "blocking_reqwest")]
use crate::hierarchy::Hierarchy;
use crate::hierarchy::PotreeHierarchyError;
use crate::metadata::Points;
use crate::octree::node::{iter_one_bits, NodeType};
use crate::octree::NodeId;
#[cfg(feature = "blocking_reqwest")]
use crate::octree::Octree;
use crate::prelude::{PointCloud, PotreePointCloudError};
use binrw::prelude::*;

pub trait PointCloudSync<T: BlockingPotreeAsset> {
    fn load_initial_hierarchy(&mut self) -> Result<(), PotreeHierarchyError>;

    fn load_hierarchy(&mut self, node_id: NodeId) -> Result<(), PotreePointCloudError>;

    fn load_entire_hierarchy(&mut self) -> Result<(), PotreePointCloudError>;

    fn load_entire_hierarchy_recursive(
        &mut self,
        node_id: NodeId,
    ) -> Result<(), PotreePointCloudError>;

    // Functions to load points
    fn load_points(&self, node_id: NodeId) -> Result<Points, PotreePointCloudError>;
}

impl<T: BlockingPotreeAsset> PointCloudSync<T> for PointCloud<T> {
    fn load_initial_hierarchy(&mut self) -> Result<(), PotreeHierarchyError> {
        // load root node metadatas
        let initial_hierarchy = self.hierarchy.load_initial_hierarchy()?;

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

    fn load_hierarchy(&mut self, node_id: NodeId) -> Result<(), PotreePointCloudError> {
        // get the root node
        let node = self
            .octree
            .node(node_id)
            .ok_or_else(|| PotreePointCloudError::NodeNotFound(node_id))?;

        if matches!(node.node_type, NodeType::Proxy) {
            let nodes = self.hierarchy.load_hierarchy(node)?;

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

    fn load_entire_hierarchy(&mut self) -> Result<(), PotreePointCloudError> {
        // get the root node
        let root = self
            .octree
            .root()
            .ok_or_else(|| PotreePointCloudError::NodeNotFound(self.octree.root_id()))?;
        let children = root.children;

        for i in iter_one_bits(root.children_mask) {
            let child = children[i as usize];
            self.load_entire_hierarchy_recursive(child)?;
        }

        Ok(())
    }

    fn load_entire_hierarchy_recursive(
        &mut self,
        node_id: NodeId,
    ) -> Result<(), PotreePointCloudError> {
        // load node's hierarchy if needed
        self.load_hierarchy(node_id)?;

        // get the node
        let node = self
            .octree
            .node(node_id)
            .ok_or_else(|| PotreePointCloudError::NodeNotFound(node_id))?;

        let children = node.children;

        for i in iter_one_bits(node.children_mask) {
            let child = children[i as usize];
            self.load_entire_hierarchy_recursive(child)?;
        }

        Ok(())
    }

    // Functions to load points
    fn load_points(&self, node_id: NodeId) -> Result<Points, PotreePointCloudError> {
        let node = self
            .octree
            .node(node_id)
            .ok_or(PotreePointCloudError::NodeNotFound(node_id))?;

        Ok(self.hierarchy.load_points(node)?)
    }
}

#[cfg(feature = "blocking_reqwest")]
impl PointCloud<BlockingPotreeHttpAsset> {
    /// Load a Potree point cloud from a URL.
    /// Metadatas, hierarchy and octree are supposed to be accessible relatively to the provided url:
    ///  - Metadata: `<url>/metadata.json`
    ///  - Hierarchy: `<url>/hierarchy.bin`
    ///  - Octree: `<url>/octree.bin`
    pub async fn from_url(
        url: &str,
    ) -> Result<PointCloud<BlockingPotreeHttpAsset>, PotreeHierarchyError> {
        let hierarchy = Hierarchy::from_http_url_blocking(url)
            .await
            .map_err(|e| PotreeHierarchyError::Read(Box::new(e)))?;
        let octree = Octree::new();

        let mut this = Self { hierarchy, octree };

        this.load_initial_hierarchy()?;

        Ok(this)
    }
}
