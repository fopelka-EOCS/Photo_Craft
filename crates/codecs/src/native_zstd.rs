/// Reusable Zstandard encoder at the library's default level, with frame checksums.
pub struct ZstdEncoder<'a>(zstd::zstd_safe::CCtx<'a>);

fn zstd_error(code: usize) -> std::io::Error {
    std::io::Error::other(zstd::zstd_safe::get_error_name(code))
}

/// Decodes the first Zstandard frame into at most `max_bytes` of output.
pub fn decode_zstd(bytes: &[u8], max_bytes: usize) -> std::io::Result<Vec<u8>> {
    let size = zstd::zstd_safe::find_frame_compressed_size(bytes).map_err(zstd_error)?;
    let frame = bytes.get(..size).ok_or_else(|| std::io::Error::other("invalid Zstandard frame size"))?;
    let mut decoded = Vec::new();
    decoded.try_reserve_exact(max_bytes).map_err(std::io::Error::other)?;
    let mut decoder = zstd::zstd_safe::DCtx::try_create().ok_or_else(|| std::io::Error::other("cannot allocate Zstandard decoder"))?;
    decoder.decompress(&mut decoded, frame).map_err(zstd_error)?;
    Ok(decoded)
}

/// Decodes the first frame under an output budget and a 128 MiB window limit.
/// Unlike [`decode_zstd`], this does not reserve the output budget up front.
pub fn decode_zstd_bounded(bytes: &[u8], max_bytes: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut context = zstd::zstd_safe::DCtx::try_create().ok_or_else(|| std::io::Error::other("cannot allocate Zstandard decoder"))?;
    context.set_parameter(zstd::zstd_safe::DParameter::WindowLogMax(27)).map_err(zstd_error)?;
    let decoder = zstd::stream::read::Decoder::with_context(bytes, &mut context).single_frame();
    let limit = u64::try_from(max_bytes).unwrap_or(u64::MAX).saturating_add(1);
    let mut decoded = Vec::new();
    decoder.take(limit).read_to_end(&mut decoded)?;
    if decoded.len() > max_bytes {
        return Err(std::io::Error::new(std::io::ErrorKind::FileTooLarge, "Zstandard output exceeds its budget"));
    }
    Ok(decoded)
}

/// Compares the first decoded frame with `expected` without allocating its full output.
/// Enforces the same 128 MiB window limit as [`decode_zstd_bounded`].
pub fn zstd_matches(bytes: &[u8], mut expected: &[u8]) -> std::io::Result<bool> {
    use std::io::Read;
    let mut context = zstd::zstd_safe::DCtx::try_create().ok_or_else(|| std::io::Error::other("cannot allocate Zstandard decoder"))?;
    context.set_parameter(zstd::zstd_safe::DParameter::WindowLogMax(27)).map_err(zstd_error)?;
    let mut decoder = zstd::stream::read::Decoder::with_context(bytes, &mut context).single_frame();
    let mut buffer = [0; 64 << 10];
    loop {
        let n = decoder.read(&mut buffer)?;
        if n == 0 {
            return Ok(expected.is_empty());
        }
        let chunk = buffer.get(..n).ok_or_else(|| std::io::Error::other("invalid Zstandard output chunk"))?;
        let Some(rest) = expected.strip_prefix(chunk) else { return Ok(false) };
        expected = rest;
    }
}

impl ZstdEncoder<'_> {
    /// Creates an encoder that writes independent frames with checksums.
    pub fn new() -> std::io::Result<Self> {
        let mut encoder = zstd::zstd_safe::CCtx::try_create().ok_or_else(|| std::io::Error::other("cannot allocate Zstandard encoder"))?;
        encoder.set_parameter(zstd::zstd_safe::CParameter::ChecksumFlag(true)).map_err(zstd_error)?;
        Ok(Self(encoder))
    }

    /// Returns a compact, owned frame for storage or caching.
    pub fn compress(&mut self, bytes: &[u8]) -> std::io::Result<Vec<u8>> {
        let mut encoded = Vec::new();
        self.compress_into(bytes, &mut encoded)?;
        // Avoid retaining the input-sized allocation for saved objects.
        encoded.shrink_to_fit();
        Ok(encoded)
    }

    /// Replaces the output with one complete frame, reusing its allocation.
    pub fn compress_into(&mut self, bytes: &[u8], encoded: &mut Vec<u8>) -> std::io::Result<()> {
        encoded.clear();
        encoded.try_reserve_exact(zstd::zstd_safe::compress_bound(bytes.len())).map_err(std::io::Error::other)?;
        self.0.compress2(encoded, bytes).map_err(zstd_error)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_decode_checks_bounds_frames_and_checksums() {
        use std::io::Write;
        let bytes = vec![37; 262_147];
        let known = ZstdEncoder::new().unwrap().compress(&bytes).unwrap();
        let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 0).unwrap();
        encoder.include_checksum(true).unwrap();
        encoder.write_all(&bytes).unwrap();
        let unknown = encoder.finish().unwrap();
        assert_eq!(zstd::zstd_safe::get_frame_content_size(&unknown).unwrap(), None);
        for mut frame in [known, unknown] {
            assert_eq!(decode_zstd_bounded(&frame, bytes.len()).unwrap(), bytes);
            assert!(zstd_matches(&frame, &bytes).unwrap());
            assert!(!zstd_matches(&frame, &bytes[..bytes.len() - 1]).unwrap());
            assert!(!zstd_matches(&frame, &[0; 32]).unwrap());
            assert_eq!(decode_zstd_bounded(&frame, usize::MAX).unwrap(), bytes);
            assert_eq!(decode_zstd_bounded(&frame, bytes.len() - 1).unwrap_err().kind(), std::io::ErrorKind::FileTooLarge);
            for end in 0..frame.len() {
                assert!(decode_zstd_bounded(&frame[..end], bytes.len()).is_err());
                assert!(!matches!(zstd_matches(&frame[..end], &bytes), Ok(true)));
            }
            let joined = [frame.as_slice(), frame.as_slice()].concat();
            assert_eq!(decode_zstd_bounded(&joined, bytes.len()).unwrap(), bytes);
            assert_eq!(decode_zstd(&joined, bytes.len()).unwrap(), bytes);
            assert!(zstd_matches(&joined, &bytes).unwrap());
            let both = [bytes.as_slice(), bytes.as_slice()].concat();
            assert!(!zstd_matches(&joined, &both).unwrap());
            *frame.last_mut().unwrap() ^= 1;
            assert!(decode_zstd_bounded(&frame, bytes.len()).is_err());
            assert!(zstd_matches(&frame, &bytes).is_err());
        }
        let mut window = [0x28, 0xb5, 0x2f, 0xfd, 0, 0, 1, 0, 0];
        assert!(decode_zstd_bounded(&window, 0).unwrap().is_empty());
        window[5] = 0x90;
        assert!(decode_zstd_bounded(&window, 0).is_err());
        assert!(zstd_matches(&window, &[]).is_err());
        let empty = ZstdEncoder::new().unwrap().compress(&[]).unwrap();
        assert!(decode_zstd_bounded(&empty, 0).unwrap().is_empty());
        assert!(zstd_matches(&empty, &[]).unwrap());
        assert!(!zstd_matches(&empty, &[0]).unwrap());
    }

    #[test]
    fn frames_are_independent_and_checksummed() {
        assert!(decode_zstd(&[], 0).is_err());
        let mut encoder = ZstdEncoder::new().unwrap();
        let mut reference = zstd::bulk::Compressor::new(0).unwrap();
        reference.include_checksum(true).unwrap();
        let mut reused = Vec::new();
        for bytes in [vec![42; 262_144], vec![], (0..262_147).map(|i| ((i * 37 + i / 97) % 251) as u8).collect(), vec![19; 1_048_576]] {
            let mut encoded = encoder.compress(&bytes).unwrap();
            encoder.compress_into(&bytes, &mut reused).unwrap();
            assert_eq!(reused, encoded);
            assert_eq!(encoded, reference.compress(&bytes).unwrap());
            assert_eq!(zstd::stream::decode_all(encoded.as_slice()).unwrap(), bytes);
            assert_eq!(decode_zstd(&encoded, bytes.len()).unwrap(), bytes);
            assert!(decode_zstd(&encoded[..encoded.len() - 1], bytes.len()).is_err());
            assert!(decode_zstd(&encoded, usize::MAX).is_err());
            if !bytes.is_empty() {
                assert!(decode_zstd(&encoded, bytes.len() - 1).is_err());
            }
            assert_ne!(encoded[4] & 4, 0);
            *encoded.last_mut().unwrap() ^= 1;
            assert!(zstd::stream::decode_all(encoded.as_slice()).is_err());
            assert!(decode_zstd(&encoded, bytes.len()).is_err());
        }
    }
}
