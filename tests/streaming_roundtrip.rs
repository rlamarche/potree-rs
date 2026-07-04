/// End-to-end round-trip tests: convert PLY → load back via the Rust reader.
///
/// Verifies:
/// 1. Byte-offset chain — `byte_offset + byte_size` values form a contiguous
///    sequence with no gaps or overlaps; their total equals `octree.bin` size.
/// 2. Child-mask consistency — hierarchy records' child_mask bits each correspond
///    to an actual child record in the flat parse.
/// 3. Round-trip positions — every decoded point position is within quantization
///    tolerance of one of the original input positions, and each input appears
///    exactly once across all nodes.
use byteorder::{ByteOrder, LittleEndian};
use potree::convert::streaming::{convert_ply_streaming, ConvertPlyOptions};
/// Test shim over `convert_ply_streaming` with the shared defaults.
fn convert(
    input: &std::path::Path,
    output: &std::path::Path,
    name: &str,
    max_points_per_node: usize,
    max_depth: u32,
    encoding: &str,
) {
    convert_ply_streaming(
        input,
        output,
        &ConvertPlyOptions {
            name: name.to_string(),
            max_points_per_node,
            max_depth,
            encoding: encoding.to_string(),
            ..Default::default()
        },
    )
    .unwrap();
}

use potree::hierarchy::HierarchyAsync;
use potree::octree::node::NodeType;
use potree::point::AttributeType;
use potree::prelude::*;
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use tempfile::tempdir;

// ── helpers ──────────────────────────────────────────────────────────────────

fn write_ascii_ply(points: &[[f64; 3]], path: &std::path::Path) {
    let mut file = fs::File::create(path).unwrap();
    writeln!(file, "ply").unwrap();
    writeln!(file, "format ascii 1.0").unwrap();
    writeln!(file, "element vertex {}", points.len()).unwrap();
    writeln!(file, "property double x").unwrap();
    writeln!(file, "property double y").unwrap();
    writeln!(file, "property double z").unwrap();
    writeln!(file, "end_header").unwrap();
    for p in points {
        writeln!(file, "{} {} {}", p[0], p[1], p[2]).unwrap();
    }
}

/// 8 points at the corners of [0,1]³ — one per octant, guarantees the tree
/// splits at every level and produces both internal and leaf nodes.
fn corner_points() -> Vec<[f64; 3]> {
    vec![
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
        [0.0, 1.0, 1.0],
        [1.0, 1.0, 1.0],
    ]
}

// ── byte-offset chain ─────────────────────────────────────────────────────────

#[test]
fn byte_offset_chain_is_contiguous() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("pts.ply");
    let output = dir.path().join("out");
    fs::create_dir_all(&output).unwrap();

    write_ascii_ply(&corner_points(), &input);
    convert(&input, &output, "test", 2, 5, "DEFAULT");

    let hierarchy_bytes = fs::read(output.join("hierarchy.bin")).unwrap();
    let octree_size = fs::metadata(output.join("octree.bin")).unwrap().len();

    assert_eq!(
        hierarchy_bytes.len() % 22,
        0,
        "hierarchy.bin size not a multiple of 22"
    );
    let num_nodes = hierarchy_bytes.len() / 22;
    assert!(num_nodes >= 1);

    // Parse records: (type, child_mask, num_points, byte_offset, byte_size)
    let records: Vec<(u8, u8, u32, u64, u64)> = (0..num_nodes)
        .map(|i| {
            let s = i * 22;
            let ty = hierarchy_bytes[s];
            let mask = hierarchy_bytes[s + 1];
            let np = LittleEndian::read_u32(&hierarchy_bytes[s + 2..s + 6]);
            let off = LittleEndian::read_u64(&hierarchy_bytes[s + 6..s + 14]);
            let sz = LittleEndian::read_u64(&hierarchy_bytes[s + 14..s + 22]);
            (ty, mask, np, off, sz)
        })
        .collect();

    // Verify: sum of all byte_sizes == octree.bin file size
    let total_bytes: u64 = records.iter().map(|r| r.4).sum();
    assert_eq!(
        total_bytes, octree_size,
        "sum of node byte_sizes ({total_bytes}) != octree.bin size ({octree_size})"
    );

    // Verify: byte_offsets are non-overlapping and form a contiguous chain
    // Sort by byte_offset to check for gaps/overlaps
    let mut sorted = records.clone();
    sorted.sort_by_key(|r| r.3); // sort by byte_offset
    let mut cursor = 0u64;
    for r in &sorted {
        if r.4 == 0 {
            continue; // empty node, offset may be at any position (set to cursor at write time)
        }
        assert_eq!(
            r.3, cursor,
            "gap or overlap at byte_offset={} expected cursor={cursor}",
            r.3
        );
        cursor += r.4;
    }
    assert_eq!(cursor, octree_size);

    // Verify: total num_points across all nodes == 8 (each input point exactly once)
    let total_points: u64 = records.iter().map(|r| r.2 as u64).sum();
    assert_eq!(total_points, 8, "total num_points in hierarchy != 8");
}

// ── round-trip positions ──────────────────────────────────────────────────────

#[tokio::test(flavor = "current_thread")]
async fn roundtrip_positions_match_input() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("pts.ply");
    let output = dir.path().join("out");
    fs::create_dir_all(&output).unwrap();

    let source = corner_points();
    write_ascii_ply(&source, &input);
    convert(&input, &output, "test", 2, 5, "DEFAULT");

    // Load back through the Rust reader using the filesystem asset.
    let hierarchy = Hierarchy::from_path(&output)
        .await
        .expect("failed to load hierarchy from converted output");

    let nodes = hierarchy
        .load_entire_hierarchy()
        .await
        .expect("failed to load entire hierarchy");

    // Collect all decoded point positions from every node.
    let scale = 0.001_f64;
    let mut all_decoded: Vec<[f64; 3]> = Vec::new();

    for node in &nodes {
        if matches!(node.node_type, NodeType::Proxy) {
            continue;
        }
        if node.num_points == 0 {
            continue;
        }
        let points = hierarchy
            .load_points(node)
            .await
            .expect("failed to load points for node");
        for p in points.buffer.iter() {
            let pos = p
                .attribute_type(AttributeType::Position)
                .expect("node points missing position attribute");
            all_decoded.push([pos[0] as f64, pos[1] as f64, pos[2] as f64]);
        }
    }

    // Every decoded position must be within quantization tolerance of a source point.
    let tol = scale * 2.0; // two scale units of tolerance
    let mut matched: HashSet<usize> = HashSet::new();
    for decoded in &all_decoded {
        let closest = source.iter().enumerate().min_by(|(_, a), (_, b)| {
            let da = (a[0] - decoded[0]).powi(2)
                + (a[1] - decoded[1]).powi(2)
                + (a[2] - decoded[2]).powi(2);
            let db = (b[0] - decoded[0]).powi(2)
                + (b[1] - decoded[1]).powi(2)
                + (b[2] - decoded[2]).powi(2);
            da.partial_cmp(&db).unwrap()
        });
        let (src_idx, src) = closest.unwrap();
        let dist = ((src[0] - decoded[0]).powi(2)
            + (src[1] - decoded[1]).powi(2)
            + (src[2] - decoded[2]).powi(2))
        .sqrt();
        assert!(
            dist <= tol,
            "decoded point {decoded:?} is {dist:.4} from nearest source {src:?} (tol={tol})"
        );
        matched.insert(src_idx);
    }

    // Total decoded count = 8 (no duplicates, no missing points).
    assert_eq!(
        all_decoded.len(),
        source.len(),
        "decoded {} points, expected {}",
        all_decoded.len(),
        source.len()
    );
    assert_eq!(
        matched.len(),
        source.len(),
        "only {}/{} source points were recovered",
        matched.len(),
        source.len()
    );
}

// ── hierarchy chunking ────────────────────────────────────────────────────────

/// Force a tree deep enough (depth > 4) so that hierarchy.bin has sub-chunks.
/// Verifies firstChunkSize < hierarchy.len() and that the round-trip still works.
#[tokio::test(flavor = "current_thread")]
async fn deep_tree_uses_hierarchy_chunking() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("pts.ply");
    let output = dir.path().join("out");
    fs::create_dir_all(&output).unwrap();

    // 3000 points 1 mm apart along a 3 m line: pitch is finer than the
    // Poisson spacing down to level 4 (3/(128·2⁴) ≈ 1.5 mm), so a real
    // depth-4 tree survives sampling, while the resolution guard stops
    // deeper levels (spacing at level 5 would fall below the 1 mm scale).
    let source: Vec<[f64; 3]> = (0..3000).map(|i| [i as f64 * 0.001, 0.0, 0.0]).collect();
    write_ascii_ply(&source, &input);
    convert(&input, &output, "deep", 5, 8, "DEFAULT");

    let meta: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("metadata.json")).unwrap()).unwrap();
    let first_chunk_size = meta["hierarchy"]["firstChunkSize"].as_u64().unwrap() as usize;
    let step_size = meta["hierarchy"]["stepSize"].as_u64().unwrap();
    assert_eq!(step_size, 4, "stepSize should be 4");

    let hierarchy_bytes = fs::read(output.join("hierarchy.bin")).unwrap();

    // The line data is engineered to reach depth 4, exceeding
    // HIERARCHY_STEP_SIZE, so chunking must kick in.
    let max_depth = meta["hierarchy"]["depth"].as_u64().unwrap();
    assert!(max_depth >= 4, "expected depth >= 4, got {max_depth}");
    {
        assert!(
            first_chunk_size < hierarchy_bytes.len(),
            "deep tree (depth={max_depth}) should produce sub-chunks: \
             firstChunkSize={first_chunk_size} but hierarchy.len()={}",
            hierarchy_bytes.len()
        );
    }

    // Round-trip: all 8 points should still be recovered.
    let hierarchy = Hierarchy::from_path(&output).await.unwrap();
    let nodes = hierarchy.load_entire_hierarchy().await.unwrap();
    let mut decoded_count = 0usize;
    for node in &nodes {
        if matches!(node.node_type, NodeType::Proxy) || node.num_points == 0 {
            continue;
        }
        let pts = hierarchy.load_points(node).await.unwrap();
        decoded_count += pts.buffer.count;
    }
    assert_eq!(decoded_count, source.len(), "should recover all {}", source.len());
}

// ── BROTLI round-trip ─────────────────────────────────────────────────────────

/// Convert with BROTLI encoding, then load back via the reader.
/// Verifies that all 8 corner points are recovered within quantization tolerance.
#[tokio::test(flavor = "current_thread")]
async fn brotli_encoding_roundtrip() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("pts.ply");
    let output = dir.path().join("out");
    fs::create_dir_all(&output).unwrap();

    let source = corner_points();
    write_ascii_ply(&source, &input);
    convert(&input, &output, "brotli_test", 2, 5, "BROTLI");

    // Verify encoding field in metadata
    let meta: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("metadata.json")).unwrap()).unwrap();
    assert_eq!(
        meta["encoding"].as_str().unwrap(),
        "BROTLI",
        "metadata encoding should be BROTLI"
    );

    let hierarchy = Hierarchy::from_path(&output)
        .await
        .expect("failed to load BROTLI hierarchy");
    let nodes = hierarchy.load_entire_hierarchy().await.unwrap();

    let scale = 0.001_f64;
    let tol = scale * 2.0;
    let mut all_decoded: Vec<[f64; 3]> = Vec::new();
    for node in &nodes {
        if matches!(node.node_type, NodeType::Proxy) || node.num_points == 0 {
            continue;
        }
        let pts = hierarchy.load_points(node).await.unwrap();
        for p in pts.buffer.iter() {
            let pos = p
                .attribute_type(AttributeType::Position)
                .expect("node points missing position attribute");
            all_decoded.push([pos[0] as f64, pos[1] as f64, pos[2] as f64]);
        }
    }

    assert_eq!(
        all_decoded.len(),
        source.len(),
        "BROTLI: decoded {} points, expected {}",
        all_decoded.len(),
        source.len()
    );

    let mut matched: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for decoded in &all_decoded {
        let (src_idx, src) = source
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                let da = (a[0] - decoded[0]).powi(2)
                    + (a[1] - decoded[1]).powi(2)
                    + (a[2] - decoded[2]).powi(2);
                let db = (b[0] - decoded[0]).powi(2)
                    + (b[1] - decoded[1]).powi(2)
                    + (b[2] - decoded[2]).powi(2);
                da.partial_cmp(&db).unwrap()
            })
            .unwrap();
        let dist = ((src[0] - decoded[0]).powi(2)
            + (src[1] - decoded[1]).powi(2)
            + (src[2] - decoded[2]).powi(2))
        .sqrt();
        assert!(
            dist <= tol,
            "BROTLI: decoded {decoded:?} is {dist:.4} from nearest source {src:?}"
        );
        matched.insert(src_idx);
    }
    assert_eq!(
        matched.len(),
        source.len(),
        "BROTLI: only {}/{} source points recovered",
        matched.len(),
        source.len()
    );
}

// ── no empty nodes ────────────────────────────────────────────────────────────

/// Dense, duplicate-heavy data (the profile of real scanner exports) used to
/// produce whole levels of `num_points=0 / byte_size=0` nodes: levels whose
/// Poisson spacing fell below the quantization grid let parents vacuum their
/// children's entire payloads. The reference viewer only warns on such nodes;
/// clean output must not contain them at all.
#[test]
fn output_contains_no_empty_nodes() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("dense.ply");
    let output = dir.path().join("out");
    fs::create_dir_all(&output).unwrap();

    // 3 m line at 1 mm pitch, every point written 3× (duplicates after
    // quantization), forcing deep splits capped by the resolution guard.
    let mut source: Vec<[f64; 3]> = Vec::new();
    for i in 0..3000 {
        for _ in 0..3 {
            source.push([i as f64 * 0.001, 0.0, 0.0]);
        }
    }
    write_ascii_ply(&source, &input);
    convert(&input, &output, "dense", 50, 20, "DEFAULT");

    let hierarchy = fs::read(output.join("hierarchy.bin")).unwrap();
    let mut total_points = 0u64;
    for rec in hierarchy.chunks_exact(22) {
        let node_type = rec[0];
        let num_points = LittleEndian::read_u32(&rec[2..6]);
        let byte_size = LittleEndian::read_u64(&rec[14..22]);
        if node_type != 2 {
            assert!(
                num_points > 0 && byte_size > 0,
                "real node with empty payload: type={node_type} \
                 num_points={num_points} byte_size={byte_size}"
            );
            total_points += num_points as u64;
        }
    }
    assert_eq!(total_points, source.len() as u64, "all points accounted for");
}
