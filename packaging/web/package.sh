#!/usr/bin/env bash
# Build the browser version and zip it:  $DIST/photocraft-web-<version>.zip
#
# Usage: packaging/web/package.sh [--skip-build]
#   PHOTOCRAFT_WEB_THREADS=1  also pack the threads build (WebAssembly threads, `cargo xtask
#                             web-threads`; needs its nightly), which the page starts where it is
#                             cross-origin isolated (README.md › Multithreaded build)
#
# Needs: trunk (brew install trunk / cargo install trunk --locked) and the wasm32-unknown-unknown
# target. The zip holds a self-contained static site in photocraft-web-<version>/ that works
# from any URL path and inside an <iframe>. Hosting notes: packaging/web/README.md.
#
# The site holds the app's wasm gzipped and cut into parts (app/, and app-threads/ for the threads
# build), started by the small photocraft-web-boot module, so no file is larger than a part however
# large the app grows (issue #5302). dist/web still holds the app as Trunk builds it, in one .wasm.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/web"
THREADS="${PHOTOCRAFT_WEB_THREADS:-0}"
case "$THREADS" in
  0 | 1) ;;
  *) echo "error: PHOTOCRAFT_WEB_THREADS is 0 or 1, not '$THREADS'" >&2; exit 1 ;;
esac

if [ "${1:-}" != "--skip-build" ]; then
  command -v trunk >/dev/null || { echo "error: trunk not found (cargo install trunk --locked)" >&2; exit 1; }
  (cd "$ROOT/apps/photocraft-web" && trunk build --release)
  if [ "$THREADS" = 1 ]; then
    (cd "$ROOT" && cargo xtask web-threads)
    (cd "$ROOT/apps/photocraft-web-boot" && trunk build --release --features threads)
  else
    (cd "$ROOT/apps/photocraft-web-boot" && trunk build --release)
  fi
fi

APP="$ROOT/dist/web"                 # the app: index.html, wasm-bindgen glue, one .wasm
APP_THREADS="$ROOT/dist/web-threads" # the same with threads (cargo xtask web-threads), and snippets/
BOOT="$ROOT/dist/web-boot"           # the page that ships, and the module that starts the app from parts
SITES=("$APP" "$BOOT")
[ "$THREADS" = 0 ] || SITES+=("$APP_THREADS")
for site in "${SITES[@]}"; do
  [ -f "$site/index.html" ] || { echo "error: $site/index.html missing; run without --skip-build" >&2; exit 1; }
  # Paths must be relative so the site works under any prefix (public_url = "./" in Trunk.toml).
  if grep -Eq '(src|href)="/[^/]' "$site/index.html"; then
    echo "error: $site/index.html has root-absolute URLs; it would break when served from a sub-path" >&2
    exit 1
  fi
done
# The boot module imports the threads build's glue only when built with `--features threads`.
boot_threads=0
grep -q 'app-threads/photocraft-web\.js' "$BOOT"/photocraft-web-boot-*.js 2>/dev/null && boot_threads=1
if [ "$boot_threads" != "$THREADS" ]; then
  echo "error: $BOOT was built for PHOTOCRAFT_WEB_THREADS=$boot_threads; run without --skip-build" >&2
  exit 1
fi

# Size limit (issues #198, #5302): Cloudflare Pages/Workers reject any single file over 25 MiB,
# and other hosts and CDNs have similar caps. No file in the site may come near that; the app's
# wasm, the one file that does, ships as parts of PART_BYTES. Override only to investigate.
MAX_FILE_BYTES="${PHOTOCRAFT_WEB_MAX_FILE_BYTES:-25165824}" # 24 MiB
PART_BYTES="${PHOTOCRAFT_WEB_PART_BYTES:-20971520}"         # 20 MiB
if [ "$PART_BYTES" -le 0 ] || [ "$PART_BYTES" -gt "$MAX_FILE_BYTES" ]; then
  echo "error: a part of $PART_BYTES bytes doesn't fit the $MAX_FILE_BYTES-byte file limit" >&2
  exit 1
fi

NAME="photocraft-web-$VERSION"
WORK="$CARGO_TARGET_DIR/web-package"
rm -rf "$WORK"
mkdir -p "$WORK/$NAME"
cp -R "$BOOT/." "$WORK/$NAME/"
size() { wc -c <"$1" | tr -d ' '; }

# pack <Trunk's dist> <folder in the site>: the app's glue, its wasm in parts, and parts.txt.
pack() {
  local from="$1" to="$WORK/$NAME/$2" wasm="" stem glue
  for f in "$from"/*_bg.wasm; do
    [ -f "$f" ] || continue
    [ -z "$wasm" ] || { echo "error: more than one .wasm in $from; remove it and run without --skip-build" >&2; exit 1; }
    wasm="$f"
  done
  [ -n "$wasm" ] || { echo "error: no .wasm in $from" >&2; exit 1; }
  stem="$(basename "$wasm" _bg.wasm)" # photocraft-web-<content hash>
  glue="$from/$stem.js"
  [ -f "$glue" ] || { echo "error: $glue missing" >&2; exit 1; }
  mkdir -p "$to"
  # The boot module imports the app's glue by this name (it can't know Trunk's content hash).
  cp "$glue" "$to/photocraft-web.js"
  # Modules the glue imports from next to itself (the threads build's Web Worker start-up).
  [ ! -d "$from/snippets" ] || cp -R "$from/snippets" "$to/"
  # The parts keep the content hash, so they can be cached forever. Their extension is one no
  # server knows: a server that recognised gzip might decode it or label it, and the page inflates
  # the parts itself.
  gzip -9 -n -c "$wasm" | (cd "$to" && split -b "$PART_BYTES" - "$stem.part-")
  {
    echo "photocraft-parts 1"
    for part in "$to/$stem".part-*; do basename "$part"; done
  } >"$to/parts.txt"
  local wasm_bytes gzip_bytes part_count
  wasm_bytes=$(size "$wasm")
  gzip_bytes=$(cat "$to/$stem".part-* | wc -c | tr -d ' ')
  part_count=$(($(wc -l <"$to/parts.txt") - 1))
  echo "$2/: $(basename "$wasm"): $wasm_bytes bytes ($((wasm_bytes / 1048576)) MiB), shipped as $gzip_bytes bytes of gzip in $part_count part(s) of up to $PART_BYTES"
}
pack "$APP" app
[ "$THREADS" = 0 ] || pack "$APP_THREADS" app-threads

# Sample server configs (MIME type, caching, compression); harmless where unused.
cp "$HERE/_headers" "$HERE/.htaccess" "$WORK/$NAME/"
if [ "$THREADS" = 1 ]; then
  # The threads build needs a cross-origin isolated page (shared WebAssembly memory). Where the
  # host doesn't send these, the page starts the single-threaded build in app/ instead.
  cat >>"$WORK/$NAME/_headers" <<'EOF'

# The threads build (app-threads/) runs only where the page is cross-origin isolated: these make it
# so. Without them the page starts the single-threaded build (app/) instead.
/*
  Cross-Origin-Opener-Policy: same-origin
  Cross-Origin-Embedder-Policy: require-corp
  Cross-Origin-Resource-Policy: same-origin

/app-threads/*
  Cache-Control: public, max-age=31536000, immutable

/app-threads/photocraft-web.js
  ! Cache-Control
  Cache-Control: no-cache

/app-threads/parts.txt
  ! Cache-Control
  Cache-Control: no-cache
EOF
  cat >>"$WORK/$NAME/.htaccess" <<'EOF'

# The threads build (app-threads/) runs only where the page is cross-origin isolated: these make it
# so. Without them the page starts the single-threaded build (app/) instead.
<IfModule mod_headers.c>
  Header set Cross-Origin-Opener-Policy "same-origin"
  Header set Cross-Origin-Embedder-Policy "require-corp"
  Header set Cross-Origin-Resource-Policy "same-origin"
</IfModule>
EOF
fi
cp "$HERE/README.md" "$WORK/$NAME/HOSTING.md"
copy_docs "$WORK/$NAME"
# Optional served fonts (README.md › Fonts): PHOTOCRAFT_WEB_FONTS_DIR=<a craft-fonts checkout, or any
# folder with a fonts/manifest.txt in its format> copies the manifest, every font and licence file
# it lists into the site, at the same paths. The wasm is unchanged; the app fetches fonts on demand.
if [ -n "${PHOTOCRAFT_WEB_FONTS_DIR:-}" ]; then
  FONTS_MANIFEST="$PHOTOCRAFT_WEB_FONTS_DIR/fonts/manifest.txt"
  [ -f "$FONTS_MANIFEST" ] || { echo "error: $FONTS_MANIFEST missing (PHOTOCRAFT_WEB_FONTS_DIR)" >&2; exit 1; }
  mkdir -p "$WORK/$NAME/fonts"
  cp "$FONTS_MANIFEST" "$WORK/$NAME/fonts/manifest.txt"
  # Field 3 is the font file, field 6 (if any) its licence file; both relative to the folder.
  awk -F' [|] ' '!/^[[:space:]]*(#|$)/ { print $3; if ($6 != "") print $6 }' "$FONTS_MANIFEST" | sort -u | while IFS= read -r f; do
    case "$f" in
      "" | /* | *..* | *\\* | *://*) echo "error: $FONTS_MANIFEST: '$f' is not a relative path inside the folder" >&2; exit 1 ;;
    esac
    mkdir -p "$WORK/$NAME/$(dirname "$f")"
    cp "$PHOTOCRAFT_WEB_FONTS_DIR/$f" "$WORK/$NAME/$f"
  done
  echo "served fonts: $(grep -cvE '^[[:space:]]*(#|$)' "$FONTS_MANIFEST") manifest lines from $PHOTOCRAFT_WEB_FONTS_DIR"
fi
largest=0
while IFS= read -r file; do
  bytes=$(size "$file")
  [ "$bytes" -le "$largest" ] || largest=$bytes
  if [ "$bytes" -gt "$MAX_FILE_BYTES" ]; then
    echo "error: ${file#"$WORK/$NAME/"} is $bytes bytes, over the $MAX_FILE_BYTES-byte limit (Cloudflare's per-file cap is 25 MiB)" >&2
    exit 1
  fi
done < <(find "$WORK/$NAME" -type f)
echo "largest file in the site: $largest bytes (limit $MAX_FILE_BYTES)"
rm -f "$DIST/$NAME.zip"
(cd "$WORK" && zip -qr9 "$DIST/$NAME.zip" "$NAME")
echo "wrote $DIST/$NAME.zip"
ls -lh "$DIST/$NAME.zip"
