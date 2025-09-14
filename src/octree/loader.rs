use thiserror::Error;
use crate::{AttributeMetadata, Metadata};
use crate::resource::ResourceClient;
use crate::octree::node::OctreeNodeInner;


#[derive(Error, Debug)]
pub enum LoaderError {
    #[error("Resource error: {0}")]
    Resource(#[from] crate::resource::ResourceError),
}

pub async fn load<C: ResourceClient>(url: &str, resource_client: C) -> Result<(), LoaderError> {
    let metadata: Metadata = resource_client.get_json(url, None).await?;



    Ok(())
}

fn parse_attributes(attributes: Vec<AttributeMetadata>) {

}


pub struct NodeLoader {
    url: String,
}

impl NodeLoader {
    pub fn new(url: String) -> NodeLoader {
        Self {
            url,
        }
    }

    async fn load(node: &mut OctreeNodeInner) {

    }
}

