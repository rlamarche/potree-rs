use crate::morton::{read_morton_128, read_morton_64};
use crate::octree::aabb::Aabb;
use crate::octree::node::{NodeType, OctreeNode};
use crate::point::{AttributeInfo, AttributeType, PointBuffer};
use byteorder::{ByteOrder, LittleEndian};
use glam::{UVec3, Vec3};
use serde::Deserialize;
use std::io::{Cursor, Read};
use std::ops::Sub;
use thiserror::Error;

const GRID_SIZE: f32 = 32.0;
const GRID_SIZE_UINT: u32 = GRID_SIZE as u32;
const GRID_SIZE_SPLAT: UVec3 = UVec3::splat(GRID_SIZE as u32 - 1);

#[derive(Clone, Debug, Default)]
pub struct Points {
    pub buffer: PointBuffer,
    pub density: u32,
}

#[derive(Error, Debug)]
pub enum LoadPointsError {
    #[error("Encoding not implemented: {0}")]
    EncodingUnimplemented(String),

    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),
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
    /// Stored as f64: georeferenced clouds (e.g. UTM coordinates ~10^5 m) lose
    /// decimeter-level precision in f32. Values are only narrowed to f32 at the
    /// render boundary (`Aabb`, `PointBuffer`), after node-relative math.
    pub offset: [f64; 3],
    pub scale: [f64; 3],
    pub spacing: f32,
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
pub enum AttributeFormat {
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

impl AttributeFormat {
    pub fn parse(&self, bytes: &[u8]) -> f64 {
        match self {
            AttributeFormat::Int8 => bytes[0] as i8 as f64,

            AttributeFormat::UInt8 => bytes[0] as f64,

            AttributeFormat::Int16 => {
                let v = i16::from_le_bytes(bytes[0..2].try_into().unwrap());
                v as f64
            }

            AttributeFormat::UInt16 => {
                let v = u16::from_le_bytes(bytes[0..2].try_into().unwrap());
                v as f64
            }

            AttributeFormat::Int32 => {
                let v = i32::from_le_bytes(bytes[0..4].try_into().unwrap());
                v as f64
            }

            AttributeFormat::UInt32 => {
                let v = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
                v as f64
            }

            AttributeFormat::Int64 => {
                let v = i64::from_le_bytes(bytes[0..8].try_into().unwrap());
                v as f64
            }

            AttributeFormat::UInt64 => {
                let v = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
                v as f64
            }

            AttributeFormat::Float => f32::from_le_bytes(bytes[0..4].try_into().unwrap()) as f64,

            AttributeFormat::Double => {
                f64::from_le_bytes(bytes[0..8].try_into().unwrap())
            }

            AttributeFormat::Undefined => 0.0,
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AttributeMetadata {
    pub name: String,
    pub description: String,
    /// usually contains total size of a point attribute (`num_elements * element_size`)
    pub size: u16,
    /// number of elements in the attribute
    pub num_elements: u16,
    /// contains a single element size
    pub element_size: u16,
    pub r#type: AttributeFormat,
    pub min: Vec<f64>,
    pub max: Vec<f64>,
}

impl Metadata {
    #[allow(unused)]
    pub(crate) fn create_flat_root_node(&self) -> OctreeNode {
        OctreeNode {
            name: "r".to_string(),
            bounding_box: self.bounding_box.clone().into(),
            spacing: self.spacing,
            node_type: NodeType::Proxy,
            hierarchy_byte_size: self.hierarchy.first_chunk_size,
            ..Default::default()
        }
    }

    pub fn load_points(
        &self,
        num_points: u32,
        bounding_box: &Aabb,
        buffer: &[u8],
    ) -> Result<Points, LoadPointsError> {
        let points = match self.encoding.as_str() {
            "BROTLI" => self.parse_points_brotli(num_points, bounding_box, buffer)?,
            // PotreeConverter writes "DEFAULT" by default but also accepts and
            // emits "UNCOMPRESSED"; the Potree viewer treats both identically.
            "DEFAULT" | "UNCOMPRESSED" => {
                self.parse_points_default(num_points, bounding_box, buffer)?
            }
            _ => {
                return Err(LoadPointsError::EncodingUnimplemented(
                    self.encoding.clone(),
                ));
            }
        };

        Ok(points)
    }

    fn parse_points_default(
        &self,
        num_points: u32,
        bounding_box: &Aabb,
        buffer: &[u8],
    ) -> Result<Points, LoadPointsError> {
        let num_points_usize = num_points as usize;

        let layout = self.create_attribute_layout();

        // allocate point buffer
        let mut point_buffer = PointBuffer::with_size(num_points_usize, layout);

        let size = bounding_box.max.sub(bounding_box.min);
        let mut grid = vec![0_u32; (GRID_SIZE_UINT * GRID_SIZE_UINT * GRID_SIZE_UINT) as usize];
        let mut num_occupied_cells = 0;

        // compute bytes per point
        let bytes_per_point: usize = self.attributes.iter().map(|a| a.size as usize).sum();

        let mut attribute_offset: usize = 0;

        for point_attribute in &self.attributes {
            let attribute_size = point_attribute.size as usize;
            let Some(mut attribute_slice) = point_buffer.attribute_slice_mut(&point_attribute.name)
            else {
                attribute_offset += attribute_size;
                continue;
            };

            let element_size = point_attribute.element_size as usize;

            match point_attribute.name.as_str() {
                "POSITION_CARTESIAN" | "position" => {
                    let scale = &self.scale;
                    let offset = &self.offset;

                    for j in 0..num_points_usize {
                        let attribute = attribute_slice.get_mut(j);

                        let point_offset = j * bytes_per_point;
                        let bytes = &buffer[(point_offset + attribute_offset)
                            ..(point_offset + attribute_offset + attribute_size)];

                        let x = LittleEndian::read_u32(&bytes[0..element_size]);
                        let y = LittleEndian::read_u32(&bytes[element_size..2 * element_size]);
                        let z = LittleEndian::read_u32(&bytes[2 * element_size..3 * element_size]);

                        // Decode in f64: the absolute coordinate can exceed f32
                        // precision (georeferenced clouds); only the small
                        // node-relative result is narrowed.
                        attribute[0] =
                            (x as f64 * scale[0] + offset[0] - bounding_box.min.x as f64) as f32;
                        attribute[1] =
                            (y as f64 * scale[1] + offset[1] - bounding_box.min.y as f64) as f32;
                        attribute[2] =
                            (z as f64 * scale[2] + offset[2] - bounding_box.min.z as f64) as f32;

                        let index = to_index(&Vec3::from_slice(attribute), &size);
                        grid[index] += 1;
                        if grid[index] == 1 {
                            num_occupied_cells += 1;
                        }

                        attribute[0] += bounding_box.min.x;
                        attribute[1] += bounding_box.min.y;
                        attribute[2] += bounding_box.min.z;

                        // points[j as usize].position = position + bounding_box.min;
                    }
                }
                "RGBA" | "rgba" | "RGB" | "rgb" => {
                    for j in 0..num_points_usize {
                        let attribute = attribute_slice.get_mut(j);

                        let point_offset = j * bytes_per_point;
                        let bytes = &buffer[(point_offset + attribute_offset)
                            ..(point_offset + attribute_offset + attribute_size)];

                        let r = LittleEndian::read_u16(&bytes[0..element_size]);
                        let g = LittleEndian::read_u16(&bytes[element_size..2 * element_size]);
                        let b = LittleEndian::read_u16(&bytes[2 * element_size..3 * element_size]);

                        store_color(r, g, b, attribute);
                    }
                }
                _ => {
                    for j in 0..num_points_usize {
                        let attribute = attribute_slice.get_mut(j);
                        let element_size = point_attribute.element_size as usize;

                        let point_offset = j * bytes_per_point;
                        let bytes = &buffer[(point_offset + attribute_offset)
                            ..(point_offset + attribute_offset + attribute_size)];

                        for i in 0..point_attribute.num_elements as usize {
                            // compute offset & scale
                            let (offset, scale) = if point_attribute.element_size > 4 {
                                let min = point_attribute.min[i];
                                let max = point_attribute.max[i];
                                (min, 1.0 / (max - min))
                            } else {
                                (0.0, 1.0)
                            };

                            attribute[i] = (point_attribute
                                .r#type
                                .parse(&bytes[i..i + element_size])
                                * scale
                                + offset) as f32;
                        }
                    }
                }
            }

            attribute_offset += attribute_size;
        }

        // println!("Final offset: {}, size: {}", byte_offset, size);

        Ok(Points {
            buffer: point_buffer,
            density: num_points
                .checked_div(num_occupied_cells)
                .unwrap_or_default(),
        })
    }

    fn parse_points_brotli(
        &self,
        num_points: u32,
        bounding_box: &Aabb,
        buffer: &[u8],
    ) -> Result<Points, LoadPointsError> {
        let num_points_usize = num_points as usize;
        let mut cursor = Cursor::new(buffer);
        let mut input = brotli_decompressor::Decompressor::new(&mut cursor, 4096);
        let mut decompressed_buffer = Vec::new();
        input.read_to_end(&mut decompressed_buffer)?;

        let layout = self.create_attribute_layout();

        // allocate point buffer
        let mut point_buffer = PointBuffer::with_size(num_points_usize, layout);

        let size = bounding_box.max.sub(bounding_box.min);
        let mut grid = vec![0_u32; (GRID_SIZE_UINT * GRID_SIZE_UINT * GRID_SIZE_UINT) as usize];
        let mut num_occupied_cells = 0;

        let mut byte_offset: usize = 0;

        for point_attribute in &self.attributes {
            let attribute_size = point_attribute.size as usize;
            let Some(mut attribute_slice) = point_buffer.attribute_slice_mut(&point_attribute.name)
            else {
                byte_offset += attribute_size;
                continue;
            };

            match point_attribute.name.as_str() {
                "POSITION_CARTESIAN" | "position" => {
                    let scale = &self.scale;
                    let offset = &self.offset;

                    for j in 0..num_points_usize {
                        let attribute = attribute_slice.get_mut(j);

                        let bytes = &decompressed_buffer[byte_offset..byte_offset + 16];
                        let (x, y, z) = read_morton_128(bytes);

                        // Decode in f64: the absolute coordinate can exceed f32
                        // precision (georeferenced clouds); only the small
                        // node-relative result is narrowed.
                        attribute[0] =
                            (x as f64 * scale[0] + offset[0] - bounding_box.min.x as f64) as f32;
                        attribute[1] =
                            (y as f64 * scale[1] + offset[1] - bounding_box.min.y as f64) as f32;
                        attribute[2] =
                            (z as f64 * scale[2] + offset[2] - bounding_box.min.z as f64) as f32;

                        let index = to_index(&Vec3::from_slice(attribute), &size);
                        grid[index] += 1;
                        if grid[index] == 1 {
                            num_occupied_cells += 1;
                        }

                        attribute[0] += bounding_box.min.x;
                        attribute[1] += bounding_box.min.y;
                        attribute[2] += bounding_box.min.z;

                        byte_offset += 16;
                    }
                }
                "RGBA" | "rgba" | "RGB" | "rgb" => {
                    for j in 0..num_points_usize {
                        let attribute = attribute_slice.get_mut(j);

                        let bytes = &decompressed_buffer[byte_offset..byte_offset + 8];
                        let (r, g, b) = read_morton_64(bytes);

                        store_color(r, g, b, attribute);
                        byte_offset += 8;
                    }
                }
                _ => {
                    for j in 0..num_points_usize {
                        let attribute = attribute_slice.get_mut(j);
                        let element_size = point_attribute.element_size as usize;

                        let bytes = &decompressed_buffer
                            [byte_offset..byte_offset + point_attribute.size as usize];

                        for i in 0..point_attribute.num_elements as usize {
                            // compute offset & scale
                            let (offset, scale) = if point_attribute.element_size > 4 {
                                let min = point_attribute.min[i];
                                let max = point_attribute.max[i];
                                (min, 1.0 / (max - min))
                            } else {
                                (0.0, 1.0)
                            };

                            attribute[i] = (point_attribute
                                .r#type
                                .parse(&bytes[i..i + element_size])
                                * scale
                                + offset) as f32;

                            byte_offset += point_attribute.size as usize;
                        }
                    }
                }
            }
        }

        Ok(Points {
            buffer: point_buffer,
            density: num_points
                .checked_div(num_occupied_cells)
                .unwrap_or_default(),
        })
    }

    fn create_attribute_layout(&self) -> Vec<AttributeInfo> {
        self.attributes
            .iter()
            .scan(0_usize, |offset, a| {
                let current_offset = *offset;
                *offset += a.num_elements as usize;

                Some(AttributeInfo {
                    name: a.name.clone(),
                    r#type: AttributeType::from(a.name.as_str()),
                    offset: current_offset,
                    stride: a.num_elements as usize,
                })
            })
            .collect::<Vec<_>>()
    }
}

fn to_index(position: &Vec3, size: &Vec3) -> usize {
    let index = (GRID_SIZE * position / size)
        .as_uvec3()
        .min(GRID_SIZE_SPLAT);
    (index.x + GRID_SIZE_UINT * index.y + GRID_SIZE_UINT * GRID_SIZE_UINT * index.z) as usize
}

impl From<BoundingBox> for Aabb {
    fn from(val: BoundingBox) -> Self {
        // Render-boundary narrowing: Aabb is glam/f32 territory.
        Aabb {
            min: val.min.map(|v| v as f32).into(),
            max: val.max.map(|v| v as f32).into(),
        }
    }
}

/// Reproduce original potree behaviour, but store color between 0 and 1.0 as f32.
fn store_color(r: u16, g: u16, b: u16, attribute: &mut [f32]) {
    attribute[0] = if r > 255 {
        r as f32 / 65536.0
    } else {
        r as f32 / 256.0
    };
    attribute[1] = if g > 255 {
        g as f32 / 65536.0
    } else {
        g as f32 / 256.0
    };
    attribute[2] = if b > 255 {
        b as f32 / 65536.0
    } else {
        b as f32 / 256.0
    };
}
