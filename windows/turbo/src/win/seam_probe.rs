//! Armed counters that price the candidate fork seams before any is taken.
//!
//! Each family answers one question a static read cannot: is the client's
//! own two-thread bone-animation arm (`[view+4] & 4`) live on this stack;
//! how many animate-eligible roots does a draw-list pass walk and how is
//! their cost distributed (the measured bound on what a worker split could
//! recover); does any model instance get visited twice in one frame through
//! the `+0x40` stamp (the aliasing hazard a fork must exclude); what does an
//! empty publish/join round trip on the worker pool cost at frame rate; how
//! full does the draw-list dedup scratch run and how much probing it costs;
//! and how many dynamic-buffer lock cycles the particle and precipitation
//! paths pay per session.
//!
//! Counters ride the same arm as the event gauge (`wow::events` at debug):
//! unarmed, every entry point is a load and a branch. All writers are the
//! game thread except the two bone-animation entry counters, which the
//! client's own worker thread can also reach and which therefore use locked
//! adds instead of the single-writer load-add-store idiom.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

static ANIM_PASSES: AtomicU32 = AtomicU32::new(0);
static ANIM_MULTI: AtomicU32 = AtomicU32::new(0);
static ANIM_WORKER_ARM: AtomicU32 = AtomicU32::new(0);
static ROOTS_SUM: AtomicU64 = AtomicU64::new(0);
static ROOTS_MAX: AtomicU32 = AtomicU32::new(0);
static LIST_SUM: AtomicU64 = AtomicU64::new(0);
static LIST_MAX: AtomicU32 = AtomicU32::new(0);
static ROOT_TICKS_SUM: AtomicU64 = AtomicU64::new(0);
static ROOT_TICKS_MAX: AtomicU64 = AtomicU64::new(0);

static ANIM_CALLS: AtomicU32 = AtomicU32::new(0);
static REPEAT_VISITS: AtomicU32 = AtomicU32::new(0);

static GC_RT_N: AtomicU32 = AtomicU32::new(0);
static GC_RT_SUM: AtomicU64 = AtomicU64::new(0);
static GC_RT_MAX: AtomicU64 = AtomicU64::new(0);

static FIN_PASSES: AtomicU32 = AtomicU32::new(0);
static FIN_B0_SUM: AtomicU64 = AtomicU64::new(0);
static FIN_B0_MAX: AtomicU32 = AtomicU32::new(0);
static FIN_PROBE_STEPS: AtomicU64 = AtomicU64::new(0);
static FIN_CMP_CALLS: AtomicU64 = AtomicU64::new(0);

static PARTICLE_LOCKS: AtomicU32 = AtomicU32::new(0);
static RAIN_LOCKS: AtomicU32 = AtomicU32::new(0);

/// Whether the probe counters are live (the event gauge's arm).
#[inline]
#[must_use]
pub fn armed() -> bool {
    super::events::armed()
}

/// Single-writer bump: exact because only the game thread writes.
///
/// A `fetch_add` would be a `lock`-prefixed RMW on i686; the plain
/// load-add-store avoids that on paths only one thread ever runs.
fn bump32(counter: &AtomicU32) {
    counter.store(
        counter.load(Ordering::Relaxed).wrapping_add(1),
        Ordering::Relaxed,
    );
}

/// Single-writer add into a 64-bit accumulator.
fn add64(counter: &AtomicU64, v: u64) {
    counter.store(
        counter.load(Ordering::Relaxed).wrapping_add(v),
        Ordering::Relaxed,
    );
}

/// Single-writer running maximum.
fn max64(counter: &AtomicU64, v: u64) {
    if v > counter.load(Ordering::Relaxed) {
        counter.store(v, Ordering::Relaxed);
    }
}

/// Single-writer running maximum, 32-bit.
fn max32(counter: &AtomicU32, v: u32) {
    if v > counter.load(Ordering::Relaxed) {
        counter.store(v, Ordering::Relaxed);
    }
}

/// One draw-list animation pass: which arm ran and what it walked.
///
/// `roots` counts the animate-eligible nodes our walk dispatched (the even
/// half when the client's worker arm is on), `root_ticks_*` their measured
/// cost, `list_len` the full visible-list length from the particle pass.
/// Caller gates on [`armed`]; this only records.
pub fn anim_phase(worker_arm: bool, roots: u32, ticks_sum: u64, ticks_max: u64, list_len: u32) {
    bump32(&ANIM_PASSES);
    if worker_arm {
        bump32(&ANIM_WORKER_ARM);
    }
    if roots > 1 {
        bump32(&ANIM_MULTI);
    }
    add64(&ROOTS_SUM, u64::from(roots));
    max32(&ROOTS_MAX, roots);
    add64(&LIST_SUM, u64::from(list_len));
    max32(&LIST_MAX, list_len);
    add64(&ROOT_TICKS_SUM, ticks_sum);
    max64(&ROOT_TICKS_MAX, ticks_max);
}

/// One bone-animation entry; `repeat` means the `+0x40` stamp matched.
///
/// A repeat is the dedup the serial walk relies on firing twice in one
/// frame, i.e. an instance reachable from two roots. Locked adds: with the
/// client's worker arm live this entry runs on two threads.
#[inline]
pub fn anim_entry(repeat: bool) {
    if !armed() {
        return;
    }
    ANIM_CALLS.fetch_add(1, Ordering::Relaxed);
    if repeat {
        REPEAT_VISITS.fetch_add(1, Ordering::Relaxed);
    }
}

/// One empty publish/join round trip on the worker pool, in ticks.
pub fn gc_roundtrip(ticks: u64) {
    bump32(&GC_RT_N);
    add64(&GC_RT_SUM, ticks);
    max64(&GC_RT_MAX, ticks);
}

/// One dedup pass over bucket 0: its record count and what probing cost.
pub fn finalize_stats(bucket0: u32, probe_steps: u32, cmp_calls: u32) {
    bump32(&FIN_PASSES);
    add64(&FIN_B0_SUM, u64::from(bucket0));
    max32(&FIN_B0_MAX, bucket0);
    add64(&FIN_PROBE_STEPS, u64::from(probe_steps));
    add64(&FIN_CMP_CALLS, u64::from(cmp_calls));
}

/// One dynamic-buffer resize/lock cycle in the particle emitter render.
#[inline]
pub fn particle_lock() {
    if armed() {
        bump32(&PARTICLE_LOCKS);
    }
}

/// One dynamic-buffer resize/lock cycle in the precipitation draw.
#[inline]
pub fn rain_lock() {
    if armed() {
        bump32(&RAIN_LOCKS);
    }
}

/// Ticks to microseconds through the calibrated engine clock.
fn ticks_to_us(ticks: u64) -> u64 {
    super::hooks::clock_ticks_to_ms(ticks.saturating_mul(1000))
}

/// Emit the cumulative seam lines on the gauge's heartbeat cadence.
///
/// Counters are never reset: like the sibling memo counters these are
/// session-cumulative, and the log's last line carries the totals.
pub fn emit_cumulative() {
    let passes = ANIM_PASSES.load(Ordering::Relaxed);
    if passes != 0 {
        let calls = ANIM_CALLS.load(Ordering::Relaxed);
        log::debug!(
            target: "wow::events",
            "seam anim: {passes} passes ({} multi-root, {} worker-arm), roots sum {} max {}, \
             list sum {} max {}, root us sum {} max {}, repeat {}/{calls}",
            ANIM_MULTI.load(Ordering::Relaxed),
            ANIM_WORKER_ARM.load(Ordering::Relaxed),
            ROOTS_SUM.load(Ordering::Relaxed),
            ROOTS_MAX.load(Ordering::Relaxed),
            LIST_SUM.load(Ordering::Relaxed),
            LIST_MAX.load(Ordering::Relaxed),
            ticks_to_us(ROOT_TICKS_SUM.load(Ordering::Relaxed)),
            ticks_to_us(ROOT_TICKS_MAX.load(Ordering::Relaxed)),
            REPEAT_VISITS.load(Ordering::Relaxed),
        );
    }
    let rounds = GC_RT_N.load(Ordering::Relaxed);
    if rounds != 0 {
        let sum = GC_RT_SUM.load(Ordering::Relaxed);
        log::debug!(
            target: "wow::events",
            "seam pool: {rounds} rounds, round-trip mean {}us max {}us",
            ticks_to_us(sum / u64::from(rounds)),
            ticks_to_us(GC_RT_MAX.load(Ordering::Relaxed)),
        );
    }
    let fin = FIN_PASSES.load(Ordering::Relaxed);
    if fin != 0 {
        log::debug!(
            target: "wow::events",
            "seam finalize: {fin} passes, bucket0 sum {} max {}, probe steps {}, cmp calls {}",
            FIN_B0_SUM.load(Ordering::Relaxed),
            FIN_B0_MAX.load(Ordering::Relaxed),
            FIN_PROBE_STEPS.load(Ordering::Relaxed),
            FIN_CMP_CALLS.load(Ordering::Relaxed),
        );
    }
    let p = PARTICLE_LOCKS.load(Ordering::Relaxed);
    let r = RAIN_LOCKS.load(Ordering::Relaxed);
    if p != 0 || r != 0 {
        log::debug!(target: "wow::events", "seam locks: particle {p}, rain {r}");
    }
}
