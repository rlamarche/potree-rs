pub mod node;
pub mod aabb;
pub mod snapshot;

pub mod point_attributes;

use std::fmt::Display;

use slab::Slab;

#[derive(Clone, Debug, Copy, Default)]
pub struct NodeId(pub(crate) usize);

impl Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Debug)]
pub struct FlatOctree<T> {
    storage: Slab<T>,
    root_id: NodeId,
}
impl<T> FlatOctree<T>
where
    T: Default,
{
    pub fn root(&self) -> &T {
        self.storage
            .get(self.root_id.0)
            .expect("root node not found - invariant broken")
    }

    pub fn root_mut(&mut self) -> &mut T {
        self.storage
            .get_mut(self.root_id.0)
            .expect("root node not found - invariant broken")
    }

    pub fn root_id(&self) -> NodeId {
        self.root_id
    }

    pub fn node(&self, node_id: NodeId) -> Option<&T> {
        self.storage.get(node_id.0)
    }

    pub fn node_mut(&mut self, node_id: NodeId) -> Option<&mut T> {
        self.storage.get_mut(node_id.0)
    }

    pub fn reserve(&mut self, additional: usize) {
        self.storage.reserve(additional);
    }

    pub fn insert(&mut self, node: T) -> NodeId {
        NodeId(self.storage.insert(node))
    }
}

impl<T> FlatOctree<T>
where
    T: Default,
{
    pub fn new() -> Self {
        let mut storage = Slab::new();

        let root_node = T::default();
        let root_id = NodeId(storage.insert(root_node));

        Self { storage, root_id }
    }
}
