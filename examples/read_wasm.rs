use potree::{parse_entire_hierarchy, parse_metadatas};
use potree::resource::Resource;
use potree::resource::ehttp::EhttpClient;
use std::sync::Arc;

use potree::resource::ehttp_local::EhttpClientLocal;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use wasm_thread as thread;

#[wasm_bindgen]
extern "C" {
    // Use `js_namespace` here to bind `console.log(..)` instead of just
    // `log(..)`
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);

    // The `console.log` is quite polymorphic, so we can bind it with multiple
    // signatures. Note that we need to use `js_name` to ensure we always call
    // `log` in JS.
    #[wasm_bindgen(js_namespace = console, js_name = log)]
    fn log_u32(a: u32);

    // Multiple arguments too!
    #[wasm_bindgen(js_namespace = console, js_name = log)]
    fn log_many(a: &str, b: &str);
}

pub fn main() {
    let http_client = EhttpClientLocal::new();

    thread::spawn({
        let http_client = http_client.clone();
        || {
            log("Hello from thread!");

            // can't use spawn_local here because we are in a worker thread
            pollster::block_on(async move {
                log("Hello from spawned local!");
                let metadata_resource = Resource::new(
                    "http://localhost:8080/assets/heidentor/metadata.json",
                    http_client.clone(),
                );
                let metadatas = parse_metadatas(metadata_resource).await.unwrap();

                log(&format!("{:#?}", metadatas));

                let hierarchy_resource = Resource::new(
                    "http://localhost:8080/assets/heidentor/hierarchy.bin",
                    http_client,
                );
                let initial_herarchy = metadatas.read_initial_hierarchy(&hierarchy_resource).await.unwrap();

                log(&format!("Initial herarchy: {:#?}", initial_herarchy));

                let entire_hierarchy = metadatas.read_entire_hierarchy(&hierarchy_resource).await.unwrap();

                log(&format!("Entire hierarchy: {:#?}", entire_hierarchy));

            });
        }
    });
}
