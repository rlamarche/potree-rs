use bytes::Bytes;
use thiserror::Error;
use url::Url;

#[cfg(feature = "fs")]
use crate::asset::fs::PotreeFsAsset;
#[cfg(any(feature = "reqwest", feature = "ehttp"))]
use crate::asset::http::PotreeHttpAsset;
use crate::asset::PotreeAsset;

pub struct PotreeUrlAsset {
    inner: InnerAsset,
}

pub enum InnerAsset {
    #[cfg(any(feature = "reqwest", feature = "ehttp"))]
    Http(PotreeHttpAsset),
    #[cfg(feature = "fs")]
    Fs(PotreeFsAsset),
}

impl PotreeUrlAsset {
    pub fn from_url(url: &str) -> Result<Self, PotreeUrlAssetError> {
        if url.contains("://") {
            let parsed_url = Url::parse(url)?;
            let scheme = parsed_url.scheme();

            match scheme {
                #[cfg(any(feature = "ehttp", feature = "reqwest"))]
                "http" | "https" => Ok(Self {
                    inner: InnerAsset::Http(PotreeHttpAsset::from_url(url)),
                }),
                #[cfg(feature = "fs")]
                "file" => Ok(Self {
                    inner: InnerAsset::Fs(PotreeFsAsset::from_path(parsed_url.path())),
                }),
                _ => Err(PotreeUrlAssetError::Unsupported(format!(
                    "Unknown scheme {}",
                    scheme
                ))),
            }
        } else {
            #[cfg(feature = "fs")]
            {
                Ok(Self {
                    inner: InnerAsset::Fs(PotreeFsAsset::from_path(url)),
                })
            }

            #[cfg(all(not(feature = "fs"), any(feature = "reqwest", feature = "ehttp")))]
            {
                Ok(Self {
                    inner: InnerAsset::Http(PotreeHttpAsset::from_url(url)),
                })
            }

            #[cfg(all(not(feature = "fs"), not(feature = "reqwest"), not(feature = "ehttp"),))]
            Err(PotreeUrlAssetError::Unsupported(
                "Relative urls are not supported in this configuration.".to_string(),
            ))
        }
    }
}

impl PotreeAsset for PotreeUrlAsset {
    type Error = PotreeUrlAssetError;

    async fn read_metadata(&self) -> Result<Bytes, Self::Error> {
        match &self.inner {
            #[cfg(any(feature = "reqwest", feature = "ehttp"))]
            InnerAsset::Http(potree_http_asset) => Ok(potree_http_asset
                .read_metadata()
                .await
                .map_err(|err| Self::Error::Read(err.to_string()))?),
            #[cfg(feature = "fs")]
            InnerAsset::Fs(potree_fs_asset) => Ok(potree_fs_asset
                .read_metadata()
                .await
                .map_err(|err| Self::Error::Read(err.to_string()))?),
        }
    }

    async fn read_hierarchy(&self, offset: u64, length: usize) -> Result<Bytes, Self::Error> {
        match &self.inner {
            #[cfg(any(feature = "reqwest", feature = "ehttp"))]
            InnerAsset::Http(potree_http_asset) => Ok(potree_http_asset
                .read_hierarchy(offset, length)
                .await
                .map_err(|err| Self::Error::Read(err.to_string()))?),
            #[cfg(feature = "fs")]
            InnerAsset::Fs(potree_fs_asset) => Ok(potree_fs_asset
                .read_hierarchy(offset, length)
                .await
                .map_err(|err| Self::Error::Read(err.to_string()))?),
        }
    }

    async fn read_octree(&self, offset: u64, length: usize) -> Result<Bytes, Self::Error> {
        match &self.inner {
            #[cfg(any(feature = "reqwest", feature = "ehttp"))]
            InnerAsset::Http(potree_http_asset) => Ok(potree_http_asset
                .read_octree(offset, length)
                .await
                .map_err(|err| Self::Error::Read(err.to_string()))?),
            #[cfg(feature = "fs")]
            InnerAsset::Fs(potree_fs_asset) => Ok(potree_fs_asset
                .read_octree(offset, length)
                .await
                .map_err(|err| Self::Error::Read(err.to_string()))?),
        }
    }
}

#[derive(Debug, Error)]
pub enum PotreeUrlAssetError {
    #[error("Read error: {0}")]
    Read(String),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Unable to parse url: {0}")]
    Url(#[from] url::ParseError),

    #[error("{0}")]
    Other(String),

    #[error("Unsupported scheme: {0}")]
    Unsupported(String),
}
