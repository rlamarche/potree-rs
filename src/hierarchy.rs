use binrw::prelude::*;

#[binrw]
#[derive(Clone, Debug)]
#[br(little)]
pub struct HierarchyNodeEntry {
    pub r#type: u8,
    pub child_mask: u8,
    pub num_points: u32,
    pub byte_offset: u64,
    pub byte_size: u64,
}
