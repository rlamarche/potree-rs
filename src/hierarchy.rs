use crate::octree::node::{Aabb, OctreeNode, OctreeNodeData, OctreeNodeInner, ParentOctreeNode};
use crate::{Metadata, ReadHierarchyError};
use bytes::Bytes;
use std::io::Read;
use thiserror::Error;
use tracing::{span, Level};


use binrw::{binrw, BinRead, BinReaderExt};
use std::cell::RefCell;
use std::io::Cursor;
use std::sync::{Arc, Weak};
use glam::Vec3;
use std::io::Seek;
use futures::FutureExt;
use crate::resource::{Resource, ResourceClient};

#[binrw]
#[derive(Debug, Clone)]
#[br(little)]
pub struct HierarchyNodeEntry {
    pub r#type: u8,
    pub child_mask: u8,
    pub num_points: u32,
    pub byte_offset: u64,
    pub byte_size: u64,
}


// #[derive(Debug)]
// struct OctreeNode {
//     name: String,
//     bounding_box: AABB,
//     spacing: f64,
//     level: u32,
//     parent: Option<Weak<RefCell<OctreeNode>>>,
//     children: Vec<Arc<RefCell<OctreeNode>>>,
//
//     node_type: u8,
//     num_points: u32,
//     byte_offset: u64,
//     byte_size: u64,
//     hierarchy_byte_offset: u64,
//     hierarchy_byte_size: u64,
// }
//
// impl OctreeNode {
//     fn new_root(name: String, bounding_box: AABB, spacing: f64) -> Arc<RefCell<Self>> {
//         Arc::new(RefCell::new(Self {
//             name,
//             bounding_box,
//             spacing,
//             level: 0,
//             parent: None,
//             children: Vec::new(),
//             node_type: 0,
//             num_points: 0,
//             byte_offset: 0,
//             byte_size: 0,
//             hierarchy_byte_offset: 0,
//             hierarchy_byte_size: 0,
//         }))
//     }
// }

fn create_child_aabb(aabb: &Aabb, index: usize) -> Aabb {
    let mut min = aabb.min;
    let mut max = aabb.max;
    let size = (max - min) * 0.5;

    if (index & 0b0001) > 0 { min.z += size.z; } else { max.z -= size.z; }
    if (index & 0b0010) > 0 { min.y += size.y; } else { max.y -= size.y; }
    if (index & 0b0100) > 0 { min.x += size.x; } else { max.x -= size.x; }

    Aabb::new(min, max)
}

pub fn parse_hierarchy(node: OctreeNode, buf: &[u8]) -> binrw::BinResult<()> {
    const BYTES_PER_NODE: usize = 22;
    let mut cursor = Cursor::new(buf);
    let num_nodes = buf.len() / BYTES_PER_NODE;

    let mut nodes = vec![OctreeNode::default(); num_nodes];

    // store the root node at index 0
    nodes[0] = node;
    let mut node_pos = 1;

    for i in 0..num_nodes {

        let (left, right) = nodes.split_at_mut(i + 1);
        let current = &left[i];

        let header: HierarchyNodeEntry = cursor.read_le()?;

        let mut current_mut = current.borrow_mut();

        if current_mut.data.node_type == 2 {
            current_mut.data.byte_offset = header.byte_offset;
            current_mut.data.byte_size = header.byte_size;
            current_mut.data.num_points = header.num_points;
        } else if header.r#type == 2 {
            current_mut.data.hierarchy_byte_offset = header.byte_offset;
            current_mut.data.hierarchy_byte_size = header.byte_size;
            current_mut.data.num_points = header.num_points;
        } else {
            current_mut.data.byte_offset = header.byte_offset;
            current_mut.data.byte_size = header.byte_size;
            current_mut.data.num_points = header.num_points;
        }

        if current_mut.data.byte_size == 0 {
            // workaround for issue https://github.com/potree/potree/issues/1125
            // some inner nodes erroneously report >0 points even though have 0 points
            // however, they still report a ByteSize of 0, so based on that we now set node.NumPoints to 0
            current_mut.data.num_points = 0;
        }

        current_mut.data.node_type = header.r#type;

        if current_mut.data.node_type == 2 {
            continue;
        }

        for child_index in 0..8 {
            let child_exists = ((1 << child_index) & header.child_mask) != 0;
            if !child_exists {
                continue;
            }

            let child = OctreeNode::with_parent(OctreeNodeData {
                name: format!("{}{}", current_mut.data.name, child_index),
                bounding_box: create_child_aabb(&current_mut.data.bounding_box, child_index),
                spacing: current_mut.data.spacing / 2.0,
                level: current_mut.data.level + 1,
                ..Default::default()
            }, current.into());

            current_mut.children.push(child.clone());

            right[node_pos - (i + 1)] = child;
            node_pos += 1;
        }
    }

    Ok(())
}

pub fn parse_entire_hierarchy_from_buf(root: OctreeNode, buf: &[u8]) -> binrw::BinResult<()> {
    {
        let root_data = root.data();
        if root_data.node_type == 2 {
            let start = root_data.hierarchy_byte_offset as usize;
            let end = start + root_data.hierarchy_byte_size as usize;
            let scoped = &buf[start..end];
            drop(root_data);
            parse_hierarchy(root.clone(), scoped)?;
        }
    }

    for child in root.children().clone() {
        parse_entire_hierarchy_from_buf(child, buf)?;
    }

    Ok(())
}

pub async fn parse_entire_hierarchy<C: ResourceClient>(root: OctreeNode, resource: &Resource<C>) -> Result<(), ReadHierarchyError> {
    {
        let root_data = root.data();
        if root_data.node_type == 2 {
            let data = resource
                .get_range(
                    root_data.hierarchy_byte_offset,
                    root_data.hierarchy_byte_size as usize,
                    None,
                )
                .await?;
            drop(root_data);
            parse_hierarchy(root.clone(), &data)?;
        }
    }

    for child in root.children().clone() {
        Box::pin(parse_entire_hierarchy(child, resource)).await?;
    }

    Ok(())
}