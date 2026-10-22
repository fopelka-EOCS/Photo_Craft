//! Command allowlist for the MCP server (`--commands <file>`).

use std::path::Path;

/// Largest allowlist file `load` reads.
pub const MAX_ALLOWLIST_BYTES: usize = 1 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
enum AllowPattern {
    Exact(String),
    Prefix(String),
}

/// A parsed allowlist of command ids and prefix globs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandAllowlist {
    patterns: Vec<AllowPattern>,
}

impl CommandAllowlist {
    /// Parse an allowlist from text.
    ///
    /// File format: one command id or trailing `*` prefix glob (`layer.*`) per line.
    /// Blank lines and `#` comments are ignored. Surrounding whitespace is trimmed.
    /// Only a trailing `*` glob is supported; anything else containing `*` is a parse
    /// error naming the line. An empty rule list is rejected.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut patterns = Vec::new();
        // Editors on Windows often save a UTF-8 BOM; it must not end up inside the first rule.
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        for raw_line in text.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.contains('#') {
                return Err(format!("invalid pattern `{line}`: `#` starts a comment only at the start of a line"));
            }
            if line == "*" {
                return Err("invalid pattern `*`: a bare `*` allows every command; omit `--commands` instead".to_string());
            }
            if line.contains('*') {
                let Some(prefix) = line.strip_suffix('*') else {
                    return Err(format!("invalid pattern `{line}`: only a trailing `*` glob is supported"));
                };
                if prefix.contains('*') {
                    return Err(format!("invalid pattern `{line}`: only a trailing `*` glob is supported"));
                }
                patterns.push(AllowPattern::Prefix(prefix.to_string()));
            } else {
                patterns.push(AllowPattern::Exact(line.to_string()));
            }
        }
        if patterns.is_empty() {
            return Err("commands allowlist is empty".to_string());
        }
        Ok(Self { patterns })
    }

    /// Load and parse an allowlist from a file path.
    pub fn load(path: &Path) -> Result<Self, String> {
        use std::io::Read;
        let name = path.display();
        let file = std::fs::File::open(path).map_err(|e| format!("{name}: {e}"))?;
        let mut bytes = Vec::new();
        file.take(MAX_ALLOWLIST_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|e| format!("{name}: {e}"))?;
        if bytes.len() > MAX_ALLOWLIST_BYTES {
            return Err(format!("{name}: allowlist exceeds {MAX_ALLOWLIST_BYTES} bytes"));
        }
        let text = String::from_utf8(bytes).map_err(|_| format!("{name}: not valid UTF-8"))?;
        Self::parse(&text).map_err(|e| format!("{name}: {e}"))
    }

    /// Check if a command id is permitted by this allowlist.
    pub fn allows(&self, command_id: &str) -> bool {
        self.patterns.iter().any(|p| match p {
            AllowPattern::Exact(exact) => exact == command_id,
            AllowPattern::Prefix(prefix) => command_id.starts_with(prefix),
        })
    }
}

/// The error every gated surface returns for a command the allowlist excludes.
pub fn refusal(command_id: &str) -> String {
    format!("command `{command_id}` is not allowed by `--commands`")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_exact_and_prefix_rules() {
        let text = "
            # Allowed commands
            file.new
            layer.*
        ";
        let al = CommandAllowlist::parse(text).unwrap();
        assert!(al.allows("file.new"));
        assert!(!al.allows("file.open"));
        assert!(al.allows("layer.new.layer"));
        assert!(al.allows("layer.duplicate"));
        assert!(!al.allows("layers.x"));
    }

    #[test]
    fn parse_ignores_a_leading_bom() {
        assert!(CommandAllowlist::parse("\u{feff}layer.*\n").unwrap().allows("layer.new"));
        assert!(CommandAllowlist::parse("\u{feff}# note\nfile.new").unwrap().allows("file.new"));
        assert!(CommandAllowlist::parse("\u{feff}").is_err());
    }

    #[test]
    fn parse_rejects_empty() {
        assert!(CommandAllowlist::parse("").is_err());
        assert!(CommandAllowlist::parse("   \n# only comments\n   ").is_err());
    }

    #[test]
    fn parse_rejects_bare_star_and_inline_comment() {
        assert!(CommandAllowlist::parse("*").unwrap_err().contains("bare `*`"));
        assert!(CommandAllowlist::parse("file.new # note").unwrap_err().contains("file.new # note"));
    }

    #[test]
    fn load_rejects_oversized_file() {
        let path = std::env::temp_dir().join(format!("pc-allow-big-{}.txt", std::process::id()));
        std::fs::write(&path, "a".repeat(MAX_ALLOWLIST_BYTES + 1)).unwrap();
        let err = CommandAllowlist::load(&path).unwrap_err();
        let _ = std::fs::remove_file(&path);
        assert!(err.contains("exceeds"), "{err}");
    }

    #[test]
    fn parse_rejects_malformed_globs() {
        for bad in ["*foo", "layer.*.blur", "foo*bar", "**", "layer.*.*"] {
            let err = CommandAllowlist::parse(bad).unwrap_err();
            assert!(err.contains(bad), "{bad}: {err}");
            assert!(err.contains("only a trailing `*` glob is supported"), "{bad}: {err}");
        }
    }
}
