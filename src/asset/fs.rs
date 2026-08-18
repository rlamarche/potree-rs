#[cfg(not(feature = "tokio"))]
use std::io::SeekFrom;
use std::path::PathBuf;

use async_trait::async_trait;
use bytes::Bytes;
use thiserror::Error;
#[cfg(feature = "tokio")]
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::asset::PotreeAsset;

pub struct PotreeFsAsset {
    base_path: PathBuf,
}

impl PotreeFsAsset {
    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        Self {
            base_path: path.into(),
        }
    }
}

#[async_trait]
impl PotreeAsset for PotreeFsAsset {
    type Error = PotreeFsAssetError;

    async fn read_metadata(&self) -> Result<Bytes, Self::Error> {
        let metadata_path = self.base_path.join("metadata.json");

        #[cfg(feature = "tokio")]
        let bytes = tokio::fs::read(metadata_path).await?;

        #[cfg(not(feature = "tokio"))]
        let bytes = std::fs::read(metadata_path)?;

        Ok(bytes.into())
    }

    async fn read_hierarchy(&self, offset: u64, length: usize) -> Result<Bytes, Self::Error> {
        let hierarchy_path = self.base_path.join("hierarchy.bin");

        #[cfg(feature = "tokio")]
        let bytes = {
            use std::io::SeekFrom;

            let mut file = tokio::fs::File::open(hierarchy_path).await?;

            file.seek(SeekFrom::Start(offset)).await?;
            let mut bytes = vec![0; length];
            file.read_exact(&mut bytes).await?;

            bytes
        };

        #[cfg(not(feature = "tokio"))]
        let bytes = {
            use std::io::{Read, Seek};

            let mut file = std::fs::File::open(hierarchy_path)?;

            file.seek(SeekFrom::Start(offset))?;
            let mut bytes = vec![0; length];
            file.read_exact(&mut bytes)?;

            bytes
        };

        Ok(bytes.into())
    }

    async fn read_octree(&self, offset: u64, length: usize) -> Result<Bytes, Self::Error> {
        let octree_path = self.base_path.join("octree.bin");

        #[cfg(feature = "tokio")]
        let bytes = {
            use std::io::SeekFrom;

            let mut file = tokio::fs::File::open(octree_path).await?;

            file.seek(SeekFrom::Start(offset)).await?;
            let mut bytes = vec![0; length];
            file.read_exact(&mut bytes).await?;

            bytes
        };

        #[cfg(not(feature = "tokio"))]
        let bytes = {
            use std::io::{Read, Seek};

            let mut file = std::fs::File::open(octree_path)?;

            file.seek(SeekFrom::Start(offset))?;
            let mut bytes = vec![0; length];
            file.read_exact(&mut bytes)?;

            bytes
        };

        Ok(bytes.into())
    }
}

#[derive(Debug, Error)]
pub enum PotreeFsAssetError {
    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}
