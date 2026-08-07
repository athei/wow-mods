//! Counters for the archive file-entry memo, reported with the other gauges.
//!
//! The memo itself lives with the lookup adapter; what is here is how well it
//! answers, which is the one thing about it a session can only learn at run
//! time. A replayed positive answer saved an archive-list walk, a replayed
//! negative one saved the whole miss path, and a miss is a lookup the memo
//! could not serve.
//!
//! The counters are declared as the locked-add shape: archive lookups are the
//! client's asset path, and nothing here establishes a single writer.

use super::tally::SharedCounter;

static POS_HITS: SharedCounter = SharedCounter::zero();
static NEG_HITS: SharedCounter = SharedCounter::zero();
static MISSES: SharedCounter = SharedCounter::zero();

/// A positive answer replayed from the memo.
#[inline]
pub fn hit_positive() {
    super::tally::bump_shared(&POS_HITS);
}

/// A negative answer replayed from the memo.
#[inline]
pub fn hit_negative() {
    super::tally::bump_shared(&NEG_HITS);
}

/// A lookup the memo could not serve, which ran stock.
#[inline]
pub fn miss() {
    super::tally::bump_shared(&MISSES);
}

/// One cumulative line on the gauge's 60-second cadence, once anything ran.
pub fn emit_cumulative() {
    let pos = POS_HITS.get();
    let neg = NEG_HITS.get();
    let misses = MISSES.get();
    if pos | neg | misses == 0 {
        return;
    }
    log::debug!(
        target: super::tally::TARGET,
        "filecache: {pos} pos hits, {neg} neg hits, {misses} misses",
    );
}
