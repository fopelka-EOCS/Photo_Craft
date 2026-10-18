//! The browser threads build (wasm with `atomics`, `cargo xtask web-threads`): before the app
//! starts, a rayon pool of Web Workers (wasm-bindgen-rayon) shares the module's memory, so the
//! steps that run on every core natively run on these workers in the browser.
//!
//! The pool starts before anything touches rayon, which would otherwise make its one-thread
//! fallback pool for good. It has one worker per core but one (the page's own thread keeps the
//! UI), at most [`MAX_DEFAULT`]; `?threads=N` asks for N. The packaged site starts this build only
//! on a cross-origin isolated page and the single-threaded one otherwise (`photocraft-web-boot`).

/// Workers at most unless `?threads=` asks for more: past this, more workers mostly add start-up
/// time and memory.
const MAX_DEFAULT: usize = 16;

/// Workers at most, whatever `?threads=` asks.
const MAX: usize = 64;

/// The `threads=N` value in a page's query (`?webgl&threads=4`), if one parses.
pub fn requested(query: &str) -> Option<usize> {
    query.trim_start_matches('?').split('&').find_map(|pair| pair.strip_prefix("threads=")?.parse().ok())
}

/// Workers for a device with `cores` logical cores, opened with `query`.
pub fn pool_size(cores: usize, query: &str) -> usize {
    match requested(query) {
        Some(n) => n.clamp(1, MAX),
        None => cores.saturating_sub(1).clamp(1, MAX_DEFAULT),
    }
}

/// Starts the worker pool, then the app. The workers instantiate this same module, and its
/// `main` runs there too: only the page, which has a window, starts anything.
#[cfg(all(target_arch = "wasm32", target_feature = "atomics"))]
pub fn start() {
    let Some(window) = web_sys::window() else { return };
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    let cores = window.navigator().hardware_concurrency();
    let cores = if cores.is_finite() && cores >= 1.0 { cores.min(1024.0) as usize } else { 1 };
    let workers = pool_size(cores, &window.location().search().unwrap_or_default());
    wasm_bindgen_futures::spawn_local(async move {
        let t0 = js_sys::Date::now();
        match wasm_bindgen_futures::JsFuture::from(wasm_bindgen_rayon::init_thread_pool(workers)).await {
            Ok(_) => log::info!("worker pool: {} threads, started in {:.0} ms", rayon::current_num_threads(), js_sys::Date::now() - t0),
            // rayon then runs everything on the page's thread, as the single-threaded build does.
            Err(e) => log::warn!("no worker threads, running on one: {e:?}"),
        }
        crate::web::start();
    });
}

#[cfg(test)]
mod tests {
    use super::{pool_size, requested};

    #[test]
    fn the_pool_leaves_a_core_for_the_page() {
        assert_eq!(pool_size(8, ""), 7);
        assert_eq!(pool_size(2, ""), 1);
        assert_eq!(pool_size(1, ""), 1, "one core still gets one worker");
        assert_eq!(pool_size(0, ""), 1, "a browser that reports no cores");
        assert_eq!(pool_size(128, ""), 16, "capped by default");
    }

    #[test]
    fn a_threads_query_sets_the_pool_size() {
        assert_eq!(requested("?threads=4"), Some(4));
        assert_eq!(requested("?webgl&threads=12&cpu"), Some(12));
        assert_eq!(requested("?webgl"), None);
        assert_eq!(requested("?threads=many"), None, "unreadable: the default");
        assert_eq!(requested("?mythreads=3"), None, "a different key");
        assert_eq!(pool_size(8, "?threads=24"), 24, "past the default cap when asked");
        assert_eq!(pool_size(8, "?threads=0"), 1);
        assert_eq!(pool_size(8, "?threads=100000"), 64);
        assert_eq!(pool_size(8, "?threads="), 7);
    }
}
