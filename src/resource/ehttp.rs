use std::collections::BTreeMap;
use async_trait::async_trait;
use super::{ResourceClient, ResourceError};
#[cfg(target_arch = "wasm32")]
use ehttp::Mode;

pub struct EhttpClient;

#[async_trait]
impl ResourceClient for EhttpClient {
    async fn get(
        &self,
        url: &str,
        headers: Option<BTreeMap<String, String>>, // `ehttp` has limited headers support
    ) -> Result<Vec<u8>, ResourceError> {
        let (tx, rx) = futures::channel::oneshot::channel();

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

        ehttp::fetch(request, move |res| {
            let _ = tx.send(res);
        });

        let response = rx.await.map_err(|_| ResourceError::Network("channel closed".to_string()))?;
        let response = response.map_err(|e| ResourceError::Network(format!("{:?}", e)))?;

        Ok(response.bytes)
    }
}
