//! Portable zlib-inflate kernel for the MPQ sector-decompressor hook.
//!
//! `Storm__DecompressBlock`'s pure-zlib path (sector method-mask byte `0x02`)
//! runs the client's statically-linked ~2006 zlib: `Zlib_Uncompress` builds a
//! fresh `z_stream` and calls `inflateInit_`/`zlib_inflate`/`inflateEnd` **per
//! sector**. This replaces the decode with `miniz_oxide`'s low-level
//! decompressor, reusing one [`DecompressorOxide`] across sectors, so the
//! per-sector init/end and its pool-allocator churn disappear and the inner loop
//! is a modern `inflate_fast`. `miniz_oxide` is used directly (not via `flate2`,
//! which would force-enable `simd-adler32`'s runtime CPU dispatch) and with
//! `simd` off, so it stays on the build's pinned SSE baseline — no Rosetta AVX2
//! mispick.

use miniz_oxide::inflate::{
    TINFLStatus,
    core::{DecompressorOxide, decompress, inflate_flags},
};

/// Inflate one zlib-wrapped stream `input` fully into `output`, reusing `dec`.
///
/// Returns the produced byte count on a complete, checksum-valid stream, or
/// `None` if `input` is not a valid zlib stream or `output` is too small to hold
/// the result (caller must then fall back to the stock decompressor). `dec` is
/// reset on entry, so a single [`DecompressorOxide`] can be threaded across many
/// calls to amortize setup.
pub fn inflate_zlib(dec: &mut DecompressorOxide, input: &[u8], output: &mut [u8]) -> Option<usize> {
    // Fresh stream each call.
    dec.init();
    // Whole sector decoded in one call into a flat (non-ring) output buffer;
    // parse + verify the zlib header and adler32 trailer to match the client's
    // default `inflateInit_` (windowBits 15) semantics.
    let flags = inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER
        | inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF
        | inflate_flags::TINFL_FLAG_COMPUTE_ADLER32;
    let (status, _consumed, written) = decompress(dec, input, output, 0, flags);
    match status {
        TINFLStatus::Done => Some(written),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use miniz_oxide::{deflate::compress_to_vec_zlib, inflate::core::DecompressorOxide};

    use super::inflate_zlib;

    /// zlib-wrap `data` exactly as an MPQ zlib sector would be stored.
    fn zlib_compress(data: &[u8]) -> Vec<u8> {
        compress_to_vec_zlib(data, 6)
    }

    #[test]
    fn round_trips_into_exact_size_buffer() {
        let data = b"The quick brown fox jumps over the lazy dog. ".repeat(40);
        let comp = zlib_compress(&data);
        let mut dec = DecompressorOxide::new();
        let mut out = vec![0u8; data.len()];
        let n = inflate_zlib(&mut dec, &comp, &mut out).expect("valid stream into exact buffer");
        assert_eq!(n, data.len());
        assert_eq!(&out[..n], &data[..]);
    }

    #[test]
    fn output_too_small_returns_none() {
        let data = b"x".repeat(1000);
        let comp = zlib_compress(&data);
        let mut dec = DecompressorOxide::new();
        let mut out = vec![0u8; 10];
        assert!(inflate_zlib(&mut dec, &comp, &mut out).is_none());
    }

    #[test]
    fn corrupt_input_returns_none() {
        let data = b"some content to mangle".repeat(8);
        let mut comp = zlib_compress(&data);
        // Wreck the deflate payload past the 2-byte zlib header so the header
        // parses but the stream/adler fails.
        for b in comp.iter_mut().skip(2) {
            *b ^= 0xff;
        }
        let mut dec = DecompressorOxide::new();
        let mut out = vec![0u8; data.len()];
        assert!(inflate_zlib(&mut dec, &comp, &mut out).is_none());
    }

    #[test]
    fn one_decompressor_reused_across_distinct_streams() {
        let mut dec = DecompressorOxide::new();
        let inputs = [
            b"first stream payload".repeat(50),
            b"a second, different one".repeat(30),
        ];
        for data in inputs {
            let comp = zlib_compress(&data);
            let mut out = vec![0u8; data.len()];
            let n = inflate_zlib(&mut dec, &comp, &mut out).expect("reused decompressor");
            assert_eq!(&out[..n], &data[..]);
        }
    }
}
