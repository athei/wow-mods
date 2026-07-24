//! Runtime hooks for FMOD3's mixer and MP3-decode DSP.
//!
//! The one family of reimpls that targets a module other than `Wow.exe`.
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

/// `_FSOUND_SetMixer@4`'s RVA into fmod's unpacked image.
///
/// The export we anchor the per-launch base to.
const SETMIXER_RVA: usize = 0x2_1ed4;
/// `fmod__mixer_fpu`'s RVA — the synthesis-filterbank dewindow we replace.
const MIXER_RVA: usize = 0x3_48e0;
/// RVA of the `.data` slot holding the window coefficient table pointer.
///
/// The mixer loads it via `MOV EBX,[disp32]`; the operand is relocated at load,
/// so the detour reads the table base from this global at call time.
const WINDOW_PTR_RVA: usize = 0x5_5144;

/// `fmod__mixer_fpu`'s prologue.
///
/// The `MOV EBX,[disp32]` operand (byte indices 21..25) is the relocated
/// window-table global, so its four bytes are wildcarded; everything else is
/// fixed.
const MIXER_SIG: &str = "55 89 E5 50 53 51 52 56 57 50 8B 45 0C 8B 7D 10 \
                         C1 E0 02 8B 1D ?? ?? ?? ?? 81 C3 40 00 00 00 8B 75 08";

const MIXER_LABEL: &str = "fmod::mixer_fpu";
const INIT_LABEL: &str = "fmod::FSOUND_Init";

/// RVA of the Layer-III alias-reduction stage (`__cdecl(hybrid_buf, gr_info)`).
const ANTIALIAS_RVA: usize = 0x1_d890;
/// RVA of the 8-entry `ca` butterfly table (runtime-computed BSS).
const ANTIALIAS_CA_RVA: usize = 0x5_9720;
/// RVA of the 8-entry `cs` butterfly table (runtime-computed BSS).
const ANTIALIAS_CS_RVA: usize = 0x5_9740;
/// The alias-reduction prologue: the `gr_info` block-mode dispatch.
///
/// No relocated operands in the window (the butterfly tables are addressed
/// later, inside the loop), so every byte is fixed.
const ANTIALIAS_SIG: &str = "8B 44 24 08 83 78 10 02 75 0E 8B 48 14 85 C9 74 63 \
                             B8 01 00 00 00 EB 04";
const ANTIALIAS_LABEL: &str = "fmod::iii_antialias";

/// fmod's unpacked base, resolved at attach.
///
/// Read by the init detour to locate the mixer and its window-table global.
static FMOD_BASE: AtomicUsize = AtomicUsize::new(0);
/// The `FSOUND_Init` trampoline (the original prologue).
///
/// Published before the init hook is enabled so the detour can always call
/// through.
static FSOUND_INIT_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
/// Address of fmod's window-table pointer global (`base + WINDOW_PTR_RVA`).
///
/// Published before the mixer hook is enabled; the mixer detour dereferences it
/// each call. `0` until the mixer hook is live.
static WINDOW_GLOBAL: AtomicUsize = AtomicUsize::new(0);
/// The alias-reduction trampoline (the original prologue).
///
/// Read only by the compare path in `wow_turbo_diff` builds, where the
/// original must run on the live buffer as ground truth.
static ANTIALIAS_TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
/// Address of the `ca` butterfly table (`base + ANTIALIAS_CA_RVA`).
static ANTIALIAS_CA: AtomicUsize = AtomicUsize::new(0);
/// Address of the `cs` butterfly table (`base + ANTIALIAS_CS_RVA`).
static ANTIALIAS_CS: AtomicUsize = AtomicUsize::new(0);

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleA(module_name: *const u8) -> usize;
    fn GetProcAddress(module: usize, proc_name: *const u8) -> usize;
}

/// `FSOUND_Init`.
///
/// `__stdcall(int mixrate, int maxsoftwarechannels, uint flags)` returning a
/// `BOOL`. `extern "system"` is stdcall on the 32-bit target.
type FSoundInitFn = unsafe extern "system" fn(i32, i32, u32) -> i32;

/// Hook fmod's `FSOUND_Init` so the mixer hook installs once, right after sound init.
///
/// Call from `DllMain` attach (fmod is already loaded + unpacked then). On any
/// resolution failure it logs and leaves audio on the stock path.
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

/// Detour for `FSOUND_Init`.
///
/// Run the original (which allocates the window table), then install the mixer
/// hook now that fmod is fully initialized.
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
    install_fmod_hooks();
    result
}

/// One fmod hook: an RVA into the unpacked image plus its install collateral.
///
/// Entries are instantiated in a local array (the detour pointer keeps the
/// type `!Sync`, and nothing needs the table after install).
struct FmodHook {
    /// Target RVA into fmod's unpacked image.
    rva: usize,
    /// Prologue signature, reloc-affected dwords wildcarded.
    sig: &'static str,
    /// Log label.
    label: &'static str,
    /// The detour to install.
    detour: *mut c_void,
    /// Slot for the created trampoline, when the detour delegates.
    ///
    /// Stored (`Release`) before `publish` runs and before the hook enables.
    trampoline: Option<&'static AtomicUsize>,
    /// Publish base-derived globals the detour reads.
    ///
    /// Runs after `create_hook`, before `enable_hook`, so an audio-thread call
    /// landing the instant the patch goes live never reads a `0` slot.
    publish: fn(base: usize),
}

/// Verify and install every fmod detour.
///
/// Called once, from the `FSOUND_Init` detour, after fmod is initialized.
/// Each hook verifies and installs independently: a signature mismatch skips
/// that one hook (stock code keeps running there) and the rest proceed.
fn install_fmod_hooks() {
    let base = FMOD_BASE.load(Ordering::Relaxed);
    if base == 0 {
        return;
    }

    let hooks = [
        FmodHook {
            rva: MIXER_RVA,
            sig: MIXER_SIG,
            label: MIXER_LABEL,
            detour: mixer_detour as *mut c_void,
            trampoline: None,
            publish: publish_mixer_globals,
        },
        FmodHook {
            rva: ANTIALIAS_RVA,
            sig: ANTIALIAS_SIG,
            label: ANTIALIAS_LABEL,
            detour: antialias_detour as *mut c_void,
            trampoline: Some(&ANTIALIAS_TRAMPOLINE),
            publish: publish_antialias_globals,
        },
    ];

    for hook in hooks {
        let va = base + hook.rva;
        // SAFETY: `va` is within fmod's mapped code; reads at most the
        // signature's token count of bytes.
        if !unsafe { wow_hook::signature_matches(va, hook.sig) } {
            log::warn!(
                target: LOG_TARGET,
                "{} signature mismatch at {va:#010x} (base {base:#010x}) — refusing to patch",
                hook.label,
            );
            continue;
        }

        // SAFETY: `va`'s prologue matched the signature, and each table entry's
        // detour matches its target's ABI (documented on the detour).
        let Some(trampoline) = (unsafe { wow_hook::create_hook(va, hook.detour, hook.label) })
        else {
            continue;
        };
        if let Some(slot) = hook.trampoline {
            slot.store(trampoline as usize, Ordering::Release);
        }
        (hook.publish)(base);
        // SAFETY: `va` is the hook just created above.
        let _ = unsafe { wow_hook::enable_hook(va, hook.label) };
    }
}

/// Publish the mixer detour's window-table global.
fn publish_mixer_globals(base: usize) {
    WINDOW_GLOBAL.store(base + WINDOW_PTR_RVA, Ordering::Release);
}

/// Publish the alias-reduction detour's butterfly-table addresses.
///
/// Unlike the mixer's window slot these are the tables themselves (BSS,
/// filled at init), so the detour uses the published addresses directly.
fn publish_antialias_globals(base: usize) {
    ANTIALIAS_CA.store(base + ANTIALIAS_CA_RVA, Ordering::Release);
    ANTIALIAS_CS.store(base + ANTIALIAS_CS_RVA, Ordering::Release);
}

/// Detour for `fmod__mixer_fpu`.
///
/// Read the live window-table base from fmod's relocated global, then run the
/// reimpl. `__cdecl`, matching the original's stack args
/// `(src, phase, out_stride, out)`; the original's return value is unused by
/// its sole caller, so this returns nothing.
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

/// The alias-reduction stage's ABI, for calling the trampoline in compare mode.
#[cfg(wow_turbo_diff)]
type AntialiasFn = unsafe extern "C" fn(*mut f32, *const u8);

/// Detour for the Layer-III alias-reduction stage.
///
/// `__cdecl(hybrid_buf, gr_info)`, void. The original dispatches on two
/// `gr_info` fields: block type 2 (short blocks) folds a single boundary and
/// only when the mixed-block flag is set; every other block type folds
/// `subbands - 1` boundaries. The dispatch is mirrored here 1:1 (including
/// the wrap on a zero subband count, which the kernel documents) and the
/// kernel gets the boundary count plus the two published butterfly tables.
///
/// # Safety
///
/// Installed only over the verified alias-reduction prologue; FMOD invokes it
/// with the original ABI, so `buf` covers the folded region and `gr_info` the
/// three fields read here.
unsafe extern "C" fn antialias_detour(buf: *mut f32, gr_info: *const u8) {
    // SAFETY: `gr_info` is the original's second argument; +0x10 is the
    // block-type field the original reads first.
    let block_type = unsafe { gr_info.wrapping_add(0x10).cast::<u32>().read_unaligned() };
    let boundaries = if block_type == 2 {
        // SAFETY: +0x14 is the mixed-block flag, read only for block type 2.
        if unsafe { gr_info.wrapping_add(0x14).cast::<u32>().read_unaligned() } == 0 {
            return;
        }
        1
    } else {
        // SAFETY: +0x40 is the subband count, read on every non-short path.
        let subbands = unsafe { gr_info.wrapping_add(0x40).cast::<u32>().read_unaligned() };
        subbands.wrapping_sub(1)
    };
    if boundaries == 0 {
        return;
    }
    let ca = ANTIALIAS_CA.load(Ordering::Acquire) as *const f32;
    let cs = ANTIALIAS_CS.load(Ordering::Acquire) as *const f32;

    #[cfg(wow_turbo_diff)]
    if antialias_diff(buf, gr_info, boundaries, ca, cs) {
        return;
    }

    // SAFETY: the detour fires only for genuine alias-reduction calls, so
    // `buf` covers the kernel's documented extent for `boundaries`, and the
    // table addresses were published before the hook was enabled.
    unsafe { crate::math::fmod_mp3::fmod_iii_antialias__1d890(buf, boundaries, ca, cs) };
}

/// Shadow-compare for the alias-reduction hook (compare-mode builds).
///
/// When armed via `WOW_TURBO_DIFF_ARM` (`all` or `fmod__iii_antialias__1d890`):
/// snapshot the folded region, run the reimpl on the snapshot, run the
/// original on the live buffer (the game proceeds on ground truth), then
/// compare the two regions lane-by-lane. Returns whether the call was
/// handled; `false` falls back to the normal reimpl path.
#[cfg(wow_turbo_diff)]
fn antialias_diff(
    buf: *mut f32,
    gr_info: *const u8,
    boundaries: u32,
    ca: *const f32,
    cs: *const f32,
) -> bool {
    use std::sync::LazyLock;

    const LABEL: &str = "fmod__iii_antialias__1d890";
    /// Snapshot capacity: 32 subband lines of 18 f32s (a full long-block granule).
    const REGION_CAP: usize = 32 * 18 * 4;
    static ARMED_NOTE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
    static STATS: super::diff::Stats = super::diff::Stats::new();
    static SELECTED: LazyLock<bool> = LazyLock::new(|| {
        std::env::var("WOW_TURBO_DIFF_ARM").ok().is_some_and(|s| {
            s.split(',').any(|t| {
                let t = t.trim();
                t == "all" || t == LABEL
            })
        })
    });

    if !*SELECTED {
        return false;
    }
    let lines = boundaries.wrapping_add(1);
    let Ok(len_bytes) = usize::try_from(lines).map(|l| l * 18 * 4) else {
        return false;
    };
    if len_bytes > REGION_CAP {
        // A count the snapshot cannot cover; run the normal reimpl path.
        return false;
    }
    let trampoline = ANTIALIAS_TRAMPOLINE.load(Ordering::Acquire);
    if trampoline == 0 {
        return false;
    }
    super::diff::note_armed(&ARMED_NOTE, LABEL);

    let mut shadow = super::diff::Buf::<REGION_CAP>::zeroed();
    // SAFETY: `buf` covers `len_bytes` (the folded region for `boundaries`,
    // which the original is about to read and write in full) and the shadow
    // buffer is at least that large.
    unsafe { core::ptr::copy_nonoverlapping(buf.cast::<u8>(), shadow.0.as_mut_ptr(), len_bytes) };
    // SAFETY: the snapshot covers the kernel's documented extent for
    // `boundaries`; the table addresses were published before enable.
    unsafe {
        crate::math::fmod_mp3::fmod_iii_antialias__1d890(
            shadow.0.as_mut_ptr().cast::<f32>(),
            boundaries,
            ca,
            cs,
        );
    }
    let original =
        // SAFETY: `ANTIALIAS_TRAMPOLINE` holds the trampoline published before
        // this hook was enabled — the original prologue with its own ABI.
        unsafe { core::mem::transmute::<usize, AntialiasFn>(trampoline) };
    // SAFETY: forwarding the original's own arguments under its own ABI.
    unsafe { original(buf, gr_info) };
    // SAFETY: the original just wrote the live region; `len_bytes` is within
    // the extent it touched for this `gr_info`.
    let live = unsafe { core::slice::from_raw_parts(buf.cast::<u8>(), len_bytes) };
    // ±1 ULP: the kernel carries f64 where the original carries the x87 stack,
    // so a double-rounding can move the narrowed f32 by one bit.
    super::diff::region_f32(&STATS, LABEL, false, 1, &shadow.0[..len_bytes], live);
    true
}
