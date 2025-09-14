use super::{ResourceClient, ResourceError};
use async_trait::async_trait;
use std::collections::BTreeMap;

pub struct ReqwestClient {
    client: reqwest::Client,
}

impl ReqwestClient {
    pub fn new() -> Self {
        Self { client: reqwest::Client::new() }
    }
}

#[async_trait]
impl ResourceClient for ReqwestClient {
    async fn get(
        &self,
        url: &str,
        headers: Option<BTreeMap<String, String>>,
    ) -> Result<Vec<u8>, ResourceError> {
        let mut req = self.client.get(url);
        if let Some(hdrs) = headers {
            for (k, v) in hdrs {
                req = req.header(k, v);
            }
        }
        let resp = req.send().await.map_err(|e| ResourceError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(ResourceError::Status(status));
        }
        Ok(resp.bytes().await.map_err(|e| ResourceError::Network(e.to_string()))?.to_vec())
    }
}
