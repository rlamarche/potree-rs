pub mod aabb;
pub mod node;
pub mod snapshot;

pub mod point_attributes;

#[cfg(feature = "slab")]
use slab::Slab;

pub type NodeId = usize;

#[cfg(feature = "slab")]
#[derive(Debug)]
pub struct VacantEntry<'a, T> {
    vacant_entry: slab::VacantEntry<'a, T>,
    key: NodeId,
}

#[cfg(feature = "slab")]
impl<'a, T> VacantEntry<'a, T> {
    /// Insert a value in the entry, returning a mutable reference to the value.
    ///
    /// To get the key associated with the value, use `key` prior to calling
    /// `insert`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use slab::*;
    /// let mut slab = Slab::new();
    ///
    /// let hello = {
    ///     let entry = slab.vacant_entry();
    ///     let key = entry.key();
    ///
    ///     entry.insert((key, "hello"));
    ///     key
    /// };
    ///
    /// assert_eq!(hello, slab[hello].0);
    /// assert_eq!("hello", slab[hello].1);
    /// ```
    pub fn insert(self, val: T) -> &'a mut T {
        self.vacant_entry.insert(val)
    }

    /// Return the key associated with this entry.
    ///
    /// A value stored in this entry will be associated with this key.
    ///
    /// # Examples
    ///
    /// ```
    /// use slab::Slab;
    /// let mut slab = Slab::new();
    ///
    /// let hello = {
    ///     let entry = slab.vacant_entry();
    ///     let key = entry.key();
    ///
    ///     entry.insert((key, "hello"));
    ///     key
    /// };
    ///
    /// assert_eq!(hello, slab[hello].0);
    /// assert_eq!("hello", slab[hello].1);
    /// ```
    pub fn key(&self) -> NodeId {
        self.key
    }
}

#[cfg(feature = "slab")]
#[derive(Clone, Debug)]
pub struct Octree<T> {
    storage: Slab<T>,
    root_id: NodeId,
}

#[cfg(feature = "slab")]
impl<T> Default for Octree<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "slab")]
impl<T> Octree<T> {
    pub fn new() -> Self {
        let storage = Slab::new();

        Self {
            storage,
            root_id: 0,
        }
    }

    pub fn root(&self) -> Option<&T> {
        self.storage.get(self.root_id)
    }

    pub fn root_mut(&mut self) -> Option<&mut T> {
        self.storage.get_mut(self.root_id)
    }

    pub fn root_id(&self) -> NodeId {
        self.root_id
    }

    pub fn node(&self, node_id: NodeId) -> Option<&T> {
        self.storage.get(node_id)
    }

    pub fn node_mut(&mut self, node_id: NodeId) -> Option<&mut T> {
        self.storage.get_mut(node_id)
    }

    pub fn reserve(&mut self, additional: usize) {
        self.storage.reserve(additional);
    }

    pub fn insert(&mut self, node: T) -> NodeId {
        self.storage.insert(node)
    }

    pub fn vacant_entry(&mut self) -> VacantEntry<'_, T> {
        let vacant_entry = self.storage.vacant_entry();
        let index = vacant_entry.key();

        VacantEntry {
            vacant_entry,
            key: index,
        }
    }
}
