//! The `distanceBetween` command: record reads feeding the pure meters.
//!
//! Resolution order, sentinels and type gates mirror the command set this
//! stands in for: identical raw objects are distance zero before any type
//! check, anything that is not a unit or player answers -1, and a missing
//! reach record counts as zero reach inside the meters. The arithmetic
//! itself lives in the portable kernel (`crate::math::reach`), where it is
//! unit-tested; verification of this resolver is the in-game side-by-side.

use crate::math::reach::{Meter, UnitReach, distance};

fn reach_of(unit: super::super::objmgr::UnitRef) -> UnitReach {
    UnitReach {
        combat_reach: unit.combat_reach().unwrap_or(-1.0),
        bounding_radius: unit.bounding_radius().unwrap_or(-1.0),
        is_npc: unit.object_type() == super::super::objmgr::TYPE_UNIT,
    }
}

/// Distance between two live units under `meter`; -1 for an unmodelled shape.
pub fn between_units(
    u0: super::super::objmgr::UnitRef,
    u1: super::super::objmgr::UnitRef,
    meter: Meter,
) -> f32 {
    if u0.raw() == u1.raw() {
        return 0.0;
    }
    if !u0.is_unit_or_player() || !u1.is_unit_or_player() {
        return -1.0;
    }
    distance(
        meter,
        u0.position(),
        u1.position(),
        &reach_of(u0),
        &reach_of(u1),
    )
}

/// The command body over two raw string arguments and the optional meter name.
///
/// The Lua-facing default meter is ranged; an unrecognized meter name keeps
/// that default, exactly as the original's name chain does.
pub fn command(unit0: &[u8], unit1: &[u8], meter_name: Option<&[u8]>) -> f32 {
    let meter = match meter_name {
        Some(b"meleeAutoAttack") => Meter::MeleeAutoAttack,
        Some(b"AoE") => Meter::Aoe,
        Some(b"chains") => Meter::Chains,
        Some(b"Gaussian") => Meter::Gaussian,
        _ => Meter::Ranged,
    };
    let Some(guid0) = super::insight::parse_unit_guid(unit0) else {
        return -1.0;
    };
    let Some(guid1) = super::insight::parse_unit_guid(unit1) else {
        return -1.0;
    };
    if guid0 == guid1 {
        return 0.0;
    }
    let Some(u0) = super::super::objmgr::object_by_guid(guid0) else {
        return -1.0;
    };
    let Some(u1) = super::super::objmgr::object_by_guid(guid1) else {
        return -1.0;
    };
    between_units(u0, u1, meter)
}
