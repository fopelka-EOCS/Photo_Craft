# Optional native Zstandard backend

The `native-zstd` feature selects the reference Zstandard C implementation for `.pcraft` compression and decoding, Affinity archive decoding, and Zstandard TIFF import. Default builds keep Rust Zstandard implementations. TIFF export keeps its existing options. Enable `photocraft-io/native-zstd` to select all three paths.

Native `.pcraft` compression uses the library default level 3 and independent checksummed frames. The default writer uses `ruzstd` Fastest. Native blob reads stream under the output budget and a 128 MiB window limit. Affinity reuses one decoder context across an entry's concatenated frames, with its existing 64 MiB window, output, import-budget and CRC checks.

The feature adds `zstd`/`zstd-sys` and needs a C compiler. WASM builds need a WASM-capable C compiler. The Rust wrapper is 0.13.3 and the C library is Zstandard 1.5.7. Whether to accept this dependency or enable it in official builds remains a maintainer decision.

General changes also apply to default builds: directory verification compares decoded bytes directly, directory output reuses at most 2 MiB between objects, encoders are created only for new work, ZIP loads borrow checked STORE payloads, decoded tiles transfer their storage into surfaces, and Affinity avoids per-pixel allocations and redundant raster copies. Native verification of objects over 2 MiB compares streamed chunks without a full decoded buffer.

## Method and limits

Measured on 2026-10-20 against main `872748fb`, with default and native branch builds. Apple M4 Max, 16 CPUs, 128 GiB, macOS arm64, rustc 1.99.1. Release O3, LTO off, 16 codegen units, debug off; repository-locked dependencies. Measurements ran serially at normal priority, without concurrent builds, with 16 Rayon workers. These are manual API measurements, not CI performance assertions or x86/browser results.

The tables predate decoder-state reuse in `9dba39c8` and the exact-input verification guard. They do not measure those changes.

CPU is process user plus system time around the operation, including its worker threads. Wall time uses a monotonic clock. Input reads, photo development, thumbnail rendering, output checks and cleanup are outside timers. Load validation remains inside. Directory saves include atomic writes and flushes. ZIP saves use a fresh `PcraftWriter`; common reads use main-written files, while own reads use each backend's files. Directory cases cover a new bundle, an existing bundle with a fresh writer, and a writer with its verification cache populated.

Each configuration has three fresh-process samples, with version order rotated per trial. Layout own reads and the U16 Deflate control have nine paired samples. The embedded-photo own read and large-directory verification have seven pairs. Changes compare medians. The intervals below resample paired ratios. Small three-sample differences do not establish a regression. TIFF compression variants are descriptive comparisons on matching pixels.

Memory is current physical footprint sampled every 2 ms during the first operation, with output held. It includes prepared inputs and memory retained by the allocator during setup. Short peaks can be missed. MB means decimal file size; MiB means 1,048,576 bytes of memory. Negative changes mean less time or memory.

Inputs use the pinned CC0 Pixls `IMG_4059.CR2` city photo (12 MP) and `P1000475.RW2` foliage photo (unresampled 24 MP crop), developed with ProPhoto RGB at U8/U16/F32. Float samples derive from U16 development, not HDR capture. Four-layer edits add Levels, Curves and a gradient mask. The embedded-photo edit also contains a masked crop of the other photo and its original 24 MP U16 TIFF. MIT psd-tools `masks.psd` supplies the 22-layer layout and three masks; `mask-index.psd` supplies the illustrated card. No noisy generators, enlarged images or duplicated background-photo layers are timing inputs.

Nine pinned CC0/MIT Affinity files cover raster, vector, brushes and RGB/CMYK data: seven use Zstandard and two are zlib controls. These are small authored documents, not large Affinity photo edits. TIFF variants are independently encoded by libtiff with 64-row strips, integer horizontal or floating-point prediction, Deflate level 6 and Zstandard level 3.

Every measured save is loaded and compared with the complete source document. Loads compare canonical document content, including tile and blob hashes and metadata. All 13 native-written real `.pcraft` documents also load identically with main and the default branch reader. Affinity checks every supported model field and image payload; TIFF checks exact decoded samples, dimensions and layout.

## Photographs

| Document | Save CPU / wall change | Common-read CPU / wall change | Own-read CPU / wall change | Main / native MB | Sampled peak save / own-read change, MiB |
|---|---:|---:|---:|---:|---:|
| City U8 | -55.4% / -26.0% | -61.3% / -61.3% | -59.6% / -59.5% | 26.15 / 21.08 | -35.8 / -6.5 |
| City U16 | -72.0% / -39.4% | -41.4% / -41.4% | -17.6% / -17.6% | 88.75 / 72.91 | -73.9 / -17.7 |
| City F32 | -57.7% / -28.4% | -61.5% / -61.5% | -57.6% / -57.6% | 120.96 / 99.96 | -81.4 / -25.2 |
| Foliage U8 | -59.3% / -28.7% | -59.6% / -59.6% | -58.6% / -58.6% | 49.64 / 37.70 | -49.4 / -12.7 |
| Foliage U16 | -68.9% / -37.0% | -41.4% / -41.4% | -17.0% / -17.0% | 175.68 / 143.90 | -121.3 / -32.2 |
| Foliage F32 | -58.6% / -28.3% | -58.4% / -58.4% | -51.6% / -51.6% | 252.23 / 206.02 | -147.6 / -48.4 |

## Layered documents

| Document | Save CPU / wall change | Common-read CPU / wall change | Own-read CPU / wall change | Main / native MB | Sampled peak save / own-read change, MiB |
|---|---:|---:|---:|---:|---:|
| City edit U8 | -55.0% / -24.2% | -61.2% / -61.2% | -59.4% / -59.4% | 26.22 / 21.13 | -35.0 / -5.8 |
| City edit U16 | -71.7% / -38.5% | -41.2% / -41.2% | -17.2% / -17.2% | 88.81 / 72.96 | -76.0 / -16.8 |
| City edit F32 | -57.5% / -27.6% | -61.3% / -61.3% | -57.6% / -57.6% | 121.02 / 100.01 | -82.1 / -27.4 |
| Foliage edit U16 | -68.4% / -35.8% | -40.9% / -40.9% | -15.8% / -15.8% | 175.78 / 143.99 | -122.7 / -31.0 |
| 22-layer layout | -72.1% / -19.9% | -7.1% / -7.1% | +1.5% / +1.5% | 0.43 / 0.40 | -13.7 / -0.8 |
| Illustrated card | -75.4% / -29.5% | -28.8% / -28.8% | -23.8% / -23.8% | 0.34 / 0.17 | -16.6 / -1.0 |
| Embedded-photo edit | -71.4% / -75.8% | -40.0% / -40.0% | -12.1% / -12.1% | 169.91 / 153.85 | -159.3 / -149.5 |

## Directory saves

Cells show default / native CPU changes. Wall changes are native. All saves include normal atomic writes and flushes.

| Document | First save CPU | Full verification CPU | Cached save CPU | First / verify / cached wall | Sampled peak first / verify / cached change, MiB |
|---|---:|---:|---:|---:|---:|
| City U8 | -1.7% / -44.3% | -23.2% / -67.2% | -26.5% / -62.0% | -16.1% / -64.0% / -56.5% | -0.7 / -0.2 / -3.3 |
| Foliage U16 | +0.7% / -44.8% | -43.2% / -49.5% | -42.6% / -29.5% | -20.4% / -48.2% / -23.1% | -3.4 / -0.2 / -5.1 |
| 22-layer layout | -11.0% / -25.6% | -29.0% / -25.3% | -52.1% / -54.6% | -3.7% / -19.7% / -34.2% | -4.6 / -0.6 / -2.6 |
| Embedded-photo edit | -0.6% / -56.4% | -42.6% / -49.6% | -0.6% / +4.8% | -33.5% / -48.2% / +4.8% | -270.7 / -420.5 / -272.5 |

## Affinity

Full reads compare the complete supported document model. Archive reads decode every current entry and are diagnostic; they do not render unsupported content.

| Document | Full-read CPU main / native ms | Full-read wall main / native ms | Default CPU change | Archive CPU change | Sampled peak full-read change, MiB |
|---|---:|---:|---:|---:|---:|
| raster_test | 0.540 / 0.211 | 0.538 / 0.210 | -62.0% | -3.3% | -0.1 |
| layer_test | 0.134 / 0.124 | 0.132 / 0.122 | -6.0% | -2.0% | -0.0 |
| 12-rgb8 | 0.595 / 0.476 | 0.594 / 0.474 | -2.7% | -79.0% | -0.1 |
| 12-rgb16 | 0.593 / 0.467 | 0.592 / 0.466 | -3.4% | -75.3% | -0.1 |
| 12-cmyk8 | 0.614 / 0.493 | 0.612 / 0.491 | -2.8% | -78.3% | -0.1 |
| 03-brush-strokes | 12.516 / 2.498 | 12.515 / 2.497 | -29.1% | -79.3% | -0.4 |
| 13-blend-modes | 0.511 / 0.403 | 0.509 / 0.402 | +1.0% | -79.5% | -0.1 |
| playing-cards | 3.414 / 2.958 | 3.413 / 2.956 | +2.1% | -82.0% | -0.3 |
| flowers | 0.605 / 0.481 | 0.604 / 0.479 | -0.8% | -63.5% | -0.1 |

## TIFF

The Zstandard files are new supported inputs. Main cannot decode them. These comparisons use matching real pixels, strip geometry and predictors in Deflate and Zstandard variants.

| Photo | CPU Deflate / Zstandard ms | Wall Deflate / Zstandard ms | Sampled peak Deflate / Zstandard MiB | File Deflate / Zstandard MB |
|---|---:|---:|---:|---:|
| City U8 | 195.21 / 117.04 | 15.15 / 8.92 | 53.6 / 68.3 | 14.76 / 15.08 |
| City U16 | 332.65 / 189.41 | 25.92 / 13.99 | 130.0 / 157.1 | 58.76 / 58.15 |
| City F32 | 792.91 / 524.08 | 60.39 / 38.97 | 220.0 / 257.9 | 80.33 / 79.79 |
| Foliage U16 | 691.81 / 358.28 | 50.74 / 26.18 | 249.9 / 288.6 | 112.20 / 110.38 |

## Remaining tradeoffs

The 22-layer layout own read is 16.832 → 17.091 ms: +1.5%, or +0.26 ms, with a paired 95% interval of +1.0..+3.3%. Its common-file read is faster. This small own-output cost remains a backend tradeoff; no broader performance ceiling is claimed.

The large embedded-photo cached directory save is 143.847 → 150.725 CPU ms (+4.8%), with an interval of -0.8..+10.0%. It does not establish a slowdown in the final repeat. Cached saves recheck a subset of objects. First directory saves are often limited by filesystem flushes, so CPU gains are larger than wall-time gains.

Default F32 ZIP saves show up to about 5 MiB more sampled peak memory. Native directory cases show up to about 2.5 MiB more in some operations. The zero-copy and scratch paths are retained for CPU performance. Native TIFF Zstandard reads use 15–39 MiB more sampled peak memory than the Deflate variants. The city U8 Zstandard TIFF is about 2.2% larger; the other three Zstandard TIFFs are smaller. This branch adds a reader, not a TIFF encoding policy.

A direct seven-pair comparison of large-object verification before and after streamed comparison reduced sampled peak memory by 133 MiB. CPU changed by -1.5%, with an interval of -3.2..+0.2%; no practical CPU cost was detected.

## Compatibility checks

The ignored complete-document test exchanges U8/U16/F32 files between default and native builds. The second and third commands require the other backend's files, so missing fixtures fail. From the repository root:

```sh
export PHOTOCRAFT_BACKEND_FIXTURES="$(mktemp -d)"
cargo test -p photocraft-format --locked --test roundtrip cross_backend_documents -- --ignored --exact
PHOTOCRAFT_BACKEND_READ_OTHER=1 cargo test -p photocraft-format --locked --features native-zstd --test roundtrip cross_backend_documents -- --ignored --exact
PHOTOCRAFT_BACKEND_READ_OTHER=1 cargo test -p photocraft-format --locked --test roundtrip cross_backend_documents -- --ignored --exact
```

Decoder and format tests cover truncation, wrong checksums, excessive output, oversized windows, large-object verification and directory repair. Large native directory objects must contain exactly one frame, with no trailing input. The first-frame matching API remains available; Affinity reads concatenated frames.

CI runs optional-backend checks only for relevant source, test, dependency and build-input changes; manual runs and `full-ci` force them. It includes decoder tests, an Affinity import, format tests, cross-backend fixtures, strict Clippy and WASM checks with Clang. Default builds keep their existing checks.
