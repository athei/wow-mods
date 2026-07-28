//! `wow_translate.dll` — drop-in replacement for the legacy C++ `WoWTranslate.dll`.
//!
//! Hooks `WoW` 1.12's `UnitXP` Lua C function and routes
//! `UnitXP("WoWTranslate", subcmd, …)` calls through the `wow_mods.dll` bridge
//! to Apple's `Translation.framework` on the unix side (the `WoWTranslate` Lua
//! addon is untouched). The addon-facing subcommand contract is narrowed to the
//! three subcommands the live addon issues (`ping`, `translate_async`, `poll`).
//!
//! This is a native game-side mod loaded from `mods/` (via `dlls.txt`). It has
//! no `.so` of its own; it raw-dylib imports `wow_mods_unix_call` from the
//! `wow_mods.dll` builtin bridge (see `unix_call`), which owns the unixlib
//! pairing. `DllMain` fires a one-shot `InitLogger` thunk before installing the
//! Lua hook.
//!
//! `WoW` 1.12 is 32-bit only and the hook target VAs are 32-bit specific, so the
//! entire crate is gated to `cfg(target_arch = "x86")`. On x64 the resulting
//! `cdylib` is empty (no `DllMain`, no exports).

#![cfg(target_arch = "x86")]

mod hook;
mod lua_ffi;
mod queue;
mod unit_xp_addrs;
mod unix_call;

use core::ffi::c_void;

use wow_shared::identity;

const LOG_TARGET: &str = "wow_translate";
const DLL_PROCESS_ATTACH: u32 = 1;

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
    log_identity(instance);

    // Register the unix-side logger through the bridge before any translate
    // thunk can run. Importing `wow_mods_unix_call` has already forced Wine to
    // load and initialize `wow_mods.dll` (and pair `wow_mods.so`).
    let mut params = wow_shared::InitLoggerParams { reserved: 0 };
    let _ = unix_call::call(&mut params);

    hook::install();
}

/// Name this build in the log, as the first line this mod emits.
///
/// [`identity::BUILD`] says which release the source came from; the image ID is
/// the PDB GUID the linker assigned, which names this exact binary and picks
/// the `.pdb` that symbolicates it out of the release's debug archive.
fn log_identity(instance: *mut c_void) {
    // SAFETY: `instance` is the HINSTANCE the loader passed to DllMain, so this
    // image is mapped in full.
    let id = unsafe { identity::image_id(instance) };
    let id = id.as_deref().unwrap_or("no-image-id");
    let build = identity::BUILD;
    log::info!(target: LOG_TARGET, "wow_translate {build} {id} initialized");
}
