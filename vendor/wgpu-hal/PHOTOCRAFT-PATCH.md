# PhotoCraft patch to wgpu-hal 30.0.1

Source: https://crates.io/crates/wgpu-hal/30.0.1 (MIT OR Apache-2.0; licenses are alongside this file).

PhotoCraft changes `src/dx12/mod.rs`: the `DXGI_SWAP_CHAIN_DESC1` uses
`DXGI_SCALING_NONE` for HWND swap chains and retains `DXGI_SCALING_STRETCH`
for composition swap chains. No other upstream source is changed. The Cargo
`[patch.crates-io]` entry in the workspace manifest applies this to the
existing wgpu dependency graph.

On the affected Windows system, `DXGI_SCALING_STRETCH` resamples the restored
window's UI despite a sharp framebuffer. `DXGI_SCALING_NONE` prevents that
extra resampling. The same root cause was measured in FigCraft issue #778.
Microsoft requires `DXGI_SCALING_STRETCH` for
`CreateSwapChainForComposition`, so the patch preserves it for composition
targets: https://learn.microsoft.com/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgifactory2-createswapchainforcomposition
On a 2560×1440 Windows display, the patched PhotoCraft framebuffer and
presented window matched in the sampled UI area at 100%, 125%, and 150%
display scale, both restored and maximized and after six successive resizes.

Upstream tracking: https://github.com/gfx-rs/wgpu/issues/10196 discusses
configurable swap-chain scaling, including the DX12 backend's hard-coded
`DXGI_SCALING_STRETCH`. Its Vulkan symptom differs from PhotoCraft's DX12
blur, so this issue tracks the scaling policy rather than a landed fix.

Remove this vendor copy once a released wgpu-hal supports non-stretch DX12
presentation (or exposes a scaling mode that PhotoCraft can select), and
PhotoCraft has upgraded to and verified that release.
