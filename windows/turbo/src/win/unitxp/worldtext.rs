//! The combat-text command surface, with the renderer deliberately absent.
//!
//! The overlay renderer (fonts, sprites, animation) is not built into this
//! version; the command set it belongs to always modelled that outcome — its
//! own renderer disables itself whenever the graphics side fails — so every
//! command here answers exactly as the original does with a dead scene:
//! settings store and echo back, the second return value reports the scene
//! disabled, and the debug query names the reason. Scripts that check the
//! flag fall back to the stock combat text; the experience-text suppression
//! (the one part needing no renderer) works fully.

use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

/// Why the scene reports disabled, served by the debug query.
pub const DISABLE_REASON: &str =
    "the combat text overlay is not built into this version; stock combat text stays active";

/// The scene never enables in this version.
pub const SCENE_ENABLED: bool = false;

/// Whether scripts asked for the replacement combat text.
static USE_COMBAT_TEXT: AtomicBool = AtomicBool::new(false);
/// The configured font size, stored and echoed for the settings round-trip.
static FONT_SIZE: AtomicI32 = AtomicI32::new(40);
/// The configured text height above nameplates, f64 bits.
static NAMEPLATE_HEIGHT: AtomicU64 = AtomicU64::new(f64::to_bits(55.0));
/// The configured font name.
static FONT_NAME: LazyLock<Mutex<String>> = LazyLock::new(|| Mutex::new(String::new()));

/// See [`USE_COMBAT_TEXT`].
pub fn use_combat_text() -> bool {
    USE_COMBAT_TEXT.load(Ordering::Relaxed)
}

/// See [`USE_COMBAT_TEXT`].
pub fn set_use_combat_text(value: bool) {
    USE_COMBAT_TEXT.store(value, Ordering::Relaxed);
}

/// Set the font size, clamped to `[10, 100]`; answers the stored value.
pub fn set_font_size(value: f64) -> i32 {
    let clamped = value.clamp(10.0, 100.0);
    // the command truncates its clamped double to an integer size, as the
    // reference does
    #[allow(clippy::cast_possible_truncation)]
    let size = clamped as i32;
    FONT_SIZE.store(size, Ordering::Relaxed);
    size
}

/// Set the nameplate text height, clamped to `[0, 256]`; answers it.
pub fn set_nameplate_height(value: f64) -> f64 {
    let clamped = value.clamp(0.0, 256.0);
    NAMEPLATE_HEIGHT.store(clamped.to_bits(), Ordering::Relaxed);
    clamped
}

/// Store the font name; answers a copy for the echo.
pub fn set_font_name(name: String) -> String {
    let mut slot = FONT_NAME.lock().expect("font name is game-thread only");
    *slot = name;
    slot.clone()
}
