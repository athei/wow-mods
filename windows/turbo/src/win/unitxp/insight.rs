//! Sight tests between units and from the camera, behind a position-aware cache.
//!
//! The command set this stands in for cached sight verdicts for 100-160 ms
//! per key, which its common pollers (asking every ~100 ms) missed almost
//! every time — each miss costing one or two engine traces. The cache here
//! keeps that envelope as a floor and adds what the original could not say:
//! a verdict whose endpoints have not moved is still true. Each entry stores
//! both endpoint positions quantized to half-yard cells; while they match the
//! fresh reads, the verdict is served up to a hard age cap.
//!
//! The cap exists because geometry is not quite static: an instance door can
//! flip a verdict between two stationary endpoints. Half a second bounds that
//! staleness to a few times the original's own worst case, on an event that
//! is rare, in exchange for cutting the steady-state trace rate several-fold.
//!
//! The camera side deliberately caches LESS than the original: only the
//! occlusion trace (a function of camera and target positions) is cached,
//! and the frustum test runs fresh on every call — so verdicts follow camera
//! rotation immediately, where the original answered stale for up to 160 ms.
//! A frustum fail writes nothing.
//!
//! Verdicts are 1 in sight, 0 not, -1 error; errors are never cached.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::win::tally::{self, Counter};

/// Cache geometry: 256 sets of 4 ways, addressed as one flat array.
const SETS: usize = 256;
/// See [`SETS`].
const WAYS: usize = 4;
/// See [`SETS`].
const ENTRIES: usize = SETS * WAYS;

/// Serve any entry younger than this many milliseconds — the original's TTL.
///
/// This floor is what bounds the worst case: no key is ever traced more
/// often than the command set it replaces would have traced it.
const TTL_FLOOR_MS: u32 = 100;

/// Millisecond spread added to the floor, hashed per key (the original's dither).
const TTL_DITHER_MS: u32 = 61;

/// Age cap for position-stable serving, dithered ±60 ms — the door trade.
///
/// A verdict older than this re-traces even with both endpoints unmoved, so
/// animating geometry (doors, elevator cars) can flip an answer between two
/// stationary units at most this late. Tighten to 250 if door response ever
/// reads as laggy in play.
const MAX_AGE_BASE_MS: u32 = 440;
/// See [`MAX_AGE_BASE_MS`].
const MAX_AGE_DITHER_MS: u32 = 121;

/// Unit-pair endpoints quantize to half-yard cells.
const UNIT_CELLS_PER_YARD: f32 = 2.0;

/// Camera endpoints quantize to 0.75-yard cells (orbit jitter tolerance).
const CAMERA_CELLS_PER_YARD: f32 = 4.0 / 3.0;

/// The camera frustum half-angle base (the client's screen-border constant).
///
/// Read from the live camera this disagrees with actual border culling when a
/// field-of-view tweak is active, so the constant the original validated
/// in-game is kept.
// the reference validated this exact literal against in-game border culling;
// the mathematical half-pi is a different value and changes the cone
#[allow(clippy::approx_constant)]
const CAMERA_FOV: f32 = 1.5708;

/// Camera hits at or under this fraction are the camera's own resting surface.
///
/// Not an occluder: stands in for the original's intersect-blur wrapper.
const CAMERA_HIT_FLOOR: f32 = 0.01;

/// Fixed camera-target head bump.
///
/// Real collision heights overshoot the frame on very tall models, so the
/// original uses a constant.
const CAMERA_TARGET_BUMP: f32 = 2.1;

/// One sight cache in structure-of-arrays form.
///
/// Keys are GUID pairs (`key_hi` zero for the camera cache's single-GUID
/// keys); `key_lo == 0` marks an empty entry and is written last on insert so
/// a torn entry is never live. Single-writer (the game thread): atomics are
/// shared mutability for a `static`, not cross-thread ordering.
struct SightCache {
    key_lo: [AtomicU64; ENTRIES],
    key_hi: [AtomicU64; ENTRIES],
    pos_a: [AtomicU64; ENTRIES],
    pos_b: [AtomicU64; ENTRIES],
    /// Verdict bit 32, insertion stamp (engine ms) in the low half.
    meta: [AtomicU64; ENTRIES],
    /// Round-robin eviction cursor; a full set evicts `cursor % WAYS`.
    cursor: AtomicU32,
    /// TTL hits, position-stable hits, misses, evictions (armed runs only).
    ttl_hits: Counter,
    /// See above.
    pos_hits: Counter,
    /// See above.
    misses: Counter,
    /// See above.
    evictions: Counter,
}

impl SightCache {
    const fn new() -> Self {
        Self {
            key_lo: [const { AtomicU64::new(0) }; ENTRIES],
            key_hi: [const { AtomicU64::new(0) }; ENTRIES],
            pos_a: [const { AtomicU64::new(0) }; ENTRIES],
            pos_b: [const { AtomicU64::new(0) }; ENTRIES],
            meta: [const { AtomicU64::new(0) }; ENTRIES],
            cursor: AtomicU32::new(0),
            ttl_hits: Counter::zero(),
            pos_hits: Counter::zero(),
            misses: Counter::zero(),
            evictions: Counter::zero(),
        }
    }

    /// Serve a verdict for `(lo, hi)` at endpoints `(qa, qb)`, or record a miss.
    fn lookup(&self, lo: u64, hi: u64, qa: u64, qb: u64) -> Option<i32> {
        let mixed = mix(lo, hi);
        let set = (mixed >> 33) as usize & (SETS - 1);
        for way in 0..WAYS {
            let idx = set * WAYS + way;
            if self.key_lo[idx].load(Ordering::Relaxed) != lo
                || self.key_hi[idx].load(Ordering::Relaxed) != hi
            {
                continue;
            }
            let meta = self.meta[idx].load(Ordering::Relaxed);
            let verdict = i32::from(meta >> 32 != 0);
            let age = super::super::objmgr::game_tick_ms().wrapping_sub(meta as u32);
            if age < TTL_FLOOR_MS + (mixed as u32) % TTL_DITHER_MS {
                tally::bump(&self.ttl_hits);
                return Some(verdict);
            }
            if age < MAX_AGE_BASE_MS + ((mixed >> 8) as u32) % MAX_AGE_DITHER_MS
                && self.pos_a[idx].load(Ordering::Relaxed) == qa
                && self.pos_b[idx].load(Ordering::Relaxed) == qb
            {
                tally::bump(&self.pos_hits);
                return Some(verdict);
            }
            tally::bump(&self.misses);
            return None;
        }
        tally::bump(&self.misses);
        None
    }

    /// Record a fresh verdict, reusing the key's entry or evicting one.
    fn insert(&self, lo: u64, hi: u64, qa: u64, qb: u64, verdict: i32) {
        let mixed = mix(lo, hi);
        let set = (mixed >> 33) as usize & (SETS - 1);
        let mut chosen = None;
        for way in 0..WAYS {
            let idx = set * WAYS + way;
            let key = self.key_lo[idx].load(Ordering::Relaxed);
            if key == lo && self.key_hi[idx].load(Ordering::Relaxed) == hi {
                chosen = Some(idx);
                break;
            }
            if key == 0 && chosen.is_none() {
                chosen = Some(idx);
            }
        }
        let idx = chosen.unwrap_or_else(|| {
            let cursor = self.cursor.load(Ordering::Relaxed);
            self.cursor.store(cursor.wrapping_add(1), Ordering::Relaxed);
            tally::bump(&self.evictions);
            set * WAYS + cursor as usize % WAYS
        });
        self.key_lo[idx].store(0, Ordering::Relaxed);
        self.key_hi[idx].store(hi, Ordering::Relaxed);
        self.pos_a[idx].store(qa, Ordering::Relaxed);
        self.pos_b[idx].store(qb, Ordering::Relaxed);
        let stamp = u64::from(super::super::objmgr::game_tick_ms());
        self.meta[idx].store(stamp | u64::from(verdict != 0) << 32, Ordering::Relaxed);
        self.key_lo[idx].store(lo, Ordering::Relaxed);
    }
}

/// Unit-pair verdicts, keyed by the canonical (smaller, larger) GUID pair.
static UNIT_CACHE: SightCache = SightCache::new();

/// Camera-trace verdicts, keyed by target GUID (`key_hi` zero).
static CAMERA_CACHE: SightCache = SightCache::new();

/// Frustum pre-culls that skipped the trace entirely (armed runs only).
static FRUSTUM_CULLED: Counter = Counter::zero();

/// Mix a pair key into well-spread bits (set index and per-key dither).
const fn mix(lo: u64, hi: u64) -> u64 {
    let folded = lo.wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ hi.rotate_left(32).wrapping_mul(0xff51_afd7_ed55_8ccd);
    folded ^ (folded >> 29)
}

/// Quantize a position to cells, packed 3 x 21 bits.
fn quantize(pos: [f32; 3], cells_per_yard: f32) -> u64 {
    let mut packed = 0u64;
    for (i, &c) in pos.iter().enumerate() {
        let cell = (c * cells_per_yard).floor() as i32;
        packed |= u64::from(cell.cast_unsigned() & 0x1f_ffff) << (i * 21);
    }
    packed
}

/// Whether `pos` sits inside the camera cone, computed fresh (no `acos`).
///
/// In-cone iff the angle to the camera forward vector is at most
/// `CAMERA_FOV / cone`, tested as `dot >= cos(threshold) * |v| * |fwd|` — the
/// same predicate the original derives through `acos`, without the
/// transcendental. A degenerate vector answers "outside", as the original's
/// sentinel angle does.
pub fn in_viewing_frustum(pos: [f32; 3], cone: f32) -> bool {
    let cam = super::camera::translated_position();
    let fwd = super::camera::rotated_forward();
    let v = [pos[0] - cam[0], pos[1] - cam[1], pos[2] - cam[2]];
    let len_product = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
        * (fwd[0] * fwd[0] + fwd[1] * fwd[1] + fwd[2] * fwd[2]).sqrt();
    if len_product <= 0.0 {
        return false;
    }
    let dot = v[0] * fwd[0] + v[1] * fwd[1] + v[2] * fwd[2];
    dot >= cos_of(CAMERA_FOV / cone) * len_product
}

/// `cos(threshold)`, memoized on the threshold bits (cones change rarely).
fn cos_of(threshold: f32) -> f32 {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let bits = threshold.to_bits();
    let last = LAST.load(Ordering::Relaxed);
    if last as u32 == bits && last != 0 {
        return f32::from_bits((last >> 32) as u32);
    }
    let value = threshold.cos();
    LAST.store(
        u64::from(value.to_bits()) << 32 | u64::from(bits),
        Ordering::Relaxed,
    );
    value
}

/// Sight between two live units: 1, 0, or -1 for a shape not modelled.
pub fn unit_in_sight(u0: super::super::objmgr::UnitRef, u1: super::super::objmgr::UnitRef) -> i32 {
    if !u0.is_unit_or_player() || !u1.is_unit_or_player() {
        return -1;
    }
    let g0 = u0.guid();
    let g1 = u1.guid();
    if g0 == g1 {
        return 1;
    }
    // Endpoints are needed for the trace anyway, so probing costs no extra
    // record reads; the pair and its positions canonicalize together.
    let p0 = u0.position();
    let p1 = u1.position();
    let (lo, hi, qa, qb) = if g0 <= g1 {
        (
            g0,
            g1,
            quantize(p0, UNIT_CELLS_PER_YARD),
            quantize(p1, UNIT_CELLS_PER_YARD),
        )
    } else {
        (
            g1,
            g0,
            quantize(p1, UNIT_CELLS_PER_YARD),
            quantize(p0, UNIT_CELLS_PER_YARD),
        )
    };
    if let Some(verdict) = UNIT_CACHE.lookup(lo, hi, qa, qb) {
        return verdict;
    }
    let verdict = i32::from(line_of_sight(u0, u1, p0, p1));
    UNIT_CACHE.insert(lo, hi, qa, qb, verdict);
    verdict
}

/// The two-trace sight protocol between two units.
///
/// Trace one looks across level eyes: BOTH endpoints raised by the shorter
/// unit's collision height (deliberately, per the original), a hit counting
/// as final only when the heights are exactly equal. Otherwise trace two
/// looks up at the taller unit, each endpoint raised by its own height.
fn line_of_sight(
    u0: super::super::objmgr::UnitRef,
    u1: super::super::objmgr::UnitRef,
    p0: [f32; 3],
    p1: [f32; 3],
) -> bool {
    let mut h0 = u0.collision_box_height();
    let mut h1 = u1.collision_box_height();
    let (mut p0, mut p1) = (p0, p1);
    if h0 > h1 {
        core::mem::swap(&mut h0, &mut h1);
        core::mem::swap(&mut p0, &mut p1);
    }
    let a = [p0[0], p0[1], p0[2] + h0];
    let b = [p1[0], p1[1], p1[2] + h0];
    match super::trace::world_intersect(&a, &b) {
        Some(fraction) if (0.0..=1.0).contains(&fraction) => {
            // The exact-equality height compare is the original's own branch:
            // equal heights make trace two identical to trace one.
            // the reference tests exact equality here; an epsilon would run a
            // second trace the original skips for same-model pairs
            #[allow(clippy::float_cmp)]
            if h0 == h1 {
                return false;
            }
            let a = [p0[0], p0[1], p0[2] + h0];
            let b = [p1[0], p1[1], p1[2] + h1];
            !matches!(
                super::trace::world_intersect(&a, &b),
                Some(fraction) if (0.0..=1.0).contains(&fraction)
            )
        }
        _ => true,
    }
}

/// Sight from the camera to a unit: 1, 0, or -1 for a shape not modelled.
pub fn camera_in_sight(unit: super::super::objmgr::UnitRef) -> i32 {
    if !unit.is_unit_or_player() {
        return -1;
    }
    let guid = unit.guid();
    if guid == super::super::objmgr::active_player_guid() {
        return 1;
    }
    let pos = unit.position();
    // Fresh every call — a cached frustum verdict would lag camera rotation.
    if !in_viewing_frustum(pos, 2.0) {
        tally::bump(&FRUSTUM_CULLED);
        return 0;
    }
    let cam = super::camera::translated_position();
    let q_unit = quantize(pos, CAMERA_CELLS_PER_YARD);
    let q_cam = quantize(cam, CAMERA_CELLS_PER_YARD);
    if let Some(verdict) = CAMERA_CACHE.lookup(guid, 0, q_unit, q_cam) {
        return verdict;
    }
    let target = [pos[0], pos[1], pos[2] + CAMERA_TARGET_BUMP];
    let verdict = match super::trace::world_intersect(&cam, &target) {
        Some(fraction) if fraction > CAMERA_HIT_FLOOR && fraction <= 1.0 => 0,
        _ => 1,
    };
    CAMERA_CACHE.insert(guid, 0, q_unit, q_cam, verdict);
    verdict
}

/// [`camera_in_sight`] from a GUID, -1 when it resolves to nothing.
pub fn camera_in_sight_guid(guid: u64) -> i32 {
    super::super::objmgr::object_by_guid(guid).map_or(-1, camera_in_sight)
}

/// Whether `me` is behind `mob`: 1, 0, or -1 for a shape not modelled.
///
/// The forward vector is the mob's facing — except for a stationary NPC in
/// combat with a visible target, whose server-side facing goes stale mid-melee:
/// there the vector toward its target stands in. Behind means the planar angle
/// from that forward to `me` exceeds the configured threshold.
pub fn behind(me: super::super::objmgr::UnitRef, mob: super::super::objmgr::UnitRef) -> i32 {
    if me.raw() == mob.raw() {
        return -1;
    }
    if !me.is_unit_or_player() || !mob.is_unit_or_player() {
        return -1;
    }
    let pos_me = me.position();
    let pos_mob = mob.position();
    let facing = mob.facing();
    let mut forward = [facing.cos(), facing.sin()];
    if mob.object_type() == super::super::objmgr::TYPE_UNIT && mob.in_combat() && !mob.is_moving() {
        let target_guid = mob.target_guid();
        if let Some(target) = super::super::objmgr::object_by_guid(target_guid)
            && target.is_unit_or_player()
            && unit_in_sight(mob, target) > 0
        {
            let pos_target = target.position();
            forward = [pos_target[0] - pos_mob[0], pos_target[1] - pos_mob[1]];
        }
    }
    let check = [pos_me[0] - pos_mob[0], pos_me[1] - pos_mob[1]];
    let len_product = (forward[0] * forward[0] + forward[1] * forward[1]).sqrt()
        * (check[0] * check[0] + check[1] * check[1]).sqrt();
    if len_product <= 0.0 {
        // The original's degenerate-vector sentinel angle reports an error.
        return -1;
    }
    let dot = forward[0] * check[0] + forward[1] * check[1];
    // In front iff the angle is under the threshold: `cos` is monotone
    // decreasing on the angle range, so the strict compare inverts exactly.
    let threshold = super::settings::behind_threshold();
    if dot > cos_of(threshold) * len_product {
        return 0;
    }
    1
}

/// Resolve a script unit argument to a GUID.
///
/// A `0x`-prefixed argument is a hex GUID (the whole rest must parse, which
/// tightens the original's contains-`0x` scan); anything else is a unit token
/// resolved through the client. Zero and unparsable answer `None`.
pub fn parse_unit_guid(arg: &[u8]) -> Option<u64> {
    if arg.is_empty() {
        return None;
    }
    if let Some(hex) = arg.strip_prefix(b"0x") {
        let text = core::str::from_utf8(hex).ok()?;
        return u64::from_str_radix(text, 16).ok().filter(|&g| g != 0);
    }
    let mut token = [0u8; 64];
    if arg.len() >= token.len() {
        return None;
    }
    token[..arg.len()].copy_from_slice(arg);
    let token = core::ffi::CStr::from_bytes_until_nul(&token).ok()?;
    let guid = super::super::objmgr::guid_of_token(token);
    (guid != 0).then_some(guid)
}

/// The `inSight` command body over two raw string arguments.
pub fn command_in_sight(unit0: &[u8], unit1: &[u8]) -> i32 {
    // The second unit resolves first, exactly as the original orders its
    // errors; only then is the camera form of the first argument considered.
    let Some(guid1) = parse_unit_guid(unit1) else {
        return -1;
    };
    if unit0 == b"camera" {
        return camera_in_sight_guid(guid1);
    }
    let Some(guid0) = parse_unit_guid(unit0) else {
        return -1;
    };
    if guid0 == guid1 {
        return 1;
    }
    let Some(u0) = super::super::objmgr::object_by_guid(guid0) else {
        return -1;
    };
    let Some(u1) = super::super::objmgr::object_by_guid(guid1) else {
        return -1;
    };
    unit_in_sight(u0, u1)
}

/// The `behind` command body over two raw string arguments.
pub fn command_behind(unit0: &[u8], unit1: &[u8]) -> i32 {
    let Some(guid1) = parse_unit_guid(unit1) else {
        return -1;
    };
    let Some(guid0) = parse_unit_guid(unit0) else {
        return -1;
    };
    if guid0 == guid1 {
        return -1;
    }
    let Some(me) = super::super::objmgr::object_by_guid(guid0) else {
        return -1;
    };
    let Some(mob) = super::super::objmgr::object_by_guid(guid1) else {
        return -1;
    };
    behind(me, mob)
}

/// One cumulative line per sight cache, when that cache has been asked.
pub fn emit_cumulative() {
    for (label, cache) in [("unit", &UNIT_CACHE), ("cam", &CAMERA_CACHE)] {
        let ttl = cache.ttl_hits.get();
        let pos = cache.pos_hits.get();
        let misses = cache.misses.get();
        if ttl | pos | misses != 0 {
            log::debug!(
                target: tally::TARGET,
                "unitxp sight {label}: {ttl} ttl + {pos} pos / {misses} miss, evict {}",
                cache.evictions.get(),
            );
        }
    }
    let culled = FRUSTUM_CULLED.get();
    if culled != 0 {
        log::debug!(target: tally::TARGET, "unitxp sight: frustum-cull {culled}");
    }
}
