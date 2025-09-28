use super::{ResourceClient, ResourceError};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::io::SeekFrom;
#[cfg(all(not(feature = "tokio"), not(feature = "async-fs")))]
use std::io::{Read, Seek};

#[cfg(feature = "async-fs")]
use futures::{AsyncReadExt, AsyncSeekExt};
#[cfg(feature = "tokio")]
use tokio::io::{AsyncReadExt, AsyncSeekExt};

#[derive(Clone, Debug)]
pub struct FileClient;

#[async_trait]
impl ResourceClient for FileClient {
    async fn get(
        &self,
        url: &str,
        _headers: Option<BTreeMap<String, String>>,
    ) -> Result<Vec<u8>, ResourceError> {
        let path = {
            if url.starts_with("file://") {
                url.strip_prefix("file://").unwrap()
            } else {
                url
            }
        };
        #[cfg(feature = "tokio")]
        let bytes = tokio::fs::read(path).await?;

        #[cfg(not(feature = "tokio"))]
        let bytes = std::fs::read(path)?;

        Ok(bytes)
    }

    async fn get_range(
        &self,
        url: &str,
        offset: u64,
        length: usize,
        _headers: Option<BTreeMap<String, String>>,
    ) -> Result<Vec<u8>, ResourceError> {
        let path = {
            if url.starts_with("file://") {
                url.strip_prefix("file://").unwrap()
            } else {
                url
            }
        };
        #[cfg(feature = "tokio")]
        {
            let mut file = tokio::fs::File::open(path).await?;
            file.seek(SeekFrom::Start(offset)).await?;
            let mut bytes = vec![0; length];
            file.read_exact(&mut bytes).await?;
            Ok(bytes)
        }

        #[cfg(feature = "async-fs")]
        {
            let mut file = async_fs::File::open(path).await?;
            file.seek(SeekFrom::Start(offset)).await?;
            let mut bytes = vec![0; length];
            file.read_exact(&mut bytes).await?;
            Ok(bytes)
        }

        #[cfg(all(not(feature = "tokio"), not(feature = "async-fs")))]
        {
            let mut file = std::fs::File::open(path)?;
            file.seek(SeekFrom::Start(offset))?;
            let mut bytes = vec![0; length];
            file.read_exact(&mut bytes)?;
            Ok(bytes)
        }
    }
}
