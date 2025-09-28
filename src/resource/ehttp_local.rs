use crate::resource::ehttp::EhttpClient;
use crate::resource::{ResourceClient, ResourceError};
use async_trait::async_trait;
use futures::channel::mpsc::{SendError, UnboundedReceiver, UnboundedSender, unbounded};
use futures::channel::oneshot;
use futures::{SinkExt, StreamExt};
use std::collections::BTreeMap;
use wasm_bindgen_futures::spawn_local;

#[derive(Clone, Debug)]
pub struct EhttpClientLocal {
    tx_request: UnboundedSender<RequestMessage>,
}

struct RequestMessage {
    tx_response: oneshot::Sender<ResponseMessage>,
    payload: RequestPayload,
}

enum RequestPayload {
    Get {
        url: String,
        headers: Option<BTreeMap<String, String>>,
    },
    GetRange {
        url: String,
        offset: u64,
        length: usize,
        headers: Option<BTreeMap<String, String>>,
    },
}

struct ResponseMessage {
    payload: Result<Vec<u8>, ResourceError>,
}

enum ResponsePayload {
    Success(Vec<u8>),
    Error(ResourceError),
}

impl EhttpClientLocal {
    pub fn new() -> Self {
        let (tx_request, rx_request) = unbounded();

        spawn_local(async move {
            process_requests(rx_request)
                .await
                .expect("Failed to process requests");
        });

        Self { tx_request }
    }

    async fn send_request(&self, payload: RequestPayload) -> Result<Vec<u8>, ResourceError> {
        let (tx_response, rx_response) = oneshot::channel();

        let request_message = RequestMessage {
            tx_response,
            payload,
        };

        let mut tx_request = self.tx_request.clone();

        tx_request
            .send(request_message)
            .await
            .map_err(|err| ResourceError::Other(format!("Unable to send request: {}", err)))?;

        match rx_response.await {
            Ok(response) => response.payload,
            Err(error) => Err(ResourceError::Other(format!(
                "No response received: {}",
                error
            ))),
        }
    }
}

#[async_trait]
impl ResourceClient for EhttpClientLocal {
    async fn get(
        &self,
        url: &str,
        headers: Option<BTreeMap<String, String>>,
    ) -> Result<Vec<u8>, ResourceError> {
        self.send_request(RequestPayload::Get {
            url: url.to_string(),
            headers,
        })
        .await
    }

    async fn get_range(
        &self,
        url: &str,
        offset: u64,
        length: usize,
        headers: Option<BTreeMap<String, String>>,
    ) -> Result<Vec<u8>, ResourceError> {
        self.send_request(RequestPayload::GetRange {
            url: url.to_string(),
            offset,
            length,
            headers,
        })
        .await
    }
}

async fn process_requests(
    mut rx_requests: UnboundedReceiver<RequestMessage>,
) -> Result<(), SendError> {
    let ehttp_client = EhttpClient;

    while let Some(message) = rx_requests.next().await {
        match message.payload {
            RequestPayload::Get { url, headers } => {
                let response = ehttp_client.get(&url, headers).await;
                let _ = message
                    .tx_response
                    .send(ResponseMessage { payload: response });
            }
            RequestPayload::GetRange {
                url,
                offset,
                length,
                headers,
            } => {
                let response = ehttp_client.get_range(&url, offset, length, headers).await;
                let _ = message
                    .tx_response
                    .send(ResponseMessage { payload: response });
            }
        }
    }

    Ok(())
}
