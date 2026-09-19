//! Completed-lookup telemetry, without retaining any client pointer.

use super::super::tally::{Armed, SharedAccum, SharedCounter, TARGET};
use crate::object_lookup::{Probe, lookup_observed};

// Callers exclude manager mutation during traversal, but their thread identity
// is not established. Totals therefore use the multiple-writer discipline.
static HITS: [SharedCounter; 7] = [const { SharedCounter::zero() }; 7];
static MISSES: [SharedCounter; 7] = [const { SharedCounter::zero() }; 7];
static NODES: SharedAccum = SharedAccum::zero();
static HASH_MISSES: SharedAccum = SharedAccum::zero();
static LOW_MISSES: SharedAccum = SharedAccum::zero();
static HIGH_MISSES: SharedAccum = SharedAccum::zero();

pub fn lookup(
    armed: &Armed,
    manager: u32,
    low: u32,
    high: u32,
    read: impl FnMut(u32) -> u32,
) -> u32 {
    let mut nodes = 0u32;
    let mut hash_misses = 0u32;
    let mut low_misses = 0u32;
    let mut high_misses = 0u32;
    let result = lookup_observed(manager, low, high, read, |probe| {
        let count = match probe {
            Probe::Visit => &mut nodes,
            Probe::HashMiss => &mut hash_misses,
            Probe::LowMiss => &mut low_misses,
            Probe::HighMiss => &mut high_misses,
        };
        *count = count.wrapping_add(1);
    });
    let bin = match nodes {
        0 => 0,
        1 => 1,
        2..=3 => 2,
        4..=7 => 3,
        8..=15 => 4,
        16..=31 => 5,
        _ => 6,
    };
    let outcomes = if result == 0 { &MISSES } else { &HITS };
    outcomes[bin].bump(armed);
    for (counter, count) in [
        (&NODES, nodes),
        (&HASH_MISSES, hash_misses),
        (&LOW_MISSES, low_misses),
        (&HIGH_MISSES, high_misses),
    ] {
        if count != 0 {
            counter.add(armed, u64::from(count));
        }
    }
    result
}

pub fn emit_cumulative() {
    let hits = HITS.each_ref().map(SharedCounter::get);
    let misses = MISSES.each_ref().map(SharedCounter::get);
    if hits.iter().chain(&misses).all(|&count| count == 0) {
        return;
    }
    let nodes = NODES.get();
    let hash_misses = HASH_MISSES.get();
    let low_misses = LOW_MISSES.get();
    let high_misses = HIGH_MISSES.get();
    crate::defer_log!(target: TARGET, log::Level::Info,
        "guid-lookup: hit_bins={hits:?} miss_bins={misses:?} nodes={nodes} \
         reject_hash={hash_misses} reject_low={low_misses} reject_high={high_misses}");
}
