use crate::hierarchy::{parse_entire_hierarchy, parse_hierarchy};
use crate::octree::node::{Aabb, OctreeNode, OctreeNodeData};
use crate::resource::{Resource, ResourceClient, ResourceError};
use futures::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt};
use serde::Deserialize;
use std::io::{Read, SeekFrom};
use std::pin::Pin;
use thiserror::Error;

pub async fn parse_metadatas<C: ResourceClient>(resource: Resource<C>) -> Result<Metadata, ReadMetadataError> {
    Ok(resource.get_json(None).await?)
    // Ok(serde_json::from_reader(reader)?)
}

#[derive(Error, Debug)]
pub enum ReadMetadataError {
    #[error("Invalid json")]
    JsonError(#[from] serde_json::error::Error),

    #[error("Resource error")]
    ResourceError(#[from] ResourceError),
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub version: String,
    pub name: String,
    pub description: String,
    pub points: u64,
    pub projection: String,
    pub hierarchy: HierarchyMetadata,
    pub offset: [f32; 3],
    pub scale: [f32; 3],
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

#[derive(Error, Debug)]
pub enum ReadHierarchyError {
    #[error("Invalid json")]
    JsonError(#[from] serde_json::error::Error),

    #[error("IO Error")]
    Io(#[from] std::io::Error),

    #[error("Resource error")]
    Resource(#[from] ResourceError),

    #[error("Invalid binary data")]
    InvalidBinaryData(#[from] binrw::error::Error),
}

impl Metadata {
    #[inline]
    fn create_root_node(&self) -> OctreeNode {
        OctreeNode::new(OctreeNodeData {
            name: "r".to_string(),
            bounding_box: self.bounding_box.clone().into(),
            spacing: self.spacing,
            node_type: 2,
            hierarchy_byte_size: self.hierarchy.first_chunk_size,
            ..Default::default()
        })
    }

    pub async fn read_initial_hierarchy<C: ResourceClient>(
        &self,
        resource: &Resource<C>,
    ) -> Result<OctreeNode, ReadHierarchyError> {
        let root = self.create_root_node();

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

        Ok(root)
    }

    pub async fn read_hierarchy<C: ResourceClient>(
        &self,
        resource: Resource<C>,
    ) -> Result<OctreeNode, ReadHierarchyError> {
        let root = self.create_root_node();

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

        Ok(root)
    }

    pub async fn read_entire_hierarchy<C: ResourceClient>(
        &self,
        resource: &Resource<C>,
    ) -> Result<OctreeNode, ReadHierarchyError> {
        let root = self.create_root_node();

        parse_entire_hierarchy(root.clone(), resource).await?;

        Ok(root)
    }

    pub async fn read_initial_hierarchy_from_async_reader(
        &self,
        mut data: Pin<Box<impl AsyncSeek + AsyncRead>>,
    ) -> Result<OctreeNode, ReadHierarchyError> {
        let root = self.create_root_node();

        {
            let root_data = root.data();
            if root_data.node_type == 2 {
                let start = root_data.hierarchy_byte_offset;
                let mut buf = vec![0; root_data.hierarchy_byte_size as usize];

                data.seek(SeekFrom::Start(start)).await?;
                data.read_exact(&mut buf).await?;
                drop(root_data);

                parse_hierarchy(root.clone(), &buf)?;
            }
        }

        Ok(root)
    }



    // pub fn read_hierarchy(&self, mut read: impl Read) -> Result<OctreeNode, ReadHierarchyError> {
    //     let root = self.create_root_node();
    //
    //     let mut buffer = Vec::new();
    //     read.read_to_end(&mut buffer)?;
    //
    //     parse_entire_hierarchy(root.clone(), buffer.as_slice())?;
    //
    //     Ok(root)
    // }
}

impl Into<Aabb> for BoundingBox {
    fn into(self) -> Aabb {
        Aabb {
            min: self.min.into(),
            max: self.max.into(),
        }
    }
}
