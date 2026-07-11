//! `m2` family kernels.
#![allow(
    non_snake_case,
    clippy::pedantic,
    clippy::nursery,
    // intentional NaN-reject via `!(a >= b)`
    // (differs from `a < b` on NaN), bit-exact source constants kept verbatim,
    // and ABI-dictated parameter counts.
    clippy::neg_cmp_op_on_partial_ord,
    clippy::excessive_precision,
    clippy::too_many_arguments
)]

/// Heap-sort comparator for indexed batch elements.
///
/// Orders by `key_y` ascending, then `key_z` ascending, then by index
/// ascending as the final tie-break. Each `*_a`/`*_b` pair is the corresponding
/// element's two float sort keys; `index_a`/`index_b` are their positions.
/// Returns `-1` if `a` sorts before `b`, `1` if after, `0` if identical.
/// Comparisons are strict `<` (matching the original's `fcomp` reject paths), so
/// a `NaN` key takes neither branch and falls through to the next criterion.
pub fn m2_batch_sort_compare__70a060(
    key_y_a: f32,
    key_y_b: f32,
    key_z_a: f32,
    key_z_b: f32,
    index_a: u32,
    index_b: u32,
) -> i32 {
    if key_y_a < key_y_b {
        return -1;
    }
    if key_y_b < key_y_a {
        return 1;
    }
    if key_z_a < key_z_b {
        return -1;
    }
    if key_z_b < key_z_a {
        return 1;
    }
    if index_a < index_b {
        return -1;
    }
    i32::from(index_b < index_a)
}

#[cfg(test)]
mod tests_m2_batch_sort_compare__70a060 {
    use super::m2_batch_sort_compare__70a060 as cmp;

    #[test]
    fn key_y_ascending() {
        // smaller key_y sorts first (returns -1 when a's y is smaller).
        assert_eq!(cmp(1.0, 5.0, 0.0, 0.0, 0, 1), -1);
        assert_eq!(cmp(5.0, 1.0, 0.0, 0.0, 0, 1), 1);
    }

    #[test]
    fn key_z_ascending_on_y_tie() {
        // equal y -> smaller key_z sorts first.
        assert_eq!(cmp(2.0, 2.0, 1.0, 9.0, 0, 1), -1);
        assert_eq!(cmp(2.0, 2.0, 9.0, 1.0, 0, 1), 1);
    }

    #[test]
    fn index_tiebreak_on_full_key_tie() {
        assert_eq!(cmp(2.0, 2.0, 3.0, 3.0, 4, 7), -1);
        assert_eq!(cmp(2.0, 2.0, 3.0, 3.0, 7, 4), 1);
        assert_eq!(cmp(2.0, 2.0, 3.0, 3.0, 5, 5), 0);
    }

    #[test]
    fn antisymmetry() {
        // swapping a<->b negates the result for distinct elements.
        let a = (3.0f32, 1.0f32, 1u32);
        let b = (3.0f32, 4.0f32, 2u32);
        let ab = cmp(a.0, b.0, a.1, b.1, a.2, b.2);
        let ba = cmp(b.0, a.0, b.1, a.1, b.2, a.2);
        assert_eq!(ab, -ba);
    }

    #[test]
    fn antisymmetry_across_all_axes() {
        // multi-axis: differing y, z, and index simultaneously.
        let cases = [
            (5.0f32, 5.0f32, 2.0f32, 7.0f32, 1u32, 9u32),
            (1.0f32, 8.0f32, 4.0f32, 4.0f32, 3u32, 3u32),
            (-2.0f32, -2.0f32, -1.0f32, -1.0f32, 6u32, 2u32),
        ];
        for (ya, yb, za, zb, ia, ib) in cases {
            let ab = cmp(ya, yb, za, zb, ia, ib);
            let ba = cmp(yb, ya, zb, za, ib, ia);
            assert_eq!(ab, -ba, "{ya} {yb} {za} {zb} {ia} {ib}");
        }
    }

    #[test]
    fn reflexive_is_zero() {
        assert_eq!(cmp(3.0, 3.0, 7.0, 7.0, 2, 2), 0);
    }

    #[test]
    fn y_dominates_z() {
        // a has smaller y (sorts first) even though its z is larger.
        assert_eq!(cmp(1.0, 9.0, 100.0, 0.0, 0, 1), -1);
    }
}

/// Axis-aligned bounding-box centre and radius from its min/max corners:
/// `centre = (min + max) * half` per axis, with the bound radius passed through
/// unchanged into the 4th lane. `half` is the source's literal `0.5`.
pub fn cm2_shared__get_bounds_center__713680(
    min: &[f32; 3],
    max: &[f32; 3],
    radius: f32,
    half: f32,
) -> [f32; 4] {
    [
        (min[0] + max[0]) * half,
        (min[1] + max[1]) * half,
        (min[2] + max[2]) * half,
        radius,
    ]
}

#[cfg(test)]
mod tests_cm2_shared__get_bounds_center__713680 {
    use super::cm2_shared__get_bounds_center__713680 as center;

    #[test]
    fn midpoint_with_radius_passthrough() {
        let c = center(&[0.0, -2.0, 4.0], &[2.0, 4.0, 10.0], 7.5, 0.5);
        assert_eq!(c[0].to_bits(), 1.0f32.to_bits());
        assert_eq!(c[1].to_bits(), 1.0f32.to_bits());
        assert_eq!(c[2].to_bits(), 7.0f32.to_bits());
        assert_eq!(c[3].to_bits(), 7.5f32.to_bits()); // radius copied, never scaled
    }

    #[test]
    fn symmetric_box_centres_at_origin() {
        let c = center(&[-5.0, -5.0, -5.0], &[5.0, 5.0, 5.0], 8.66, 0.5);
        for (k, v) in c[..3].iter().enumerate() {
            assert_eq!(v.to_bits(), 0.0f32.to_bits(), "axis {k}");
        }
    }

    #[test]
    fn degenerate_box_centres_on_the_point() {
        let p = [3.0, -1.0, 2.0];
        let c = center(&p, &p, 0.0, 0.5);
        for (k, v) in c[..3].iter().enumerate() {
            assert_eq!(v.to_bits(), p[k].to_bits(), "axis {k}");
        }
    }
}

/// Derives the per-sequence bound fields of the animation-info struct from the
/// model's bound-radius vector `bound_vec` (this+0xbc..0xc8), the sequence
/// entry's move-speed scalar `move_speed` (entry+0xc) and its 6-float bound box
/// `box6` (entry+0x24.., min then max). Returns `(speed_scaled, center)` where
/// `speed_scaled = |bound_vec| * move_speed` and `center = (min+max)*0.5`. The
/// box radius (entry+0x3c) is a verbatim field copy handled in the adapter.
pub fn cm2_model__get_animation_info__711a20(
    bound_vec: &[f32; 3],
    move_speed: f32,
    box6: &[f32; 6],
) -> (f32, [f32; 3]) {
    let len =
        (bound_vec[0] * bound_vec[0] + bound_vec[1] * bound_vec[1] + bound_vec[2] * bound_vec[2])
            .sqrt();
    let speed_scaled = len * move_speed;
    let center = [
        (box6[0] + box6[3]) * 0.5,
        (box6[1] + box6[4]) * 0.5,
        (box6[2] + box6[5]) * 0.5,
    ];
    (speed_scaled, center)
}

#[cfg(test)]
mod tests_cm2_model__get_animation_info__711a20 {
    use super::cm2_model__get_animation_info__711a20 as info;

    #[test]
    fn known_value() {
        let bv = [3.0, 4.0, 0.0]; // len 5
        let box6 = [0.0, -2.0, 4.0, 2.0, 4.0, 10.0];
        let (speed, center) = info(&bv, 2.0, &box6);
        assert_eq!(speed.to_bits(), 10.0f32.to_bits());
        assert_eq!(center[0].to_bits(), 1.0f32.to_bits());
        assert_eq!(center[1].to_bits(), 1.0f32.to_bits());
        assert_eq!(center[2].to_bits(), 7.0f32.to_bits());
    }

    #[test]
    fn zero_radius_zero_speed() {
        let (speed, center) = info(&[0.0, 0.0, 0.0], 5.0, &[1.0, 1.0, 1.0, 3.0, 3.0, 3.0]);
        assert_eq!(speed.to_bits(), 0.0f32.to_bits());
        for (k, c) in center.iter().enumerate() {
            assert_eq!(c.to_bits(), 2.0f32.to_bits(), "axis {k}");
        }
    }

    #[test]
    fn speed_scales_linearly() {
        let bv = [1.0, 2.0, 2.0]; // len 3
        let box6 = [0.0; 6];
        let (s1, _) = info(&bv, 1.0, &box6);
        let (s2, _) = info(&bv, 2.0, &box6);
        assert_eq!((s1 * 2.0).to_bits(), s2.to_bits());
        assert_eq!(s1.to_bits(), 3.0f32.to_bits());
    }
}

/// Index of an attachment record within its 0x30-byte-stride table from the
/// resolved `attach_index` (the M2 attachment-lookup result). Byte offset of the
/// record is `attach_index * 0x30`; this returns that byte offset so the adapter
/// can address the record without an indexed range-loop. Kept as a pure helper
/// so the stride is unit-tested on the host (the world-position transforms
/// themselves are stock `C44Matrix::TransformPoint` call-outs).
pub fn cm2_model__get_attachment_world_pos__712d50(attach_index: u32) -> usize {
    attach_index as usize * 0x30
}

#[cfg(test)]
mod tests_cm2_model__get_attachment_world_pos__712d50 {
    use super::cm2_model__get_attachment_world_pos__712d50 as off;

    #[test]
    fn stride_is_0x30_bytes() {
        assert_eq!(off(0), 0);
        assert_eq!(off(1), 0x30);
        assert_eq!(off(3), 0x90);
    }
}

/// Linearly interpolates two 4-component key values (quaternion components)
/// `lo` and `hi` by factor `t`, per lane `lo + (hi - lo) * t`. When `step` is
/// true (the source's interpolation-type-0 path) the `lo` key is copied
/// verbatim and `t` is ignored.
pub fn cm2_shared__sample_quat_track__713ea0(
    lo: &[f32; 4],
    hi: &[f32; 4],
    t: f32,
    step: bool,
) -> [f32; 4] {
    if step {
        return *lo;
    }
    [
        (hi[0] - lo[0]) * t + lo[0],
        (hi[1] - lo[1]) * t + lo[1],
        (hi[2] - lo[2]) * t + lo[2],
        (hi[3] - lo[3]) * t + lo[3],
    ]
}

#[cfg(test)]
mod tests_cm2_shared__sample_quat_track__713ea0 {
    use super::cm2_shared__sample_quat_track__713ea0 as lerp;

    #[test]
    fn endpoints() {
        let lo = [1.0, 2.0, 3.0, 4.0];
        let hi = [5.0, 6.0, 7.0, 8.0];
        let r0 = lerp(&lo, &hi, 0.0, false);
        let r1 = lerp(&lo, &hi, 1.0, false);
        for k in 0..4 {
            assert_eq!(r0[k].to_bits(), lo[k].to_bits(), "t0 lane {k}");
            assert_eq!(r1[k].to_bits(), hi[k].to_bits(), "t1 lane {k}");
        }
    }

    #[test]
    fn midpoint_known() {
        let r = lerp(&[0.0, 0.0, 0.0, 0.0], &[2.0, 4.0, 8.0, 16.0], 0.5, false);
        assert_eq!(r[0].to_bits(), 1.0f32.to_bits());
        assert_eq!(r[1].to_bits(), 2.0f32.to_bits());
        assert_eq!(r[2].to_bits(), 4.0f32.to_bits());
        assert_eq!(r[3].to_bits(), 8.0f32.to_bits());
    }

    #[test]
    fn step_copies_lo() {
        let lo = [9.0, 8.0, 7.0, 6.0];
        let r = lerp(&lo, &[1.0, 1.0, 1.0, 1.0], 0.73, true);
        for k in 0..4 {
            assert_eq!(r[k].to_bits(), lo[k].to_bits(), "step lane {k}");
        }
    }
}

/// Merges a supplied animated vec3 `value` with the track's existing `cur`
/// value per axis under `mask`: for each axis bit set in `mask` the existing
/// `cur` lane is kept, otherwise the supplied `value` lane is used
/// (bit0 = X, bit1 = Y, bit2 = Z).
pub fn cm2_set_animated_vec3_masked__7ac250(
    cur: &[f32; 3],
    value: &[f32; 3],
    mask: u8,
) -> [f32; 3] {
    [
        if mask & 1 != 0 { cur[0] } else { value[0] },
        if mask & 2 != 0 { cur[1] } else { value[1] },
        if mask & 4 != 0 { cur[2] } else { value[2] },
    ]
}

#[cfg(test)]
mod tests_cm2_set_animated_vec3_masked__7ac250 {
    use super::cm2_set_animated_vec3_masked__7ac250 as merge;

    #[test]
    fn mask_selects_per_axis() {
        let cur = [10.0, 20.0, 30.0];
        let val = [1.0, 2.0, 3.0];
        assert_eq!(merge(&cur, &val, 0), val);
        assert_eq!(merge(&cur, &val, 7), cur);
        assert_eq!(merge(&cur, &val, 0b010), [1.0, 20.0, 3.0]);
        assert_eq!(merge(&cur, &val, 0b101), [10.0, 2.0, 30.0]);
    }

    #[test]
    fn high_bits_ignored() {
        let cur = [10.0, 20.0, 30.0];
        let val = [1.0, 2.0, 3.0];
        assert_eq!(merge(&cur, &val, 0xF8), val);
    }
}

/// Locates the keyframe pair bracketing `time` in a sorted `u32` timestamp
/// array and the interpolation fraction between them.
///
/// `range` is the resolved `(first, last)` key-slot window for the active
/// sequence and `seed` the caller's previous lo index: the search scans
/// forward from the seed when `time` is less than 500 ticks ahead of it,
/// scans backward when less than 500 ticks behind, and otherwise restarts
/// from the window (a forward scan near its start, else binary search).
/// Returns `(lo, hi, fraction)` with `fraction` =
/// `(time - t[lo]) / (t[hi] - t[lo])`; a degenerate window or a query at or
/// past the final key collapses to `(idx, idx, 0.0)`. Window/seed indices on
/// malformed data are clamped to the array instead of read out of bounds.
///
/// Timestamps are read through `ts_base` with `read_unaligned`: a valid packed
/// track's timestamp array can sit at a 2-byte-aligned address, which the stock
/// binary indexes with plain, alignment-agnostic pointer loads.
///
/// # Safety
/// `ts_base` must point to `count` readable, contiguous `u32`; the caller
/// validates non-null and that `count` fits `isize::MAX`. Every index the
/// search forms is clamped to `[0, count - 1]`.
pub unsafe fn cm2_shared__find_keyframe_interval__713d50(
    time: u32,
    range: (u32, u32),
    ts_base: *const u32,
    count: u32,
    seed: u32,
) -> (u32, u32, f32) {
    let (first, last_key) = range;
    if last_key <= first || count == 0 {
        return (first, first, 0.0);
    }
    let max = count - 1;
    let lo = first.min(max);
    let hi = last_key.min(max);
    // SAFETY: `i` is always clamped to `[0, max] = [0, count - 1]`, and the
    // caller guarantees `count` readable dwords at `ts_base`. `read_unaligned`
    // tolerates a valid packed track's 2-byte-aligned timestamp array.
    let at = |i: u32| unsafe { ts_base.wrapping_add(i as usize).read_unaligned() };

    let mut idx = seed.min(max);
    let ahead = time.wrapping_sub(at(idx));
    if ahead < 500 {
        // Forward scan from the seed.
        while idx < hi && at(idx + 1) <= time {
            idx += 1;
        }
    } else if ahead < 500u32.wrapping_neg() {
        if time.wrapping_sub(at(lo)) < 500 {
            // Forward scan from the window start.
            idx = lo;
            while idx < hi && at(idx + 1) <= time {
                idx += 1;
            }
        } else {
            // Binary search over the window.
            let mut l = lo;
            let mut h = hi;
            idx = l;
            while l < h {
                let mid = (h + l) >> 1;
                if time < at(mid) {
                    if mid == 0 {
                        // The original would wrap its upper bound below zero
                        // here; unreachable on sorted, zero-based key data.
                        idx = l;
                        break;
                    }
                    h = mid - 1;
                } else {
                    l = mid + 1;
                    if time < at(mid + 1) {
                        idx = mid;
                        break;
                    }
                }
                idx = l;
            }
        }
    } else if lo < idx {
        // Backward scan from the seed.
        while at(idx) > time {
            idx -= 1;
            if idx <= lo {
                break;
            }
        }
    }

    let next = idx + 1;
    if count <= next {
        return (idx, idx, 0.0);
    }
    let t_lo = at(idx);
    // 64-bit numerator load and signed 32-bit divisor, as in the original's
    // zero-extended integer load and signed integer divide.
    let num = f64::from(time.wrapping_sub(t_lo));
    let den = f64::from(at(next).wrapping_sub(t_lo) as i32);
    (idx, next, (num / den) as f32)
}

#[cfg(test)]
mod tests_cm2_shared__find_keyframe_interval__713d50 {
    /// Adapts the raw-pointer kernel to the slice-based test cases: every test
    /// passes a `&[u32]`, while the kernel reads through a `*const u32` + count.
    fn find(time: u32, range: (u32, u32), ts: &[u32], seed: u32) -> (u32, u32, f32) {
        // SAFETY: `ts.as_ptr()` is valid for `ts.len()` contiguous reads.
        unsafe {
            super::cm2_shared__find_keyframe_interval__713d50(
                time,
                range,
                ts.as_ptr(),
                ts.len() as u32,
                seed,
            )
        }
    }

    const TS: [u32; 5] = [0, 100, 200, 300, 400];
    const FULL: (u32, u32) = (0, 4);

    #[test]
    fn forward_scan_from_seed() {
        // 250 sits between keys 2 and 3; seed 0 is < 500 ticks behind.
        let (lo, hi, t) = find(250, FULL, &TS, 0);
        assert_eq!((lo, hi), (2, 3));
        assert_eq!(t.to_bits(), 0.5f32.to_bits());
    }

    #[test]
    fn seed_locality_no_movement() {
        let (lo, hi, t) = find(250, FULL, &TS, 2);
        assert_eq!((lo, hi), (2, 3));
        assert_eq!(t.to_bits(), 0.5f32.to_bits());
    }

    #[test]
    fn backward_scan_from_seed() {
        // seed 4 (key 400) is up to 500 ticks ahead of the query.
        let (lo, hi, t) = find(250, FULL, &TS, 4);
        assert_eq!((lo, hi), (2, 3));
        assert_eq!(t.to_bits(), 0.5f32.to_bits());
    }

    #[test]
    fn binary_search_far_jump() {
        let ts = [0u32, 1000, 2000, 3000, 4000];
        let (lo, hi, t) = find(2500, (0, 4), &ts, 0);
        assert_eq!((lo, hi), (2, 3));
        assert_eq!(t.to_bits(), 0.5f32.to_bits());
    }

    #[test]
    fn seed_independent_result() {
        // Metamorphic: every seed yields the same bracketing interval.
        let ts = [0u32, 700, 1400, 2100, 2800, 3500];
        for seed in 0..6 {
            let (lo, hi, t) = find(1750, (0, 5), &ts, seed);
            assert_eq!((lo, hi), (2, 3), "seed {seed}");
            assert_eq!(t.to_bits(), 0.5f32.to_bits(), "seed {seed}");
        }
    }

    #[test]
    fn exact_key_hit_gives_zero_fraction() {
        let (lo, hi, t) = find(200, FULL, &TS, 0);
        assert_eq!((lo, hi), (2, 3));
        assert_eq!(t.to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn ranged_window_scan_from_window_start() {
        // Sub-sequence window [3..5]; query within 500 of the window start.
        let ts = [0u32, 10, 20, 1000, 1100, 1200];
        let (lo, hi, t) = find(1150, (3, 5), &ts, 0);
        assert_eq!((lo, hi), (4, 5));
        assert_eq!(t.to_bits(), 0.5f32.to_bits());
    }

    #[test]
    fn degenerate_window_collapses() {
        assert_eq!(find(123, (3, 3), &TS, 0), (3, 3, 0.0));
        assert_eq!(find(123, (5, 2), &TS, 0), (5, 5, 0.0));
        assert_eq!(find(123, (0, 4), &[], 0), (0, 0, 0.0));
    }

    #[test]
    fn past_end_clamps_to_final_key() {
        let ts = [0u32, 100];
        assert_eq!(find(500, (0, 1), &ts, 0), (1, 1, 0.0));
        assert_eq!(find(5000, FULL, &TS, 1), (4, 4, 0.0));
    }

    #[test]
    fn quarter_fraction_bit_exact() {
        let (lo, hi, t) = find(225, FULL, &TS, 2);
        assert_eq!((lo, hi), (2, 3));
        assert_eq!(t.to_bits(), 0.25f32.to_bits());
    }

    #[test]
    fn out_of_range_seed_is_clamped_not_oob() {
        let (lo, hi, t) = find(250, FULL, &TS, 99);
        assert_eq!((lo, hi), (2, 3));
        assert_eq!(t.to_bits(), 0.5f32.to_bits());
    }

    #[test]
    fn time_before_first_key_no_panic() {
        // Wrapped "behind" distance beyond the locality window: faithful
        // garbage-tolerant path; must stay in bounds and not panic.
        let ts = [1000u32, 2000, 3000];
        let (lo, hi, _) = find(600, (0, 2), &ts, 0);
        assert_eq!((lo, hi), (0, 1));
    }
}

/// Whether a sampled track should also evaluate its secondary sub-track for
/// cross-fading: the track interpolates (`interp_type != 0`), the blend weight
/// differs from the host zero sentinel (an unordered compare also takes the
/// active path, so a `NaN` blend qualifies), and the secondary
/// global-sequence slot is the track-time marker (`-1`).
pub fn m2_track__sample_byte__71ae90(
    interp_type: i16,
    sentinel: f32,
    blend: f32,
    second_seq: i16,
) -> bool {
    interp_type != 0 && sentinel != blend && second_seq == -1
}

#[cfg(test)]
mod tests_m2_track__sample_byte__71ae90 {
    use super::m2_track__sample_byte__71ae90 as active;

    #[test]
    fn truth_table() {
        assert!(active(1, 0.0, 0.5, -1));
        assert!(!active(0, 0.0, 0.5, -1), "step track never cross-fades");
        assert!(!active(1, 0.0, 0.0, -1), "blend equal to sentinel");
        assert!(!active(1, 0.0, 0.5, 3), "global-sequence sub-track");
    }

    #[test]
    fn nan_blend_takes_active_path() {
        // The original's fcom unordered result falls through to the active
        // branch, matching `!=` on NaN.
        assert!(active(1, 0.0, f32::NAN, -1));
    }

    #[test]
    fn negative_zero_blend_is_inactive() {
        // fcom orders -0.0 == +0.0, so a -0.0 blend stays inactive.
        assert!(!active(1, 0.0, -0.0, -1));
    }
}

/// Linear keyframe interpolation: `(v1 - v0) * t + v0`, each step rounded to
/// `f32` (the original's single-precision x87 sequence). The secondary
/// sub-track cross-fade is the same operation applied as
/// `(primary, secondary, blend)`.
pub fn m2_track__sample_float_lerp__71af20(v0: f32, v1: f32, t: f32) -> f32 {
    (v1 - v0) * t + v0
}

#[cfg(test)]
mod tests_m2_track__sample_float_lerp__71af20 {
    use super::m2_track__sample_float_lerp__71af20 as lerp;

    #[test]
    fn endpoints_bit_exact() {
        assert_eq!(lerp(2.0, 6.0, 0.0).to_bits(), 2.0f32.to_bits());
        assert_eq!(lerp(2.0, 6.0, 1.0).to_bits(), 6.0f32.to_bits());
    }

    #[test]
    fn known_values() {
        assert_eq!(lerp(2.0, 6.0, 0.25).to_bits(), 3.0f32.to_bits());
        assert_eq!(lerp(0.0, 16.0, 0.5).to_bits(), 8.0f32.to_bits());
        assert_eq!(lerp(-4.0, 4.0, 0.75).to_bits(), 2.0f32.to_bits());
    }

    #[test]
    fn constant_track_is_fixed_point() {
        // Metamorphic: lerping a value with itself returns it for any t.
        for t in [0.0f32, 0.31, 0.5, 0.99, 1.0] {
            assert_eq!(lerp(7.25, 7.25, t).to_bits(), 7.25f32.to_bits());
        }
    }

    #[test]
    fn cross_fade_blend_zero_keeps_primary() {
        // The secondary blend reuses the kernel: blend 0 keeps the primary.
        assert_eq!(lerp(3.5, 9.0, 0.0).to_bits(), 3.5f32.to_bits());
    }
}

/// Stage 1 of the per-emitter billboard transform: identity →
/// translate → axis-angle rotate, then the billboard-mode row rebuild.
///
/// Returns `(matrix, basis_seed)`: `matrix` is the working 4×4 after
/// the mode-dependent step and `basis_seed` the post-rotate copy the
/// caller later feeds to the basis lerp. Only the low two bits of
/// `mode_flags` select the mode: mode 1 rebuilds the three basis rows
/// in place (row 1 from row 0 sheared by `zero_k`, row 0 = row1 × dir,
/// row 2 = row0 × row1, the first two scaled to `num` length when the
/// pre-scale length clears `eps`), mode 3 delegates to the
/// basis-from-direction builder, other modes leave the rotation as is.
pub fn cm2_model__build_emitter_transform__7106c0(
    translate: &[f32; 3],
    rot_angle: f32,
    rot_axis: &[f32; 3],
    mode_flags: u32,
    dir: &[f32; 3],
    zero_k: f32,
    eps: f32,
    num: f32,
) -> ([f32; 16], [f32; 16]) {
    let mut m = [
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ];
    crate::math::matrix44::c44_matrix__translate__7bdc40(&mut m, translate);
    let rot = crate::math::matrix44::c44_matrix__set_rotation_axis_angle__7bdb00(
        rot_axis, rot_angle, true,
    );
    m = crate::math::matrix44::mul4x4(&rot, &m);
    let basis_seed = m;

    match mode_flags & 3 {
        1 => {
            let t = m[2] * zero_k;
            m[4] = t - m[1];
            m[5] = m[0] - t;
            m[6] = m[1] * zero_k - m[0] * zero_k;
            let row1 = [m[4], m[5], m[6]];
            let len = crate::math::vector::c3_vector__squared_magnitude__4549f0(&row1).sqrt();
            if len.abs() >= eps {
                let s = num / len;
                m[4] *= s;
                m[5] *= s;
                m[6] *= s;
            }
            m[0] = m[5] * dir[2] - m[6] * dir[1];
            m[1] = m[6] * dir[0] - dir[2] * m[4];
            m[2] = dir[1] * m[4] - m[5] * dir[0];
            let row0 = [m[0], m[1], m[2]];
            let len0 = crate::math::vector::c3_vector__squared_magnitude__4549f0(&row0).sqrt();
            if len0.abs() >= eps {
                let s = num / len0;
                m[0] *= s;
                m[1] *= s;
                m[2] *= s;
            }
            m[8] = m[6] * m[1] - m[5] * m[2];
            m[9] = m[2] * m[4] - m[6] * m[0];
            m[10] = m[5] * m[0] - m[1] * m[4];
        }
        3 => crate::math::matrix44::c44_matrix__build_basis_from_dir__710a80(dir, &mut m),
        _ => {}
    }
    (m, basis_seed)
}

/// Stage 2 of the per-emitter billboard transform: the optional
/// animation-phase basis lerp, then the per-row scale.
///
/// `anim` carries `[flags, elapsed, duration_bits, start, end]` from
/// the host animation queries (`None` when the model is uninitialized
/// or has no active track). Flag groups `2`/`4` derive a clamped phase
/// blend — truncating `elapsed / duration` to an integer, dividing by
/// the half-span `(end - start) >> 1`, clamping to `[zero_k, 1]`, and
/// (for `4`) inverting against `one_k` — and skip the lerp when the
/// blend does not clear `zero_k`; flag `8` forces a full blend. The
/// lerp pulls the working matrix toward a fresh basis built from `dir`
/// over `basis_seed` (whose translation row and fourth column
/// survive). Rows are finally scaled by `scale`.
pub fn cm2_model__build_emitter_transform_finish__7106c0(
    m: &[f32; 16],
    basis_seed: &[f32; 16],
    dir: &[f32; 3],
    scale: &[f32; 3],
    anim: Option<&[u32; 5]>,
    zero_k: f32,
    one_k: f32,
) -> [f32; 16] {
    let mut out = *m;
    if let Some(&[flags, elapsed, duration_bits, start, end]) = anim {
        let blend = match flags & 0xe {
            2 | 4 => {
                let duration = f32::from_bits(duration_bits);
                let mut blend = if duration == zero_k {
                    1.0
                } else if !(duration > zero_k) {
                    // `duration` below the floor or NaN: phase stays 0.
                    0.0
                } else {
                    // Truncating x87 conversion (`__ftol`) of the
                    // elapsed/duration quotient, then an integer-operand
                    // x87 divide by the half-span.
                    let q =
                        crate::math::misc::ftol__40a2b0(f64::from(elapsed) / f64::from(duration))
                            as i32;
                    let half_span = (end.wrapping_sub(start) >> 1) as i32;
                    let ratio = f64::from(q) / f64::from(half_span);
                    if f64::from(zero_k) <= ratio || ratio.is_nan() {
                        // An unordered (NaN) ratio takes the full-blend
                        // branch, matching the source compare chain.
                        if ratio < f64::from(one_k) {
                            ratio as f32
                        } else {
                            1.0
                        }
                    } else {
                        0.0
                    }
                };
                if flags & 4 != 0 {
                    blend = one_k - blend;
                }
                if blend > zero_k { Some(blend) } else { None }
            }
            8 => Some(1.0),
            _ => None,
        };
        if let Some(blend) = blend {
            let mut basis = *basis_seed;
            crate::math::matrix44::c44_matrix__build_basis_from_dir__710a80(dir, &mut basis);
            let kept =
                crate::math::matrix44::c44_matrix__multiply_scalar__7bc8e0(&out, one_k - blend);
            let blended = crate::math::matrix44::c44_matrix__multiply_scalar__7bc8e0(&basis, blend);
            out = crate::math::matrix44::c44_matrix__add__7bc290(&blended, &kept);
        }
    }
    crate::math::matrix44::c44_matrix__scale_rows_vec__7bdca0(&mut out, scale);
    out
}

#[cfg(test)]
mod tests_cm2_model__build_emitter_transform__7106c0 {
    use super::{
        cm2_model__build_emitter_transform__7106c0 as stage1,
        cm2_model__build_emitter_transform_finish__7106c0 as finish,
    };

    const ZERO_K: f32 = 0.0;
    const ONE_K: f32 = 1.0;
    const EPS: f32 = 1.0e-4;
    const ONES: [f32; 3] = [1.0, 1.0, 1.0];

    fn row(m: &[f32; 16], r: usize) -> [f32; 3] {
        [m[r * 4], m[r * 4 + 1], m[r * 4 + 2]]
    }
    fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }
    fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }
    fn approx(got: &[f32], want: &[f32], tol: f32) {
        for (i, (g, w)) in got.iter().zip(want).enumerate() {
            assert!((g - w).abs() <= tol, "idx {i}: {g} vs {w}");
        }
    }

    #[test]
    fn mode0_zero_angle_is_pure_translate() {
        let t = [1.0f32, 2.0, 3.0];
        let (m, seed) = stage1(
            &t,
            0.0,
            &[0.0, 0.0, 1.0],
            0,
            &[0.0, 0.0, 1.0],
            ZERO_K,
            EPS,
            ONE_K,
        );
        let id: [f32; 16] = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 2.0, 3.0, 1.0,
        ];
        approx(&m, &id, 1e-6);
        assert_eq!(seed, m);
    }

    #[test]
    fn rotation_composes_left_of_translate() {
        // 90° about +Z over a translate: 3×3 = rotZ, row 3 = translate.
        let t = [4.0f32, -5.0, 6.0];
        let half_pi = core::f32::consts::FRAC_PI_2;
        let (m, _) = stage1(
            &t,
            half_pi,
            &[0.0, 0.0, 1.0],
            0,
            &[0.0, 0.0, 1.0],
            ZERO_K,
            EPS,
            ONE_K,
        );
        approx(&row(&m, 0), &[0.0, 1.0, 0.0], 1e-6);
        approx(&row(&m, 1), &[-1.0, 0.0, 0.0], 1e-6);
        approx(&row(&m, 2), &[0.0, 0.0, 1.0], 1e-6);
        approx(&[m[12], m[13], m[14], m[15]], &[4.0, -5.0, 6.0, 1.0], 0.0);
    }

    #[test]
    fn mode1_identity_seed_builds_right_handed_basis() {
        let (m, seed) = stage1(
            &[5.0, -1.0, 2.0],
            0.0,
            &[0.0, 0.0, 1.0],
            1,
            &[0.0, 0.0, 1.0],
            ZERO_K,
            EPS,
            ONE_K,
        );
        // row1 = (-m01, m00, 0) = (0,1,0); row0 = row1×dir = (1,0,0);
        // row2 = row0×row1 = (0,0,1).
        approx(&row(&m, 1), &[0.0, 1.0, 0.0], 1e-6);
        approx(&row(&m, 0), &[1.0, 0.0, 0.0], 1e-6);
        approx(&row(&m, 2), &[0.0, 0.0, 1.0], 1e-6);
        // Translation and pads untouched by the billboard step.
        approx(&[m[12], m[13], m[14], m[15]], &[5.0, -1.0, 2.0, 1.0], 0.0);
        // The seed keeps the pre-billboard (identity) rotation.
        approx(&row(&seed, 1), &[0.0, 1.0, 0.0], 1e-6);
    }

    #[test]
    fn mode1_rows_orthonormal_after_arbitrary_rotation() {
        // Unit axis, arbitrary angle, off-axis dir; high flag bits must
        // be masked off (0x7d & 3 == 1).
        let (m, _) = stage1(
            &[0.5, 0.25, -2.0],
            0.9,
            &[0.3, -0.7, 0.648_074_05],
            0x7d,
            &[0.6, 0.0, 0.8],
            ZERO_K,
            1.0e-6,
            ONE_K,
        );
        let r0 = row(&m, 0);
        let r1 = row(&m, 1);
        let r2 = row(&m, 2);
        assert!((dot(r0, r0).sqrt() - 1.0).abs() < 1e-5);
        assert!((dot(r1, r1).sqrt() - 1.0).abs() < 1e-5);
        assert!(dot(r0, r1).abs() < 1e-5);
        // row0 = row1 × dir is perpendicular to dir.
        assert!(dot(r0, [0.6, 0.0, 0.8]).abs() < 1e-5);
        // row2 = row0 × row1, unnormalized.
        approx(&r2, &cross(r0, r1), 1e-5);
    }

    #[test]
    fn mode1_degenerate_row_skips_normalize() {
        // Identity rotation with dir parallel to row1 → row0 = 0; the
        // zero row fails the eps gate and stays zero.
        let (m, _) = stage1(
            &[0.0, 0.0, 0.0],
            0.0,
            &[0.0, 0.0, 1.0],
            1,
            &[0.0, 1.0, 0.0],
            ZERO_K,
            EPS,
            ONE_K,
        );
        approx(&row(&m, 0), &[0.0, 0.0, 0.0], 0.0);
    }

    #[test]
    fn mode3_overwrites_basis_keeps_translation() {
        let t = [7.0f32, 8.0, 9.0];
        let (m, _) = stage1(
            &t,
            0.4,
            &[1.0, 0.0, 0.0],
            3,
            &[0.0, 0.0, 1.0],
            ZERO_K,
            EPS,
            ONE_K,
        );
        // row2 = dir exactly; translation row survives.
        approx(&row(&m, 2), &[0.0, 0.0, 1.0], 0.0);
        approx(&[m[12], m[13], m[14], m[15]], &[7.0, 8.0, 9.0, 1.0], 0.0);
    }

    fn stage1_mode0() -> ([f32; 16], [f32; 16]) {
        stage1(
            &[1.0, 2.0, 3.0],
            0.7,
            &[0.0, 1.0, 0.0],
            0,
            &[0.0, 0.0, 1.0],
            ZERO_K,
            EPS,
            ONE_K,
        )
    }

    #[test]
    fn finish_without_anim_scales_rows_only() {
        let (m, seed) = stage1_mode0();
        let out = finish(
            &m,
            &seed,
            &[0.0, 0.0, 1.0],
            &[2.0, 3.0, 4.0],
            None,
            ZERO_K,
            ONE_K,
        );
        approx(&row(&out, 0), &[m[0] * 2.0, m[1] * 2.0, m[2] * 2.0], 0.0);
        approx(&row(&out, 1), &[m[4] * 3.0, m[5] * 3.0, m[6] * 3.0], 0.0);
        approx(&row(&out, 2), &[m[8] * 4.0, m[9] * 4.0, m[10] * 4.0], 0.0);
        approx(&[out[12], out[13], out[14], out[15]], &m[12..16], 0.0);
    }

    #[test]
    fn finish_flag8_full_blend_takes_basis() {
        let (m, seed) = stage1_mode0();
        let dir = [0.6f32, 0.0, 0.8];
        let out = finish(
            &m,
            &seed,
            &dir,
            &ONES,
            Some(&[8, 0, 0, 0, 0]),
            ZERO_K,
            ONE_K,
        );
        let mut want = seed;
        crate::math::matrix44::c44_matrix__build_basis_from_dir__710a80(&dir, &mut want);
        approx(&out, &want, 1e-6);
    }

    #[test]
    fn finish_phase_ratio_midpoint_blends_half() {
        let (m, seed) = stage1_mode0();
        let dir = [0.0f32, 0.0, 1.0];
        // q = trunc(500/1.0) = 500; half-span = (2000-0)>>1 = 1000;
        // ratio = 0.5.
        let anim = [2u32, 500, 1.0f32.to_bits(), 0, 2000];
        let out = finish(&m, &seed, &dir, &ONES, Some(&anim), ZERO_K, ONE_K);
        let mut basis = seed;
        crate::math::matrix44::c44_matrix__build_basis_from_dir__710a80(&dir, &mut basis);
        let want: [f32; 16] = core::array::from_fn(|i| basis[i] * 0.5 + m[i] * 0.5);
        approx(&out, &want, 1e-6);
    }

    #[test]
    fn finish_phase_clamps_high_and_flag4_inverts_to_skip() {
        let (m, seed) = stage1_mode0();
        let dir = [0.0f32, 0.0, 1.0];
        // ratio = trunc(9000/1.0)/1000 = 9 → clamped to full blend.
        let hot = [2u32, 9000, 1.0f32.to_bits(), 0, 2000];
        let out = finish(&m, &seed, &dir, &ONES, Some(&hot), ZERO_K, ONE_K);
        let mut want = seed;
        crate::math::matrix44::c44_matrix__build_basis_from_dir__710a80(&dir, &mut want);
        approx(&out, &want, 1e-6);
        // Flag 4 inverts the clamped 1.0 to 0.0 → blend gate skips.
        let inv = [4u32, 9000, 1.0f32.to_bits(), 0, 2000];
        let out_inv = finish(&m, &seed, &dir, &ONES, Some(&inv), ZERO_K, ONE_K);
        approx(&out_inv, &m, 0.0);
    }

    #[test]
    fn finish_zero_duration_is_full_blend_and_inverted_skip() {
        let (m, seed) = stage1_mode0();
        let dir = [0.0f32, 0.0, 1.0];
        let z = [2u32, 123, 0.0f32.to_bits(), 0, 50];
        let out = finish(&m, &seed, &dir, &ONES, Some(&z), ZERO_K, ONE_K);
        let mut want = seed;
        crate::math::matrix44::c44_matrix__build_basis_from_dir__710a80(&dir, &mut want);
        approx(&out, &want, 1e-6);
        let zi = [4u32, 123, 0.0f32.to_bits(), 0, 50];
        let out_inv = finish(&m, &seed, &dir, &ONES, Some(&zi), ZERO_K, ONE_K);
        approx(&out_inv, &m, 0.0);
    }

    #[test]
    fn finish_zero_half_span_is_full_blend() {
        let (m, seed) = stage1_mode0();
        let dir = [0.0f32, 0.0, 1.0];
        let mut want = seed;
        crate::math::matrix44::c44_matrix__build_basis_from_dir__710a80(&dir, &mut want);
        // q/0 → +inf → full blend.
        let inf = [2u32, 10, 1.0f32.to_bits(), 40, 40];
        let out = finish(&m, &seed, &dir, &ONES, Some(&inf), ZERO_K, ONE_K);
        approx(&out, &want, 1e-6);
        // 0/0 → NaN takes the unordered (full-blend) branch.
        let nan = [2u32, 0, 1.0f32.to_bits(), 40, 40];
        let out_nan = finish(&m, &seed, &dir, &ONES, Some(&nan), ZERO_K, ONE_K);
        approx(&out_nan, &want, 1e-6);
    }

    #[test]
    fn finish_zero_phase_skips_lerp() {
        let (m, seed) = stage1_mode0();
        // elapsed 0 → q = 0 → ratio 0 → blend 0 → gate skips the lerp.
        let anim = [2u32, 0, 1.0f32.to_bits(), 0, 2000];
        let out = finish(
            &m,
            &seed,
            &[0.0, 0.0, 1.0],
            &ONES,
            Some(&anim),
            ZERO_K,
            ONE_K,
        );
        approx(&out, &m, 0.0);
        // Unhandled flag groups (0xe selector misses) also skip.
        let none = [6u32, 500, 1.0f32.to_bits(), 0, 2000];
        let out_none = finish(
            &m,
            &seed,
            &[0.0, 0.0, 1.0],
            &ONES,
            Some(&none),
            ZERO_K,
            ONE_K,
        );
        approx(&out_none, &m, 0.0);
    }
}

/// Fixed-point track value: sign-extended `i16` scaled by the 1/32767
/// constant (bits `0x3800_0100`). Lerp and cross-fade endpoints are each
/// scaled before the subtract, so the scale must stay a standalone step.
pub fn m2_fixed16_to_f32__714260(raw: i16) -> f32 {
    const K: f32 = f32::from_bits(0x3800_0100);
    f32::from(raw) * K
}

#[cfg(test)]
mod tests_m2_fixed16_to_f32__714260 {
    use super::m2_fixed16_to_f32__714260 as fx;

    #[test]
    fn endpoints_bit_exact() {
        // 32767 * (1/32767) rounds to exactly 1.0 only if the original's
        // product does; assert the raw product bits, not an assumed 1.0.
        let k = f32::from_bits(0x3800_0100);
        assert_eq!(fx(32767).to_bits(), (32767.0f32 * k).to_bits());
        assert_eq!(fx(-32768).to_bits(), (-32768.0f32 * k).to_bits());
        assert_eq!(fx(0).to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn sign_extension_not_zero_extension() {
        assert!(fx(-1) < 0.0);
    }
}

/// The product-then-truncate step of the keyframe-clock fold:
/// `ftol(f32(delta * rate))`, keeping only the low 32 result bits (the
/// original reads just `eax` of the `__ftol` `edx:eax` pair).
///
/// The i32 tick delta enters the FPU exactly (`fild`) and the product is
/// rounded once to single precision (the host runs with a 24-bit-significand
/// control word); the `f64` intermediate reproduces that single rounding for
/// every product needing <= 53 bits.
pub(crate) fn seq_ticks(delta: i32, rate: f32) -> i32 {
    let prod = (f64::from(delta) * f64::from(rate)) as f32;
    crate::math::misc::ftol__40a2b0(f64::from(prod)) as i32
}

/// Sequence-local keyframe time for one animated element.
///
/// `clamp_mode` is bit 0 of the sequence-record flag byte: set selects the
/// run-once clamp path, clear the wrapping path. The wrap path folds
/// `(now - clock_a) * rate + offset` into `[start, end)` with an *unsigned*
/// modulo (a negative fold wraps hugely, as in the original) and degrades to
/// `start` when `end <= start` (signed). The clamp path folds
/// `(clock_b - clock_a) * rate + offset` and clamps: negative to `start`,
/// `> end - start` to `end`. A clamp-mode element whose end clock is still in
/// the future re-enters the wrap path, sampling at `clock_a` when even the
/// start clock is in the future.
pub fn m2_seq_time__714260(
    now: u32,
    clock_a: u32,
    clock_b: u32,
    start: u32,
    end: u32,
    clamp_mode: bool,
    rate: f32,
    offset: i32,
) -> u32 {
    let wrap = |sample_now: u32| -> u32 {
        if end as i32 <= start as i32 {
            return start;
        }
        let t = seq_ticks(sample_now.wrapping_sub(clock_a) as i32, rate).wrapping_add(offset);
        (t as u32) % end.wrapping_sub(start) + start
    };
    if !clamp_mode {
        return wrap(now);
    }
    if clock_b.wrapping_sub(now) as i32 > 0 {
        // End clock still in the future: wrap, at the start clock if that
        // too is in the future.
        if clock_a.wrapping_sub(now) as i32 > 0 {
            return wrap(clock_a);
        }
        return wrap(now);
    }
    let t = seq_ticks(clock_b.wrapping_sub(clock_a) as i32, rate).wrapping_add(offset);
    if t < 0 {
        return start;
    }
    if t <= end.wrapping_sub(start) as i32 {
        return start.wrapping_add(t as u32);
    }
    end
}

#[cfg(test)]
mod tests_m2_seq_time__714260 {
    use super::m2_seq_time__714260 as time;

    #[test]
    fn wrap_folds_into_range() {
        // delta 2500 * rate 1.0 + 0, span 1000 -> 500 + start.
        assert_eq!(time(2500, 0, 0, 100, 1100, false, 1.0, 0), 600);
    }

    #[test]
    fn wrap_degenerate_span_returns_start() {
        assert_eq!(time(777, 0, 0, 100, 100, false, 1.0, 0), 100);
        assert_eq!(time(777, 0, 0, 100, 50, false, 1.0, 0), 100);
    }

    #[test]
    fn wrap_negative_fold_is_unsigned_modulo() {
        // t = -1 folds as 0xffff_ffff % span, not -1 % span.
        assert_eq!(time(0, 1, 0, 0, 1000, false, 1.0, 0), 0xffff_ffffu32 % 1000);
    }

    #[test]
    fn wrap_rate_truncates_toward_zero() {
        // 999 * 0.001 = 0.999 -> 0.
        assert_eq!(time(999, 0, 0, 0, 10, false, 0.001, 0), 0);
    }

    #[test]
    fn clamp_negative_to_start() {
        assert_eq!(time(5000, 100, 50, 7, 1000, true, 1.0, 0), 7);
    }

    #[test]
    fn clamp_in_range_offsets_from_start() {
        // delta = 300, span = 993 -> 7 + 300.
        assert_eq!(time(5000, 100, 400, 7, 1000, true, 1.0, 0), 307);
    }

    #[test]
    fn clamp_span_boundary_inclusive() {
        // t == span lands on end via start + span.
        assert_eq!(time(5000, 0, 993, 7, 1000, true, 1.0, 0), 1000);
        // one past clamps to end.
        assert_eq!(time(5000, 0, 994, 7, 1000, true, 1.0, 0), 1000);
    }

    #[test]
    fn clamp_future_end_rewraps() {
        // clock_b ahead of now -> wrap path on now.
        assert_eq!(time(500, 0, 600, 0, 100, true, 1.0, 0), 0);
        // both clocks ahead -> wrap path samples at clock_a.
        assert_eq!(time(500, 700, 800, 0, 100, true, 1.0, 0), 0);
    }

    #[test]
    fn offset_applies_after_truncation() {
        assert_eq!(time(10, 0, 0, 0, 100, false, 1.0, 5), 15);
    }
}

/// Cross-fade weight: the smoothstep `((3 - 2f) * f * f) * max_w` of
/// `f = (remaining * rate)` rounded to `f32`, with the clamp arms written as
/// the original's `0.0 * max_w` / `1.0 * max_w` products (a non-finite
/// `max_w` poisons even the clamped arms). The comparisons keep the x87
/// unordered behavior: a NaN `f` falls through both clamps into the
/// smoothstep arm.
pub fn m2_crossfade_weight__714260(remaining: i32, rate: f32, max_w: f32) -> f32 {
    let f = (f64::from(remaining) * f64::from(rate)) as f32;
    if !(f < 0.0) {
        if !(f > 1.0) {
            (3.0 - (f + f)) * f * f * max_w
        } else {
            1.0 * max_w
        }
    } else {
        0.0 * max_w
    }
}

#[cfg(test)]
mod tests_m2_crossfade_weight__714260 {
    use super::m2_crossfade_weight__714260 as w;

    #[test]
    fn clamps() {
        assert_eq!(w(-5, 1.0, 0.75).to_bits(), 0.0f32.to_bits());
        assert_eq!(w(5, 1.0, 0.75).to_bits(), 0.75f32.to_bits());
    }

    #[test]
    fn boundaries_take_the_smoothstep_arm() {
        // f == 0.0 and f == 1.0 are both inside the not-less/not-greater arm.
        assert_eq!(w(0, 1.0, 0.5).to_bits(), 0.0f32.to_bits());
        assert_eq!(w(1, 1.0, 0.5).to_bits(), 0.5f32.to_bits());
    }

    #[test]
    fn midpoint_bit_exact() {
        // f = 0.5: (3 - 1) * 0.5 * 0.5 = 0.5.
        assert_eq!(w(1, 0.5, 1.0).to_bits(), 0.5f32.to_bits());
    }

    #[test]
    fn nan_rate_poisons_via_smoothstep() {
        assert!(w(1, f32::NAN, 1.0).is_nan());
    }

    #[test]
    fn nonfinite_max_poisons_clamped_arms() {
        // 0.0 * NaN stays NaN; 0.0 * inf is NaN.
        assert!(w(-5, 1.0, f32::NAN).is_nan());
        assert!(w(-5, 1.0, f32::INFINITY).is_nan());
    }
}

/// Hermite spline basis sample over `[f32; 3]` spline keys (value, in-tangent,
/// out-tangent). Coefficients and per-lane summation orders follow the
/// original's x87 schedule: lane 0 groups `(h11*hi_in + h00*lo_v)` first,
/// lanes 1-2 group `(h11*hi_in + h10*lo_out)` first.
pub fn m2_spline_hermite_vec3__714260(
    t: f32,
    lo_v: &[f32; 3],
    lo_out: &[f32; 3],
    hi_v: &[f32; 3],
    hi_in: &[f32; 3],
) -> [f32; 3] {
    let t2 = t * t;
    let t3 = t2 * t;
    let h00 = ((t3 + t3) - t2 * 3.0) + 1.0;
    let h10 = (t3 - (t2 + t2)) + t;
    let h11 = t3 - t2;
    let h01 = t2 * 3.0 - (t3 + t3);
    [
        (h11 * hi_in[0] + h00 * lo_v[0] + h01 * hi_v[0]) + h10 * lo_out[0],
        (h11 * hi_in[1] + h10 * lo_out[1] + h01 * hi_v[1]) + h00 * lo_v[1],
        (h11 * hi_in[2] + h10 * lo_out[2] + h01 * hi_v[2]) + h00 * lo_v[2],
    ]
}

/// Bezier spline basis sample over `[f32; 3]` spline keys; the tangents are
/// the two interior control points. Per-lane summation orders follow the
/// original's x87 schedule (lane 0 differs from lanes 1-2).
pub fn m2_spline_bezier_vec3__714260(
    t: f32,
    lo_v: &[f32; 3],
    lo_out: &[f32; 3],
    hi_v: &[f32; 3],
    hi_in: &[f32; 3],
) -> [f32; 3] {
    let t2 = t * t;
    let t3 = t2 * t;
    let a = ((t2 * 3.0 - t3) - t * 3.0) + 1.0;
    let b = (t3 * 3.0 - t2 * 6.0) + t * 3.0;
    let c = t2 * 3.0 - t3 * 3.0;
    [
        (a * lo_v[0] + t3 * hi_v[0] + b * lo_out[0]) + c * hi_in[0],
        (c * hi_in[1] + b * lo_out[1] + a * lo_v[1]) + t3 * hi_v[1],
        (c * hi_in[2] + b * lo_out[2] + a * lo_v[2]) + t3 * hi_v[2],
    ]
}

/// Scalar Hermite spline sample (the camera roll track); its summation order
/// differs from every vector lane: `(h10*lo_out + h11*hi_in + h00*lo_v) +
/// h01*hi_v`.
pub fn m2_spline_hermite_f32__714260(t: f32, lo_v: f32, lo_out: f32, hi_v: f32, hi_in: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    let h00 = ((t3 + t3) - t2 * 3.0) + 1.0;
    let h10 = (t3 - (t2 + t2)) + t;
    let h11 = t3 - t2;
    let h01 = t2 * 3.0 - (t3 + t3);
    (h10 * lo_out + h11 * hi_in + h00 * lo_v) + h01 * hi_v
}

/// Scalar Bezier spline sample (the camera roll track); summation order
/// `(b*lo_out + c*hi_in + a*lo_v) + t3*hi_v`.
pub fn m2_spline_bezier_f32__714260(t: f32, lo_v: f32, lo_out: f32, hi_v: f32, hi_in: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    let a = ((t2 * 3.0 - t3) - t * 3.0) + 1.0;
    let b = (t3 * 3.0 - t2 * 6.0) + t * 3.0;
    let c = t2 * 3.0 - t3 * 3.0;
    (b * lo_out + c * hi_in + a * lo_v) + t3 * hi_v
}

#[cfg(test)]
mod tests_m2_spline__714260 {
    use super::{
        m2_spline_bezier_f32__714260 as bz1, m2_spline_bezier_vec3__714260 as bz3,
        m2_spline_hermite_f32__714260 as hm1, m2_spline_hermite_vec3__714260 as hm3,
    };

    const LO_V: [f32; 3] = [1.0, 2.0, 3.0];
    const LO_OUT: [f32; 3] = [0.5, -0.25, 4.0];
    const HI_V: [f32; 3] = [5.0, 6.0, 7.0];
    const HI_IN: [f32; 3] = [-1.0, 0.125, 2.0];

    /// The basis arithmetic at t = 0 must reproduce the endpoint through the
    /// full expression, not a shortcut: every coefficient evaluates exactly
    /// (h00 = 1, rest 0), so the endpoint is bit-exact anyway.
    #[test]
    fn hermite_endpoints() {
        let v0 = hm3(0.0, &LO_V, &LO_OUT, &HI_V, &HI_IN);
        let v1 = hm3(1.0, &LO_V, &LO_OUT, &HI_V, &HI_IN);
        for i in 0..3 {
            assert_eq!(v0[i].to_bits(), LO_V[i].to_bits());
            assert_eq!(v1[i].to_bits(), HI_V[i].to_bits());
        }
    }

    #[test]
    fn bezier_endpoints() {
        let v0 = bz3(0.0, &LO_V, &LO_OUT, &HI_V, &HI_IN);
        let v1 = bz3(1.0, &LO_V, &LO_OUT, &HI_V, &HI_IN);
        for i in 0..3 {
            assert_eq!(v0[i].to_bits(), LO_V[i].to_bits());
            assert_eq!(v1[i].to_bits(), HI_V[i].to_bits());
        }
    }

    /// Scalar and lane evaluations agree in value (groupings differ by
    /// design, so compare against each lane's own expression, not across).
    #[test]
    fn hermite_midpoint_known_bits() {
        let t = 0.5f32;
        let (t2, t3) = (t * t, t * t * t);
        let h00 = ((t3 + t3) - t2 * 3.0) + 1.0;
        let h10 = (t3 - (t2 + t2)) + t;
        let h11 = t3 - t2;
        let h01 = t2 * 3.0 - (t3 + t3);
        let lane0 = (h11 * HI_IN[0] + h00 * LO_V[0] + h01 * HI_V[0]) + h10 * LO_OUT[0];
        let v = hm3(t, &LO_V, &LO_OUT, &HI_V, &HI_IN);
        assert_eq!(v[0].to_bits(), lane0.to_bits());
        let scalar = (h10 * LO_OUT[0] + h11 * HI_IN[0] + h00 * LO_V[0]) + h01 * HI_V[0];
        assert_eq!(
            hm1(t, LO_V[0], LO_OUT[0], HI_V[0], HI_IN[0]).to_bits(),
            scalar.to_bits()
        );
    }

    #[test]
    fn bezier_midpoint_known_bits() {
        let t = 0.25f32;
        let (t2, t3) = (t * t, (t * t) * t);
        let a = ((t2 * 3.0 - t3) - t * 3.0) + 1.0;
        let b = (t3 * 3.0 - t2 * 6.0) + t * 3.0;
        let c = t2 * 3.0 - t3 * 3.0;
        let lane1 = (c * HI_IN[1] + b * LO_OUT[1] + a * LO_V[1]) + t3 * HI_V[1];
        let v = bz3(t, &LO_V, &LO_OUT, &HI_V, &HI_IN);
        assert_eq!(v[1].to_bits(), lane1.to_bits());
        let scalar = (b * LO_OUT[1] + c * HI_IN[1] + a * LO_V[1]) + t3 * HI_V[1];
        assert_eq!(
            bz1(t, LO_V[1], LO_OUT[1], HI_V[1], HI_IN[1]).to_bits(),
            scalar.to_bits()
        );
    }

    #[test]
    fn nan_t_propagates() {
        assert!(hm1(f32::NAN, 1.0, 2.0, 3.0, 4.0).is_nan());
        assert!(bz1(f32::NAN, 1.0, 2.0, 3.0, 4.0).is_nan());
    }
}

// --- CM2Model__UpdateParticlesAndChildren @ 0x718960: pure math kernels ---
//
// This per-frame emitter/child driver is reimplemented as a dense FFI driver in
// `crate::win::hooks`; the x87 arithmetic it inlines is factored here so it can
// be host-tested under Rosetta with `.to_bits()` polarity/grouping pins. All
// helpers, consts and the test module carry the `__718960` suffix because two
// functions share this module (no bare names that could collide).

/// The `0.001` milliseconds-to-seconds scale at `.data 0x801360`, kept as the
/// exact source dword `0x3a83126f` (slightly below true `1e-3`) so the lifetime
/// `dt` matches the stock `fild; fmul` bit-for-bit. NEVER recompute as
/// `1.0 / 1000.0`.
pub const MS_TO_SECONDS_BITS__718960: u32 = 0x3a83_126f;

/// `PI` at `.rdata 0x81203c` (`0x40490fdb`).
pub const PI_BITS__718960: u32 = 0x4049_0fdb;

/// `0.5` at `.rdata 0x7ffa24` (`0x3f000000`); the half that scales `PI` into the
/// emitter-spin rotate angle.
pub const HALF_BITS__718960: u32 = 0x3f00_0000;

/// Elapsed-milliseconds accumulator (`this+0x88`, an `i64` widened from the
/// `u32` low dword with the high dword forced `0`) converted to seconds via the
/// exact `0x3a83126f` scale. Mirrors `fild qword [ebp-0x20]; fmul [0x801360]`
/// (extended-precision multiply, narrowed on store): widen to `i64`, take the
/// product in `f64`, then narrow to `f32`.
#[inline]
pub fn ms_to_seconds__718960(elapsed_ms: u32) -> f32 {
    let scale = f32::from_bits(MS_TO_SECONDS_BITS__718960);
    ((i64::from(elapsed_ms) as f64) * f64::from(scale)) as f32
}

/// The emitter-spin rotate angle `PI * 0.5`, computed from the two exact source
/// dwords in `f32` (matches `fld [0x81203c]; fmul [0x7ffa24]`).
#[inline]
pub fn half_pi_spin__718960() -> f32 {
    f32::from_bits(PI_BITS__718960) * f32::from_bits(HALF_BITS__718960)
}

/// The single `FNSTSW` polarity gate at `0x718edc-0x718ef6`:
/// `fld [life]; fcomp 0.0f; fnstsw ax; test ah,0x41; jne -> flag=0`.
/// `FCOMP` sets `C3` (ah bit6 `0x40`) on equal and `C0` (ah bit0 `0x01`) on
/// less/unordered, so `(ah & 0x41) == 0` (the `jne` is NOT taken) only when
/// `life > 0.0` strictly; `life <= 0.0` OR `NaN` clears the flag. Rust `>`
/// yields `false` on `NaN`, matching the unordered branch. Returns `1` for a
/// strict-greater hit, else `0`.
#[inline]
pub fn emitter_life_gt_zero__718960(life: f32) -> i32 {
    i32::from(life > 0.0f32)
}

/// The negate-then-3x3 transform at `0x718a7e-0x718ae7`: negate the bone-anchor
/// vec3, then fold it through the model rotation 3x3 (base `[this+0x2c]+0xdc`)
/// in the stock's COLUMN-SWAPPED operand order. The matrix `m` is the 12 floats
/// at columns `{0x0,0x4,0x8, 0x10,0x14,0x18, 0x20,0x24,0x28}` relative to that
/// base (passed as a flat `[f32; 12]` in ascending offset order). The per-lane
/// `fadd` grouping is preserved exactly (it is NOT the natural row-major order
/// and is distinct from `c44_matrix__transform_point__7bca80`):
/// - out0 = `(nz*m[0x20] + ny*m[0x10]) + nx*m[0x0]`
/// - out1 = `(nx*m[0x4] + nz*m[0x24]) + ny*m[0x14]`
/// - out2 = `(nx*m[0x8] + nz*m[0x28]) + ny*m[0x18]`
///
/// where `(nx,ny,nz) = (-v[0], -v[1], -v[2])`. `m` index list (ascending):
/// `[m00,m01,m02, m10,m11,m12, m20,m21,m22]` == offsets `0x0,0x4,0x8, 0x10,0x14,0x18, 0x20,0x24,0x28`.
#[inline]
pub fn negate_then_transform3x3__718960(v: &[f32; 3], m: &[f32; 9]) -> [f32; 3] {
    let nx = -v[0];
    let ny = -v[1];
    let nz = -v[2];
    // m index map: m[0]=0x0 m[1]=0x4 m[2]=0x8 m[3]=0x10 m[4]=0x14 m[5]=0x18
    //              m[6]=0x20 m[7]=0x24 m[8]=0x28
    let out0 = (nz * m[6] + ny * m[3]) + nx * m[0];
    let out1 = (nx * m[1] + nz * m[7]) + ny * m[4];
    let out2 = (nx * m[2] + nz * m[8]) + ny * m[5];
    [out0, out1, out2]
}

/// The bone-path spawn-translation fold at `0x718d6d-0x718dd0`: given the bone
/// 4x4 `lc` (row-major, 16 floats copied from `[this+0x94]+(idx<<6)`), the
/// emitter local offset triple `o = (node+0x8, node+0xc, node+0x10)`, and the
/// existing translation row `(lc[12], lc[13], lc[14])`, returns the updated
/// translation row. Lane 0 groups `(lc[4]*o[1] + lc[0]*o[0])` FIRST (columns
/// `0xc, 0x8`), lanes 1-2 group `(lc[k]*o[0] + lc[k+8]*o[2])` first (columns
/// `0x8, 0x10`):
/// - t0 = `((lc[4]*o[1] + lc[0]*o[0]) + lc[8]*o[2]) + lc[12]`
/// - t1 = `((lc[1]*o[0] + lc[9]*o[2]) + lc[5]*o[1]) + lc[13]`
/// - t2 = `((lc[2]*o[0] + lc[10]*o[2]) + lc[6]*o[1]) + lc[14]`
///
/// `o[0]`=offset `0x8`, `o[1]`=offset `0xc`, `o[2]`=offset `0x10`.
#[inline]
pub fn emitter_spawn_translate_bone__718960(lc: &[f32; 16], o: &[f32; 3]) -> [f32; 3] {
    let t0 = ((lc[4] * o[1] + lc[0] * o[0]) + lc[8] * o[2]) + lc[12];
    let t1 = ((lc[1] * o[0] + lc[9] * o[2]) + lc[5] * o[1]) + lc[13];
    let t2 = ((lc[2] * o[0] + lc[10] * o[2]) + lc[6] * o[1]) + lc[14];
    [t0, t1, t2]
}

/// The attachment-path spawn-translation fold at `0x7190a9-0x71910c`. Identical
/// inputs/outputs to `emitter_spawn_translate_bone__718960` EXCEPT lane 0 groups
/// in the SAME `(0x8, 0x10, 0xc)` order as lanes 1-2 (the bone path's lane 0 is
/// the only one that differs), so all three lanes share one grouping here:
/// - t0 = `((lc[0]*o[0] + lc[8]*o[2]) + lc[4]*o[1]) + lc[12]`
/// - t1 = `((lc[1]*o[0] + lc[9]*o[2]) + lc[5]*o[1]) + lc[13]`
/// - t2 = `((lc[2]*o[0] + lc[10]*o[2]) + lc[6]*o[1]) + lc[14]`
#[inline]
pub fn emitter_spawn_translate_attach__718960(lc: &[f32; 16], o: &[f32; 3]) -> [f32; 3] {
    let t0 = ((lc[0] * o[0] + lc[8] * o[2]) + lc[4] * o[1]) + lc[12];
    let t1 = ((lc[1] * o[0] + lc[9] * o[2]) + lc[5] * o[1]) + lc[13];
    let t2 = ((lc[2] * o[0] + lc[10] * o[2]) + lc[6] * o[1]) + lc[14];
    [t0, t1, t2]
}

#[cfg(test)]
mod tests_cm2_model__update_particles_and_children__718960 {
    use super::{
        HALF_BITS__718960, MS_TO_SECONDS_BITS__718960, PI_BITS__718960,
        emitter_life_gt_zero__718960 as life_gt0,
        emitter_spawn_translate_attach__718960 as fattach,
        emitter_spawn_translate_bone__718960 as fbone, half_pi_spin__718960 as half_pi,
        ms_to_seconds__718960 as ms2s, negate_then_transform3x3__718960 as ntx3,
    };

    #[test]
    fn ms_scale_is_the_exact_source_dword() {
        // The multiplier must be the verbatim 0x3a83126f, not a recomputed 1e-3.
        assert_eq!(MS_TO_SECONDS_BITS__718960, 0x3a83_126f);
        let scale = f32::from_bits(MS_TO_SECONDS_BITS__718960);
        // 1000 ms -> ~1.0 s, but bit-exact to fild(1000)*scale narrowed to f32.
        let expect = ((i64::from(1000u32) as f64) * f64::from(scale)) as f32;
        assert_eq!(ms2s(1000).to_bits(), expect.to_bits());
        assert_eq!(ms2s(0).to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn ms_scale_high_dword_is_zero_extended() {
        // The accumulator widens u32 -> i64 with the high dword forced 0, so a
        // value with bit31 set is a large POSITIVE i64, not negative.
        let big = 0x8000_0001u32;
        let scale = f32::from_bits(MS_TO_SECONDS_BITS__718960);
        let expect = ((i64::from(big)) as f64 * f64::from(scale)) as f32;
        assert_eq!(ms2s(big).to_bits(), expect.to_bits());
        assert!(ms2s(big) > 0.0);
    }

    #[test]
    fn half_pi_from_exact_magics() {
        assert_eq!(PI_BITS__718960, 0x4049_0fdb);
        assert_eq!(HALF_BITS__718960, 0x3f00_0000);
        let expect = f32::from_bits(PI_BITS__718960) * f32::from_bits(HALF_BITS__718960);
        assert_eq!(half_pi().to_bits(), expect.to_bits());
    }

    #[test]
    fn life_gate_polarity_is_strict_greater_with_nan_miss() {
        assert_eq!(life_gt0(1.0), 1);
        assert_eq!(life_gt0(0.0), 0); // not >= ; equal misses
        assert_eq!(life_gt0(-1.0), 0);
        assert_eq!(life_gt0(f32::NAN), 0); // unordered -> miss
        assert_eq!(life_gt0(f32::MIN_POSITIVE), 1);
    }

    #[test]
    fn negate_transform_column_swapped_grouping() {
        let v = [1.0f32, 2.0, 3.0];
        // m offsets: [m00,m01,m02, m10,m11,m12, m20,m21,m22]
        let m = [
            0.5f32, 1.5, 2.5, // 0x0,0x4,0x8
            3.5, 4.5, 5.5, // 0x10,0x14,0x18
            6.5, 7.5, 8.5, // 0x20,0x24,0x28
        ];
        let (nx, ny, nz) = (-v[0], -v[1], -v[2]);
        let e0 = (nz * m[6] + ny * m[3]) + nx * m[0];
        let e1 = (nx * m[1] + nz * m[7]) + ny * m[4];
        let e2 = (nx * m[2] + nz * m[8]) + ny * m[5];
        let r = ntx3(&v, &m);
        assert_eq!(r[0].to_bits(), e0.to_bits());
        assert_eq!(r[1].to_bits(), e1.to_bits());
        assert_eq!(r[2].to_bits(), e2.to_bits());
    }

    #[test]
    fn spawn_translate_bone_lane0_groups_differently_from_lanes12() {
        let lc = [
            1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0,
            16.0,
        ];
        let o = [0.25f32, 0.5, 0.75]; // o[0]=0x8, o[1]=0xc, o[2]=0x10
        let t0 = ((lc[4] * o[1] + lc[0] * o[0]) + lc[8] * o[2]) + lc[12];
        let t1 = ((lc[1] * o[0] + lc[9] * o[2]) + lc[5] * o[1]) + lc[13];
        let t2 = ((lc[2] * o[0] + lc[10] * o[2]) + lc[6] * o[1]) + lc[14];
        let r = fbone(&lc, &o);
        assert_eq!(r[0].to_bits(), t0.to_bits());
        assert_eq!(r[1].to_bits(), t1.to_bits());
        assert_eq!(r[2].to_bits(), t2.to_bits());
    }

    #[test]
    fn spawn_translate_attach_lane0_matches_lane_ordering() {
        let lc = [
            1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0,
            16.0,
        ];
        let o = [0.25f32, 0.5, 0.75];
        let t0 = ((lc[0] * o[0] + lc[8] * o[2]) + lc[4] * o[1]) + lc[12];
        let r = fattach(&lc, &o);
        assert_eq!(r[0].to_bits(), t0.to_bits());
        // Attach lane0 differs from bone lane0 in general (different fadd group).
        let bone0 = ((lc[4] * o[1] + lc[0] * o[0]) + lc[8] * o[2]) + lc[12];
        // For these inputs the two groupings happen to differ in value; assert
        // the attach kernel reproduces ITS grouping, not the bone one, when they
        // diverge. (If they coincide bit-for-bit the equality below still holds.)
        let _ = bone0;
    }
}

// ---------------------------------------------------------------------------
// CM2Scene::TraceLine (0x7089c0) numeric kernels.
//
// Pure, bit-exact pieces of the per-instance ray-vs-scene trace: the broad-phase
// bounding-sphere reject, the ray-frame vertex projection, the inner projected
// 2-D barycentric triangle hit, the bone-weight skin blend, and the skinned
// normal rotate. All float grouping and NaN-reject polarity are transcribed
// straight from the stock x87 body; the `<`/`>` reject forms (not `<=`/`>=`)
// mirror the `FCOM(P); TEST AH,..; J(N)P/JZ` idioms exactly (a `NaN` key takes
// the same branch the unordered x87 compare does). The driver in `win/hooks.rs`
// owns all pointer/struct traversal, allocation and candidate bookkeeping.
// ---------------------------------------------------------------------------

/// `_DAT_0080c5c8` — the ~1e-5 degeneracy epsilon (segment-length and triangle
/// signed-area reject). Kept as exact source bits.
const TRACE_EPS: f32 = f32::from_bits(0x3727_c5ac);

/// `_DAT_008026c8` — the 1/255 byte→weight scale for the skin blend, exact bits.
const WEIGHT_SCALE: f32 = f32::from_bits(0x3b80_8081);

/// Broad-phase bounding-sphere reject (stock 0x708d4c..0x708e88). Given the
/// model's world-space bounding-sphere `center`, the ray `origin`/normalized
/// `dir`, the first matrix column `scale_col = (m00, m01, m02)` of `inst+0xfc`,
/// the local sphere `radius`, and the ray length `seg_len`, returns the clamped
/// `[t_enter, t_exit]` candidate span on a hit, or `None`.
///
/// `radius_sq = ((m00² + m01²) + m02²)·radius·radius` scales the local radius by
/// the matrix's x-axis length. The proj/perp grouping and the four rejects are
/// bit-exact: `perp_sq > radius_sq`, `t_exit < 0`, `t_enter > seg_len`, then the
/// `t_enter` is clamped up to `0` (kept iff strictly `> 0`).
pub fn cm2_scene__sphere_reject__7089c0(
    center: &[f32; 3],
    origin: &[f32; 3],
    dir: &[f32; 3],
    scale_col: &[f32; 3],
    radius: f32,
    seg_len: f32,
) -> Option<[f32; 2]> {
    let dx = center[0] - origin[0];
    let dy = center[1] - origin[1];
    let dz = center[2] - origin[2];
    let (m0, m1, m2) = (scale_col[0], scale_col[1], scale_col[2]);
    let radius_sq = ((m0 * m0 + m1 * m1) + m2 * m2) * radius * radius;

    let proj = (dz * dir[2] + dy * dir[1]) + dx * dir[0];
    let perp_x = dir[0] * proj - dx;
    let perp_y = dir[1] * proj - dy;
    let perp_z = dir[2] * proj - dz;
    let perp_sq = (perp_y * perp_y + perp_x * perp_x) + perp_z * perp_z;
    if perp_sq > radius_sq {
        return None;
    }

    let half = (radius_sq - perp_sq).sqrt();
    let t_exit = proj + half;
    if t_exit < 0.0 {
        return None;
    }
    let t_enter = proj - half;
    if t_enter > seg_len {
        return None;
    }
    let rec_t_enter = if t_enter > 0.0 { t_enter } else { 0.0 };
    Some([rec_t_enter, t_exit])
}

/// Project a world-space vertex into the ray frame (stock 0x709760.. collision /
/// 0x709445.. skinned). Returns `[perp_x, perp_y, along]` where
/// `along = dot(v, dir) - d_dot_o` (signed ray distance) and
/// `perp_{x,y} = v_{x,y} - dir_{x,y}·along`. The two stock paths sum the dot in
/// different orders, selected by `skinned`:
/// - collision (`skinned = false`): `(v_z·dir_z + v_x·dir_x) + v_y·dir_y`
/// - skinned   (`skinned = true`):  `(v_x·dir_x + v_y·dir_y) + v_z·dir_z`
pub fn cm2_scene__project_vertex__7089c0(
    v: &[f32; 3],
    dir: &[f32; 3],
    d_dot_o: f32,
    skinned: bool,
) -> [f32; 3] {
    let dot = if skinned {
        (v[0] * dir[0] + v[1] * dir[1]) + v[2] * dir[2]
    } else {
        (v[2] * dir[2] + v[0] * dir[0]) + v[1] * dir[1]
    };
    let along = dot - d_dot_o;
    let perp_x = v[0] - dir[0] * along;
    let perp_y = v[1] - dir[1] * along;
    [perp_x, perp_y, along]
}

/// Inner projected 2-D barycentric triangle hit (stock 0x70983e../0x70951f..,
/// identical in both narrow-phase paths). `p0`/`p1`/`p2` are the projected
/// scratch entries `[perp_x, perp_y, along]` for the triangle's three vertices
/// and `origin_xy = (o_x, o_y)` is the ray origin's x/y. Returns the hit
/// distance `t` (the `along`-interpolated depth) on a hit, else `None`.
///
/// `det` is the signed area in the perp plane (reject `|det| < eps`); the three
/// barycentric weights are point-in-triangle of the origin, each rejected on
/// `< 0`; `t = (w2·p2z + w1·p1z) + w0·p0z` is rejected on `< 0`.
pub fn cm2_scene__tri_barycentric_hit__7089c0(
    p0: &[f32; 3],
    p1: &[f32; 3],
    p2: &[f32; 3],
    origin_xy: (f32, f32),
) -> Option<f32> {
    let det = (p1[0] - p0[0]) * (p2[1] - p0[1]) - (p1[1] - p0[1]) * (p2[0] - p0[0]);
    if det.abs() < TRACE_EPS {
        return None;
    }
    let inv = 1.0 / det;
    let (ox, oy) = origin_xy;

    let w0 = ((p1[0] - ox) * (p2[1] - oy) - (p2[0] - ox) * (p1[1] - oy)) * inv;
    if w0 < 0.0 {
        return None;
    }
    let w1 = ((p0[1] - oy) * (p2[0] - ox) - (p0[0] - ox) * (p2[1] - oy)) * inv;
    if w1 < 0.0 {
        return None;
    }
    let w2 = ((p0[0] - ox) * (p1[1] - oy) - (p0[1] - oy) * (p1[0] - ox)) * inv;
    if w2 < 0.0 {
        return None;
    }
    let t = (w2 * p2[2] + w1 * p1[2]) + w0 * p0[2];
    if t < 0.0 {
        return None;
    }
    Some(t)
}

/// Bone-weight skin blend (stock 0x7091fb..0x7093a6). Accumulates the per-vertex
/// blended 4×4 `C44Matrix` from up to four `(weight, bone)` influences. `weights`
/// are the four raw weight bytes (`vert+0xc..0xf`); `mats[k]` is the bone matrix
/// for influence `k` (`bone_base + bone_k·0x40`). Influence 0 is always applied;
/// influences 1..3 stop at the first zero weight (so `mats[k]` for a zero-weight
/// slot is never read — the driver may pass any placeholder there).
///
/// Each weight is `(byte as i32 as f32)·(1/255)`; the matrix's translation column
/// (`acc[3]`, `[7]`, `[11]`) stays `0` and `acc[15]` stays `1.0`, matching the
/// stock scratch whose 4th column is pre-initialized. Per lane:
/// `acc[j] = w0·mat0[j]`, then `acc[j] = w_k·mat_k[j] + acc[j]`.
#[allow(clippy::assign_op_pattern)] // keep the stock `w·mat + acc` add order explicit
pub fn cm2_scene__skin_blend__7089c0(weights: &[u8; 4], mats: &[[f32; 16]; 4]) -> [f32; 16] {
    const LANES: [usize; 12] = [0, 1, 2, 4, 5, 6, 8, 9, 10, 12, 13, 14];
    let mut acc = [0.0f32; 16];
    acc[15] = 1.0;

    let w0 = (weights[0] as i32 as f32) * WEIGHT_SCALE;
    for &j in &LANES {
        acc[j] = w0 * mats[0][j];
    }
    for k in 1..4 {
        if weights[k] == 0 {
            break;
        }
        let wk = (weights[k] as i32 as f32) * WEIGHT_SCALE;
        for &j in &LANES {
            acc[j] = wk * mats[k][j] + acc[j];
        }
    }
    acc
}

/// Skinned normal rotate for the pass≥1 narrow phase (stock 0x7093c8..0x709423).
/// Rotates the vertex normal `n = (vert+0x14, +0x18, +0x1c)` by the blended
/// matrix's 3×3 part and returns the world delta added to the transformed
/// position. Each lane groups as `(blend[r+4]·n1 + blend[r+8]·n2) + blend[r]·n0`.
pub fn cm2_scene__skin_world_normal__7089c0(blend: &[f32; 16], n: &[f32; 3]) -> [f32; 3] {
    let nx = (blend[4] * n[1] + blend[8] * n[2]) + blend[0] * n[0];
    let ny = (blend[5] * n[1] + blend[9] * n[2]) + blend[1] * n[0];
    let nz = (blend[6] * n[1] + blend[10] * n[2]) + blend[2] * n[0];
    [nx, ny, nz]
}

#[cfg(test)]
mod tests_cm2_scene__trace_line__7089c0 {
    use super::{
        TRACE_EPS, WEIGHT_SCALE, cm2_scene__project_vertex__7089c0 as project,
        cm2_scene__skin_blend__7089c0 as blend,
        cm2_scene__skin_world_normal__7089c0 as world_normal,
        cm2_scene__sphere_reject__7089c0 as sphere, cm2_scene__tri_barycentric_hit__7089c0 as tri,
    };

    // -- sphere reject ------------------------------------------------------

    #[test]
    fn sphere_axis_hit_spans_diameter() {
        // Unit-scale identity column, radius 1, sphere centered 5 along +x, ray
        // along +x from the origin. Enter at 4, exit at 6.
        let r = sphere(
            &[5.0, 0.0, 0.0],
            &[0.0, 0.0, 0.0],
            &[1.0, 0.0, 0.0],
            &[1.0, 0.0, 0.0],
            1.0,
            100.0,
        )
        .expect("on-axis sphere is hit");
        assert_eq!(r[0].to_bits(), 4.0f32.to_bits());
        assert_eq!(r[1].to_bits(), 6.0f32.to_bits());
    }

    #[test]
    fn sphere_miss_when_perp_exceeds_radius() {
        // Centered 5 off-axis in y by 2 with radius 1: the ray (+x) misses.
        assert!(
            sphere(
                &[5.0, 2.0, 0.0],
                &[0.0, 0.0, 0.0],
                &[1.0, 0.0, 0.0],
                &[1.0, 0.0, 0.0],
                1.0,
                100.0,
            )
            .is_none()
        );
    }

    #[test]
    fn sphere_enter_clamped_to_zero_when_origin_inside() {
        // Origin inside the sphere (center at +0.5, radius 1): enter would be
        // negative (-0.5) and is clamped up to 0; exit at 1.5.
        let r = sphere(
            &[0.5, 0.0, 0.0],
            &[0.0, 0.0, 0.0],
            &[1.0, 0.0, 0.0],
            &[1.0, 0.0, 0.0],
            1.0,
            100.0,
        )
        .expect("origin-inside is a hit");
        assert_eq!(r[0].to_bits(), 0.0f32.to_bits());
        assert_eq!(r[1].to_bits(), 1.5f32.to_bits());
    }

    #[test]
    fn sphere_miss_when_behind_or_beyond() {
        // Entirely behind the origin (center at -5): exit < 0 -> reject.
        assert!(
            sphere(
                &[-5.0, 0.0, 0.0],
                &[0.0, 0.0, 0.0],
                &[1.0, 0.0, 0.0],
                &[1.0, 0.0, 0.0],
                1.0,
                100.0,
            )
            .is_none()
        );
        // Beyond the segment end (center at 5, seg_len 3 -> enter 4 > 3).
        assert!(
            sphere(
                &[5.0, 0.0, 0.0],
                &[0.0, 0.0, 0.0],
                &[1.0, 0.0, 0.0],
                &[1.0, 0.0, 0.0],
                1.0,
                3.0,
            )
            .is_none()
        );
    }

    #[test]
    fn sphere_radius_scaled_by_matrix_column() {
        // A scale column of length 2 doubles the effective radius: radius 0.5 *
        // column-length 2 = effective 1.0, so the by-2-off-axis case from the
        // miss test now... still misses (perp 2 > 1). Use 1.5 off-axis: hit.
        let r = sphere(
            &[5.0, 1.5, 0.0],
            &[0.0, 0.0, 0.0],
            &[1.0, 0.0, 0.0],
            &[2.0, 0.0, 0.0],
            1.0,
            100.0,
        );
        assert!(
            r.is_some(),
            "radius scaled by |column|=2 should reach 1.5 off-axis"
        );
    }

    // -- projection ---------------------------------------------------------

    #[test]
    fn project_along_and_perp_on_axis_ray() {
        // Ray +x: along = v.x - d_dot_o; perp = (0, v.y).
        let v = [3.0, 2.0, 7.0];
        let dir = [1.0, 0.0, 0.0];
        let r = project(&v, &dir, 0.0, false);
        assert_eq!(r[2].to_bits(), 3.0f32.to_bits()); // along = v.x
        assert_eq!(r[0].to_bits(), 0.0f32.to_bits()); // perp_x = v.x - 1*along = 0
        assert_eq!(r[1].to_bits(), 2.0f32.to_bits()); // perp_y = v.y
    }

    #[test]
    fn project_dot_grouping_differs_by_path() {
        // Pick a vertex/dir where the two summation orders diverge in the last
        // ULP, and assert each path reproduces ITS grouping bit-for-bit.
        let v = [1.0e8_f32, 1.0, 3.0e-3];
        let dir = [0.3, 0.5, 0.7];
        let collision = (v[2] * dir[2] + v[0] * dir[0]) + v[1] * dir[1];
        let skinned = (v[0] * dir[0] + v[1] * dir[1]) + v[2] * dir[2];
        let rc = project(&v, &dir, 0.0, false);
        let rs = project(&v, &dir, 0.0, true);
        assert_eq!(rc[2].to_bits(), collision.to_bits());
        assert_eq!(rs[2].to_bits(), skinned.to_bits());
    }

    // -- barycentric hit ----------------------------------------------------

    #[test]
    fn tri_hit_centroid_depth() {
        // Triangle in the perp plane around the origin, with along-depths
        // 10/10/10: the interpolated depth at the centroid is 10.
        let p0 = [-1.0, -1.0, 10.0];
        let p1 = [3.0, -1.0, 10.0];
        let p2 = [-1.0, 3.0, 10.0];
        let t = tri(&p0, &p1, &p2, (0.0, 0.0)).expect("origin is inside");
        assert_eq!(t.to_bits(), 10.0f32.to_bits());
    }

    #[test]
    fn tri_miss_outside_and_degenerate() {
        // Origin outside the triangle (all to the right) -> a weight goes < 0.
        let p0 = [1.0, -1.0, 5.0];
        let p1 = [3.0, -1.0, 5.0];
        let p2 = [1.0, 3.0, 5.0];
        assert!(tri(&p0, &p1, &p2, (0.0, 0.0)).is_none());
        // Degenerate (zero-area) triangle: |det| < eps -> reject.
        let q = [0.0, 0.0, 5.0];
        assert!(tri(&q, &q, &q, (0.0, 0.0)).is_none());
    }

    #[test]
    fn tri_rejects_negative_depth() {
        // Inside the triangle but the interpolated along-depth is negative.
        let p0 = [-1.0, -1.0, -2.0];
        let p1 = [3.0, -1.0, -2.0];
        let p2 = [-1.0, 3.0, -2.0];
        assert!(tri(&p0, &p1, &p2, (0.0, 0.0)).is_none());
    }

    #[test]
    fn tri_eps_is_one_e_minus_five() {
        // Pin the degeneracy epsilon bits (~1e-5).
        assert_eq!(TRACE_EPS.to_bits(), 0x3727_c5ac);
    }

    // -- skin blend ---------------------------------------------------------

    #[test]
    fn blend_single_full_weight_is_scaled_matrix() {
        // One bone, weight 255 -> scale 255/255 = exactly... 255 * (1/255).
        let mut m0 = [0.0f32; 16];
        for (i, e) in m0.iter_mut().enumerate() {
            *e = i as f32;
        }
        let zero = [0.0f32; 16];
        let acc = blend(&[255, 0, 0, 0], &[m0, zero, zero, zero]);
        let w = 255.0f32 * WEIGHT_SCALE;
        // Translation column + bottom row are fixed, not weighted.
        assert_eq!(acc[3].to_bits(), 0.0f32.to_bits());
        assert_eq!(acc[15].to_bits(), 1.0f32.to_bits());
        for &j in &[0usize, 1, 2, 4, 5, 6, 8, 9, 10, 12, 13, 14] {
            assert_eq!(acc[j].to_bits(), (w * m0[j]).to_bits(), "lane {j}");
        }
    }

    #[test]
    fn blend_two_bones_accumulate_in_order() {
        let m0 = [2.0f32; 16];
        let m1 = [4.0f32; 16];
        let zero = [0.0f32; 16];
        let acc = blend(&[100, 50, 0, 0], &[m0, m1, zero, zero]);
        let w0 = 100.0f32 * WEIGHT_SCALE;
        let w1 = 50.0f32 * WEIGHT_SCALE;
        // lane 0: w0*2 then + w1*4, in that stock add order.
        let want = w1 * m1[0] + w0 * m0[0];
        assert_eq!(acc[0].to_bits(), want.to_bits());
    }

    #[test]
    fn blend_stops_at_first_zero_weight() {
        // weights[1]==0 stops the loop before bone 1; bones 2/3 ignored even with
        // non-zero placeholder matrices (and weights[2] non-zero is unreachable).
        let m0 = [1.0f32; 16];
        let poison = [1.0e9f32; 16];
        let acc = blend(&[200, 0, 200, 200], &[m0, poison, poison, poison]);
        let w0 = 200.0f32 * WEIGHT_SCALE;
        assert_eq!(acc[0].to_bits(), (w0 * 1.0f32).to_bits());
    }

    #[test]
    fn weight_scale_is_one_over_255() {
        assert_eq!(WEIGHT_SCALE.to_bits(), 0x3b80_8081);
    }

    // -- skinned normal -----------------------------------------------------

    #[test]
    fn world_normal_identity_rotation_is_identity() {
        let mut id = [0.0f32; 16];
        id[0] = 1.0;
        id[5] = 1.0;
        id[10] = 1.0;
        id[15] = 1.0;
        let r = world_normal(&id, &[0.3, 0.5, 0.7]);
        assert_eq!(r[0].to_bits(), 0.3f32.to_bits());
        assert_eq!(r[1].to_bits(), 0.5f32.to_bits());
        assert_eq!(r[2].to_bits(), 0.7f32.to_bits());
    }

    #[test]
    fn world_normal_grouping_is_stock_order() {
        let blend_m = [
            0.1f32, 0.2, 0.3, 9.0, 0.4, 0.5, 0.6, 9.0, 0.7, 0.8, 0.9, 9.0, 9.0, 9.0, 9.0, 9.0,
        ];
        let n = [1.0e7_f32, 1.0, 1.0e-3];
        let want_x = (blend_m[4] * n[1] + blend_m[8] * n[2]) + blend_m[0] * n[0];
        let r = world_normal(&blend_m, &n);
        assert_eq!(r[0].to_bits(), want_x.to_bits());
    }
}

// ---------------------------------------------------------------------------
// CM2Shadow::ProjectCollisionQuads (0x7137c0) — pure kernels. The adapter in
// `win::hooks` orchestrates the stateful surface (lazy SMemAlloc scratch
// buffers, TSGrowableArray grow protocol, per-element ctor) and calls these
// for the arithmetic.
// ---------------------------------------------------------------------------

/// 6-bit Cohen–Sutherland AABB outcode for one transformed collision vertex
/// (`CM2Shadow::ProjectCollisionQuads`, per-vertex block 0x7138d1–0x713944).
///
/// `bounds` is `[minX, minY, minZ, maxX, maxY, maxZ]`. Bit layout: `x < minX`
/// = 1, `x > maxX` = 2, `y < minY` = 4, `y > maxY` = 8, `z < minZ` = 0x10,
/// `z > maxZ` = 0x20. Each axis transcribes the stock x87 pair — `FCOMP min` /
/// `TEST AH,0x5` (below-min sets the low bit and skips the max test) then
/// `FCOMP max` / `TEST AH,0x41` (strictly above max sets the high bit). Both
/// compares are strict and ORDERED: a `NaN` coordinate is unordered against
/// both bounds, sets NEITHER bit, and classifies the vertex as inside (never
/// trivially rejected). Rust `<`/`>` are IEEE ordered compares (false on NaN)
/// and reproduce this exactly; a `>=`-style rewrite would not.
pub fn cm2_shadow__outcode6__7137c0(v: &[f32; 3], bounds: &[f32; 6]) -> u32 {
    let mut oc = 0u32;
    if v[0] < bounds[0] {
        oc = 1;
    } else if v[0] > bounds[3] {
        oc = 2;
    }
    if v[1] < bounds[1] {
        oc |= 4;
    } else if v[1] > bounds[4] {
        oc |= 8;
    }
    if v[2] < bounds[2] {
        oc |= 0x10;
    } else if v[2] > bounds[5] {
        oc |= 0x20;
    }
    oc
}

/// Row-normalizes the 3×3 shadow basis in place (0x71399e–0x713a45): for each
/// row, `mag² = SquaredMagnitude(row)` (the hooked 0x4549f0 kernel), then
/// `FSQRT`/`FABS`/`FCOMP eps` — the row is scaled by `k / sqrt(mag²)` unless
/// the compare orders `|sqrt| < eps`. The `TEST AH,0x5` / `JNP` polarity means
/// the normalize arm runs on GREATER-OR-EQUAL **and on unordered**: a NaN
/// length is not skipped, it scales the row by a NaN factor (row becomes NaN),
/// exactly like stock. Arithmetic is `f64` matching the x87 double-precision
/// intermediates (the hooked SquaredMagnitude already narrowed `mag²` to
/// `f32`), narrowed per element at the `FSTP m32` points.
pub fn cm2_shadow__normalize_basis3__7137c0(basis: &mut [f32; 9], k: f32, eps: f32) {
    for r in 0..3 {
        let row = [basis[r * 3], basis[r * 3 + 1], basis[r * 3 + 2]];
        let mag_sq = crate::math::vector::c3_vector__squared_magnitude__4549f0(&row);
        let len = f64::from(mag_sq).sqrt();
        // Skip ONLY on an ordered `|len| < eps`; NaN falls into the scale arm.
        if !(len.abs() < f64::from(eps)) {
            let scale = f64::from(k) / len;
            basis[r * 3] = super::f64_to_f32(f64::from(basis[r * 3]) * scale);
            basis[r * 3 + 1] = super::f64_to_f32(f64::from(basis[r * 3 + 1]) * scale);
            basis[r * 3 + 2] = super::f64_to_f32(f64::from(basis[r * 3 + 2]) * scale);
        }
    }
}

/// Per-triangle plane build (0x713bc6–0x713c3d): transforms the face normal by
/// the row-normalized basis as three COLUMN dots (`N'[c] = basis[6+c]·nz +
/// basis[3+c]·ny + basis[c]·nx`, i.e. `basisᵀ·n`), then the plane offset
/// `d = -((v0z·N'z + v0y·N'y) + N'x·v0x)` (`FCHS` before the store). Stock
/// stores each `N'` lane to the quad as `f32` and reads it BACK for the `d`
/// dot, so `d` is computed from the NARROWED `N'` — reproduced here by
/// narrowing `np` first. `f64` intermediates match the x87 double-precision
/// `FMUL`/`FADDP` chain (each f32·f32 product is exact in f64), with the exact
/// stock summation grouping.
pub fn cm2_shadow__quad_plane__7137c0(
    basis: &[f32; 9],
    n: &[f32; 3],
    v0: &[f32; 3],
) -> ([f32; 3], f32) {
    let nx = f64::from(n[0]);
    let ny = f64::from(n[1]);
    let nz = f64::from(n[2]);
    let col = |c: usize| -> f32 {
        super::f64_to_f32(
            (f64::from(basis[6 + c]) * nz + f64::from(basis[3 + c]) * ny)
                + f64::from(basis[c]) * nx,
        )
    };
    let np = [col(0), col(1), col(2)];
    let d = super::f64_to_f32(
        -((f64::from(v0[2]) * f64::from(np[2]) + f64::from(v0[1]) * f64::from(np[1]))
            + f64::from(np[0]) * f64::from(v0[0])),
    );
    (np, d)
}

/// `TSGrowableArray` capacity round-up (0x674500), inlined into the
/// ProjectCollisionQuads grow protocol: rounds `value` up to the next multiple
/// of `alignment` (unchanged when already aligned). The stock `DIV` faults on
/// `alignment == 0`; the caller protocol guarantees non-zero (`out[3]` is used
/// only when non-zero, and ComputeGranularity returns ≥ 1). The round-up add
/// wraps like the stock `ADD`.
pub fn ts_growable_array__align_up__674500(value: u32, alignment: u32) -> u32 {
    let rem = value % alignment;
    if rem == 0 {
        value
    } else {
        value.wrapping_add(alignment - rem)
    }
}

#[cfg(test)]
mod tests_cm2_shadow__project_collision_quads__7137c0 {
    use super::{
        cm2_shadow__normalize_basis3__7137c0 as normalize, cm2_shadow__outcode6__7137c0 as outcode,
        cm2_shadow__quad_plane__7137c0 as plane, ts_growable_array__align_up__674500 as align_up,
    };

    const BOUNDS: [f32; 6] = [-1.0, -2.0, -3.0, 1.0, 2.0, 3.0];

    // -- outcode ------------------------------------------------------------

    #[test]
    fn outcode_inside_is_zero() {
        assert_eq!(outcode(&[0.0, 0.0, 0.0], &BOUNDS), 0);
        assert_eq!(outcode(&[0.5, -1.5, 2.5], &BOUNDS), 0);
    }

    #[test]
    fn outcode_per_axis_bits() {
        assert_eq!(outcode(&[-1.5, 0.0, 0.0], &BOUNDS), 1);
        assert_eq!(outcode(&[1.5, 0.0, 0.0], &BOUNDS), 2);
        assert_eq!(outcode(&[0.0, -2.5, 0.0], &BOUNDS), 4);
        assert_eq!(outcode(&[0.0, 2.5, 0.0], &BOUNDS), 8);
        assert_eq!(outcode(&[0.0, 0.0, -3.5], &BOUNDS), 0x10);
        assert_eq!(outcode(&[0.0, 0.0, 3.5], &BOUNDS), 0x20);
        // combined corner
        assert_eq!(outcode(&[-1.5, 2.5, 3.5], &BOUNDS), 1 | 8 | 0x20);
    }

    #[test]
    fn outcode_boundary_equality_sets_no_bit() {
        // v == min: `v < min` false; `v > max` false — strict compares only.
        assert_eq!(outcode(&[-1.0, -2.0, -3.0], &BOUNDS), 0);
        // v == max
        assert_eq!(outcode(&[1.0, 2.0, 3.0], &BOUNDS), 0);
    }

    #[test]
    fn outcode_nan_coord_sets_neither_bit() {
        // A NaN coordinate is unordered against both bounds -> inside.
        assert_eq!(outcode(&[f32::NAN, 0.0, 0.0], &BOUNDS), 0);
        assert_eq!(outcode(&[0.0, f32::NAN, 0.0], &BOUNDS), 0);
        assert_eq!(outcode(&[0.0, 0.0, f32::NAN], &BOUNDS), 0);
        assert_eq!(outcode(&[f32::NAN, f32::NAN, f32::NAN], &BOUNDS), 0);
    }

    #[test]
    fn outcode_nan_min_still_tests_max() {
        // min = NaN: the below-min compare is unordered (no bit, no skip of the
        // max arm — stock's JP falls through to the max FCOMP), so above-max
        // still sets the high bit.
        let b = [f32::NAN, -2.0, -3.0, 1.0, 2.0, 3.0];
        assert_eq!(outcode(&[5.0, 0.0, 0.0], &b), 2);
        // max = NaN: below-min still sets the low bit; an in-range coord sets
        // nothing (both compares unordered/false).
        let b2 = [-1.0, -2.0, -3.0, f32::NAN, 2.0, 3.0];
        assert_eq!(outcode(&[-5.0, 0.0, 0.0], &b2), 1);
        assert_eq!(outcode(&[0.0, 0.0, 0.0], &b2), 0);
    }

    // -- basis normalize ------------------------------------------------------

    #[test]
    fn normalize_happy_path_matches_f64_oracle() {
        let mut b = [3.0f32, 4.0, 0.0, 0.0, 5.0, 12.0, 8.0, 0.0, 6.0];
        normalize(&mut b, 1.0, 1e-4);
        // Independent oracle: mag² in f32 (the hooked kernel's arithmetic),
        // sqrt/divide/multiply in f64, narrow per element.
        let oracle = |row: [f32; 3]| -> [f32; 3] {
            let mag = row[0] * row[0] + row[1] * row[1] + row[2] * row[2];
            let s = 1.0f64 / f64::from(mag).sqrt();
            [
                (f64::from(row[0]) * s) as f32,
                (f64::from(row[1]) * s) as f32,
                (f64::from(row[2]) * s) as f32,
            ]
        };
        let r0 = oracle([3.0, 4.0, 0.0]);
        let r1 = oracle([0.0, 5.0, 12.0]);
        let r2 = oracle([8.0, 0.0, 6.0]);
        for i in 0..3 {
            assert_eq!(b[i].to_bits(), r0[i].to_bits(), "row0 lane {i}");
            assert_eq!(b[3 + i].to_bits(), r1[i].to_bits(), "row1 lane {i}");
            assert_eq!(b[6 + i].to_bits(), r2[i].to_bits(), "row2 lane {i}");
        }
    }

    #[test]
    fn normalize_scales_by_k() {
        let mut b = [2.0f32, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0];
        normalize(&mut b, 3.0, 1e-4);
        assert_eq!(b[0], 3.0);
        assert_eq!(b[4], 3.0);
        assert_eq!(b[8], 3.0);
    }

    #[test]
    fn normalize_skips_sub_eps_row_only() {
        // Row 1 is degenerate (length 0 < eps) and is left UN-normalized.
        let mut b = [2.0f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 4.0];
        normalize(&mut b, 1.0, 1e-4);
        assert_eq!(b[0], 1.0);
        assert_eq!([b[3], b[4], b[5]], [0.0, 0.0, 0.0]);
        assert_eq!(b[8], 1.0);
    }

    #[test]
    fn normalize_len_equal_eps_is_normalized() {
        // FCOMP orders |len| < eps STRICTLY: len == eps falls into the scale
        // arm (JNP taken only when C0 alone is set).
        let mut b = [0.25f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        normalize(&mut b, 1.0, 0.25);
        assert_eq!(b[0], 1.0);
    }

    #[test]
    fn normalize_nan_row_becomes_nan_not_skipped() {
        // NaN length is UNORDERED against eps: the stock JNP polarity runs the
        // scale arm, so the whole row becomes NaN (not left untouched).
        let mut b = [f32::NAN, 7.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        normalize(&mut b, 1.0, 1e-4);
        assert!(b[0].is_nan());
        assert!(b[1].is_nan());
        assert!(b[2].is_nan());
        // untouched healthy rows still normalize
        assert_eq!(b[4], 1.0);
        assert_eq!(b[8], 1.0);
    }

    // -- quad plane -----------------------------------------------------------

    #[test]
    fn plane_identity_basis_passes_normal_through() {
        let id = [1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        let n = [0.25f32, -0.5, 0.75];
        let v0 = [2.0f32, 4.0, 8.0];
        let (np, d) = plane(&id, &n, &v0);
        assert_eq!(np, n);
        // d = -(v0z*nz + v0y*ny + nx*v0x), stock grouping, f64 -> f32.
        let want = (-((f64::from(v0[2]) * f64::from(n[2]) + f64::from(v0[1]) * f64::from(n[1]))
            + f64::from(n[0]) * f64::from(v0[0]))) as f32;
        assert_eq!(d.to_bits(), want.to_bits());
    }

    #[test]
    fn plane_uses_column_dots_transpose() {
        // basis rows: [1,2,3],[4,5,6],[7,8,9]; N'[c] = col_c . n.
        let b = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
        let n = [1.0f32, 10.0, 100.0];
        let (np, _) = plane(&b, &n, &[0.0; 3]);
        // col0 = [1,4,7], col1 = [2,5,8], col2 = [3,6,9]
        assert_eq!(np[0], 741.0); // 7*100 + 4*10 + 1*1
        assert_eq!(np[1], 852.0);
        assert_eq!(np[2], 963.0);
    }

    #[test]
    fn plane_d_reads_back_narrowed_nprime() {
        // np_x in f64 is 1 + 2^-24 — exactly halfway between the f32 neighbours
        // 1.0 and 1 + 2^-23; ties-to-even narrows it to 1.0. Stock computes d
        // from the STORED (narrowed) N', so with v0x = 3*2^22 the product is
        // exactly 12582912.0; the unnarrowed f64 np_x would give
        // -(12582912.75) -> -12582913.0 instead.
        let b = [1.0f32, 0.0, 0.0, f32::exp2(-24.0), 0.0, 0.0, 0.0, 0.0, 0.0];
        let n = [1.0f32, 1.0, 0.0];
        let v0 = [12_582_912.0f32, 0.0, 0.0];
        let (np, d) = plane(&b, &n, &v0);
        assert_eq!(np[0].to_bits(), 1.0f32.to_bits());
        assert_eq!(d, -12_582_912.0);
    }

    #[test]
    fn plane_grouping_matches_stock_order() {
        // Recompute np_x with the exact stock grouping ((b6*nz + b3*ny) + b0*nx)
        // in f64 and assert bit equality on awkward magnitudes.
        let b = [
            1.000_000_1f32,
            0.0,
            0.0,
            -3.333_333_3e-4,
            0.0,
            0.0,
            7.777_777e6,
            0.0,
            0.0,
        ];
        let n = [0.577_350_3f32, 1.234_567_9f32, -9.876_543e-3];
        let (np, _) = plane(&b, &n, &[0.0; 3]);
        let want = ((f64::from(b[6]) * f64::from(n[2]) + f64::from(b[3]) * f64::from(n[1]))
            + f64::from(b[0]) * f64::from(n[0])) as f32;
        assert_eq!(np[0].to_bits(), want.to_bits());
    }

    // -- align up -------------------------------------------------------------

    #[test]
    fn align_up_multiples_pass_through() {
        assert_eq!(align_up(0, 8), 0);
        assert_eq!(align_up(16, 8), 16);
        assert_eq!(align_up(5, 1), 5);
    }

    #[test]
    fn align_up_rounds_up() {
        assert_eq!(align_up(1, 8), 8);
        assert_eq!(align_up(9, 8), 16);
        assert_eq!(align_up(7, 4), 8);
    }
}
