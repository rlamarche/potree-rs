pub mod ehttp;

#[cfg(feature = "fs")]
pub mod file;

#[cfg(feature = "reqwest")]
pub mod reqwest;

#[cfg(all(feature = "fs", feature = "reqwest"))]
pub mod hybrid;

#[cfg(feature = "wasm")]
pub mod ehttp_local;

use async_trait::async_trait;
use futures::{AsyncRead, AsyncSeek, Stream};
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;
use std::io::SeekFrom;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

#[async_trait]
pub trait ResourceClient: Send + Sync {
    async fn get(
        &self,
        url: &str,
        headers: Option<BTreeMap<String, String>>,
    ) -> Result<Vec<u8>, ResourceError>;

    async fn get_range(
        &self,
        url: &str,
        offset: u64,
        length: usize,
        headers: Option<BTreeMap<String, String>>,
    ) -> Result<Vec<u8>, ResourceError> {
        // Compute the Range header
        let end = offset
            .checked_add(length as u64)
            .map(|v| v - 1)
            .ok_or_else(|| ResourceError::Other("Range overflow".into()))?;
        let range_value = format!("bytes={}-{}", offset, end);

        // Merge headers
        let mut all_headers = headers.unwrap_or_default();
        all_headers.insert("Range".to_string(), range_value);

        // Call get() with Range header
        self.get(url, Some(all_headers)).await
    }

    async fn get_json<T: DeserializeOwned + Send>(
        &self,
        url: &str,
        headers: Option<BTreeMap<String, String>>,
    ) -> Result<T, ResourceError> {
        let bytes = self.get(url, headers).await?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}

#[async_trait]
impl<C: ResourceClient> ResourceClient for Arc<C> {
    async fn get(
        &self,
        url: &str,
        headers: Option<BTreeMap<String, String>>,
    ) -> Result<Vec<u8>, ResourceError> {
        (**self).get(url, headers).await
    }

    async fn get_range(
        &self,
        url: &str,
        offset: u64,
        length: usize,
        headers: Option<BTreeMap<String, String>>,
    ) -> Result<Vec<u8>, ResourceError> {
        (**self).get_range(url, offset, length, headers).await
    }

    async fn get_json<T: DeserializeOwned + Send>(
        &self,
        url: &str,
        headers: Option<BTreeMap<String, String>>,
    ) -> Result<T, ResourceError> {
        (**self).get_json(url, headers).await
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResourceError {
    #[error("Network error: {0}")]
    Network(String),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Unexpected HTTP status code: {0}")]
    Status(u16),

    #[error("File error: {0}")]
    File(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),

    #[error("Unsupported scheme: {0}")]
    Unsupported(String),
}

pub struct Resource<C: ResourceClient> {
    url: String,
    client: C,
    pos: Option<u64>,
}

impl<C: ResourceClient> Resource<C> {
    pub fn new(url: &str, client: C) -> Self {
        Self {
            url: url.to_string(),
            client,
            pos: None,
        }
    }

    pub async fn get(
        &self,
        headers: Option<BTreeMap<String, String>>,
    ) -> Result<Vec<u8>, ResourceError> {
        self.client.get(&self.url, headers).await
    }
    pub async fn get_range(
        &self,
        offset: u64,
        length: usize,
        headers: Option<BTreeMap<String, String>>,
    ) -> Result<Vec<u8>, ResourceError> {
        self.client
            .get_range(&self.url, offset, length, headers)
            .await
    }

    pub async fn get_json<T: DeserializeOwned + Send>(
        &self,
        headers: Option<BTreeMap<String, String>>,
    ) -> Result<T, ResourceError> {
        self.client.get_json(&self.url, headers).await
    }
}

impl<C: ResourceClient> Unpin for Resource<C> {}

impl<C: ResourceClient> AsyncSeek for Resource<C> {
    fn poll_seek(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        pos: SeekFrom,
    ) -> Poll<std::io::Result<u64>> {
        let this = self.get_mut();

        match pos {
            SeekFrom::Start(pos) => this.pos = Some(pos),
            SeekFrom::End(_) => {}
            SeekFrom::Current(_) => {}
        }

        Poll::Ready(Ok(this.pos.unwrap_or(0)))
    }
}

impl<C: ResourceClient> AsyncRead for Resource<C> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        todo!()
    }
}
