//! `wow_mods.dll` — the PE↔unix bridge builtin for the `WoW` mods.
//!
//! A game-side mod loaded from `mods/` by absolute path is a *native* PE and
//! cannot pair a companion `.so`. So the mods (e.g. `wow_translate.dll`) stay
//! native and reach the unix side through this thin builtin instead: it calls
//! `__wine_init_unix_call` in its own `DllMain` (pairing `wow_mods.so`) and
//! re-exports the dispatcher as `wow_mods_unix_call`. A mod that raw-dylib
//! imports `wow_mods_unix_call` makes Wine auto-load this builtin — and thus
//! pair the `.so` — before the mod's own `DllMain` runs. Mirrors the proven
//! builtin-`.dll`/`.so` pairing from the D3D9→Metal layer.

use core::ffi::c_void;

use log::{error, info};

const LOG_TARGET: &str = "wow_mods";

// Wine unixlib symbols from winecrt0's unix_lib.o.
// __wine_unixlib_handle: set by the loader after __wine_init_unix_call succeeds.
// __wine_unix_call_dispatcher: fn-pointer, initially a lazy-init stub, patched
// by the Wine loader to the real dispatcher.
unsafe extern "C" {
    static __wine_unixlib_handle: u64;
    static __wine_unix_call_dispatcher: unsafe extern "system" fn(u64, u32, *mut c_void) -> i32;
}

unsafe extern "system" {
    fn __wine_init_unix_call() -> i32;
    fn DisableThreadLibraryCalls(lib_module: *mut c_void) -> i32;
}

#[unsafe(export_name = "DllMain")]
pub extern "system" fn dll_main(instance: *mut c_void, reason: u32, _reserved: *mut c_void) -> i32 {
    if reason != 1 {
        return 1;
    }

    wow_shared::init_logger();
    attach_process(instance)
}

#[unsafe(no_mangle)]
pub extern "C" fn wow_mods_unix_call(code: u32, args: *mut c_void) -> i32 {
    dispatch_unix_call(code, args)
}

/// `DLL_PROCESS_ATTACH` body.
///
/// Kept as a private helper so the exported `dll_main` stub stays safe —
/// `not_unsafe_ptr_arg_deref` only checks `pub` functions, so the
/// pointer-passing unsafe work lives here.
fn attach_process(instance: *mut c_void) -> i32 {
    // SAFETY: `instance` is the HINSTANCE the loader passed to DllMain; Win32 accepts it as-is.
    unsafe { DisableThreadLibraryCalls(instance) };

    // SAFETY: Wine-published thunk; init exactly once on PROCESS_ATTACH per the unix-call ABI.
    let status = unsafe { __wine_init_unix_call() };
    if status != 0 {
        error!(target: LOG_TARGET, "__wine_init_unix_call failed");
        return 0;
    }

    info!(target: LOG_TARGET, "unix call initialized");
    1
}

/// Forwards to Wine's unix-call dispatcher.
///
/// Private helper for the same reason as `attach_process` — keeps the exported
/// `wow_mods_unix_call` free of raw-pointer unsafe work that clippy would
/// otherwise flag.
fn dispatch_unix_call(code: u32, args: *mut c_void) -> i32 {
    // SAFETY: Wine-published dispatcher fn-pointer + static unixlib handle; `args` is opaque to us.
    unsafe { (__wine_unix_call_dispatcher)(__wine_unixlib_handle, code, args) }
}
