//! The browser threads build: the web app compiled with WebAssembly threads (atomics, shared
//! memory) on a pinned nightly that rebuilds std with them, running rayon on a pool of Web Workers
//! (wasm-bindgen-rayon).
//!
//! - `cargo xtask web-threads` builds it with Trunk, from the app's own `index.html`, into
//!   `dist/web-threads` (as `trunk build --release` builds the single-threaded app into
//!   `dist/web`). `PHOTOCRAFT_WEB_THREADS=1 packaging/web/package.sh` runs it and packs both
//!   builds into one site.
//! - `cargo xtask wasm --threads` type-checks the wasm-safe crates and the web app the same way, so
//!   the `target_feature = "atomics"` gates keep building (CI's `wasm-threads` job).

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{root, run};

/// The nightly both commands use (`-Z build-std` and atomics need one). CI's `wasm-threads` job
/// installs the same one (a test checks).
pub const NIGHTLY: &str = "nightly-2026-10-09";

/// atomics + bulk-memory: threads (std is rebuilt with them). The linker makes the memory shared
/// and imported, up to 4 GiB, so every worker instantiates the module on the same memory, and
/// exports the thread-local storage symbols wasm-bindgen needs to start each thread. simd128: every
/// browser with Wasm threads also has Wasm SIMD.
const RUSTFLAGS: &str = "-C target-feature=+atomics,+bulk-memory,+simd128 \
    -C link-arg=--shared-memory -C link-arg=--import-memory -C link-arg=--max-memory=4294967296 \
    -C link-arg=--export=__heap_base -C link-arg=--export=__wasm_init_tls -C link-arg=--export=__tls_size \
    -C link-arg=--export=__tls_align -C link-arg=--export=__tls_base";

/// `<cargo's target dir>/<name>`: `CARGO_TARGET_DIR` if set, else `target/` in the workspace.
pub fn target_dir(name: &str) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root().join("target"), PathBuf::from).join(name)
}

/// What to install when the nightly or its standard library sources are missing.
fn install_hint() -> String {
    format!("rustup toolchain install {NIGHTLY} --component rust-src --target wasm32-unknown-unknown")
}

/// Points `cmd` (cargo, or a tool that runs cargo) at the threads build: the nightly, its flags, std
/// rebuilt from source, and `target_dir` (the flags differ from every other build).
fn configure(cmd: &mut Command, target_dir: &Path) {
    cmd.env("RUSTUP_TOOLCHAIN", NIGHTLY)
        .env("RUSTFLAGS", RUSTFLAGS)
        .env("CARGO_UNSTABLE_BUILD_STD", "panic_abort,std")
        .env("CARGO_TARGET_DIR", target_dir)
        // Set by the `cargo` that runs xtask: the stable toolchain's own cargo, and flags that would
        // take precedence over RUSTFLAGS.
        .env_remove("CARGO")
        .env_remove("CARGO_ENCODED_RUSTFLAGS");
}

/// Fails with what to install unless the nightly and its standard library sources are there.
fn check_toolchain() -> Result<(), String> {
    let out = Command::new("rustup")
        .args(["run", NIGHTLY, "rustc", "--print", "sysroot"])
        .output()
        .map_err(|e| format!("rustup: {e} (the threads build needs rustup)"))?;
    if !out.status.success() {
        return Err(format!("the threads build needs {NIGHTLY}: {}", install_hint()));
    }
    let sysroot = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if !Path::new(&sysroot).join("lib/rustlib/src/rust/library/std").is_dir() {
        return Err(format!("the threads build rebuilds std, which needs rust-src for {NIGHTLY}: {}", install_hint()));
    }
    Ok(())
}

/// `cargo` on the threads build's toolchain and flags, in the workspace root.
pub fn cargo(target_dir: &Path) -> Command {
    let mut c = Command::new("rustup");
    c.args(["run", NIGHTLY, "cargo"]).current_dir(root());
    configure(&mut c, target_dir);
    c
}

/// Before `cargo xtask wasm --threads`.
pub fn preflight() -> Result<(), String> {
    check_toolchain()
}

/// `cargo xtask web-threads`: the threads build of the web app into `dist/web-threads`.
pub fn cmd_build(args: &[&str]) -> Result<(), String> {
    if let Some(other) = args.first() {
        return Err(format!("web-threads takes no arguments (got `{other}`)"));
    }
    check_toolchain()?;
    let trunk = Command::new("trunk").arg("--version").output().map_err(|e| format!("trunk not found ({e}): cargo install trunk --locked"))?;
    if !trunk.status.success() {
        return Err("trunk --version failed: cargo install trunk --locked".into());
    }
    let root = root();
    let dist = root.join("dist/web-threads");
    // Through rustup, so the cargo Trunk runs is the nightly's whatever is first on PATH.
    let mut c = Command::new("rustup");
    c.args(["run", NIGHTLY, "trunk", "build", "--release", "--locked", "--dist"]).arg(&dist).current_dir(root.join("apps/photocraft-web"));
    configure(&mut c, &target_dir("web-threads"));
    run(c, &format!("trunk build --release (apps/photocraft-web, {NIGHTLY}, atomics) into dist/web-threads"))?;
    let wasm = single_wasm(&dist)?;
    if !dist.join("snippets").is_dir() {
        return Err(format!("{}: no snippets/ (the Web Worker start-up that wasm-bindgen-rayon ships)", dist.display()));
    }
    let bytes = std::fs::metadata(&wasm).map_err(|e| format!("{}: {e}", wasm.display()))?.len();
    println!("threads build: {} ({bytes} bytes) in {}", wasm.file_name().and_then(|n| n.to_str()).unwrap_or("?"), dist.display());
    Ok(())
}

/// The one `*_bg.wasm` Trunk wrote into `dist`.
fn single_wasm(dist: &Path) -> Result<PathBuf, String> {
    let entries = std::fs::read_dir(dist).map_err(|e| format!("{}: {e}", dist.display()))?;
    let wasm: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.to_str().is_some_and(|s| s.ends_with("_bg.wasm"))).collect();
    match wasm.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(format!("{}: no .wasm", dist.display())),
        _ => Err(format!("{}: more than one .wasm", dist.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::{NIGHTLY, RUSTFLAGS};

    /// CI type-checks the threads build on the nightly the commands use; moving one without the
    /// other would test a toolchain nobody builds with.
    #[test]
    fn ci_installs_the_threads_nightly() {
        let ci = std::fs::read_to_string(crate::root().join(".github/workflows/ci.yml")).expect("ci.yml");
        assert!(ci.contains(&format!("toolchain: {NIGHTLY}")), "ci.yml's wasm-threads job should install {NIGHTLY}");
        assert!(ci.contains("cargo xtask wasm --threads"), "ci.yml should run the threads check");
    }

    #[test]
    fn the_flags_make_a_shared_memory_build() {
        for flag in ["+atomics", "+bulk-memory", "--shared-memory", "--import-memory", "--export=__wasm_init_tls"] {
            assert!(RUSTFLAGS.contains(flag), "{flag}");
        }
        assert!(!RUSTFLAGS.contains("  "), "one space between flags: {RUSTFLAGS}");
    }
}
