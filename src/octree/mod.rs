pub mod node;
pub mod aabb;
pub mod snapshot;

pub mod point_attributes;

use slab::Slab;

#[derive(Clone, Debug, Copy, Default)]
pub struct NodeId(pub(crate) usize);

#[derive(Clone, Debug)]
pub struct FlatOctree<T> {
    storage: Slab<T>,
    root_id: NodeId,
}
impl<T> FlatOctree<T>
where
    T: Default,
{
    pub(crate) fn root(&self) -> &T {
        self.storage
            .get(self.root_id.0)
            .expect("root node not found - invariant broken")
    }

    pub(crate) fn root_mut(&mut self) -> &mut T {
        self.storage
            .get_mut(self.root_id.0)
            .expect("root node not found - invariant broken")
    }

    pub(crate) fn root_id(&self) -> NodeId {
        self.root_id
    }

    pub(crate) fn node(&self, node_id: NodeId) -> Option<&T> {
        self.storage.get(node_id.0)
    }

    pub(crate) fn node_mut(&mut self, node_id: NodeId) -> Option<&mut T> {
        self.storage.get_mut(node_id.0)
    }

    pub(crate) fn reserve(&mut self, additional: usize) {
        self.storage.reserve(additional);
    }

    pub(crate) fn insert(&mut self, node: T) -> NodeId {
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
