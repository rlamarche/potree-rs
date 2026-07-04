//! Dump a Potree directory as text: one line per node (name, level, type,
//! num_points), then all decoded point positions. Used to diff converter
//! outputs (e.g. potree-rs converter vs the C++ PotreeConverter).
//!
//! Usage: cargo run --release --features fs,tokio_dev --example dump_potree -- <potree_dir>
//!
//! Output format:
//!   node <name> <level> <type> <num_points>
//!   ...
//!   point <x> <y> <z>
//!   ...

use potree::hierarchy::HierarchyAsync;
use potree::octree::node::NodeType;
use potree::point::AttributeType;
use potree::prelude::*;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args().nth(1).expect("usage: dump_potree <potree_dir>");

    let hierarchy = Hierarchy::from_path(&dir).await?;
    let nodes = hierarchy.load_entire_hierarchy().await?;

    let mut out = String::new();
    for node in &nodes {
        let ty = match node.node_type {
            NodeType::Node => "node",
            NodeType::Leaf => "leaf",
            NodeType::Proxy => "proxy",
            NodeType::Other(_) => "other",
        };
        out.push_str(&format!(
            "node {} {} {} {}\n",
            node.name, node.level, ty, node.num_points
        ));
    }

    for node in &nodes {
        if matches!(node.node_type, NodeType::Proxy) || node.num_points == 0 {
            continue;
        }
        let points = hierarchy.load_points(node).await?;
        for p in points.buffer.iter() {
            let pos = p
                .attribute_type(AttributeType::Position)
                .expect("missing position attribute");
            match p.attribute_type(AttributeType::Rgb) {
                Some(rgb) => out.push_str(&format!(
                    "point {} {} {} {} {} {}\n",
                    pos[0], pos[1], pos[2], rgb[0], rgb[1], rgb[2]
                )),
                None => out.push_str(&format!("point {} {} {}\n", pos[0], pos[1], pos[2])),
            }
        }
    }

    print!("{out}");
    Ok(())
}
