//! Distance meters between two units, with server-side reach subtraction.
//!
//! Pure arithmetic behind the `distanceBetween` script command: five meters
//! whose formulas mirror the reference server's own range tests, so the
//! numbers a script sees agree with what the server would accept. Inputs are
//! plain positions and per-unit reach values; the record reads live in the
//! 32-bit resolver. All meters floor at zero except the raw one.

/// Which meter to measure with.
///
/// The Lua-facing default is [`Meter::Ranged`]; the raw 3-D distance is
/// [`Meter::Gaussian`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Meter {
    /// Raw 3-D Euclidean distance.
    Gaussian,
    /// `max(0, d - reach0 - reach1)` — the ranged-attack meter.
    Ranged,
    /// The melee auto-attack meter, 2-D with a height gate (see [`distance`]).
    MeleeAutoAttack,
    /// One-sided reach, counted only for a non-player unit.
    Aoe,
    /// Bounding radii subtracted — the chain-target meter.
    Chains,
}

/// Reach facts of one unit, as the resolver read them.
pub struct UnitReach {
    /// Server combat reach; negative means the record was missing.
    pub combat_reach: f32,
    /// Server bounding radius; negative means the record was missing.
    pub bounding_radius: f32,
    /// Whether the object is a non-player unit (the `AoE` meter counts only those).
    pub is_npc: bool,
}

/// The melee leeway added on top of both reaches (the f32 nearest 4/3).
const MELEE_LEEWAY: f32 = 4.0 / 3.0;

fn length3(dx: f32, dy: f32, dz: f32) -> f32 {
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// Measure `pos0 -> pos1` under `meter`.
///
/// The melee meter applies only when the height difference is under 6.0;
/// past that it falls through to the raw 3-D distance, exactly as the
/// reference command does. Missing reach records count as zero reach.
pub fn distance(
    meter: Meter,
    pos0: [f32; 3],
    pos1: [f32; 3],
    u0: &UnitReach,
    u1: &UnitReach,
) -> f32 {
    let dx = pos0[0] - pos1[0];
    let dy = pos0[1] - pos1[1];
    let dz = pos0[2] - pos1[2];
    let reach0 = u0.combat_reach.max(0.0);
    let reach1 = u1.combat_reach.max(0.0);
    if meter == Meter::MeleeAutoAttack && (pos0[2] - pos1[2]).abs() < 6.0 {
        let reach0 = reach0.max(1.5);
        let reach1 = reach1.max(1.5);
        let total = (reach0 + reach1 + MELEE_LEEWAY).max(5.0);
        return ((dx * dx + dy * dy).sqrt() - total).max(0.0);
    }
    match meter {
        Meter::Ranged => (length3(dx, dy, dz) - reach0 - reach1).max(0.0),
        Meter::Aoe => {
            // One reach only, and the second unit's overwrites the first's
            // when both are non-players — the reference's assignment order.
            let mut total = 0.0;
            if u0.is_npc {
                total = reach0;
            }
            if u1.is_npc {
                total = reach1;
            }
            (length3(dx, dy, dz) - total).max(0.0)
        }
        Meter::Chains => {
            let br0 = u0.bounding_radius.max(0.0);
            let br1 = u1.bounding_radius.max(0.0);
            (length3(dx, dy, dz) - br0 - br1).max(0.0)
        }
        // The melee meter past its height gate lands here too.
        Meter::Gaussian | Meter::MeleeAutoAttack => length3(dx, dy, dz),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn npc(combat_reach: f32, bounding_radius: f32) -> UnitReach {
        UnitReach {
            combat_reach,
            bounding_radius,
            is_npc: true,
        }
    }

    fn player(combat_reach: f32, bounding_radius: f32) -> UnitReach {
        UnitReach {
            combat_reach,
            bounding_radius,
            is_npc: false,
        }
    }

    #[test]
    fn gaussian_is_raw_distance() {
        let d = distance(
            Meter::Gaussian,
            [0.0, 0.0, 0.0],
            [3.0, 4.0, 0.0],
            &npc(2.0, 1.0),
            &npc(2.0, 1.0),
        );
        assert!((d - 5.0).abs() < 1e-6);
    }

    #[test]
    fn ranged_subtracts_both_reaches_and_floors() {
        let a = npc(2.0, 1.0);
        let b = npc(2.5, 1.0);
        let d = distance(Meter::Ranged, [0.0; 3], [10.0, 0.0, 0.0], &a, &b);
        assert!((d - 5.5).abs() < 1e-6);
        let close = distance(Meter::Ranged, [0.0; 3], [1.0, 0.0, 0.0], &a, &b);
        assert!(close.abs() < 1e-6);
    }

    #[test]
    fn missing_reach_counts_as_zero() {
        let a = npc(-1.0, -1.0);
        let d = distance(Meter::Ranged, [0.0; 3], [10.0, 0.0, 0.0], &a, &a);
        assert!((d - 10.0).abs() < 1e-6);
    }

    #[test]
    fn melee_is_planar_with_clamped_total() {
        // Reaches clamp to 1.5 each; total = max(5, 1.5 + 1.5 + leeway) = 5.
        let a = npc(0.0, 1.0);
        let d = distance(Meter::MeleeAutoAttack, [0.0; 3], [10.0, 0.0, 3.0], &a, &a);
        assert!((d - 5.0).abs() < 1e-6);
        // Big reaches take reach0 + reach1 + leeway instead of the 5 floor.
        let big = npc(4.0, 1.0);
        let d = distance(
            Meter::MeleeAutoAttack,
            [0.0; 3],
            [20.0, 0.0, 0.0],
            &big,
            &big,
        );
        assert!((d - (20.0 - (8.0 + MELEE_LEEWAY))).abs() < 1e-5);
    }

    #[test]
    fn melee_height_gate_falls_through_to_raw() {
        let a = npc(2.0, 1.0);
        let d = distance(Meter::MeleeAutoAttack, [0.0; 3], [0.0, 0.0, 8.0], &a, &a);
        assert!((d - 8.0).abs() < 1e-6);
    }

    #[test]
    fn aoe_counts_one_reach_second_npc_wins() {
        let d = distance(
            Meter::Aoe,
            [0.0; 3],
            [10.0, 0.0, 0.0],
            &npc(2.0, 1.0),
            &npc(3.0, 1.0),
        );
        assert!((d - 7.0).abs() < 1e-6);
        let vs_player = distance(
            Meter::Aoe,
            [0.0; 3],
            [10.0, 0.0, 0.0],
            &npc(2.0, 1.0),
            &player(3.0, 1.0),
        );
        assert!((vs_player - 8.0).abs() < 1e-6);
        let both_players = distance(
            Meter::Aoe,
            [0.0; 3],
            [10.0, 0.0, 0.0],
            &player(2.0, 1.0),
            &player(3.0, 1.0),
        );
        assert!((both_players - 10.0).abs() < 1e-6);
    }

    #[test]
    fn chains_subtracts_bounding_radii() {
        let d = distance(
            Meter::Chains,
            [0.0; 3],
            [10.0, 0.0, 0.0],
            &npc(2.0, 1.5),
            &npc(3.0, 2.5),
        );
        assert!((d - 6.0).abs() < 1e-6);
    }
}
