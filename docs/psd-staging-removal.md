# PSD staging removal

Decoded layer, mask, flattened-fallback and extra-channel planes write directly into tiled
surfaces. Layer/mask export gathers final channel planes directly; extra merged channels write
into their final tail. Native conversion stays parallel and WASM stays sequential. Public
interleave/deinterleave helpers remain available. Bounded merged-color render bands remain.

## Measurements

Release build on Apple M4 Max, seven rotating paired samples after warmup. Real 24 MP foliage
and 12 MP city photo edits contain Levels, Curves and a gradient-masked adjustment. ProPhoto
RGB is retained; F32 derives from developed U16 samples. An upstream layered PSD design adds
authored-file coverage. The parent is the PSD interleave refactor. No generated noise is timed.

Import/export timers include parsing/serialization and compression. Output hashing and
equivalence checks are outside timers. Intervals describe paired bootstrap 95% sample variation.

| Document | Operation | Main ms | Parent ms | Retained ms | Change vs parent |
|---|---|---:|---:|---:|---:|
| 24 MP foliage, U16 | psd-import | 653.1 | 628.8 | 621.3 | -1.2% (-2.8..-0.5) |
| 24 MP foliage, U16 | psd-export | 4685.0 | 4681.5 | 4564.7 | -2.5% (-3.8..-1.5) |
| 12 MP city, F32 | psd-import | 300.4 | 276.7 | 262.3 | -5.2% (-5.7..+0.4) |
| 12 MP city, F32 | psd-export | 1735.6 | 1733.7 | 1671.7 | -3.6% (-5.8..-1.9) |
| Layered design | psd-import | 18.3 | 14.4 | 11.9 | -17.5% (-20.4..-16.2) |
| Layered design | psd-export | 83.9 | 84.0 | 69.5 | -17.2% (-17.7..-16.7) |

Fresh-process peak RSS, median of three runs, includes input/setup. Large-photo import peaks
fall from 1,109 to 932 MiB (U16) and 1,032 to 855 MiB (F32) versus the parent: 16–17% lower.
Against main they are 29–31% lower. Export peaks vary near the parent (+0.1–2.4% median);
no export peak-memory improvement is claimed. The removed full 24 MP RGBA staging buffer is
96/192/384 MB at U8/U16/F32, while required decoded planes and final tiles still exist.

All encoded PSDs are byte-identical to the baselines; imported tile/default/sample data match.
Unit oracles cover all depths in RGB, Gray, CMYK and Lab, negative origins, padding, missing
or truncated planes, Lab16 scaling, CMYK inversion and nonzero mask defaults. Affected color,
composite, round-trip and real-file corpus oracles pass.
