//! Targeting sweeps over the object manager, behind the `target` command.
//!
//! Every action is a synchronous walk of the live object list applying the
//! command set's admission filter — attackable, alive, in the viewing cone,
//! critters only in combat, and (for most actions) in line of sight through
//! the cached sight test — then a selection rule: nearest, most health,
//! world bosses, flat GUID-ordered cycles, raid-mark priority cycles, or the
//! melee/charge/far bucket cycle. Filter order, the per-action distance
//! meters, and the bucket comparators (including their historical `<=` vs
//! `<` asymmetry between the in-combat branch and the other) are preserved
//! exactly; the player object, its combat state and its target resolve once
//! per sweep instead of once per candidate, which cannot change mid-walk.
//!
//! Candidates collect into a fixed buffer: a post-filter sweep past its size
//! does not exist in practice (the far-range cap is 61 yards), and overflow
//! drops extras under a counter rather than allocating.

use crate::win::tally::{self, Counter};

/// Candidate cap per sweep; overflow is counted, not grown.
const CAP: usize = 256;

/// Melee, charge and far bucket edges of the distance-considering cycle.
const MELEE_RANGE: f32 = 8.0;
/// See [`MELEE_RANGE`].
const CHARGE_RANGE: f32 = 25.0;
/// Melee bucket admission for a candidate out of sight but already in reach.
const MELEE_ADMIT_DISTANCE: f32 = 5.0;
/// Bucket caps: nearest three charge candidates, nearest five far ones.
const CHARGE_CAP: usize = 3;
/// See [`CHARGE_CAP`].
const FAR_CAP: usize = 5;

/// Sweeps run, candidates admitted, and buffer overflows (armed runs only).
static SWEEPS: Counter = Counter::zero();
/// See [`SWEEPS`].
static ADMITTED: Counter = Counter::zero();
/// See [`SWEEPS`].
static OVERFLOW: Counter = Counter::zero();

/// One admitted candidate.
#[derive(Clone, Copy, Default)]
struct MobEntry {
    guid: u64,
    distance: f32,
    current_hp: f64,
    mark: u32,
}

/// A bounded candidate list on the sweep's stack frame.
struct Candidates {
    entries: [MobEntry; CAP],
    len: usize,
}

impl Candidates {
    const fn new() -> Self {
        Self {
            entries: [MobEntry {
                guid: 0,
                distance: 0.0,
                current_hp: 0.0,
                mark: 0,
            }; CAP],
            len: 0,
        }
    }

    fn push(&mut self, entry: MobEntry) {
        if self.len == CAP {
            tally::bump(&OVERFLOW);
            return;
        }
        self.entries[self.len] = entry;
        self.len += 1;
        tally::bump(&ADMITTED);
    }

    fn as_mut_slice(&mut self) -> &mut [MobEntry] {
        &mut self.entries[..self.len]
    }

    const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// Once-per-sweep state the original re-resolved per candidate.
struct SweepContext {
    player: Option<super::super::objmgr::UnitRef>,
    self_in_combat: bool,
    last_target: u64,
}

fn sweep_context() -> SweepContext {
    let player = super::super::objmgr::player();
    SweepContext {
        player,
        self_in_combat: player.is_some_and(super::super::objmgr::UnitRef::in_combat),
        last_target: player.map_or(0, super::super::objmgr::UnitRef::target_guid),
    }
}

/// The admission filter shared by every sweep (sans the per-action extras).
///
/// Attackable non-player-controlled unit or player, not dead, critter only
/// in combat, inside the targeting cone. A missing record fails the filter,
/// as the original's error sentinels did.
fn admit_common(unit: super::super::objmgr::UnitRef, ctx: &SweepContext) -> bool {
    let object_type = unit.object_type();
    let type_ok = (object_type == super::super::objmgr::TYPE_UNIT
        && unit.is_player_controlled() == Some(false))
        || object_type == super::super::objmgr::TYPE_PLAYER;
    type_ok
        && ctx.player.is_some_and(|player| player.can_attack(unit))
        && unit.is_dead() == Some(false)
        && (unit.in_combat() || unit.creature_type() != super::super::objmgr::CREATURE_TYPE_CRITTER)
        && super::insight::in_viewing_frustum(
            unit.position(),
            super::settings::targeting_range_cone(),
        )
}

/// The in-combat admission gate: in combat, only fighting units qualify.
fn combat_gate(unit: super::super::objmgr::UnitRef, ctx: &SweepContext) -> bool {
    !(super::settings::targeting_in_combat_filter()
        && unit.object_type() == super::super::objmgr::TYPE_UNIT
        && ctx.self_in_combat)
        || unit.in_combat()
}

/// Walk every unit or player in the manager, applying `admit`.
fn sweep(ctx: &SweepContext, mut admit: impl FnMut(super::super::objmgr::UnitRef, &SweepContext)) {
    tally::bump(&SWEEPS);
    for unit in super::super::objmgr::objects() {
        if unit.is_unit_or_player() {
            admit(unit, ctx);
        }
    }
}

/// Target the closest admissible enemy inside `limit`; true when one was.
pub fn nearest_enemy(limit: f32) -> bool {
    let ctx = sweep_context();
    let mut mobs = Candidates::new();
    sweep(&ctx, |unit, ctx| {
        let distance = distance_to(ctx, unit, crate::math::reach::Meter::Gaussian);
        if distance <= limit
            && admit_common(unit, ctx)
            && in_sight(ctx, unit)
            && combat_gate(unit, ctx)
        {
            mobs.push(MobEntry {
                guid: unit.guid(),
                distance,
                ..Default::default()
            });
        }
    });
    if mobs.is_empty() {
        return false;
    }
    let front = mobs
        .as_mut_slice()
        .iter()
        .min_by(|a, b| a.distance.total_cmp(&b.distance))
        .map_or(0, |m| m.guid);
    super::super::objmgr::target_by_guid(front);
    true
}

/// Target the admissible enemy with the most health inside `limit`.
pub fn most_hp(limit: f32) -> bool {
    let ctx = sweep_context();
    let mut mobs = Candidates::new();
    sweep(&ctx, |unit, ctx| {
        let distance = distance_to(ctx, unit, crate::math::reach::Meter::Gaussian);
        if distance <= limit
            && admit_common(unit, ctx)
            && in_sight(ctx, unit)
            && combat_gate(unit, ctx)
        {
            mobs.push(MobEntry {
                guid: unit.guid(),
                current_hp: unit.current_hp().map_or(-1.0, f64::from),
                ..Default::default()
            });
        }
    });
    if mobs.is_empty() {
        return false;
    }
    let front = mobs
        .as_mut_slice()
        .iter()
        .max_by(|a, b| a.current_hp.total_cmp(&b.current_hp))
        .map_or(0, |m| m.guid);
    super::super::objmgr::target_by_guid(front);
    true
}

/// Target a world boss, cycling among several; no sight requirement.
pub fn world_boss(limit: f32) -> bool {
    let ctx = sweep_context();
    let mut mobs = Candidates::new();
    sweep(&ctx, |unit, ctx| {
        let distance = distance_to(ctx, unit, crate::math::reach::Meter::Gaussian);
        if distance <= limit
            && admit_common(unit, ctx)
            && unit.classification() == Some(super::super::objmgr::CLASSIFICATION_WORLD_BOSS)
            && combat_gate(unit, ctx)
        {
            mobs.push(MobEntry {
                guid: unit.guid(),
                distance,
                ..Default::default()
            });
        }
    });
    if mobs.is_empty() {
        return false;
    }
    let last_is_boss = mobs
        .as_mut_slice()
        .iter()
        .any(|m| m.guid == ctx.last_target);
    if ctx.last_target == 0 || !last_is_boss {
        let front = mobs
            .as_mut_slice()
            .iter()
            .min_by(|a, b| a.distance.total_cmp(&b.distance))
            .map_or(0, |m| m.guid);
        super::super::objmgr::target_by_guid(front);
    } else {
        let choice = select_adjacent(ctx.last_target, mobs.as_mut_slice(), true);
        super::super::objmgr::target_by_guid(choice);
    }
    true
}

/// Cycle to the adjacent admissible enemy in flat GUID order.
///
/// With nothing targeted the cycle has no anchor and the action degrades to
/// the nearest-enemy pick inside the far range, as the original does.
pub fn enemy_in_cycle(forward: bool) -> bool {
    let ctx = sweep_context();
    if ctx.last_target == 0 {
        return nearest_enemy(super::settings::targeting_far_range());
    }
    let far = super::settings::targeting_far_range();
    let mut list = Candidates::new();
    sweep(&ctx, |unit, ctx| {
        let distance = distance_to(ctx, unit, crate::math::reach::Meter::Ranged);
        if distance <= far
            && admit_common(unit, ctx)
            && in_sight(ctx, unit)
            && combat_gate(unit, ctx)
        {
            list.push(MobEntry {
                guid: unit.guid(),
                distance,
                ..Default::default()
            });
        }
    });
    if list.is_empty() {
        return false;
    }
    let choice = select_adjacent(ctx.last_target, list.as_mut_slice(), forward);
    super::super::objmgr::target_by_guid(choice);
    true
}

/// Cycle among raid-marked enemies in the priority order given.
///
/// The priority string's first eight characters name marks 1-8 best-first;
/// empty (or nothing valid) falls back to 8 down to 1. Candidates whose mark
/// is not in the list are excluded. No distance limit and no sight test.
pub fn marked_enemy_in_cycle(forward: bool, priority: &[u8]) -> bool {
    let mut order = [0u32; 8];
    let mut count = 0usize;
    for &ch in priority.iter().take(8) {
        if (b'1'..=b'8').contains(&ch) {
            order[count] = u32::from(ch - b'0');
            count += 1;
        }
    }
    if count == 0 {
        for (i, slot) in order.iter_mut().enumerate() {
            *slot = 8 - u32::try_from(i).expect("mark index is 0..8");
        }
        count = 8;
    }
    let order = &order[..count];
    let ctx = sweep_context();
    let mut list = Candidates::new();
    sweep(&ctx, |unit, ctx| {
        let guid = unit.guid();
        let mark = super::super::objmgr::target_mark_of(guid);
        if mark > 0
            && admit_common(unit, ctx)
            && combat_gate(unit, ctx)
            && mark_position(mark.cast_unsigned(), order) != u32::MAX
        {
            list.push(MobEntry {
                guid,
                distance: distance_to(ctx, unit, crate::math::reach::Meter::Gaussian),
                mark: mark.cast_unsigned(),
                ..Default::default()
            });
        }
    });
    if list.is_empty() {
        return false;
    }
    let choice = select_adjacent_mark(ctx.last_target, list.as_mut_slice(), order, forward);
    super::super::objmgr::target_by_guid(choice);
    true
}

/// The bucket cycle: melee first, then the nearest chargeable, then far.
///
/// The melee bucket admits a candidate out of sight when the melee meter
/// already reaches it; the charge and far buckets require sight and keep
/// only their nearest three and five. Bucket edges keep the original's
/// branch asymmetry: `<=` bounds while the in-combat filter applies, `<`
/// otherwise.
pub fn considering_distance(forward: bool) -> bool {
    let ctx = sweep_context();
    if ctx.last_target == 0 {
        return nearest_enemy(super::settings::targeting_far_range());
    }
    let far = super::settings::targeting_far_range();
    let mut melee = Candidates::new();
    let mut charge = Candidates::new();
    let mut far_bucket = Candidates::new();
    sweep(&ctx, |unit, ctx| {
        let distance = distance_to(ctx, unit, crate::math::reach::Meter::Ranged);
        if !(distance <= far && admit_common(unit, ctx)) {
            return;
        }
        let sighted = in_sight(ctx, unit);
        let melee_distance = distance_to(ctx, unit, crate::math::reach::Meter::MeleeAutoAttack);
        let gated = super::settings::targeting_in_combat_filter()
            && unit.object_type() == super::super::objmgr::TYPE_UNIT
            && ctx.self_in_combat;
        if gated && !unit.in_combat() {
            return;
        }
        let entry = MobEntry {
            guid: unit.guid(),
            distance,
            ..Default::default()
        };
        let (melee_edge, charge_edge) = if gated {
            (distance <= MELEE_RANGE, distance <= CHARGE_RANGE)
        } else {
            (distance < MELEE_RANGE, distance < CHARGE_RANGE)
        };
        if melee_edge {
            if sighted || melee_distance < MELEE_ADMIT_DISTANCE {
                melee.push(entry);
            }
        } else if charge_edge && sighted {
            charge.push(entry);
        } else if distance < far && sighted {
            far_bucket.push(entry);
        }
    });
    for (bucket, cap) in [
        (&mut melee, usize::MAX),
        (&mut charge, CHARGE_CAP),
        (&mut far_bucket, FAR_CAP),
    ] {
        if bucket.is_empty() {
            continue;
        }
        let slice = bucket.as_mut_slice();
        let slice = if slice.len() > cap {
            slice.sort_unstable_by(|a, b| a.distance.total_cmp(&b.distance));
            &mut slice[..cap]
        } else {
            slice
        };
        let choice = select_adjacent(ctx.last_target, slice, forward);
        super::super::objmgr::target_by_guid(choice);
        return true;
    }
    false
}

/// Distance from the player to `unit`, -1 with no player (fails every bound).
fn distance_to(
    ctx: &SweepContext,
    unit: super::super::objmgr::UnitRef,
    meter: crate::math::reach::Meter,
) -> f32 {
    ctx.player.map_or(-1.0, |player| {
        super::distance::between_units(player, unit, meter)
    })
}

/// Whether the player has line of sight to `unit` (no player fails).
fn in_sight(ctx: &SweepContext, unit: super::super::objmgr::UnitRef) -> bool {
    ctx.player
        .is_some_and(|player| super::insight::unit_in_sight(player, unit) == 1)
}

/// A mark's position in the priority order, largest when absent.
fn mark_position(mark: u32, order: &[u32]) -> u32 {
    order.iter().position(|&m| m == mark).map_or(u32::MAX, |i| {
        u32::try_from(i).expect("order holds at most 8")
    })
}

/// The GUID after `current` in GUID order (wrapping), the front when absent.
fn select_adjacent(current: u64, list: &mut [MobEntry], forward: bool) -> u64 {
    if forward {
        list.sort_unstable_by_key(|m| m.guid);
    } else {
        list.sort_unstable_by_key(|m| core::cmp::Reverse(m.guid));
    }
    let position = list.iter().position(|m| m.guid == current);
    if let Some(i) = position
        && i + 1 < list.len()
    {
        return list[i + 1].guid;
    }
    list.first().map_or(0, |m| m.guid)
}

/// The GUID after `current` in mark-priority order (wrapping).
fn select_adjacent_mark(current: u64, list: &mut [MobEntry], order: &[u32], forward: bool) -> u64 {
    if forward {
        list.sort_unstable_by_key(|m| mark_position(m.mark, order));
    } else {
        list.sort_unstable_by_key(|m| core::cmp::Reverse(mark_position(m.mark, order)));
    }
    let position = list.iter().position(|m| m.guid == current);
    if let Some(i) = position
        && i + 1 < list.len()
    {
        return list[i + 1].guid;
    }
    list.first().map_or(0, |m| m.guid)
}

/// One cumulative line for the target sweeps, when any has run.
pub fn emit_cumulative() {
    let sweeps = SWEEPS.get();
    if sweeps != 0 {
        log::info!(
            target: tally::TARGET,
            "unitxp target: {sweeps} sweeps, {} admitted, {} overflow",
            ADMITTED.get(),
            OVERFLOW.get(),
        );
    }
}
