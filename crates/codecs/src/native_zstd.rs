/// Reusable Zstandard encoder at the library's default level, with frame checksums.
pub struct ZstdEncoder<'a>(zstd::zstd_safe::CCtx<'a>);

fn zstd_error(code: usize) -> std::io::Error {
    std::io::Error::other(zstd::zstd_safe::get_error_name(code))
}

/// Decode Zstandard frames into at most `max_bytes` of output.
pub fn decode_zstd(bytes: &[u8], max_bytes: usize) -> std::io::Result<Vec<u8>> {
    let mut decoded = Vec::new();
    decoded.try_reserve_exact(max_bytes).map_err(std::io::Error::other)?;
    let mut decoder = zstd::zstd_safe::DCtx::try_create().ok_or_else(|| std::io::Error::other("cannot allocate Zstandard decoder"))?;
    decoder.decompress(&mut decoded, bytes).map_err(zstd_error)?;
    Ok(decoded)
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
    fn frames_are_independent_and_checksummed() {
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
