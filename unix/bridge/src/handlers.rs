use core::ffi::{c_char, c_void};

use log::{debug, info};
use wow_shared::{InPtrMut, TranslateParams, TranslateStatus, identity};

use crate::LOG_TARGET;

const STATUS_SUCCESS: i32 = 0;
// NTSTATUS bit-pattern reinterpret for the `unix_call` return value.
const STATUS_UNSUCCESSFUL: i32 = 0xC000_0001_u32.cast_signed();

/// One-shot logger init.
///
/// `wow_translate.dll` dispatches this as its first thunk from `DllMain`, after
/// it has wired up its own PE-side `env_logger`.
pub extern "C" fn init_logger_handler(_args: *mut c_void) -> i32 {
    // The PE side can replay this (a second attach), so the one-time process
    // setup runs under a single `Once`.
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        wow_shared::init_logger();
        log_identity();
        // Map the shared crash crumb (cfg-gated no-op unless WOW_CRUMB=1).
        wow_shared::crumb::init();
    });
    STATUS_SUCCESS
}

/// Name this build in the log, as the first line the unix side emits.
///
/// [`identity::BUILD`] says which release the source came from; the image ID is
/// the Mach-O `LC_UUID` the linker assigned, which names this exact binary and
/// picks the `.dSYM` that symbolicates it out of the release's debug archive.
fn log_identity() {
    let id = identity::image_id();
    let id = id.as_deref().unwrap_or("no-image-id");
    let build = identity::BUILD;
    info!(target: LOG_TARGET, "wow_mods.so {build} {id} initialized");
}

pub extern "C" fn translate_handler(args: *mut c_void) -> i32 {
    // SAFETY: unix-call handler params; PE side passes *mut TranslateParams.
    let Some(params) = (unsafe { InPtrMut::<TranslateParams>::opt(args) }) else {
        return -1;
    };

    if params.count == 0
        || params.src_lang_ptr == 0
        || params.tgt_lang_ptr == 0
        || params.in_lens_ptr == 0
        || params.in_bytes_ptr == 0
        || params.out_bytes_ptr == 0
        || params.out_lens_ptr == 0
        || params.out_status_ptr == 0
    {
        return STATUS_UNSUCCESSFUL;
    }

    // SAFETY: PE-supplied source-language tag slice valid for the call duration.
    let src = unsafe {
        core::slice::from_raw_parts(
            params.src_lang_ptr as *const u8,
            params.src_lang_len as usize,
        )
    };
    // SAFETY: PE-supplied target-language tag slice valid for the call duration.
    let tgt = unsafe {
        core::slice::from_raw_parts(
            params.tgt_lang_ptr as *const u8,
            params.tgt_lang_len as usize,
        )
    };

    // SAFETY: FFI into the Swift static lib. The packed input arrays
    // (`in_lens` of `count` u32 lengths, `in_bytes` their concatenation) and the
    // output arrays (`out_bytes` of `count * out_slot_bytes`, `out_lens`,
    // `out_status`, each `count` long) are PE-allocated and exclusive to this
    // blocking call. Swift splits `in_bytes` by `in_lens` into `count` UTF-8
    // texts, batch-translates them through the warm per-pair session, and writes
    // each result into its `out_bytes` slot with the byte count in `out_lens[i]`
    // and the per-item status discriminant in `out_status[i]`.
    let raw = unsafe {
        wow_translate_sys::wow_translate_sync(
            params.count,
            src.as_ptr().cast::<c_char>(),
            params.src_lang_len,
            tgt.as_ptr().cast::<c_char>(),
            params.tgt_lang_len,
            params.in_lens_ptr as *const u32,
            params.in_bytes_ptr as *const c_char,
            params.in_bytes_len,
            params.out_bytes_ptr as *mut c_char,
            params.out_slot_bytes,
            params.out_lens_ptr as *mut u32,
            params.out_status_ptr as *mut u32,
        )
    };

    let status = TranslateStatus::from_repr(raw).unwrap_or(TranslateStatus::Internal);
    if matches!(status, TranslateStatus::Ok) {
        debug!(target: LOG_TARGET, "translate batch of {} dispatched", params.count);
        STATUS_SUCCESS
    } else {
        wow_shared::log_once_warn!(
            target: LOG_TARGET,
            "translate: Swift batch returned {status:?}",
        );
        STATUS_UNSUCCESSFUL
    }
}
