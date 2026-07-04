use byteorder::{BigEndian, LittleEndian, WriteBytesExt};
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

use std::fs;
use std::io::Write;
use tempfile::tempdir;

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
fn streaming_builds_hierarchy_and_offsets() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("tiny.ply");
    let output = dir.path().join("out");
    fs::create_dir_all(&output).unwrap();

    // 8 points, forced split with small max_points_per_node
    let points = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
        [0.0, 1.0, 1.0],
        [1.0, 1.0, 1.0],
    ];
    write_ascii_ply(&points, None, &input);

    convert(&input, &output, "test", 2, 4, "DEFAULT");

    let hierarchy = fs::read(output.join("hierarchy.bin")).unwrap();
    // first node
    assert_eq!(hierarchy.len() % 22, 0);
    // octree exists
    let octree = fs::metadata(output.join("octree.bin")).unwrap();
    assert!(octree.len() > 0);
    // metadata exists
    assert!(output.join("metadata.json").exists());
}

/// Write an ASCII PLY with extra `intensity` (uint16) and `classification` (uint8) properties.
fn write_ascii_ply_with_extras(
    points: &[[f64; 3]],
    intensities: &[u16],
    classifications: &[u8],
    path: &std::path::Path,
) {
    let mut file = fs::File::create(path).unwrap();
    writeln!(file, "ply").unwrap();
    writeln!(file, "format ascii 1.0").unwrap();
    writeln!(file, "element vertex {}", points.len()).unwrap();
    writeln!(file, "property float x").unwrap();
    writeln!(file, "property float y").unwrap();
    writeln!(file, "property float z").unwrap();
    writeln!(file, "property ushort intensity").unwrap();
    writeln!(file, "property uchar classification").unwrap();
    writeln!(file, "end_header").unwrap();
    for (i, p) in points.iter().enumerate() {
        writeln!(
            file,
            "{} {} {} {} {}",
            p[0], p[1], p[2], intensities[i], classifications[i]
        )
        .unwrap();
    }
}

#[test]
fn extra_attributes_stored_in_metadata() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("extras.ply");
    let output = dir.path().join("out");
    fs::create_dir_all(&output).unwrap();

    let points = [[0.0f64, 0.0, 0.0], [1.0, 1.0, 1.0], [2.0, 2.0, 2.0]];
    let intensities = [100u16, 200, 300];
    let classes = [1u8, 2, 3];
    write_ascii_ply_with_extras(&points, &intensities, &classes, &input);

    convert(&input, &output, "extras_test", 10, 4, "DEFAULT");

    let meta: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("metadata.json")).unwrap()).unwrap();

    let attrs = meta["attributes"].as_array().unwrap();
    let names: Vec<&str> = attrs.iter().map(|a| a["name"].as_str().unwrap()).collect();

    assert!(names.contains(&"position"), "missing position attribute");
    assert!(names.contains(&"intensity"), "missing intensity attribute");
    assert!(
        names.contains(&"classification"),
        "missing classification attribute"
    );

    // intensity should be uint16 (2 bytes)
    let intensity_attr = attrs.iter().find(|a| a["name"] == "intensity").unwrap();
    assert_eq!(intensity_attr["type"], "uint16");
    assert_eq!(intensity_attr["size"], 2);

    // classification should be uint8 (1 byte)
    let class_attr = attrs
        .iter()
        .find(|a| a["name"] == "classification")
        .unwrap();
    assert_eq!(class_attr["type"], "uint8");
    assert_eq!(class_attr["size"], 1);

    // min/max reflect the observed data range, not the scalar type's range
    assert_eq!(intensity_attr["min"][0], 100.0);
    assert_eq!(intensity_attr["max"][0], 300.0);
    assert_eq!(class_attr["min"][0], 1.0);
    assert_eq!(class_attr["max"][0], 3.0);

    // record size = 12 (position) + 2 (intensity) + 1 (classification) = 15 bytes per point × 3 points
    let octree_size = fs::metadata(output.join("octree.bin")).unwrap().len();
    assert_eq!(
        octree_size,
        15 * 3,
        "expected 15 bytes/point × 3 points in octree.bin"
    );
}

/// CloudCompare-style `scalar_<Name>` properties are normalized to canonical
/// Potree attribute names so viewers recognize them.
#[test]
fn cloudcompare_scalar_names_are_normalized() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("pts.ply");
    let output = dir.path().join("out");
    fs::create_dir_all(&output).unwrap();

    let mut file = fs::File::create(&input).unwrap();
    writeln!(file, "ply").unwrap();
    writeln!(file, "format ascii 1.0").unwrap();
    writeln!(file, "element vertex 2").unwrap();
    writeln!(file, "property float x").unwrap();
    writeln!(file, "property float y").unwrap();
    writeln!(file, "property float z").unwrap();
    writeln!(file, "property float scalar_Intensity").unwrap();
    writeln!(file, "property float scalar_confidence").unwrap();
    writeln!(file, "end_header").unwrap();
    writeln!(file, "0 0 0 0.25 0.5").unwrap();
    writeln!(file, "1 1 1 0.75 0.9").unwrap();
    drop(file);

    convert(&input, &output, "cc_test", 10, 4, "DEFAULT");

    let meta: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("metadata.json")).unwrap()).unwrap();
    let attrs = meta["attributes"].as_array().unwrap();
    let names: Vec<&str> = attrs.iter().map(|a| a["name"].as_str().unwrap()).collect();

    // canonical Potree name; prefix stripped from unknown scalar fields
    assert!(names.contains(&"intensity"), "attrs: {names:?}");
    assert!(names.contains(&"confidence"), "attrs: {names:?}");

    // observed range, not the [0,1] float placeholder
    let intensity = attrs.iter().find(|a| a["name"] == "intensity").unwrap();
    assert_eq!(intensity["min"][0], 0.25);
    assert_eq!(intensity["max"][0], 0.75);
}

/// Write a binary big-endian PLY with 4 points.
fn write_binary_be_ply(points: &[[f32; 3]], path: &std::path::Path) {
    let mut file = fs::File::create(path).unwrap();
    write!(file, "ply\n").unwrap();
    write!(file, "format binary_big_endian 1.0\n").unwrap();
    write!(file, "element vertex {}\n", points.len()).unwrap();
    write!(file, "property float x\n").unwrap();
    write!(file, "property float y\n").unwrap();
    write!(file, "property float z\n").unwrap();
    write!(file, "end_header\n").unwrap();
    for p in points {
        file.write_f32::<BigEndian>(p[0]).unwrap();
        file.write_f32::<BigEndian>(p[1]).unwrap();
        file.write_f32::<BigEndian>(p[2]).unwrap();
    }
}

#[test]
fn binary_big_endian_ply_converts() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("be.ply");
    let output = dir.path().join("out");
    fs::create_dir_all(&output).unwrap();

    let points = [
        [0.0f32, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [1.0, 1.0, 0.0],
    ];
    write_binary_be_ply(&points, &input);

    convert(&input, &output, "be_test", 10, 4, "DEFAULT");

    // hierarchy.bin and octree.bin must exist with valid data
    let hierarchy = fs::read(output.join("hierarchy.bin")).unwrap();
    assert_eq!(hierarchy.len() % 22, 0);
    let octree_size = fs::metadata(output.join("octree.bin")).unwrap().len();
    // 4 points × 12 bytes each = 48 bytes (no color)
    assert_eq!(octree_size, 48, "expected 4 points × 12 bytes = 48 bytes");
}

/// Compare little-endian and big-endian PLY of the same points — octree.bin must be identical.
#[test]
fn big_endian_matches_little_endian_output() {
    let points_f32 = [[0.1f32, 0.2, 0.3], [0.9, 0.8, 0.7]];

    let dir = tempdir().unwrap();

    // Write big-endian binary PLY
    let be_input = dir.path().join("be.ply");
    write_binary_be_ply(&points_f32, &be_input);

    // Write little-endian binary PLY
    let le_input = dir.path().join("le.ply");
    {
        let mut file = fs::File::create(&le_input).unwrap();
        write!(file, "ply\n").unwrap();
        write!(file, "format binary_little_endian 1.0\n").unwrap();
        write!(file, "element vertex {}\n", points_f32.len()).unwrap();
        write!(file, "property float x\n").unwrap();
        write!(file, "property float y\n").unwrap();
        write!(file, "property float z\n").unwrap();
        write!(file, "end_header\n").unwrap();
        for p in &points_f32 {
            file.write_f32::<LittleEndian>(p[0]).unwrap();
            file.write_f32::<LittleEndian>(p[1]).unwrap();
            file.write_f32::<LittleEndian>(p[2]).unwrap();
        }
    }

    let be_out = dir.path().join("be_out");
    let le_out = dir.path().join("le_out");
    fs::create_dir_all(&be_out).unwrap();
    fs::create_dir_all(&le_out).unwrap();

    convert(&be_input, &be_out, "be", 10, 4, "DEFAULT");
    convert(&le_input, &le_out, "le", 10, 4, "DEFAULT");

    let be_octree = fs::read(be_out.join("octree.bin")).unwrap();
    let le_octree = fs::read(le_out.join("octree.bin")).unwrap();
    assert_eq!(
        be_octree, le_octree,
        "big-endian and little-endian PLY must produce identical octree.bin"
    );
}
