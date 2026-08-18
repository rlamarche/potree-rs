#[cfg(any(feature = "reqwest", feature = "ehttp"))]
pub mod http;

#[cfg(feature = "fs")]
pub mod fs;

#[cfg(any(feature = "reqwest", feature = "ehttp", feature = "fs"))]
pub mod url;

use async_trait::async_trait;
use bytes::Bytes;

#[async_trait]
pub trait PotreeAsset: Sync + Send {
    type Error: std::error::Error + Sync + Send + 'static;

    async fn read_metadata(&self) -> Result<Bytes, Self::Error>;

    async fn read_hierarchy(&self, offset: u64, length: usize) -> Result<Bytes, Self::Error>;

    async fn read_octree(&self, offset: u64, length: usize) -> Result<Bytes, Self::Error>;
}
