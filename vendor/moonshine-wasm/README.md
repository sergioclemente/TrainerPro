# Moonshine WASM package provenance

TrainerPro vendors a prebuilt single-thread `@moonshine-ai/moonshine-wasm`
package because upstream release 0.1.5 requires pthreads. Tauri's macOS
WKWebView does not expose `SharedArrayBuffer`, so that upstream artifact cannot
initialize there.

The canonical source is the TrainerPro Moonshine fork, not patches in this
repository:

- repository: `https://github.com/simoeswolf/moonshine.git`
- branch: `trainerpro-v0.1.5-wasm-compat`
- commit: `663c475a86e5d738041646c7a522160db6870839`
- upstream base: Moonshine 0.1.5 at
  `234f60faa0eb388b01cdf7e60aca232af37aefda`
- ONNX Runtime: 1.23.2
- Emscripten SDK: 4.0.8
- CMake option: `MOONSHINE_WASM_SINGLE_THREAD=ON`
- package version: `0.1.5-trainerpro.singlethread.8`
- tarball SHA-256:
  `d71137f94836c8e5fc02078887a79e854cfcc549677ab26ce8da524140a09b23`

That commit contains the WebView compatibility work: minifier-safe generated
AudioWorklet source, best-effort Cache Storage, caller-supplied worker and exact
WASM URLs, worker error stacks, the exported STT worker, package license, and a
cache regression test. Keep future source changes and upstreamable tests in
the fork. This directory holds only the derived package needed for reproducible
TrainerPro installs and this provenance note.

To rebuild, check out the commit above with Git LFS enabled, prepare the
single-thread ONNX Runtime archive, and run:

```sh
./scripts/build-ort-wasm.sh single-thread force
./scripts/build-wasm.sh single-thread skip-ort
```

Then run `npm pack` from `language-bindings/wasm` and verify the archive hash
before replacing TrainerPro's vendored package.
