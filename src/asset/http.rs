use async_trait::async_trait;
use bytes::Bytes;
#[cfg(target_arch = "wasm32")]
use ehttp::Mode;
#[cfg(feature = "ehttp_local")]
use futures::{
    channel::{
        mpsc::{UnboundedReceiver, UnboundedSender},
        oneshot,
    },
    StreamExt,
};
use std::collections::BTreeMap;
use thiserror::Error;
#[cfg(feature = "ehttp_local")]
use wasm_bindgen_futures::spawn_local;

use crate::{asset::PotreeAsset, metadata::Metadata};

pub struct PotreeHttpAsset {
    base_url: String,
    #[cfg(feature = "reqwest")]
    client: reqwest::Client,
    #[cfg(feature = "ehttp_local")]
    #[allow(unused)]
    tx_request: UnboundedSender<Request>,
}

#[cfg(feature = "ehttp_local")]
struct Request {
    tx_response: oneshot::Sender<Result<Bytes, <PotreeHttpAsset as PotreeAsset>::Error>>,
    url: String,
    headers: Option<BTreeMap<String, String>>,
}

impl PotreeHttpAsset {
    pub fn from_url(url: &str) -> Self {
        #[cfg(feature = "ehttp_local")]
        let tx_request = {
            use futures::channel::mpsc::unbounded;
            use wasm_bindgen_futures::spawn_local;

            let (tx_request, rx_request) = unbounded();

            spawn_local(async move {
                tracing::info!("Starting requests");
                process_requests(rx_request)
                    .await
                    .expect("Failed to process requests");
            });

            tx_request
        };

        let base_url = normalize_base_url(url);

        Self {
            base_url,
            #[cfg(feature = "reqwest")]
            client: reqwest::Client::new(),
            #[cfg(feature = "ehttp_local")]
            tx_request,
        }
    }
}

#[async_trait]
impl PotreeAsset for PotreeHttpAsset {
    type Error = PotreeHttpAssetError;

    async fn read_metadata(&self) -> Result<Metadata, Self::Error> {
        let metadata_url = asset_url(&self.base_url, "metadata.json");
        let data = self.get(metadata_url.as_str(), None).await?;

        Ok(serde_json::from_slice(&data)?)
    }

    async fn read_hierarchy(&self, offset: u64, length: usize) -> Result<Bytes, Self::Error> {
        let hierarchy_url = asset_url(&self.base_url, "hierarchy.bin");
        Ok(self
            .get_range(hierarchy_url.as_str(), offset, length, None)
            .await?)
    }

    async fn read_octree(&self, offset: u64, length: usize) -> Result<Bytes, Self::Error> {
        let octree_url = asset_url(&self.base_url, "octree.bin");
        Ok(self
            .get_range(octree_url.as_str(), offset, length, None)
            .await?)
    }
}

impl PotreeHttpAsset {
    #[cfg(feature = "reqwest")]
    async fn get(
        &self,
        url: &str,
        headers: Option<BTreeMap<String, String>>,
    ) -> Result<Bytes, <Self as PotreeAsset>::Error> {
        let mut req = self.client.get(url);

        if let Some(hdrs) = headers {
            for (k, v) in hdrs {
                req = req.header(k, v);
            }
        }

        Ok(req.send().await?.error_for_status()?.bytes().await?)
    }

    #[cfg(all(feature = "ehttp", not(feature = "reqwest")))]
    async fn get(
        &self,
        url: &str,
        headers: Option<BTreeMap<String, String>>,
    ) -> Result<Bytes, <Self as PotreeAsset>::Error> {
        #[cfg(not(feature = "ehttp_local"))]
        {
            ehttp_get(url, headers).await
        }

        #[cfg(feature = "ehttp_local")]
        {
            use futures::SinkExt;

            let (tx_response, rx_response) = oneshot::channel();

            let request_message = Request {
                tx_response,
                url: url.to_string(),
                headers,
            };

            let mut tx_request = self.tx_request.clone();

            tx_request.send(request_message).await.map_err(|err| {
                <Self as PotreeAsset>::Error::Other(format!("Unable to send request: {}", err))
            })?;

            match rx_response.await {
                Ok(response) => response,
                Err(error) => Err(<Self as PotreeAsset>::Error::Other(format!(
                    "No response received: {}",
                    error
                ))),
            }
        }
    }

    async fn get_range(
        &self,
        url: &str,
        offset: u64,
        length: usize,
        headers: Option<BTreeMap<String, String>>,
    ) -> Result<Bytes, <Self as PotreeAsset>::Error> {
        let end = offset
            .checked_add(length as u64)
            .map(|v| v - 1)
            .ok_or_else(|| <Self as PotreeAsset>::Error::Other("Range overflow".into()))?;
        let range_value = format!("bytes={}-{}", offset, end);

        let mut headers = headers.unwrap_or_default();
        headers.insert("range".to_string(), range_value);

        self.get(url, Some(headers)).await
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
    #[cfg(feature = "reqwest")]
    #[error("Reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[cfg(feature = "ehttp")]
    #[error("EHttp error: {0}")]
    Ehttp(String),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Unable to parse url: {0}")]
    Url(#[from] url::ParseError),

    #[error("{0}")]
    Other(String),

    #[error("Unsupported scheme: {0}")]
    Unsupported(String),
}

#[cfg(feature = "ehttp_local")]
async fn process_requests(
    mut rx_requests: UnboundedReceiver<Request>,
) -> Result<(), <PotreeHttpAsset as PotreeAsset>::Error> {
    while let Some(message) = rx_requests.next().await {
        spawn_local(async move {
            let response = ehttp_get(&message.url, message.headers).await;
            let _ = message.tx_response.send(response);
        });
    }

    Ok(())
}

#[cfg(any(feature = "ehttp", feature = "ehttp_local"))]
#[allow(unused)]
async fn ehttp_get(
    url: &str,
    headers: Option<BTreeMap<String, String>>,
) -> Result<Bytes, <PotreeHttpAsset as PotreeAsset>::Error> {
    let headers = {
        if let Some(hdrs) = headers {
            let mut headers = ehttp::Headers::default();
            for (k, v) in hdrs {
                headers.insert(k, v);
            }
            headers
        } else {
            Default::default()
        }
    };

    let request = ehttp::Request {
        method: "GET".to_owned(),
        url: url.to_string(),
        body: vec![],
        headers,
        // To support headers, see https://github.com/emilk/ehttp/issues/57#issuecomment-2278447524
        #[cfg(target_arch = "wasm32")]
        mode: Mode::default(),
    };

    let (tx, rx) = futures::channel::oneshot::channel();
    ehttp::fetch(request, move |res| {
        let _ = tx.send(res);
    });

    let result = rx.await.map_err(|_| {
        <PotreeHttpAsset as PotreeAsset>::Error::Other("channel closed".to_string())
    })?;

    let response =
        result.map_err(|e| <PotreeHttpAsset as PotreeAsset>::Error::Ehttp(format!("{:?}", e)))?;

    if response.status < 200 || response.status >= 300 {
        return Err(<PotreeHttpAsset as PotreeAsset>::Error::Ehttp(format!(
            "HTTP Request error, got {} status code.",
            response.status
        )));
    }

    Ok(response.bytes.into())
}
