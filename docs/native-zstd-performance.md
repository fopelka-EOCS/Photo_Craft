# Optional native Zstandard backend

Opt in to the reference Zstandard C implementation at default level 3 for native-format compression and bounded tile decoding. Blobs retain the bounded Rust streaming reader. Default builds retain the Rust backend. Enabling the feature adds zstd/zstd-sys and requires a C compiler, including a WASM-capable compiler for WASM.

The opt-in backend gives substantial real-photo save/load and size improvements, but own-output loading of the 22-layer layout is about 5% slower (under 1 ms), confirmed separately. Decoder-context reuse and native decoding of sized blobs did not remove the regression; both changes were removed. Fast mode introduced photo size regressions, while level 5 still regressed layout loading. Those diagnostic variants are not retained. Production code remains unchanged, with the small-layout read tradeoff explicit.

Measured on 2026-10-19 against main `d95fdf3181c01ad41e95456091a394b105a9afb2`, independently of the other splits. Apple M4 Max, 16 CPUs, 128 GiB, macOS arm64, rustc 1.99.1; release O3, LTO off, 16 codegen units, debug off. Builds used nice 10 and warm APFS-cloned caches; measurements used default priority without concurrent builds or profiling. Five alternating paired samples follow a warmup; confirmation and fix runs use nine pairs. Time change compares the two medians; intervals resample paired ratios. Negative time change means faster. Small differences are not automatically established improvements or regressions.

The full-resolution CC0 Pixls photographs are P1000475.RW2 (24 MP foliage centre crop, no resampling) and IMG_4059.CR2 (12 MP city), each converted to U8/U16/F32 with the ProPhoto RGB profile. F32 derives from 16-bit development, not HDR capture. MIT psd-tools masks.psd supplies a 640 x 1394 web layout with 22 imported layers and three masks; mask-index.psd supplies a 1000 x 867 illustrated card with a mask. The sources and hashes are pinned by the existing corpus. These are the available authored layered files, not a large layered-photo corpus. No generated noise, repeating ramps, enlarged charts or duplicated photo layers are used.

The shared harness times in-memory operations. Input loading, image conversion, thumbnail rendering, hashes and equivalence checks are outside timers. Every save uses a fresh PcraftWriter and a real rendered thumbnail. Common reads use main-encoded files; own-output reads are separate. PSD exports use ZIP merged images at U8/U16, Raw merged images at F32, and normal RLE layer channels at U8/U16 / ZIP prediction at F32. Results do not measure filesystem throughput or browser performance.

| Document | Operation | Main ms | Branch ms | Time change | Paired 95% interval | Main / branch bytes |
|---|---|---:|---:|---:|---:|---:|
| layout-masks | load-own (confirm) | 15.664 | 16.430 | +4.9% | +2.9..+6.7% | 433,949 / 404,015 |
| panasonic-8 | save | 168.402 | 121.899 | -27.6% | -36.6..-27.3% | 49,643,217 / 37,701,432 |
| panasonic-8 | load-common | 413.392 | 167.135 | -59.6% | -60.1..-59.4% | 49,643,217 / 49,643,217 |
| panasonic-8 | load-own | 414.007 | 170.341 | -58.9% | -60.2..-58.6% | 49,643,217 / 37,701,432 |
| panasonic-16 | save | 367.188 | 232.491 | -36.7% | -37.7..-35.7% | 175,681,852 / 143,903,128 |
| panasonic-16 | load-common | 401.666 | 238.322 | -40.7% | -41.2..-40.2% | 175,681,852 / 175,681,852 |
| panasonic-16 | load-own | 403.651 | 339.629 | -15.9% | -16.7..-15.4% | 175,681,852 / 143,903,128 |
| panasonic-32 | save | 644.043 | 456.315 | -29.1% | -36.1..-28.6% | 252,231,671 / 206,015,394 |
| panasonic-32 | load-common | 1330.527 | 547.415 | -58.9% | -59.2..-58.1% | 252,231,671 / 252,231,671 |
| panasonic-32 | load-own | 1329.653 | 637.016 | -52.1% | -52.2..-51.1% | 252,231,671 / 206,015,394 |
| canon-8 | save | 89.255 | 65.432 | -26.7% | -27.2..-26.7% | 26,151,631 / 21,078,090 |
| canon-8 | load-common | 216.029 | 83.236 | -61.5% | -61.6..-61.2% | 26,151,631 / 26,151,631 |
| canon-8 | load-own | 215.782 | 86.225 | -60.0% | -60.2..-60.0% | 26,151,631 / 21,078,090 |
| canon-16 | save | 184.899 | 115.484 | -37.5% | -38.0..-36.8% | 88,750,003 / 72,905,769 |
| canon-16 | load-common | 192.776 | 113.956 | -40.9% | -41.2..-40.7% | 88,750,003 / 88,750,003 |
| canon-16 | load-own | 192.402 | 158.457 | -17.6% | -18.5..-16.4% | 88,750,003 / 72,905,769 |
| canon-32 | save | 324.610 | 232.344 | -28.4% | -30.0..-27.9% | 120,957,984 / 99,956,114 |
| canon-32 | load-common | 764.505 | 291.715 | -61.8% | -62.0..-61.7% | 120,957,984 / 120,957,984 |
| canon-32 | load-own | 764.817 | 322.372 | -57.8% | -58.2..-57.3% | 120,957,984 / 99,956,114 |
| layout-masks | save | 18.203 | 14.859 | -18.4% | -20.0..-18.1% | 433,949 / 404,015 |
| layout-masks | load-common | 15.895 | 14.809 | -6.8% | -7.8..-4.1% | 433,949 / 433,949 |
| layout-mask-index | save | 7.122 | 5.229 | -26.6% | -28.1..-22.6% | 344,892 / 168,059 |
| layout-mask-index | load-common | 6.693 | 5.053 | -24.5% | -26.5..-23.1% | 344,892 / 344,892 |
| layout-mask-index | load-own | 6.591 | 5.070 | -23.1% | -23.8..-21.6% | 344,892 / 168,059 |

Every real save and load compares the complete document with its source, including tiles and metadata. Native and Rust encoded bytes may differ. Existing unit tests cover independent frames, reference encoder equivalence, Rust/native cross-decoding, output bounds, checksums and truncation. Current main also read native-written versions of all eight real documents with full document equality.

Current main read all eight real native-written documents with complete equality. The real-image harness checks full document equality on every native save and load. Earlier split validation remains in the branch performance document; no production-code change is retained and no synthetic timing test was run.

Local raw evidence and the shared harness: `.local/perf-real-261019/` in the repository checkout (ignored). This comparison supersedes the earlier synthetic performance table. Small synthetic unit fixtures remain correctness checks; ignored synthetic timing tests were not run.

## Earlier split validation

These checks passed when the split was prepared; they were not all rerun for this benchmark review. Current affected checks are listed above where production code changed.

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
