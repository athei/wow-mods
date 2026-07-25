//! 32-bit host-image machinery (gated as a whole by `lib.rs`).
//!
//! Holds everything that only exists for the shipped `i686` DLL: the generated
//! [`symbols`] table (function types, trampoline storage, the `extern "abi"`
//! thunks `MinHook` installs, and `install_all`), the [`hooks`] FFI adapters the
//! thunks dispatch to, and the `DllMain` entry. The portable reimplementation
//! logic the adapters call lives in `crate::math`.

#[cfg(wow_turbo_diff)]
mod diff;
mod events;
mod fmod;
mod hooks;
mod symbols;

use core::ffi::c_void;

const LOG_TARGET: &str = "wow_turbo";
const DLL_PROCESS_ATTACH: u32 = 1;

/// Build identity, stamped in by `build.rs` from `git describe`.
///
/// Named in the startup line so a captured session log says which build wrote
/// it; `unknown` when built outside a checkout.
const BUILD: &str = env!("WOW_TURBO_BUILD");

/// The 1.12 client is non-`DYNAMICBASE` and always loads here.
///
/// The reimpls read fixed host globals by absolute address (no per-call base
/// lookup), so this is verified once at load before any hook is installed; a
/// mismatch refuses to patch rather than reading the wrong addresses.
const EXPECTED_IMAGE_BASE: usize = 0x0040_0000;

#[unsafe(export_name = "DllMain")]
pub extern "system" fn dll_main(instance: *mut c_void, reason: u32, _reserved: *mut c_void) -> i32 {
    if reason == DLL_PROCESS_ATTACH {
        attach_process(instance);
    }
    1
}

fn attach_process(instance: *mut c_void) {
    // SAFETY: `instance` is the HINSTANCE the loader passed to DllMain.
    unsafe { wow_hook::on_dll_attach(instance) };
    // Join the shared breadcrumb ring so the generated thunks' `record` calls
    // land in the same mmap the crash handler dumps (no-op unless wow_crumb).
    wow_shared::crumb::init();
    let image_base = wow_hook::host_image_base();
    log::info!(
        target: LOG_TARGET,
        "wow_turbo {BUILD} initialized, image_base = {image_base:#010x}",
    );
    // The reimplementations read host globals by absolute address, valid only at
    // the fixed base of the non-DYNAMICBASE 1.12 client. Fail loudly at load if
    // that assumption ever breaks rather than reading wrong addresses.
    assert!(
        image_base == EXPECTED_IMAGE_BASE,
        "host image base {image_base:#010x} != expected {EXPECTED_IMAGE_BASE:#010x}",
    );
    // Precompute the engine-clock (`OsGetTimeMs`) tick->ms scale from our own TSC
    // calibration, BEFORE install_all so the hook never observes an unset scale.
    // `tsc_hz()` blocks ~50 ms here on first use — a plain calibrate on the loader
    // thread, no thread spawned (unlike a `CreateThread`, which would deadlock the
    // loader lock). One-time cost at this early, single-threaded mod load.
    hooks::init_engine_clock(wow_shared::tsc::tsc_hz());
    symbols::install_all(image_base);
    // fmod is a separate, packed module (not Wow.exe), so it gets its own install
    // path: hook its FSOUND_Init export now so the mixer reimpl patches in once,
    // right after sound init — no per-frame/per-call poll.
    fmod::install_init_hook();
}
