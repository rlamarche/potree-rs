use crate::octree::aabb::Aabb;

#[derive(Clone, Debug, Default)]
pub struct OctreeNodeSnapshot {
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
    pub children: Vec<OctreeNodeSnapshot>,
}

pub struct SnapshotIter<'a> {
    stack: Vec<&'a OctreeNodeSnapshot>,
}

impl<'a> Iterator for SnapshotIter<'a> {
    type Item = &'a OctreeNodeSnapshot;

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.stack.pop()?;
        for child in node.children.iter().rev() {
            self.stack.push(child);
        }
        Some(node)
    }
}

impl OctreeNodeSnapshot {
    pub fn iter(&self) -> SnapshotIter<'_> {
        SnapshotIter { stack: vec![self] }
    }
}
