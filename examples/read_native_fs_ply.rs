//! Load a local PLY, convert to Potree buffers in-memory, and read it via the existing loader.
//!
//! Run with:
//! `cargo run --example read_native_fs_ply --features="convert slab tokio_dev" -- <path/to/file.ply> [scale]`

use potree::convert::ply_loader::load_ply_positions;
use potree::octree::node::{iter_one_bits, NodeType};
use potree::point_cloud::PointCloudAsync;
use potree::prelude::*;
use std::collections::VecDeque;
use std::env;
use std::path::PathBuf;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let mut args = env::args().skip(1);
    let ply_path = PathBuf::from(
        args.next()
            .expect("Usage: read_native_fs_ply <path/to/file.ply> [scale]"),
    );
    let scale: f64 = args
        .next()
        .as_deref()
        .map(|s| s.parse().expect("scale must be a number"))
        .unwrap_or(0.001);

    tracing::info!("Loading PLY from {}", ply_path.display());
    let ply = load_ply_positions(&ply_path)?;
    let name = ply_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("ply_cloud");

    tracing::info!("Converting to Potree buffers in memory");
    let buffers = ply
        .into_potree_builder()
        .name(name.to_string())
        .target_scale([scale, scale, scale])
        .build()?;

    let mut point_cloud = PointCloud::from_buffers(
        buffers.metadata_json,
        buffers.hierarchy,
        buffers.octree,
    )
    .await?;

    let mut queue: VecDeque<_> = VecDeque::from([point_cloud.octree().root_id()]);
    let mut visited = 0usize;

    while let Some(node_id) = queue.pop_front() {
        let node = point_cloud
            .octree()
            .node(node_id)
            .cloned()
            .expect("node missing from octree");

        // If node is a proxy, load its hierarchy first
        if matches!(node.node_type, NodeType::Proxy) {
            point_cloud.load_hierarchy(node_id).await?;
            let node = point_cloud
                .octree()
                .node(node_id)
                .cloned()
                .expect("node missing after load");
            enqueue_children(&node, &mut queue);
            continue;
        }

        if node.node_type.has_points() {
            match point_cloud.load_points(node_id).await {
                Ok(points) => {
                    tracing::info!("Node {}: loaded {} points", node.name, points.points.len())
                }
                Err(err) => tracing::error!("Failed to load points for {}: {err}", node.name),
            }
        }

        enqueue_children(&node, &mut queue);
        visited += 1;
    }

    tracing::info!("Visited {} nodes", visited);

    Ok(())
}

fn enqueue_children(node: &potree::octree::node::OctreeNode, queue: &mut VecDeque<usize>) {
    for child_idx in iter_one_bits(node.children_mask) {
        queue.push_back(node.children[child_idx as usize]);
    }
}
