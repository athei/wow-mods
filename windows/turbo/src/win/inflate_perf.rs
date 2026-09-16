//! MPQ decoder usage and cost, compiled away outside `PERF=1` builds.
//!
//! Asset-loading workers can write every counter here. Totals use atomic adds
//! and are never reset. A report reads each total separately, so a decode in
//! flight can straddle its fields. Timing sums are work across threads, not
//! elapsed loading time, and different input mixes make the libdeflate and
//! stock times unsuitable as a direct speed comparison.

use super::tally::{Armed, SharedAccum};

static ATTEMPTS: SharedAccum = SharedAccum::zero();
static SUCCESSES: SharedAccum = SharedAccum::zero();
static INPUT_BYTES: SharedAccum = SharedAccum::zero();
static OUTPUT_BYTES: SharedAccum = SharedAccum::zero();
static ATTEMPT_TICKS: SharedAccum = SharedAccum::zero();
static STOCK_CALLS: SharedAccum = SharedAccum::zero();
static STOCK_FAILURES: SharedAccum = SharedAccum::zero();
static STOCK_TICKS: SharedAccum = SharedAccum::zero();

/// One eligible zlib attempt, including TLS access and lazy decoder allocation.
///
/// `input_bytes` excludes the MPQ mask byte. Output bytes count only successful
/// decodes; failed attempts still contribute input bytes and time.
#[inline]
pub fn attempt(armed: &Armed, input_bytes: usize, output: Option<usize>, ticks: u64) {
    ATTEMPTS.add(armed, 1);
    INPUT_BYTES.add(armed, input_bytes as u64);
    ATTEMPT_TICKS.add(armed, ticks);
    if let Some(bytes) = output {
        SUCCESSES.add(armed, 1);
        OUTPUT_BYTES.add(armed, bytes as u64);
    }
}

/// One completed stock fallback, timed without fallback logging or bookkeeping.
#[inline]
pub fn stock(armed: &Armed, result: u32, ticks: u64) {
    STOCK_CALLS.add(armed, 1);
    STOCK_TICKS.add(armed, ticks);
    if result == 0 {
        STOCK_FAILURES.add(armed, 1);
    }
}

/// Session totals on the diagnostic layer's 60-second cadence, once used.
pub fn emit_cumulative() {
    let attempts = ATTEMPTS.get();
    let stock_calls = STOCK_CALLS.get();
    if attempts | stock_calls == 0 {
        return;
    }
    log::info!(
        target: super::tally::TARGET,
        "mpq-inflate: libdeflate_attempts={attempts} libdeflate_ok={} \
         zlib_input_bytes={} decoded_bytes={} libdeflate_ms={} \
         stock_calls={stock_calls} stock_failed={} stock_ms={}",
        SUCCESSES.get(),
        INPUT_BYTES.get(),
        OUTPUT_BYTES.get(),
        super::hooks::clock_ticks_to_ms(ATTEMPT_TICKS.get()),
        STOCK_FAILURES.get(),
        super::hooks::clock_ticks_to_ms(STOCK_TICKS.get()),
    );
}
