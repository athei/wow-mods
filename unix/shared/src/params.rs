use strum::FromRepr;

use super::{Thunk, Thunks};

// The whole PE↔unix thunk protocol assumes a `repr(C)` `u64` is 8-byte aligned
// on all three targets (i686 + x86_64 PE, x86_64 unix). If a 32-bit target ever
// aligned `u64` to 4, the `u64` fields after the odd run of `u32`s in
// `TranslateParams` would shift and the unix handler would write out-params past
// the PE caller's (often stack-allocated) struct — smashing its return address.
// Checked at compile time on every target this crate builds for.
const _: () = {
    #[repr(C)]
    struct U64After4 {
        a: u32,
        b: u64,
    }
    assert!(core::mem::offset_of!(U64After4, b) == 8);
    assert!(core::mem::size_of::<TranslateParams>() == 80);
    assert!(core::mem::align_of::<TranslateParams>() == 8);
};

/// One-shot "register `env_logger` on the unix side" thunk.
///
/// Fired once from `wow_translate.dll`'s `DllMain` on load, before any other
/// thunk that might want to log. No payload — the `reserved` field keeps the
/// struct non-zero-sized so the pointer handed across the boundary is distinct.
#[repr(C, align(8))]
pub struct InitLoggerParams {
    // Keeps the struct non-zero-sized so the pointer handed across the
    // PE/Unix boundary is distinct. Constructed by name across crates, hence pub.
    pub reserved: u64,
}

impl Thunk for InitLoggerParams {
    const CODE: u32 = Thunks::InitLogger as u32;
}

/// Synchronous on-device translation through Apple's `Translation.framework`.
///
/// Translates a **batch** of `count` UTF-8 texts that all share one
/// `(src, tgt)` language pair (Apple's session API is per-pair). A single
/// request is just `count == 1`. `wow_translate.dll`'s worker thread coalesces
/// whatever is queued into one of these calls so a burst of chat costs ~one
/// model invocation instead of one per message.
///
/// Inputs are packed: `in_lens` is a `[u32; count]` of per-item byte lengths and
/// `in_bytes` is their concatenation (total `in_bytes_len`). Outputs use fixed
/// slots: item `i` is written into `out_bytes[i * out_slot_bytes ..]` as UTF-8
/// (no NUL); `out_lens[i]` gets the byte count written (or, on
/// [`TranslateStatus::BufferTooSmall`], the required size) and `out_status[i]`
/// gets that item's [`TranslateStatus`] discriminant. All buffers are
/// PE-allocated; only fixed-width ints cross the boundary.
#[repr(C, align(8))]
pub struct TranslateParams {
    pub src_lang_ptr: u64,   // in: UTF-8 BCP-47, e.g. "zh-Hans" (shared)
    pub tgt_lang_ptr: u64,   // in: UTF-8 BCP-47, e.g. "en" (shared)
    pub in_lens_ptr: u64,    // in: -> [u32; count] per-item input byte lengths
    pub in_bytes_ptr: u64,   // in: -> concatenated UTF-8 of all inputs
    pub out_bytes_ptr: u64,  // in: -> count * out_slot_bytes (PE-allocated)
    pub out_lens_ptr: u64,   // out: -> [u32; count] bytes written (or required)
    pub out_status_ptr: u64, // out: -> [u32; count] TranslateStatus per item
    pub count: u32,          // in: batch size (>= 1)
    pub in_bytes_len: u32,   // in: total input bytes (sum of in_lens)
    pub out_slot_bytes: u32, // in: capacity per output slot
    pub src_lang_len: u32,   // in
    pub tgt_lang_len: u32,   // in
    // allow: FFI struct padding; keeps the u32 count even so there is no
    // implicit trailing pad and 32-bit PE / 64-bit Unix agree on layout.
    pub pad0: u32,
}

impl Thunk for TranslateParams {
    const CODE: u32 = Thunks::Translate as u32;
}

/// Wire-format result code for a translated item. `#[repr(u32)]` so it crosses
/// the PE↔unix boundary as a plain fixed-width int.
///
/// PE callers default to `Internal` before the call so a handler that returns
/// without writing status doesn't surface as a spurious `Ok`.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, FromRepr)]
pub enum TranslateStatus {
    Internal = 0,
    Ok = 1,
    Unavailable = 2,
    InvalidParams = 3,
    UnsupportedPair = 4,
    LanguagePackMissing = 5,
    Timeout = 6,
    BufferTooSmall = 7,
}

#[cfg(test)]
mod tests {
    use super::TranslateParams;

    #[test]
    fn translate_layout() {
        // 7 * u64 + 6 * u32 = 56 + 24 = 80, already 8-aligned.
        assert_eq!(core::mem::align_of::<TranslateParams>(), 8);
        assert_eq!(core::mem::size_of::<TranslateParams>(), 80);
    }
}
