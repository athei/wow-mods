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
}

impl Stats {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            events: AtomicU32::new(0),
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
pub fn note_armed(armed: &::core::sync::atomic::AtomicBool, label: &str) {
    if !armed.swap(true, ::core::sync::atomic::Ordering::Relaxed) {
        log::info!(target: super::LOG_TARGET, "[diff] armed: {label}");
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

/// Compare integer returns exactly (widened to `u64` by the generated caller).
pub fn scalar_int(stats: &Stats, label: &str, expected: bool, ours: u64, orig: u64) {
    if ours == orig {
        return;
    }
    report(stats, label, expected, || {
        format!("ret: ours {ours:#x} vs orig {orig:#x}")
    });
}
