//! PhotoCraft in the browser (WebAssembly library entry point).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(any(target_arch = "wasm32", test))]
pub mod preset_bridge;

#[cfg(target_arch = "wasm32")]
pub mod indexed_presets;

#[cfg(target_arch = "wasm32")]
pub mod web;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// Start the PhotoCraft web application.
///
/// Mounts the application canvas and initializes the eframe web runner.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn start() {
    web::start();
}

#[cfg(not(target_arch = "wasm32"))]
pub fn start() {
    eprintln!("photocraft-web only runs in the browser: build it with `wasm-pack` or `trunk`");
}
