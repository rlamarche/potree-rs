use bytes::Bytes;
use thiserror::Error;

use super::BlockingPotreeAsset;
use crate::metadata::Metadata;

pub struct BlockingPotreeHttpAsset {
    base_url: String,
    #[cfg(feature = "blocking_reqwest")]
    client: reqwest::blocking::Client,
}

impl BlockingPotreeHttpAsset {
    pub fn from_url(url: &str) -> Self {
        let base_url = normalize_base_url(url);

        Self {
            base_url,
            client: reqwest::blocking::Client::new(),
        }
    }
}

impl BlockingPotreeAsset for BlockingPotreeHttpAsset {
    type Error = PotreeHttpAssetError;

    fn read_metadata(&self) -> Result<Metadata, Self::Error> {
        let metadata_url = asset_url(&self.base_url, "metadata.json");

        Ok(self
            .client
            .get(metadata_url)
            .send()?
            .error_for_status()?
            .json()?)
    }

    fn read_hierarchy(&self, offset: u64, length: usize) -> Result<Bytes, Self::Error> {
        let end = offset
            .checked_add(length as u64)
            .map(|v| v - 1)
            .ok_or_else(|| Self::Error::Other("Range overflow".into()))?;
        let range_value = format!("bytes={}-{}", offset, end);

        let hierarchy_url = asset_url(&self.base_url, "hierarchy.bin");

        Ok(self
            .client
            .get(hierarchy_url)
            .header("range", range_value)
            .send()?
            .error_for_status()?
            .bytes()?)
    }

    fn read_octree(&self, offset: u64, length: usize) -> Result<Bytes, Self::Error> {
        let end = offset
            .checked_add(length as u64)
            .map(|v| v - 1)
            .ok_or_else(|| Self::Error::Other("Range overflow".into()))?;
        let range_value = format!("bytes={}-{}", offset, end);

        let octree_url = asset_url(&self.base_url, "octree.bin");

        Ok(self
            .client
            .get(octree_url)
            .header("range", range_value)
            .send()?
            .error_for_status()?
            .bytes()?)
    }
}

fn normalize_base_url(url: &str) -> String {
    if let Ok(mut parsed) = url::Url::parse(url) {
        let path = strip_known_asset_filename(parsed.path()).to_string();
        parsed.set_path(&path);
        return parsed.to_string().trim_end_matches('/').to_string();
    }

    let (path, suffix) = split_url_suffix(url);
    format!(
        "{}{}",
        strip_known_asset_filename(path).trim_end_matches('/'),
        suffix
    )
}

fn asset_url(base_url: &str, filename: &str) -> String {
    if let Ok(mut parsed) = url::Url::parse(base_url) {
        let path = parsed.path().trim_end_matches('/');
        parsed.set_path(&format!("{path}/{filename}"));
        return parsed.to_string();
    }

    let (path, suffix) = split_url_suffix(base_url);
    format!("{}/{filename}{suffix}", path.trim_end_matches('/'))
}

fn strip_known_asset_filename(path: &str) -> &str {
    let path = path.trim_end_matches('/');
    match path.rsplit_once('/') {
        Some((base, "metadata.json" | "hierarchy.bin" | "octree.bin")) => base,
        _ => path,
    }
}

fn split_url_suffix(url: &str) -> (&str, &str) {
    match url.find(|c| c == '?' || c == '#') {
        Some(index) => url.split_at(index),
        None => (url, ""),
    }
}

#[derive(Debug, Error)]
pub enum PotreeHttpAssetError {
    #[error("Reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),

    #[error("Unsupported scheme: {0}")]
    Unsupported(String),
}
