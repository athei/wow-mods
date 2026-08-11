mod inject;

use core::ffi::c_void;

use wow_shared::identity;

const LOG_TARGET: &str = "wow";
const DLL_PROCESS_ATTACH: u32 = 1;

/// `VIF_CANNOTREADSRC` — generic install-time failure.
///
/// Returned by the `VerInstallFile{A,W}` stubs below. Games we care about
/// don't call these; the value matches what an unprivileged install attempt
/// would surface on real Windows.
const VIF_CANNOTREADSRC: u32 = 0x80;

#[cfg_attr(
    target_arch = "x86",
    link(
        name = "kernel32",
        kind = "raw-dylib",
        import_name_type = "undecorated"
    )
)]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn DisableThreadLibraryCalls(lib_module: *mut c_void) -> i32;
}

/// `version.dll` proxy.
///
/// Replaces Wine's builtin `version.dll` for the bundled Wine in
/// `WINE_INSTALL_DIR`.
///
/// Statically imported by every `WoW` client we care about, so its `DllMain
/// ATTACH` runs during process init in the same Wine process as the game —
/// exactly the timing `dll_loader.exe` was emulating from a separate Wine
/// process.
///
/// Job: read `dlls.txt` next to the host EXE and `LoadLibraryW` each
/// entry. The 14 real version.dll APIs are forwarded to `kernelbase.dll`
/// at link time via `version.def`; only `VerInstallFile{A,W}` (which no
/// game we ship calls) live in this image as `log_once_warn!` stubs.
#[unsafe(export_name = "DllMain")]
pub extern "system" fn dll_main(instance: *mut c_void, reason: u32, _reserved: *mut c_void) -> i32 {
    if reason == DLL_PROCESS_ATTACH {
        attach_process(instance);
    }
    1
}

/// `DLL_PROCESS_ATTACH` body.
///
/// Private helper so the exported `dll_main` stub stays safe — clippy's
/// `not_unsafe_ptr_arg_deref` only checks `pub` functions.
fn attach_process(instance: *mut c_void) {
    // SAFETY: `instance` is the HINSTANCE the loader passed to DllMain; Win32 accepts it as-is.
    unsafe { DisableThreadLibraryCalls(instance) };
    wow_shared::init_logger();
    log_identity(instance);
    crate::inject::run();
}

/// Name this build in the log, as the first line the injector emits.
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
    log::info!(target: LOG_TARGET, "version.dll {build} {id} loaded");
}

#[unsafe(export_name = "VerInstallFileA")]
pub extern "system" fn ver_install_file_a(
    _flags: u32,
    _src_file_name: *const u8,
    _dest_file_name: *const u8,
    _src_path: *const u8,
    _dest_path: *const u8,
    _cur_dir: *const u8,
    _tmp_file: *mut u8,
    _tmp_file_len: *mut u32,
) -> u32 {
    wow_shared::log_once_warn!(target: LOG_TARGET, "VerInstallFileA stub → VIF_CANNOTREADSRC");
    VIF_CANNOTREADSRC
}

#[unsafe(export_name = "VerInstallFileW")]
pub extern "system" fn ver_install_file_w(
    _flags: u32,
    _src_file_name: *const u16,
    _dest_file_name: *const u16,
    _src_path: *const u16,
    _dest_path: *const u16,
    _cur_dir: *const u16,
    _tmp_file: *mut u16,
    _tmp_file_len: *mut u32,
) -> u32 {
    wow_shared::log_once_warn!(target: LOG_TARGET, "VerInstallFileW stub → VIF_CANNOTREADSRC");
    VIF_CANNOTREADSRC
}
