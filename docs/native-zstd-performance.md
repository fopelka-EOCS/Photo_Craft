# Optional native Zstandard backend

This opt-in feature uses the reference Zstandard C implementation for native-format compression and bounded tile decoding. Default builds retain the existing fresh Rust compressor. Enabling native-zstd adds zstd/zstd-sys and requires a C compiler (with WASM support for a WASM build). The 24 MP RGBA8 bundle cases are a repeating ramp and deterministic noise.

Measured on 2026-10-19 against `d95fdf3181c01ad41e95456091a394b105a9afb2` on an Apple M4 Max
(16 CPUs, 128 GiB), macOS arm64, rustc 1.99.1. Each branch starts directly at that
base; none of the other performance splits is applied. Release uses optimization
level 3, LTO off, 16 codegen units, debug info off and incremental compilation off.
Builds ran at nice 10 with APFS-cloned warm caches; measurements ran at normal
priority with no concurrent builds. Timings are medians of five samples after one
warm-up, except the compositor's ten Criterion samples. These synthetic workloads
are reproducible checks of this change, not a representative image corpus.

| Workload | Main ms | This change ms | Time change |
|---|---:|---:|---:|
| ramp-save | 82.948 | 83.232 | +0.3% |
| ramp-load | 1.437 | 1.412 | -1.7% |
| noise-save | 180.929 | 97.123 | -46.3% |
| noise-load | 102.426 | 151.481 | +47.9% |

| Bundle | Default Rust bytes | Native bytes |
|---|---:|---:|
| ramp-save | 88,308 | 77,401 |
| noise-save | 96,572,050 | 82,235,809 |

For the comparison, run the harness once without the feature and once with it. Timings include hashing, compression/decompression and ZIP assembly. The ramp deduplicates repeated tiles. Noise loading is slower with the native decoder; this is a build/product tradeoff, not an across-the-board speedup. No deflate, PNG, raster, compositor or Rust workspace-reuse split is included.

Committed tests compare reusable native frames with the library bulk encoder, enforce output bounds/checksums/truncation handling, and cross-decode frames with Rust and native implementations. The complete-document harness was run default → native → default using shared fixtures at U8/U16/F32, including tiles, metadata and blobs. CI separately tests the feature and checks its WASM build.

Reproduce with the same release profile environment:

```sh
CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 CARGO_PROFILE_RELEASE_DEBUG=0 CARGO_INCREMENTAL=0 nice -n 10 cargo test --release -p photocraft-format --test backend_compatibility --no-run --features native-zstd
# Run the printed executable at normal priority:
<printed-executable> benchmark_backend --ignored --nocapture --test-threads=1
```

## Local validation

- `cargo test -p photocraft-format -p photocraft-codecs --locked`: passed (929 passed, 4 ignored).
- `cargo clippy -p photocraft-format -p photocraft-codecs --locked --all-targets -- -D warnings`: passed.
- `cargo xtask layers`: passed.
- `cargo check -p photocraft-format -p photocraft-codecs --locked --target wasm32-unknown-unknown`: passed.
- `cargo test -p photocraft-format -p photocraft-codecs --locked --features photocraft-format/native-zstd`: passed (930 passed, 4 ignored).
- `cargo clippy -p photocraft-format -p photocraft-codecs --locked --all-targets --features photocraft-format/native-zstd -- -D warnings`: passed.
- `cargo check -p photocraft-format --locked --target wasm32-unknown-unknown --features native-zstd`: passed.
- `cargo xtask test-corpus -p codecs`: passed (795 passed, 3 ignored).
- Cross-backend complete-document fixtures: default → native → default, U8/U16/F32, passed.
- Workflow actionlint: passed with existing Node 20 action deprecations filtered; unfiltered actionlint reports those pre-existing deprecations.
