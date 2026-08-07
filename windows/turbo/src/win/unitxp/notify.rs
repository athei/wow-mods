//! Out-of-focus notifications: a taskbar flash and the system alert sounds.
//!
//! Both act only while the game window is not in the foreground — a player
//! looking at the game needs no nudge — and both are plain OS calls. Under a
//! translating loader they degrade to whatever attention the host maps them
//! to; on a real desktop they behave as named.

/// The client's window accessor — `fastcall(ecx = 0)`, `HWND` in `eax`.
const GET_GAME_WINDOW_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0003_5c30;

/// `FLASHWINFO` for the tray flash-until-foreground request.
#[repr(C)]
struct FlashInfo {
    size: u32,
    window: usize,
    flags: u32,
    count: u32,
    timeout: u32,
}

const FLASHW_TRAY: u32 = 0x2;
const FLASHW_TIMERNOFG: u32 = 0xc;
const SND_ASYNC: u32 = 0x1;
const SND_ALIAS: u32 = 0x1_0000;
const SND_SENTRY: u32 = 0x8_0000;

unsafe extern "system" {
    fn GetModuleHandleA(module_name: *const u8) -> usize;
    fn GetProcAddress(module: usize, proc_name: *const u8) -> usize;
}

#[link(name = "user32")]
unsafe extern "system" {
    fn GetForegroundWindow() -> usize;
    fn FlashWindowEx(info: *const FlashInfo) -> i32;
}

fn game_window() -> usize {
    // SAFETY: a fixed `.text` entry in the live host image (base verified
    // at load); the transmuted signature matches the declared prototype
    // (`__fastcall(ecx = index)`, handle in `eax`).
    let get_window: extern "fastcall" fn(i32) -> usize =
        unsafe { core::mem::transmute(GET_GAME_WINDOW_VA) };
    get_window(0)
}

fn in_foreground() -> bool {
    // SAFETY: plain user32 query with no arguments.
    unsafe { GetForegroundWindow() == game_window() }
}

/// Flash the taskbar entry until the window regains the foreground.
pub fn flash_taskbar_icon() {
    if in_foreground() {
        return;
    }
    let info = FlashInfo {
        size: u32::try_from(size_of::<FlashInfo>()).expect("record is 20 bytes"),
        window: game_window(),
        flags: FLASHW_TRAY | FLASHW_TIMERNOFG,
        count: 0,
        timeout: 0,
    };
    // SAFETY: the record is fully initialized with its size in the leading
    // field, as the call requires.
    unsafe { FlashWindowEx(&raw const info) };
}

/// The system alert aliases the sound command accepts.
const SOUND_ALIASES: [&str; 8] = [
    "SystemAsterisk",
    "SystemDefault",
    "SystemExclamation",
    "SystemExit",
    "SystemHand",
    "SystemQuestion",
    "SystemStart",
    "SystemWelcome",
];

/// Play a system alert sound by alias; whether the request was issued.
pub fn play_system_sound(name: &[u8]) -> bool {
    if in_foreground() {
        return false;
    }
    let Some(&alias) = SOUND_ALIASES
        .iter()
        .find(|&&alias| alias.as_bytes() == name)
    else {
        return false;
    };
    // SAFETY: query only; loads nothing. The multimedia service may simply
    // not be loaded, which answers "not issued".
    let winmm = unsafe { GetModuleHandleA(c"winmm.dll".as_ptr().cast()) };
    if winmm == 0 {
        return false;
    }
    // SAFETY: the handle is live and the name is a literal.
    let play = unsafe { GetProcAddress(winmm, c"PlaySoundW".as_ptr().cast()) };
    if play == 0 {
        return false;
    }
    let mut wide: Vec<u16> = alias.encode_utf16().collect();
    wide.push(0);
    // SAFETY: the export's published signature (alias name, no module,
    // flags).
    let play_sound: extern "stdcall" fn(*const u16, usize, u32) -> i32 =
        unsafe { core::mem::transmute(play) };
    play_sound(wide.as_ptr(), 0, SND_ALIAS | SND_ASYNC | SND_SENTRY) != 0
}
