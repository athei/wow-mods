//! Runtime hook for FMOD3's `fmod__mixer_fpu` — the one reimpl that targets a
//! module other than `Wow.exe`.
//!
//! `fmod.dll` is a separate, packed module that unpacks its code into runtime
//! memory, so the `symbols.toml` install path — RVA over `Wow.exe`'s `.text`,
//! patched once at attach — can't reach it. It is a static import of `Wow.exe`,
//! though, so by the time our `DllMain` runs it is already loaded and unpacked;
//! its exports resolve and the mixer's prologue is in place.
//!
//! Rather than poll a hot `Wow.exe` hook to discover when the mixer is live, we
//! hook fmod's own `FSOUND_Init` export at attach. The host calls `FSOUND_Init`
//! once, early (before any music plays); the detour runs the original — which allocates
//! the window coefficient table the mixer reads — and then installs the mixer
//! hook, exactly once, on the cold init path. Nothing sits on a per-frame or
//! per-call hot path.
//!
//! fmod's unpacked image lands at a runtime base, so addresses are resolved per
//! launch: `_FSOUND_SetMixer` has a known RVA into the unpacked image
//! (`== fmod_unpacked.dll`), and every other RVA — the mixer and its window-table
//! global — is relative to the same base.

use core::{
    ffi::c_void,
    sync::atomic::{AtomicUsize, Ordering},
};

use super::LOG_TARGET;

/// `_FSOUND_SetMixer@4`'s RVA into fmod's unpacked image — the export we anchor
/// the per-launch base to.
const SETMIXER_RVA: usize = 0x2_1ed4;
/// `fmod__mixer_fpu`'s RVA — the synthesis-filterbank dewindow we replace.
const MIXER_RVA: usize = 0x3_48e0;
/// RVA of the `.data` slot holding the window coefficient table pointer. The
/// mixer loads it via `MOV EBX,[disp32]`; the operand is relocated at load, so
/// the detour reads the table base from this global at call time.
const WINDOW_PTR_RVA: usize = 0x5_5144;

/// `fmod__mixer_fpu`'s prologue. The `MOV EBX,[disp32]` operand (byte indices
/// 21..25) is the relocated window-table global, so its four bytes are wildcarded;
/// everything else is fixed.
const MIXER_SIG: &str = "55 89 E5 50 53 51 52 56 57 50 8B 45 0C 8B 7D 10 \
                         C1 E0 02 8B 1D ?? ?? ?? ?? 81 C3 40 00 00 00 8B 75 08";

const MIXER_LABEL: &str = "fmod::mixer_fpu";
const INIT_LABEL: &str = "fmod::FSOUND_Init";

/// fmod's unpacked base, resolved at attach; read by the init detour to locate
/// the mixer and its window-table global.
static FMOD_BASE: AtomicUsize = AtomicUsize::new(0);
/// The `FSOUND_Init` trampoline (the original prologue), published before the
/// init hook is enabled so the detour can always call through.
static FSOUND_INIT_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
/// Address of fmod's window-table pointer global (`base + WINDOW_PTR_RVA`),
/// published before the mixer hook is enabled; the mixer detour dereferences it
/// each call. `0` until the mixer hook is live.
static WINDOW_GLOBAL: AtomicUsize = AtomicUsize::new(0);

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleA(module_name: *const u8) -> usize;
    fn GetProcAddress(module: usize, proc_name: *const u8) -> usize;
}

/// `FSOUND_Init` — `__stdcall(int mixrate, int maxsoftwarechannels, uint flags)`
/// returning a `BOOL`. `extern "system"` is stdcall on the 32-bit target.
type FSoundInitFn = unsafe extern "system" fn(i32, i32, u32) -> i32;

/// Hook fmod's `FSOUND_Init` so the mixer hook installs once, right after sound
/// init. Call from `DllMain` attach (fmod is already loaded + unpacked then). On
/// any resolution failure it logs and leaves audio on the stock path.
pub fn install_init_hook() {
    // SAFETY: a NUL-terminated module name.
    let module = unsafe { GetModuleHandleA(c"fmod.dll".as_ptr().cast()) };
    if module == 0 {
        log::warn!(target: LOG_TARGET, "{INIT_LABEL}: fmod.dll not loaded at attach — audio stays stock");
        return;
    }
    // SAFETY: `module` is a live HMODULE and the export name is NUL-terminated.
    let setmixer = unsafe { GetProcAddress(module, c"_FSOUND_SetMixer@4".as_ptr().cast()) };
    // SAFETY: `module` is a live HMODULE and the export name is NUL-terminated.
    let init = unsafe { GetProcAddress(module, c"_FSOUND_Init@12".as_ptr().cast()) };
    if setmixer == 0 || init == 0 {
        log::warn!(target: LOG_TARGET, "{INIT_LABEL}: fmod exports unresolved — audio stays stock");
        return;
    }
    let base = setmixer - SETMIXER_RVA;
    FMOD_BASE.store(base, Ordering::Relaxed);

    // SAFETY: `init` is fmod's live `FSOUND_Init` export and `fsound_init_detour`
    // is `extern "system"` matching its `__stdcall(i32, i32, u32) -> i32`.
    let Some(trampoline) =
        (unsafe { wow_hook::create_hook(init, fsound_init_detour as *mut c_void, INIT_LABEL) })
    else {
        return;
    };
    // Publish the trampoline before enabling so the detour can always call through.
    FSOUND_INIT_ORIGINAL.store(trampoline as usize, Ordering::Release);
    // SAFETY: `init` is the hook just created above.
    let _ = unsafe { wow_hook::enable_hook(init, INIT_LABEL) };
}

/// Detour for `FSOUND_Init`: run the original (which allocates the window table),
/// then install the mixer hook now that fmod is fully initialized.
///
/// # Safety
///
/// Installed only over fmod's verified `FSOUND_Init` export; the host invokes it
/// with the original stdcall ABI.
unsafe extern "system" fn fsound_init_detour(mixrate: i32, max_channels: i32, flags: u32) -> i32 {
    let original = FSOUND_INIT_ORIGINAL.load(Ordering::Acquire);
    // SAFETY: `FSOUND_INIT_ORIGINAL` holds the trampoline published before this
    // hook was enabled — the original prologue with `FSOUND_Init`'s ABI.
    let call_original: FSoundInitFn =
        unsafe { core::mem::transmute::<usize, FSoundInitFn>(original) };
    // SAFETY: forwarding `FSOUND_Init`'s own arguments under its own ABI.
    let result = unsafe { call_original(mixrate, max_channels, flags) };
    install_mixer_hook();
    result
}

/// Verify and install the `fmod__mixer_fpu` detour. Called once, from the
/// `FSOUND_Init` detour, after fmod is initialized.
fn install_mixer_hook() {
    let base = FMOD_BASE.load(Ordering::Relaxed);
    if base == 0 {
        return;
    }
    let window_global = base + WINDOW_PTR_RVA;
    let mixer_va = base + MIXER_RVA;

    // SAFETY: `mixer_va` is within fmod's mapped code; reads at most the
    // signature's token count of bytes.
    if !unsafe { wow_hook::signature_matches(mixer_va, MIXER_SIG) } {
        log::warn!(
            target: LOG_TARGET,
            "{MIXER_LABEL} signature mismatch at {mixer_va:#010x} (base {base:#010x}) — refusing to patch",
        );
        return;
    }

    // SAFETY: `mixer_va`'s prologue matched the signature, and `mixer_detour` is
    // `extern "C"` matching the original `__cdecl(src, phase, out_stride, out)`.
    let created =
        unsafe { wow_hook::create_hook(mixer_va, mixer_detour as *mut c_void, MIXER_LABEL) };
    if created.is_none() {
        return;
    }
    // Publish the window global before enabling so a mixer-thread call landing the
    // instant the patch goes live never reads a `0` slot.
    WINDOW_GLOBAL.store(window_global, Ordering::Release);
    // SAFETY: `mixer_va` is the hook just created above.
    let _ = unsafe { wow_hook::enable_hook(mixer_va, MIXER_LABEL) };
}

/// Detour for `fmod__mixer_fpu`: read the live window-table base from fmod's
/// relocated global, then run the reimpl. `__cdecl`, matching the original's
/// stack args `(src, phase, out_stride, out)`; the original's return value is
/// unused by its sole caller, so this returns nothing.
///
/// # Safety
///
/// Installed only over the verified `fmod__mixer_fpu` prologue; FMOD invokes it
/// with the original ABI and buffers, which satisfy the kernel's contract.
unsafe extern "C" fn mixer_detour(src: *const f32, phase: i32, out_stride: i32, out: *mut i16) {
    let window_global = WINDOW_GLOBAL.load(Ordering::Acquire);
    // SAFETY: the detour is enabled only after `window_global` (fmod's window-table
    // pointer global) is published; it holds the table base FMOD allocated at init.
    let window = unsafe { *(window_global as *const *const f32) };
    // SAFETY: the detour fires only for genuine `fmod__mixer_fpu` calls, so
    // `window`/`src`/`out` are the buffers the original would window over.
    unsafe { crate::math::fmod_mixer::fmod_mixer_fpu__348e0(window, src, phase, out_stride, out) };
}
