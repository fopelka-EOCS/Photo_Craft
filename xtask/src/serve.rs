//! `cargo xtask serve <dir> [--port N] [--no-isolation]`: serves a built web site on
//! http://127.0.0.1 for trying it locally, with the cross-origin isolation headers the threads
//! build needs (COOP, COEP, CORP). `--no-isolation` leaves them out, so the page starts the
//! single-threaded build as on a host that doesn't send them.
//!
//! A local test server only: GET and HEAD of files inside `dir`, one thread per connection, no
//! caching (`Cache-Control: no-cache`, so a rebuild shows on reload).

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};

const DEFAULT_PORT: u16 = 8765;

pub fn run(args: &[&str]) -> Result<(), String> {
    let mut dir = None;
    let mut port = DEFAULT_PORT;
    let mut isolation = true;
    let mut it = args.iter();
    while let Some(&arg) = it.next() {
        match arg {
            "--port" => port = it.next().and_then(|p| p.parse().ok()).ok_or("--port needs a number")?,
            "--no-isolation" => isolation = false,
            _ if dir.is_none() && !arg.starts_with('-') => dir = Some(PathBuf::from(arg)),
            _ => return Err(format!("serve: unexpected `{arg}`\nusage: cargo xtask serve <dir> [--port N] [--no-isolation]")),
        }
    }
    let dir = dir.ok_or("usage: cargo xtask serve <dir> [--port N] [--no-isolation]")?;
    let dir = dir.canonicalize().map_err(|e| format!("{}: {e}", dir.display()))?;
    if !dir.join("index.html").is_file() {
        return Err(format!("{}: no index.html", dir.display()));
    }
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("127.0.0.1:{port}: {e}"))?;
    println!("serving {} on http://127.0.0.1:{port}/ ({}; Ctrl-C stops)", dir.display(), if isolation { "cross-origin isolated" } else { "not isolated" });
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let dir = dir.clone();
        std::thread::spawn(move || {
            let _ = answer(stream, &dir, isolation);
        });
    }
    Ok(())
}

fn answer(mut stream: TcpStream, dir: &Path, isolation: bool) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    // The headers aren't needed; read them so the client sees a complete exchange.
    let mut header = String::new();
    let mut seen = 0usize;
    while reader.read_line(&mut header)? > 2 && seen < 100 {
        header.clear();
        seen += 1;
    }
    let mut words = line.split_whitespace();
    let (method, target) = (words.next().unwrap_or(""), words.next().unwrap_or(""));
    let head = method == "HEAD";
    let (status, kind, body) = if method != "GET" && !head {
        ("405 Method Not Allowed", "text/plain; charset=utf-8", b"GET or HEAD only\n".to_vec())
    } else {
        match resolve(dir, target).and_then(|p| std::fs::read(&p).ok().map(|b| (p, b))) {
            Some((path, bytes)) => ("200 OK", mime(&path), bytes),
            None => ("404 Not Found", "text/plain; charset=utf-8", b"not found\n".to_vec()),
        }
    };
    let mut out = format!("HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
    for (name, value) in headers(isolation) {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str("\r\n");
    stream.write_all(out.as_bytes())?;
    if !head {
        stream.write_all(&body)?;
    }
    stream.flush()
}

/// The file `target` (a request path) names inside `dir`, if it is one: `index.html` for a
/// folder, never anything outside `dir`.
fn resolve(dir: &Path, target: &str) -> Option<PathBuf> {
    let path = target.split(['?', '#']).next()?;
    let mut out = dir.to_path_buf();
    for segment in path.split('/') {
        let segment = decode(segment)?;
        match segment.as_str() {
            "" | "." => {}
            ".." => return None,
            s if s.contains(['/', '\\']) => return None,
            s => out.push(s),
        }
    }
    if out.is_dir() {
        out.push("index.html");
    }
    // Symbolic links could still lead out of `dir`.
    out.canonicalize().ok().filter(|p| p.starts_with(dir) && p.is_file())
}

/// `%xx` escapes decoded, or `None` for a malformed one or bytes that aren't UTF-8.
fn decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The `Content-Type` for `path`, by extension. Anything else (the app's `.part-aa` files among
/// them) is `application/octet-stream`, as on a typical host.
fn mime(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css",
        Some("json") => "application/json",
        Some("txt" | "md") => "text/plain; charset=utf-8",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// Headers on every answer.
fn headers(isolation: bool) -> Vec<(&'static str, &'static str)> {
    let mut h = vec![("Cache-Control", "no-cache"), ("X-Content-Type-Options", "nosniff")];
    if isolation {
        h.extend([
            ("Cross-Origin-Opener-Policy", "same-origin"),
            ("Cross-Origin-Embedder-Policy", "require-corp"),
            ("Cross-Origin-Resource-Policy", "same-origin"),
        ]);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::{decode, headers, mime, resolve};
    use std::path::Path;

    fn site() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("xtask-serve-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("app")).expect("mkdir");
        std::fs::write(dir.join("index.html"), "<!doctype html>").expect("write");
        std::fs::write(dir.join("app/parts.txt"), "photocraft-parts 1\n").expect("write");
        std::fs::write(dir.join("app/my file.js"), "").expect("write");
        dir.canonicalize().expect("canonical")
    }

    #[test]
    fn requests_map_to_files_inside_the_site_only() {
        let dir = site();
        assert_eq!(resolve(&dir, "/"), Some(dir.join("index.html")));
        assert_eq!(resolve(&dir, "/?threads=4"), Some(dir.join("index.html")), "the query is not part of the path");
        assert_eq!(resolve(&dir, "/app/parts.txt"), Some(dir.join("app/parts.txt")));
        assert_eq!(resolve(&dir, "/app/my%20file.js"), Some(dir.join("app/my file.js")));
        assert_eq!(resolve(&dir, "/app/"), None, "a folder without index.html");
        assert_eq!(resolve(&dir, "/missing.js"), None);
        for escape in ["/../etc/passwd", "/app/../../etc/passwd", "/%2e%2e/etc/passwd", "/app%2f..%2f..%2fetc", "/a%5c..%5cb", "/%zz", "/%c3"] {
            assert_eq!(resolve(&dir, escape), None, "{escape}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn escapes_decode_or_refuse() {
        assert_eq!(decode("a%20b").as_deref(), Some("a b"));
        assert_eq!(decode("%E4%B8%AD").as_deref(), Some("中"));
        assert_eq!(decode("%"), None);
        assert_eq!(decode("%4"), None);
        assert_eq!(decode("%ff"), None, "not UTF-8");
    }

    #[test]
    fn types_and_isolation_headers() {
        assert_eq!(mime(Path::new("x/photocraft-web.js")), "text/javascript");
        assert_eq!(mime(Path::new("x_bg.wasm")), "application/wasm");
        assert_eq!(mime(Path::new("app/photocraft-web-0123.part-aa")), "application/octet-stream");
        assert_eq!(mime(Path::new("INDEX.HTML")), "text/html; charset=utf-8");
        let isolated = headers(true);
        assert!(isolated.contains(&("Cross-Origin-Opener-Policy", "same-origin")));
        assert!(isolated.contains(&("Cross-Origin-Embedder-Policy", "require-corp")));
        assert!(!headers(false).iter().any(|(name, _)| name.starts_with("Cross-Origin")), "--no-isolation");
    }
}
