# Snapdragon X Elite NPU Subject Selection Worker

This directory contains standalone worker implementations that connect PhotoCraft's
compatibility-adapter host contract to native on-device NPU hardware (such as the Qualcomm
Snapdragon X Elite Hexagon NPU on Windows 11 Copilot+ PCs), reproducing the architecture used
by Affinity Photo 2.6's on-device machine learning selection tools.

## Architecture

```text
PhotoCraft (Safe Rust, wasm-compatible)
  │  Session::execute("select.subject", {})
  ▼
NpuSubjectSelectionAdapter (implements Adapter trait)
  │  Encodes active canvas RGBA pixels into stdin stream
  ▼
photocraft-npu-worker (External native ARM64 process)
  │  Windows AI ImageForegroundExtractor (Copilot+ PC NPU)
  │  or ONNX Runtime (QNN HTP / DirectML EP)
  ▼
Output 8-bit Grayscale Mask
  │  Returned via stdout stream to Adapter
  ▼
PhotoCraft Selection Pipeline
  │  Converts mask into document selection region
  │  Optional edge refinement (Refine Selection)
  ▼
Document Selection (with single-step Undo)
```

## Available Implementations

1. `Program.cs`:
   Uses Windows 11 Copilot+ PC's built-in `Microsoft.Windows.AI.Imaging.ImageForegroundExtractor` (MIT licensed sample pattern from `microsoft/ai-dev-gallery`).
   - Runs directly on the Hexagon NPU on Snapdragon X Elite.
   - Zero model downloading or quantization needed (system-managed AI model).
   - Returns a `BitmapPixelFormat.Gray8` mask.

2. `worker_directml.py`:
   Uses ONNX Runtime with DirectML or QNN Execution Provider (`QnnHtp.dll`).
   - Suitable for running custom quantized models (such as SINet or MobileSAM from Qualcomm AI Hub).