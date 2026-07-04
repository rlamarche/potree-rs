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

use serde::Deserialize;
use std::fs;
use std::io::Write;
use tempfile::tempdir;

#[derive(Debug, Deserialize)]
struct Metadata {
    points: u64,
    hierarchy: Hierarchy,
    attributes: Vec<Attribute>,
}

#[derive(Debug, Deserialize)]
struct Hierarchy {
    #[serde(rename = "firstChunkSize")]
    first_chunk_size: u64,
    depth: u16,
}

#[derive(Debug, Deserialize)]
struct Attribute {
    name: String,
    size: u16,
}

fn write_ascii_ply(points: &[[f64; 3]], colors: Option<&[[u8; 3]]>, path: &std::path::Path) {
    let mut file = fs::File::create(path).unwrap();
    writeln!(file, "ply").unwrap();
    writeln!(file, "format ascii 1.0").unwrap();
    writeln!(file, "element vertex {}", points.len()).unwrap();
    writeln!(file, "property double x").unwrap();
    writeln!(file, "property double y").unwrap();
    writeln!(file, "property double z").unwrap();
    if colors.is_some() {
        writeln!(file, "property uchar red").unwrap();
        writeln!(file, "property uchar green").unwrap();
        writeln!(file, "property uchar blue").unwrap();
    }
    writeln!(file, "end_header").unwrap();
    for (i, p) in points.iter().enumerate() {
        if let Some(colors) = colors {
            let c = colors[i];
            writeln!(
                file,
                "{} {} {} {} {} {}",
                p[0], p[1], p[2], c[0], c[1], c[2]
            )
            .unwrap();
        } else {
            writeln!(file, "{} {} {}", p[0], p[1], p[2]).unwrap();
        }
    }
}

#[test]
fn streaming_creates_internal_payloads() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("tiny.ply");
    let output = dir.path().join("out");
    fs::create_dir_all(&output).unwrap();

    // 300 points 2 mm apart along a 0.6 m line: denser than the root Poisson
    // spacing (0.6/128 ≈ 4.7 mm), so the root sample rejects neighbours and
    // children keep real payloads (a sparse cloud would be fully absorbed
    // into the root and the tree pruned to a single node).
    let points: Vec<[f64; 3]> = (0..300).map(|i| [i as f64 * 0.002, 0.0, 0.0]).collect();
    write_ascii_ply(&points, None, &input);

    convert(&input, &output, "test", 1, 5, "DEFAULT");

    // Read hierarchy and metadata
    let hierarchy_bytes = fs::read(output.join("hierarchy.bin")).unwrap();
    assert!(hierarchy_bytes.len() >= 22);
    let metadata: Metadata =
        serde_json::from_slice(&fs::read(output.join("metadata.json")).unwrap()).unwrap();
    assert_eq!(metadata.points, points.len() as u64);

    // parse hierarchy records (22 bytes each)
    let num_nodes = hierarchy_bytes.len() / 22;
    assert!(num_nodes >= 2); // at least root + some children

    let mut records = Vec::new();
    for i in 0..num_nodes {
        let start = i * 22;
        let r#type = hierarchy_bytes[start];
        let child_mask = hierarchy_bytes[start + 1];
        let num_points = LittleEndian::read_u32(&hierarchy_bytes[start + 2..start + 6]);
        let byte_size = LittleEndian::read_u64(&hierarchy_bytes[start + 14..start + 22]);
        records.push((r#type, child_mask, num_points, byte_size));
    }

    let root = records[0];
    assert_eq!(root.0, 0); // internal
    assert!(root.1 > 0); // has children

    // root children mask should be subset of 0..7
    assert!(root.1 & !0b1111_1111 == 0);

    let mut total_node_sum = 0u64;
    let mut internal_payload = 0u64;
    for record in &records {
        if record.0 == 2 {
            // proxy records duplicate their real node's num_points in the
            // next hierarchy chunk — don't double count
            continue;
        }
        total_node_sum += record.2 as u64;
        if record.0 == 0 {
            internal_payload += record.3;
        }
    }
    // Every input point lives in exactly one node's payload (no duplicates).
    assert_eq!(
        total_node_sum, metadata.points,
        "sum of all node num_points should equal total input points"
    );
    assert!(
        internal_payload > 0,
        "internal nodes should have sampled payload bytes"
    );

    // octree.bin size should equal sum of byte_size across real nodes
    // (proxy records carry hierarchy-chunk sizes, not octree bytes)
    let octree_size = fs::metadata(output.join("octree.bin")).unwrap().len();
    let sum_sizes: u64 = records.iter().filter(|r| r.0 != 2).map(|r| r.3).sum();
    assert_eq!(octree_size, sum_sizes);
}
