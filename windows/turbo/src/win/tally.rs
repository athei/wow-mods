//! Counter primitives for the armed instrumentation: one arm, two disciplines.
//!
//! Every diagnostic counter in the crate is written through this module and
//! rides one arm, the event gauge's `wow::events` at debug, resolved once. An
//! unarmed session therefore pays a cached load and a branch per counting site
//! and touches no counter at all. The exception is a permanent tripwire that
//! has to fire at the default filter, which counts on its cold branch only and
//! says so where it does it.
//!
//! Two properties are carried separately, because they are independent of each
//! other. Being armed is a value the caller holds: [`arm`] hands out the
//! [`Armed`] token and a counter cannot be written without one, so no record
//! can be reached from an unarmed path. Writer discipline belongs to the
//! counter's declared type: [`Counter`] and [`Accum`] are load-add-store, which
//! is exact where the game thread is the only writer and avoids the `lock`
//! prefix a `fetch_add` carries on i686, while [`SharedCounter`] pays that
//! prefix for the entries the client's own worker threads also reach. The
//! method names are the same on both, so the declaration decides the machine
//! code and a call site cannot pick the wrong one.
//!
//! Counters are session-cumulative and are never reset, so the last line of a
//! log carries the totals, and a family emits its line only when it has
//! something in it.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Log target every cumulative counter line is written to.
pub const TARGET: &str = "wow::events";

/// Proof that the arm was read, without which no counter can be written.
///
/// Zero-sized, so a `&Armed` parameter costs neither a register nor a stack
/// slot: it exists to put the arm check in the type system rather than in a
/// convention every new counting site has to remember.
pub struct Armed;

/// The token while the instrumentation is armed, `None` while it is not.
#[must_use]
pub fn arm() -> Option<Armed> {
    super::events::armed().then_some(Armed)
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
    #[must_use]
    pub fn get(&self) -> u32 {
        self.0.load(Ordering::Relaxed)
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

    /// The running value, for a report.
    #[must_use]
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
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

    /// The running value, for a report.
    #[must_use]
    pub fn get(&self) -> u32 {
        self.0.load(Ordering::Relaxed)
    }
}

/// Count one on a single-writer counter, reading the arm here.
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

/// Count one on a multi-writer counter, reading the arm here.
#[inline]
pub fn bump_shared(counter: &SharedCounter) {
    if let Some(armed) = arm() {
        counter.bump(&armed);
    }
}
