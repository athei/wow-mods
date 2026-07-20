//! Shared hook-installation plumbing for the injected 32-bit helper DLLs.
//!
//! Both `wow_translate` and `wow_turbo` are standalone `cdylib`s that get
//! injected next to `wow.exe` and patch in-process functions of the host
//! image via `MinHook`. The boilerplate they share — the `DLL_PROCESS_ATTACH`
//! setup, resolving the host image base, and the `create_hook`/`enable_hook`
//! dance — lives here so neither crate reimplements it.
//!
//! The hook targets are 32-bit-specific VAs, so the whole crate is gated to
//! `cfg(target_arch = "x86")`; on x64 it compiles to nothing (its consumers
//! are likewise empty there). Each consumer keeps its own `DllMain` export —
//! that has to be per-cdylib — and calls the helpers below from it.

#![cfg(target_arch = "x86")]

use core::ffi::c_void;

use minhook::MinHook;

const LOG_TARGET: &str = "wow::hook";

#[link(name = "kernel32")]
unsafe extern "system" {
    fn DisableThreadLibraryCalls(lib_module: *mut c_void) -> i32;
    fn GetModuleHandleA(module_name: *const u8) -> usize;
}

/// Standard `DLL_PROCESS_ATTACH` setup for an injected helper DLL.
///
/// Opt out of per-thread loader notifications and bring up the shared logger.
///
/// Call this once, from `DllMain` on `DLL_PROCESS_ATTACH`, before installing
/// any hooks.
///
/// # Safety
///
/// `instance` must be the `HINSTANCE` the loader passed to `DllMain` (this
/// module's own handle); it is forwarded to `DisableThreadLibraryCalls`.
pub unsafe fn on_dll_attach(instance: *mut c_void) {
    // SAFETY: per the contract, `instance` is this module's loader-provided
    // HINSTANCE.
    unsafe { DisableThreadLibraryCalls(instance) };
    wow_shared::init_logger();
}

/// Load base of the host process image (`Wow.exe`).
///
/// Function RVAs in `symbols.toml` are relative to this; resolve the absolute
/// target as `host_image_base() + rva`. Read it rather than hardcoding the
/// base, even though the 1.12 client is non-`DYNAMICBASE` and loads at
/// `0x0040_0000` under Wine.
#[must_use]
pub fn host_image_base() -> usize {
    // SAFETY: GetModuleHandleA(null) returns the module handle of the calling
    // process's own image, which is its load base. No handle is opened.
    unsafe { GetModuleHandleA(core::ptr::null()) }
}

/// Create a `MinHook` detour at `target_va` without enabling it.
///
/// Returns the trampoline — a pointer to a thunk that runs the original prologue
/// then jumps past the patch. Splitting create from [`enable_hook`] lets a caller
/// publish the trampoline (and any per-hook state the detour reads) before the
/// detour can fire. On failure logs against `label` and returns `None`.
///
/// # Safety
///
/// `target_va` must be the address of a function whose calling convention and
/// signature match `detour`; the detour is invoked with the original ABI and a
/// mismatch is undefined behaviour.
pub unsafe fn create_hook(
    target_va: usize,
    detour: *mut c_void,
    label: &str,
) -> Option<*mut c_void> {
    let target = target_va as *mut c_void;

    // SAFETY: per the contract, `target_va` is a real function VA and `detour`
    // has its ABI. MinHook patches the prologue and allocates a near-trampoline
    // holding the displaced bytes.
    match unsafe { MinHook::create_hook(target, detour) } {
        Ok(trampoline) => Some(trampoline),
        Err(e) => {
            log::warn!(target: LOG_TARGET, "MinHook::create_hook({label}) failed: {e}");
            None
        }
    }
}

/// Enable a hook previously created at `target_va` by [`create_hook`].
///
/// Returns `true` on success; logs against `label` and returns `false` on
/// failure.
///
/// # Safety
///
/// `target_va` must be the VA of a hook already created via [`create_hook`].
#[must_use]
pub unsafe fn enable_hook(target_va: usize, label: &str) -> bool {
    let target = target_va as *mut c_void;

    // SAFETY: per the contract, `target_va` is a hook created by a matching
    // `create_hook` call.
    if let Err(e) = unsafe { MinHook::enable_hook(target) } {
        log::warn!(target: LOG_TARGET, "MinHook::enable_hook({label}) failed: {e}");
        return false;
    }

    log::info!(target: LOG_TARGET, "hook installed: {label} @ {target_va:#010x}");
    true
}

/// Queue a hook previously created at `target_va` by [`create_hook`].
///
/// For enabling by the next [`apply_queued`] call. Returns `true` on success;
/// logs against `label` and returns `false` on failure.
///
/// Every `MinHook` enable freezes all threads of the process to patch safely;
/// queueing lets an installer with many hooks pay that freeze once for the
/// whole batch instead of once per hook. The success log says "installed"
/// even though the prologue patch lands at apply time — an apply failure is
/// loud (see [`apply_queued`]), so the per-hook lines remain a faithful
/// record of what is live.
///
/// # Safety
///
/// `target_va` must be the VA of a hook already created via [`create_hook`].
#[must_use]
pub unsafe fn queue_enable_hook(target_va: usize, label: &str) -> bool {
    let target = target_va as *mut c_void;

    // SAFETY: per the contract, `target_va` is a hook created by a matching
    // `create_hook` call.
    if let Err(e) = unsafe { MinHook::queue_enable_hook(target) } {
        log::warn!(target: LOG_TARGET, "MinHook::queue_enable_hook({label}) failed: {e}");
        return false;
    }

    log::info!(target: LOG_TARGET, "hook installed: {label} @ {target_va:#010x}");
    true
}

/// Apply every enable queued via [`queue_enable_hook`] in a single thread-freeze.
///
/// Returns `true` on success. On failure logs a loud error against `label` —
/// a failed apply leaves EVERY queued hook disabled, so the host runs fully
/// stock — and returns `false`. Applying an empty queue is a harmless no-op.
///
/// # Safety
///
/// Each queued `target_va` must carry a detour whose calling convention and
/// signature match the original function (the [`create_hook`] contract); the
/// patches all go live here.
#[must_use]
pub unsafe fn apply_queued(label: &str) -> bool {
    // SAFETY: per the contract, every queued hook pairs a verified target with
    // an ABI-matching detour.
    if let Err(e) = unsafe { MinHook::apply_queued() } {
        log::error!(
            target: LOG_TARGET,
            "MinHook::apply_queued({label}) failed: {e} — every queued hook is still \
             disabled, the host is running fully stock"
        );
        return false;
    }
    true
}

/// Install a `MinHook` detour at `target_va` and enable it in one step.
///
/// On success returns the trampoline — a pointer to a thunk that runs the
/// original prologue then jumps past the patch — which the caller transmutes
/// to the function's typed signature to call through to the unhooked original.
/// On failure logs against `label` and returns `None`; an install failure
/// must never crash the host, so callers skip rather than panic. Use
/// [`create_hook`] + [`enable_hook`] instead when the detour needs state
/// published before it can fire.
///
/// # Safety
///
/// `target_va` must be the address of a function whose calling convention and
/// signature match `detour`. `MinHook` patches the prologue at `target_va` and
/// the detour is invoked with the original ABI; a mismatch is undefined
/// behaviour.
pub unsafe fn install(target_va: usize, detour: *mut c_void, label: &str) -> Option<*mut c_void> {
    // SAFETY: forwards this function's contract — `target_va` is a real function
    // VA and `detour` carries its ABI.
    let trampoline = unsafe { create_hook(target_va, detour, label) }?;

    // SAFETY: `target_va` is the hook just created above.
    if unsafe { enable_hook(target_va, label) } {
        Some(trampoline)
    } else {
        None
    }
}

/// Whether the bytes at `va` match the IDA-style signature `sig`.
///
/// Space-separated hex bytes where `??` is a wildcard, e.g.
/// `"55 8B EC ?? E8 ?? ?? ?? ??"`.
///
/// Patching a function whose prologue no longer matches (a client update, a
/// different build variant, a stale RVA) corrupts the host, so installers verify
/// the signature first and refuse on a non-match. An empty or malformed signature,
/// or any concrete byte that differs, is a non-match.
///
/// # Safety
///
/// `va` must point at memory readable for at least the signature's token count of
/// bytes — the function's own code, located via the manifest RVA.
#[must_use]
pub unsafe fn signature_matches(va: usize, sig: &str) -> bool {
    let count = sig.split_ascii_whitespace().count();
    if count == 0 {
        return false;
    }

    // SAFETY: per the contract, `va` is readable for `count` bytes.
    let bytes = unsafe { core::slice::from_raw_parts(va as *const u8, count) };
    for (token, &actual) in sig.split_ascii_whitespace().zip(bytes) {
        if token == "??" || token == "?" {
            continue;
        }
        match u8::from_str_radix(token, 16) {
            Ok(expected) if expected == actual => {}
            _ => return false,
        }
    }
    true
}
