//! Object storage (ZIP or directory), incremental writer and loader.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::SystemTime;

use photocraft_color::{PixelFormat, SampleType};
use photocraft_doc::Document;
use photocraft_raster::{Rgba8Image, Tile};

use crate::convert::{self, Fetch, Loader, Sink, is_valid_hash, swap_to_le};
use crate::manifest::{ContentM, FORMAT_VERSION, Hash, LayerM, Manifest};
use crate::zip::{ZipReader, ZipWriter};
use crate::{FormatError, LoadOptions, Result, SaveOptions};

pub(crate) const MANIFEST: &str = "manifest.json";
pub(crate) const THUMB: &str = "thumb.png";
pub(crate) const COMPOSITE: &str = "composite/preview.png";

fn tile_path(h: &str) -> String {
    format!("tiles/{h}.zst")
}
fn blob_path(h: &str) -> String {
    format!("blobs/{h}.zst")
}

#[cfg(not(feature = "native-zstd"))]
type Compressor<'a> = std::marker::PhantomData<&'a ()>;
#[cfg(feature = "native-zstd")]
type Compressor<'a> = photocraft_codecs::ZstdEncoder<'a>;

/// Creates the selected encoder for one compression worker or directory save.
fn new_compressor<'a>() -> Result<Compressor<'a>> {
    #[cfg(feature = "native-zstd")]
    {
        #[cfg(test)]
        tests::COMPRESSORS.with(|n| n.set(n.get() + 1));
        return Ok(Compressor::new()?);
    }
    #[cfg(not(feature = "native-zstd"))]
    Ok(Default::default())
}

/// Decoder workspaces live for one document load, not across imports.
#[derive(Default)]
struct Decompressor {
    #[cfg(feature = "native-zstd")]
    native: Option<photocraft_codecs::ZstdDecoder>,
    #[cfg(not(feature = "native-zstd"))]
    rust: ruzstd::decoding::FrameDecoder,
}

impl Decompressor {
    /// Decodes one frame, checking the output budget and optional exact length.
    fn decode(&mut self, data: &[u8], max: usize, exact: Option<usize>, what: &str) -> Result<Vec<u8>> {
        let zstd = |e: &dyn std::fmt::Display| FormatError::corrupt(format!("{what}: zstd: {e}"));
        #[cfg(feature = "native-zstd")]
        let out = {
            let decoder = match &mut self.native {
                Some(decoder) => decoder,
                slot @ None => slot.insert(photocraft_codecs::ZstdDecoder::new()?),
            };
            let decoded = if exact.is_some() { decoder.decode(data, max) } else { decoder.decode_bounded(data, max) };
            decoded.map_err(|e| {
                if e.kind() == std::io::ErrorKind::FileTooLarge { FormatError::LimitExceeded(format!("{what} exceeds {max} bytes")) } else { zstd(&e) }
            })?
        };
        #[cfg(not(feature = "native-zstd"))]
        let out = {
            let mut dec = ruzstd::decoding::StreamingDecoder::new_with_decoder(data, &mut self.rust).map_err(|e| zstd(&e))?;
            let mut out = Vec::with_capacity(exact.unwrap_or(0).min(max).min(64 << 20));
            // `max` may be `usize::MAX`: one byte past it must not overflow.
            let limit = u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1);
            (&mut dec).take(limit).read_to_end(&mut out).map_err(|e| zstd(&e))?;
            out
        };
        match exact {
            Some(n) if out.len() != n => Err(FormatError::corrupt(format!("{what}: expected {n} bytes, got {}", out.len()))),
            None if out.len() > max => Err(FormatError::LimitExceeded(format!("{what} exceeds {max} bytes"))),
            _ => Ok(out),
        }
    }
}

/// Decodes a standalone frame with an output budget and optional exact length.
fn decompress_bounded(data: &[u8], max: usize, exact: Option<usize>, what: &str) -> Result<Vec<u8>> {
    Decompressor::default().decode(data, max, exact, what)
}

fn decompress(data: &[u8], expected: usize, what: &str) -> Result<Vec<u8>> {
    decompress_bounded(data, expected, Some(expected), what)
}

fn hash_bytes(b: &[u8]) -> Hash {
    blake3::hash(b).to_hex().to_string()
}

fn png(img: &Rgba8Image) -> Result<Vec<u8>> {
    let image = photocraft_codecs::Image::from_u8(img.width, img.height, photocraft_codecs::ChannelLayout::Rgba, img.pixels.clone())
        .map_err(|e| FormatError::Unsupported(format!("preview: {e}")))?;
    photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).map_err(|e| FormatError::Unsupported(format!("preview: {e}")))
}

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------

pub trait ByteSource {
    fn get(&self, path: &str, max: usize) -> Result<Vec<u8>>;

    /// Reads with the same limits as [`Self::get`], borrowing when the source permits it.
    /// Sources that only implement `get` keep returning owned buffers.
    fn get_cow(&self, path: &str, max: usize) -> Result<Cow<'_, [u8]>> {
        self.get(path, max).map(Cow::Owned)
    }
}

pub(crate) struct ZipSource<'a> {
    zip: ZipReader<'a>,
}

impl<'a> ZipSource<'a> {
    pub fn new(bytes: &'a [u8]) -> Result<Self> {
        Ok(ZipSource { zip: ZipReader::new(bytes)? })
    }
}

impl ByteSource for ZipSource<'_> {
    fn get(&self, path: &str, max: usize) -> Result<Vec<u8>> {
        self.zip.read_by_name(path, max)
    }

    fn get_cow(&self, path: &str, max: usize) -> Result<Cow<'_, [u8]>> {
        let entry = self.zip.find(path).ok_or_else(|| FormatError::corrupt(format!("missing `{path}`")))?;
        self.zip.read_cow(entry, max)
    }
}

#[test]
fn zip_source_borrows_checked_payloads() {
    let mut zip = ZipWriter::new();
    zip.add("tiles/test.zst", b"payload").unwrap();
    let mut bytes = zip.finish().unwrap();
    let offset = bytes.windows(7).position(|b| b == b"payload").unwrap();
    let source = ZipSource::new(&bytes).unwrap();
    assert!(matches!(source.get_cow("tiles/test.zst", 7).unwrap(), Cow::Borrowed(b"payload")));
    assert_eq!(source.get_cow("tiles/test.zst", 7).unwrap().as_ptr(), bytes[offset..].as_ptr());
    assert!(source.get_cow("tiles/test.zst", 6).is_err());
    assert!(source.get_cow("missing", 7).is_err());
    bytes[offset] ^= 1;
    assert!(ZipSource::new(&bytes).unwrap().get_cow("tiles/test.zst", 7).is_err());
}

pub(crate) struct DirSource {
    pub root: PathBuf,
}

impl ByteSource for DirSource {
    fn get(&self, path: &str, max: usize) -> Result<Vec<u8>> {
        let p = self.root.join(path);
        let metadata = std::fs::metadata(&p).map_err(|_| FormatError::corrupt(format!("missing `{path}`")))?;
        if !metadata.is_file() {
            return Err(FormatError::corrupt(format!("`{path}` is not a regular file")));
        }
        let mut file = std::fs::File::open(&p).map_err(|_| FormatError::corrupt(format!("missing `{path}`")))?;
        read_entry(&mut file, metadata.len(), max, path)
    }
}

/// Read one opened bundle entry under its `max` budget. The stat `len` is a buffer hint only
/// (files can change while open, and a FIFO reports 0), so the bound is enforced on the read
/// itself, never on the hint.
pub fn read_entry(file: &mut impl std::io::Read, len: u64, max: usize, path: &str) -> Result<Vec<u8>> {
    let mut limited = file.take((max as u64).saturating_add(1));
    let hint = len.min((max as u64).saturating_add(1));
    let bytes = crate::read::read_all(&mut limited, hint)?;
    if bytes.len() > max {
        return Err(FormatError::LimitExceeded(format!("`{path}` exceeds {max} bytes")));
    }
    Ok(bytes)
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// Deepest JSON nesting a manifest may have: each group level adds three (the layer object, its
/// content object and the `children` array), with headroom for the document and leaf layer fields.
/// The saver checks the same bound, so every bundle it writes loads again.
const MAX_MANIFEST_DEPTH: usize = 3 * (convert::MAX_GROUP_DEPTH + 1) + 64;

/// Rejects JSON nested deeper than [`MAX_MANIFEST_DEPTH`] with a linear scan (no recursion), so
/// parsing with serde_json's recursion limit disabled can't exhaust the stack on hostile input.
fn check_manifest_depth(json: &[u8]) -> Result<()> {
    let (mut depth, mut in_str, mut escaped) = (0usize, false, false);
    for &b in json {
        if in_str {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'[' | b'{' => {
                depth += 1;
                if depth > MAX_MANIFEST_DEPTH {
                    return Err(FormatError::LimitExceeded(format!("manifest JSON nested deeper than {MAX_MANIFEST_DEPTH} levels")));
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

pub(crate) fn read_manifest(src: &dyn ByteSource, opts: &LoadOptions) -> Result<Manifest> {
    let raw = src.get(MANIFEST, opts.max_manifest_bytes)?;
    // serde_json stops at 128 levels (about 40 nested groups); the bounded check replaces that limit.
    check_manifest_depth(&raw)?;
    let mut de = serde_json::Deserializer::from_slice(&raw);
    de.disable_recursion_limit();
    let mut v = <serde_json::Value as serde::Deserialize>::deserialize(&mut de)?;
    de.end()?;
    crate::migrate::migrate(&mut v)?;
    let layers = take_array(&mut v, "/document/layers");
    let mut m: Manifest = serde_json::from_value(v)?;
    m.document.layers = layers_from_json(layers)?;
    // A number past f32's range decodes as infinity, which no save could write back (#6862).
    if let Err(crate::finite::Problem::NonFinite { path }) = crate::finite::check(&m) {
        return Err(FormatError::corrupt(format!("`{path}` is out of range")));
    }
    Ok(m)
}

/// Decodes layers one at a time, group children first. Decoding the whole tree in one go costs
/// kilobytes of stack per nesting level; here each level only holds a few pointers.
fn layers_from_json(values: Vec<serde_json::Value>) -> Result<Vec<LayerM>> {
    let mut out = Vec::with_capacity(values.len());
    for mut v in values {
        let children = layers_from_json(take_array(&mut v, "/content/children"))?;
        out.push(layer_from_json(v, children)?);
    }
    Ok(out)
}

/// One layer without its children (kept out of line so its large locals stay off the recursion).
#[inline(never)]
fn layer_from_json(v: serde_json::Value, children: Vec<LayerM>) -> Result<LayerM> {
    let mut layer: LayerM = serde_json::from_value(v)?;
    if let ContentM::Group { children: c, .. } = &mut layer.content {
        *c = children;
    }
    Ok(layer)
}

/// Takes the array at JSON `pointer` out of `v`, leaving an empty one (nothing if absent).
fn take_array(v: &mut serde_json::Value, pointer: &str) -> Vec<serde_json::Value> {
    match v.pointer_mut(pointer) {
        Some(serde_json::Value::Array(a)) => std::mem::take(a),
        _ => Vec::new(),
    }
}

struct LoadFetch<'a> {
    src: &'a dyn ByteSource,
    opts: LoadOptions,
    total: u64,
    blobs: HashMap<String, Arc<Vec<u8>>>,
    decompressor: Decompressor,
}

impl LoadFetch<'_> {
    fn account(&mut self, n: usize) -> Result<()> {
        self.total += n as u64;
        if self.total > self.opts.max_total_bytes {
            return Err(FormatError::LimitExceeded(format!("bundle expands beyond {} bytes", self.opts.max_total_bytes)));
        }
        Ok(())
    }
}

impl Fetch for LoadFetch<'_> {
    fn tile(&mut self, hash: &str, len: usize) -> Result<Vec<u8>> {
        if !is_valid_hash(hash) {
            return Err(FormatError::corrupt(format!("invalid tile hash `{hash}`")));
        }
        self.account(len)?;
        let path = tile_path(hash);
        let z = self.src.get_cow(&path, len + len / 8 + 4096)?;
        let data = self.decompressor.decode(&z, len, Some(len), &path)?;
        if blake3::hash(&data).to_hex().as_str() != hash {
            return Err(FormatError::corrupt(format!("{path}: content does not match its hash")));
        }
        Ok(data)
    }

    fn blob(&mut self, hash: &str) -> Result<Arc<Vec<u8>>> {
        if let Some(b) = self.blobs.get(hash) {
            return Ok(b.clone());
        }
        if !is_valid_hash(hash) {
            return Err(FormatError::corrupt(format!("invalid blob hash `{hash}`")));
        }
        let path = blob_path(hash);
        // The budget limits decoded bytes; allow the same frame overhead as tiles.
        let max_compressed = self.opts.max_blob_bytes.saturating_add(self.opts.max_blob_bytes / 8).saturating_add(4096);
        let z = self.src.get_cow(&path, max_compressed)?;
        // Blob length is not in the manifest; the frame header bounds it and
        // we cap at max_blob_bytes.
        let data = self.decompressor.decode(&z, self.opts.max_blob_bytes, None, &path)?;
        self.account(data.len())?;
        if blake3::hash(&data).to_hex().as_str() != hash {
            return Err(FormatError::corrupt(format!("{path}: content does not match its hash")));
        }
        let data = Arc::new(data);
        self.blobs.insert(hash.to_owned(), data.clone());
        Ok(data)
    }
}

/// Refuses a declared canvas outside `opts`' budget before anything is decoded for it (#2935).
fn admit_canvas(size: photocraft_geom::Size, opts: &LoadOptions) -> Result<()> {
    let (w, h) = (size.width, size.height);
    if w == 0 || h == 0 {
        return Err(FormatError::corrupt(format!("canvas size {w}x{h} is empty")));
    }
    if w > opts.max_canvas_side || h > opts.max_canvas_side {
        return Err(FormatError::LimitExceeded(format!("canvas {w}x{h} exceeds {} pixels per side", opts.max_canvas_side)));
    }
    if size.area() > opts.max_canvas_pixels {
        return Err(FormatError::LimitExceeded(format!("canvas {w}x{h} exceeds {} pixels", opts.max_canvas_pixels)));
    }
    Ok(())
}

pub(crate) fn load(src: &dyn ByteSource, opts: &LoadOptions) -> Result<Document> {
    let m = read_manifest(src, opts)?;
    admit_canvas(m.document.size, opts)?;
    let mut fetch = LoadFetch { src, opts: *opts, total: 0, blobs: HashMap::new(), decompressor: Decompressor::default() };
    let mut loader = Loader {
        fetch: &mut fetch,
        preserve_ids: opts.preserve_ids,
        id_map: HashMap::new(),
        contents_id_map: HashMap::new(),
        legacy_linked_contents: HashMap::new(),
        tiles: HashMap::new(),
    };
    loader.document(&m.document)
}

// ---------------------------------------------------------------------------
// Saving
// ---------------------------------------------------------------------------

/// What a save did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SaveStats {
    /// Distinct tiles referenced by the document.
    pub tiles_total: usize,
    /// Tiles that had to be compressed (ZIP) or written (directory).
    pub tiles_written: usize,
    /// Tiles reused from the previous save.
    pub tiles_reused: usize,
    pub blobs_total: usize,
    pub blobs_written: usize,
    /// Unreferenced objects removed (directory bundles).
    pub objects_removed: usize,
    /// Bytes of the manifest.
    pub manifest_bytes: usize,
}

#[derive(Default)]
struct Collect<'c> {
    tiles: BTreeMap<Hash, (Arc<Tile>, SampleType)>,
    blobs: BTreeMap<Hash, Arc<Vec<u8>>>,
    hash_cache: Option<&'c mut HashMap<usize, (Weak<Tile>, Hash)>>,
}

impl Sink for Collect<'_> {
    fn tile(&mut self, format: PixelFormat, tile: &Arc<Tile>) -> Hash {
        let key = Arc::as_ptr(tile) as usize;
        if let Some(cache) = self.hash_cache.as_deref_mut()
            && let Some((w, h)) = cache.get(&key)
            && w.upgrade().is_some_and(|t| Arc::ptr_eq(&t, tile))
        {
            let h = h.clone();
            self.tiles.entry(h.clone()).or_insert_with(|| (tile.clone(), format.sample));
            return h;
        }
        let h = hash_bytes(&tile_le(tile, format.sample));
        if let Some(cache) = self.hash_cache.as_deref_mut() {
            cache.insert(key, (Arc::downgrade(tile), h.clone()));
        }
        self.tiles.entry(h.clone()).or_insert_with(|| (tile.clone(), format.sample));
        h
    }

    fn blob(&mut self, data: &Arc<Vec<u8>>) -> Hash {
        let h = hash_bytes(data);
        self.blobs.entry(h.clone()).or_insert_with(|| data.clone());
        h
    }
}

fn tile_le(t: &Tile, sample: SampleType) -> std::borrow::Cow<'_, [u8]> {
    if cfg!(target_endian = "big") {
        let mut b = t.bytes().to_vec();
        swap_to_le(&mut b, sample);
        std::borrow::Cow::Owned(b)
    } else {
        std::borrow::Cow::Borrowed(t.bytes())
    }
}

/// Default for [`PcraftWriter::set_reverify_budget`]: compressed bytes of already-verified
/// objects re-read per directory save.
pub const DEFAULT_REVERIFY_BUDGET: u64 = 16 * 1024 * 1024;

/// Incremental `.pcraft` writer. Keep one per open document: it remembers
/// tile hashes (by `Arc` identity), compressed objects, and objects it has
/// verified in the current directory, so repeated saves stay cheap.
///
/// **What the directory cache trusts.** A directory save reuses an existing object file only
/// after decompressing it and checking its content hash. After that, the writer remembers the
/// file's signature (length and modification time, plus inode, device and change time on Unix)
/// and skips the full check on later saves while the signature is unchanged. A file damaged in
/// place without changing its signature (bit rot, or a tool that restores timestamps where
/// there is no change time) would be trusted, so each save also re-verifies a rolling share of
/// the cached objects, oldest-checked first, up to [`DEFAULT_REVERIFY_BUDGET`] bytes (see
/// [`Self::set_reverify_budget`]). Over enough saves every object is read again, and a damaged
/// one is rewritten.
pub struct PcraftWriter {
    hash_cache: HashMap<usize, (Weak<Tile>, Hash)>,
    /// Compressed objects by bundle path (ZIP mode).
    compressed: HashMap<String, Arc<Vec<u8>>>,
    verified_directory: Option<DirectoryVerificationCache>,
    reverify_budget: u64,
}

impl Default for PcraftWriter {
    fn default() -> Self {
        Self { hash_cache: HashMap::new(), compressed: HashMap::new(), verified_directory: None, reverify_budget: DEFAULT_REVERIFY_BUDGET }
    }
}

#[derive(Default)]
struct DirectoryVerificationCache {
    directory: PathBuf,
    /// Directory saves so far; an object's `checked_at` is the save that last read it fully.
    generation: u64,
    objects: HashMap<String, Verified>,
}

#[derive(Clone, Copy)]
struct Verified {
    signature: FileSignature,
    checked_at: u64,
}

/// What a file looked like when it was verified. Any change means "verify again".
#[derive(Clone, Copy, PartialEq, Eq)]
struct FileSignature {
    len: u64,
    modified: SystemTime,
    /// Inode, device and status-change time (s, ns): an in-place rewrite changes the change
    /// time even when a tool restores the modification time.
    #[cfg(unix)]
    unix: (u64, u64, i64, i64),
}

struct Prepared {
    manifest: Vec<u8>,
    objects: BTreeMap<String, Object>,
    previews: Vec<(&'static str, Vec<u8>)>,
    stats: SaveStats,
}

enum Object {
    Tile(Arc<Tile>, SampleType),
    Blob(Arc<Vec<u8>>),
}

impl Object {
    /// Compresses one object with the selected backend.
    fn compressed(&self, _compressor: &mut Compressor<'_>) -> Result<Vec<u8>> {
        let bytes = self.content();
        #[cfg(feature = "native-zstd")]
        return Ok(_compressor.compress(&bytes)?);
        #[cfg(not(feature = "native-zstd"))]
        Ok(ruzstd::encoding::compress_to_vec(bytes.as_ref(), ruzstd::encoding::CompressionLevel::Fastest))
    }

    /// Replaces a directory save's temporary output without compacting each small object.
    fn compress_into(&self, _compressor: &mut Compressor<'_>, encoded: &mut Vec<u8>) -> Result<()> {
        let bytes = self.content();
        #[cfg(feature = "native-zstd")]
        _compressor.compress_into(&bytes, encoded)?;
        #[cfg(not(feature = "native-zstd"))]
        {
            encoded.clear();
            ruzstd::encoding::compress(bytes.as_ref(), encoded, ruzstd::encoding::CompressionLevel::Fastest);
        }
        Ok(())
    }

    /// The bytes this object stores (tiles little-endian).
    fn content(&self) -> std::borrow::Cow<'_, [u8]> {
        match self {
            Object::Tile(tile, sample) => tile_le(tile, *sample),
            Object::Blob(blob) => std::borrow::Cow::Borrowed(blob.as_slice()),
        }
    }
}

/// At most 2 MiB of output scratch survives between objects in one directory save.
const MAX_DIRECTORY_SCRATCH: usize = 2 << 20;

/// Writes an independent frame and releases oversized output after the write.
fn write_object(root: &Path, path: &Path, object: &Object, compressor: &mut Compressor<'_>, encoded: &mut Vec<u8>) -> Result<()> {
    object.compress_into(compressor, encoded)?;
    if encoded.capacity() > MAX_DIRECTORY_SCRATCH {
        encoded.shrink_to_fit();
    }
    let result = write_bundle_file(root, path, encoded);
    if encoded.capacity() > MAX_DIRECTORY_SCRATCH {
        *encoded = Vec::new();
    }
    result
}

/// Whether `compressed` decodes to exactly `expected`.
fn content_matches(expected: &[u8], compressed: &[u8], what: &str) -> bool {
    #[cfg(feature = "native-zstd")]
    if expected.len() > MAX_DIRECTORY_SCRATCH {
        return photocraft_codecs::zstd_matches_exact(compressed, expected).unwrap_or(false);
    }
    decompress(compressed, expected.len(), what).is_ok_and(|actual| actual == expected)
}

fn file_signature(metadata: &std::fs::Metadata) -> Option<FileSignature> {
    #[cfg(unix)]
    let unix = {
        use std::os::unix::fs::MetadataExt;
        (metadata.ino(), metadata.dev(), metadata.ctime(), metadata.ctime_nsec())
    };
    Some(FileSignature {
        len: metadata.len(),
        modified: metadata.modified().ok()?,
        #[cfg(unix)]
        unix,
    })
}

fn path_signature(path: &Path) -> Result<Option<FileSignature>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(file_signature(&metadata)),
        Ok(_) => Err(unsafe_bundle_entry(path)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn unsafe_bundle_entry(path: &Path) -> FormatError {
    FormatError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("refusing directory bundle save: {} is a symlink or not a regular file", path.display()),
    ))
}

/// The generic atomic writer intentionally follows a destination symlink. Directory bundles
/// must reject one instead: all of their managed entries stay below the bundle root.
fn plain_file_or_missing(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(unsafe_bundle_entry(path)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn plain_directory_or_missing(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(FormatError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("refusing directory bundle save: {} is a symlink or not a directory", path.display()),
        ))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Use the canonical root after creation so a later replacement of the caller's path cannot
/// redirect subsequent bundle operations. The caller may choose a symlinked root, but managed
/// subdirectories and files below its resolved target must not be symlinks.
fn bundle_root(dir: &Path) -> Result<PathBuf> {
    let dir: PathBuf = dir.components().collect();
    std::fs::create_dir_all(&dir)?;
    let root = std::fs::canonicalize(&dir)?;
    plain_directory_or_missing(&root)?;
    for sub in ["tiles", "blobs", "composite"] {
        let path = root.join(sub);
        plain_directory_or_missing(&path)?;
        if !path.exists() {
            match std::fs::create_dir(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            }
        }
        plain_directory_or_missing(&path)?;
        if std::fs::canonicalize(&path)? != path {
            return Err(unsafe_bundle_entry(&path));
        }
    }
    Ok(root)
}

fn validate_bundle_file(root: &Path, path: &Path) -> Result<()> {
    let parent = path.parent().ok_or_else(|| unsafe_bundle_entry(path))?;
    if parent != root && parent != root.join("tiles") && parent != root.join("blobs") && parent != root.join("composite") {
        return Err(unsafe_bundle_entry(path));
    }
    plain_directory_or_missing(parent)?;
    if std::fs::canonicalize(parent)? != parent {
        return Err(unsafe_bundle_entry(parent));
    }
    plain_file_or_missing(path)
}

fn write_bundle_file(root: &Path, path: &Path, data: &[u8]) -> Result<()> {
    validate_bundle_file(root, path)?;
    let parent = path.parent().ok_or_else(|| unsafe_bundle_entry(path))?;
    let name = path.file_name().ok_or_else(|| unsafe_bundle_entry(path))?;
    let mut temporary = None;
    for _ in 0..8 {
        let candidate = parent.join(crate::atomic::temp_name(&name.to_string_lossy()));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&candidate) {
            Ok(file) => {
                temporary = Some((candidate, file));
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
    }
    let (tmp, mut file) = temporary.ok_or_else(|| {
        FormatError::Io(std::io::Error::new(std::io::ErrorKind::AlreadyExists, format!("could not create a temporary file beside {}", path.display())))
    })?;
    let result = (|| -> Result<()> {
        // The parent may have changed between validation and create_new. Check the new file's
        // resolved location before putting document bytes into it.
        if std::fs::canonicalize(&tmp)? != tmp {
            return Err(unsafe_bundle_entry(&tmp));
        }
        validate_bundle_file(root, path)?;
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        // Preserve the existing mode where possible, as the generic atomic writer does.
        if let Ok(permissions) = std::fs::metadata(path).map(|m| m.permissions()) {
            let _ = std::fs::set_permissions(&tmp, permissions);
        }
        // Recheck the parent and target just before publishing. rename replaces a symlink
        // itself, never its referent, even if one appears after this check.
        validate_bundle_file(root, path)?;
        crate::atomic::retry_rename(crate::atomic::RenameRetry::platform(), || std::fs::rename(&tmp, path))?;
        #[cfg(unix)]
        {
            let _ = std::fs::File::open(parent).and_then(|d| d.sync_all());
        }
        Ok(())
    })();
    if result.is_err() && validate_bundle_file(root, &tmp).is_ok() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Keep the existing leaf/path validation and pin the actual unlink to opened
/// directory handles on Unix. This closes the check-then-remove path-swap gap.
fn remove_bundle_file(root: &Path, path: &Path, gc: &BundleGc) -> Result<()> {
    validate_bundle_file(root, path)?;
    let relative = path.strip_prefix(root).map_err(|_| unsafe_bundle_entry(path))?;
    gc.remove(relative)
}

#[cfg(unix)]
struct BundleGc {
    root: rustix::fd::OwnedFd,
    tiles: rustix::fd::OwnedFd,
    blobs: rustix::fd::OwnedFd,
    composite: rustix::fd::OwnedFd,
}

#[cfg(unix)]
fn gc_directory_flags() -> rustix::fs::OFlags {
    use rustix::fs::OFlags;

    // GC needs directory traversal, not permission to read a directory listing.
    // Both modes retain a descriptor for the directory opened here, so a later
    // pathname replacement cannot redirect descriptor-relative cleanup.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let access = OFlags::PATH;
    #[cfg(target_vendor = "apple")]
    let access = OFlags::from_bits_retain(libc::O_SEARCH.cast_unsigned());
    #[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
    let access = OFlags::RDONLY;
    access | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}

#[cfg(unix)]
impl BundleGc {
    fn new(root: &Path) -> Result<Self> {
        use rustix::fs::{Mode, open, openat};

        let flags = gc_directory_flags();
        let root = open(root, flags, Mode::empty()).map_err(std::io::Error::from)?;
        let tiles = openat(&root, "tiles", flags, Mode::empty()).map_err(std::io::Error::from)?;
        let blobs = openat(&root, "blobs", flags, Mode::empty()).map_err(std::io::Error::from)?;
        let composite = openat(&root, "composite", flags, Mode::empty()).map_err(std::io::Error::from)?;
        Ok(Self { root, tiles, blobs, composite })
    }

    fn remove(&self, relative: &Path) -> Result<()> {
        let relative = relative.to_str().ok_or_else(|| unsafe_bundle_entry(relative))?;
        let (handle, name) = match relative {
            THUMB => (&self.root, THUMB),
            COMPOSITE => (&self.composite, "preview.png"),
            _ => match relative.split_once('/') {
                Some(("tiles", name)) if name.strip_suffix(".zst").is_some_and(is_valid_hash) => (&self.tiles, name),
                Some(("blobs", name)) if name.strip_suffix(".zst").is_some_and(is_valid_hash) => (&self.blobs, name),
                _ => return Err(FormatError::Unsupported(format!("invalid bundle cleanup target: {relative}"))),
            },
        };
        rustix::fs::unlinkat(handle, name, rustix::fs::AtFlags::empty()).map_err(std::io::Error::from)?;
        Ok(())
    }
}

#[cfg(not(unix))]
struct BundleGc {
    root: PathBuf,
}

#[cfg(not(unix))]
impl BundleGc {
    fn new(root: &Path) -> Result<Self> {
        Ok(Self { root: root.to_path_buf() })
    }

    fn remove(&self, relative: &Path) -> Result<()> {
        std::fs::remove_file(self.root.join(relative))?;
        Ok(())
    }
}

fn existing_object_is_valid(path: &Path, object: &Object, what: &str) -> Result<(bool, Option<FileSignature>)> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((false, None)),
        Err(e) => return Err(e.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Ok((false, None));
    }
    let signature = file_signature(&metadata);

    let expected = object.content();
    let expected_len = expected.len();
    // This is the same bound used by the tile loader. It comfortably includes
    // zstd framing overhead while keeping damaged files bounded when inspected.
    let max_compressed_len = expected_len.saturating_add(expected_len / 8).saturating_add(4096);
    let max_compressed_len_u64 = u64::try_from(max_compressed_len).unwrap_or(u64::MAX);
    if metadata.len() > max_compressed_len_u64 {
        return Ok((false, None));
    }

    let mut compressed = Vec::new();
    (&mut file).take(max_compressed_len_u64.saturating_add(1)).read_to_end(&mut compressed)?;
    if compressed.len() > max_compressed_len {
        return Ok((false, None));
    }
    Ok((content_matches(&expected, &compressed, what), signature))
}

/// A bundle path and its compressed bytes.
type Compressed<'a> = (&'a String, Arc<Vec<u8>>);

/// Compresses `objects` on scoped worker threads (sequentially on wasm).
fn par_compress<'a>(objects: &[(&'a String, &'a Object)]) -> Result<Vec<Compressed<'a>>> {
    if objects.is_empty() {
        return Ok(Vec::new());
    }
    let threads = if cfg!(target_arch = "wasm32") { 1 } else { std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 32) };
    if threads < 2 || objects.len() < 2 {
        let mut compressor = new_compressor()?;
        return objects.iter().map(|(p, o)| Ok((*p, Arc::new(o.compressed(&mut compressor)?)))).collect();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|sc| {
        let hs: Vec<_> = (0..threads.min(objects.len()))
            .map(|_| {
                sc.spawn(|| -> Result<_> {
                    let mut compressor = new_compressor()?;
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some((p, o)) = objects.get(i) else { break };
                        out.push((*p, Arc::new(o.compressed(&mut compressor)?)));
                    }
                    Ok(out)
                })
            })
            .collect();
        // Join every worker before looking at the results, so one failure can't leave
        // another worker unjoined (the scope would then panic).
        let joined: Vec<_> = hs.into_iter().map(|h| h.join()).collect();
        let mut out = Vec::with_capacity(objects.len());
        for r in joined {
            out.extend(r.map_err(|_| FormatError::Io(std::io::Error::other("compression worker panicked")))??);
        }
        Ok(out)
    })
}

impl PcraftWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many compressed bytes of already-verified objects each directory save re-reads and
    /// re-checks (oldest-checked first; at least one object when the budget is non-zero). `0`
    /// trusts unchanged signatures entirely; `u64::MAX` re-verifies everything on every save.
    pub fn set_reverify_budget(&mut self, bytes: u64) {
        self.reverify_budget = bytes;
    }

    fn prepare(&mut self, doc: &Document, opts: &SaveOptions) -> Result<Prepared> {
        self.hash_cache.retain(|_, (w, _)| w.strong_count() > 0);
        convert::check_nesting(&doc.layers)?;
        let mut c = Collect { hash_cache: Some(&mut self.hash_cache), ..Default::default() };
        let document = convert::doc_m(doc, &mut c);
        let mut previews = Vec::new();
        if let Some(t) = &opts.thumbnail {
            previews.push((THUMB, png(t)?));
        }
        if let Some(t) = &opts.composite {
            previews.push((COMPOSITE, png(t)?));
        }
        let manifest = Manifest {
            format_version: FORMAT_VERSION,
            generator: format!("photocraft-format {}", env!("CARGO_PKG_VERSION")),
            document,
            thumbnail: opts.thumbnail.as_ref().map(|_| THUMB.to_owned()),
            composite: opts.composite.as_ref().map(|_| COMPOSITE.to_owned()),
        };
        // serde_json writes NaN and the infinities as `null`, which the manifest's float fields
        // refuse on load (issue #1101): refuse the save instead, naming the value.
        crate::finite::check(&manifest).map_err(|e| match e {
            crate::finite::Problem::NonFinite { path } => FormatError::NonFinite { path },
            crate::finite::Problem::Other(m) => FormatError::Unsupported(format!("manifest: {m}")),
        })?;
        let manifest = serde_json::to_vec_pretty(&manifest)?;
        check_manifest_depth(&manifest)?;
        let stats = SaveStats { tiles_total: c.tiles.len(), blobs_total: c.blobs.len(), manifest_bytes: manifest.len(), ..Default::default() };
        let mut objects = BTreeMap::new();
        for (h, (t, s)) in c.tiles {
            objects.insert(tile_path(&h), Object::Tile(t, s));
        }
        for (h, b) in c.blobs {
            objects.insert(blob_path(&h), Object::Blob(b));
        }
        Ok(Prepared { manifest, objects, previews, stats })
    }

    /// Save as a ZIP bundle in memory.
    pub fn save_zip(&mut self, doc: &Document, opts: &SaveOptions) -> Result<(Vec<u8>, SaveStats)> {
        let p = self.prepare(doc, opts)?;
        let mut stats = p.stats;
        let mut z = ZipWriter::new();
        z.add(MANIFEST, &p.manifest)?;
        for (name, data) in &p.previews {
            z.add(name, data)?;
        }
        let mut next = HashMap::with_capacity(p.objects.len());
        // New objects are compressed on all cores first (zstd dominates a full save).
        let todo: Vec<(&String, &Object)> = p.objects.iter().filter(|(path, _)| !self.compressed.contains_key(*path)).collect();
        let mut fresh: HashMap<&String, Arc<Vec<u8>>> = par_compress(&todo)?.into_iter().collect();
        for (path, obj) in &p.objects {
            let data = match self.compressed.get(path) {
                Some(d) => {
                    if matches!(obj, Object::Tile(..)) {
                        stats.tiles_reused += 1;
                    }
                    d.clone()
                }
                None => {
                    match obj {
                        Object::Tile(..) => stats.tiles_written += 1,
                        Object::Blob(_) => stats.blobs_written += 1,
                    }
                    match fresh.remove(path) {
                        Some(data) => data,
                        None => Arc::new(obj.compressed(&mut new_compressor()?)?),
                    }
                }
            };
            z.add(path, &data)?;
            next.insert(path.clone(), data);
        }
        self.compressed = next;
        Ok((z.finish()?, stats))
    }

    /// Save into a directory bundle: reuses only valid existing objects,
    /// rewrites missing or damaged objects, then writes the manifest
    /// atomically and removes unreferenced objects.
    pub fn save_dir(&mut self, doc: &Document, dir: &Path, opts: &SaveOptions) -> Result<SaveStats> {
        let canonical_dir = bundle_root(dir)?;
        let dir = canonical_dir.as_path();
        let existing = list_objects(dir)?;
        // Reject a hostile existing entry before writing any object or manifest. This also
        // covers entries that this save would otherwise leave untouched or garbage-collect.
        for path in [MANIFEST, THUMB, COMPOSITE] {
            validate_bundle_file(dir, &dir.join(path))?;
        }
        let p = self.prepare(doc, opts)?;
        let mut stats = p.stats;
        if self.verified_directory.as_ref().is_none_or(|cache| cache.directory != canonical_dir) {
            self.verified_directory = Some(DirectoryVerificationCache { directory: canonical_dir.clone(), ..Default::default() });
        }
        let generation = match &mut self.verified_directory {
            Some(cache) => {
                cache.generation = cache.generation.saturating_add(1);
                cache.generation
            }
            None => 0,
        };
        let recheck = self.rolling_recheck(&p.objects);
        let mut compressor = None;
        let mut encoded = Vec::new();
        for (path, obj) in &p.objects {
            let object_path = dir.join(path);
            // bundle_root and list_objects validated the fixed parent directories once. Check
            // each leaf here; write_bundle_file rechecks its parent around every mutation.
            plain_file_or_missing(&object_path)?;
            let signature = path_signature(&object_path)?;
            let already_verified = !recheck.contains(path)
                && signature.is_some_and(|signature| {
                    self.verified_directory.as_ref().is_some_and(|cache| cache.objects.get(path).is_some_and(|v| v.signature == signature))
                });
            if existing.contains(path) && already_verified {
                if matches!(obj, Object::Tile(..)) {
                    stats.tiles_reused += 1;
                }
                continue;
            }
            if existing.contains(path) {
                let (valid, signature) = existing_object_is_valid(&object_path, obj, path)?;
                if valid {
                    if let Some(signature) = signature
                        && let Some(cache) = &mut self.verified_directory
                    {
                        cache.objects.insert(path.clone(), Verified { signature, checked_at: generation });
                    }
                    if matches!(obj, Object::Tile(..)) {
                        stats.tiles_reused += 1;
                    }
                    continue;
                }
            }
            match obj {
                Object::Tile(..) => stats.tiles_written += 1,
                Object::Blob(_) => stats.blobs_written += 1,
            }
            let compressor = match &mut compressor {
                Some(c) => c,
                None => compressor.insert(new_compressor()?),
            };
            write_object(dir, &object_path, obj, compressor, &mut encoded)?;
            let written = path_signature(&object_path)?;
            if let Some(cache) = &mut self.verified_directory {
                match written {
                    Some(signature) => {
                        cache.objects.insert(path.clone(), Verified { signature, checked_at: generation });
                    }
                    None => {
                        cache.objects.remove(path);
                    }
                }
            }
        }
        for (name, data) in &p.previews {
            write_bundle_file(dir, &dir.join(name), data)?;
        }
        // Pin all GC directories before removing stale previews or objects.
        // Unix cleanup then stays within these directories even if a pathname is swapped.
        let gc = BundleGc::new(dir)?;
        for stale in [THUMB, COMPOSITE] {
            if !p.previews.iter().any(|(n, _)| *n == stale) {
                let path = dir.join(stale);
                validate_bundle_file(dir, &path)?;
                match remove_bundle_file(dir, &path, &gc) {
                    Err(FormatError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {}
                    other => other?,
                }
            }
        }
        write_bundle_file(dir, &dir.join(MANIFEST), &p.manifest)?;
        // The manifest is committed: the save has succeeded. Removing objects it no longer
        // references is best-effort housekeeping (a vanished or locked file must not turn a
        // complete save into a reported failure); a leftover is retried on the next save.
        for path in existing {
            if !p.objects.contains_key(&path) {
                if remove_bundle_file(dir, &dir.join(&path), &gc).is_ok() {
                    stats.objects_removed += 1;
                }
                if let Some(cache) = &mut self.verified_directory {
                    cache.objects.remove(&path);
                }
            }
        }
        Ok(stats)
    }

    /// The cached objects this save re-verifies even though their signature is unchanged:
    /// oldest-checked first, until their recorded sizes reach the re-verify budget (always at
    /// least one, so an object larger than the budget is still reached in turn).
    fn rolling_recheck(&self, objects: &BTreeMap<String, Object>) -> HashSet<String> {
        let mut out = HashSet::new();
        let Some(cache) = &self.verified_directory else { return out };
        if self.reverify_budget == 0 {
            return out;
        }
        let mut candidates: Vec<(&String, &Verified)> = cache.objects.iter().filter(|(path, _)| objects.contains_key(*path)).collect();
        candidates.sort_by(|a, b| a.1.checked_at.cmp(&b.1.checked_at).then_with(|| a.0.cmp(b.0)));
        let mut spent = 0u64;
        for (path, verified) in candidates {
            let next = spent.saturating_add(verified.signature.len);
            if !out.is_empty() && next > self.reverify_budget {
                break;
            }
            spent = next;
            out.insert(path.clone());
        }
        out
    }

    /// Save to `path`: a directory bundle if `path` is an existing directory
    /// or ends with a path separator, otherwise a ZIP file (written atomically).
    pub fn save_path(&mut self, doc: &Document, path: &Path, opts: &SaveOptions) -> Result<SaveStats> {
        let s = path.to_string_lossy();
        if path.is_dir() || s.ends_with('/') || s.ends_with('\\') {
            self.save_dir(doc, path, opts)
        } else {
            let (bytes, stats) = self.save_zip(doc, opts)?;
            write_atomic(path, &bytes)?;
            Ok(stats)
        }
    }
}

#[cfg(all(test, unix))]
#[test]
fn gc_pinned_directory_handles_resist_subdirectory_swap() {
    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!("pcraft-gc-swap-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let bundle = root.join("project.pcraft");
    let outside = root.join("outside");
    for sub in ["tiles", "blobs", "composite"] {
        std::fs::create_dir_all(bundle.join(sub)).unwrap();
    }
    std::fs::create_dir(&outside).unwrap();
    let filename = format!("{}.zst", "c".repeat(64));
    std::fs::write(bundle.join("tiles").join(&filename), b"stale bundle file").unwrap();
    std::fs::write(outside.join(&filename), b"outside file must survive").unwrap();

    let gc = BundleGc::new(&bundle).unwrap();
    std::fs::rename(bundle.join("tiles"), bundle.join("old-tiles")).unwrap();
    symlink(&outside, bundle.join("tiles")).unwrap();

    // Exercise the descriptor-relative unlink directly: the public save path
    // also validates the leaf, but that check itself cannot prevent a later swap.
    gc.remove(Path::new(&format!("tiles/{filename}"))).unwrap();
    assert!(!bundle.join("old-tiles").join(&filename).exists());
    assert_eq!(std::fs::read(outside.join(&filename)).unwrap(), b"outside file must survive");
    std::fs::remove_dir_all(root).unwrap();
}

fn list_objects(dir: &Path) -> Result<HashSet<String>> {
    let mut out = HashSet::new();
    for sub in ["tiles", "blobs"] {
        let object_dir = dir.join(sub);
        plain_directory_or_missing(&object_dir)?;
        if std::fs::canonicalize(&object_dir)? != object_dir {
            return Err(unsafe_bundle_entry(&object_dir));
        }
        let rd = std::fs::read_dir(&object_dir)?;
        for e in rd {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            if name.strip_suffix(".zst").is_some_and(is_valid_hash) {
                let path = e.path();
                match std::fs::symlink_metadata(&path) {
                    Ok(metadata) if metadata.file_type().is_symlink() => return Err(unsafe_bundle_entry(&path)),
                    Ok(metadata) if metadata.is_file() => {
                        out.insert(format!("{sub}/{name}"));
                    }
                    // Stale directories cannot be removed with remove_file; leave them for later
                    // cleanup without failing a save of otherwise valid content.
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }
    Ok(out)
}

/// Crash-safe replace of one file (see [`crate::atomic`]).
pub(crate) fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    crate::atomic::atomic_write(path, data)?;
    Ok(())
}

/// Reuse must preserve independent objects and recover after a failed frame.
#[test]
fn decoder_reuse_preserves_independent_objects() {
    let mut decoder = Decompressor::default();
    for bytes in [vec![37; 262_147], vec![], vec![19; 131_072], vec![42; 17]] {
        let encoded = Object::Blob(Arc::new(bytes.clone())).compressed(&mut new_compressor().unwrap()).unwrap();
        let mut reference = ruzstd::decoding::StreamingDecoder::new(encoded.as_slice()).unwrap();
        let mut expected = Vec::new();
        reference.read_to_end(&mut expected).unwrap();
        assert_eq!(decoder.decode(&encoded, bytes.len(), Some(bytes.len()), "tile").unwrap(), expected);
        assert!(decoder.decode(&encoded[..encoded.len() - 1], bytes.len(), None, "blob").is_err());
        assert_eq!(decoder.decode(&encoded, bytes.len(), None, "blob").unwrap(), bytes);
        if !bytes.is_empty() {
            assert!(matches!(decoder.decode(&encoded, bytes.len() - 1, None, "blob"), Err(FormatError::LimitExceeded(_))));
        }
    }
}

/// Blob limits apply to decoded bytes, including empty and expanding frames.
#[test]
fn blob_budget_allows_frame_overhead() {
    for bytes in [vec![], vec![42], (0..32).collect()] {
        let hash = hash_bytes(&bytes);
        let mut encoded = Object::Blob(Arc::new(bytes.clone())).compressed(&mut new_compressor().unwrap()).unwrap();
        assert!(encoded.len() > bytes.len());
        for corrupt in [false, true] {
            if corrupt {
                encoded.pop();
            }
            let mut zip = ZipWriter::new();
            zip.add(&blob_path(&hash), &encoded).unwrap();
            let zip = zip.finish().unwrap();
            let source = ZipSource::new(&zip).unwrap();
            let fetch = |max| LoadFetch {
                src: &source,
                opts: LoadOptions { max_blob_bytes: max, max_total_bytes: bytes.len() as u64, ..LoadOptions::default() },
                total: 0,
                blobs: HashMap::new(),
                decompressor: Decompressor::default(),
            };
            if corrupt {
                assert!(fetch(bytes.len()).blob(&hash).is_err());
            } else {
                assert_eq!(*fetch(bytes.len()).blob(&hash).unwrap(), bytes);
                if !bytes.is_empty() {
                    assert!(matches!(fetch(bytes.len() - 1).blob(&hash), Err(FormatError::LimitExceeded(_))));
                }
            }
        }
    }
}

/// Buffer reuse preserves frame bytes and releases large outputs, including failed writes.
#[test]
fn directory_output_reuse_is_bounded_and_preserves_frames() {
    let dir = std::env::temp_dir().join(format!("photocraft-output-reuse-{}", std::process::id()));
    let root = bundle_root(&dir).unwrap();
    let path = root.join("blobs/test.zst");
    let mut compressor = new_compressor().unwrap();
    let mut encoded = Vec::new();
    let mut first = std::ptr::null();
    // Incompressible bytes exercise the output limit, without a timing fixture or document.
    let mut state = 0x1234_5678u32;
    let big: Vec<u8> = (0..MAX_DIRECTORY_SCRATCH + 4096)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect();
    for (i, bytes) in [vec![42; 1024], vec![19; 1024], big, vec![]].into_iter().enumerate() {
        let object = Object::Blob(Arc::new(bytes));
        let expected = object.compressed(&mut new_compressor().unwrap()).unwrap();
        write_object(&root, &path, &object, &mut compressor, &mut encoded).unwrap();
        let actual = std::fs::read(&path).unwrap();
        assert_eq!(actual, expected);
        let content = object.content();
        assert!(content_matches(&content, &actual, "test"));
        assert_eq!(decompress_bounded(&actual, content.len(), Some(content.len()), "test").unwrap(), content.as_ref());
        assert!(encoded.capacity() <= MAX_DIRECTORY_SCRATCH);
        if i == 0 {
            first = encoded.as_ptr();
        }
        if i == 1 {
            assert_eq!(encoded.as_ptr(), first);
        }
        if i == 2 {
            let mut wrong = content.to_vec();
            wrong[0] ^= 1;
            assert!(!content_matches(&wrong, &actual, "test"));
            assert!(actual.len() > MAX_DIRECTORY_SCRATCH);
            assert_eq!(encoded.capacity(), 0);
            assert!(write_object(&root, &root.join("invalid/test.zst"), &object, &mut compressor, &mut encoded).is_err());
            assert_eq!(encoded.capacity(), 0);
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(all(test, feature = "native-zstd"))]
mod tests {
    use super::*;

    thread_local! {
        pub(super) static COMPRESSORS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// Empty and cached-only saves need no encoder, while new objects share one.
    #[test]
    fn unchanged_saves_skip_encoder_creation() {
        use photocraft_color::{Color, ColorMode};
        let doc = Document::with_background("reuse", photocraft_geom::Size::new(1, 1), ColorMode::Rgb, SampleType::U8, Color::BLACK);
        let mut writer = PcraftWriter::new();
        COMPRESSORS.set(0);
        assert!(par_compress(&[]).unwrap().is_empty());
        assert_eq!(COMPRESSORS.get(), 0);
        let (first, _) = writer.save_zip(&doc, &SaveOptions::default()).unwrap();
        assert_eq!(COMPRESSORS.get(), 1);
        assert_eq!(writer.save_zip(&doc, &SaveOptions::default()).unwrap().0, first);
        assert_eq!(COMPRESSORS.get(), 1);
        let dir = std::env::temp_dir().join(format!("photocraft-native-reuse-{}", std::process::id()));
        writer.save_dir(&doc, &dir, &SaveOptions::default()).unwrap();
        assert_eq!(COMPRESSORS.get(), 2);
        writer.save_dir(&doc, &dir, &SaveOptions::default()).unwrap();
        assert_eq!(COMPRESSORS.get(), 2);
        assert_eq!(crate::load_path(&dir).unwrap(), doc);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Reusing a native encoder preserves independent frames in both readers.
    #[test]
    fn reused_compressor_produces_independent_frames() {
        let data: Vec<Arc<Vec<u8>>> =
            [0, 1, 8, 131_072, 262_144, 262_147, 131_072, 1].into_iter().map(|n| Arc::new((0..n).map(|i| ((i * 37 + i / 97) % 251) as u8).collect())).collect();
        let objects: Vec<_> = data.iter().cloned().map(Object::Blob).collect();
        let mut compressor = new_compressor().unwrap();
        for (object, bytes) in objects.iter().zip(&data) {
            let encoded = object.compressed(&mut compressor).unwrap();
            assert_eq!(decompress(&encoded, bytes.len(), "test").unwrap(), **bytes);
            assert_eq!(decompress_bounded(&encoded, bytes.len(), None, "blob").unwrap(), **bytes);
            if !bytes.is_empty() {
                assert!(matches!(decompress_bounded(&encoded, bytes.len() - 1, None, "blob"), Err(FormatError::LimitExceeded(_))));
            }
            let mut rust_reader = ruzstd::decoding::StreamingDecoder::new(encoded.as_slice()).unwrap();
            let mut rust_bytes = Vec::new();
            rust_reader.read_to_end(&mut rust_bytes).unwrap();
            assert_eq!(rust_bytes, **bytes);
            let rust = ruzstd::encoding::compress_to_vec(bytes.as_slice(), ruzstd::encoding::CompressionLevel::Fastest);
            assert_eq!(decompress(&rust, bytes.len(), "Rust encoder").unwrap(), **bytes);
        }
    }
}
