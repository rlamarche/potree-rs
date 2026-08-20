#[cfg(any(feature = "reqwest", feature = "ehttp"))]
pub mod http;

#[cfg(feature = "fs")]
pub mod fs;

#[cfg(any(feature = "reqwest", feature = "ehttp", feature = "fs"))]
pub mod url;

use bytes::Bytes;

use crate::utils::ConditionalSendFuture;

pub trait PotreeAsset: Sync + Send {
    type Error: std::error::Error + Sync + Send + 'static;

    fn read_metadata(&self) -> impl ConditionalSendFuture<Output = Result<Bytes, Self::Error>>;

    fn read_hierarchy(
        &self,
        offset: u64,
        length: usize,
    ) -> impl ConditionalSendFuture<Output = Result<Bytes, Self::Error>>;

    fn read_octree(
        &self,
        offset: u64,
        length: usize,
    ) -> impl ConditionalSendFuture<Output = Result<Bytes, Self::Error>>;
}
