//! TSC-based timing helper.
//!
//! Compiles to a single `rdtsc` instruction on both PE targets (i386 and
//! `x86_64`), bypassing any Wine-side `QueryPerformanceCounter` translation.
//! Under Rosetta 2 on Apple Silicon the instruction is trapped to the ARM
//! generic timer; cost stays near-native (~10–30 cycles). Native host
//! (aarch64) test/lint builds — which never ship — read `CNTVCT_EL0`
//! directly instead of going through Rosetta.
//!
//! Returns raw TSC cycles, not nanoseconds. TSC is invariant on Nehalem+
//! so a single runtime calibration against `Instant` yields a
//! process-lifetime Hz value. Under Rosetta the emulated ARM generic
//! timer is likewise fixed-frequency (~945 MHz in practice, NOT CPU
//! clock), which is why the calibration is runtime-determined instead of
//! hard-coded.

use std::{
    sync::LazyLock,
    thread,
    time::{Duration, Instant},
};

use log::info;

/// Calibration message rides on the perf log target.
///
/// TSC is a perf-only primitive across this workspace.
const LOG_TARGET: &str = "wow::perf";

/// Read the timestamp / cycle counter.
///
/// Single-instruction primitive on the x86 PE targets (`rdtsc`); `#[inline]`
/// lets a measurement bracket compile to a pair of reads in release. Native
/// host (aarch64) test/lint builds read the ARM generic timer `CNTVCT_EL0`
/// instead — shipped Wine code always runs an x86 path.
#[inline]
#[must_use]
pub fn rdtsc() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: RDTSC is unprivileged on every x86_64 mode we run in
        // (Wine PE process). No preconditions; returns the cycle counter.
        unsafe { core::arch::x86_64::_rdtsc() }
    }
    #[cfg(target_arch = "x86")]
    {
        // SAFETY: RDTSC is unprivileged on every x86 mode we run in
        // (Wine PE process). No preconditions; returns the cycle counter.
        unsafe { core::arch::x86::_rdtsc() }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // Native host (test/lint) builds only — shipped Wine code always runs
        // an x86 path above. CNTVCT_EL0 is the ARM generic-timer counter, the
        // aarch64 analogue of the TSC; `calibrate()` derives its Hz at runtime.
        let cnt: u64;
        // SAFETY: CNTVCT_EL0 is readable from EL0 on macOS arm64 (the kernel
        // enables EL0VCTEN); the read has no preconditions and no side effects.
        unsafe {
            core::arch::asm!("mrs {}, cntvct_el0", out(reg) cnt, options(nomem, nostack, preserves_flags));
        }
        cnt
    }
}

/// Return the calibrated TSC frequency in Hz.
///
/// The first call blocks for the 50 ms calibration sleep; subsequent calls
/// are a single atomic load. Intended to be warmed from a background thread
/// at `DllMain` so the hot path never pays the sleep.
#[must_use]
pub fn tsc_hz() -> u64 {
    *TSC_HZ
}

/// Lossless u64 → f64 reconstruction via hi/lo u32 split + fused MAC.
///
/// Each `f64::from(u32)` is exact (u32 ⊂ f64 mantissa); `2^32` is exactly
/// representable. The fused MAC composes them without the truncating
/// `u64 as f64` direct cast that clippy rejects. Values exceeding 2^53
/// still incur precision loss inside the multiply, but no
/// `cast_precision_loss` lint fires because every cast is exact.
///
/// # Panics
///
/// The two `try_from` calls are statically unreachable: `v >> 32` and
/// `v & 0xFFFF_FFFF` both fit u32 by construction.
#[inline]
#[must_use]
// Plain mul/add, not `mul_add`: on i686 (no FMA) `mul_add` is an fmaf/fma
// LIBCALL. Bit-identical here anyway — hi·2^32 is exact (power-of-two scale),
// so the sum's single rounding matches the fused form.
pub fn u64_to_f64_exact(v: u64) -> f64 {
    let hi = u32::try_from(v >> 32).expect("u64 >> 32 fits u32");
    let lo = u32::try_from(v & 0xFFFF_FFFF).expect("u64 masked to 32 bits fits u32");
    f64::from(hi) * 4_294_967_296.0 + f64::from(lo)
}

/// Lossless `usize` → f64 via [`u64_to_f64_exact`].
///
/// `usize` is ≤ u64 on every platform we target, so the widening cast is exact.
///
/// # Panics
///
/// Unreachable on supported targets (`x86_64-apple-darwin`,
/// `aarch64-apple-darwin`, `i686-pc-windows-msvc`) — usize is at most 64 bits
/// everywhere.
#[inline]
#[must_use]
pub fn usize_to_f64_exact(v: usize) -> f64 {
    u64_to_f64_exact(u64::try_from(v).expect("usize fits u64 on all supported targets"))
}

/// Return the TSC cycle count that corresponds to the given wall-clock duration in seconds.
///
/// Used to build rate-limit thresholds.
#[inline]
#[must_use]
pub fn secs_to_cycles(seconds: u64) -> u64 {
    tsc_hz().saturating_mul(seconds)
}

/// Return the cost of one counter read, in THOUSANDTHS of a tick.
///
/// The counterpart to [`tsc_hz`]: that says what a tick is worth, this says
/// what it costs to ask. A measurement bracket is two reads, so every span a
/// caller times is inflated by one read's latency; on a span of milliseconds
/// that rounds away, but on spans of a fraction of a microsecond taken millions
/// of times a second it is percent-scale, and a caller that wants to subtract
/// it needs a number rather than a guess.
///
/// Thousandths because the cost is a FRACTION of a tick under Rosetta, where
/// the counter is a scaled ARM timer: it advances in steps far larger than one
/// nominal tick, so a whole-tick figure would round to nothing. The first call
/// runs the calibration (about a millisecond); force it off the hot path.
#[must_use]
pub fn read_cost_milli_ticks() -> u64 {
    *READ_COST
}

static TSC_HZ: LazyLock<u64> = LazyLock::new(calibrate);
static READ_COST: LazyLock<u64> = LazyLock::new(calibrate_read_cost);

/// Time a long run of reads, because a single pair cannot resolve one.
///
/// Under Rosetta the counter advances in steps much larger than one nominal
/// tick, so back-to-back reads usually return the same value and the median
/// pair delta is zero — a real per-read cost reading as no cost at all. Timing
/// thousands of reads and dividing recovers the fraction. The best of several
/// runs is taken because a run that was descheduled part-way through can only
/// ever overstate the result, and overstating it means over-subtracting from
/// every span a caller corrects.
fn calibrate_read_cost() -> u64 {
    const RUNS: usize = 8;
    const READS: u64 = 4_096;
    let mut best = u64::MAX;
    for _ in 0..RUNS {
        let start = rdtsc();
        for _ in 0..READS {
            std::hint::black_box(rdtsc());
        }
        let spent = rdtsc().wrapping_sub(start);
        best = best.min(spent.saturating_mul(1_000) / READS);
    }
    info!(
        target: LOG_TARGET,
        "tsc read cost: {}.{:03} ticks",
        best / 1_000,
        best % 1_000,
    );
    best
}

fn calibrate() -> u64 {
    const SLEEP: Duration = Duration::from_millis(50);
    let t0 = Instant::now();
    let c0 = rdtsc();
    thread::sleep(SLEEP);
    let c1 = rdtsc();
    let elapsed = t0.elapsed();
    let elapsed_us =
        u64::try_from(elapsed.as_micros()).expect("calibration sleep is 50 ms — elapsed fits u64");
    // Integer math: ticks/sec = ticks * 1_000_000 / elapsed_µs. SLEEP ≥1 µs by
    // construction, so the divisor is non-zero.
    let hz = (c1 - c0).saturating_mul(1_000_000) / elapsed_us.max(1);
    let mhz = u64_to_f64_exact(hz) / 1e6;
    info!(
        target: LOG_TARGET,
        "tsc calibrated: {hz} Hz ({mhz:.2} MHz) over {:.1} ms",
        elapsed.as_secs_f64() * 1e3,
    );
    hz
}

#[cfg(test)]
mod tests {
    use super::read_cost_milli_ticks;

    /// A read that measures as free is a broken estimator, not a free read.
    ///
    /// The first estimator here took the median of back-to-back pairs, which is
    /// zero on a stepped counter, so every caller correcting for the read cost
    /// silently corrected by nothing.
    #[test]
    fn a_counter_read_costs_something_measurable() {
        let cost = read_cost_milli_ticks();
        assert!(cost > 0, "a counter read cannot cost zero ticks");
        assert!(cost < 1_000_000, "{cost} milli-ticks is not a read cost");
    }
}
