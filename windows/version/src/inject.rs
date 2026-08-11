use core::{ffi::c_void, ptr};
use std::{ffi::OsStr, path::Path};

use log::{error, info, warn};

const LOG_TARGET: &str = "wow";

const FORMAT_MESSAGE_FROM_SYSTEM: u32 = 0x1000;
const FORMAT_MESSAGE_IGNORE_INSERTS: u32 = 0x200;

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
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetLastError() -> u32;
    fn FormatMessageA(
        flags: u32,
        source: *const c_void,
        code: u32,
        lang: u32,
        buf: *mut u8,
        size: u32,
        args: *mut c_void,
    ) -> u32;
}

/// Read `dlls.txt` next to the host EXE and `LoadLibraryW` each entry.
///
/// Lets users replace external launchers (`CreateRemoteThread` / IAT-patch
/// style) for detour-based mod DLLs — by the time `WoW`'s `main()` runs,
/// every mod listed has had its `DllMain` ATTACH and installed its hooks.
///
/// Called from `version.dll`'s `DllMain` ATTACH. Missing `dlls.txt` is a
/// silent no-op so non-mod users (and any non-game Wine app that happens
/// to share this Wine install) see no log churn.
///
/// Synchronous on the loader-lock'd main thread by design — at this
/// point that's the only thread in the process, so no `DLL_THREAD_ATTACH`
/// cascade against already-resident mods is possible.
pub fn run() {
    let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    else {
        return;
    };
    let cfg_path = exe_dir.join("dlls.txt");
    let text = match std::fs::read_to_string(&cfg_path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            warn!(target: LOG_TARGET, "skipping inject: {} unreadable: {e}", cfg_path.display());
            return;
        }
    };

    let mut loaded = 0usize;
    let mut failed = 0usize;
    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let p = Path::new(line);
        let resolved = if p.is_absolute() {
            p.to_path_buf()
        } else {
            exe_dir.join(p)
        };
        match load_one(line, &resolved) {
            Status::Loaded => loaded += 1,
            Status::Failed => failed += 1,
            Status::AlreadyLoaded => {}
        }
    }
    let attempted = loaded + failed;
    if attempted > 0 {
        info!(target: LOG_TARGET, "dlls.txt: {loaded} of {attempted} loaded");
    }
}

enum Status {
    Loaded,
    AlreadyLoaded,
    Failed,
}

fn strip_comment(line: &str) -> &str {
    line.split_once('#').map_or(line, |(head, _)| head)
}

fn load_one(line: &str, resolved: &Path) -> Status {
    let wide = to_wide(resolved.as_os_str());
    // Pre-probe so a probe-reload's silent refcount bump doesn't pollute
    // the log with duplicate "loaded" lines for the same DLL.
    // SAFETY: `wide` is a NUL-terminated UTF-16 string from `to_wide`; the Win32 thunk reads until NUL.
    let already = unsafe { !GetModuleHandleW(wide.as_ptr()).is_null() };
    // SAFETY: same — `wide` lives until end of this fn; LoadLibraryW reads until NUL.
    let h = unsafe { LoadLibraryW(wide.as_ptr()) };
    if h.is_null() {
        error!(target: LOG_TARGET, "{line} → failed: {}", last_error_text());
        return Status::Failed;
    }
    if already {
        return Status::AlreadyLoaded;
    }
    info!(target: LOG_TARGET, "{line} → loaded");
    Status::Loaded
}

fn to_wide(s: &OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// Render the most recent `GetLastError` as `error <code>: <text>`.
///
/// Uses the Win32 system message table. Trailing `\r\n` from `FormatMessage`
/// is stripped. Empty `FormatMessage` result falls back to just the code.
fn last_error_text() -> String {
    const BUF_LEN: u32 = 256;
    // SAFETY: Win32 GetLastError reads thread-local state, no preconditions.
    let code = unsafe { GetLastError() };
    let mut buf = [0u8; BUF_LEN as usize];
    // SAFETY: `buf` lives for the duration of the call; Win32 writes at most
    // `BUF_LEN` bytes into it.
    let len = unsafe {
        FormatMessageA(
            FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
            ptr::null(),
            code,
            0,
            buf.as_mut_ptr(),
            BUF_LEN,
            ptr::null_mut(),
        )
    } as usize;
    let msg = String::from_utf8_lossy(&buf[..len]);
    let msg = msg.trim();
    if msg.is_empty() {
        format!("error {code}")
    } else {
        format!("error {code}: {msg}")
    }
}
