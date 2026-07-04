use crate::asset::PotreeAsset;
use crate::metadata::Metadata;
use async_trait::async_trait;
use bytes::Bytes;
#[cfg(feature = "convert")]
use std::collections::VecDeque;
use std::sync::Arc;

#[cfg(feature = "convert")]
use byteorder::{ByteOrder, LittleEndian, WriteBytesExt};
#[cfg(feature = "convert")]
use rand::{rngs::StdRng, Rng, SeedableRng};
#[cfg(feature = "convert")]
use rayon::prelude::*;
#[cfg(feature = "convert")]
use serde_json::json;
#[cfg(feature = "convert")]
use thiserror::Error;

#[cfg(feature = "convert")]
use std::fs::{self, File};
#[cfg(feature = "convert")]
use std::io::{BufWriter, Write};
#[cfg(feature = "convert")]
use std::path::Path;

#[cfg(feature = "convert")]
/// A collection of Potree buffers in memory.
pub struct PotreeBuffers {
    pub metadata_json: Vec<u8>,
    pub hierarchy: Vec<u8>,
    pub octree: Vec<u8>,
}

/// Options for building Potree buffers.
#[cfg(feature = "convert")]
#[derive(Clone, Debug)]
pub struct BuildOptions {
    pub target_scale: [f64; 3],
    pub encoding: String,
    pub max_points_per_node: usize,
    pub max_depth: u32,
    pub seed: Option<u64>,
}

#[cfg(feature = "convert")]
impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            // default to millimeter-scale quantization to avoid coarse grids
            target_scale: [0.001, 0.001, 0.001],
            encoding: "DEFAULT".to_string(),
            max_points_per_node: usize::MAX,
            max_depth: 20,
            seed: None,
        }
    }
}

/// A builder for `PotreeBuffers`.
#[cfg(feature = "convert")]
#[derive(Debug, Clone, Default)]
pub struct PotreeBuilder {
    pointcloud_name: Option<String>,
    positions: Option<Vec<[f64; 3]>>,
    colors: Option<Vec<[u16; 3]>>,
    projection: Option<String>,
    target_scale: Option<[f64; 3]>,
    encoding: Option<String>,
    max_points_per_node: Option<usize>,
    max_depth: Option<u32>,
    seed: Option<u64>,
}

#[cfg(feature = "convert")]
impl PotreeBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn name(mut self, name: String) -> Self {
        self.pointcloud_name = Some(name);
        self
    }

    pub fn positions(mut self, positions: Vec<[f64; 3]>) -> Self {
        self.positions = Some(positions);
        self
    }

    pub fn colors(mut self, colors: Vec<[u16; 3]>) -> Self {
        self.colors = Some(colors);
        self
    }

    pub fn projection(mut self, projection: String) -> Self {
        self.projection = Some(projection);
        self
    }

    pub fn encoding(mut self, encoding: String) -> Self {
        self.encoding = Some(encoding);
        self
    }

    pub fn target_scale(mut self, target_scale: [f64; 3]) -> Self {
        self.target_scale = Some(target_scale);
        self
    }

    pub fn max_points_per_node(mut self, max_points: usize) -> Self {
        self.max_points_per_node = Some(max_points);
        self
    }

    pub fn max_depth(mut self, max_depth: u32) -> Self {
        self.max_depth = Some(max_depth);
        self
    }

    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    pub fn build(self) -> Result<PotreeBuffers, ConvertError> {
        let positions = if let Some(positions) = self.positions {
            if positions.is_empty() {
                return Err(ConvertError::InvalidInput(
                    "Pointcloud must have at least one position".to_string(),
                ));
            }
            positions
        } else {
            return Err(ConvertError::InvalidInput(
                "no positions provided".to_string(),
            ));
        };
        let pointcloud_name = self.pointcloud_name.as_deref().unwrap_or("pointcloud");
        let defaults = BuildOptions::default();
        let options = BuildOptions {
            target_scale: self.target_scale.unwrap_or(defaults.target_scale),
            encoding: self.encoding.unwrap_or(defaults.encoding),
            max_points_per_node: self
                .max_points_per_node
                .unwrap_or(defaults.max_points_per_node),
            max_depth: self.max_depth.unwrap_or(defaults.max_depth),
            seed: self.seed,
        };

        build_potree_buffers_with_options(
            pointcloud_name,
            self.projection.as_deref().unwrap_or(""),
            &positions,
            self.colors.as_deref(),
            &options,
        )
    }
}

#[cfg(feature = "convert")]
#[derive(Debug, Error)]
pub enum ConvertError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid input: {0}")]
    InvalidInput(String),
}

#[cfg(feature = "convert")]
pub const HIERARCHY_BYTES_PER_NODE: usize = 22;

/// Compute the scale and offset for the given minimum and maximum values.
#[cfg(feature = "convert")]
pub fn compute_scale_offset(
    min: [f64; 3],
    max: [f64; 3],
    target_scale: [f64; 3],
) -> ([f64; 3], [f64; 3]) {
    let size = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];

    let min_scale = [
        size[0] / 2f64.powi(30),
        size[1] / 2f64.powi(30),
        size[2] / 2f64.powi(30),
    ];

    let scale = [
        target_scale[0].max(min_scale[0]),
        target_scale[1].max(min_scale[1]),
        target_scale[2].max(min_scale[2]),
    ];

    let offset = min;

    (scale, offset)
}

/// Expand `max` so the box becomes a cube anchored at `min`.
///
/// Matches the C++ PotreeConverter convention: the root bounding box is cubed
/// to the largest extent so octree subdivision yields cubic nodes and the
/// viewer's spacing/LOD assumptions hold.
#[cfg(feature = "convert")]
pub fn cube_bounds(min: [f64; 3], max: [f64; 3]) -> [f64; 3] {
    let side = (0..3).map(|i| max[i] - min[i]).fold(0.0f64, f64::max);
    [min[0] + side, min[1] + side, min[2] + side]
}

/// Estimate the spacing for the given minimum and maximum values.
///
/// Matches the C++ PotreeConverter convention: `max_extent / 128`.
/// This gives approximately `max_points_per_node` LOD points at the root
/// independent of input point density, and halves at every deeper level.
#[cfg(feature = "convert")]
pub fn estimate_spacing(min: [f64; 3], max: [f64; 3], _points: u64) -> f64 {
    let max_extent = (0..3).map(|i| max[i] - min[i]).fold(0.0f64, f64::max);
    if max_extent <= 0.0 {
        return 1.0;
    }
    max_extent / 128.0
}

#[cfg(feature = "convert")]
fn write_octree_bin_positions_impl<W: Write>(
    writer: &mut W,
    positions: &[[f64; 3]],
    colors: Option<&[[u16; 3]]>,
    scale: [f64; 3],
    offset: [f64; 3],
) -> Result<u64, ConvertError> {
    let write_color = colors.is_some();

    for (idx, p) in positions.iter().enumerate() {
        let ix = quantize_i32(p[0], scale[0], offset[0]);
        let iy = quantize_i32(p[1], scale[1], offset[1]);
        let iz = quantize_i32(p[2], scale[2], offset[2]);

        writer.write_i32::<LittleEndian>(ix)?;
        writer.write_i32::<LittleEndian>(iy)?;
        writer.write_i32::<LittleEndian>(iz)?;

        if write_color {
            let colors = colors
                .ok_or_else(|| ConvertError::InvalidInput("missing colors array".to_string()))?;
            let [r, g, b] = colors
                .get(idx)
                .ok_or_else(|| ConvertError::InvalidInput("color count mismatch".to_string()))?;
            writer.write_u16::<LittleEndian>(*r)?;
            writer.write_u16::<LittleEndian>(*g)?;
            writer.write_u16::<LittleEndian>(*b)?;
        }
    }

    let bytes_per_point = if write_color { 18 } else { 12 };
    Ok((positions.len() * bytes_per_point) as u64)
}

#[cfg(feature = "convert")]
pub fn write_octree_bin_positions(
    path: &Path,
    positions: &[[f64; 3]],
    colors: Option<&[[u16; 3]]>,
    scale: [f64; 3],
    offset: [f64; 3],
) -> Result<u64, ConvertError> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);

    let byte_size = write_octree_bin_positions_impl(&mut writer, positions, colors, scale, offset)?;
    writer.flush()?;

    Ok(byte_size)
}

#[cfg(feature = "convert")]
pub fn build_octree_bin_positions(
    positions: &[[f64; 3]],
    colors: Option<&[[u16; 3]]>,
    scale: [f64; 3],
    offset: [f64; 3],
) -> Result<(Vec<u8>, u64), ConvertError> {
    let write_color = colors.is_some();
    let bytes_per_point = if write_color { 18 } else { 12 };
    let mut buffer = vec![0u8; positions.len() * bytes_per_point];

    buffer
        .par_chunks_mut(bytes_per_point)
        .enumerate()
        .try_for_each(|(idx, chunk)| -> Result<(), ConvertError> {
            let p = positions
                .get(idx)
                .ok_or_else(|| ConvertError::InvalidInput("index out of bounds".to_string()))?;
            let ix = quantize_i32(p[0], scale[0], offset[0]);
            let iy = quantize_i32(p[1], scale[1], offset[1]);
            let iz = quantize_i32(p[2], scale[2], offset[2]);

            LittleEndian::write_i32(&mut chunk[0..4], ix);
            LittleEndian::write_i32(&mut chunk[4..8], iy);
            LittleEndian::write_i32(&mut chunk[8..12], iz);

            if write_color {
                let colors = colors.ok_or_else(|| {
                    ConvertError::InvalidInput("missing colors array".to_string())
                })?;
                let [r, g, b] = colors.get(idx).ok_or_else(|| {
                    ConvertError::InvalidInput("color count mismatch".to_string())
                })?;
                LittleEndian::write_u16(&mut chunk[12..14], *r);
                LittleEndian::write_u16(&mut chunk[14..16], *g);
                LittleEndian::write_u16(&mut chunk[16..18], *b);
            }
            Ok(())
        })?;

    Ok((buffer, (positions.len() * bytes_per_point) as u64))
}

#[cfg(feature = "convert")]
pub fn write_single_root_hierarchy(
    path: &Path,
    num_points: u32,
    byte_size: u64,
) -> Result<(), ConvertError> {
    let data = build_single_root_hierarchy(num_points, byte_size, 0)?;
    let mut file = File::create(path)?;
    file.write_all(&data)?;

    Ok(())
}

#[cfg(feature = "convert")]
pub fn build_single_root_hierarchy(
    num_points: u32,
    byte_size: u64,
    _depth: u32,
) -> Result<Vec<u8>, ConvertError> {
    let mut data = Vec::with_capacity(HIERARCHY_BYTES_PER_NODE);

    let node_type: u8 = 1; // LEAF
    let child_mask: u8 = 0;
    let byte_offset: u64 = 0;

    data.write_u8(node_type)?;
    data.write_u8(child_mask)?;
    data.write_u32::<LittleEndian>(num_points)?;
    data.write_u64::<LittleEndian>(byte_offset)?;
    data.write_u64::<LittleEndian>(byte_size)?;

    // depth is not encoded per node; returned via metadata
    Ok(data)
}

#[cfg(feature = "convert")]
pub fn write_metadata_json(path: &Path, params: &MetadataParams) -> Result<(), ConvertError> {
    let content = build_metadata_json(params)?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = File::create(path)?;
    file.write_all(&content)?;

    Ok(())
}

/// Inputs for `build_metadata_json` / `write_metadata_json`.
#[cfg(feature = "convert")]
#[derive(Clone, Debug)]
pub struct MetadataParams<'a> {
    pub name: &'a str,
    pub projection: &'a str,
    pub points: u64,
    /// Tight data bounds; the written `boundingBox` is cubed from these.
    pub min: [f64; 3],
    pub max: [f64; 3],
    pub scale: [f64; 3],
    pub offset: [f64; 3],
    pub spacing: f64,
    pub encoding: &'a str,
    pub has_color: bool,
    pub depth: u32,
    pub hierarchy_bytes: usize,
    pub step_size: u32,
    pub extra_attrs: &'a [serde_json::Value],
}

#[cfg(feature = "convert")]
pub fn build_metadata_json(params: &MetadataParams) -> Result<Vec<u8>, ConvertError> {
    let MetadataParams {
        name,
        projection,
        points,
        min,
        max,
        scale,
        offset,
        spacing,
        encoding,
        has_color,
        depth,
        hierarchy_bytes,
        step_size,
        extra_attrs,
    } = *params;

    // The root bounding box is cubed (reference converter convention) while the
    // position attribute keeps the data range, snapped to the quantization grid
    // like the reference converter writes it.
    let cubed_max = cube_bounds(min, max);
    let snap = |v: f64, s: f64| if s > 0.0 { (v / s).round() * s } else { v };
    let attr_min: Vec<f64> = (0..3).map(|i| snap(min[i], scale[i])).collect();
    let attr_max: Vec<f64> = (0..3).map(|i| snap(max[i], scale[i])).collect();

    let mut attributes = vec![json!({
        "name": "position",
        "description": "",
        "size": 12,
        "numElements": 3,
        "elementSize": 4,
        "type": "int32",
        "min": attr_min,
        "max": attr_max,
        "scale": [scale[0], scale[1], scale[2]],
        "offset": [offset[0], offset[1], offset[2]],
    })];

    if has_color {
        attributes.push(json!({
            "name": "rgb",
            "description": "",
            "size": 6,
            "numElements": 3,
            "elementSize": 2,
            "type": "uint16",
            "min": [0, 0, 0],
            "max": [65535, 65535, 65535]
        }));
    }

    attributes.extend_from_slice(extra_attrs);

    let metadata = json!({
        "version": "2.0",
        "name": name,
        "description": "",
        "points": points,
        "projection": projection,
        "hierarchy": {
            "firstChunkSize": hierarchy_bytes,
            "stepSize": step_size,
            "depth": depth
        },
        "offset": [offset[0], offset[1], offset[2]],
        "scale": [scale[0], scale[1], scale[2]],
        "spacing": spacing,
        "boundingBox": {
            "min": [min[0], min[1], min[2]],
            "max": [cubed_max[0], cubed_max[1], cubed_max[2]]
        },
        "encoding": encoding,
        "attributes": attributes
    });

    let content = serde_json::to_vec_pretty(&metadata)
        .map_err(|err| ConvertError::InvalidInput(err.to_string()))?;
    Ok(content)
}

#[cfg(feature = "convert")]
fn quantize_i32(value: f64, scale: f64, offset: f64) -> i32 {
    if scale == 0.0 {
        return 0;
    }
    let scaled = (value - offset) / scale;
    let rounded = scaled.round();

    if rounded > i32::MAX as f64 {
        i32::MAX
    } else if rounded < i32::MIN as f64 {
        i32::MIN
    } else {
        rounded as i32
    }
}

#[cfg(feature = "convert")]
pub fn build_potree_buffers(
    name: &str,
    projection: &str,
    positions: &[[f64; 3]],
    colors: Option<&[[u16; 3]]>,
    target_scale: [f64; 3],
    encoding: &str,
) -> Result<PotreeBuffers, ConvertError> {
    build_potree_buffers_with_options(
        name,
        projection,
        positions,
        colors,
        &BuildOptions {
            target_scale,
            encoding: encoding.to_string(),
            ..Default::default()
        },
    )
}

#[cfg(feature = "convert")]
#[derive(Clone, Debug)]
struct WorkingNode {
    name: String,
    level: u32,
    min: [f64; 3],
    max: [f64; 3],
    points: Vec<usize>,
    stored_points: Vec<usize>,
    children: [Option<usize>; 8],
    child_mask: u8,
    byte_offset: u64,
    byte_size: u64,
}

#[cfg(feature = "convert")]
fn child_bounds(min: [f64; 3], max: [f64; 3], index: u8) -> ([f64; 3], [f64; 3]) {
    let mut child_min = min;
    let mut child_max = max;
    let size = [
        (max[0] - min[0]) * 0.5,
        (max[1] - min[1]) * 0.5,
        (max[2] - min[2]) * 0.5,
    ];

    if (index & 0b0001) > 0 {
        child_min[2] += size[2];
    } else {
        child_max[2] -= size[2];
    }
    if (index & 0b0010) > 0 {
        child_min[1] += size[1];
    } else {
        child_max[1] -= size[1];
    }
    if (index & 0b0100) > 0 {
        child_min[0] += size[0];
    } else {
        child_max[0] -= size[0];
    }

    (child_min, child_max)
}

#[cfg(feature = "convert")]
fn child_index_for_point(p: [f64; 3], center: [f64; 3]) -> u8 {
    let mut idx = 0;
    if p[2] >= center[2] {
        idx |= 0b0001;
    }
    if p[1] >= center[1] {
        idx |= 0b0010;
    }
    if p[0] >= center[0] {
        idx |= 0b0100;
    }
    idx
}

#[cfg(feature = "convert")]
fn sample_indices(indices: &[usize], max_points: usize, rng: &mut impl Rng) -> Vec<usize> {
    if indices.len() <= max_points {
        return indices.to_vec();
    }

    let mut reservoir = Vec::with_capacity(max_points);
    for (i, &idx) in indices.iter().enumerate() {
        if i < max_points {
            reservoir.push(idx);
        } else {
            let j = rng.gen_range(0..=i);
            if j < max_points {
                reservoir[j] = idx;
            }
        }
    }
    reservoir
}

#[cfg(feature = "convert")]
fn write_points_by_index<W: Write>(
    writer: &mut W,
    positions: &[[f64; 3]],
    colors: Option<&[[u16; 3]]>,
    indices: &[usize],
    scale: [f64; 3],
    offset: [f64; 3],
) -> Result<u64, ConvertError> {
    let write_color = colors.is_some();
    for &idx in indices {
        let p = positions
            .get(idx)
            .ok_or_else(|| ConvertError::InvalidInput("index out of bounds".to_string()))?;
        let ix = quantize_i32(p[0], scale[0], offset[0]);
        let iy = quantize_i32(p[1], scale[1], offset[1]);
        let iz = quantize_i32(p[2], scale[2], offset[2]);

        writer.write_i32::<LittleEndian>(ix)?;
        writer.write_i32::<LittleEndian>(iy)?;
        writer.write_i32::<LittleEndian>(iz)?;

        if write_color {
            let colors = colors
                .ok_or_else(|| ConvertError::InvalidInput("missing colors array".to_string()))?;
            let [r, g, b] = colors
                .get(idx)
                .ok_or_else(|| ConvertError::InvalidInput("color count mismatch".to_string()))?;
            writer.write_u16::<LittleEndian>(*r)?;
            writer.write_u16::<LittleEndian>(*g)?;
            writer.write_u16::<LittleEndian>(*b)?;
        }
    }
    let bytes_per_point = if write_color { 18 } else { 12 };
    Ok((indices.len() * bytes_per_point) as u64)
}

#[cfg(feature = "convert")]
pub fn build_potree_buffers_with_options(
    name: &str,
    projection: &str,
    positions: &[[f64; 3]],
    colors: Option<&[[u16; 3]]>,
    options: &BuildOptions,
) -> Result<PotreeBuffers, ConvertError> {
    if positions.is_empty() {
        return Err(ConvertError::InvalidInput(
            "no positions provided".to_string(),
        ));
    }

    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];

    for p in positions {
        for i in 0..3 {
            min[i] = min[i].min(p[i]);
            max[i] = max[i].max(p[i]);
        }
    }

    let (scale, offset) = compute_scale_offset(min, max, options.target_scale);
    let points = positions.len() as u64;
    let spacing = estimate_spacing(min, max, points);
    // Octree nodes subdivide a cube (reference converter convention); `max`
    // stays the tight data range for the metadata position attribute.
    let cubed_max = cube_bounds(min, max);

    let mut rng = options
        .seed
        .map(StdRng::seed_from_u64)
        .unwrap_or_else(StdRng::from_entropy);

    let mut nodes: Vec<WorkingNode> = Vec::new();
    nodes.push(WorkingNode {
        name: "r".to_string(),
        level: 0,
        min,
        max: cubed_max,
        points: (0..positions.len()).collect(),
        stored_points: Vec::new(),
        children: [None; 8],
        child_mask: 0,
        byte_offset: 0,
        byte_size: 0,
    });

    let mut queue = VecDeque::from([0usize]);
    let mut max_depth_seen = 0u32;

    while let Some(node_idx) = queue.pop_front() {
        let child_limit = options.max_points_per_node;
        let max_depth = options.max_depth;

        let mut child_buckets: [_; 8] = std::array::from_fn(|_| Vec::new());
        let center = [
            (nodes[node_idx].min[0] + nodes[node_idx].max[0]) * 0.5,
            (nodes[node_idx].min[1] + nodes[node_idx].max[1]) * 0.5,
            (nodes[node_idx].min[2] + nodes[node_idx].max[2]) * 0.5,
        ];

        let current_points = std::mem::take(&mut nodes[node_idx].points);

        if current_points.len() >= child_limit && nodes[node_idx].level < max_depth {
            for &idx in &current_points {
                let p = positions[idx];
                let child = child_index_for_point(p, center);
                child_buckets[child as usize].push(idx);
            }
        }

        let mut child_mask = 0u8;
        let mut child_indices: [Option<usize>; 8] = [None; 8];
        let mut created_children = 0usize;

        for (child_idx, bucket) in child_buckets.into_iter().enumerate() {
            if bucket.is_empty() {
                continue;
            }
            child_mask |= 1 << child_idx;
            let (cmin, cmax) =
                child_bounds(nodes[node_idx].min, nodes[node_idx].max, child_idx as u8);
            let child_node_idx = nodes.len();
            nodes.push(WorkingNode {
                name: format!("{}{}", nodes[node_idx].name, child_idx),
                level: nodes[node_idx].level + 1,
                min: cmin,
                max: cmax,
                points: bucket,
                stored_points: Vec::new(),
                children: [None; 8],
                child_mask: 0,
                byte_offset: 0,
                byte_size: 0,
            });
            child_indices[child_idx] = Some(child_node_idx);
            queue.push_back(child_node_idx);
            created_children += 1;
        }

        nodes[node_idx].child_mask = child_mask;
        nodes[node_idx].children = child_indices;

        // sample points to store at this node (LOD). If no children, keep everything up to limit.
        let stored = if child_mask == 0 && current_points.len() <= child_limit {
            current_points
        } else {
            sample_indices(&current_points, child_limit, &mut rng)
        };
        nodes[node_idx].stored_points = stored;
        max_depth_seen = max_depth_seen.max(nodes[node_idx].level);

        // Avoid infinite splitting when all points fall into a single child
        if child_mask == 0 && created_children == 0 {
            continue;
        }
    }

    let write_color = colors.is_some();
    let bytes_per_point = if write_color { 18 } else { 12 };
    let mut octree = Vec::with_capacity(
        nodes.iter().map(|n| n.stored_points.len()).sum::<usize>() * bytes_per_point,
    );
    let mut hierarchy = Vec::with_capacity(nodes.len() * HIERARCHY_BYTES_PER_NODE);

    let mut current_offset = 0u64;
    for node in &mut nodes {
        node.byte_offset = current_offset;
        node.byte_size = write_points_by_index(
            &mut octree,
            positions,
            colors,
            &node.stored_points,
            scale,
            offset,
        )?;
        current_offset += node.byte_size;

        hierarchy.write_u8(if node.child_mask > 0 { 0 } else { 1 })?;
        hierarchy.write_u8(node.child_mask)?;
        hierarchy.write_u32::<LittleEndian>(node.stored_points.len() as u32)?;
        hierarchy.write_u64::<LittleEndian>(node.byte_offset)?;
        hierarchy.write_u64::<LittleEndian>(node.byte_size)?;
    }

    let metadata_json = build_metadata_json(&MetadataParams {
        name,
        projection,
        points,
        min,
        max,
        scale,
        offset,
        spacing,
        encoding: &options.encoding,
        has_color: colors.is_some(),
        depth: max_depth_seen,
        hierarchy_bytes: hierarchy.len(),
        step_size: 5, // legacy step_size for buffer-based builds
        extra_attrs: &[],
    })?;

    Ok(PotreeBuffers {
        metadata_json,
        hierarchy,
        octree,
    })
}

/// An in-memory Potree dataset implementing [`PotreeAsset`].
///
/// This is the read-side counterpart of [`PotreeBuffers`]: convert a point
/// cloud in memory, then serve it straight back through the regular
/// `Hierarchy` / `PointCloud` readers without touching the filesystem.
#[derive(Clone, Debug, Default)]
pub struct PotreeBufferAsset {
    metadata_json: Arc<Vec<u8>>,
    hierarchy: Arc<Vec<u8>>,
    octree: Arc<Vec<u8>>,
}

impl PotreeBufferAsset {
    pub fn new(metadata_json: Vec<u8>, hierarchy: Vec<u8>, octree: Vec<u8>) -> Self {
        Self {
            metadata_json: Arc::new(metadata_json),
            hierarchy: Arc::new(hierarchy),
            octree: Arc::new(octree),
        }
    }
}

#[cfg(feature = "convert")]
impl From<PotreeBuffers> for PotreeBufferAsset {
    fn from(buffers: PotreeBuffers) -> Self {
        Self::new(buffers.metadata_json, buffers.hierarchy, buffers.octree)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PotreeBufferAssetError {
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Range {offset}+{length} out of bounds for in-memory {target} (len {len})")]
    OutOfBounds {
        target: &'static str,
        offset: u64,
        length: usize,
        len: usize,
    },
}

fn slice_range(
    data: &[u8],
    target: &'static str,
    offset: u64,
    length: usize,
) -> Result<Bytes, PotreeBufferAssetError> {
    let out_of_bounds = || PotreeBufferAssetError::OutOfBounds {
        target,
        offset,
        length,
        len: data.len(),
    };

    let start = usize::try_from(offset).map_err(|_| out_of_bounds())?;
    let end = start.checked_add(length).ok_or_else(out_of_bounds)?;
    if end > data.len() {
        return Err(out_of_bounds());
    }

    Ok(Bytes::copy_from_slice(&data[start..end]))
}

#[async_trait]
impl PotreeAsset for PotreeBufferAsset {
    type Error = PotreeBufferAssetError;

    async fn read_metadata(&self) -> Result<Metadata, Self::Error> {
        Ok(serde_json::from_slice(&self.metadata_json)?)
    }

    async fn read_hierarchy(&self, offset: u64, length: usize) -> Result<Bytes, Self::Error> {
        slice_range(&self.hierarchy, "hierarchy.bin", offset, length)
    }

    async fn read_octree(&self, offset: u64, length: usize) -> Result<Bytes, Self::Error> {
        slice_range(&self.octree, "octree.bin", offset, length)
    }
}
