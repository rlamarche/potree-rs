#[cfg(feature = "blocking_reqwest")]
use crate::blocking::asset::http::BlockingPotreeHttpAsset;
use crate::blocking::asset::url::BlockingPotreeUrlAsset;
use crate::blocking::asset::BlockingPotreeAsset;
use crate::hierarchy::{Hierarchy, PotreeHierarchyError};
use crate::metadata::Points;
use crate::octree::node::{NodeType, OctreeNode};
use crate::parse::parse_flat_hierarchy;
use binrw::prelude::*;
use std::collections::VecDeque;
use tracing::warn;

pub trait HierarchySync<T: BlockingPotreeAsset> {
    fn load_initial_hierarchy(&self) -> Result<Vec<OctreeNode>, PotreeHierarchyError>;

    fn load_hierarchy(&self, node: &OctreeNode) -> Result<Vec<OctreeNode>, PotreeHierarchyError>;

    fn load_entire_hierarchy(&self) -> Result<Vec<OctreeNode>, PotreeHierarchyError>;

    fn load_entire_hierarchy_from_proxy(
        &self,
        node: OctreeNode,
    ) -> Result<Vec<OctreeNode>, PotreeHierarchyError>;

    // Functions to load points
    fn load_points(&self, node: &OctreeNode) -> Result<Points, PotreeHierarchyError>;
}

impl<T: BlockingPotreeAsset> Hierarchy<T> {
    pub fn load_blocking(asset: T) -> Result<Self, T::Error> {
        let metadata = asset.read_metadata()?;

        Ok(Self { metadata, asset })
    }
}

impl Hierarchy<BlockingPotreeUrlAsset> {
    /// Load a Potree point cloud from a URL.
    /// Metadatas, hierarchy and octree are supposed to be accessible relatively to the provided url:
    ///  - Metadata: `<url>/metadata.json`
    ///  - Hierarchy: `<url>/hierarchy.bin`
    ///  - Octree: `<url>/octree.bin`
    pub async fn try_from_url_blocking(
        url: &str,
    ) -> Result<Self, <BlockingPotreeUrlAsset as BlockingPotreeAsset>::Error> {
        let asset = BlockingPotreeUrlAsset::from_url(url)?;

        Self::load_blocking(asset)
    }
}

#[cfg(feature = "blocking_reqwest")]
impl Hierarchy<BlockingPotreeHttpAsset> {
    /// Load a Potree point cloud from a URL in a blocking way.
    /// Metadatas, hierarchy and octree are supposed to be accessible relatively to the provided url:
    ///  - Metadata: `<url>/metadata.json`
    ///  - Hierarchy: `<url>/hierarchy.bin`
    ///  - Octree: `<url>/octree.bin`
    #[cfg(feature = "blocking_reqwest")]
    pub async fn from_http_url_blocking(
        url: &str,
    ) -> Result<Self, <BlockingPotreeHttpAsset as BlockingPotreeAsset>::Error> {
        let asset = BlockingPotreeHttpAsset::from_url(url);

        Self::load_blocking(asset)
    }
}

impl<T: BlockingPotreeAsset> HierarchySync<T> for Hierarchy<T> {
    fn load_initial_hierarchy(&self) -> Result<Vec<OctreeNode>, PotreeHierarchyError> {
        // load root node metadatas
        let root = self.metadata.create_flat_root_node();

        // load its hierarchy
        let nodes = self.load_hierarchy(&root)?;

        Ok(nodes)
    }

    fn load_hierarchy(&self, node: &OctreeNode) -> Result<Vec<OctreeNode>, PotreeHierarchyError> {
        if matches!(node.node_type, NodeType::Proxy) {
            let data = self
                .asset
                .read_hierarchy(
                    node.hierarchy_byte_offset,
                    node.hierarchy_byte_size as usize,
                )
                .map_err(|e| PotreeHierarchyError::Read(Box::new(e)))?;

            Ok(parse_flat_hierarchy(node, &data)?)
        } else {
            // this node is not a proxy, so its hierarchy can't be loaded
            Err(PotreeHierarchyError::NothingToLoad)
        }
    }

    fn load_entire_hierarchy(&self) -> Result<Vec<OctreeNode>, PotreeHierarchyError> {
        let root = self.metadata.create_flat_root_node();

        self.load_entire_hierarchy_from_proxy(root)
    }

    fn load_entire_hierarchy_from_proxy(
        &self,
        node: OctreeNode,
    ) -> Result<Vec<OctreeNode>, PotreeHierarchyError> {
        if !matches!(node.node_type, NodeType::Proxy) {
            // node is not a proxy
            return Err(PotreeHierarchyError::NothingToLoad);
        }

        // initialize a stack
        let mut stack = VecDeque::new();
        stack.push_back(node);

        // initialize the output nodes
        let mut nodes: Vec<OctreeNode> = Vec::new();

        // for each node in stack
        while let Some(node) = stack.pop_front() {
            let next_index = nodes.len();
            let stack_offset = stack.len();

            let node = match node.node_type {
                // if it's proxy, load it and its hierarchy, and all new nodes to the stack
                NodeType::Proxy => {
                    // keep node's parent
                    let first_node_parent = node.parent;
                    let nodes = self.load_hierarchy(&node)?;

                    // assign correct parent index
                    let Some(node) = ({
                        let mut first_node = None;
                        for (i, mut node) in nodes.into_iter().enumerate() {
                            if i == 0 {
                                // reassign initial node's parent because it hasn't changed
                                node.parent = first_node_parent;
                                first_node = Some(node);
                            } else {
                                // update node's parent index
                                match node.parent {
                                    Some(0) => {
                                        node.parent = Some(next_index);
                                    }
                                    Some(parent @ 1..) => {
                                        node.parent = Some(parent + next_index + stack_offset);
                                    }
                                    None => {}
                                }
                                stack.push_back(node);
                            }
                        }
                        first_node
                    }) else {
                        warn!("A proxy node has missing hierarchy, should not happen.");
                        continue;
                    };
                    node
                }
                // else, just process this node
                _ => node,
            };

            // update child index in parent's children
            if let Some(parent) = node.parent {
                nodes[parent].children[node.child_index as usize] = next_index;
            }

            // add the node to the list of nodes
            nodes.push(node);
        }

        Ok(nodes)
    }

    // Functions to load points
    fn load_points(&self, node: &OctreeNode) -> Result<Points, PotreeHierarchyError> {
        let buffer = self
            .asset
            .read_octree(node.byte_offset, node.byte_size as usize)
            .map_err(|e| PotreeHierarchyError::Read(Box::new(e)))?;

        let points = self
            .metadata
            .load_points(node.num_points, &node.bounding_box, &buffer)?;

        Ok(points)
    }
}
