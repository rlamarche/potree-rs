use potree::prelude::*;

#[tokio::main(flavor = "current_thread")]
pub async fn main() {
    tracing_subscriber::fmt::init();

    tracing::info!("Load pointcloud from local filesystem");
    let mut point_cloud = PotreePointCloud::from_url("file://assets/heidentor", ResourceLoader::new())
        .await
        .expect("Unable to load point cloud");


    tracing::info!("Successfuly loaded point cloud hierarchy.");

    point_cloud.load_entire_hierarchy().await.expect("Unable to load entire hierarchy");

    tracing::info!("Successfuly loaded entire point cloud hierarchy.");

}
