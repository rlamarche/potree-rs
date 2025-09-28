use potree::prelude::*;
use tracing_subscriber::fmt;
use tracing_subscriber_wasm::MakeConsoleWriter;
use wasm_bindgen::prelude::*;
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
    fmt()
        .with_writer(
            // To avoide trace events in the browser from showing their
            // JS backtrace, which is very annoying, in my opinion
            MakeConsoleWriter::default().map_trace_level_to(tracing::Level::DEBUG),
        )
        // For some reason, if we don't do this in the browser, we get
        // a runtime error.
        .without_time()
        .init();

    // must be instantiated in main thread
    let resource_loader = ResourceLoader::new();

    thread::spawn({
        let resource_loader = resource_loader.clone();
        || {
            log("Hello from thread!");

            // can't use spawn_local here because we are in a worker thread
            pollster::block_on(async move {
                log("Hello from spawned local!");

                tracing::info!("Load pointcloud from local filesystem");
                let mut point_cloud = PotreePointCloud::from_url(
                    "http://localhost:8080/assets/heidentor",
                    resource_loader,
                )
                .await
                .expect("Unable to load point cloud");

                tracing::info!("Successfuly loaded point cloud hierarchy.");
                tracing::info!("{:#?}", point_cloud.hierarchy_snapshot());

                point_cloud
                    .load_entire_hierarchy()
                    .await
                    .expect("Unable to load entire hierarchy");

                tracing::info!("Successfuly loaded entire point cloud hierarchy.");
                tracing::info!("{:#?}", point_cloud.hierarchy_snapshot());
            });
        }
    });
}
