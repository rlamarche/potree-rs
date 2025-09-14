# Potree file parser in RUST

Features / Roadmap:

- [x] Load data asynchronously
- [x] Load (asynchronously) and parse hierarchy (lazy & entire) from filesystem or http
- [x] Native & WASM compatibility
- [x] WASM Multithread compatibility (using SharedArrayBuffer and specific http headers)
- [ ] Load points 
- [ ] Octree frustum culling helpers

# WASM Multithreaded

This sample uses a webworker for parsing (blocking async using pollster), and delegates the http requests to the main thread (using provided `EhttpClientLocal`).

Build using the provided script:

```bash
./build_wasm.sh
```

Install express and run `serve.js`:

```bash
npm install express
node serve.js
```

