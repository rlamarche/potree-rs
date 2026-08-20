#[cfg(feature = "fs")]
use crate::asset::fs::PotreeFsAsset;
#[cfg(any(feature = "reqwest", feature = "ehttp"))]
use crate::asset::http::PotreeHttpAsset;
#[cfg(any(feature = "reqwest", feature = "ehttp", feature = "fs"))]
use crate::asset::url::PotreeUrlAsset;
#[cfg(feature = "async")]
use crate::asset::PotreeAsset;
#[cfg(feature = "async")]
use crate::metadata::Points;
use crate::metadata::{LoadPointsError, Metadata};
#[cfg(feature = "async")]
use crate::octree::node::{NodeType, OctreeNode};
#[cfg(feature = "async")]
use crate::parse::parse_flat_hierarchy;
use crate::parse::ParseHierarchyError;
#[cfg(feature = "async")]
use async_trait::async_trait;
use binrw::prelude::*;
#[cfg(feature = "async")]
use std::collections::VecDeque;
#[cfg(feature = "fs")]
use std::path::PathBuf;
use thiserror::Error;
#[cfg(feature = "async")]
use tracing::warn;

#[derive(Clone, Debug)]
pub struct Hierarchy<T> {
    #[allow(unused)]
    pub(crate) metadata: Metadata,
    #[allow(unused)]
    pub(crate) asset: T,
}

impl<T> Hierarchy<T> {
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }
}

#[cfg(any(feature = "reqwest", feature = "ehttp", feature = "fs"))]
impl Hierarchy<PotreeUrlAsset> {
    /// Load a Potree point cloud from a URL.
    /// Metadatas, hierarchy and octree are supposed to be accessible relatively to the provided url:
    ///  - Metadata: `<url>/metadata.json`
    ///  - Hierarchy: `<url>/hierarchy.bin`
    ///  - Octree: `<url>/octree.bin`
    pub async fn try_from_url(url: &str) -> Result<Self, PotreeHierarchyError> {
        let asset =
            PotreeUrlAsset::from_url(url).map_err(|e| PotreeHierarchyError::Read(Box::new(e)))?;

        Self::load(asset).await
    }
}

#[cfg(any(feature = "reqwest", feature = "ehttp"))]
impl Hierarchy<PotreeHttpAsset> {
    /// Load a Potree point cloud from a URL.
    /// Metadatas, hierarchy and octree are supposed to be accessible relatively to the provided url:
    ///  - Metadata: `<url>/metadata.json`
    ///  - Hierarchy: `<url>/hierarchy.bin`
    ///  - Octree: `<url>/octree.bin`
    #[cfg(any(feature = "reqwest", feature = "ehttp"))]
    pub async fn from_http_url(url: &str) -> Result<Self, PotreeHierarchyError> {
        let asset = PotreeHttpAsset::from_url(url);

        Self::load(asset).await
    }
}

#[cfg(feature = "fs")]
impl Hierarchy<PotreeFsAsset> {
    /// Load a Potree point cloud from a path.
    /// Metadatas, hierarchy and octree are supposed to be accessible relatively to the provided path:
    ///  - Metadata: `<path>/metadata.json`
    ///  - Hierarchy: `<path>/hierarchy.bin`
    ///  - Octree: `<path>/octree.bin`
    pub async fn from_path(path: impl Into<PathBuf>) -> Result<Self, PotreeHierarchyError> {
        let asset = PotreeFsAsset::from_path(path);

        Self::load(asset).await
    }
}

#[cfg(feature = "async")]
#[async_trait]
pub trait HierarchyAsync<T: PotreeAsset> {
    async fn load_initial_hierarchy(&self) -> Result<Vec<OctreeNode>, PotreeHierarchyError>;

    async fn load_hierarchy(
        &self,
        node: &OctreeNode,
    ) -> Result<Vec<OctreeNode>, PotreeHierarchyError>;

    async fn load_entire_hierarchy(&self) -> Result<Vec<OctreeNode>, PotreeHierarchyError>;

    async fn load_entire_hierarchy_from_proxy(
        &self,
        node: OctreeNode,
    ) -> Result<Vec<OctreeNode>, PotreeHierarchyError>;

    // Functions to load points
    async fn load_points(&self, node: &OctreeNode) -> Result<Points, PotreeHierarchyError>;
}

#[cfg(feature = "async")]
impl<T: PotreeAsset> Hierarchy<T> {
    /// Load a Potree point cloud from a URL.
    /// Relatives urls works only if the provided client supports it.
    /// Metadatas, hierarchy and octree are supposed to be accessible relatively to the provided url:
    ///  - Metadata: `<url>/metadata.json`
    ///  - Hierarchy: `<url>/hierarchy.bin`
    ///  - Octree: `<url>/octree.bin`
    pub async fn load(asset: T) -> Result<Self, PotreeHierarchyError> {
        let buffer = asset
            .read_metadata()
            .await
            .map_err(|e| PotreeHierarchyError::Read(Box::new(e)))?;

        let metadata = serde_json::from_slice(&buffer)?;

        Ok(Self { metadata, asset })
    }
}

#[cfg(feature = "async")]
#[async_trait]
impl<T: PotreeAsset> HierarchyAsync<T> for Hierarchy<T> {
    async fn load_initial_hierarchy(&self) -> Result<Vec<OctreeNode>, PotreeHierarchyError> {
        // load root node metadatas
        let root = self.metadata.create_flat_root_node();

        // load its hierarchy
        let nodes = self.load_hierarchy(&root).await?;

        Ok(nodes)
    }

    async fn load_hierarchy(
        &self,
        node: &OctreeNode,
    ) -> Result<Vec<OctreeNode>, PotreeHierarchyError> {
        if matches!(node.node_type, NodeType::Proxy) {
            let data = self
                .asset
                .read_hierarchy(
                    node.hierarchy_byte_offset,
                    node.hierarchy_byte_size as usize,
                )
                .await
                .map_err(|e| PotreeHierarchyError::Read(Box::new(e)))?;

            Ok(parse_flat_hierarchy(node, &data)?)
        } else {
            // this node is not a proxy, so its hierarchy can't be loaded
            Err(PotreeHierarchyError::NothingToLoad)
        }
    }

    async fn load_entire_hierarchy(&self) -> Result<Vec<OctreeNode>, PotreeHierarchyError> {
        let root = self.metadata.create_flat_root_node();

        Ok(self.load_entire_hierarchy_from_proxy(root).await?)
    }

    async fn load_entire_hierarchy_from_proxy(
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
                    let nodes = self.load_hierarchy(&node).await?;

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

    async fn load_points(&self, node: &OctreeNode) -> Result<Points, PotreeHierarchyError> {
        let buffer = self
            .asset
            .read_octree(node.byte_offset, node.byte_size as usize)
            .await
            .map_err(|e| PotreeHierarchyError::Read(Box::new(e)))?;

        let points = self
            .metadata
            .load_points(node.num_points, &node.bounding_box, &buffer)?;

        Ok(points)
    }
}

pub type ReadError = Box<dyn std::error::Error + Send + Sync + 'static>;

#[derive(Debug, Error)]
pub enum PotreeHierarchyError {
    #[error("parse metadata error: {0}")]
    ParseMetadata(#[from] serde_json::Error),

    #[error("parse hierarchy error: {0}")]
    ParseHierarchy(#[from] ParseHierarchyError),

    #[error("load points error: {0}")]
    ParsePoints(#[from] LoadPointsError),

    #[error("read error: {0}")]
    Read(#[source] ReadError),

    #[error("There is nothing to load because node is not a proxy")]
    NothingToLoad,
}
