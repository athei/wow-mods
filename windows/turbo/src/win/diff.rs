//! Divergence reporting for compare mode (`wow_turbo_diff` builds).
//!
//! A hook that carries a `[diff]` `out`/`inout` table and is selected via
//! `WOW_TURBO_DIFF_ARM` runs in compare mode: its generated `*_diff` path calls
//! the hooked **original** on the live arguments (so the game proceeds on ground
//! truth), re-runs the Rust reimplementation against snapshots of the same
//! inputs, and reports here when the two outputs disagree. Hooks without a table
//! have no diff path — an armed run is silent unless a reimpl actually diverges.
//!
//! Reporting is rate-limited per hook so a divergence inside a hot per-frame
//! function cannot flood the log: the first few events log with full detail
//! (built lazily — the matched fast path formats nothing), after which only a
//! periodic running count is emitted.

// The reporting API is consumed by generated code: which functions are
// referenced depends on the manifest's diff annotations (modes, float policy,
// return types), so any of them may be momentarily unreferenced.
#![allow(dead_code)]

use core::sync::atomic::{AtomicU32, Ordering};

use crate::math::ulp;

/// Detailed-log budget per hook for output mismatches.
const MISMATCH_DETAIL: u32 = 8;
/// After the detail budget, log a mismatch running count every this many events.
const MISMATCH_EVERY: u32 = 1024;

/// Per-hook event counter; one generated `static` per diff-annotated entry.
pub struct Stats {
    events: AtomicU32,
    /// One-shot latch for [`dump_case`], which fires once and not once per call.
    ///
    /// A non-zero event count stays true on every *matching* call between the
    /// first and second divergence, so counting alone would turn a hot hook into
    /// a flood.
    dumped: ::core::sync::atomic::AtomicBool,
}

impl Stats {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            events: AtomicU32::new(0),
            dumped: ::core::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Count one event; returns its zero-based ordinal.
    fn tick(&self) -> u32 {
        self.events.fetch_add(1, Ordering::Relaxed)
    }
}

impl Default for Stats {
    fn default() -> Self {
        Self::new()
    }
}

/// Stack scratch for input/output snapshots.
///
/// Byte storage over-aligned to 16 so a kernel reading the snapshot through any
/// lane type (`f32`/`u32`) stays aligned regardless of the region's layout.
#[repr(C, align(16))]
pub struct Buf<const N: usize>(pub [u8; N]);

impl<const N: usize> Buf<N> {
    #[must_use]
    pub const fn zeroed() -> Self {
        Self([0; N])
    }
}

/// Report a mismatch event.
///
/// Detailed for the first [`MISMATCH_DETAIL`] events, then a periodic count.
/// `expected = true` (a hook documented as deliberately more precise than the
/// original) logs at debug instead of warn so the operator's warn stream stays
/// actionable.
fn report(stats: &Stats, label: &str, expected: bool, detail: impl FnOnce() -> String) {
    let n = stats.tick();
    if n < MISMATCH_DETAIL {
        if expected {
            log::debug!(target: super::LOG_TARGET, "[diff] {label} diverged (expected): {}", detail());
        } else {
            log::warn!(target: super::LOG_TARGET, "[diff] {label} diverged: {}", detail());
        }
    } else if (n + 1).is_multiple_of(MISMATCH_EVERY) {
        if expected {
            log::debug!(target: super::LOG_TARGET, "[diff] {label}: {} divergences so far (expected)", n + 1);
        } else {
            log::warn!(target: super::LOG_TARGET, "[diff] {label}: {} divergences so far", n + 1);
        }
    }
}

/// One-shot per hook.
///
/// On the first ARMED call (diff selector set AND the hook reached), log that
/// the compare path is live. Without it an armed-but-faithful hook is
/// indistinguishable from one that never ran or was never armed. Subsequent
/// calls are a cheap already-set atomic load.
/// `delegates = true` says the adapter runs the original on some inputs, so a
/// clean result for this hook is not evidence of anything: on a delegated call
/// both sides of the compare are the same machine code and match by
/// construction. Nothing at runtime tells the two kinds of call apart, so the
/// caveat is stated once, here, rather than implied by silence.
pub fn note_armed(armed: &::core::sync::atomic::AtomicBool, label: &str, delegates: bool) {
    if !armed.swap(true, ::core::sync::atomic::Ordering::Relaxed) {
        if delegates {
            log::info!(
                target: super::LOG_TARGET,
                "[diff] armed: {label} (delegating — matches may be vacuous, not coverage)"
            );
        } else {
            log::info!(target: super::LOG_TARGET, "[diff] armed: {label}");
        }
    }
}

/// Print the operands and every differing lane on a hook's first divergence.
///
/// The region comparators name the first byte or lane that disagrees, which is
/// enough to classify a rounding but not enough to *reproduce* one. A divergence
/// that survives every static reading of the bytes — matching term order, seed,
/// element mapping and widths, with the inputs proven not to move — can only be
/// pinned down by replaying the operands that produced it, and until now nothing
/// recorded them. `C44Matrix::Multiply` sat in exactly that state.
///
/// Called after the region compare, so a non-zero event count means the divergence
/// this is about to describe is the one that just happened. Latched, so a hot
/// per-frame hook pays one hex dump per session and an atomic load thereafter.
pub fn dump_case(
    stats: &Stats,
    label: &str,
    expected: bool,
    ins: &[(usize, &[u8])],
    ours: &[u8],
    orig: &[u8],
) {
    // A divergence must have happened (events was ticked by the compare just
    // above) and this must be the first dump for this hook.
    if stats.events.load(Ordering::Relaxed) == 0 || stats.dumped.swap(true, Ordering::Relaxed) {
        return;
    }
    for (arg, bytes) in ins {
        emit_case(
            expected,
            format_args!("{label} case: arg{arg} = {}", Hex(bytes)),
        );
    }
    emit_case(
        expected,
        format_args!("{label} case: out ours = {}", Hex(ours)),
    );
    emit_case(
        expected,
        format_args!("{label} case: out orig = {}", Hex(orig)),
    );

    // Per-lane, so a signed zero or a one-ulp step is readable without counting
    // hex digits. Only the lanes that differ: the rest is already above.
    if ours.len() == orig.len() && ours.len().is_multiple_of(4) {
        for (lane, (a, b)) in ours.chunks_exact(4).zip(orig.chunks_exact(4)).enumerate() {
            let (ab, bb) = (
                u32::from_le_bytes([a[0], a[1], a[2], a[3]]),
                u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            );
            if ab != bb {
                emit_case(
                    expected,
                    format_args!(
                        "{label} case: lane {lane} ours {ab:#010x} ({}) orig {bb:#010x} ({})",
                        f32::from_bits(ab),
                        f32::from_bits(bb)
                    ),
                );
            }
        }
    }
}

/// Route a case line to the same level the entry's divergences use.
fn emit_case(expected: bool, args: core::fmt::Arguments) {
    if expected {
        log::debug!(target: super::LOG_TARGET, "[diff] {args}");
    } else {
        log::warn!(target: super::LOG_TARGET, "[diff] {args}");
    }
}

/// Lowercase hex of a byte slice, no separators, written without allocating.
struct Hex<'a>(&'a [u8]);

impl core::fmt::Display for Hex<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

/// Compare an output region lane-by-lane as little-endian `f32`s within `max_ulp`.
///
/// Report the first diverging lane.
pub fn region_f32(
    stats: &Stats,
    label: &str,
    expected: bool,
    max_ulp: u32,
    ours: &[u8],
    orig: &[u8],
) {
    let Some(d) = ulp::first_divergence_f32(ours, orig, max_ulp) else {
        return;
    };
    report(stats, label, expected, || {
        format!(
            "out lane {}: ours {:?} ({:#010x}) vs orig {:?} ({:#010x})",
            d.lane,
            f32::from_bits(d.ours),
            d.ours,
            f32::from_bits(d.orig),
            d.orig,
        )
    });
}

/// Compare a region whose head is raw bytes and whose tail is `f32` lanes.
///
/// For an output region that is not uniformly float — a game object where only
/// a trailing span holds the floats a kernel computes, with a vtable pointer and
/// flag bytes ahead of it. The head is compared exactly, so nothing there is
/// weakened; only the tail takes the `max_ulp` tolerance. Comparing such a
/// region wholly as lanes would be worse than useless: an arbitrary pointer can
/// carry a NaN bit pattern, and any two NaNs compare equal regardless of
/// payload, so two *different* pointers would pass.
///
/// `float_from` is a byte offset into the region and is validated at build time
/// to be within it and 4-byte aligned.
pub fn region_split(
    stats: &Stats,
    label: &str,
    expected: bool,
    max_ulp: u32,
    float_from: usize,
    ours: &[u8],
    orig: &[u8],
) {
    if let Some(at) = ulp::first_divergence_bytes(&ours[..float_from], &orig[..float_from]) {
        report(stats, label, expected, || {
            format!(
                "out byte {at}: ours {:#04x} vs orig {:#04x}",
                ours[at], orig[at],
            )
        });
        return;
    }
    let Some(d) = ulp::first_divergence_f32(&ours[float_from..], &orig[float_from..], max_ulp)
    else {
        return;
    };
    report(stats, label, expected, || {
        format!(
            "out lane {} (byte {}): ours {:?} ({:#010x}) vs orig {:?} ({:#010x})",
            d.lane,
            float_from + d.lane * 4,
            f32::from_bits(d.ours),
            d.ours,
            f32::from_bits(d.orig),
            d.orig,
        )
    });
}

/// Report an input region that changed while the compare was running.
///
/// The compare path snapshots each `ins` region, runs the original on the live
/// arguments, then replays the reimplementation against the snapshots. That is
/// only a fair comparison while the live region still holds what was
/// snapshotted. If it does not, the two sides were handed different inputs and
/// **every divergence the entry reports is unattributable** — it says nothing
/// about whether the reimplementation matches.
///
/// Nothing in the harness could distinguish that from a real arithmetic
/// difference before this existed, which is exactly how a reimplementation
/// verified from the bytes can still be reported as diverging.
pub fn note_input_drift(stats: &Stats, label: &str, arg: usize, snapshot: &[u8], live: &[u8]) {
    let Some(at) = ulp::first_divergence_bytes(snapshot, live) else {
        return;
    };
    report(stats, label, false, || {
        format!(
            "INPUT DRIFT: arg{arg} byte {at} changed under the compare \
             (snapshot {:#04x}, now {:#04x}) — divergences for this entry are \
             not attributable to the reimplementation",
            snapshot[at], live[at],
        )
    });
}

/// Compare an output region byte-exactly; report the first diverging offset.
pub fn region_bytes(stats: &Stats, label: &str, expected: bool, ours: &[u8], orig: &[u8]) {
    let Some(at) = ulp::first_divergence_bytes(ours, orig) else {
        return;
    };
    report(stats, label, expected, || {
        format!(
            "out byte {at}: ours {:#04x} vs orig {:#04x}",
            ours[at], orig[at],
        )
    });
}

/// Compare scalar `f32` returns within `max_ulp`.
pub fn scalar_f32(stats: &Stats, label: &str, expected: bool, max_ulp: u32, ours: f32, orig: f32) {
    if ulp::f32_within_ulp(ours, orig, max_ulp) {
        return;
    }
    report(stats, label, expected, || {
        format!(
            "ret: ours {ours:?} ({:#010x}) vs orig {orig:?} ({:#010x})",
            ours.to_bits(),
            orig.to_bits(),
        )
    });
}

/// Compare integer returns over `mask` (widened to `u64` by the generated caller).
///
/// `mask` is every bit unless the manifest narrowed it to the bits the original's
/// callers actually read; see the `ret_mask` annotation. The report prints the raw
/// values as well as the masked ones, so a narrowed comparison still shows what
/// the two sides really returned rather than hiding it.
pub fn scalar_int(stats: &Stats, label: &str, expected: bool, mask: u64, ours: u64, orig: u64) {
    if ours & mask == orig & mask {
        return;
    }
    report(stats, label, expected, || {
        if mask == u64::MAX {
            format!("ret: ours {ours:#x} vs orig {orig:#x}")
        } else {
            format!(
                "ret: ours {:#x} vs orig {:#x} (masked {mask:#x}; raw ours {ours:#x} vs orig {orig:#x})",
                ours & mask,
                orig & mask
            )
        }
    });
}
