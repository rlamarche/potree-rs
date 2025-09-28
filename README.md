# Potree file parser in RUST

Features / Roadmap:

- [x] Load data asynchronously
- [x] Load (asynchronously) and parse hierarchy (lazy & entire) from filesystem or http
- [x] Native & WASM compatibility
- [x] WASM Multithread compatibility (using SharedArrayBuffer and specific http headers)
- [ ] Load points 
- [ ] Octree frustum culling helpers

# Download sample potree file

Go in the `assets/heidentor` folder and run `dl.sh` script:

```bash
cd assets/heidentor/
./dl.sh
```

# Run WASM Multithreaded example

This example uses a webworker for parsing, and delegates the http requests to the main thread (using provided `EhttpClientLocal`).

To prevent the worker to terminate and not executing async tasks, the example uses the hack mentionned in this issue: https://github.com/rustwasm/wasm-bindgen/issues/2945.


To build the example, comment the tokio dev dependency from `Cargo.toml`.

Then, build using the provided script: (install the required rust nightly if asked)

```bash
./build_wasm.sh
```

Install express and run `serve.js`:

Note: this server sends the security headers to allow using workers in WASM.

```bash
npm install express
node serve.js
```

Open the browser at address http://localhost:8080 and check network / console panels to see the requests / logs.
