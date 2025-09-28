use crate::octree::NodeId;
use super::aabb::Aabb;

#[derive(Clone, Debug, Default)]
pub struct OctreeNode {
    pub name: String,
    pub bounding_box: Aabb,
    pub spacing: f64,
    pub level: u32,
    pub node_type: u8,
    pub num_points: u32,
    pub byte_offset: u64,
    pub byte_size: u64,
    pub hierarchy_byte_offset: u64,
    pub hierarchy_byte_size: u64,

    // The node's id if known. None means not yet stored.
    pub(crate) id: Option<NodeId>,

    // The node's parent id. None means it's the root node.
    pub(crate) parent: Option<NodeId>,

    // Children node ids. A node does not always have 8 children.
    // If it's empty, it means either it's a leaf, or the children have not been loaded.
    pub(crate) children: Vec<NodeId>,
}
