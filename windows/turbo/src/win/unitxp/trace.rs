//! The world line-of-sight trace behind the sight features.
//!
//! Calls the client's top-level world intersect (`0x672170`) — an entry this
//! mod deliberately does NOT hook, so the call lands in the reimplemented
//! terrain/WMO/scene arms underneath it and stays free for other mods to
//! wrap. The query flag is the conservative one the command set has always
//! used: the fuller flag is a known dungeon crash.
//!
//! Two guards run before any real trace, both squared so no root is taken:
//! endpoints further apart than 150 units answer a fabricated mid-segment hit
//! (mixed coordinate systems around transports once crashed the trace, and
//! "blocked" is the safe answer), and endpoints closer than the float
//! tolerance answer "no hit" without tracing.

use crate::win::tally::{self, Counter};

/// `CWorld::Intersect` — `fastcall(ecx = p1, edx = p2)` + 4 stack args, `RET 0x10`.
const CWORLD_INTERSECT_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0027_2170;

/// The query flag every sight trace passes (the fuller `0x100171` crashes).
const QUERY_FLAG: u32 = 0x0010_0111;

/// Squared distance beyond which a hit is fabricated instead of traced.
const GUARD_DIST_SQ: f32 = 150.0 * 150.0;

/// Squared endpoint tolerance under which the answer is "no hit".
const DEGENERATE_SQ: f32 = 1e-5 * 1e-5;

/// Fabricated over-distance hits and degenerate short-circuits (armed runs).
static FABRICATED: Counter = Counter::zero();
/// See [`FABRICATED`].
static DEGENERATE: Counter = Counter::zero();

/// [`world_intersect_flagged`] with the sight features' conservative flag.
pub fn world_intersect(from: &[f32; 3], to: &[f32; 3]) -> Option<f32> {
    world_intersect_flagged(from, to, QUERY_FLAG)
}

/// Trace `from -> to` against the world; `Some(fraction)` when it hit.
///
/// A fraction in `[0, 1]` is a hit between the endpoints; past `1` the ray
/// continued beyond `to` before hitting. Endpoints further apart than the
/// guard answer `Some(0.5)` without tracing. The camera feature passes the
/// game's own camera-collision flag; everything else takes the default.
pub fn world_intersect_flagged(from: &[f32; 3], to: &[f32; 3], flag: u32) -> Option<f32> {
    let dx = to[0] - from[0];
    let dy = to[1] - from[1];
    let dz = to[2] - from[2];
    let dist_sq = dx * dx + dy * dy + dz * dz;
    if dist_sq > GUARD_DIST_SQ {
        tally::bump(&FABRICATED);
        return Some(0.5);
    }
    if dist_sq <= DEGENERATE_SQ {
        tally::bump(&DEGENERATE);
        return None;
    }
    let mut intersect_point = [0.0f32; 3];
    let mut fraction = 1.0f32;
    // SAFETY: a fixed `.text` entry in the live host image (base verified
    // at load); the transmuted signature matches the declared prototype
    // (`__fastcall(ecx = p1, edx = p2)` plus four stack arguments,
    // `RET 0x10`, result in `al`).
    let intersect: extern "fastcall" fn(
        *const [f32; 3],
        *const [f32; 3],
        i32,
        *mut [f32; 3],
        *mut f32,
        u32,
    ) -> u8 = unsafe { core::mem::transmute(CWORLD_INTERSECT_VA) };
    let hit = intersect(
        from,
        to,
        0,
        &raw mut intersect_point,
        &raw mut fraction,
        flag,
    );
    (hit != 0).then_some(fraction)
}

/// One cumulative line for the trace guards, when either has fired.
pub fn emit_cumulative() {
    let fabricated = FABRICATED.get();
    let degenerate = DEGENERATE.get();
    if fabricated | degenerate != 0 {
        log::info!(
            target: tally::TARGET,
            "unitxp trace: {fabricated} fabricated, {degenerate} degenerate",
        );
    }
}
