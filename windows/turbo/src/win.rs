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
mod getname;
mod hooks;
mod script_method;
mod seam_probe;
mod symbols;

use core::ffi::c_void;

use wow_shared::identity;

const LOG_TARGET: &str = "wow_turbo";
const DLL_PROCESS_ATTACH: u32 = 1;

/// ISA baseline this DLL was compiled for.
///
/// Two artifacts ship under this same filename: the nehalem build for the
/// Wine-on-macOS stack, whose translated vector unit is 128-bit, and the
/// haswell build for native Windows (`.cargo/avx.toml`). They are otherwise
/// indistinguishable in a log, and the numeric kernels differ between them, so
/// the startup line has to say which one is running.
const ISA: &str = if cfg!(target_feature = "avx2") {
    "avx2"
} else {
    "sse"
};

unsafe extern "system" {
    fn GetModuleHandleA(module_name: *const u8) -> usize;
    fn GetProcAddress(module: usize, proc_name: *const u8) -> usize;
}

/// `RTL_OSVERSIONINFOW`, the version record `ntdll` fills in.
#[repr(C)]
struct OsVersionInfoW {
    size: u32,
    major: u32,
    minor: u32,
    build: u32,
    platform_id: u32,
    csd_version: [u16; 128],
}

/// Byte count the version query expects in the record's leading field.
///
/// Stated as a literal and checked against the layout below, so the value the
/// call validates cannot drift from the struct it describes.
const OS_VERSION_INFO_SIZE: u32 = 276;
const _: () = assert!(size_of::<OsVersionInfoW>() == OS_VERSION_INFO_SIZE as usize);

/// Log what the process is running on, one line, always.
///
/// Under a translating loader the interesting fact is the machine underneath,
/// which the loader publishes through `ntdll` (`cdecl`, static strings it
/// owns). That is the only route to it from a 32-bit guest, because the Win32
/// version APIs describe the emulated system rather than the host.
///
/// On a real Windows host those exports are absent, and the fallback reports
/// the actual Windows version through `RtlGetVersion` (`stdcall`, present
/// everywhere, and unlike `GetVersionEx` not rewritten by compatibility
/// shims). Every log carries the line either way: a missing line would be
/// indistinguishable from a build predating it, which is the ambiguity this
/// exists to remove.
///
/// The translated-host string is the kernel release, not the marketing
/// version: kernel 25.5 is macOS 26.5, and a patch release does not move it.
/// That is enough to tell which translator generation a captured log came
/// from, which is what a reader of somebody else's log actually needs.
fn log_host() {
    // SAFETY: `ntdll` is loaded in every process before any user DLL; this only
    // takes a handle to it and never loads anything.
    let ntdll = unsafe { GetModuleHandleA(c"ntdll.dll".as_ptr().cast()) };
    if ntdll == 0 {
        return;
    }
    // SAFETY: the module handle is live and the name is a nul-terminated
    // literal; a missing export returns null, which is checked below.
    let host_version = unsafe { GetProcAddress(ntdll, c"wine_get_host_version".as_ptr().cast()) };
    if host_version == 0 {
        log_windows_version(ntdll);
        return;
    }
    // SAFETY: the export's published signature, writing two pointers to
    // loader-owned static strings.
    let host_version: extern "cdecl" fn(*mut *const u8, *mut *const u8) =
        unsafe { core::mem::transmute(host_version) };
    let (mut sysname, mut release) = (core::ptr::null(), core::ptr::null());
    host_version(&raw mut sysname, &raw mut release);
    let name = |p: *const u8| {
        if p.is_null() {
            return String::from("?");
        }
        // SAFETY: the loader wrote a pointer to one of its own nul-terminated
        // static strings, valid for the process lifetime.
        String::from_utf8_lossy(unsafe { core::ffi::CStr::from_ptr(p.cast()) }.to_bytes())
            .into_owned()
    };
    // SAFETY: same module handle and literal-name contract as above.
    let loader = unsafe { GetProcAddress(ntdll, c"wine_get_version".as_ptr().cast()) };
    let loader = if loader == 0 {
        String::from("?")
    } else {
        // SAFETY: the export's published signature, returning a pointer to a
        // loader-owned static string.
        let f: extern "cdecl" fn() -> *const u8 = unsafe { core::mem::transmute(loader) };
        name(f())
    };
    log::info!(
        target: LOG_TARGET,
        "host: {} {} (loader {loader})",
        name(sysname),
        name(release),
    );
}

/// Report the Windows version, for a process not running under a loader.
fn log_windows_version(ntdll: usize) {
    // SAFETY: the module handle is live and the name is a nul-terminated
    // literal; a missing export returns null, which is checked below.
    let get_version = unsafe { GetProcAddress(ntdll, c"RtlGetVersion".as_ptr().cast()) };
    if get_version == 0 {
        return;
    }
    // SAFETY: the export's published signature, filling in a caller-owned
    // record whose leading field states its size.
    let get_version: extern "stdcall" fn(*mut OsVersionInfoW) -> i32 =
        unsafe { core::mem::transmute(get_version) };
    let mut info = OsVersionInfoW {
        size: OS_VERSION_INFO_SIZE,
        major: 0,
        minor: 0,
        build: 0,
        platform_id: 0,
        csd_version: [0; 128],
    };
    if get_version(&raw mut info) != 0 {
        return;
    }
    log::info!(
        target: LOG_TARGET,
        "host: Windows {}.{}.{}",
        info.major, info.minor, info.build,
    );
}

/// Hot script-API entries checked for a prologue another module has patched.
///
/// Only functions this mod does NOT hook. Every hooked entry already reports a
/// patched prologue on its own: the signature check refuses to patch and names
/// the owner, over all of `symbols.toml` rather than a list somebody curated.
/// What is left is the addresses nothing here installs on and so nothing would
/// otherwise look at — measured top rows of the script-API ranking whose cost
/// includes whatever another module does to them.
const PROBED_ENTRIES: [(usize, &str); 4] = [
    (0x003a_1390, "GetName"),
    (0x003a_1460, "GetParent"),
    (0x0011_7020, "UnitName"),
    (0x002f_3890, "lua_pushstring"),
];

/// Report unhooked script-API entries another module has already detoured.
fn log_foreign_detours(image_base: usize) {
    let mut found = 0;
    for (rva, label) in PROBED_ENTRIES {
        let va = image_base + rva;
        if wow_hook::detour_target(va).is_some() {
            found += 1;
            log::info!(
                target: LOG_TARGET,
                "{label} @ {va:#010x} {}",
                wow_hook::prologue_owner(va),
            );
        }
    }
    if found == 0 {
        log::info!(
            target: LOG_TARGET,
            "script API: {} unhooked entries checked, none detoured",
            PROBED_ENTRIES.len(),
        );
    }
}

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
    // SAFETY: `instance` is the HINSTANCE the loader passed to DllMain, so this
    // image is mapped in full.
    let id = unsafe { identity::image_id(instance) };
    let id = id.as_deref().unwrap_or("no-image-id");
    let build = identity::BUILD;
    log::info!(
        target: LOG_TARGET,
        "wow_turbo {build} {ISA} {id} initialized, image_base = {image_base:#010x}",
    );
    log_host();
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
    log_foreign_detours(image_base);
    // Reads whoever owns the `GetName` prologue, which only this side of
    // `install_all` can see — afterwards those bytes are our own detour.
    getname::detect_underlying(image_base);
    symbols::install_all(image_base);
    // Another module installs over this entry after world entry, which the
    // periodic prologue check is what notices; the policy for what to do about
    // it belongs to the entry, so register it now that the hook exists.
    getname::arm_reclaim(image_base);
    // fmod is a separate, packed module (not Wow.exe), so it gets its own install
    // path: hook its FSOUND_Init export now so the mixer reimpl patches in once,
    // right after sound init — no per-frame/per-call poll.
    fmod::install_init_hook();
}
