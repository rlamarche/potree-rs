use crate::convert::buffer::{
    build_metadata_json, compute_scale_offset, cube_bounds, estimate_spacing, ConvertError,
    MetadataParams, HIERARCHY_BYTES_PER_NODE,
};
use byteorder::{BigEndian, ByteOrder, LittleEndian, ReadBytesExt, WriteBytesExt};
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Hierarchy is split into sub-chunks every HIERARCHY_STEP_SIZE levels (matches C++ default).
const HIERARCHY_STEP_SIZE: u32 = 4;

#[derive(Debug, Clone)]
struct Node {
    name: String,
    level: u32,
    min: [f64; 3],
    max: [f64; 3],
    children: [Option<usize>; 8],
    child_mask: u8,
    num_points: u32,
    byte_offset: u64,
    byte_size: u64,
    temp_path: Option<PathBuf>,
    sample_data: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
enum PlyFormat {
    Ascii,
    BinaryLittleEndian,
    BinaryBigEndian,
}

#[derive(Debug, Clone, Copy)]
enum PlyScalar {
    Char,
    UChar,
    Short,
    UShort,
    Int,
    UInt,
    Float,
    Double,
}

impl PlyScalar {
    fn byte_size(self) -> usize {
        match self {
            PlyScalar::Char | PlyScalar::UChar => 1,
            PlyScalar::Short | PlyScalar::UShort => 2,
            PlyScalar::Int | PlyScalar::UInt | PlyScalar::Float => 4,
            PlyScalar::Double => 8,
        }
    }
    fn potree_type(self) -> &'static str {
        match self {
            PlyScalar::Char => "int8",
            PlyScalar::UChar => "uint8",
            PlyScalar::Short => "int16",
            PlyScalar::UShort => "uint16",
            PlyScalar::Int => "int32",
            PlyScalar::UInt => "uint32",
            PlyScalar::Float => "float",
            PlyScalar::Double => "double",
        }
    }
    fn value_range(self) -> [f64; 2] {
        match self {
            PlyScalar::Char => [-128.0, 127.0],
            PlyScalar::UChar => [0.0, 255.0],
            PlyScalar::Short => [-32768.0, 32767.0],
            PlyScalar::UShort => [0.0, 65535.0],
            PlyScalar::Int => [i32::MIN as f64, i32::MAX as f64],
            PlyScalar::UInt => [0.0, u32::MAX as f64],
            PlyScalar::Float | PlyScalar::Double => [0.0, 1.0],
        }
    }
    /// Decode one value from the little-endian record bytes extras are stored as.
    fn decode_le(self, bytes: &[u8]) -> f64 {
        match self {
            PlyScalar::Char => bytes[0] as i8 as f64,
            PlyScalar::UChar => bytes[0] as f64,
            PlyScalar::Short => LittleEndian::read_i16(bytes) as f64,
            PlyScalar::UShort => LittleEndian::read_u16(bytes) as f64,
            PlyScalar::Int => LittleEndian::read_i32(bytes) as f64,
            PlyScalar::UInt => LittleEndian::read_u32(bytes) as f64,
            PlyScalar::Float => LittleEndian::read_f32(bytes) as f64,
            PlyScalar::Double => LittleEndian::read_f64(bytes),
        }
    }
}

#[derive(Debug, Clone)]
struct ExtraAttribute {
    /// Normalized attribute name written to metadata (see `normalize_extra_name`).
    name: String,
    /// Property name as it appears in the PLY header; used to match values
    /// while reading points.
    source_name: String,
    scalar: PlyScalar,
}

impl ExtraAttribute {
    fn byte_size(&self) -> usize {
        self.scalar.byte_size()
    }
    /// `observed_range` is the actual data min/max from the bounds pass; falls
    /// back to the scalar type's full range when no finite values were seen.
    fn to_metadata_json(&self, observed_range: [f64; 2]) -> serde_json::Value {
        let sz = self.byte_size();
        let [vmin, vmax] = if observed_range[0].is_finite() && observed_range[1].is_finite() {
            observed_range
        } else {
            self.scalar.value_range()
        };
        serde_json::json!({
            "name": self.name,
            "description": "",
            "size": sz,
            "numElements": 1,
            "elementSize": sz,
            "type": self.scalar.potree_type(),
            "min": [vmin],
            "max": [vmax]
        })
    }
}

/// Normalize a PLY extra-property name to the Potree attribute it represents.
///
/// CloudCompare exports scalar fields as `scalar_<Name>`; the Potree viewer
/// looks attributes up by their canonical lowercase names (e.g. "intensity",
/// "classification"). Strips the `scalar_` prefix and lowercases names that
/// match a known Potree attribute; anything else keeps the stripped name.
fn normalize_extra_name(raw: &str) -> String {
    let stripped = if raw.len() > 7 && raw[..7].eq_ignore_ascii_case("scalar_") {
        &raw[7..]
    } else {
        raw
    };
    const CANONICAL: &[&str] = &[
        "intensity",
        "classification",
        "return number",
        "number of returns",
        "gps-time",
        "point source id",
        "user data",
    ];
    // PLY property names cannot contain spaces, so LAS-style names arrive with
    // underscores (e.g. "return_number"); map them to the spaced canonical form.
    let spaced = stripped.to_ascii_lowercase().replace('_', " ");
    if CANONICAL.contains(&spaced.as_str()) {
        spaced
    } else {
        stripped.to_string()
    }
}

#[derive(Debug)]
struct PlyHeader {
    format: PlyFormat,
    vertex_count: usize,
    properties: Vec<(String, PlyScalar)>,
    header_len: u64,
    has_color: bool,
    extra_attributes: Vec<ExtraAttribute>,
}

#[derive(Debug, Clone)]
struct ParsedPoint {
    position: [f64; 3],
    color: Option<[u16; 3]>,
    extra: Vec<u8>,
}

/// Options for [`convert_ply_streaming`].
#[derive(Clone, Debug)]
pub struct ConvertPlyOptions {
    pub name: String,
    pub projection: String,
    pub target_scale: [f64; 3],
    /// Split a node once its bucket exceeds this count. Matches the C++
    /// PotreeConverter's `maxPointsPerNode` default.
    pub max_points_per_node: usize,
    pub max_depth: u32,
    /// "DEFAULT" (raw AoS) or "BROTLI" (SoA + Brotli compression).
    pub encoding: String,
}

impl Default for ConvertPlyOptions {
    fn default() -> Self {
        Self {
            name: "pointcloud".to_string(),
            projection: String::new(),
            target_scale: [0.001; 3],
            max_points_per_node: 10_000,
            max_depth: 20,
            encoding: "DEFAULT".to_string(),
        }
    }
}

pub fn convert_ply_streaming(
    input: &Path,
    output: &Path,
    options: &ConvertPlyOptions,
) -> Result<(), ConvertError> {
    let name = options.name.as_str();
    let projection = options.projection.as_str();
    let target_scale = options.target_scale;
    let max_points_per_node = options.max_points_per_node;
    let max_depth = options.max_depth;
    let encoding = options.encoding.as_str();

    let header = parse_ply_header(input)?;

    // Pass 1: bbox/spacings + observed extra-attribute value ranges
    let pb_bounds = progress_bar(header.vertex_count as u64, "Scanning PLY");
    let (min, data_max, extra_ranges) = pass_compute_bounds(input, &header, Some(&pb_bounds))?;
    pb_bounds.finish_and_clear();
    let total_points = header.vertex_count as u64;
    let (scale, offset) = compute_scale_offset(min, data_max, target_scale);
    let spacing = estimate_spacing(min, data_max, total_points);
    // The octree subdivides a cube (reference converter convention); metadata
    // still records the tight data range for the position attribute.
    let max = cube_bounds(min, data_max);

    // Root node with temp bucket
    let run_id = rand::random::<u64>();
    let extra_size: usize = header.extra_attributes.iter().map(|a| a.byte_size()).sum();
    let record_size = 12 + if header.has_color { 6 } else { 0 } + extra_size;

    let mut nodes: Vec<Node> = Vec::new();
    let mut root_path = std::env::temp_dir();
    root_path.push(format!("potree_{run_id}_r.bin"));
    nodes.push(Node {
        name: "r".to_string(),
        level: 0,
        min,
        max,
        children: [None; 8],
        child_mask: 0,
        num_points: 0,
        byte_offset: 0,
        byte_size: 0,
        temp_path: Some(root_path.clone()),
        sample_data: Vec::new(),
    });

    // Pass 2: stream points into root bucket
    {
        let mut reader = open_after_header(input, &header)?;
        let pb = progress_bar(header.vertex_count as u64, "Ingesting points");
        let mut root_file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&root_path)?;
        for _ in 0..header.vertex_count {
            let point = read_point(&mut reader, &header)?;
            let mut record = Vec::with_capacity(record_size);
            write_point_quantized(&mut record, &point, scale, offset)?;
            root_file.write_all(&record)?;
            nodes[0].num_points = nodes[0].num_points.saturating_add(1);
            pb.inc(1);
        }
        pb.finish_and_clear();
    }

    // Build adaptive tree by splitting buckets
    split_tree(
        &mut nodes,
        &SplitConfig {
            record_size,
            max_points_per_node,
            max_depth,
            scale,
            offset,
            run_id,
        },
    )?;

    // Bottom-up sampling for internal nodes
    sample_tree(&mut nodes, record_size, max_points_per_node, scale, offset, spacing)?;

    // Drop nodes that ended up empty (e.g. leaves whose entire payload was
    // sampled into an ancestor) — the reference viewer only warns on
    // byte_size=0 nodes, but clean output shouldn't contain them.
    prune_empty_nodes(&mut nodes);

    // Reorder to preorder for Potree
    let mut nodes = reorder_nodes_preorder(nodes);

    // Write octree.bin in preorder (internal node samples, leaf buckets), sorted by Morton code.
    let mut octree_file = File::create(output.join("octree.bin"))?;
    let mut current_offset = 0u64;
    let pb_write = progress_bar(nodes.len() as u64, "Writing octree ");
    for node in &mut nodes {
        pb_write.inc(1);
        // Leaves carry their bucket in a temp file, internal nodes carry their
        // LOD sample in memory. An internal node pruned down to a leaf (all
        // children were empty) still holds its payload in `sample_data`.
        let raw: Vec<u8> = if let Some(path) = &node.temp_path {
            let mut f = File::open(path)?;
            let mut data = Vec::new();
            f.read_to_end(&mut data)?;
            let _ = fs::remove_file(path);
            data
        } else {
            std::mem::take(&mut node.sample_data)
        };

        let encoded = if encoding == "BROTLI" {
            encode_soa_brotli(&raw, record_size, header.has_color, &header.extra_attributes)?
        } else {
            let mut data = raw;
            sort_records_by_morton(&mut data, record_size);
            data
        };

        let size = encoded.len() as u64;
        octree_file.write_all(&encoded)?;
        node.byte_offset = current_offset;
        node.byte_size = size;
        current_offset += size;
    }
    pb_write.finish_and_clear();

    // hierarchy.bin (chunked)
    let (hierarchy, first_chunk_size) = build_chunked_hierarchy(&nodes, HIERARCHY_STEP_SIZE);
    fs::write(output.join("hierarchy.bin"), &hierarchy)?;

    let max_level = nodes.iter().map(|n| n.level).max().unwrap_or(0);

    // metadata.json
    let extra_attrs_json: Vec<serde_json::Value> = header
        .extra_attributes
        .iter()
        .zip(&extra_ranges)
        .map(|(a, range)| a.to_metadata_json(*range))
        .collect();
    let metadata = build_metadata_json(&MetadataParams {
        name,
        projection,
        points: total_points,
        min,
        max: data_max,
        scale,
        offset,
        spacing,
        encoding,
        has_color: header.has_color,
        depth: max_level,
        hierarchy_bytes: first_chunk_size,
        step_size: HIERARCHY_STEP_SIZE,
        extra_attrs: &extra_attrs_json,
    })?;
    fs::write(output.join("metadata.json"), metadata)?;

    // Quality summary
    let total_nodes = nodes.len();
    let leaf_count = nodes.iter().filter(|n| n.child_mask == 0).count();
    let internal_count = total_nodes - leaf_count;
    let pts_per_node: Vec<u32> = nodes.iter().map(|n| n.num_points).collect();
    let root_pts = pts_per_node[0];
    let max_pts = pts_per_node.iter().copied().max().unwrap_or(0);
    let avg_pts = pts_per_node.iter().copied().sum::<u32>() as f64 / total_nodes as f64;
    eprintln!(
        "Quality: {} nodes ({} internal, {} leaves) | depth {} | \
         root={} pts | max={} pts/node | avg={:.0} pts/node | spacing={:.4}",
        total_nodes, internal_count, leaf_count, max_level,
        root_pts, max_pts, avg_pts, spacing
    );

    Ok(())
}

fn parse_ply_header(path: &Path) -> Result<PlyHeader, ConvertError> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if !line.trim().eq_ignore_ascii_case("ply") {
        return Err(ConvertError::InvalidInput("Not a PLY file".to_string()));
    }

    line.clear();
    reader.read_line(&mut line)?;
    let format = if line.contains("ascii") {
        PlyFormat::Ascii
    } else if line.contains("binary_little_endian") {
        PlyFormat::BinaryLittleEndian
    } else if line.contains("binary_big_endian") {
        PlyFormat::BinaryBigEndian
    } else {
        return Err(ConvertError::InvalidInput(
            "Unsupported PLY format".to_string(),
        ));
    };

    let mut vertex_count = 0usize;
    let mut properties = Vec::new();
    let mut has_color = false;

    loop {
        line.clear();
        reader.read_line(&mut line)?;
        let trimmed = line.trim();
        if trimmed == "end_header" {
            break;
        }
        if trimmed.starts_with("element") {
            let parts: Vec<_> = trimmed.split_whitespace().collect();
            if parts.len() == 3 && parts[1] == "vertex" {
                vertex_count = parts[2]
                    .parse::<usize>()
                    .map_err(|_| ConvertError::InvalidInput("Invalid vertex count".to_string()))?;
            }
        } else if trimmed.starts_with("property") && vertex_count > 0 {
            let parts: Vec<_> = trimmed.split_whitespace().collect();
            if parts.len() == 3 {
                let scalar = match parts[1] {
                    "char" | "int8" => PlyScalar::Char,
                    "uchar" | "uint8" => PlyScalar::UChar,
                    "short" | "int16" => PlyScalar::Short,
                    "ushort" | "uint16" => PlyScalar::UShort,
                    "int" | "int32" => PlyScalar::Int,
                    "uint" | "uint32" => PlyScalar::UInt,
                    "float" | "float32" => PlyScalar::Float,
                    "double" | "float64" => PlyScalar::Double,
                    _ => {
                        return Err(ConvertError::InvalidInput(
                            "Unsupported property type".to_string(),
                        ))
                    }
                };
                let name = parts[2].to_string();
                if matches!(name.as_str(), "red" | "green" | "blue" | "r" | "g" | "b") {
                    has_color = true;
                }
                properties.push((name, scalar));
            }
        }
    }

    let header_len = reader.stream_position()?;

    let mut extra_attributes: Vec<ExtraAttribute> = Vec::new();
    for (name, scalar) in properties.iter().filter(|(name, _)| {
        !matches!(
            name.as_str(),
            "x" | "y" | "z" | "red" | "green" | "blue" | "r" | "g" | "b"
        )
    }) {
        let normalized = normalize_extra_name(name);
        // Fall back to the original name if normalization would collide with
        // another attribute.
        let taken = |n: &str| {
            n == "position" || n == "rgb" || extra_attributes.iter().any(|a| a.name == n)
        };
        let final_name = if taken(&normalized) {
            name.clone()
        } else {
            normalized
        };
        extra_attributes.push(ExtraAttribute {
            name: final_name,
            source_name: name.clone(),
            scalar: *scalar,
        });
    }

    Ok(PlyHeader {
        format,
        vertex_count,
        properties,
        header_len,
        has_color,
        extra_attributes,
    })
}

fn open_after_header(path: &Path, header: &PlyHeader) -> Result<BufReader<File>, ConvertError> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(header.header_len))?;
    Ok(BufReader::new(file))
}

/// Returns the position bounds plus the observed `[min, max]` of every extra
/// attribute (in `header.extra_attributes` order).
#[expect(clippy::type_complexity, reason = "internal two-pass scan result")]
fn pass_compute_bounds(
    path: &Path,
    header: &PlyHeader,
    progress: Option<&ProgressBar>,
) -> Result<([f64; 3], [f64; 3], Vec<[f64; 2]>), ConvertError> {
    let mut reader = open_after_header(path, header)?;
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    let mut extra_ranges = vec![[f64::INFINITY, f64::NEG_INFINITY]; header.extra_attributes.len()];
    for _ in 0..header.vertex_count {
        let point = read_point(&mut reader, header)?;
        for i in 0..3 {
            min[i] = min[i].min(point.position[i]);
            max[i] = max[i].max(point.position[i]);
        }
        let mut cursor = 0usize;
        for (attr, range) in header.extra_attributes.iter().zip(&mut extra_ranges) {
            let size = attr.byte_size();
            let v = attr.scalar.decode_le(&point.extra[cursor..cursor + size]);
            cursor += size;
            if v.is_finite() {
                range[0] = range[0].min(v);
                range[1] = range[1].max(v);
            }
        }
        if let Some(pb) = progress {
            pb.inc(1);
        }
    }
    Ok((min, max, extra_ranges))
}

fn read_point<R: BufRead + Read>(
    reader: &mut R,
    header: &PlyHeader,
) -> Result<ParsedPoint, ConvertError> {
    match header.format {
        PlyFormat::Ascii => read_point_ascii(reader, header),
        PlyFormat::BinaryLittleEndian => read_point_binary::<_, LittleEndian>(reader, header),
        PlyFormat::BinaryBigEndian => read_point_binary::<_, BigEndian>(reader, header),
    }
}

fn sample_tree(
    nodes: &mut [Node],
    record_size: usize,
    max_points_per_node: usize,
    scale: [f64; 3],
    offset: [f64; 3],
    base_spacing: f64,
) -> Result<(), ConvertError> {
    // Below the quantization grid the Poisson test is vacuous (no two distinct
    // stored points can be closer than one scale unit), so an uncapped sample
    // would vacuum the entire child payload and leave empty descendants.
    // Capacity-bound those levels instead; levels with real rejection keep the
    // natural (reference-matching) Poisson yield.
    let min_spacing = scale.iter().cloned().fold(0.0f64, f64::max);
    // Build postorder traversal order (leaves first, root last).
    let mut order: Vec<usize> = Vec::new();
    let mut stack = vec![0usize];
    while let Some(idx) = stack.pop() {
        order.push(idx);
        for child in nodes[idx].children.iter().flatten() {
            stack.push(*child);
        }
    }

    let internal: Vec<usize> = order.iter().rev().copied()
        .filter(|&i| nodes[i].child_mask != 0)
        .collect();
    let pb = progress_bar(internal.len() as u64, "Sampling nodes");

    for &idx in order.iter().rev() {
        if nodes[idx].child_mask == 0 {
            // Leaf: payload stays as-is; num_points already set during bucket fill.
            continue;
        }
        pb.inc(1);

        let node_min = nodes[idx].min;
        let node_max = nodes[idx].max;
        let center = [
            (node_min[0] + node_max[0]) * 0.5,
            (node_min[1] + node_max[1]) * 0.5,
            (node_min[2] + node_max[2]) * 0.5,
        ];
        let spacing = base_spacing / 2f64.powi(nodes[idx].level as i32);

        // Stable list of child node indices for this internal node.
        let child_indices: Vec<usize> = nodes[idx].children.iter().flatten().copied().collect();

        // Read the current raw-byte payload of each child.
        let child_raw: Vec<Vec<Vec<u8>>> = child_indices
            .iter()
            .map(|&ci| read_raw_records_from_node(&nodes[ci], record_size))
            .collect::<Result<_, _>>()?;

        // Flatten to positions + source tags for Poisson sampling.
        let mut all_positions: Vec<[f64; 3]> = Vec::new();
        let mut record_tags: Vec<(usize, usize)> = Vec::new(); // (child_list_idx, record_idx)
        for (ci, records) in child_raw.iter().enumerate() {
            for (ri, record) in records.iter().enumerate() {
                let pt = decode_position(record, scale, offset);
                all_positions.push(pt);
                record_tags.push((ci, ri));
            }
        }

        // Determine which points are accepted into this node's LOD payload.
        let cap = if spacing >= min_spacing {
            usize::MAX
        } else {
            max_points_per_node
        };
        let mut flags = poisson_accept(&all_positions, spacing, center, cap);

        // Never fully drain a child that has children of its own: an internal
        // node with num_points=0 can't be pruned (its subtree still holds
        // points) and would be emitted as an empty byte_size=0 record. Flip
        // one accepted record back to keep such children non-empty.
        for (ci, &child_idx) in child_indices.iter().enumerate() {
            if nodes[child_idx].child_mask == 0 {
                continue;
            }
            let child_flag_idxs: Vec<usize> = record_tags
                .iter()
                .enumerate()
                .filter(|(_, t)| t.0 == ci)
                .map(|(i, _)| i)
                .collect();
            if !child_flag_idxs.is_empty() && child_flag_idxs.iter().all(|&i| flags[i]) {
                flags[*child_flag_idxs.last().unwrap()] = false;
            }
        }

        // Partition: accepted → this node's sample_data; rejected → back to child.
        let mut sample_data: Vec<u8> = Vec::new();
        let mut per_child_rejected: Vec<Vec<Vec<u8>>> =
            child_indices.iter().map(|_| Vec::new()).collect();
        for (i, &(ci, ri)) in record_tags.iter().enumerate() {
            if flags[i] {
                sample_data.extend_from_slice(&child_raw[ci][ri]);
            } else {
                per_child_rejected[ci].push(child_raw[ci][ri].clone());
            }
        }

        // Write back only the rejected records to each child, preserving the
        // invariant that every input point lives in exactly one node's payload.
        for (ci, &child_idx) in child_indices.iter().enumerate() {
            let rejected = std::mem::take(&mut per_child_rejected[ci]);
            write_rejected_to_node(&mut nodes[child_idx], rejected)?;
        }

        nodes[idx].sample_data = sample_data;
        nodes[idx].num_points = (nodes[idx].sample_data.len() / record_size) as u32;
    }
    pb.finish_and_clear();

    Ok(())
}

fn read_point_ascii<R: BufRead + Read>(
    reader: &mut R,
    header: &PlyHeader,
) -> Result<ParsedPoint, ConvertError> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.is_empty() {
        return Err(ConvertError::InvalidInput("Unexpected EOF".to_string()));
    }
    let parts: Vec<_> = line.split_whitespace().collect();
    if parts.len() < header.properties.len() {
        return Err(ConvertError::InvalidInput(
            "Incomplete vertex line".to_string(),
        ));
    }
    let mut pos = [0f64; 3];
    let mut color: Option<[u16; 3]> = None;
    let mut extra: Vec<u8> = Vec::new();
    let extra_names: HashSet<&str> = header
        .extra_attributes
        .iter()
        .map(|a| a.source_name.as_str())
        .collect();
    for (i, (name, ty)) in header.properties.iter().enumerate() {
        let val = parts[i];
        let v = parse_scalar_ascii(val, ty)?;
        match name.as_str() {
            "x" => pos[0] = v,
            "y" => pos[1] = v,
            "z" => pos[2] = v,
            "red" | "r" => {
                let r = v.round().clamp(0.0, 255.0) as u16 * 257;
                color.get_or_insert([0, 0, 0])[0] = r;
            }
            "green" | "g" => {
                let g = v.round().clamp(0.0, 255.0) as u16 * 257;
                color.get_or_insert([0, 0, 0])[1] = g;
            }
            "blue" | "b" => {
                let b = v.round().clamp(0.0, 255.0) as u16 * 257;
                color.get_or_insert([0, 0, 0])[2] = b;
            }
            _ => {
                if extra_names.contains(name.as_str()) {
                    let sz = ty.byte_size();
                    let mut buf = [0u8; 8];
                    match ty {
                        PlyScalar::Char => buf[0] = v as i8 as u8,
                        PlyScalar::UChar => buf[0] = v as u8,
                        PlyScalar::Short => LittleEndian::write_i16(&mut buf[..2], v as i16),
                        PlyScalar::UShort => LittleEndian::write_u16(&mut buf[..2], v as u16),
                        PlyScalar::Int => LittleEndian::write_i32(&mut buf[..4], v as i32),
                        PlyScalar::UInt => LittleEndian::write_u32(&mut buf[..4], v as u32),
                        PlyScalar::Float => LittleEndian::write_f32(&mut buf[..4], v as f32),
                        PlyScalar::Double => LittleEndian::write_f64(&mut buf[..8], v),
                    }
                    extra.extend_from_slice(&buf[..sz]);
                }
            }
        }
    }
    Ok(ParsedPoint {
        position: pos,
        color,
        extra,
    })
}

fn parse_scalar_ascii(val: &str, _ty: &PlyScalar) -> Result<f64, ConvertError> {
    let v = val
        .parse::<f64>()
        .map_err(|_| ConvertError::InvalidInput(format!("Unable to parse value {}", val)))?;
    Ok(v)
}

fn read_point_binary<R: Read, BO: ByteOrder>(
    reader: &mut R,
    header: &PlyHeader,
) -> Result<ParsedPoint, ConvertError> {
    let mut pos = [0f64; 3];
    let mut color: Option<[u16; 3]> = None;
    let mut extra: Vec<u8> = Vec::new();
    let extra_names: HashSet<&str> = header
        .extra_attributes
        .iter()
        .map(|a| a.source_name.as_str())
        .collect();
    for (name, ty) in &header.properties {
        let v = match ty {
            PlyScalar::Char => reader.read_i8()? as f64,
            PlyScalar::UChar => reader.read_u8()? as f64,
            PlyScalar::Short => reader.read_i16::<BO>()? as f64,
            PlyScalar::UShort => reader.read_u16::<BO>()? as f64,
            PlyScalar::Int => reader.read_i32::<BO>()? as f64,
            PlyScalar::UInt => reader.read_u32::<BO>()? as f64,
            PlyScalar::Float => reader.read_f32::<BO>()? as f64,
            PlyScalar::Double => reader.read_f64::<BO>()?,
        };
        match name.as_str() {
            "x" => pos[0] = v,
            "y" => pos[1] = v,
            "z" => pos[2] = v,
            "red" | "r" => {
                let r = (v.round().clamp(0.0, 255.0) as u16) * 257;
                color.get_or_insert([0, 0, 0])[0] = r;
            }
            "green" | "g" => {
                let g = (v.round().clamp(0.0, 255.0) as u16) * 257;
                color.get_or_insert([0, 0, 0])[1] = g;
            }
            "blue" | "b" => {
                let b = (v.round().clamp(0.0, 255.0) as u16) * 257;
                color.get_or_insert([0, 0, 0])[2] = b;
            }
            _ => {
                if extra_names.contains(name.as_str()) {
                    let sz = ty.byte_size();
                    let mut buf = [0u8; 8];
                    match ty {
                        PlyScalar::Char => buf[0] = v as i8 as u8,
                        PlyScalar::UChar => buf[0] = v as u8,
                        PlyScalar::Short => LittleEndian::write_i16(&mut buf[..2], v as i16),
                        PlyScalar::UShort => LittleEndian::write_u16(&mut buf[..2], v as u16),
                        PlyScalar::Int => LittleEndian::write_i32(&mut buf[..4], v as i32),
                        PlyScalar::UInt => LittleEndian::write_u32(&mut buf[..4], v as u32),
                        PlyScalar::Float => LittleEndian::write_f32(&mut buf[..4], v as f32),
                        PlyScalar::Double => LittleEndian::write_f64(&mut buf[..8], v),
                    }
                    extra.extend_from_slice(&buf[..sz]);
                }
            }
        }
    }
    Ok(ParsedPoint {
        position: pos,
        color,
        extra,
    })
}

struct SplitConfig {
    record_size: usize,
    max_points_per_node: usize,
    max_depth: u32,
    scale: [f64; 3],
    offset: [f64; 3],
    run_id: u64,
}

fn split_tree(nodes: &mut Vec<Node>, cfg: &SplitConfig) -> Result<(), ConvertError> {
    let SplitConfig {
        record_size,
        max_points_per_node,
        max_depth,
        scale,
        offset,
        run_id,
    } = *cfg;
    let pb = spinner("Splitting tree");
    let mut queue: VecDeque<usize> = VecDeque::new();
    queue.push_back(0);

    while let Some(idx) = queue.pop_front() {
        pb.set_message(format!("Splitting tree  {} nodes", nodes.len()));
        if nodes[idx].num_points as usize <= max_points_per_node
            || nodes[idx].level >= max_depth
        {
            continue;
        }

        let node_min = nodes[idx].min;
        let node_max = nodes[idx].max;

        // redistribute this node's payload to children
        if let Some(path) = nodes[idx].temp_path.take() {
            let mut f = File::open(&path)?;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)?;

            let center = [
                (node_min[0] + node_max[0]) * 0.5,
                (node_min[1] + node_max[1]) * 0.5,
                (node_min[2] + node_max[2]) * 0.5,
            ];

            // Buffer all records per child first, then flush once per child.
            let mut child_bufs: [Vec<u8>; 8] = Default::default();

            let mut pos = 0;
            while pos + record_size <= buf.len() {
                let record = &buf[pos..pos + record_size];
                pos += record_size;
                let point = decode_position(record, scale, offset);

                let mut child = 0u8;
                if point[2] >= center[2] {
                    child |= 0b0001;
                }
                if point[1] >= center[1] {
                    child |= 0b0010;
                }
                if point[0] >= center[0] {
                    child |= 0b0100;
                }
                child_bufs[child as usize].extend_from_slice(record);
            }
            drop(buf);

            // Futility check: if every point lands in a single octant
            // (typically duplicate-heavy data sharing one quantized position),
            // subdividing can never reduce the bucket — keep this node a leaf.
            let non_empty_octants = child_bufs.iter().filter(|b| !b.is_empty()).count();
            if non_empty_octants <= 1 {
                nodes[idx].temp_path = Some(path);
                continue;
            }
            fs::remove_file(path)?;

            for child in 0u8..8 {
                let records = &child_bufs[child as usize];
                if records.is_empty() {
                    continue;
                }
                // Create children lazily: only octants that actually receive
                // points exist in the hierarchy (no empty-leaf records).
                let child_idx = match nodes[idx].children[child as usize] {
                    Some(ci) => ci,
                    None => {
                        let (cmin, cmax) = child_bounds(node_min, node_max, child);
                        let new_idx = nodes.len();
                        nodes.push(Node {
                            name: format!("{}{}", nodes[idx].name, child),
                            level: nodes[idx].level + 1,
                            min: cmin,
                            max: cmax,
                            children: [None; 8],
                            child_mask: 0,
                            num_points: 0,
                            byte_offset: 0,
                            byte_size: 0,
                            temp_path: None,
                            sample_data: Vec::new(),
                        });
                        nodes[idx].children[child as usize] = Some(new_idx);
                        nodes[idx].child_mask |= 1 << child;
                        new_idx
                    }
                };
                ensure_leaf_bucket(nodes, child_idx, run_id)?;
                let child_path = nodes[child_idx].temp_path.as_ref().unwrap();
                let mut cf = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(child_path)?;
                cf.write_all(records)?;
                nodes[child_idx].num_points +=
                    (records.len() / record_size) as u32;
            }
        }

        // clear this node's payload
        nodes[idx].temp_path = None;
        nodes[idx].num_points = 0;

        // enqueue children
        for child in nodes[idx].children.iter().flatten() {
            queue.push_back(*child);
        }
    }
    pb.finish_and_clear();

    Ok(())
}

fn ensure_leaf_bucket(nodes: &mut [Node], idx: usize, run_id: u64) -> Result<(), ConvertError> {
    if nodes[idx].temp_path.is_none() {
        let mut p = std::env::temp_dir();
        p.push(format!("potree_{}_{}.bin", run_id, nodes[idx].name));
        nodes[idx].temp_path = Some(p);
    }
    Ok(())
}

/// Read all raw point records from a node's current payload.
/// - Leaves: reads from the temp file on disk.
/// - Internal nodes: reads from `sample_data` (their already-computed LOD payload).
fn read_raw_records_from_node(
    node: &Node,
    record_size: usize,
) -> Result<Vec<Vec<u8>>, ConvertError> {
    let slice: &[u8] = if node.child_mask == 0 {
        if let Some(path) = &node.temp_path {
            let mut f = File::open(path)?;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)?;
            return Ok(buf.chunks_exact(record_size).map(|c| c.to_vec()).collect());
        }
        return Ok(Vec::new());
    } else {
        &node.sample_data
    };
    Ok(slice
        .chunks_exact(record_size)
        .map(|c| c.to_vec())
        .collect())
}

/// Write the rejected records back into a node's payload after the parent has
/// taken the accepted points for its LOD sample.
/// - Leaves: overwrites the temp file (removes it when empty).
/// - Internal nodes: replaces `sample_data`.
fn write_rejected_to_node(node: &mut Node, rejected: Vec<Vec<u8>>) -> Result<(), ConvertError> {
    let n = rejected.len() as u32;
    if node.child_mask == 0 {
        if n == 0 {
            if let Some(path) = node.temp_path.take() {
                let _ = fs::remove_file(&path);
            }
        } else {
            let path = node
                .temp_path
                .get_or_insert_with(|| {
                    let mut p = std::env::temp_dir();
                    p.push(format!("potree_node_{}.bin", node.name));
                    p
                })
                .clone();
            let mut f = File::create(&path)?;
            for r in &rejected {
                f.write_all(r)?;
            }
        }
    } else {
        node.sample_data = rejected.into_iter().flatten().collect();
    }
    node.num_points = n;
    Ok(())
}

/// Core Poisson-disk acceptance algorithm matching PotreeConverter `SamplerPoisson`.
///
/// Returns `flags[i] = true` when `positions[i]` is accepted (not masked by
/// any closer-to-center accepted point within `spacing`).
///
/// Algorithm:
/// - Sort positions by distance-to-`center` ascending (closest processed first).
/// - For each candidate, scan already-accepted list backwards (furthest first).
///   - Early-exit when an accepted point's center-dist² < `(cd - spacing)²`
///     (no earlier accepted point can be within `spacing` of the candidate).
///   - Hard cap of 10 000 backward-scan steps per candidate.
fn poisson_accept(
    positions: &[[f64; 3]],
    spacing: f64,
    center: [f64; 3],
    cap: usize,
) -> Vec<bool> {
    if positions.is_empty() {
        return Vec::new();
    }

    let mut order: Vec<(usize, f64)> = positions
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let d = (p[0] - center[0]).powi(2)
                + (p[1] - center[1]).powi(2)
                + (p[2] - center[2]).powi(2);
            (i, d)
        })
        .collect();
    order.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let spacing_sq = spacing * spacing;
    let mut accepted_positions: Vec<[f64; 3]> = Vec::new();
    let mut flags = vec![false; positions.len()];

    'outer: for (orig_idx, _) in &order {
        if accepted_positions.len() >= cap {
            break;
        }
        let pos = positions[*orig_idx];
        let cx = pos[0] - center[0];
        let cy = pos[1] - center[1];
        let cz = pos[2] - center[2];
        let cd = (cx * cx + cy * cy + cz * cz).sqrt();
        let limit = cd - spacing;
        let limit_sq = limit * limit;

        let mut checks = 0usize;
        for ap in accepted_positions.iter().rev() {
            let px = ap[0] - center[0];
            let py = ap[1] - center[1];
            let pz = ap[2] - center[2];
            let pdd = px * px + py * py + pz * pz;
            if pdd < limit_sq {
                break; // early exit: all remaining accepted points are even closer → safe
            }
            let dx = ap[0] - pos[0];
            let dy = ap[1] - pos[1];
            let dz = ap[2] - pos[2];
            if dx * dx + dy * dy + dz * dz < spacing_sq {
                continue 'outer; // too close → reject candidate
            }
            checks += 1;
            if checks >= 10_000 {
                break;
            }
        }

        flags[*orig_idx] = true;
        accepted_positions.push(pos);
    }

    flags
}

/// Poisson-disk sample `points` for an internal octree node.
/// Uses [`poisson_accept`] and re-encodes accepted points into raw records.
#[cfg(test)]
fn poisson_sample(
    points: &[ParsedPoint],
    record_size: usize,
    spacing: f64,
    scale: [f64; 3],
    offset: [f64; 3],
    node_min: [f64; 3],
    node_max: [f64; 3],
) -> Result<Vec<u8>, ConvertError> {
    if points.is_empty() {
        return Ok(Vec::new());
    }

    let center = [
        (node_min[0] + node_max[0]) * 0.5,
        (node_min[1] + node_max[1]) * 0.5,
        (node_min[2] + node_max[2]) * 0.5,
    ];
    let positions: Vec<[f64; 3]> = points.iter().map(|p| p.position).collect();
    let flags = poisson_accept(&positions, spacing, center, usize::MAX);

    let mut out = Vec::new();
    for (p, &accepted) in points.iter().zip(flags.iter()) {
        if accepted {
            let mut record = Vec::with_capacity(record_size);
            write_point_quantized(&mut record, p, scale, offset)?;
            out.extend_from_slice(&record);
        }
    }
    Ok(out)
}

fn decode_position(record: &[u8], scale: [f64; 3], offset: [f64; 3]) -> [f64; 3] {
    let x = LittleEndian::read_i32(&record[0..4]) as f64 * scale[0] + offset[0];
    let y = LittleEndian::read_i32(&record[4..8]) as f64 * scale[1] + offset[1];
    let z = LittleEndian::read_i32(&record[8..12]) as f64 * scale[2] + offset[2];
    [x, y, z]
}

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

/// Drop empty nodes from the tree: a node is kept only if it has points or a
/// kept descendant (the root is always kept). Parents' child links/masks are
/// updated. Relies on children having larger indices than their parent (true
/// for `split_tree`, which pushes children after the parent).
fn prune_empty_nodes(nodes: &mut [Node]) {
    let mut keep = vec![false; nodes.len()];
    for idx in (0..nodes.len()).rev() {
        let mut any_kept_child = false;
        for c in 0..8 {
            if let Some(ci) = nodes[idx].children[c] {
                debug_assert!(ci > idx, "child index must be larger than parent");
                if keep[ci] {
                    any_kept_child = true;
                } else {
                    nodes[idx].children[c] = None;
                    nodes[idx].child_mask &= !(1 << c);
                }
            }
        }
        keep[idx] = idx == 0 || nodes[idx].num_points > 0 || any_kept_child;
    }
}

fn reorder_nodes_preorder(nodes: Vec<Node>) -> Vec<Node> {
    let mut new_nodes = Vec::with_capacity(nodes.len());
    let mut mapping = vec![usize::MAX; nodes.len()];
    let mut stack = Vec::new();
    stack.push(0usize);

    while let Some(old_idx) = stack.pop() {
        if mapping[old_idx] != usize::MAX {
            continue;
        }
        let new_idx = new_nodes.len();
        mapping[old_idx] = new_idx;
        new_nodes.push(nodes[old_idx].clone());

        // push children in reverse order to visit 0..7 in preorder
        for child in (0..8).rev() {
            if let Some(child_idx) = nodes[old_idx].children[child] {
                stack.push(child_idx);
            }
        }
    }

    // remap children/masks (skip nodes unreachable from the root, e.g. pruned)
    for old_idx in 0..nodes.len() {
        let new_idx = mapping[old_idx];
        if new_idx == usize::MAX {
            continue;
        }
        let mut new_children = [None; 8];
        let mut mask = 0u8;
        for (i, child) in nodes[old_idx].children.iter().enumerate() {
            if let Some(old_child) = child {
                let mapped = mapping[*old_child];
                new_children[i] = Some(mapped);
                mask |= 1 << i;
            }
        }
        new_nodes[new_idx].children = new_children;
        new_nodes[new_idx].child_mask = mask;
    }

    new_nodes
}

// ── Morton-code sorting ───────────────────────────────────────────────────────

/// Spread the low 21 bits of `x` into every third bit position (bits 0, 3, 6, …, 60).
/// Correct magic constants for a 63-bit Morton code with 21 bits per axis.
/// See <https://www.forceflow.be/2013/10/07/morton-encodingdecoding-through-bit-interleaving-implementations/>
fn split_by_3(mut x: u64) -> u64 {
    x &= 0x0000_0000_001f_ffff; // keep 21 bits
    x = (x | (x << 32)) & 0x001f_0000_0000_ffff;
    x = (x | (x << 16)) & 0x001f_0000_ff00_00ff;
    x = (x | (x << 8))  & 0x100f_00f0_0f00_f00f;
    x = (x | (x << 4))  & 0x10c3_0c30_c30c_30c3;
    x = (x | (x << 2))  & 0x1249_2492_4924_9249;
    x
}

fn morton_encode(x: u32, y: u32, z: u32) -> u64 {
    split_by_3(x as u64) | (split_by_3(y as u64) << 1) | (split_by_3(z as u64) << 2)
}

/// Read the quantized i32 position from the first 12 bytes of a record and
/// return its 63-bit Morton code (coordinates shifted to unsigned range).
fn morton_code_from_record(record: &[u8]) -> u64 {
    let ix = LittleEndian::read_i32(&record[0..4]);
    let iy = LittleEndian::read_i32(&record[4..8]);
    let iz = LittleEndian::read_i32(&record[8..12]);
    // XOR with 0x8000_0000 maps i32 → u32 preserving order ([MIN..MAX] → [0..MAX_U32]).
    let ux = (ix as u32) ^ 0x8000_0000;
    let uy = (iy as u32) ^ 0x8000_0000;
    let uz = (iz as u32) ^ 0x8000_0000;
    morton_encode(ux, uy, uz)
}

/// Sort the raw byte buffer of point records in-place by Morton code.
fn sort_records_by_morton(data: &mut [u8], record_size: usize) {
    if data.len() < record_size * 2 {
        return;
    }
    let n = data.len() / record_size;
    let mut indices: Vec<usize> = (0..n).collect();
    indices.sort_by_key(|&i| morton_code_from_record(&data[i * record_size..(i + 1) * record_size]));
    // Apply permutation using a temp buffer.
    let orig = data.to_vec();
    for (new_i, &old_i) in indices.iter().enumerate() {
        data[new_i * record_size..(new_i + 1) * record_size]
            .copy_from_slice(&orig[old_i * record_size..(old_i + 1) * record_size]);
    }
}

// ── BROTLI (SoA + Morton) encoding ───────────────────────────────────────────

/// Encode a 32-bit-per-axis position as a 128-bit Morton code (16 bytes).
///
/// The layout matches what `read_morton_128` in `src/morton.rs` decodes:
/// - bytes[0..8]  = Morton of high 16 bits (mc_h), stored little-endian
/// - bytes[8..16] = Morton of low  16 bits (mc_l), stored little-endian
fn encode_morton_128(x: u32, y: u32, z: u32) -> [u8; 16] {
    let lower = morton_encode(x & 0xFFFF, y & 0xFFFF, z & 0xFFFF);
    let upper = morton_encode(x >> 16, y >> 16, z >> 16);
    let mut out = [0u8; 16];
    LittleEndian::write_u32(&mut out[0..4], upper as u32);
    LittleEndian::write_u32(&mut out[4..8], (upper >> 32) as u32);
    LittleEndian::write_u32(&mut out[8..12], lower as u32);
    LittleEndian::write_u32(&mut out[12..16], (lower >> 32) as u32);
    out
}

/// Encode a 16-bit-per-channel RGB triple as a 64-bit Morton code (8 bytes).
///
/// The layout matches what `read_morton_64` in `src/morton.rs` decodes:
/// - bytes[0..4] = lower 32 bits of Morton code, bytes[4..8] = upper 32 bits
fn encode_morton_64(r: u16, g: u16, b: u16) -> [u8; 8] {
    let mc = morton_encode(r as u32, g as u32, b as u32);
    let mut out = [0u8; 8];
    LittleEndian::write_u32(&mut out[0..4], mc as u32);
    LittleEndian::write_u32(&mut out[4..8], (mc >> 32) as u32);
    out
}

/// Encode an AoS record buffer as Brotli-compressed Struct-of-Arrays with Morton-coded
/// position and colour.  Matches the C++ PotreeConverter BROTLI format:
///   - position attribute: n × 16 bytes (128-bit Morton, absolute quantised u32)
///   - rgb attribute (if any): n × 8 bytes (64-bit Morton, u16 per channel)
///   - extra attributes: n × attribute.size bytes each (raw, no Morton)
fn encode_soa_brotli(
    records: &[u8],
    record_size: usize,
    has_color: bool,
    extra_attributes: &[ExtraAttribute],
) -> Result<Vec<u8>, ConvertError> {
    if record_size == 0 || records.is_empty() {
        return Ok(Vec::new());
    }
    let n = records.len() / record_size;

    let color_off = 12usize;
    let extra_off = color_off + if has_color { 6 } else { 0 };
    let extra_total: usize = extra_attributes.iter().map(|a| a.byte_size()).sum();
    let mut soa: Vec<u8> =
        Vec::with_capacity(n * 16 + n * if has_color { 8 } else { 0 } + n * extra_total);

    // Position array: absolute quantised i32 → u32 → 128-bit Morton
    for i in 0..n {
        let o = i * record_size;
        let ix = LittleEndian::read_i32(&records[o..o + 4]);
        let iy = LittleEndian::read_i32(&records[o + 4..o + 8]);
        let iz = LittleEndian::read_i32(&records[o + 8..o + 12]);
        soa.extend_from_slice(&encode_morton_128(ix as u32, iy as u32, iz as u32));
    }

    // Colour array: u16 RGB → 64-bit Morton
    if has_color {
        for i in 0..n {
            let o = i * record_size + color_off;
            let r = LittleEndian::read_u16(&records[o..o + 2]);
            let g = LittleEndian::read_u16(&records[o + 2..o + 4]);
            let b = LittleEndian::read_u16(&records[o + 4..o + 6]);
            soa.extend_from_slice(&encode_morton_64(r, g, b));
        }
    }

    // Extra attribute arrays: raw bytes, no Morton
    let mut ea_off = extra_off;
    for attr in extra_attributes {
        let sz = attr.byte_size();
        for i in 0..n {
            let o = i * record_size + ea_off;
            soa.extend_from_slice(&records[o..o + sz]);
        }
        ea_off += sz;
    }

    // Brotli-compress the SoA buffer
    let mut compressed = Vec::new();
    {
        let mut writer = brotli::CompressorWriter::new(&mut compressed, 4096, 5, 22);
        writer
            .write_all(&soa)
            .map_err(|e| ConvertError::InvalidInput(e.to_string()))?;
    }
    Ok(compressed)
}

// ── Hierarchy chunking ────────────────────────────────────────────────────────

/// Build `hierarchy.bin` with sub-chunk support (matches C++ `hierarchyStepSize`).
///
/// Each chunk covers `step_size` levels. Nodes at depths that are exact multiples
/// of `step_size` (and depth > 0) appear **twice**:
/// - As a PROXY record (type=2) in the parent chunk, with `byteOffset`/`byteSize`
///   pointing to the sub-chunk in `hierarchy.bin`.
/// - As the **first** NORMAL/LEAF record in their own sub-chunk.
///
/// Returns `(hierarchy_bytes, first_chunk_byte_size)`.
fn build_chunked_hierarchy(nodes: &[Node], step_size: u32) -> (Vec<u8>, usize) {
    if nodes.is_empty() {
        return (Vec::new(), 0);
    }

    // Collect all chunk-root names (one per node whose depth is a multiple of step_size).
    let mut chunk_roots: Vec<&str> = Vec::new();
    for node in nodes {
        if node.level % step_size == 0 {
            chunk_roots.push(&node.name);
        }
    }
    // BFS order: shorter names first, then lexicographic within same depth.
    chunk_roots.sort_by(|a, b| a.len().cmp(&b.len()).then(a.cmp(b)));
    chunk_roots.dedup();

    // Number of records in the chunk rooted at `root_name`:
    //   all N where N.name starts_with root_name AND depth(N) <= depth(root) + step_size.
    // Boundary nodes (depth == depth(root)+step_size) are the PROXY entries.
    let chunk_record_count = |root: &str| -> usize {
        let root_depth = (root.len() - 1) as u32;
        nodes
            .iter()
            .filter(|n| n.name.starts_with(root) && n.level <= root_depth + step_size)
            .count()
    };

    // Compute cumulative byte offsets for each chunk (BFS order).
    let mut chunk_offset: HashMap<&str, u64> = HashMap::new();
    let mut cursor = 0u64;
    for &root in &chunk_roots {
        chunk_offset.insert(root, cursor);
        cursor += (chunk_record_count(root) * HIERARCHY_BYTES_PER_NODE) as u64;
    }

    let first_chunk_size = chunk_record_count("r") * HIERARCHY_BYTES_PER_NODE;

    let mut out: Vec<u8> = Vec::with_capacity(cursor as usize);

    for &chunk_root in &chunk_roots {
        let root_depth = (chunk_root.len() - 1) as u32;
        let boundary_depth = root_depth + step_size;

        // Collect records for this chunk in BFS order.
        let mut chunk_nodes: Vec<&Node> = nodes
            .iter()
            .filter(|n| n.name.starts_with(chunk_root) && n.level <= boundary_depth)
            .collect();
        chunk_nodes.sort_by(|a, b| a.name.len().cmp(&b.name.len()).then(a.name.cmp(&b.name)));

        for node in chunk_nodes {
            // A node at the boundary depth is written as PROXY in this chunk (its own
            // chunk starts with it again as NORMAL/LEAF with actual octree.bin data).
            let is_proxy = node.level == boundary_depth;

            let (node_type, byte_offset, byte_size) = if is_proxy {
                let sub_off = *chunk_offset.get(node.name.as_str()).unwrap_or(&0);
                let sub_size = (chunk_record_count(&node.name) * HIERARCHY_BYTES_PER_NODE) as u64;
                (2u8, sub_off, sub_size)
            } else {
                let t = if node.child_mask == 0 { 1u8 } else { 0u8 };
                (t, node.byte_offset, node.byte_size)
            };

            out.write_u8(node_type).unwrap();
            out.write_u8(node.child_mask).unwrap();
            out.write_u32::<LittleEndian>(node.num_points).unwrap();
            out.write_u64::<LittleEndian>(byte_offset).unwrap();
            out.write_u64::<LittleEndian>(byte_size).unwrap();
        }
    }

    (out, first_chunk_size)
}

fn write_point_quantized<W: Write>(
    writer: &mut W,
    point: &ParsedPoint,
    scale: [f64; 3],
    offset: [f64; 3],
) -> Result<(), ConvertError> {
    let ix = quantize_i32(point.position[0], scale[0], offset[0]);
    let iy = quantize_i32(point.position[1], scale[1], offset[1]);
    let iz = quantize_i32(point.position[2], scale[2], offset[2]);

    writer.write_i32::<LittleEndian>(ix)?;
    writer.write_i32::<LittleEndian>(iy)?;
    writer.write_i32::<LittleEndian>(iz)?;

    if let Some([r, g, b]) = point.color {
        writer.write_u16::<LittleEndian>(r)?;
        writer.write_u16::<LittleEndian>(g)?;
        writer.write_u16::<LittleEndian>(b)?;
    }
    writer.write_all(&point.extra)?;
    Ok(())
}

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

fn progress_bar(len: u64, msg: &str) -> ProgressBar {
    let pb = ProgressBar::new(len);
    let style = ProgressStyle::with_template(
        "{msg} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} eta:{eta}",
    )
    .unwrap_or_else(|_| ProgressStyle::default_bar())
    .progress_chars("##-");
    pb.set_style(style);
    pb.set_message(msg.to_string());
    pb
}

fn spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    let style = ProgressStyle::with_template("{msg} [{elapsed_precise}] {spinner}")
        .unwrap_or_else(|_| ProgressStyle::default_spinner());
    pb.set_style(style);
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(std::time::Duration::from_millis(100));
    pb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_bounds_expands_to_largest_extent() {
        let min = [1.0, 2.0, 3.0];
        let max = [21.0, 12.0, 8.0]; // extents 20, 10, 5
        assert_eq!(cube_bounds(min, max), [21.0, 22.0, 23.0]);
    }

    #[test]
    fn cube_bounds_is_noop_for_cubes() {
        let min = [0.0, 0.0, 0.0];
        let max = [1.0, 1.0, 1.0];
        assert_eq!(cube_bounds(min, max), [1.0, 1.0, 1.0]);
    }

    #[test]
    fn extra_names_normalize_cloudcompare_and_las_styles() {
        assert_eq!(normalize_extra_name("scalar_Intensity"), "intensity");
        assert_eq!(normalize_extra_name("Scalar_Classification"), "classification");
        assert_eq!(normalize_extra_name("return_number"), "return number");
        assert_eq!(normalize_extra_name("intensity"), "intensity");
        // unknown fields: prefix stripped, original casing kept
        assert_eq!(normalize_extra_name("scalar_confidence"), "confidence");
        assert_eq!(normalize_extra_name("scalar_Original_cloud_index"), "Original_cloud_index");
        // non-prefixed unknown names pass through untouched
        assert_eq!(normalize_extra_name("nx"), "nx");
    }

    fn pt(x: f64, y: f64, z: f64) -> ParsedPoint {
        ParsedPoint {
            position: [x, y, z],
            color: None,
            extra: vec![],
        }
    }

    // Decode the xyz from a 12-byte (no-color) record.
    fn decode_xyz(record: &[u8], scale: [f64; 3], offset: [f64; 3]) -> [f64; 3] {
        let x = LittleEndian::read_i32(&record[0..4]) as f64 * scale[0] + offset[0];
        let y = LittleEndian::read_i32(&record[4..8]) as f64 * scale[1] + offset[1];
        let z = LittleEndian::read_i32(&record[8..12]) as f64 * scale[2] + offset[2];
        [x, y, z]
    }

    #[test]
    fn poisson_accept_empty() {
        let flags = poisson_accept(&[], 1.0, [0.0; 3], usize::MAX);
        assert!(flags.is_empty());
    }

    #[test]
    fn poisson_accept_flags_count_matches_input() {
        let positions = vec![[0.0f64; 3], [10.0, 0.0, 0.0], [20.0, 0.0, 0.0]];
        let flags = poisson_accept(&positions, 1.0, [10.0, 0.0, 0.0], usize::MAX);
        assert_eq!(flags.len(), 3);
    }

    #[test]
    fn poisson_accept_preserves_original_index_order() {
        // Three points: only the one nearest geometric center should be accepted.
        // Center [5,5,5], points at x=8.0 (cd=3) and x=8.3 (cd=3.3), spacing=0.5.
        // [8.0] (cd=3) → accepted first; [8.3] too close → rejected.
        let positions = vec![[8.3f64, 5.0, 5.0], [8.0, 5.0, 5.0]]; // note: 8.3 is index 0
        let flags = poisson_accept(&positions, 0.5, [5.0, 5.0, 5.0], usize::MAX);
        assert_eq!(flags.len(), 2);
        assert!(!flags[0], "[8.3] (farther from center) should be rejected");
        assert!(flags[1], "[8.0] (closer to center) should be accepted");
    }

    #[test]
    fn poisson_empty_input() {
        let out = poisson_sample(&[], 12, 1.0, [0.001; 3], [0.0; 3], [0.0; 3], [1.0; 3]).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn poisson_single_point_always_accepted() {
        let points = vec![pt(0.5, 0.5, 0.5)];
        let out =
            poisson_sample(&points, 12, 1.0, [0.001; 3], [0.0; 3], [0.0; 3], [1.0; 3]).unwrap();
        assert_eq!(out.len(), 12);
    }

    #[test]
    fn poisson_two_close_points_one_rejected() {
        // Points must be FAR from center (cd > spacing) for rejection to work.
        // spacing = 0.5, node [0,10]^3, center=[5,5,5]
        // Both points near x=8 (cd≈3 >> 0.5), 0.3 apart → second is rejected.
        let points = vec![pt(8.0, 5.0, 5.0), pt(8.3, 5.0, 5.0)];
        let out =
            poisson_sample(&points, 12, 0.5, [0.001; 3], [0.0; 3], [0.0; 3], [10.0; 3]).unwrap();
        assert_eq!(out.len() / 12, 1);
    }

    #[test]
    fn poisson_far_apart_points_both_accepted() {
        // Two points 2.0 apart, spacing = 1.0 → both accepted
        let points = vec![pt(0.0, 0.0, 0.0), pt(2.0, 0.0, 0.0)];
        let scale = [0.001; 3];
        let offset = [0.0; 3];
        let out = poisson_sample(&points, 12, 1.0, scale, offset, [-1.0; 3], [3.0; 3]).unwrap();
        assert_eq!(out.len() / 12, 2);
    }

    #[test]
    fn poisson_minimum_distance_invariant() {
        // 100 points on a regular grid with spacing 0.1; poisson spacing = 0.25
        // All accepted pairs must be at least 0.25 apart (within quantization error).
        let mut points = Vec::new();
        for ix in 0..10 {
            for iy in 0..10 {
                points.push(pt(ix as f64 * 0.1, iy as f64 * 0.1, 0.0));
            }
        }
        let scale = [0.001; 3];
        let offset = [0.0; 3];
        let spacing = 0.25;
        let out = poisson_sample(&points, 12, spacing, scale, offset, [-0.1; 3], [1.1; 3]).unwrap();

        let n = out.len() / 12;
        assert!(n > 0);

        // Decode accepted points and verify minimum distance
        let accepted_xyz: Vec<[f64; 3]> = (0..n)
            .map(|i| decode_xyz(&out[i * 12..(i + 1) * 12], scale, offset))
            .collect();

        for i in 0..accepted_xyz.len() {
            for j in (i + 1)..accepted_xyz.len() {
                let a = accepted_xyz[i];
                let b = accepted_xyz[j];
                let d =
                    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
                // allow small quantization slop (1 scale unit = 0.001)
                assert!(
                    d >= spacing - 0.002,
                    "points too close: {d:.4} < {spacing} (i={i}, j={j})"
                );
            }
        }
    }

    #[test]
    fn poisson_uses_geometric_center_not_centroid() {
        // Node bbox [0, 100]^3 → geometric center = [50,50,50].
        // Two points both near x=80, 0.2 apart with spacing=0.3.
        // Both have cd >> spacing so the C++ early-exit won't misfire.
        // [80,50,50] has cd=30 (closer to geom center) → accepted first.
        // [80.2,50,50] has cd=30.2 → checked against [80]: dist=0.2 < 0.3 → rejected.
        // So only 1 point accepted, and its x ≈ 80.0 (not 80.2).
        let points = vec![pt(80.0, 50.0, 50.0), pt(80.2, 50.0, 50.0)];
        let scale = [0.001; 3];
        let offset = [0.0; 3];
        let out = poisson_sample(&points, 12, 0.3, scale, offset, [0.0; 3], [100.0; 3]).unwrap();
        assert_eq!(out.len() / 12, 1);
        let xyz = decode_xyz(&out[0..12], scale, offset);
        assert!(
            (xyz[0] - 80.0).abs() < 0.002,
            "expected x≈80.0 (closer to geom center), got {}",
            xyz[0]
        );
    }

    // ── Morton code ──────────────────────────────────────────────────────────

    #[test]
    fn morton_encode_origin_is_zero() {
        assert_eq!(morton_encode(0, 0, 0), 0);
    }

    #[test]
    fn morton_encode_bits_interleaved() {
        // x=1 (bit 0 set) → lands at bit position 0 of Morton code
        assert_eq!(morton_encode(1, 0, 0), 0b001);
        // y=1 → bit 1
        assert_eq!(morton_encode(0, 1, 0), 0b010);
        // z=1 → bit 2
        assert_eq!(morton_encode(0, 0, 1), 0b100);
        // all 1 → bit 0,1,2
        assert_eq!(morton_encode(1, 1, 1), 0b111);
    }

    #[test]
    fn morton_sort_orders_by_z_curve() {
        // Build 3 records with i32 quantized positions (no color, record_size=12)
        let mut data = Vec::new();
        // Three points: [100,0,0], [0,0,0], [0,100,0] — after i32 shift to u32,
        // Morton ordering should put [0,0,0] first.
        for &(x, y, z) in &[(100i32, 0, 0), (0, 0, 0), (0, 100, 0)] {
            data.write_i32::<LittleEndian>(x).unwrap();
            data.write_i32::<LittleEndian>(y).unwrap();
            data.write_i32::<LittleEndian>(z).unwrap();
        }
        sort_records_by_morton(&mut data, 12);
        let first_x = LittleEndian::read_i32(&data[0..4]);
        let first_y = LittleEndian::read_i32(&data[4..8]);
        let first_z = LittleEndian::read_i32(&data[8..12]);
        assert_eq!([first_x, first_y, first_z], [0, 0, 0], "origin should sort first");
    }

    #[test]
    fn morton_sort_stable_on_single_record() {
        let mut data: Vec<u8> = vec![1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0]; // one 12-byte record
        let orig = data.clone();
        sort_records_by_morton(&mut data, 12);
        assert_eq!(data, orig);
    }

    // ── Morton 128-bit round-trip ─────────────────────────────────────────────

    #[test]
    fn encode_morton_128_roundtrip() {
        use crate::morton::read_morton_128;
        for &(x, y, z) in &[
            (0u32, 0u32, 0u32),
            (1, 0, 0),
            (0, 1, 0),
            (0, 0, 1),
            (1000, 0, 0),
            (0, 0, 1000),
            (1000, 1000, 1000),
            (0xFFFF, 0xFFFF, 0xFFFF),
        ] {
            let bytes = encode_morton_128(x, y, z);
            let (dx, dy, dz) = read_morton_128(&bytes);
            assert_eq!((dx, dy, dz), (x, y, z), "roundtrip failed for ({x},{y},{z})");
        }
    }

    #[test]
    fn encode_morton_64_roundtrip() {
        use crate::morton::read_morton_64;
        for &(r, g, b) in &[
            (0u16, 0u16, 0u16),
            (255, 0, 0),
            (0, 0, 255),
            (65535, 65535, 65535),
            (1000, 2000, 3000),
        ] {
            let bytes = encode_morton_64(r, g, b);
            let (dr, dg, db) = read_morton_64(&bytes);
            assert_eq!((dr, dg, db), (r, g, b), "roundtrip failed for ({r},{g},{b})");
        }
    }

    // ── Hierarchy chunking ────────────────────────────────────────────────────

    fn make_leaf_node(name: &str, byte_offset: u64, byte_size: u64, num_points: u32) -> Node {
        let level = (name.len() - 1) as u32;
        Node {
            name: name.to_string(),
            level,
            min: [0.0; 3],
            max: [1.0; 3],
            children: [None; 8],
            child_mask: 0,
            num_points,
            byte_offset,
            byte_size,
            temp_path: None,
            sample_data: Vec::new(),
        }
    }

    #[test]
    fn chunked_hierarchy_flat_tree_matches_no_chunking() {
        // Tree depth < step_size: one chunk, firstChunkSize == total size.
        let nodes = vec![
            make_leaf_node("r", 0, 100, 10),
            make_leaf_node("r0", 100, 50, 5),
            make_leaf_node("r1", 150, 50, 5),
        ];
        let (bytes, first_chunk_size) = build_chunked_hierarchy(&nodes, 4);
        assert_eq!(bytes.len() % 22, 0);
        assert_eq!(first_chunk_size, bytes.len(), "flat tree: all in one chunk");
        // All 3 nodes, no proxies
        assert_eq!(bytes.len(), 3 * 22);
    }

    #[test]
    fn chunked_hierarchy_deep_tree_has_proxy_and_subchunk() {
        // Depth-4 node → proxy in root chunk + first record of sub-chunk.
        // Root chunk (depth 0-4 inclusive): nodes r, r0, r0/0, r0/00, r0000 (depth4 = proxy)
        // Sub-chunk r0000: first record = r0000 as NORMAL, then r00000.
        let nodes = vec![
            {
                let mut n = make_leaf_node("r", 0, 0, 0);
                n.child_mask = 1;
                n
            },
            {
                let mut n = make_leaf_node("r0", 0, 0, 0);
                n.child_mask = 1;
                n
            },
            {
                let mut n = make_leaf_node("r00", 0, 0, 0);
                n.child_mask = 1;
                n
            },
            {
                let mut n = make_leaf_node("r000", 0, 0, 0);
                n.child_mask = 1;
                n
            },
            {
                let mut n = make_leaf_node("r0000", 200, 60, 6); // depth 4 = boundary
                n.child_mask = 1;
                n
            },
            make_leaf_node("r00000", 260, 30, 3), // depth 5: in sub-chunk
        ];

        let (bytes, first_chunk_size) = build_chunked_hierarchy(&nodes, 4);

        // Root chunk: r, r0, r00, r000, r0000(proxy) = 5 records
        assert_eq!(first_chunk_size, 5 * 22);
        // Sub-chunk r0000: r0000(normal), r00000(leaf) = 2 records
        assert_eq!(bytes.len(), (5 + 2) * 22, "7 total records (r0000 appears twice)");

        // Check proxy record for r0000 in root chunk (4th record, index 4):
        let proxy_off = 4 * 22;
        let proxy_type = bytes[proxy_off];
        assert_eq!(proxy_type, 2, "r0000 must be PROXY (type=2) in root chunk");

        // The proxy's byteOffset should point to the sub-chunk (starts right after root chunk)
        let proxy_hier_off = LittleEndian::read_u64(&bytes[proxy_off + 6..proxy_off + 14]);
        assert_eq!(proxy_hier_off, first_chunk_size as u64);

        // Sub-chunk first record: r0000 as NORMAL (type 0 since it has a child)
        let sub_type = bytes[first_chunk_size];
        assert_eq!(sub_type, 0, "r0000 in sub-chunk must be NORMAL (type=0)");

        // Sub-chunk first record's byteOffset = octree offset of r0000's actual data
        let sub_oct_off = LittleEndian::read_u64(&bytes[first_chunk_size + 6..first_chunk_size + 14]);
        assert_eq!(sub_oct_off, 200, "r0000 sub-chunk record should have octree byte_offset=200");
    }
}
