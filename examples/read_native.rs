use std::sync::Arc;
use potree::parse_metadatas;
use potree::resource::file::FileClient;
use potree::resource::Resource;

#[tokio::main(flavor = "current_thread")]
pub async fn main() {
    let file_client = Arc::new(FileClient);
    let metadata_resource = Resource::new("file://assets/heidentor/metadata.json", file_client.clone());
    let metadatas = parse_metadatas(metadata_resource).await.unwrap();

    let hierarchy_resource = Resource::new("file://assets/heidentor/hierarchy.bin", file_client.clone());
    // let root_node = metadatas.read_initial_hierarchy(&hierarchy_resource).await.unwrap();

    let initial_herarchy = metadatas.read_initial_hierarchy(&hierarchy_resource).await.unwrap();

    println!("Initial herarchy: {:#?}", initial_herarchy);

    let entire_hierarchy = metadatas.read_entire_hierarchy(&hierarchy_resource).await.unwrap();

    println!("Entire hierarchy: {:#?}", entire_hierarchy);
}
