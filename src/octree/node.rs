use crate::{AttributeMetadata, BoundingBox};
use glam::{DVec3, Vec3};
use std::cell::{Ref, RefCell, RefMut};
use std::ops::Deref;
use std::sync::{Arc, Weak};

#[derive(Debug)]
pub struct OctreeGeometry {
    pub url: String,
    pub spacing: f32,
    pub bounding_box: BoundingBox,
    pub root: OctreeNodeInner,
    pub point_attributes: Vec<AttributeMetadata>,
}

#[derive(Clone, Debug, Default)]
pub struct Aabb {
    pub min: DVec3,
    pub max: DVec3,
}

impl Aabb {
    pub fn new(min: DVec3, max: DVec3) -> Self {
        Self { min, max }
    }
}

#[derive(Clone, Debug, Default)]
pub struct OctreeNodeData {
    // pub id: usize,
    pub name: String,
    // pub index: usize,
    // pub octree_geometry: Box<OctreeGeometry>,
    pub bounding_box: Aabb,
    pub spacing: f64,
    pub level: u32,
    pub node_type: u8,
    pub num_points: u32,
    pub byte_offset: u64,
    pub byte_size: u64,
    pub hierarchy_byte_offset: u64,
    pub hierarchy_byte_size: u64,
}

#[derive(Clone, Debug, Default)]
pub struct OctreeNodeInner {
    pub data: OctreeNodeData,
    pub parent: Option<ParentOctreeNode>,
    pub children: Vec<OctreeNode>,
}

#[derive(Clone, Debug, Default)]
pub struct OctreeNode(Arc<RefCell<OctreeNodeInner>>);

#[derive(Clone, Debug, Default)]
pub struct ParentOctreeNode(Weak<RefCell<OctreeNodeInner>>);


impl OctreeNode {
    /// Constructor
    pub fn new(data: OctreeNodeData) -> Self {
        Self(Arc::new(RefCell::new(OctreeNodeInner {
            data,
            parent: None,
            children: Vec::new(),
        })))
    }
    pub fn with_parent(data: OctreeNodeData, parent: ParentOctreeNode) -> Self {
        Self(Arc::new(RefCell::new(OctreeNodeInner {
            data,
            parent: Some(parent),
            children: Vec::new(),
        })))
    }

    pub fn borrow(&self) -> Ref<'_, OctreeNodeInner> {
        self.0.borrow()
    }

    pub fn borrow_mut(&self) -> RefMut<'_, OctreeNodeInner> {
        self.0.borrow_mut()
    }

    /// Immutable access to the payload
    pub fn data(&self) -> Ref<'_, OctreeNodeData> {
        Ref::map(self.0.borrow(), |inner| &inner.data)
    }

    /// Mutable access to the payload
    pub fn data_mut(&self) -> RefMut<'_, OctreeNodeData> {
        RefMut::map(self.0.borrow_mut(), |inner| &mut inner.data)
    }

    /// Add a child and update both parent/child links
    pub fn add_child(&self, child: OctreeNode) {
        {
            let mut inner = self.0.borrow_mut();
            inner.children.push(child.clone());
        }
        child.0.borrow_mut().parent = Some(ParentOctreeNode(Arc::downgrade(&self.0)));
    }

    /// Return all children (cloned handles)
    pub fn children(&self) -> Vec<OctreeNode> {
        self.0.borrow().children.clone()
    }

    /// Return the parent, if it exists
    pub fn parent(&self) -> Option<OctreeNode> {
        self.0
            .borrow()
            .parent
            .as_ref()
            .and_then(|w| w.0.upgrade())
            .map(OctreeNode)
    }
}

impl Into<ParentOctreeNode> for &OctreeNode {
    fn into(self) -> ParentOctreeNode {
        ParentOctreeNode(Arc::downgrade(&self.0))
    }
}