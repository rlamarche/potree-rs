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

This example uses a webworker for parsing (blocking async using pollster), and delegates the http requests to the main thread (using provided `EhttpClientLocal`).

First comment the tokio dev dependency from `Cargo.toml`.

Build using the provided script:

```bash
./build_wasm.sh
```

Install express and run `serve.js`:

```bash
npm install express
node serve.js
```

Open the browser at address http://localhost:8080 and check network / console panels to see the requests / logs.
