use super::{ResourceClient, ResourceError};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::io::SeekFrom;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

#[derive(Clone)]
pub struct FileClient;

#[async_trait]
impl ResourceClient for FileClient {
    async fn get(
        &self,
        url: &str,
        headers: Option<BTreeMap<String, String>>,
    ) -> Result<Vec<u8>, ResourceError> {
        if let Some(path) = url.strip_prefix("file://") {
            let bytes = tokio::fs::read(path).await?;
            Ok(bytes)
        } else {
            Err(ResourceError::Unsupported(
                "This client supports only file:// urls.".to_string(),
            ))
        }
    }

    async fn get_range(&self, url: &str, offset: u64, length: usize, headers: Option<BTreeMap<String, String>>) -> Result<Vec<u8>, ResourceError> {
        if let Some(path) = url.strip_prefix("file://") {
            let mut file = tokio::fs::File::open(path).await?;
            file.seek(SeekFrom::Start(offset)).await?;
            let mut bytes = vec![0; length];
            file.read_exact(&mut bytes).await?;
            Ok(bytes)
        } else {
            Err(ResourceError::Unsupported(
                "This client supports only file:// urls.".to_string(),
            ))
        }
    }
}
