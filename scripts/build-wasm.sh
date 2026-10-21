#!/usr/bin/env bash
# Compile the Rust codebase into WebAssembly files using wasm-pack
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

OUT_DIR="${1:-pkg}"
PROFILE="${2:---release}"

echo "Compiling photocraft-web into WebAssembly with wasm-pack..."
wasm-pack build apps/photocraft-web --target web "$PROFILE" --out-dir "$OUT_DIR"

if [ -f "apps/photocraft-web/index-wasm.html" ]; then
  cp "apps/photocraft-web/index-wasm.html" "apps/photocraft-web/$OUT_DIR/index.html"
fi

echo "Build complete! WebAssembly files generated in apps/photocraft-web/$OUT_DIR"
