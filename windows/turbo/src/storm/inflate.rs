//! Owning, reusable libdeflate decoder for compressed MPQ sectors.
//!
//! Each instance stays on its creating thread. Allocation failure is reported
//! to the adapter, which delegates to the client's decoder.

use core::ptr::NonNull;

/// One independently reusable decoder allocation.
///
/// The exclusive decode borrow prevents simultaneous reuse of its scratch space.
pub struct Decoder(NonNull<libdeflate_sys::libdeflate_decompressor>);

impl Decoder {
    /// Allocate a decoder, returning `None` if allocation fails.
    pub fn new() -> Option<Self> {
        // SAFETY: the library returns a fresh allocation or null; ownership is
        // held here until the matching free call in Drop.
        NonNull::new(unsafe { libdeflate_sys::libdeflate_alloc_decompressor() }).map(Self)
    }

    /// Decode a complete, checksum-valid zlib stream into the supplied buffer.
    ///
    /// Failure leaves output contents unspecified; the caller must use stock
    /// fallback and must not publish a new output length from a failed decode.
    pub fn decode(&mut self, input: &[u8], output: &mut [u8]) -> Result<usize, Fallback> {
        let mut written = 0;
        // SAFETY: the live decoder is exclusively borrowed; input and output
        // describe disjoint readable/writable slices. `written` is a live out
        // slot. The library neither retains these buffers nor unwinds.
        let status = unsafe {
            libdeflate_sys::libdeflate_zlib_decompress(
                self.0.as_ptr(),
                input.as_ptr().cast(),
                input.len(),
                output.as_mut_ptr().cast(),
                output.len(),
                &raw mut written,
            )
        };
        match status {
            libdeflate_sys::libdeflate_result_LIBDEFLATE_SUCCESS => Ok(written),
            libdeflate_sys::libdeflate_result_LIBDEFLATE_BAD_DATA => Err(Fallback::InvalidStream),
            libdeflate_sys::libdeflate_result_LIBDEFLATE_INSUFFICIENT_SPACE => {
                Err(Fallback::OutputTooSmall)
            }
            _ => Err(Fallback::DecodeRejected),
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: this is the unique live allocation returned by new, freed once.
        unsafe { libdeflate_sys::libdeflate_free_decompressor(self.0.as_ptr()) };
    }
}

/// Identify a compressed pure-zlib sector, excluding raw and mixed-method data.
///
/// An equal input/output size is raw passthrough even when its first byte is 2.
pub fn sector_payload(input: &[u8], capacity: usize) -> Result<&[u8], Fallback> {
    if input.len() < 2 || input.len() > capacity {
        Err(Fallback::SizeIneligible)
    } else if input.len() == capacity {
        Err(Fallback::RawSector)
    } else if input[0] != 2 {
        Err(Fallback::CompressionMask)
    } else {
        Ok(&input[1..])
    }
}

/// Why a sector is delegated to the original client handler.
///
/// Normal format routing is informational; rejected zlib streams and unavailable
/// decoder state are warnings. No input contents or host addresses are logged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fallback {
    InvalidArguments,
    SizeIneligible,
    RawSector,
    CompressionMask,
    DecoderUnavailable,
    DecoderBusy,
    InvalidStream,
    OutputTooSmall,
    DecodeRejected,
    OutputSizeOverflow,
}

impl Fallback {
    /// Stable reason label used in the MPQ diagnostic log.
    pub const fn label(self) -> &'static str {
        match self {
            Self::InvalidArguments => "invalid-arguments",
            Self::SizeIneligible => "size-ineligible",
            Self::RawSector => "raw-sector",
            Self::CompressionMask => "compression-mask",
            Self::DecoderUnavailable => "decoder-unavailable",
            Self::DecoderBusy => "decoder-busy",
            Self::InvalidStream => "invalid-stream",
            Self::OutputTooSmall => "output-too-small",
            Self::DecodeRejected => "decode-rejected",
            Self::OutputSizeOverflow => "output-size-overflow",
        }
    }

    /// Select the severity and count for a sampled fallback.
    ///
    /// Available in normal builds. Counts 1 through 4 and subsequent powers of two
    /// are emitted; successful libdeflate calls never touch these counters.
    #[cold]
    pub fn sample(self, stock_result: u32) -> Option<(log::Level, u32)> {
        static COUNTS: [[core::sync::atomic::AtomicU32; 2]; 10] =
            [const { [const { core::sync::atomic::AtomicU32::new(0) }; 2] }; 10];
        let level = if stock_result != 0
            && matches!(
                self,
                Self::RawSector | Self::CompressionMask | Self::SizeIneligible
            ) {
            log::Level::Info
        } else {
            log::Level::Warn
        };
        if !log::log_enabled!(target: "wow::mpq", level) {
            return None;
        }
        let counter = &COUNTS[self as usize][usize::from(stock_result == 0)];
        fallback_sample(counter).map(|count| (level, count))
    }
}

/// Count a fallback and select a bounded set of diagnostic samples.
fn fallback_sample(counter: &core::sync::atomic::AtomicU32) -> Option<u32> {
    let count = counter
        .fetch_add(1, core::sync::atomic::Ordering::Relaxed)
        .wrapping_add(1);
    (count != 0 && (count <= 4 || count.is_power_of_two())).then_some(count)
}

#[cfg(test)]
mod tests {
    use miniz_oxide::deflate::compress_to_vec_zlib;

    use super::{Decoder, Fallback, fallback_sample, sector_payload};

    #[test]
    fn only_compressed_pure_zlib_sectors_are_eligible() {
        assert_eq!(sector_payload(&[2, 120, 156], 4), Ok(&[120, 156][..]));
        for input in [&[][..], &[2]] {
            assert_eq!(sector_payload(input, 4096), Err(Fallback::SizeIneligible));
        }
        for mask in [0, 10, 18] {
            assert_eq!(
                sector_payload(&[mask, 120, 156], 4096),
                Err(Fallback::CompressionMask)
            );
        }
        assert_eq!(sector_payload(&[2, 120, 156], 3), Err(Fallback::RawSector));
        assert_eq!(
            sector_payload(&[2, 120, 156], 2),
            Err(Fallback::SizeIneligible)
        );
    }

    #[test]
    fn fallback_reasons_have_distinct_searchable_labels() {
        let reasons = [
            Fallback::InvalidArguments,
            Fallback::SizeIneligible,
            Fallback::RawSector,
            Fallback::CompressionMask,
            Fallback::DecoderUnavailable,
            Fallback::DecoderBusy,
            Fallback::InvalidStream,
            Fallback::OutputTooSmall,
            Fallback::DecodeRejected,
            Fallback::OutputSizeOverflow,
        ];
        let mut labels = std::collections::HashSet::new();
        for reason in reasons {
            let label = reason.label();
            assert!(!label.is_empty() && !label.contains(char::is_whitespace));
            assert!(labels.insert(label));
            // Also exercise the facade with its normal disabled test logger.
            let _ = reason.sample(0);
        }
    }

    #[test]
    fn fallback_sampling_is_bounded_and_counters_are_independent() {
        use core::sync::atomic::AtomicU32;
        let count = AtomicU32::new(0);
        let samples: Vec<_> = (0..1000).filter_map(|_| fallback_sample(&count)).collect();
        assert_eq!(samples, [1, 2, 3, 4, 8, 16, 32, 64, 128, 256, 512]);
        assert_eq!(fallback_sample(&AtomicU32::new(0)), Some(1));
        assert_eq!(fallback_sample(&AtomicU32::new(u32::MAX)), None);
    }

    #[test]
    fn reuse_matches_expected_bytes_and_rejects_bad_streams() {
        let mut dec = Decoder::new().expect("decoder allocation");
        for data in [
            vec![],
            b"sector payload".repeat(300),
            (0..=255).cycle().take(4096).collect(),
        ] {
            let stream = compress_to_vec_zlib(&data, 6);
            let mut out = vec![0; data.len()];
            for _ in 0..3 {
                assert_eq!(dec.decode(&stream, &mut out), Ok(data.len()));
                assert_eq!(out, data);
            }
            for len in 0..stream.len() {
                assert!(dec.decode(&stream[..len], &mut out).is_err());
            }
            let mut bad = stream.clone();
            *bad.last_mut().unwrap() ^= 1;
            assert_eq!(dec.decode(&bad, &mut out), Err(Fallback::InvalidStream));
            if !data.is_empty() {
                assert_eq!(
                    dec.decode(&stream, &mut out[..data.len() - 1]),
                    Err(Fallback::OutputTooSmall)
                );
            }
            assert_eq!(dec.decode(&stream, &mut out), Ok(data.len()));
        }
    }

    #[test]
    fn concurrent_decoders_have_independent_state() {
        let handles: Vec<_> = (0..4)
            .map(|byte| {
                std::thread::spawn(move || {
                    let mut dec = Decoder::new().expect("decoder allocation");
                    let data = vec![byte; 4096];
                    let stream = compress_to_vec_zlib(&data, 6);
                    let mut out = vec![0; data.len()];
                    for _ in 0..100 {
                        assert_eq!(dec.decode(&stream, &mut out), Ok(data.len()));
                        assert_eq!(out, data);
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
    }
}
