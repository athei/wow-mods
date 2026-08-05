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

use core::{
    ffi::c_void,
    sync::atomic::{AtomicU32, Ordering},
};

use minhook::MinHook;

const LOG_TARGET: &str = "wow::hook";

#[link(name = "kernel32")]
unsafe extern "system" {
    fn DisableThreadLibraryCalls(lib_module: *mut c_void) -> i32;
    fn GetModuleHandleA(module_name: *const u8) -> usize;
    fn GetModuleHandleExA(flags: u32, module_name: *const u8, module: *mut usize) -> i32;
    fn GetModuleFileNameA(module: usize, filename: *mut u8, size: u32) -> u32;
}

/// Resolve a module handle from an address inside it, taking no reference.
///
/// `FROM_ADDRESS | UNCHANGED_REFCOUNT`: this only wants to read a name, so
/// pinning the module would outlive the question.
const MODULE_FROM_ADDRESS: u32 = 0x0000_0004 | 0x0000_0002;

/// Where a patched prologue jumps, or `None` if it is the host's own code.
///
/// The three encodings a hooking library reaches for on i386: a relative `jmp`
/// (`E9 rel32`), an indirect one through a pointer slot (`FF 25 abs32`), and a
/// `push imm32` / `ret` pair. A short `jmp` (`EB rel8`) cannot leave the image
/// and so is the host's own branch, never a detour.
#[must_use]
pub const fn detour_target(va: usize) -> Option<usize> {
    // SAFETY: `va` is a host function's entry point, so its first byte is
    // mapped code.
    let opcode = unsafe { *(va as *const u8) };
    match opcode {
        0xe9 => {
            // SAFETY: an `E9` instruction carries a four-byte displacement at
            // `+1`, mapped as part of the instruction that owns it.
            let rel = unsafe { ((va + 1) as *const i32).read_unaligned() };
            Some(va.wrapping_add(5).wrapping_add_signed(rel as isize))
        }
        0xff => {
            // SAFETY: the second opcode byte of a two-byte form is mapped with
            // the first.
            if unsafe { *((va + 1) as *const u8) } != 0x25 {
                return None;
            }
            // SAFETY: an `FF 25` instruction carries the address of its pointer
            // slot at `+2`.
            let slot = unsafe { ((va + 2) as *const usize).read_unaligned() };
            // SAFETY: the slot is the indirection this instruction dereferences
            // on every call, so it is mapped wherever the installer placed it.
            Some(unsafe { *(slot as *const usize) })
        }
        0x68 => {
            // SAFETY: the `ret` that turns a `push imm32` into a jump sits at
            // `+5`, inside the same six-byte prologue.
            if unsafe { *((va + 5) as *const u8) } != 0xc3 {
                return None;
            }
            // SAFETY: the pushed target is the dword at `+1` of the same
            // instruction.
            Some(unsafe { ((va + 1) as *const usize).read_unaligned() })
        }
        _ => None,
    }
}

/// File name and base of the module owning `va`, or `None` if nothing claims it.
///
/// A bare address is only readable by someone holding that process's module
/// map, which the reader of a captured log never is: bases differ per run, so
/// the same number means a different module elsewhere. One point lookup per
/// finding — not an enumeration — is what makes such a line portable. The base
/// comes along because `va - base` is stable across processes: it identifies
/// the function, and thereby the module version, that a log line points at.
#[must_use]
pub fn module_of(va: usize) -> Option<(String, usize)> {
    // Declared in the width the call takes, so the buffer and the length it is
    // told about cannot drift apart and no conversion can fail.
    const PATH_CAP: u32 = 260;
    let mut module = 0usize;
    // SAFETY: the published signature; `FROM_ADDRESS` reads `va` as an address
    // rather than a name, and the handle is written only on success.
    let ok = unsafe { GetModuleHandleExA(MODULE_FROM_ADDRESS, va as *const u8, &raw mut module) };
    if ok == 0 || module == 0 {
        return None;
    }
    let mut buf = [0u8; PATH_CAP as usize];
    // SAFETY: the handle is live and the buffer's length is passed with it.
    let n = unsafe { GetModuleFileNameA(module, buf.as_mut_ptr(), PATH_CAP) } as usize;
    if n == 0 || n >= buf.len() {
        return None;
    }
    let path = String::from_utf8_lossy(&buf[..n]).into_owned();
    let name = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_owned();
    Some((name, module))
}

/// Who owns the prologue at `va`, phrased for a log line.
///
/// A changed prologue has two very different meanings and the byte comparison
/// that finds it cannot tell them apart: another module detoured the function,
/// or this is simply not the build the signature was recorded against. Decoding
/// the jump separates them, and naming the module it lands in is what turns a
/// refusal into something actionable in somebody else's capture. The offset
/// within that module comes too: the absolute target is meaningless outside
/// this process, but name plus offset pins the exact handler, and so the
/// version of the module that owns it.
#[must_use]
pub fn prologue_owner(va: usize) -> String {
    let Some(target) = detour_target(va) else {
        return String::from("no detour decoded — likely a different host build");
    };
    let owner = module_of(target).map_or_else(
        || String::from("an unnamed module"),
        |(name, base)| format!("{name}+{:#x}", target.wrapping_sub(base)),
    );
    format!("detoured to {target:#010x} by {owner}")
}

/// Bytes of a live prologue patch, recorded so a later read can spot a change.
///
/// Five bytes is the whole patch on i386 — the `E9 rel32` the hooking library
/// writes over the prologue — so a change anywhere in them means the hook no
/// longer receives calls.
const PATCH_LEN: usize = 5;

/// One watched prologue: where it is, what it is called, what was written.
struct Patch {
    va: usize,
    label: String,
    /// The prologue as it read right after the enable batch went live.
    ///
    /// `None` until [`snapshot_patches`] runs: enables are queued, so at
    /// registration time the prologue still holds the pre-patch bytes.
    bytes: Option<[u8; PATCH_LEN]>,
    /// Whether a divergence was already reported, so each entry warns once.
    reported: bool,
    /// Called once, if set, when this prologue is found overwritten.
    ///
    /// Returning `true` asks for the patch to be re-asserted. The owner of
    /// the entry decides that, because whether reclaiming an entry is
    /// legitimate depends on what the new owner does and on what the
    /// displaced code underneath can still serve.
    on_overwrite: Option<fn(owner_va: usize) -> bool>,
    /// Whether the one-shot re-assert has already been attempted.
    reasserted: bool,
}

/// Every prologue registered for the overwrite check, in install order.
static PATCHES: std::sync::Mutex<Vec<Patch>> = std::sync::Mutex::new(Vec::new());

/// Register a queued hook's prologue for the periodic overwrite check.
///
/// Call after the enable was queued; the bytes are read later, by
/// [`snapshot_patches`], once the batch is live.
pub fn watch_patch(va: usize, label: &str) {
    if let Ok(mut patches) = PATCHES.lock() {
        patches.push(Patch {
            va,
            label: label.to_owned(),
            bytes: None,
            reported: false,
            on_overwrite: None,
            reasserted: false,
        });
    }
}

/// Ask to be consulted, once, if `va`'s prologue is found overwritten.
///
/// The callback receives the address the rewritten prologue now jumps to and
/// returns whether this hook should be re-asserted over it. Reclaiming an
/// entry another module took is only correct when the caller can say that the
/// new owner's behaviour is reproduced and that the displaced code underneath
/// still serves whatever the caller does not reproduce — which is a question
/// only the entry's owner can answer, so the policy lives there and the
/// mechanism lives here.
pub fn on_overwrite(va: usize, decide: fn(owner_va: usize) -> bool) {
    if let Ok(mut patches) = PATCHES.lock()
        && let Some(patch) = patches.iter_mut().find(|p| p.va == va)
    {
        patch.on_overwrite = Some(decide);
    }
}

/// Re-assert a hook whose prologue another module overwrote.
///
/// Disabling restores the bytes this process displaced at create time, which
/// removes the other module's jump; enabling writes this hook's own jump back
/// over them. Both halves freeze the process's threads, which is why the
/// sequence goes through `MinHook` rather than poking the five bytes directly.
/// The trampoline is untouched, so the delegate path still reaches whatever
/// code was there when this hook was created.
///
/// # Safety
///
/// `target_va` must be the VA of a hook created and enabled by this process.
#[must_use]
pub unsafe fn reassert_hook(target_va: usize, label: &str) -> bool {
    let target = target_va as *mut c_void;
    // SAFETY: per the contract, `target_va` is a live hook of this process.
    if let Err(e) = unsafe { MinHook::disable_hook(target) } {
        log::warn!(target: LOG_TARGET, "MinHook::disable_hook({label}) failed: {e}");
        return false;
    }
    // SAFETY: the hook still exists; only its patch was just withdrawn.
    if let Err(e) = unsafe { MinHook::enable_hook(target) } {
        log::warn!(target: LOG_TARGET, "MinHook::enable_hook({label}) failed: {e}");
        return false;
    }
    true
}

/// Read the prologue at `va`, which stays mapped code for the process lifetime.
const fn prologue_bytes(va: usize) -> [u8; PATCH_LEN] {
    // SAFETY: `va` is a hooked function's entry point in the host image or a
    // loaded module, mapped as code for the life of the process.
    unsafe { *(va as *const [u8; PATCH_LEN]) }
}

/// Record the live prologue bytes of every watched patch.
///
/// Call once, right after the queued enables were applied: what the prologues
/// hold at that moment is what this process installed, and it is the baseline
/// the periodic check compares against.
pub fn snapshot_patches() {
    if let Ok(mut patches) = PATCHES.lock() {
        for patch in patches.iter_mut().filter(|p| p.bytes.is_none()) {
            patch.bytes = Some(prologue_bytes(patch.va));
        }
    }
}

/// Follow a detour to the first address that belongs to a loaded module.
///
/// A hooking library often points the prologue at a thunk it generated in
/// private memory rather than straight at its handler, and that allocation
/// belongs to no module and sits at a different address every run. Returns
/// `va` itself when it is already inside a module, so a caller can resolve
/// the name and the in-module offset from one address. Bounded, because a
/// chain that long is a loop rather than a hook.
#[must_use]
pub fn detour_endpoint(va: usize) -> Option<usize> {
    let mut at = va;
    for _ in 0..4 {
        if module_of(at).is_some() {
            return Some(at);
        }
        at = detour_target(at)?;
    }
    None
}

/// Base address of a loaded module, by file name.
///
/// The companion to [`module_of`] for the case where the name is known and the
/// address is not — resolving a handler inside a module this process never
/// hooked.
#[must_use]
pub fn module_base(name: &str) -> Option<usize> {
    let mut owned = String::with_capacity(name.len() + 1);
    owned.push_str(name);
    owned.push('\0');
    // SAFETY: the published signature; the pointer is a NUL-terminated name
    // living for the call, and the returned handle is a base address or null.
    let module = unsafe { GetModuleHandleA(owned.as_ptr()) };
    (module != 0).then_some(module)
}

/// The first bytes of a generated thunk, for a report that cannot name it.
///
/// When the chain above dead-ends, what it dead-ended *on* is the only thing
/// that makes the next attempt possible, so it goes in the log rather than
/// being summarised away.
#[must_use]
pub fn thunk_bytes(va: usize) -> String {
    use core::fmt::Write as _;

    let mut out = String::new();
    for i in 0..8 {
        // SAFETY: `va` is the target of a jump this process just decoded out
        // of a patched prologue, so it is mapped code.
        let byte = unsafe { *((va + i) as *const u8) };
        let sep = if i == 0 { "" } else { " " };
        let _ = write!(out, "{sep}{byte:02x}");
    }
    out
}

/// Drive [`verify_patches`] from a per-frame caller, cheaply.
///
/// The check has to run whether or not anything is instrumented: an entry
/// another module takes is dead for every player, not only the one who armed
/// a gauge. Every call is a counter bump, and only one in `period` reaches the
/// real check. Single-writer load-add-store — the caller is the game thread.
pub fn verify_periodically(period: u32) {
    static TICKS: AtomicU32 = AtomicU32::new(0);
    let n = TICKS.load(Ordering::Relaxed);
    TICKS.store(n.wrapping_add(1), Ordering::Relaxed);
    if period == 0 || !n.is_multiple_of(period) {
        return;
    }
    verify_patches();
}

/// Warn once per watched prologue that no longer holds the bytes we installed.
///
/// A hook whose prologue changed underneath it is still "installed" by every
/// record this process keeps, while receiving no calls at all — the failure is
/// silent by construction, which is why this check exists. Naming the module
/// the rewritten jump lands in (via [`prologue_owner`]) is what turns the line
/// into something actionable in somebody else's capture. Cheap enough for a
/// periodic caller: one five-byte read per watched entry.
pub fn verify_patches() {
    let Ok(mut patches) = PATCHES.lock() else {
        return;
    };
    for patch in patches.iter_mut().filter(|p| !p.reported) {
        let Some(expected) = patch.bytes else {
            continue;
        };
        if prologue_bytes(patch.va) == expected {
            continue;
        }
        patch.reported = true;
        log::warn!(
            target: LOG_TARGET,
            "{} prologue overwritten at {:#010x} ({}) — this hook no longer receives calls",
            patch.label,
            patch.va,
            prologue_owner(patch.va),
        );
        let Some(decide) = patch.on_overwrite.filter(|_| !patch.reasserted) else {
            continue;
        };
        // One attempt per entry for the life of the process. A module that
        // re-patches on a timer would otherwise turn this into a patch war,
        // and losing that war quietly is worse than losing the entry.
        patch.reasserted = true;
        let Some(owner) = detour_target(patch.va) else {
            continue;
        };
        if !decide(owner) {
            log::info!(
                target: LOG_TARGET,
                "{}: leaving the new owner in place",
                patch.label,
            );
            continue;
        }
        // SAFETY: `patch.va` names a hook this process created and enabled —
        // that is what put it on the watch list.
        if unsafe { reassert_hook(patch.va, &patch.label) } {
            // Re-baseline against what is live now, so a later overwrite by
            // anyone (including this same module) is still detected.
            patch.bytes = Some(prologue_bytes(patch.va));
            patch.reported = false;
            log::info!(
                target: LOG_TARGET,
                "{} re-asserted at {:#010x}",
                patch.label,
                patch.va,
            );
        }
    }
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

/// Mapped size of the host process image, read from its own PE headers.
///
/// With [`host_image_base`] this is the address range the client's own code
/// occupies, which is what tells a call into the client apart from a call into
/// some other module loaded beside it. `e_lfanew` is the dword at `+0x3c` of
/// the DOS header, and a PE32 optional header keeps `SizeOfImage` `0x50` past
/// the start of the NT headers.
#[must_use]
pub fn host_image_size() -> usize {
    let base = host_image_base();
    if base == 0 {
        return 0;
    }
    // SAFETY: `base` is the loaded image's own base, so its DOS header is
    // mapped and `e_lfanew` is the dword at `+0x3c`.
    let nt = base + unsafe { *((base + 0x3c) as *const u32) } as usize;
    // SAFETY: `nt` is the NT header the DOS stub points at, mapped in the same
    // image; `SizeOfImage` is the dword at `+0x50`.
    unsafe { *((nt + 0x50) as *const u32) as usize }
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

/// Whether the bytes at `va` match the wildcard byte pattern `sig`.
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
