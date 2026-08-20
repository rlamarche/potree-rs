use potree::{asset::http::PotreeHttpAsset, hierarchy::HierarchyAsync, prelude::*};

#[tokio::main(flavor = "current_thread")]
pub async fn main() -> Result<(), Box<dyn std::error::Error + 'static>> {
    tracing_subscriber::fmt::init();

    let url: &str = "https://potree.org/pointclouds/heidentor/";

    let asset = PotreeHttpAsset::from_url(url);

    tracing::info!("Load pointcloud from url {}", url);
    let hierarchy = Hierarchy::load(asset)
        .await
        .expect("Unable to load point cloud");

    let nodes = hierarchy
        .load_initial_hierarchy()
        .await
        .expect("Unable to load initial hierarchy");
    tracing::info!(
        "Successfuly loaded point cloud hierarchy with {} nodes",
        nodes.len()
    );

    let points = hierarchy
        .load_points(&nodes[0])
        .await
        .expect("Unable to load points");
    tracing::info!(
        "Successfuly loaded {} points for the root node",
        points.buffer.count
    );

    Ok(())
}
