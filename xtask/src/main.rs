//! Workspace tooling: `cargo xtask <command>`.
//!
//! Pure Rust (std + serde_json). External tools (`cargo`, `curl`, `tar`) are
//! invoked through `std::process::Command`.

mod corpus;
mod corpus_pins;
mod i18n_coverage;
mod ico;
mod layers;
mod perf;
mod pinned;
mod scorecard;
mod serve;
mod sha256;
mod stats;
mod threads;
mod version;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  layers          enforce the crate dependency layering (plan/architecture.md §3)
  wasm [--threads]
                  cargo check --target wasm32-unknown-unknown for the wasm-safe crates; --threads:
                  the same and the web app with WebAssembly threads (atomics, on the pinned nightly)
  web-threads     build the web app with WebAssembly threads into dist/web-threads (Trunk, pinned
                  nightly); PHOTOCRAFT_WEB_THREADS=1 packaging/web/package.sh packs it with the app
  serve <dir> [--port N] [--no-isolation]
                  serve a built site on http://127.0.0.1:8765 with the cross-origin isolation
                  headers the threads build needs (--no-isolation: without them)
  ci              fmt --check, clippy -D warnings, test, layers, wasm (stops at first failure)
  corpus [--all | --pngsuite | --psd | --psd-tools | --pixls | --photoshop] [--local] [--update-manifest]
                  show where test corpora live and their pins (xtask/src/corpus_pins.rs), or fetch
                  them into corpus/ (pinned commits, sha256-verified; --all = every corpus but pixls,
                  --photoshop --local copies from ../photocraft-corpus or $PHOTOCRAFT_CORPUS_REPO)
  test-corpus [-p <crate>]... [--changed] [--pixls] [--local] [-- <test args>]
                  fetch every corpus, then cargo test --release --features corpus on the corpus
                  crates; --changed runs only if psd/io/codecs/compose/gpu/text/format/raw changed;
                  --pixls (or -p raw) also fetches corpus/pixls and runs the raw corpus tests;
                  --local takes corpus/photoshop from the photocraft-corpus authoring clone
  stats [--exact] count tests and lines per crate (--exact: ask the test harness via `-- --list`)
  parity          Photoshop menu parity; rewrites docs/parity-checklist.md
  i18n-coverage   report stable UI translation coverage for registered languages
  perf [--quick] [--update-baseline] [--threshold PCT] [--bench NAME]... [--skip-build] [--reuse]
                  run the release benches, merge them by scenario id into target/perf/results.json,
                  check perf/budgets.toml and perf/baseline.json (non-zero on a broken budget or regression)
  scorecard [--check]
                  regenerate docs/scorecard.md (--check: fail if it is stale)
  version [set X.Y.Z[-pre]]
                  print the workspace version, or set it (Cargo.toml + Cargo.lock)
  ico <out.ico> <in.png>...
                  pack square PNGs (<= 256 px) into a Windows .ico (see packaging/icons.sh)
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();
    let result = match args.first().map(String::as_str) {
        Some("layers") => cmd_layers(),
        Some("wasm") => match rest.as_slice() {
            [] => cmd_wasm(false),
            ["--threads"] => cmd_wasm(true),
            _ => Err("usage: cargo xtask wasm [--threads]".into()),
        },
        Some("web-threads") => threads::cmd_build(&rest),
        Some("serve") => serve::run(&rest),
        Some("ci") => cmd_ci(),
        Some("corpus") => corpus::cmd(&rest),
        Some("test-corpus") => corpus::test_cmd(&rest),
        Some("stats") => stats::run(&root(), rest.contains(&"--exact")),
        Some("parity") => cmd_parity(),
        Some("i18n-coverage") => i18n_coverage::run(&root()),
        Some("perf") => perf::run(&root(), &rest),
        Some("scorecard") => scorecard::run(&root(), &rest),
        Some("version") => version::run(&root(), &rest),
        Some("ico") => ico::run(&rest),
        Some("-h" | "--help" | "help") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Workspace root (parent of the xtask crate).
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask has a parent dir").to_path_buf()
}

pub fn cargo() -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.current_dir(root());
    c
}

fn run(mut cmd: Command, what: &str) -> Result<(), String> {
    eprintln!("$ {what}");
    let status = cmd.status().map_err(|e| format!("{what}: failed to spawn: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{what}: exited with {status}")) }
}

pub fn metadata() -> Result<serde_json::Value, String> {
    let out = cargo().args(["metadata", "--format-version", "1", "--no-deps"]).output().map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: bad JSON: {e}"))
}

fn cmd_layers() -> Result<(), String> {
    let crates = layers::from_metadata(&metadata()?)?;
    println!("Dependency layering (plan/architecture.md §3)\n");
    println!("{:<28} {:<14} workspace deps", "crate", "layer");
    for c in &crates {
        let ws: Vec<String> = c
            .deps
            .iter()
            .filter(|d| d.workspace)
            .map(|d| {
                let k = match d.kind {
                    layers::DepKind::Normal => "",
                    layers::DepKind::Dev => " (dev)",
                    layers::DepKind::Build => " (build)",
                };
                format!("{}{k}", layers::short_name(&d.name))
            })
            .collect();
        println!("{:<28} {:<14} {}", c.name, layers::describe(layers::classify(&c.name)), ws.join(", "));
    }
    let violations = layers::check(&crates);
    println!();
    if violations.is_empty() {
        println!("OK: {} crates, no layering violations.", crates.len());
        Ok(())
    } else {
        println!("{} violation(s):", violations.len());
        for v in &violations {
            println!("  - {v}");
        }
        Err(format!("{} layering violation(s)", violations.len()))
    }
}

/// Workspace packages that must build for wasm32: all L0–L5 crates plus
/// the egui shell.
fn wasm_set() -> Result<Vec<String>, String> {
    let crates = layers::from_metadata(&metadata()?)?;
    Ok(crates
        .into_iter()
        .filter(|c| match layers::classify(&c.name) {
            Some(layers::Class::Layer(l)) => l <= 5 || layers::short_name(&c.name) == "ui-egui",
            Some(layers::Class::Standalone) => true,
            _ => false,
        })
        .map(|c| c.name)
        .collect())
}

/// Optional features that official builds enable and the web app ships, also checked for wasm32:
/// (package, feature).
const WASM_FEATURES: &[(&str, &str)] = &[("photocraft-codecs", "heif")];

/// Checked for wasm32 without threads only: the boot module that picks the threads build is itself
/// a plain module.
const WASM_FEATURES_PLAIN: &[(&str, &str)] = &[("photocraft-web-boot", "threads")];

/// Checked for wasm32 with threads only: the app's own threads start-up (it needs the browser
/// build's dependencies, so it isn't in the wasm-safe set).
const WASM_FEATURES_THREADS: &[(&str, &str)] = &[("photocraft-web", "heif")];

/// `threads`: with WebAssembly threads ([`threads`]), in its own target dir.
fn cmd_wasm(threads: bool) -> Result<(), String> {
    if threads {
        threads::preflight()?;
    }
    let target_dir = threads::target_dir("wasm-threads");
    let check = |pkg: &str, feature: Option<&str>| {
        let mut c = if threads { threads::cargo(&target_dir) } else { cargo() };
        c.args(["check", "--target", "wasm32-unknown-unknown", "-p", pkg]);
        if threads {
            // The nightly's resolver must not rewrite the lockfile the rest of the workspace uses.
            c.arg("--locked");
        }
        if let Some(f) = feature {
            c.args(["--features", f]);
        }
        let label = format!("{pkg}{}", feature.map(|f| format!(" --features {f}")).unwrap_or_default());
        let ok = run(c, &format!("cargo check --target wasm32-unknown-unknown{} -p {label}", if threads { " (threads)" } else { "" })).is_ok();
        (label, ok)
    };
    let mut results = Vec::new();
    for pkg in &wasm_set()? {
        results.push(check(pkg, None));
    }
    let extra = if threads { WASM_FEATURES_THREADS } else { WASM_FEATURES_PLAIN };
    for (pkg, feature) in WASM_FEATURES.iter().chain(extra) {
        results.push(check(pkg, Some(feature)));
    }
    println!("\nwasm32-unknown-unknown check{}:", if threads { " with threads (atomics)" } else { "" });
    for (p, ok) in &results {
        println!("  {:<6} {p}", if *ok { "ok" } else { "FAIL" });
    }
    let failed = results.iter().filter(|r| !r.1).count();
    if failed == 0 { Ok(()) } else { Err(format!("{failed} crate(s) failed the wasm check")) }
}

fn cmd_parity() -> Result<(), String> {
    let mut c = cargo();
    c.args(["run", "-q", "-p", "photocraft-ui-egui", "--example", "parity", "--", "--write", "docs/parity-checklist.md"]);
    run(c, "cargo run -p photocraft-ui-egui --example parity")
}

fn cmd_ci() -> Result<(), String> {
    type Step = (&'static str, Box<dyn Fn() -> Result<(), String>>);
    let steps: Vec<Step> = vec![
        (
            "fmt",
            Box::new(|| {
                let mut c = cargo();
                c.args(["fmt", "--all", "--", "--check"]);
                run(c, "cargo fmt --all -- --check")
            }),
        ),
        (
            "clippy",
            Box::new(|| {
                let mut c = cargo();
                c.args(["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]);
                run(c, "cargo clippy --workspace --all-targets -- -D warnings")
            }),
        ),
        (
            "test",
            Box::new(|| {
                let mut c = cargo();
                c.args(["test", "--workspace"]);
                run(c, "cargo test --workspace")
            }),
        ),
        ("layers", Box::new(cmd_layers)),
        ("wasm", Box::new(|| cmd_wasm(false))),
    ];
    let mut done = Vec::new();
    for (name, f) in &steps {
        eprintln!("\n=== ci: {name} ===");
        if let Err(e) = f() {
            println!("\nCI summary:");
            for d in &done {
                println!("  ok    {d}");
            }
            println!("  FAIL  {name}: {e}");
            for (n, _) in steps.iter().skip(done.len() + 1) {
                println!("  skip  {n}");
            }
            return Err(format!("ci failed at `{name}`"));
        }
        done.push(*name);
    }
    println!("\nCI summary: all {} steps passed ({})", done.len(), done.join(", "));
    Ok(())
}
