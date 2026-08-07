//! Nameplate filters: veto plates at creation, cull them per frame.
//!
//! Two wrapper hooks drive this. The creation entry refuses a plate for a
//! unit the filters exclude; the per-frame render entry walks the live plate
//! list and removes plates whose units stopped qualifying, so the decision
//! tracks sight and combat state frame by frame. The filter order and every
//! quirk mirror the command set this stands in for — including that the
//! marked-plate prescan runs only while the marked-priority filter is on, so
//! the mark flag is deliberately stale otherwise.
//!
//! The distance rule is the headline feature: instead of the client's short
//! plate range, a plate exists wherever the camera can actually see the unit,
//! and inside ten yards even through a wall. The camera-sight test is served
//! by the cached trace in the sight module, which is what keeps this loop's
//! steady-state trace rate low.
//!
//! No diff table is possible (the effect is which plates exist on screen);
//! verification is the armed veto/remove counters and in-game observation.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Head of the client's intrusive nameplate list.
const NAMEPLATE_LIST: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0084_d92c;

/// Remove a unit's nameplate — `thiscall(ecx = unit)`.
const REMOVE_NAMEPLATE_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0020_8a10;

/// Plates are kept through walls inside this camera distance.
const SEE_THROUGH_WALL_DISTANCE: f32 = 10.0;

/// The near-player force-show radius of the in-combat filter.
const NEAR_PLAYER_DISTANCE: f32 = 8.0;

/// Whether any live plate's unit carries a raid mark.
///
/// Refreshed once per frame while the marked-priority filter is on, stale
/// otherwise — the original's exact behaviour, preserved.
static HAS_MARKED_PLATE: AtomicBool = AtomicBool::new(false);

/// Units evaluated in the per-frame walk (armed runs only).
static WALKED: AtomicU32 = AtomicU32::new(0);
/// See [`WALKED`].
static VETOED: AtomicU32 = AtomicU32::new(0);
/// See [`WALKED`].
static REMOVED: AtomicU32 = AtomicU32::new(0);

/// Per-evaluation state hoisted out of the filter chain.
///
/// The original re-resolves the target and player tokens inside every check;
/// neither can change within a frame, so they are read once per walk (and
/// once per creation veto).
struct FrameContext {
    target_guid: u64,
    player: Option<super::super::objmgr::UnitRef>,
}

fn frame_context() -> FrameContext {
    FrameContext {
        target_guid: super::super::objmgr::guid_of_token(c"target"),
        player: super::super::objmgr::player(),
    }
}

/// The filter chain: 1 keep, 0 drop, -1 for a shape not modelled.
fn should_have_nameplate(unit: super::super::objmgr::UnitRef, ctx: &FrameContext) -> i32 {
    let guid = unit.guid();
    let prioritize_target = super::settings::prioritize_target_nameplate();
    let prioritize_marked = super::settings::prioritize_marked_nameplate();
    if (prioritize_marked || prioritize_target)
        && (ctx.target_guid > 0 || HAS_MARKED_PLATE.load(Ordering::Relaxed))
    {
        // The combination table is the original's, fall-throughs included: a
        // target-only priority with no target, and a marked-only priority
        // with no marked plate, both fall through to the normal filters.
        if prioritize_target && prioritize_marked {
            let keep = guid == ctx.target_guid || super::super::objmgr::target_mark_of(guid) > 0;
            return i32::from(keep);
        }
        if prioritize_target && ctx.target_guid > 0 {
            return i32::from(guid == ctx.target_guid);
        }
        if prioritize_marked && HAS_MARKED_PLATE.load(Ordering::Relaxed) {
            return i32::from(super::super::objmgr::target_mark_of(guid) > 0);
        }
    }
    if super::settings::hide_critter_nameplate()
        && unit.is_unit_or_player()
        && unit.creature_type() == super::super::objmgr::CREATURE_TYPE_CRITTER
        && !unit.in_combat()
    {
        return 0;
    }
    if super::settings::nameplate_combat_filter() {
        // A missing health record reads as the original's -1 sentinels, whose
        // compare then answers "full": preserved by mapping both to -1.
        let current = unit.current_hp().map_or(-1.0, f64::from);
        let maximum = unit.max_hp().map_or(-1.0, f64::from);
        let full_hp = current >= maximum;
        let in_combat = unit.in_combat()
            || (unit.object_type() == super::super::objmgr::TYPE_PLAYER
                && ctx.player.is_some_and(|player| player.can_attack(unit)));
        if full_hp && !in_combat {
            return 0;
        }
    }
    if super::settings::show_in_combat_nameplates_near_player()
        && unit.in_combat()
        && let Some(player) = ctx.player
    {
        let distance =
            super::distance::between_units(unit, player, crate::math::reach::Meter::Ranged);
        if (0.0..NEAR_PLAYER_DISTANCE).contains(&distance) {
            return 1;
        }
    }
    let in_sight = super::insight::camera_in_sight(unit) > 0;
    let cam = super::camera::translated_position();
    let pos = unit.position();
    let dx = cam[0] - pos[0];
    let dy = cam[1] - pos[1];
    let dz = cam[2] - pos[2];
    let camera_distance = (dx * dx + dy * dy + dz * dz).sqrt();
    i32::from(camera_distance <= SEE_THROUGH_WALL_DISTANCE || in_sight)
}

/// One pass over the live plate list looking for any raid-marked unit.
fn refresh_mark_status() {
    HAS_MARKED_PLATE.store(false, Ordering::Relaxed);
    // SAFETY: `NAMEPLATE_LIST` is a fixed host global at the verified image
    // base, holding the list head (zero when no plates exist).
    let mut item = unsafe { *(NAMEPLATE_LIST as *const usize) };
    while item != 0 && item & 1 == 0 {
        // SAFETY: `item` passed the liveness heuristic; `+0x4e0` is the next
        // link of the intrusive list.
        let next = unsafe { *((item + 0x4e0) as *const usize) };
        // SAFETY: `+0x4e8` is the plate's unit GUID (unaligned by contract).
        let guid = unsafe { ((item + 0x4e8) as *const u64).read_unaligned() };
        if super::super::objmgr::target_mark_of(guid) > 0 {
            HAS_MARKED_PLATE.store(true, Ordering::Relaxed);
            return;
        }
        item = next;
    }
}

/// The per-frame cull, run by the render wrapper before the frame draws.
pub fn cull_before_render() {
    if !super::settings::modern_nameplate_distance() {
        return;
    }
    if super::settings::prioritize_marked_nameplate() {
        refresh_mark_status();
    }
    let ctx = frame_context();
    // SAFETY: `NAMEPLATE_LIST` is a fixed host global at the verified image
    // base, holding the list head (zero when no plates exist).
    let mut item = unsafe { *(NAMEPLATE_LIST as *const usize) };
    while item != 0 && item & 1 == 0 {
        // The next link is saved before the filter runs: dropping a plate
        // frees the current node.
        // SAFETY: `item` passed the liveness heuristic; `+0x4e0` is the next
        // link of the intrusive list.
        let next = unsafe { *((item + 0x4e0) as *const usize) };
        // SAFETY: `+0x4e8` is the plate's unit GUID (unaligned by contract).
        let guid = unsafe { ((item + 0x4e8) as *const u64).read_unaligned() };
        if let Some(unit) = super::super::objmgr::object_by_guid(guid)
            && unit.is_unit_or_player()
        {
            super::bump(&WALKED);
            if should_have_nameplate(unit, &ctx) == 0 {
                super::bump(&REMOVED);
                // SAFETY: a fixed `.text` entry in the live host image
                // (base verified at load); the transmuted signature
                // matches the declared prototype (`__thiscall(ecx =
                // unit)`, no return).
                let remove: extern "thiscall" fn(usize) =
                    unsafe { core::mem::transmute(REMOVE_NAMEPLATE_VA) };
                remove(unit.raw());
            }
        }
        item = next;
    }
}

/// Whether the creation wrapper should refuse this plate.
///
/// The odd-pointer veto is the original's: a half-formed unit gets no plate
/// rather than a crash. A null unit runs the displaced code unchanged.
pub fn should_veto_add(unit_raw: usize) -> bool {
    if !super::settings::modern_nameplate_distance() || unit_raw == 0 {
        return false;
    }
    if unit_raw & 1 != 0 {
        return true;
    }
    let Some(unit) = super::super::objmgr::UnitRef::try_from_raw(unit_raw as u32) else {
        return false;
    };
    if unit.is_unit_or_player() && should_have_nameplate(unit, &frame_context()) == 0 {
        super::bump(&VETOED);
        return true;
    }
    false
}

/// Counter snapshot for the dispatcher's cumulative line.
pub fn counters() -> [u32; 3] {
    [
        WALKED.load(Ordering::Relaxed),
        VETOED.load(Ordering::Relaxed),
        REMOVED.load(Ordering::Relaxed),
    ]
}
