//! The page's URL query: renderer flags (`?cpu`, `?webgl`) and `?open=<url>`, which opens the image
//! at that URL when the page loads, so another site can link straight into the editor.
//!
//! The browser fetches the URL, so a cross-origin server must allow it with CORS
//! (`Access-Control-Allow-Origin`). The value must be percent-encoded when it has its own query
//! (`?open=https%3A%2F%2Fexample.com%2Fimg%3Fid%3D7`).

/// The query's `key=value` pairs, percent-decoded (`+` is a space), in order. A key without `=`
/// has an empty value.
pub fn parse_query(search: &str) -> Vec<(String, String)> {
    search
        .strip_prefix('?')
        .unwrap_or(search)
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decode(key), decode(value))
        })
        .collect()
}

/// Whether the query has `name` as a key (`?webgl`, `?webgl=1`). Values don't count, so
/// `?open=https://example.com/cpu.png` doesn't force the CPU canvas.
pub fn has_flag(query: &[(String, String)], name: &str) -> bool {
    query.iter().any(|(key, _)| key == name)
}

/// The URL to open from `?open=`: `None` without one. Anything but an absolute `http:` or
/// `https:` URL is an error that carries the value as given.
pub fn open_target(query: &[(String, String)]) -> Option<Result<String, String>> {
    let (_, url) = query.iter().find(|(key, _)| key == "open")?;
    let url = url.trim();
    let host = url.split_once("://").and_then(|(scheme, rest)| matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https").then_some(rest));
    Some(match host {
        Some(rest) if has_host(rest) => Ok(url.to_string()),
        _ => Err(url.to_string()),
    })
}

/// Whether the authority at the start of `after_scheme` (`user@host:port`, up to the first `/`,
/// `?` or `#`) names a host: `https://?x`, `https://:80/a.png` and `https://user@/a.png` don't.
fn has_host(after_scheme: &str) -> bool {
    let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, host_port)| host_port);
    let host = match host_port.strip_prefix('[') {
        // An IPv6 literal: `[::1]:8080`.
        Some(v6) => v6.split_once(']').map_or("", |(host, _)| host),
        None => host_port.split_once(':').map_or(host_port, |(host, _)| host),
    };
    !host.is_empty()
}

/// The document's name for an image fetched from `url`: the last segment of its path. When that
/// has no extension, or one that doesn't match the content (an image proxy's `/img?id=7`), the
/// detected format's extension is added.
pub fn file_name(url: &str, detected_exts: Option<&[&str]>) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let after_host = path.split_once("://").map_or(path, |(_, rest)| rest.split_once('/').map_or("", |(_, p)| p));
    let last = decode(after_host.rsplit('/').find(|s| !s.is_empty()).unwrap_or(""));
    // Not a path for anything that later splits names on separators.
    let last = last.replace(['/', '\\'], "_");
    let stem = if last.trim().is_empty() { "image".to_string() } else { last };
    let Some(exts) = detected_exts.filter(|exts| !exts.is_empty()) else { return stem };
    let has_match = stem.rsplit_once('.').is_some_and(|(base, ext)| !base.is_empty() && exts.iter().any(|e| e.eq_ignore_ascii_case(ext)));
    match exts.first() {
        Some(ext) if !has_match => format!("{stem}.{ext}"),
        _ => stem,
    }
}

/// Percent-decodes `s`, with `+` as a space. Malformed escapes are kept as they are, and bytes
/// that aren't UTF-8 become U+FFFD.
fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let escaped = if b == b'%' { bytes.get(i + 1..i + 3).and_then(|hex| u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()) } else { None };
        match (b, escaped) {
            (_, Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (b'+', None) => {
                out.push(b' ');
                i += 1;
            }
            (b, None) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pairs_and_bare_flags() {
        let q = parse_query("?webgl&open=https%3A%2F%2Fexample.com%2Fa.png&x=1+2");
        assert_eq!(q, vec![("webgl".into(), "".into()), ("open".into(), "https://example.com/a.png".into()), ("x".into(), "1 2".into())]);
        assert!(parse_query("").is_empty());
        assert!(parse_query("?").is_empty());
    }

    #[test]
    fn flags_match_keys_not_values() {
        let q = parse_query("?open=https://example.com/cpu.png&webgl=1");
        assert!(has_flag(&q, "webgl"));
        assert!(!has_flag(&q, "cpu"));
    }

    #[test]
    fn open_target_accepts_only_http_urls() {
        let target = |s: &str| open_target(&parse_query(s));
        assert_eq!(target("?webgl"), None);
        assert_eq!(target("?open=https://example.com/a.png"), Some(Ok("https://example.com/a.png".into())));
        assert_eq!(target("?open=HTTP://example.com/a.png"), Some(Ok("HTTP://example.com/a.png".into())));
        // An unencoded inner query is cut at its `&`; encoded it arrives whole.
        assert_eq!(target("?open=https%3A%2F%2Fp.example%2Fimg%3Fu%3Dx%26w%3D2"), Some(Ok("https://p.example/img?u=x&w=2".into())));
        assert_eq!(target("?open=javascript:alert(1)"), Some(Err("javascript:alert(1)".into())));
        for bad in [
            "?open=",
            "?open=javascript:alert(1)",
            "?open=data:image/png;base64,AAAA",
            "?open=/local.png",
            "?open=ftp://example.com/a.png",
            "?open=https://",
            "?open=https:///a.png",
        ] {
            assert!(matches!(target(bad), Some(Err(_))), "{bad}");
        }
    }

    #[test]
    fn open_target_needs_a_host() {
        let target = |url: &str| open_target(&[("open".to_string(), url.to_string())]);
        for good in [
            "https://example.com",
            "https://user:pw@example.com/a.png",
            "http://example.com:8080/a.png?x#y",
            "http://[::1]:8765/a.png",
            "https://[2001:db8::1]/a.png",
        ] {
            assert_eq!(target(good), Some(Ok(good.to_string())), "{good}");
        }
        for bad in
            ["https://?x", "https://#a", "https://:80/a.png", "https://user@/a.png", "https://user:pw@:80/a.png", "https://[]/a.png", "https://[::1/a.png"]
        {
            assert_eq!(target(bad), Some(Err(bad.to_string())), "{bad}");
        }
    }

    #[test]
    fn names_come_from_the_path() {
        assert_eq!(file_name("https://example.com/photos/Great%20Wave.jpg?w=200#top", Some(&["jpg", "jpeg"])), "Great Wave.jpg");
        assert_eq!(file_name("https://example.com/a/b.JPEG", Some(&["jpg", "jpeg"])), "b.JPEG");
        assert_eq!(file_name("https://example.com/a/b.psd", None), "b.psd");
    }

    #[test]
    fn names_gain_the_detected_extension() {
        assert_eq!(file_name("https://proxy.example/img?u=https://x/y.png", Some(&["png", "apng"])), "img.png");
        assert_eq!(file_name("https://example.com/", Some(&["webp"])), "image.webp");
        assert_eq!(file_name("https://example.com", Some(&["webp"])), "image.webp");
        assert_eq!(file_name("https://example.com/photo.php", Some(&["jpg", "jpeg"])), "photo.php.jpg");
        assert_eq!(file_name("https://example.com/", None), "image");
    }

    #[test]
    fn names_never_hold_separators() {
        assert_eq!(file_name("https://example.com/a%2F..%2Fb%5Cc.png", Some(&["png"])), "a_.._b_c.png");
    }

    #[test]
    fn decoding_survives_malformed_input() {
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("%zz%4"), "%zz%4");
        assert_eq!(decode("%E2%82%AC"), "€");
        assert_eq!(decode("%FF"), "\u{FFFD}");
        assert_eq!(decode("é%C3%A9"), "éé");
    }
}
