//! Process-global feature settings, toggled through the dispatcher.
//!
//! Nothing here persists: the command channel IS the configuration surface,
//! and the companion addon re-issues every setting at login and after every
//! panel change. All statics are single-writer (the game thread); atomics
//! provide shared mutability for a `static`, not cross-thread ordering.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Whether the player is in world, per the script-side notifications.
static IN_WORLD: AtomicBool = AtomicBool::new(false);

/// See [`IN_WORLD`].
pub fn set_in_world(value: bool) {
    IN_WORLD.store(value, Ordering::Relaxed);
}

/// See [`IN_WORLD`].
pub fn in_world() -> bool {
    IN_WORLD.load(Ordering::Relaxed)
}

/// Weather suppression, one flag per precipitation type.
static WEATHER_NO_RAIN: AtomicBool = AtomicBool::new(false);
/// See [`WEATHER_NO_RAIN`].
static WEATHER_NO_SNOW: AtomicBool = AtomicBool::new(false);
/// See [`WEATHER_NO_RAIN`].
static WEATHER_NO_SANDSTORM: AtomicBool = AtomicBool::new(false);

/// See [`WEATHER_NO_RAIN`]; order rain, snow, sandstorm.
pub fn weather_suppressed() -> [bool; 3] {
    [
        WEATHER_NO_RAIN.load(Ordering::Relaxed),
        WEATHER_NO_SNOW.load(Ordering::Relaxed),
        WEATHER_NO_SANDSTORM.load(Ordering::Relaxed),
    ]
}

/// Set one suppression flag (0 rain, 1 snow, 2 sandstorm).
pub fn set_weather_suppressed(kind: usize, value: bool) {
    let slot = match kind {
        0 => &WEATHER_NO_RAIN,
        1 => &WEATHER_NO_SNOW,
        _ => &WEATHER_NO_SANDSTORM,
    };
    slot.store(value, Ordering::Relaxed);
}

/// The weather type after suppression: a suppressed type becomes clear (0).
pub fn weather_filtered_type(weather_type: i32) -> i32 {
    let [no_rain, no_snow, no_sandstorm] = weather_suppressed();
    match weather_type {
        1 if no_rain => 0,
        2 if no_snow => 0,
        3 if no_sandstorm => 0,
        other => other,
    }
}

/// Suppress the client's floating experience text.
static HIDE_EXP_TEXT: AtomicBool = AtomicBool::new(false);

/// See [`HIDE_EXP_TEXT`].
pub fn hide_exp_text() -> bool {
    HIDE_EXP_TEXT.load(Ordering::Relaxed)
}

/// See [`HIDE_EXP_TEXT`].
pub fn set_hide_exp_text(value: bool) {
    HIDE_EXP_TEXT.store(value, Ordering::Relaxed);
}

/// Screenshots re-encode as PNG (true) or JPEG (false, the default).
static SCREENSHOT_PNG: AtomicBool = AtomicBool::new(false);

/// See [`SCREENSHOT_PNG`].
pub fn screenshot_png() -> bool {
    SCREENSHOT_PNG.load(Ordering::Relaxed)
}

/// See [`SCREENSHOT_PNG`].
pub fn set_screenshot_png(value: bool) {
    SCREENSHOT_PNG.store(value, Ordering::Relaxed);
}

/// The camera-sight nameplate rule (the headline filter), on by default.
static MODERN_NAMEPLATE_DISTANCE: AtomicBool = AtomicBool::new(true);
/// Hide out-of-combat critter plates, on by default.
static HIDE_CRITTER_NAMEPLATE: AtomicBool = AtomicBool::new(true);
/// Only the current target keeps a plate while one exists.
static PRIORITIZE_TARGET_NAMEPLATE: AtomicBool = AtomicBool::new(false);
/// Only raid-marked units keep plates while any marked plate exists.
static PRIORITIZE_MARKED_NAMEPLATE: AtomicBool = AtomicBool::new(false);
/// Hide full-health, out-of-combat, non-attackable-player plates.
static NAMEPLATE_COMBAT_FILTER: AtomicBool = AtomicBool::new(false);
/// Force-show in-combat plates within eight yards of the player.
static SHOW_IN_COMBAT_NAMEPLATES_NEAR_PLAYER: AtomicBool = AtomicBool::new(false);

/// See [`MODERN_NAMEPLATE_DISTANCE`].
pub fn modern_nameplate_distance() -> bool {
    MODERN_NAMEPLATE_DISTANCE.load(Ordering::Relaxed)
}

/// See [`MODERN_NAMEPLATE_DISTANCE`].
pub fn set_modern_nameplate_distance(value: bool) {
    MODERN_NAMEPLATE_DISTANCE.store(value, Ordering::Relaxed);
}

/// See [`HIDE_CRITTER_NAMEPLATE`].
pub fn hide_critter_nameplate() -> bool {
    HIDE_CRITTER_NAMEPLATE.load(Ordering::Relaxed)
}

/// See [`HIDE_CRITTER_NAMEPLATE`].
pub fn set_hide_critter_nameplate(value: bool) {
    HIDE_CRITTER_NAMEPLATE.store(value, Ordering::Relaxed);
}

/// See [`PRIORITIZE_TARGET_NAMEPLATE`].
pub fn prioritize_target_nameplate() -> bool {
    PRIORITIZE_TARGET_NAMEPLATE.load(Ordering::Relaxed)
}

/// See [`PRIORITIZE_TARGET_NAMEPLATE`].
pub fn set_prioritize_target_nameplate(value: bool) {
    PRIORITIZE_TARGET_NAMEPLATE.store(value, Ordering::Relaxed);
}

/// See [`PRIORITIZE_MARKED_NAMEPLATE`].
pub fn prioritize_marked_nameplate() -> bool {
    PRIORITIZE_MARKED_NAMEPLATE.load(Ordering::Relaxed)
}

/// See [`PRIORITIZE_MARKED_NAMEPLATE`].
pub fn set_prioritize_marked_nameplate(value: bool) {
    PRIORITIZE_MARKED_NAMEPLATE.store(value, Ordering::Relaxed);
}

/// See [`NAMEPLATE_COMBAT_FILTER`].
pub fn nameplate_combat_filter() -> bool {
    NAMEPLATE_COMBAT_FILTER.load(Ordering::Relaxed)
}

/// See [`NAMEPLATE_COMBAT_FILTER`].
pub fn set_nameplate_combat_filter(value: bool) {
    NAMEPLATE_COMBAT_FILTER.store(value, Ordering::Relaxed);
}

/// See [`SHOW_IN_COMBAT_NAMEPLATES_NEAR_PLAYER`].
pub fn show_in_combat_nameplates_near_player() -> bool {
    SHOW_IN_COMBAT_NAMEPLATES_NEAR_PLAYER.load(Ordering::Relaxed)
}

/// See [`SHOW_IN_COMBAT_NAMEPLATES_NEAR_PLAYER`].
pub fn set_show_in_combat_nameplates_near_player(value: bool) {
    SHOW_IN_COMBAT_NAMEPLATES_NEAR_PLAYER.store(value, Ordering::Relaxed);
}

/// The targeting cone divisor (2.0 matches the camera frustum), f32 bits.
static TARGETING_RANGE_CONE: AtomicU32 = AtomicU32::new(f32::to_bits(2.2));
/// The far targeting range in yards, f32 bits.
static TARGETING_FAR_RANGE: AtomicU32 = AtomicU32::new(f32::to_bits(41.0));
/// While in combat, restrict targeting to units already fighting.
static TARGETING_IN_COMBAT_FILTER: AtomicBool = AtomicBool::new(true);

/// See [`TARGETING_RANGE_CONE`].
pub fn targeting_range_cone() -> f32 {
    f32::from_bits(TARGETING_RANGE_CONE.load(Ordering::Relaxed))
}

/// Set the targeting cone; out-of-range values leave it unchanged.
pub fn set_targeting_range_cone(value: f64) {
    if value > 1.99 && value < f64::from(f32::MAX) {
        // the command narrows its ranged-checked double to f32 by value, as
        // the reference does; the bound check keeps it in f32 range
        #[allow(clippy::cast_possible_truncation)]
        let narrowed = value as f32;
        TARGETING_RANGE_CONE.store(narrowed.to_bits(), Ordering::Relaxed);
    }
}

/// See [`TARGETING_FAR_RANGE`].
pub fn targeting_far_range() -> f32 {
    f32::from_bits(TARGETING_FAR_RANGE.load(Ordering::Relaxed))
}

/// Set the far range; values outside (25, 61) leave it unchanged.
pub fn set_targeting_far_range(value: f64) {
    if value > 25.0 && value < 61.0 {
        // the command narrows its range-checked double to f32 by value, as the
        // reference does; the bound check keeps it in f32 range
        #[allow(clippy::cast_possible_truncation)]
        let narrowed = value as f32;
        TARGETING_FAR_RANGE.store(narrowed.to_bits(), Ordering::Relaxed);
    }
}

/// See [`TARGETING_IN_COMBAT_FILTER`].
pub fn targeting_in_combat_filter() -> bool {
    TARGETING_IN_COMBAT_FILTER.load(Ordering::Relaxed)
}

/// See [`TARGETING_IN_COMBAT_FILTER`].
pub fn set_targeting_in_combat_filter(value: bool) {
    TARGETING_IN_COMBAT_FILTER.store(value, Ordering::Relaxed);
}

/// The camera shoulder offset in yards, f32 bits.
static CAMERA_HORIZONTAL: AtomicU32 = AtomicU32::new(0);
/// The camera height offset in yards, f32 bits.
static CAMERA_VERTICAL: AtomicU32 = AtomicU32::new(0);
/// The camera pitch addend in radians, f32 bits.
static CAMERA_PITCH: AtomicU32 = AtomicU32::new(0);
/// Rebuild the view basis toward the current friendly target.
static CAMERA_FOLLOW_TARGET: AtomicBool = AtomicBool::new(false);
/// The client's eased camera-distance smoothing, on by default.
static CAMERA_ORGANIC_SMOOTH: AtomicBool = AtomicBool::new(true);
/// Pin the camera eye at the subject's collision-box height.
static CAMERA_PIN_HEIGHT: AtomicBool = AtomicBool::new(false);

/// See [`CAMERA_HORIZONTAL`].
pub fn camera_horizontal() -> f32 {
    f32::from_bits(CAMERA_HORIZONTAL.load(Ordering::Relaxed))
}

/// Set the shoulder offset, clamped to `[-4, 4]`.
pub fn set_camera_horizontal(value: f32) {
    let clamped = value.clamp(-4.0, 4.0);
    CAMERA_HORIZONTAL.store(clamped.to_bits(), Ordering::Relaxed);
}

/// See [`CAMERA_VERTICAL`].
pub fn camera_vertical() -> f32 {
    f32::from_bits(CAMERA_VERTICAL.load(Ordering::Relaxed))
}

/// Set the height offset, clamped to `[lo, 4]` (the two commands differ in `lo`).
pub fn set_camera_vertical(value: f32, lo: f32) {
    let clamped = value.clamp(lo, 4.0);
    CAMERA_VERTICAL.store(clamped.to_bits(), Ordering::Relaxed);
}

/// See [`CAMERA_PITCH`].
pub fn camera_pitch() -> f32 {
    f32::from_bits(CAMERA_PITCH.load(Ordering::Relaxed))
}

/// Set the pitch addend, clamped to `[0, 0.3]`.
pub fn set_camera_pitch(value: f32) {
    let clamped = value.clamp(0.0, 0.3);
    CAMERA_PITCH.store(clamped.to_bits(), Ordering::Relaxed);
}

/// See [`CAMERA_FOLLOW_TARGET`].
pub fn camera_follow_target() -> bool {
    CAMERA_FOLLOW_TARGET.load(Ordering::Relaxed)
}

/// See [`CAMERA_FOLLOW_TARGET`].
pub fn set_camera_follow_target(value: bool) {
    CAMERA_FOLLOW_TARGET.store(value, Ordering::Relaxed);
}

/// See [`CAMERA_ORGANIC_SMOOTH`].
pub fn camera_organic_smooth() -> bool {
    CAMERA_ORGANIC_SMOOTH.load(Ordering::Relaxed)
}

/// See [`CAMERA_ORGANIC_SMOOTH`].
pub fn set_camera_organic_smooth(value: bool) {
    CAMERA_ORGANIC_SMOOTH.store(value, Ordering::Relaxed);
}

/// See [`CAMERA_PIN_HEIGHT`].
pub fn camera_pin_height() -> bool {
    CAMERA_PIN_HEIGHT.load(Ordering::Relaxed)
}

/// See [`CAMERA_PIN_HEIGHT`].
pub fn set_camera_pin_height(value: bool) {
    CAMERA_PIN_HEIGHT.store(value, Ordering::Relaxed);
}

/// The `behind` test's planar angle threshold in radians, f32 bits.
static BEHIND_THRESHOLD: AtomicU32 = AtomicU32::new(f32::to_bits(core::f32::consts::FRAC_PI_2));

/// See [`BEHIND_THRESHOLD`].
pub fn behind_threshold() -> f32 {
    f32::from_bits(BEHIND_THRESHOLD.load(Ordering::Relaxed))
}

/// Set the behind threshold, clamped to `[0, pi]` as the command always has.
pub fn set_behind_threshold(radians: f64) {
    let clamped = radians.clamp(0.0, core::f64::consts::PI);
    // the command narrows its clamped double to f32 by value, as the reference
    // does; the clamp bounds it well inside f32 range
    #[allow(clippy::cast_possible_truncation)]
    let narrowed = clamped as f32;
    BEHIND_THRESHOLD.store(narrowed.to_bits(), Ordering::Relaxed);
}
