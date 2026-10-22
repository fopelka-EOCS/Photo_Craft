#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"
command -v cargo >/dev/null || { echo 'error: Rust fehlt. Siehe README.md.'; exit 1; }
case "${PLATFORM_NAME:-}" in
  iphoneos) TARGET=aarch64-apple-ios ;;
  iphonesimulator)
    case "${CURRENT_ARCH:-arm64}" in
      arm64|undefined_arch) TARGET=aarch64-apple-ios-sim ;;
      x86_64) TARGET=x86_64-apple-ios ;;
      *) echo 'error: Simulator-Architektur nicht unterstützt.'; exit 1 ;;
    esac ;;
  *) echo 'error: Dieses Projekt ist ausschließlich für iOS.'; exit 1 ;;
esac
# Install once ahead of the build: rustup target add aarch64-apple-ios aarch64-apple-ios-sim
export IPHONEOS_DEPLOYMENT_TARGET=17.0
export CARGO_TARGET_DIR="${PHOTOCRAFT_CARGO_TARGET_DIR:-${DERIVED_FILE_DIR}/rust}"
cargo +1.95.0 build --locked --release --manifest-path "$ROOT/Bridge/Cargo.toml" --target "$TARGET"
mkdir -p "${BUILT_PRODUCTS_DIR}/Rust"
cp "$CARGO_TARGET_DIR/$TARGET/release/libphotocraft_ios_bridge.a" "${BUILT_PRODUCTS_DIR}/Rust/"
