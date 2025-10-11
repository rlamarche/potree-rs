use crate::octree::aabb::Aabb;
use crate::octree::node::OctreeNode;
use serde::Deserialize;

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub version: String,
    pub name: String,
    pub description: String,
    pub points: u64,
    pub projection: String,
    pub hierarchy: HierarchyMetadata,
    pub offset: [f64; 3],
    pub scale: [f64; 3],
    pub spacing: f64,
    pub bounding_box: BoundingBox,
    pub encoding: String,
    pub attributes: Vec<AttributeMetadata>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HierarchyMetadata {
    pub first_chunk_size: u64,
    pub step_size: u16,
    pub depth: u16,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BoundingBox {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

#[derive(Deserialize, Clone, Debug)]
pub enum AttributeType {
    #[serde(rename = "int8")]
    Int8,
    #[serde(rename = "int16")]
    Int16,
    #[serde(rename = "int32")]
    Int32,
    #[serde(rename = "int64")]
    Int64,
    #[serde(rename = "uint8")]
    UInt8,
    #[serde(rename = "uint16")]
    UInt16,
    #[serde(rename = "uint32")]
    UInt32,
    #[serde(rename = "uint64")]
    UInt64,
    #[serde(rename = "float")]
    Float,
    #[serde(rename = "double")]
    Double,
    #[serde(rename = "undefined")]
    Undefined,
}
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AttributeMetadata {
    pub name: String,
    pub description: String,
    pub size: u16,
    pub num_elements: u16,
    pub element_size: u16,
    pub r#type: AttributeType,
    pub min: Vec<f32>,
    pub max: Vec<f32>,
}

impl Metadata {

    pub(crate) fn create_root_node(&self) -> OctreeNode {
        OctreeNode {
            name: "r".to_string(),
            bounding_box: self.bounding_box.clone().into(),
            spacing: self.spacing,
            node_type: 2,
            hierarchy_byte_size: self.hierarchy.first_chunk_size,
            ..Default::default()
        }
    }
}

impl Into<Aabb> for BoundingBox {
    fn into(self) -> Aabb {
        Aabb {
            min: self.min.into(),
            max: self.max.into(),
        }
    }
}
