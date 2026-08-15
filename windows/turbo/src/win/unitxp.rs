//! The `UnitXP` script entry (`0x517350`), served as a chained dispatcher.
//!
//! Stock 1.12 registers a near-useless `UnitXP` script function, and mods
//! borrow its entry as an RPC channel into native code: each detours the
//! address, answers the first-argument commands it owns, and runs the
//! displaced code for everything else. This dispatcher takes over the command
//! set of a discontinued mod so scripts written against it keep working with
//! that mod removed — same command names, argument shapes and return values,
//! documented per feature module.
//!
//! Chaining is the contract. Unknown commands (and calls with fewer than two
//! arguments, which is the command set's own gate) delegate to the displaced
//! code, so any other mod on this entry keeps working whichever order the
//! hooks landed in; and when another mod detours the entry after this one,
//! the overwrite policy leaves it in place — its trampoline still reaches
//! this dispatcher, while re-asserting would orphan the newcomer's chain.
//!
//! Argument reads decode the Lua stack directly (no in-place coercion), so
//! the per-call cost for foreign traffic is a length-bucketed byte match that
//! exits in a couple of compares. No differential table is possible on a live
//! Lua stack; verification is the armed counters plus in-game observation.

pub mod camera;
mod distance;
pub mod fpscap;
mod insight;
pub mod nameplate;
mod notify;
pub mod screenshot;
pub mod settings;
mod targeting;
pub mod timer;
mod trace;
pub mod worldtext;

use super::tally::Counter;

/// RVA of the stock `UnitXP` script function inside the host image.
const UNIT_XP_RVA: usize = 0x0011_7350;

/// The client's locale code global.
const GAME_LOCALE: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0080_e080;

/// Identity served for `version additionalInformation`.
///
/// Deliberately NOT the discontinued mod's own string: a script matching that
/// exact identity is asking for the original, and features this dispatcher
/// dropped would misattribute to it. Interoperability notes are what the
/// string is for, so it names this mod and its build.
const IDENTITY_PREFIX: &str = "wow_turbo ";

/// Commands handled here, delegated calls, and calls below the argument gate.
static HANDLED: Counter = Counter::zero();
/// See [`HANDLED`].
static DELEGATED: Counter = Counter::zero();

/// The dispatcher body behind the generated thunk; `fastcall(ecx = L)`.
pub fn unit_xp(l: i32) -> i32 {
    // SAFETY: `l` is the live `lua_State` the host dispatched this call with.
    let lua = unsafe { super::lua::LuaState::from_raw(l) };
    if let Some(results) = dispatch(&lua) {
        super::tally::bump(&HANDLED);
        return results;
    }
    super::tally::bump(&DELEGATED);
    super::symbols::originals::script_unit_xp__517350()(l)
}

/// Answer a command this dispatcher owns, or `None` to run the displaced code.
fn dispatch(lua: &super::lua::LuaState) -> Option<i32> {
    if lua.argc() < 2 {
        // The command set's own gate: every command takes a second argument.
        return None;
    }
    match lua.str_arg(1)? {
        b"nop" => {
            lua.push_boolean(true);
            Some(1)
        }
        b"inSight" if lua.argc() >= 3 => {
            let verdict = match (lua.str_arg(2), lua.str_arg(3)) {
                (Some(unit0), Some(unit1)) => insight::command_in_sight(unit0, unit1),
                _ => -1,
            };
            Some(push_verdict(lua, verdict))
        }
        b"behind" if lua.argc() >= 3 => {
            let verdict = match (lua.str_arg(2), lua.str_arg(3)) {
                (Some(unit0), Some(unit1)) => insight::command_behind(unit0, unit1),
                _ => -1,
            };
            Some(push_verdict(lua, verdict))
        }
        b"distanceBetween" if lua.argc() >= 3 => {
            let measured = match (lua.str_arg(2), lua.str_arg(3)) {
                (Some(unit0), Some(unit1)) => distance::command(unit0, unit1, lua.str_arg(4)),
                _ => -1.0,
            };
            if measured >= 0.0 {
                lua.push_number(f64::from(measured));
            } else {
                lua.push_nil();
            }
            Some(1)
        }
        b"target" => Some(target_command(lua)),
        b"modernNameplateDistance" => Some(toggle(
            lua,
            settings::modern_nameplate_distance,
            settings::set_modern_nameplate_distance,
        )),
        b"hideCritterNameplate" => Some(toggle(
            lua,
            settings::hide_critter_nameplate,
            settings::set_hide_critter_nameplate,
        )),
        b"prioritizeTargetNameplate" => Some(toggle(
            lua,
            settings::prioritize_target_nameplate,
            settings::set_prioritize_target_nameplate,
        )),
        b"prioritizeMarkedNameplate" => Some(toggle(
            lua,
            settings::prioritize_marked_nameplate,
            settings::set_prioritize_marked_nameplate,
        )),
        b"nameplateCombatFilter" => Some(toggle(
            lua,
            settings::nameplate_combat_filter,
            settings::set_nameplate_combat_filter,
        )),
        b"showInCombatNameplatesNearPlayer" => Some(toggle(
            lua,
            settings::show_in_combat_nameplates_near_player,
            settings::set_show_in_combat_nameplates_near_player,
        )),
        b"cameraHeight" => Some(camera_offset(lua, settings::camera_vertical, |v| {
            settings::set_camera_vertical(v, 0.0);
        })),
        b"cameraVerticalDisplacement" => Some(camera_offset(lua, settings::camera_vertical, |v| {
            settings::set_camera_vertical(v, -1.0);
        })),
        b"cameraHorizontalDisplacement" => Some(camera_offset(
            lua,
            settings::camera_horizontal,
            settings::set_camera_horizontal,
        )),
        b"cameraPitch" => Some(camera_offset(
            lua,
            settings::camera_pitch,
            settings::set_camera_pitch,
        )),
        b"cameraFollowTarget" => Some(toggle(
            lua,
            settings::camera_follow_target,
            settings::set_camera_follow_target,
        )),
        b"cameraOrganicSmooth" => Some(toggle(
            lua,
            settings::camera_organic_smooth,
            settings::set_camera_organic_smooth,
        )),
        b"cameraPinHeight" => Some(toggle(
            lua,
            settings::camera_pin_height,
            settings::set_camera_pin_height,
        )),
        b"weatherAlwaysClear" => Some(weather_command(lua)),
        b"hideEXPtext" => Some(toggle(
            lua,
            settings::hide_exp_text,
            settings::set_hide_exp_text,
        )),
        b"FPScap" => {
            if let Some(fps) = lua.number_arg(2) {
                fpscap::set_target(fps);
            }
            lua.push_number(fpscap::target());
            Some(1)
        }
        b"backgroundFPScap" => {
            if let Some(fps) = lua.number_arg(2) {
                fpscap::set_background(fps);
            }
            lua.push_number(fpscap::background());
            Some(1)
        }
        b"timer" => Some(timer_command(lua)),
        b"notify" => Some(notify_command(lua)),
        b"screenshot" => {
            let png = lua.str_arg(2) == Some(b"perfect");
            settings::set_screenshot_png(png);
            // The answer is the file-type code: 0 the compressed default,
            // 1 the lossless form.
            lua.push_number(if png { 1.0 } else { 0.0 });
            Some(1)
        }
        b"addCombatText" if lua.argc() >= 6 => {
            // The reference order is style, text, then the three color
            // channels, and both strings are read before the numbers because
            // coercing a number slot to a string rewrites it in place.
            let style = lua.str_arg(2);
            let text = lua.coerced_str(3).unwrap_or_default();
            let color = [
                color_byte(lua.number_arg(4)),
                color_byte(lua.number_arg(5)),
                color_byte(lua.number_arg(6)),
            ];
            let shown = match style {
                Some(b"crit") => worldtext::add_crit_text(&text, color, 255),
                Some(b"downward") => worldtext::add_small_floating(
                    &text,
                    color,
                    255,
                    crate::worldtext::Direction::Down,
                ),
                Some(b"arc") => worldtext::add_small_floating(
                    &text,
                    color,
                    255,
                    crate::worldtext::Direction::Arc,
                ),
                _ => worldtext::add_small_floating(
                    &text,
                    color,
                    255,
                    crate::worldtext::Direction::Up,
                ),
            };
            lua.push_boolean(shown);
            Some(1)
        }
        b"combatTextSP3" => Some(combat_text_command(lua)),
        b"behindThreshold" => {
            if lua.str_arg(2) == Some(b"set")
                && lua.argc() >= 3
                && let Some(radians) = lua.number_arg(3)
            {
                settings::set_behind_threshold(radians);
            }
            lua.push_number(f64::from(settings::behind_threshold()));
            Some(1)
        }
        b"version" => version(lua),
        b"gameLocale" => {
            // SAFETY: `GAME_LOCALE` is a fixed host global at the verified
            // image base.
            let locale = unsafe { *(GAME_LOCALE as *const i32) };
            lua.push_number(f64::from(locale));
            Some(1)
        }
        b"onEvent" => Some(on_event(lua)),
        _ => None,
    }
}

/// The `target` subcommands: actions answer found-a-target, unknown answers nil.
fn target_command(lua: &super::lua::LuaState) -> i32 {
    let found = match lua.str_arg(2) {
        Some(b"nearestEnemy") => targeting::nearest_enemy(f32::MAX),
        Some(b"nextEnemyConsideringDistance") => targeting::considering_distance(true),
        Some(b"previousEnemyConsideringDistance") => targeting::considering_distance(false),
        Some(b"nextEnemyInCycle") => targeting::enemy_in_cycle(true),
        Some(b"previousEnemyInCycle") => targeting::enemy_in_cycle(false),
        Some(sub @ (b"nextMarkedEnemyInCycle" | b"previousMarkedEnemyInCycle")) => {
            let forward = sub == b"nextMarkedEnemyInCycle";
            // The priority argument coerces like the original's read did, so
            // a numeric `81234` still parses as its digits.
            let priority = if lua.argc() >= 3 {
                lua.coerced_str(3)
            } else {
                None
            };
            targeting::marked_enemy_in_cycle(
                forward,
                priority.as_deref().map_or(b"", str::as_bytes),
            )
        }
        Some(b"mostHP") => targeting::most_hp(settings::targeting_far_range()),
        Some(b"worldBoss") => targeting::world_boss(f32::MAX),
        Some(b"rangeCone") => {
            if lua.argc() >= 3
                && let Some(value) = lua.number_arg(3)
            {
                settings::set_targeting_range_cone(value);
            }
            lua.push_number(f64::from(settings::targeting_range_cone()));
            return 1;
        }
        Some(b"farRange") => {
            if lua.argc() >= 3
                && let Some(value) = lua.number_arg(3)
            {
                settings::set_targeting_far_range(value);
            }
            lua.push_number(f64::from(settings::targeting_far_range()));
            return 1;
        }
        Some(b"disableInCombatFilter") => {
            settings::set_targeting_in_combat_filter(false);
            lua.push_boolean(settings::targeting_in_combat_filter());
            return 1;
        }
        Some(b"enableInCombatFilter") => {
            settings::set_targeting_in_combat_filter(true);
            lua.push_boolean(settings::targeting_in_combat_filter());
            return 1;
        }
        _ => {
            lua.push_nil();
            return 1;
        }
    };
    lua.push_boolean(found);
    1
}

/// The `weatherAlwaysClear` subcommands; a query answers three booleans.
fn weather_command(lua: &super::lua::LuaState) -> i32 {
    let set_all = |value: bool| {
        for kind in 0..3 {
            settings::set_weather_suppressed(kind, value);
        }
    };
    match lua.str_arg(2) {
        Some(b"enable") => {
            set_all(true);
            lua.push_boolean(true);
        }
        Some(b"disable") => {
            set_all(false);
            lua.push_boolean(false);
        }
        Some(b"enableRain") => {
            settings::set_weather_suppressed(0, false);
            lua.push_boolean(true);
        }
        Some(b"disableRain") => {
            settings::set_weather_suppressed(0, true);
            lua.push_boolean(false);
        }
        Some(b"enableSnow") => {
            settings::set_weather_suppressed(1, false);
            lua.push_boolean(true);
        }
        Some(b"disableSnow") => {
            settings::set_weather_suppressed(1, true);
            lua.push_boolean(false);
        }
        Some(b"enableSandstorm") => {
            settings::set_weather_suppressed(2, false);
            lua.push_boolean(true);
        }
        Some(b"disableSandstorm") => {
            settings::set_weather_suppressed(2, true);
            lua.push_boolean(false);
        }
        _ => {
            let [no_rain, no_snow, no_sandstorm] = settings::weather_suppressed();
            lua.push_boolean(!no_rain);
            lua.push_boolean(!no_snow);
            lua.push_boolean(!no_sandstorm);
            return 3;
        }
    }
    1
}

/// The `timer` subcommands: arm answers the id, disarm whether it was live.
fn timer_command(lua: &super::lua::LuaState) -> i32 {
    match lua.str_arg(2) {
        Some(b"arm") if lua.argc() >= 5 => {
            let (Some(delay), Some(period)) = (lua.number_arg(3), lua.number_arg(4)) else {
                lua.push_nil();
                return 1;
            };
            let Some(script) = lua.coerced_str(5) else {
                lua.push_nil();
                return 1;
            };
            // the command truncates its millisecond doubles to integers, as
            // the reference does
            let id = timer::arm(delay as u64, period as u64, &script);
            lua.push_number(f64::from(id));
            1
        }
        Some(b"disarm") if lua.argc() >= 3 => {
            let Some(id) = lua.number_arg(3) else {
                lua.push_nil();
                return 1;
            };
            // the command truncates its id double to the integer id it handed
            // out, as the reference does
            let disarmed = timer::disarm(id as u32);
            lua.push_boolean(disarmed);
            1
        }
        Some(b"size") => {
            // the live-timer count is far below the 52-bit exact range
            lua.push_number(timer::live_count() as f64);
            1
        }
        _ => {
            lua.push_nil();
            1
        }
    }
}

/// The `notify` subcommands.
///
/// The flash always reports true; sounds report whether they were issued.
fn notify_command(lua: &super::lua::LuaState) -> i32 {
    match lua.str_arg(2) {
        Some(b"taskbarIcon") => {
            notify::flash_taskbar_icon();
            lua.push_boolean(true);
        }
        Some(b"systemSound") if lua.argc() >= 3 => {
            let played = lua
                .coerced_str(3)
                .is_some_and(|name| notify::play_system_sound(name.as_bytes()));
            lua.push_boolean(played);
        }
        _ => lua.push_nil(),
    }
    1
}

/// The `combatTextSP3` subcommands: settings, and the scene state second.
///
/// Settings store and echo as the original's do; the second return value
/// reports whether the overlay can render, and the debug query serves the
/// reason when it cannot, so scripts checking it fall back to the stock
/// combat text on the same ladder they always handled.
fn combat_text_command(lua: &super::lua::LuaState) -> i32 {
    match lua.str_arg(2) {
        Some(b"enable") => {
            worldtext::set_use_combat_text(true);
            lua.push_boolean(worldtext::use_combat_text());
        }
        Some(b"disable") => {
            worldtext::set_use_combat_text(false);
            worldtext::clear();
            lua.push_boolean(worldtext::use_combat_text());
        }
        Some(b"setFontSize") if lua.argc() >= 3 => {
            let Some(size) = lua.number_arg(3) else {
                lua.push_nil();
                return 1;
            };
            lua.push_number(f64::from(worldtext::set_font_size(size)));
        }
        Some(b"setNameplateHeight") if lua.argc() >= 3 => {
            let Some(height) = lua.number_arg(3) else {
                lua.push_nil();
                return 1;
            };
            lua.push_number(worldtext::set_nameplate_height(height));
        }
        Some(b"setFontName") if lua.argc() >= 3 => {
            let Some(name) = lua.coerced_str(3) else {
                lua.push_nil();
                return 1;
            };
            lua.push_str(&worldtext::set_font_name(name));
        }
        Some(b"debugText") => {
            lua.push_str(&worldtext::debug_text());
        }
        _ => {
            lua.push_nil();
            return 1;
        }
    }
    lua.push_boolean(worldtext::scene_enabled());
    2
}

/// A camera-offset command: `set` clamps and stores, the answer is current.
fn camera_offset(lua: &super::lua::LuaState, get: fn() -> f32, set: fn(f32)) -> i32 {
    if lua.str_arg(2) == Some(b"set")
        && let Some(value) = lua.number_arg(3)
    {
        // the command narrows its double argument to f32 by value before the
        // clamp, as the reference does
        set(value as f32);
    }
    lua.push_number(f64::from(get()));
    1
}

/// An enable/disable toggle command: mutate on the subcommand, answer current.
fn toggle(lua: &super::lua::LuaState, get: fn() -> bool, set: fn(bool)) -> i32 {
    match lua.str_arg(2) {
        Some(b"enable") => set(true),
        Some(b"disable") => set(false),
        _ => {}
    }
    lua.push_boolean(get());
    1
}

/// Push a sight verdict the way the command set always has: bool, nil on error.
fn push_verdict(lua: &super::lua::LuaState, verdict: i32) -> i32 {
    if verdict >= 0 {
        lua.push_boolean(verdict != 0);
    } else {
        lua.push_nil();
    }
    1
}

/// The `version` subcommands; unknown ones delegate, as the original does.
fn version(lua: &super::lua::LuaState) -> Option<i32> {
    match lua.str_arg(2)? {
        b"coffTimeDateStamp" => {
            lua.push_number(f64::from(build_epoch()));
            Some(1)
        }
        b"additionalInformation" => {
            let mut identity = String::from(IDENTITY_PREFIX);
            identity.push_str(wow_shared::identity::BUILD);
            lua.push_str(&identity);
            Some(1)
        }
        _ => None,
    }
}

/// The build's timestamp, served where scripts expect a PE stamp.
///
/// The linker writes a reproducibility hash into this image's header field,
/// which formats as garbage when a script renders it as a date — so the build
/// script bakes the real build time instead.
fn build_epoch() -> u32 {
    env!("WOW_TURBO_BUILD_EPOCH")
        .parse()
        .expect("build script emits a decimal epoch")
}

/// Script-side world/combat notifications; unknown events answer `nil`.
///
/// The combat pair is accepted and ignored, exactly as the command set always
/// has; the world pair drives the in-world gate frame-rate limiting and the
/// text overlay read.
fn on_event(lua: &super::lua::LuaState) -> i32 {
    let known = match lua.str_arg(2) {
        Some(b"PLAYER_ENTERING_WORLD") => {
            settings::set_in_world(true);
            true
        }
        Some(b"PLAYER_LEAVING_WORLD") => {
            settings::set_in_world(false);
            // The overlay's unload point: lines must not survive into the
            // next world with stale anchors and a possibly-rebuilt device.
            worldtext::clear();
            true
        }
        Some(b"PLAYER_REGEN_ENABLED" | b"PLAYER_REGEN_DISABLED") => true,
        _ => false,
    };
    if known {
        lua.push_boolean(true);
    } else {
        lua.push_nil();
    }
    1
}

/// Leave a later same-entry detour in place: its trampoline reaches us.
///
/// Re-asserting would restore the stock bytes displaced at our install time,
/// erasing the newcomer's patch and orphaning whatever chain it built.
fn chain_decide(owner_va: usize) -> bool {
    let owner = wow_hook::module_of(owner_va).map_or_else(
        || String::from("an unnamed module"),
        |(name, base)| format!("{name}+{:#x}", owner_va.wrapping_sub(base)),
    );
    log::info!(
        target: super::LOG_TARGET,
        "unitxp: the entry was re-hooked by {owner}; chaining underneath it",
    );
    false
}

/// Register the never-reassert overwrite policy for the periodic check.
pub fn arm_chain_policy(image_base: usize) {
    wow_hook::on_overwrite(image_base + UNIT_XP_RVA, chain_decide);
}

/// The command set's cumulative lines, on the gauge's 60-second cadence.
///
/// The dispatcher's own line first, then one per feature that has run: each
/// module owns its counters, so a label and the counter it names sit together
/// in the file that writes it, and a feature nobody used prints nothing.
pub fn emit_cumulative() {
    let handled = HANDLED.get();
    let delegated = DELEGATED.get();
    if handled | delegated != 0 {
        log::info!(
            target: super::tally::TARGET,
            "unitxp: {handled} handled, {delegated} delegated",
        );
    }
    insight::emit_cumulative();
    trace::emit_cumulative();
    nameplate::emit_cumulative();
    targeting::emit_cumulative();
    camera::emit_cumulative();
    worldtext::emit_cumulative();
}

/// A script color channel to a byte, the reference's saturating read.
///
/// Script colors run 0..1, so the channel scales by 255. A value that lands
/// outside a byte — a negative channel, or one past 1.0 — answers 255 rather
/// than clamping toward it, and an absent or non-numeric argument reads as the
/// 0 the host's number coercion would have handed back.
fn color_byte(value: Option<f64>) -> u8 {
    let scaled = value.unwrap_or(0.0) * 255.0;
    if (0.0..256.0).contains(&scaled) {
        // In a byte's range, and non-negative, by the test right above.
        scaled as u8
    } else {
        255
    }
}
