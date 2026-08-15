//! Counter primitives for the diagnostic layer: one gate, two disciplines.
//!
//! Every diagnostic counter in the crate is written through this module and
//! rides one gate, [`LIVE`], which is the build itself: `PERF=1` sets
//! `cfg(wow_turbo_perf)` and the layer exists, and a default build has no
//! layer at all. There is no runtime arm and nothing to switch on, so a
//! shipped session does not pay a load, a branch or a cache line for a counter
//! it was never going to read. [`arm`] answers `None` as a constant, every
//! counting site folds away with it, and so does the work a site had staged to
//! feed it.
//!
//! The corollary is that a `PERF=1` build is unconditionally instrumented: the
//! counters are always live, and the cumulative lines below report at `info`,
//! which the default filter shows. Compiling the layer in is the opt-in, so
//! nothing else has to be remembered at launch. The tripwires move with it:
//! the non-finite checks and the drift counters are part of this layer, not a
//! permanent property of a shipped build.
//!
//! This gate is deliberately NOT the event gauge's, which is a separate topic
//! ([`super::events`]) and instruments the client's script dispatch for over a
//! second of wall per minute. Both are compiled in together, but the gauge
//! keeps its own runtime arm on top, so a `PERF=1` session that only wants the
//! cheap counters does not buy the expensive gauge with them, and this module
//! owns the cadence that reports its own ([`heartbeat`]) rather than borrowing
//! the gauge's.
//!
//! Two properties are carried separately, because they are independent of each
//! other. Holding the layer open is a value the caller holds: [`arm`] hands out
//! the [`Armed`] token and a counter cannot be written without one, so no
//! record survives into a build that has no layer. Writer discipline belongs to
//! the counter's declared type: [`Counter`] and [`Accum`] are load-add-store,
//! which is exact where the game thread is the only writer and avoids the
//! `lock` prefix a `fetch_add` carries on i686, while [`SharedCounter`] pays
//! that prefix for the entries the client's own worker threads also reach. The
//! method names are the same on both, so the declaration decides the machine
//! code and a call site cannot pick the wrong one.
//!
//! Counters are session-cumulative and are never reset, so the last line of a
//! log carries the totals, and a family emits its line only when it has
//! something in it.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Log target every cumulative counter line is written to.
///
/// Grouped with the counter calibration the numbers are read through (the
/// engine clock's rate and read cost), because that is the one figure needed
/// to judge every span below it.
pub const TARGET: &str = "wow::perf";

/// Whether this build carries the diagnostic layer at all.
///
/// The one place the `cfg` is read. Every gate below is a branch on this
/// constant rather than an attribute on an item, which is what lets the ~75
/// counting sites across the crate, and the four modules of counters behind
/// them, stay written exactly once: with it false [`arm`] is `None`, a `get`
/// is `0`, so every reporting body and every argument the sites staged for one
/// is unreachable and the optimizer deletes it along with the counters
/// themselves.
const LIVE: bool = cfg!(wow_turbo_perf);

/// Milliseconds between cumulative reports.
const CADENCE_MS: u64 = 60_000;

/// Engine ticks at the last report, 0 until the first heartbeat.
static LAST_REPORT: AtomicU64 = AtomicU64::new(0);

/// Proof that the layer is compiled in, without which no counter can be written.
///
/// Zero-sized, so a `&Armed` parameter costs neither a register nor a stack
/// slot: it exists to put the gate in the type system rather than in a
/// convention every new counting site has to remember.
pub struct Armed;

/// The token in a build carrying the layer, `None` in one that does not.
#[must_use]
#[inline]
pub const fn arm() -> Option<Armed> {
    if LIVE { Some(Armed) } else { None }
}

/// Report every family's counters once a cadence has elapsed.
///
/// Called from a per-frame path and from the event gauge's own emission, so
/// the report survives either being switched off; the elapsed check makes the
/// extra call free. The first heartbeat only starts the clock, so the first
/// report covers a full cadence rather than however long start-up took.
///
/// Without the layer the token is `None` here and the whole fan-out below is
/// unreachable, which is what takes the reporting bodies, their format strings
/// and the counters they read out of the build.
pub fn heartbeat() {
    if arm().is_none() {
        return;
    }
    let now = wow_shared::tsc::rdtsc();
    let last = LAST_REPORT.load(Ordering::Relaxed);
    if last != 0 && super::hooks::clock_ticks_to_ms(now.wrapping_sub(last)) < CADENCE_MS {
        return;
    }
    LAST_REPORT.store(now, Ordering::Relaxed);
    if last == 0 {
        return;
    }
    super::getname::emit_cumulative();
    super::hooks::emit_cumulative();
    super::script_method::emit_cumulative();
    super::seam_probe::emit_cumulative();
    super::unitxp::emit_cumulative();
    super::transmog::emit_cumulative();
    #[cfg(not(wow_turbo_diff))]
    super::filecache::emit_cumulative();
}

/// A counter only the game thread writes, load-add-store on purpose.
///
/// With one writer a read-modify-write is exact, and a `fetch_add` would be a
/// `lock`-prefixed RMW on i686.
pub struct Counter(AtomicU32);

impl Counter {
    /// A counter starting from zero.
    #[must_use]
    pub const fn zero() -> Self {
        Self(AtomicU32::new(0))
    }

    /// Count one more.
    #[inline]
    pub fn bump(&self, armed: &Armed) {
        self.add(armed, 1);
    }

    /// Add `v` to the running total.
    #[inline]
    pub fn add(&self, _armed: &Armed, v: u32) {
        self.0.store(
            self.0.load(Ordering::Relaxed).wrapping_add(v),
            Ordering::Relaxed,
        );
    }

    /// Keep `v` if it is larger than what is recorded.
    #[inline]
    pub fn max(&self, _armed: &Armed, v: u32) {
        if v > self.0.load(Ordering::Relaxed) {
            self.0.store(v, Ordering::Relaxed);
        }
    }

    /// The running value, for a report.
    ///
    /// Constant `0` without the layer, which is what folds each reporting
    /// body's "did this family run" guard and deletes its line and its format
    /// string with it.
    #[must_use]
    pub fn get(&self) -> u32 {
        if LIVE {
            self.0.load(Ordering::Relaxed)
        } else {
            0
        }
    }
}

/// A 64-bit accumulator only the game thread writes.
///
/// The 64-bit width is for sums that run for a session: tick totals, and counts
/// of things counted per frame.
pub struct Accum(AtomicU64);

impl Accum {
    /// An accumulator starting from zero.
    #[must_use]
    pub const fn zero() -> Self {
        Self(AtomicU64::new(0))
    }

    /// Add `v` to the running total.
    #[inline]
    pub fn add(&self, _armed: &Armed, v: u64) {
        self.0.store(
            self.0.load(Ordering::Relaxed).wrapping_add(v),
            Ordering::Relaxed,
        );
    }

    /// Keep `v` if it is larger than what is recorded.
    #[inline]
    pub fn max(&self, _armed: &Armed, v: u64) {
        if v > self.0.load(Ordering::Relaxed) {
            self.0.store(v, Ordering::Relaxed);
        }
    }

    /// The running value, for a report; constant `0` without the layer.
    #[must_use]
    pub fn get(&self) -> u64 {
        if LIVE {
            self.0.load(Ordering::Relaxed)
        } else {
            0
        }
    }
}

/// A counter more than one thread reaches, and so a locked add.
///
/// Costs the `lock` prefix [`Counter`] avoids. Declare one only where a second
/// writer is named: an entry the client's own worker threads run, or one whose
/// callers have not been established.
pub struct SharedCounter(AtomicU32);

impl SharedCounter {
    /// A counter starting from zero.
    #[must_use]
    pub const fn zero() -> Self {
        Self(AtomicU32::new(0))
    }

    /// Count one more.
    #[inline]
    pub fn bump(&self, _armed: &Armed) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    /// The running value, for a report; constant `0` without the layer.
    #[must_use]
    pub fn get(&self) -> u32 {
        if LIVE {
            self.0.load(Ordering::Relaxed)
        } else {
            0
        }
    }
}

/// Count one on a single-writer counter, taking the token here.
///
/// The form for a site whose whole bookkeeping is the one bump. A caller with
/// more to skip — an argument to compute, a clock to read — holds the token
/// from [`arm`] over that work and calls the method instead.
#[inline]
pub fn bump(counter: &Counter) {
    if let Some(armed) = arm() {
        counter.bump(&armed);
    }
}

/// Count one on a multi-writer counter, taking the token here.
#[inline]
pub fn bump_shared(counter: &SharedCounter) {
    if let Some(armed) = arm() {
        counter.bump(&armed);
    }
}
