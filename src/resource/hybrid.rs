use super::{ResourceClient, ResourceError};
use crate::resource::file::FileClient;
use async_trait::async_trait;
use std::collections::BTreeMap;

pub struct HybridClient<T: ResourceClient> {
    file_client: FileClient,
    inner: T,
}

impl<T: ResourceClient> HybridClient<T> {
    pub fn new(inner: T) -> Self {
        Self {
            file_client: FileClient,
            inner,
        }
    }
}

#[async_trait]
impl<T: ResourceClient + Send + Sync> ResourceClient for HybridClient<T> {
    async fn get(
        &self,
        url: &str,
        headers: Option<BTreeMap<String, String>>,
    ) -> Result<Vec<u8>, ResourceError> {
        if let Some(path) = url.strip_prefix("file://") {
            self.file_client.get(path, headers).await
        } else {
            self.inner.get(url, headers).await
        }
    }
}
