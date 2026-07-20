//! FFI bindings to the Swift static lib at `swift/translate.swift`.
//!
//! The Swift side wraps Apple's `Translation.framework` (`TranslationSession`),
//! which has only a SwiftUI-bound public surface. We expose one `@_cdecl`
//! entry point so the unix-side handler can drive it from plain `extern "C"`.
//!
//! Status codes match `wow_shared::TranslateStatus` discriminants byte for
//! byte; the Swift side casts `Int32` directly out of the matching pattern.
//! Any divergence is a coupled edit in the same commit.

use core::ffi::c_char;

#[link(name = "wow_translate_sys", kind = "static")]
unsafe extern "C" {
    /// Synchronous **batch** translate.
    ///
    /// Blocks the calling thread on the Swift task via `DispatchSemaphore`
    /// until the whole batch completes (or the Swift-side deadline elapses).
    ///
    /// All `count` texts share one `(src_lang, tgt_lang)` pair (non-NUL UTF-8
    /// BCP-47 tags). Inputs are packed: `in_lens` is `[u32; count]` of per-item
    /// byte lengths, `in_bytes` their concatenation (total `in_bytes_len`).
    /// Outputs use fixed slots: item `i`'s UTF-8 result (no NUL) is written into
    /// `out_bytes[i * out_slot_bytes ..]`, with the byte count in `out_lens[i]`
    /// (or the required size on `BufferTooSmall`) and that item's
    /// `TranslateStatus` discriminant in `out_status[i]`.
    ///
    /// The return value is a top-level `TranslateStatus`: `Ok` once the batch
    /// was dispatched (inspect `out_status[i]` for per-item outcomes), or an
    /// error discriminant if the whole call was rejected (e.g. `InvalidParams`).
    /// On first use of a pair, macOS may prompt to install the language pack;
    /// the download blocks the batch until complete (or the deadline).
    pub fn wow_translate_sync(
        count: u32,
        src_lang: *const c_char,
        src_lang_len: u32,
        tgt_lang: *const c_char,
        tgt_lang_len: u32,
        in_lens: *const u32,
        in_bytes: *const c_char,
        in_bytes_len: u32,
        out_bytes: *mut c_char,
        out_slot_bytes: u32,
        out_lens: *mut u32,
        out_status: *mut u32,
    ) -> u32;
}
