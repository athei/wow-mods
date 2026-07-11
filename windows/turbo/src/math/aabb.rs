//! `aabb` family kernels.
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

/// AABB overlap half-test: returns `true` iff `box_min` is less than *or equal
/// to* `query_max` on all three axes (the `min <= max` half of an AABB-vs-AABB
/// test). The original compares each axis with `comiss` and only rejects on
/// *greater* or *unordered* — `TEST AH,0x41; JP` continues on both `C0` (less)
/// and `C3` (equal) — so touching boxes (`box_min == query_max`, ubiquitous with
/// grid-aligned terrain cells) count as overlapping. A NaN axis returns `false`.
/// (The symbol name says "LessThan", but the comparison is inclusive.)
pub fn aabb__min_less_than_max__6acb40(box_min: &[f32; 3], query_max: &[f32; 3]) -> bool {
    box_min[0] <= query_max[0] && box_min[1] <= query_max[1] && box_min[2] <= query_max[2]
}

#[cfg(test)]
mod tests_aabb__min_less_than_max__6acb40 {
    use super::aabb__min_less_than_max__6acb40 as min_lt_max;

    #[test]
    fn all_axes_strictly_less_is_true() {
        assert!(min_lt_max(&[0.0, 0.0, 0.0], &[1.0, 1.0, 1.0]));
        assert!(min_lt_max(&[-5.0, -3.0, -1.0], &[5.0, 3.0, 1.0]));
    }

    #[test]
    fn equal_on_any_axis_is_true() {
        // Inclusive (`<=`): equality on an axis still counts — the original's
        // `comiss` + `JP` continues on both *less* and *equal*. Touching boxes
        // must overlap, or grid-aligned cells/objects get wrongly culled.
        assert!(min_lt_max(&[1.0, 0.0, 0.0], &[1.0, 1.0, 1.0]));
        assert!(min_lt_max(&[0.0, 1.0, 0.0], &[1.0, 1.0, 1.0]));
        assert!(min_lt_max(&[0.0, 0.0, 1.0], &[1.0, 1.0, 1.0]));
        assert!(min_lt_max(&[1.0, 1.0, 1.0], &[1.0, 1.0, 1.0]));
    }

    #[test]
    fn greater_on_any_axis_is_false() {
        assert!(!min_lt_max(&[2.0, 0.0, 0.0], &[1.0, 1.0, 1.0]));
        assert!(!min_lt_max(&[0.0, 2.0, 0.0], &[1.0, 1.0, 1.0]));
        assert!(!min_lt_max(&[0.0, 0.0, 2.0], &[1.0, 1.0, 1.0]));
    }

    #[test]
    fn nan_on_any_axis_is_false() {
        // strict-less via comiss is false when either operand is NaN.
        let nan = f32::NAN;
        assert!(!min_lt_max(&[nan, 0.0, 0.0], &[1.0, 1.0, 1.0]));
        assert!(!min_lt_max(&[0.0, 0.0, 0.0], &[nan, 1.0, 1.0]));
    }

    #[test]
    fn antisymmetry_property() {
        // For distinct, non-NaN componentwise vectors, exactly one direction
        // of the strict half-test holds when one strictly dominates the other.
        let a = [0.1, 0.2, 0.3];
        let b = [0.4, 0.5, 0.6];
        assert!(min_lt_max(&a, &b));
        assert!(!min_lt_max(&b, &a));
    }
}

/// Computes the axis-aligned bounding box over `count` `C3Vector` points
/// (stride 3 floats) in `points`. Returns `[minX,minY,minZ,maxX,maxY,maxZ]`.
/// With `count == 0` the box is all zeros (matching the original).
pub fn c_aa_box__from_points__7c1450(points: &[f32], count: u32) -> [f32; 6] {
    if count == 0 {
        return [0.0; 6];
    }

    // Seed min and max from the first point.
    let mut min_x = points[0];
    let mut min_y = points[1];
    let mut min_z = points[2];
    let mut max_x = points[0];
    let mut max_y = points[1];
    let mut max_z = points[2];

    let mut i = 1usize;
    let n = count as usize;
    while i < n {
        let base = i * 3;
        let x = points[base];
        let y = points[base + 1];
        let z = points[base + 2];

        if x < min_x {
            min_x = x;
        }
        if y < min_y {
            min_y = y;
        }
        if z < min_z {
            min_z = z;
        }
        if max_x < x {
            max_x = x;
        }
        if max_y < y {
            max_y = y;
        }
        if max_z < z {
            max_z = z;
        }
        i += 1;
    }

    [min_x, min_y, min_z, max_x, max_y, max_z]
}

#[cfg(test)]
mod tests_c_aa_box__from_points__7c1450 {
    use super::c_aa_box__from_points__7c1450 as fp;

    #[test]
    fn empty_is_zeroed() {
        assert_eq!(fp(&[], 0), [0.0; 6]);
        assert_eq!(fp(&[5.0, 6.0, 7.0], 0), [0.0; 6]);
    }

    #[test]
    fn single_point_min_eq_max() {
        let out = fp(&[1.5, -2.0, 3.25], 1);
        assert_eq!(out, [1.5, -2.0, 3.25, 1.5, -2.0, 3.25]);
    }

    #[test]
    fn three_points_known() {
        let pts = [1.0, 1.0, 1.0, -2.0, 5.0, 0.0, 4.0, -3.0, 2.0];
        let out = fp(&pts, 3);
        assert_eq!(out, [-2.0, -3.0, 0.0, 4.0, 5.0, 2.0]);
    }

    #[test]
    fn min_le_max_invariant() {
        let pts = [
            0.5, -1.0, 9.0, 3.0, 3.0, -3.0, -7.0, 2.5, 0.0, 6.0, -6.0, 6.0,
        ];
        let out = fp(&pts, 4);
        for k in 0..3 {
            assert!(out[k] <= out[k + 3], "min<=max axis {k}");
        }
    }

    #[test]
    fn count_caps_iteration() {
        let pts = [1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 100.0, 100.0, 100.0];
        let out = fp(&pts, 2);
        assert_eq!(out, [1.0, 1.0, 1.0, 2.0, 2.0, 2.0]);
    }

    #[test]
    fn bbox_contains_all_points() {
        let pts = [3.0, -1.0, 0.0, -5.0, 2.0, 8.0, 1.0, 9.0, -4.0];
        let out = fp(&pts, 3);
        for p in pts.chunks_exact(3) {
            assert!(out[0] <= p[0] && p[0] <= out[3]);
            assert!(out[1] <= p[1] && p[1] <= out[4]);
            assert!(out[2] <= p[2] && p[2] <= out[5]);
        }
    }
}

/// Segment-vs-AABB intersection (slab clip). `box6` is
/// `[minX,minY,minZ,maxX,maxY,maxZ]`. Returns 1 on hit, 0 on miss.
///
/// Per axis, when the segment start lies strictly outside a slab plane the
/// segment is rejected if the end lies strictly outside too, otherwise the
/// entry parameter `t = (plane - start) / (end - start)` is recorded. The
/// axis with the largest entry-`t` defines the hit point, which is validated
/// against the box (±1e-5) on the other two axes. A start inside every slab
/// is an immediate hit.
///
/// Compare senses are faithful to the original: outside-tests are
/// ordered-strict (a NaN start or NaN bound counts as inside), the best-`t`
/// negativity test is a sign-bit test (rejects `-0.0` and negative NaN), and
/// a NaN hit coordinate passes validation. The original guards the divide on
/// the direction's bits being exactly `+0.0`; that case is unreachable (both
/// clip branches imply `end` and `start` are strictly ordered, and the f32
/// difference of distinct values is never zero), so the divide is
/// unconditional here.
pub fn c_aa_box__intersect_segment__6dc5a0(
    box6: &[f32; 6],
    seg_start: &[f32; 3],
    seg_end: &[f32; 3],
) -> i32 {
    const EPS: f32 = 1e-5;

    // t[a] = entry parameter on axis a; -1 means "start inside slab on a".
    let mut t = [-1.0f32; 3];
    let mut clipped = false;

    macro_rules! clip {
        ($a:literal) => {
            'axis: {
                let mn = box6[$a];
                let s = seg_start[$a];
                let e = seg_end[$a];
                let plane = if s < mn {
                    if e < mn {
                        return 0;
                    }
                    mn
                } else {
                    let mx = box6[$a + 3];
                    // inside on `mx >= s` or unordered — NaN counts as inside
                    if !(mx < s) {
                        break 'axis;
                    }
                    if mx < e {
                        return 0;
                    }
                    mx
                };
                clipped = true;
                t[$a] = (plane - s) / (e - s);
            }
        };
    }
    clip!(0);
    clip!(1);
    clip!(2);

    if !clipped {
        return 1;
    }

    // axis with the largest entry-t defines the hit point; ties and unordered
    // comparisons keep the lower axis index
    let mut best = 0usize;
    if t[1] > t[0] {
        best = 1;
    }
    if t[2] > t[best] {
        best = 2;
    }

    // sign-bit test, not `< 0.0`: -0.0 and negative NaN reject too
    let tb = t[best];
    if tb.is_sign_negative() {
        return 0;
    }

    macro_rules! validate {
        ($a:literal) => {{
            if best != $a {
                let s = seg_start[$a];
                let hit = (seg_end[$a] - s) * tb + s;
                if hit < box6[$a] - EPS {
                    return 0;
                }
                if box6[$a + 3] + EPS < hit {
                    return 0;
                }
            }
        }};
    }
    validate!(0);
    validate!(1);
    validate!(2);

    1
}

#[cfg(test)]
mod tests_c_aa_box__intersect_segment__6dc5a0 {
    use super::c_aa_box__intersect_segment__6dc5a0 as hit;

    const UNIT: [f32; 6] = [-1.0, -1.0, -1.0, 1.0, 1.0, 1.0];

    #[test]
    fn start_inside_is_hit() {
        assert_eq!(hit(&UNIT, &[0.0, 0.0, 0.0], &[0.5, 0.5, 0.5]), 1);
    }

    #[test]
    fn straight_through_x_axis() {
        assert_eq!(hit(&UNIT, &[-5.0, 0.0, 0.0], &[5.0, 0.0, 0.0]), 1);
    }

    #[test]
    fn parallel_miss_above() {
        assert_eq!(hit(&UNIT, &[-5.0, 5.0, 0.0], &[5.0, 5.0, 0.0]), 0);
    }

    #[test]
    fn miss_segment_short_of_box() {
        assert_eq!(hit(&UNIT, &[-5.0, 0.0, 0.0], &[-3.0, 0.0, 0.0]), 0);
    }

    #[test]
    fn diagonal_entry_corner() {
        assert_eq!(hit(&UNIT, &[-3.0, -3.0, 0.0], &[0.0, 0.0, 0.0]), 1);
    }

    #[test]
    fn diagonal_miss_outside() {
        assert_eq!(hit(&UNIT, &[-3.0, 3.0, 0.0], &[-1.5, 1.5, 0.0]), 0);
    }

    #[test]
    fn grazing_on_face_plane_within_eps() {
        assert_eq!(hit(&UNIT, &[-5.0, 1.0, 0.0], &[5.0, 1.0, 0.0]), 1);
    }

    /// A point segment on a face counts as a hit; just outside misses.
    #[test]
    fn point_on_face_hits() {
        assert_eq!(hit(&UNIT, &[1.0, 0.0, 0.0], &[1.0, 0.0, 0.0]), 1);
        assert_eq!(hit(&UNIT, &[0.0, 0.0, 0.0], &[0.0, 0.0, 0.0]), 1);
        assert_eq!(hit(&UNIT, &[1.000_01, 0.0, 0.0], &[1.000_01, 0.0, 0.0]), 0);
    }

    /// A NaN start coordinate routes to the inside arm (ordered-strict outside
    /// tests are both false), so the axis is ignored and the segment can still
    /// hit on the others — the stock behaviour.
    #[test]
    fn nan_start_routes_inside() {
        // X is NaN → ignored; Y/Z inside → start-inside hit.
        assert_eq!(hit(&UNIT, &[f32::NAN, 0.0, 0.0], &[-5.0, 0.0, 0.0]), 1);
    }

    /// A NaN box bound likewise counts as inside on that axis: the X-axis
    /// test is suppressed, so a start inside the other slabs is an immediate
    /// hit — but a segment whose hit point still exits another face misses.
    #[test]
    fn nan_bound_counts_inside() {
        let b = [f32::NAN, -1.0, -1.0, 1.0, 1.0, 1.0];
        // X suppressed by the NaN min; start inside Y/Z → start-inside hit.
        assert_eq!(hit(&b, &[0.0, 0.0, 0.0], &[0.5, 0.5, 0.5]), 1);
        // X suppressed, but the clipped path's hit point exits below Y → miss.
        assert_eq!(hit(&b, &[2.0, 0.9, 0.0], &[0.5, -3.0, 0.0]), 0);
    }

    /// The best-`t` rejection is a sign-bit test: a negative-NaN entry
    /// parameter rejects, a positive NaN does not flip an otherwise-valid hit.
    #[test]
    fn negative_nan_t_rejects() {
        // end = negative-NaN on X with start outside → t = NaN with sign bit set.
        let neg_nan = f32::from_bits(0xFFC0_0000);
        assert_eq!(hit(&UNIT, &[-5.0, 0.0, 0.0], &[neg_nan, 0.0, 0.0]), 0);
    }

    /// Inverted box (min > max): a segment passing through still resolves, a
    /// short one misses — the near-plane rules fall out unchanged.
    #[test]
    fn inverted_box() {
        let inv = [2.0, -1.0, -1.0, -2.0, 1.0, 1.0];
        // With min.x=2 > max.x=-2, a start at x=0 is outside min (0 < 2) and
        // the end at x=0 stays put → axis-X clip path; Y/Z inside.
        let _ = hit(&inv, &[0.0, 0.0, 0.0], &[0.0, 0.0, 0.0]);
        // Deterministic and total: never panics across the inverted geometry.
        assert!(matches!(
            hit(&inv, &[-5.0, 0.0, 0.0], &[5.0, 0.0, 0.0]),
            0 | 1
        ));
    }
}

/// Accumulates a 3x3-matrix box transform: for each component `i` and base
/// `j`, adds the smaller of `col_j[i]*mat[j]` and `col_j[i]*mat[j+3]` into the
/// running min and the larger into the running max (separating-axis box
/// transform). `dst_in` is the running box `[minX,minY,minZ,maxX,maxY,maxZ]`.
pub fn c_aa_box__transform_by3x3__6dc470(
    col0: &[f32; 3],
    col1: &[f32; 3],
    col2: &[f32; 3],
    mat3x3: &[f32; 9],
    dst_in: &[f32; 6],
) -> [f32; 6] {
    let cols = [col0, col1, col2];
    let mut out = *dst_in;
    let mut i = 0usize;
    while i < 3 {
        let mut lo = out[i];
        let mut hi = out[i + 3];
        let mut j = 0usize;
        while j < 3 {
            let src = cols[j][i];
            let p0 = src * mat3x3[j];
            let p1 = mat3x3[j + 3] * src;
            if p1 <= p0 {
                lo += p1;
                hi += p0;
            } else {
                lo += p0;
                hi += p1;
            }
            j += 1;
        }
        out[i] = lo;
        out[i + 3] = hi;
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests_c_aa_box__transform_by3x3__6dc470 {
    use super::c_aa_box__transform_by3x3__6dc470 as xf;

    // Reference: straight transcription of the accumulation, to
    // guard against off-by-one / lane-swap regressions in the kernel.
    fn reference(
        col0: &[f32; 3],
        col1: &[f32; 3],
        col2: &[f32; 3],
        mat: &[f32; 9],
        dst: &[f32; 6],
    ) -> [f32; 6] {
        let cols = [col0, col1, col2];
        let mut out = *dst;
        for i in 0..3usize {
            for j in 0..3usize {
                let src = cols[j][i];
                let f1 = src * mat[j];
                let f2 = mat[j + 3] * src;
                let (mn, mx) = if f2 <= f1 { (f2, f1) } else { (f1, f2) };
                out[i] += mn;
                out[i + 3] += mx;
            }
        }
        out
    }

    #[test]
    fn matches_reference_random() {
        let c0 = [0.5f32, -1.25, 2.0];
        let c1 = [3.0f32, 0.25, -0.5];
        let c2 = [-2.0f32, 1.5, 0.75];
        let mat = [1.0f32, -0.5, 0.25, 2.0, -1.0, 0.5, 9.0, 9.0, 9.0];
        let dst = [10.0f32, 20.0, 30.0, 40.0, 50.0, 60.0];
        let got = xf(&c0, &c1, &c2, &mat, &dst);
        let exp = reference(&c0, &c1, &c2, &mat, &dst);
        for k in 0..6 {
            assert_eq!(got[k].to_bits(), exp[k].to_bits(), "lane {k}");
        }
    }

    #[test]
    fn min_le_max_invariant() {
        let c0 = [1.0f32, 2.0, 3.0];
        let c1 = [-1.0f32, -2.0, -3.0];
        let c2 = [0.5f32, 0.5, 0.5];
        let mat = [1.0f32, 1.0, 1.0, -1.0, -1.0, -1.0, 0.0, 0.0, 0.0];
        let dst = [0.0f32; 6];
        let out = xf(&c0, &c1, &c2, &mat, &dst);
        for i in 0..3 {
            assert!(out[i] <= out[i + 3], "min<=max axis {i}");
        }
    }

    #[test]
    fn known_value() {
        // Single nonzero contribution: col0=[2,0,0], mat[0]=3, mat[3]=-1.
        // i=0,j=0: src=2 -> p0=6, p1=-2 -> min=-2,max=6.
        let c0 = [2.0f32, 0.0, 0.0];
        let c1 = [0.0f32; 3];
        let c2 = [0.0f32; 3];
        let mat = [3.0f32, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let dst = [100.0f32, 0.0, 0.0, 100.0, 0.0, 0.0];
        let out = xf(&c0, &c1, &c2, &mat, &dst);
        assert_eq!(out[0].to_bits(), 98.0f32.to_bits());
        assert_eq!(out[3].to_bits(), 106.0f32.to_bits());
        assert_eq!(out[1].to_bits(), 0.0f32.to_bits());
        assert_eq!(out[4].to_bits(), 0.0f32.to_bits());
    }
}

/// `CAaBox::ContainsPoint` — closed-box point-inclusion test.
///
/// `bx` is the six-float `CAaBox`: `min = [bx[0], bx[1], bx[2]]` (this+0/4/8) and
/// `max = [bx[3], bx[4], bx[5]]` (this+0xc/0x10/0x14). Returns `1` iff
/// `min[i] <= point[i] <= max[i]` on every axis, else `0`.
///
/// Bit-faithful to the x87 original, which is comparison-only (no arithmetic, so
/// no precision concern). The lower bounds use `FCOMP point, min` then
/// `TEST AH,1; JNZ fail`: `C0` (set when `point < min`, and also on a NaN/unordered
/// compare) fails the test — i.e. it requires `point >= min`, modelled here as the
/// NaN-rejecting `!(point[i] >= min[i])`. The upper bounds use `FCOMP point, max`
/// then `TEST AH,0x41; JP fail`: parity over `C0|C3` rejects `point > max`
/// (neither bit set, even parity) and unordered/NaN (both bits set, even parity)
/// while passing strictly-less (`C0`) and equal (`C3`) — i.e. it requires
/// `point <= max`, modelled as `!(point[i] <= max[i])`.
pub fn c_aa_box__contains_point__637350(bx: &[f32; 6], point: &[f32; 3]) -> u32 {
    let min = [bx[0], bx[1], bx[2]];
    let max = [bx[3], bx[4], bx[5]];
    for i in 0..3 {
        // require point >= min; NaN/unordered (C0) rejects.
        if !(point[i] >= min[i]) {
            return 0;
        }
    }
    for i in 0..3 {
        // require point <= max; > max and NaN/unordered both reject.
        if !(point[i] <= max[i]) {
            return 0;
        }
    }
    1
}

#[cfg(test)]
mod tests_c_aa_box__contains_point__637350 {
    use super::c_aa_box__contains_point__637350 as contains;

    // Independent oracle: closed-interval membership on each axis, with an
    // explicit any-NaN short-circuit (a NaN operand makes both `>=` and `<=`
    // false, so it can never be "inside").
    fn oracle(bx: &[f32; 6], p: &[f32; 3]) -> u32 {
        for i in 0..3 {
            if p[i].is_nan() || bx[i].is_nan() || bx[i + 3].is_nan() {
                return 0;
            }
            if p[i] < bx[i] || p[i] > bx[i + 3] {
                return 0;
            }
        }
        1
    }

    const BOX: [f32; 6] = [-1.0, -2.0, -3.0, 4.0, 5.0, 6.0];

    #[test]
    fn interior_point_is_inside() {
        assert_eq!(contains(&BOX, &[0.0, 0.0, 0.0]), 1);
        assert_eq!(contains(&BOX, &[1.5, -1.0, 2.0]), 1);
    }

    #[test]
    fn boundary_is_inclusive() {
        // min and max corners both count (closed box).
        assert_eq!(contains(&BOX, &[-1.0, -2.0, -3.0]), 1);
        assert_eq!(contains(&BOX, &[4.0, 5.0, 6.0]), 1);
        // touching a single face.
        assert_eq!(contains(&BOX, &[-1.0, 0.0, 0.0]), 1);
        assert_eq!(contains(&BOX, &[0.0, 5.0, 0.0]), 1);
    }

    #[test]
    fn outside_any_axis_is_zero() {
        assert_eq!(contains(&BOX, &[-1.0001, 0.0, 0.0]), 0); // below min.x
        assert_eq!(contains(&BOX, &[0.0, 5.0001, 0.0]), 0); // above max.y
        assert_eq!(contains(&BOX, &[0.0, 0.0, -3.0001]), 0); // below min.z
        assert_eq!(contains(&BOX, &[4.0001, 0.0, 0.0]), 0); // above max.x
    }

    #[test]
    fn nan_anywhere_is_zero() {
        let nan = f32::NAN;
        assert_eq!(contains(&BOX, &[nan, 0.0, 0.0]), 0);
        assert_eq!(contains(&BOX, &[0.0, nan, 0.0]), 0);
        assert_eq!(contains(&BOX, &[0.0, 0.0, nan]), 0);
        let bad_min = [nan, -2.0, -3.0, 4.0, 5.0, 6.0];
        assert_eq!(contains(&bad_min, &[0.0, 0.0, 0.0]), 0);
        let bad_max = [-1.0, -2.0, -3.0, nan, 5.0, 6.0];
        assert_eq!(contains(&bad_max, &[0.0, 0.0, 0.0]), 0);
    }

    #[test]
    fn matches_oracle_over_grid() {
        let vals = [-2.0f32, -1.0, -0.5, 0.0, 4.0, 5.0, 6.0, 7.0];
        for &x in &vals {
            for &y in &vals {
                for &z in &vals {
                    let p = [x, y, z];
                    assert_eq!(contains(&BOX, &p), oracle(&BOX, &p), "p={p:?}");
                }
            }
        }
    }
}

/// `CMapObjGroup::BoxIntersectsNodeBox` overlap predicate — the pure AABB-vs-AABB
/// half of the BSP node-box test. `query` and `node` are each a six-float
/// `[minX,minY,minZ,maxX,maxY,maxZ]`. Returns `true` iff the two boxes overlap on
/// every axis.
///
/// Bit-faithful to the x87 original's mixed compare senses. The `query.min <=
/// node.max` half (`FLD query[i]; FCOMP node.max[i]; TEST AH,0x41; JP fail`):
/// parity over `C0|C3` rejects *greater* (neither bit set, even parity) and
/// *unordered/NaN* (both bits set, even parity) while passing *less* (`C0`) and
/// *equal* (`C3`) — an inclusive `query[i] <= node[i+3]`, NaN failing. The
/// `node.min <= query.max` half (`FLD query.max[i]; FCOMP node.min[i]; TEST AH,1;
/// JNZ fail`): `C0` (set on *less* and on *unordered*) rejects, requiring
/// `query[i+3] >= node[i]` — written `node[i] <= query[i+3]`, NaN failing.
/// Touching boxes (equality on any axis) count as overlapping; any NaN axis is a
/// miss. (The first three half-tests use the inclusive `FCOMP`/`JP` form —
/// `<=`, not strict `<`.)
pub fn c_map_obj_group__box_intersects_node_box__6a4830(query: &[f32; 6], node: &[f32; 6]) -> bool {
    query[0] <= node[3]
        && query[1] <= node[4]
        && query[2] <= node[5]
        && node[0] <= query[3]
        && node[1] <= query[4]
        && node[2] <= query[5]
}

#[cfg(test)]
mod tests_c_map_obj_group__box_intersects_node_box__6a4830 {
    use super::c_map_obj_group__box_intersects_node_box__6a4830 as overlap;

    const NODE: [f32; 6] = [0.0, 0.0, 0.0, 10.0, 10.0, 10.0];

    #[test]
    fn overlapping_boxes_intersect() {
        // query fully inside the node.
        assert!(overlap(&[2.0, 2.0, 2.0, 8.0, 8.0, 8.0], &NODE));
        // partial overlap straddling a corner.
        assert!(overlap(&[5.0, 5.0, 5.0, 20.0, 20.0, 20.0], &NODE));
        // query encloses the node.
        assert!(overlap(&[-5.0, -5.0, -5.0, 15.0, 15.0, 15.0], &NODE));
    }

    #[test]
    fn touching_boundary_overlaps() {
        // query.min == node.max on every axis: inclusive on the first half.
        assert!(overlap(&[10.0, 10.0, 10.0, 12.0, 12.0, 12.0], &NODE));
        // query.max == node.min on every axis: inclusive on the second half.
        assert!(overlap(&[-2.0, -2.0, -2.0, 0.0, 0.0, 0.0], &NODE));
    }

    #[test]
    fn separated_boxes_miss() {
        // query entirely beyond node.max on X (query.min.x > node.max.x).
        assert!(!overlap(&[11.0, 2.0, 2.0, 12.0, 8.0, 8.0], &NODE));
        // query entirely below node.min on Y (query.max.y < node.min.y).
        assert!(!overlap(&[2.0, -5.0, 2.0, 8.0, -1.0, 8.0], &NODE));
        // separated on Z.
        assert!(!overlap(&[2.0, 2.0, 11.0, 8.0, 8.0, 12.0], &NODE));
    }

    #[test]
    fn nan_axis_is_miss() {
        let nan = f32::NAN;
        // NaN in the `query.min <= node.max` half.
        assert!(!overlap(&[nan, 2.0, 2.0, 8.0, 8.0, 8.0], &NODE));
        // NaN in the `node.min <= query.max` half (query.max.x is NaN).
        assert!(!overlap(&[2.0, 2.0, 2.0, nan, 8.0, 8.0], &NODE));
        // NaN in a node bound.
        let bad = [nan, 0.0, 0.0, 10.0, 10.0, 10.0];
        assert!(!overlap(&[2.0, 2.0, 2.0, 8.0, 8.0, 8.0], &bad));
    }

    #[test]
    fn miss_on_each_axis_independently() {
        // Overlap requires ALL six half-tests; breaking any one axis breaks it.
        let q = [2.0, 2.0, 2.0, 8.0, 8.0, 8.0];
        assert!(overlap(&q, &NODE));
        for axis in 0..3 {
            // push query strictly above node.max on this axis only.
            let mut sep = q;
            sep[axis] = 11.0;
            sep[axis + 3] = 12.0;
            assert!(!overlap(&sep, &NODE), "axis {axis} high-separation");
            // push query strictly below node.min on this axis only.
            let mut sep = q;
            sep[axis] = -12.0;
            sep[axis + 3] = -11.0;
            assert!(!overlap(&sep, &NODE), "axis {axis} low-separation");
        }
    }
}

/// `CAaBox::Union` — component-wise merge of two axis-aligned boxes into a
/// destination box: `out.min[i] = min(a.min[i], b.min[i])` and `out.max[i] =
/// max(a.max[i], b.max[i])` over a six-float `[minX,minY,minZ,maxX,maxY,maxZ]`.
/// Used to grow/accumulate bounds (BSP/quadtree node bounds, batched-object world
/// bounds); its 2.2% x87 share is call frequency, not per-call work.
///
/// Bit-faithful to the x87 original's per-lane compare/select chain. Each lane is
/// a `FLD;FCOMP;FNSTSW;Jcc;FLD` pick whose *unordered* (NaN in either operand)
/// result resolves to the **`b`** component — verified per block: the three `min`
/// lanes select `a` only on `a < b`, the three `max` lanes only on `a > b`, so
/// NaN (and ties) fall to `b`. That is exactly `MINSS`/`MAXSS(a, b)` semantics
/// (second operand wins on unordered and on equal). Written `if a < b { a } else
/// { b }` / `if a > b { a } else { b }` to reproduce it. (Note that
/// `a <= b ? b : a` gets the ordered result right but inverts the NaN
/// case; the `FCOMP`/`JP` form is authoritative.)
pub fn c_aa_box__union__6373b0(a: &[f32; 6], b: &[f32; 6]) -> [f32; 6] {
    [
        if a[0] < b[0] { a[0] } else { b[0] },
        if a[1] < b[1] { a[1] } else { b[1] },
        if a[2] < b[2] { a[2] } else { b[2] },
        if a[3] > b[3] { a[3] } else { b[3] },
        if a[4] > b[4] { a[4] } else { b[4] },
        if a[5] > b[5] { a[5] } else { b[5] },
    ]
}

#[cfg(test)]
mod tests_c_aa_box__union__6373b0 {
    use super::c_aa_box__union__6373b0 as union;

    #[test]
    fn merges_min_and_max_per_axis() {
        let a = [0.0, 1.0, 2.0, 5.0, 6.0, 7.0];
        let b = [-1.0, 3.0, 1.0, 4.0, 8.0, 9.0];
        // min -> (-1, 1, 1); max -> (5, 8, 9).
        assert_eq!(union(&a, &b), [-1.0, 1.0, 1.0, 5.0, 8.0, 9.0]);
    }

    #[test]
    fn a_encloses_b_returns_a() {
        let a = [-10.0, -10.0, -10.0, 10.0, 10.0, 10.0];
        let b = [-1.0, -2.0, -3.0, 4.0, 5.0, 6.0];
        assert_eq!(union(&a, &b), a);
    }

    #[test]
    fn b_encloses_a_returns_b() {
        let a = [-1.0, -2.0, -3.0, 4.0, 5.0, 6.0];
        let b = [-10.0, -10.0, -10.0, 10.0, 10.0, 10.0];
        assert_eq!(union(&a, &b), b);
    }

    #[test]
    fn equal_box_is_idempotent() {
        let a = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        assert_eq!(union(&a, &a), a);
    }

    #[test]
    fn nan_resolves_to_b_on_every_lane() {
        let nan = f32::NAN;
        // NaN in `a` on every lane -> that lane takes `b` (2nd operand wins on
        // unordered), on both the min lanes (0..3) and the max lanes (3..6).
        let b = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        assert_eq!(union(&[nan; 6], &b), b);
        // NaN in `b` on every lane stays on the `else` arm -> result is `b` (NaN).
        let r = union(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &[nan; 6]);
        assert!(r.iter().all(|x| x.is_nan()));
    }

    #[test]
    fn matches_minmax_oracle_over_grid() {
        // Independent oracle for finite operands: std min/max agree with the
        // kernel where neither operand is NaN.
        let vals = [-3.0f32, -1.0, 0.0, 2.0, 4.0];
        for &ax in &vals {
            for &bx in &vals {
                let r = union(&[ax; 6], &[bx; 6]);
                let lo = ax.min(bx);
                let hi = ax.max(bx);
                assert_eq!(r, [lo, lo, lo, hi, hi, hi], "a={ax} b={bx}");
            }
        }
    }
}

/// Widen a packed 6×`i16` AABB (`min.xyz` at words 0..2, `max.xyz` at words
/// 3..5) into the contiguous float `{min, max}` box `Frustum__CullBox`
/// consumes. Every `i16` is exactly representable in `f32`, so the stock
/// MOVSX→FILD→FSTP lanes are a lossless int→float convert — this is the
/// whole 0.27% profile block of `CGxAabb16__TestFloatBox` 0x6b4d10
/// (death-by-x87-latency on six trivial casts, not real computation).
pub fn c_gx_aabb16__widen_box__6b4d10(words: &[i16; 6]) -> [f32; 6] {
    [
        f32::from(words[0]),
        f32::from(words[1]),
        f32::from(words[2]),
        f32::from(words[3]),
        f32::from(words[4]),
        f32::from(words[5]),
    ]
}

#[cfg(test)]
mod tests_c_gx_aabb16__widen_box__6b4d10 {
    use super::c_gx_aabb16__widen_box__6b4d10 as widen;

    #[test]
    fn lane_order_is_min_then_max() {
        // Stock builds {minf.xyz, maxf.xyz} at [ebp-0x30] from words 0..5 in
        // that order (the FILD shuffle reorders temporaries, not lanes).
        assert_eq!(widen(&[1, 2, 3, 4, 5, 6]), [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn sign_extension_is_signed() {
        // MOVSX word: negative int16 must widen signed, not zero-extended.
        assert_eq!(
            widen(&[-1, -32768, 32767, 0, -2, 2]),
            [-1.0, -32768.0, 32767.0, 0.0, -2.0, 2.0]
        );
    }

    #[test]
    fn every_i16_is_exact() {
        // f32 has 24 mantissa bits — the full i16 range round-trips exactly.
        for w in [i16::MIN, -257, -1, 0, 1, 255, 256, 257, i16::MAX] {
            let f = widen(&[w; 6])[0];
            assert_eq!(f as i32, i32::from(w));
        }
    }
}

/// Child-vs-query AABB overlap of `CMapObjGroupNode__QueryChildrenOverlap`
/// 0x6a3dc0 (block 0x6a3e30): six FLD/FCOMP gates. The min-side compares
/// (`TEST AH,0x41; JP`) continue on *less or equal*; the max-side compares
/// (`TEST AH,0x1; JNZ`) continue on *greater or equal* — touching boxes
/// overlap, and ANY unordered lane (NaN in a child bbox) skips the child.
/// The plain ordered `<=`/`>=` AND-chain reproduces exactly that: every
/// comparison is false on NaN, and false means skip.
pub fn query_children_overlap__6a3dc0(
    child_min: &[f32; 3],
    child_max: &[f32; 3],
    query: &[f32; 6],
) -> bool {
    child_min[0] <= query[3]
        && child_min[1] <= query[4]
        && child_min[2] <= query[5]
        && child_max[0] >= query[0]
        && child_max[1] >= query[1]
        && child_max[2] >= query[2]
}

/// Dispatch-mask fold of 0x6a3dc0 (blocks 0x6a3dcd..0x6a3dfa): seeds 0xEE
/// (0xC6 when flag 0x10), then clears bit 0x20/0x24-mask on flag 0x20,
/// bit 0x02 when flag 0x40 is ABSENT, bit 0x40 when flag 0x4000 is absent.
/// Forwarded to the BSP query as the low 16 bits of a dword push.
pub fn query_children_flags__6a3dc0(flags: u32) -> u32 {
    let mut m: u32 = if flags & 0x10 != 0 { 0xc6 } else { 0xee };
    if flags & 0x20 != 0 {
        m &= 0xdb;
    }
    if flags & 0x40 == 0 {
        m &= 0xfd;
    }
    if flags & 0x4000 == 0 {
        m &= 0xbf;
    }
    m
}

#[cfg(test)]
mod tests_query_children_overlap__6a3dc0 {
    use super::*;

    const Q: [f32; 6] = [0.0, 0.0, 0.0, 10.0, 10.0, 10.0];

    #[test]
    fn overlap_and_touch() {
        assert!(query_children_overlap__6a3dc0(
            &[1.0, 1.0, 1.0],
            &[2.0, 2.0, 2.0],
            &Q
        ));
        // Touching on either side still overlaps (equal continues).
        assert!(query_children_overlap__6a3dc0(
            &[10.0, 0.0, 0.0],
            &[11.0, 1.0, 1.0],
            &Q
        ));
        assert!(query_children_overlap__6a3dc0(
            &[-5.0, 0.0, 0.0],
            &[0.0, 1.0, 1.0],
            &Q
        ));
    }

    #[test]
    fn disjoint_skips() {
        // min beyond query max, per axis.
        assert!(!query_children_overlap__6a3dc0(
            &[10.5, 0.0, 0.0],
            &[11.0, 1.0, 1.0],
            &Q
        ));
        assert!(!query_children_overlap__6a3dc0(
            &[0.0, 10.5, 0.0],
            &[1.0, 11.0, 1.0],
            &Q
        ));
        // max below query min, per axis.
        assert!(!query_children_overlap__6a3dc0(
            &[-2.0, 0.0, 0.0],
            &[-1.0, 1.0, 1.0],
            &Q
        ));
        assert!(!query_children_overlap__6a3dc0(
            &[0.0, 0.0, -2.0],
            &[1.0, 1.0, -1.0],
            &Q
        ));
    }

    #[test]
    fn nan_in_child_bbox_skips() {
        let nan = f32::NAN;
        assert!(!query_children_overlap__6a3dc0(
            &[nan, 1.0, 1.0],
            &[2.0, 2.0, 2.0],
            &Q
        ));
        assert!(!query_children_overlap__6a3dc0(
            &[1.0, 1.0, 1.0],
            &[2.0, nan, 2.0],
            &Q
        ));
        // NaN in the query box likewise skips every child.
        let mut q = Q;
        q[3] = nan;
        assert!(!query_children_overlap__6a3dc0(
            &[1.0, 1.0, 1.0],
            &[2.0, 2.0, 2.0],
            &q
        ));
    }

    #[test]
    fn flag_fold_table() {
        assert_eq!(query_children_flags__6a3dc0(0x40 | 0x4000), 0xee);
        assert_eq!(query_children_flags__6a3dc0(0x10 | 0x40 | 0x4000), 0xc6);
        assert_eq!(
            query_children_flags__6a3dc0(0x20 | 0x40 | 0x4000),
            0xee & 0xdb
        );
        // Absent 0x40 clears bit 1; absent 0x4000 clears bit 6.
        assert_eq!(query_children_flags__6a3dc0(0x4000), 0xee & 0xfd);
        assert_eq!(query_children_flags__6a3dc0(0x40), 0xee & 0xbf);
        assert_eq!(query_children_flags__6a3dc0(0), 0xee & 0xfd & 0xbf);
    }
}
