//! `spline` family kernels.
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

/// Arc-length -> (segment, localT) reparametrization.
///
/// `dist` is scaled by `scale` (the spline's total-length normaliser), then the
/// running sum of `seg_lengths` is walked until it would exceed the target,
/// yielding the segment index and the fractional distance into that segment
/// (`local_t`, in arc-length units, not normalised). With a single segment the
/// scaled distance passes straight through as `local_t`.
pub fn c_cubic_spline__arc_length_to_segment__453840(
    dist: f32,
    scale: f32,
    seg_lengths: &[f32],
) -> (u32, f32) {
    let seg_count = seg_lengths.len();
    if seg_count == 1 {
        return (0, dist);
    }
    let target = scale * dist;
    let mut acc = 0.0f32;
    let mut idx: u32 = 0;
    while (idx as usize) < seg_count {
        let next = seg_lengths[idx as usize] + acc;
        if target < next {
            break;
        }
        idx += 1;
        acc = next;
        if idx >= (seg_count as u32 - 1) {
            break;
        }
    }
    let local_t = (target - acc) / seg_lengths[idx as usize];
    (idx, local_t)
}

#[cfg(test)]
mod tests_c_cubic_spline__arc_length_to_segment__453840 {
    use super::c_cubic_spline__arc_length_to_segment__453840 as a2s;

    #[test]
    fn single_segment_passthrough() {
        let (idx, t) = a2s(7.5, 0.3, &[42.0]);
        assert_eq!(idx, 0);
        assert_eq!(t.to_bits(), 7.5f32.to_bits());
    }

    #[test]
    fn lands_in_first_segment() {
        // scale=1, segs [10,10,10]; target 4 falls in segment 0 at local 4/10.
        let (idx, t) = a2s(4.0, 1.0, &[10.0, 10.0, 10.0]);
        assert_eq!(idx, 0);
        assert!((t - 0.4).abs() <= 1e-6);
    }

    #[test]
    fn lands_in_middle_segment() {
        // target 15 → past seg0 (10), into seg1 at (15-10)/10 = 0.5.
        let (idx, t) = a2s(15.0, 1.0, &[10.0, 10.0, 10.0]);
        assert_eq!(idx, 1);
        assert!((t - 0.5).abs() <= 1e-6);
    }

    #[test]
    fn clamps_to_last_segment() {
        // target far past the end clamps at the final segment (index count-1);
        // local_t exceeds 1 because the walk stops at the last segment.
        let (idx, t) = a2s(1000.0, 1.0, &[10.0, 10.0, 10.0]);
        assert_eq!(idx, 2);
        // (1000 - 20)/10 = 98
        assert!((t - 98.0).abs() <= 1e-3);
    }

    #[test]
    fn scale_applies_before_walk() {
        // scale 0.5 halves the effective distance: 30*0.5 = 15 → seg1 @ 0.5.
        let (idx, t) = a2s(30.0, 0.5, &[10.0, 10.0, 10.0]);
        assert_eq!(idx, 1);
        assert!((t - 0.5).abs() <= 1e-6);
    }

    #[test]
    fn boundary_goes_to_next_segment() {
        // target exactly on the seg0/seg1 boundary (10): `target < next` is
        // false at idx0, so it advances to segment 1 at local 0.
        let (idx, t) = a2s(10.0, 1.0, &[10.0, 10.0, 10.0]);
        assert_eq!(idx, 1);
        assert!(t.abs() <= 1e-6);
    }
}

/// Cubic-spline tangent/derivative evaluator.
///
/// For 4 control points `cp` (4 vec3, contiguous) and a 4-row basis matrix
/// `basis` (4 rows of 4 f32, stride 4; each row a degree-2 polynomial whose
/// first 3 coefficients are used), accumulates
/// `out[axis] = Σ_i Horner2(basis_row_i, t) * cp_i[axis]`.
pub fn c_cubic_spline__eval_basis_quadratic__453640(
    cp: &[f32; 12],
    basis: &[f32; 16],
    t: f32,
) -> [f32; 3] {
    // Seed each accumulator with the first control point's contribution so the
    // inner sum starts from a real product (no dead xorps).
    let w0 = horner2(&[basis[0], basis[1], basis[2]], t);
    let mut x = w0 * cp[0];
    let mut y = w0 * cp[1];
    let mut z = w0 * cp[2];
    for i in 1..4 {
        let b = i * 4;
        let c = i * 3;
        let w = horner2(&[basis[b], basis[b + 1], basis[b + 2]], t);
        x += w * cp[c];
        y += w * cp[c + 1];
        z += w * cp[c + 2];
    }
    [x, y, z]
}

/// Degree-2 Horner evaluation: `((c0 * t) + c1) * t + c2`.
#[inline]
fn horner2(c: &[f32; 3], t: f32) -> f32 {
    (c[0] * t + c[1]) * t + c[2]
}

#[cfg(test)]
mod tests_c_cubic_spline__eval_basis_quadratic__453640 {
    use super::c_cubic_spline__eval_basis_quadratic__453640 as eval;

    // Independent oracle.
    fn oracle(cp: &[f32; 12], basis: &[f32; 16], t: f32) -> [f32; 3] {
        let mut out = [0.0f32; 3];
        for i in 0..4 {
            let b = i * 4;
            let w = (basis[b] * t + basis[b + 1]) * t + basis[b + 2];
            for axis in 0..3 {
                out[axis] += w * cp[i * 3 + axis];
            }
        }
        out
    }

    #[test]
    fn matches_oracle() {
        let cp = [
            1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, -1.0, -2.0, -3.0, 0.5, 0.25, 0.125,
        ];
        let basis = [
            0.5f32, -1.0, 0.5, 9.0, -1.0, 1.0, 0.0, 9.0, 0.5, 0.0, 0.0, 9.0, 0.0, 0.0, 1.0, 9.0,
        ];
        for &t in &[0.0f32, 0.25, 0.5, 0.75, 1.0] {
            let got = eval(&cp, &basis, t);
            let exp = oracle(&cp, &basis, t);
            for axis in 0..3 {
                assert!(
                    (got[axis] - exp[axis]).abs() <= 1e-5 * (1.0 + exp[axis].abs()),
                    "axis {axis} t {t}: {} vs {}",
                    got[axis],
                    exp[axis]
                );
            }
        }
    }

    #[test]
    fn partition_of_unity_reproduces_point() {
        // If every basis row evaluates to the same weight 0.25 (sum = 1) the
        // result is the average of the four control points.
        let cp = [
            2.0f32, 0.0, 0.0, 0.0, 4.0, 0.0, 0.0, 0.0, 6.0, 8.0, 8.0, 8.0,
        ];
        let basis = [
            0.0f32, 0.0, 0.25, 0.0, 0.0, 0.0, 0.25, 0.0, 0.0, 0.0, 0.25, 0.0, 0.0, 0.0, 0.25, 0.0,
        ];
        let got = eval(&cp, &basis, 0.7);
        let exp = [
            (2.0 + 0.0 + 0.0 + 8.0) / 4.0,
            (0.0 + 4.0 + 0.0 + 8.0) / 4.0,
            (0.0 + 0.0 + 6.0 + 8.0) / 4.0,
        ];
        for axis in 0..3 {
            assert!((got[axis] - exp[axis]).abs() <= 1e-6, "axis {axis}");
        }
    }
}

/// Linear-mode spline segment position: lerp between the two interior control
/// points `cp[seg*3+3]` and `cp[seg*3+6]` by `local_t`. The kernel takes the
/// two already-selected control points `a` and `b`; the adapter performs the
/// index arithmetic against the live control-point array.
///
/// `out = a + (b - a) * local_t`.
pub fn c_cubic_spline__eval_segment_pos__4541b0(
    a: &[f32; 3],
    b: &[f32; 3],
    local_t: f32,
) -> [f32; 3] {
    [
        (b[0] - a[0]) * local_t + a[0],
        (b[1] - a[1]) * local_t + a[1],
        (b[2] - a[2]) * local_t + a[2],
    ]
}

#[cfg(test)]
mod tests_c_cubic_spline__eval_segment_pos__4541b0 {
    use super::c_cubic_spline__eval_segment_pos__4541b0 as lerp;

    #[test]
    fn endpoints() {
        let a = [1.0f32, 2.0, 3.0];
        let b = [4.0f32, 8.0, -1.0];
        // t = 0 → a, t = 1 → b
        let at0 = lerp(&a, &b, 0.0);
        let at1 = lerp(&a, &b, 1.0);
        for i in 0..3 {
            assert!((at0[i] - a[i]).abs() <= 1e-6);
            assert!((at1[i] - b[i]).abs() <= 1e-6);
        }
    }

    #[test]
    fn midpoint_is_average() {
        let a = [0.0f32, 10.0, -4.0];
        let b = [2.0f32, 20.0, 4.0];
        let m = lerp(&a, &b, 0.5);
        assert!((m[0] - 1.0).abs() <= 1e-6);
        assert!((m[1] - 15.0).abs() <= 1e-6);
        assert!((m[2] - 0.0).abs() <= 1e-6);
    }

    #[test]
    fn monotone_in_t() {
        // For a component increasing from a to b, lerp is monotone in t.
        let a = [0.0f32, 0.0, 0.0];
        let b = [10.0f32, 0.0, 0.0];
        let mut prev = f32::NEG_INFINITY;
        for k in 0..=10 {
            let t = k as f32 / 10.0;
            let v = lerp(&a, &b, t)[0];
            assert!(v >= prev);
            prev = v;
        }
    }
}

/// Squared 2D magnitude of a vector: `v[0]^2 + v[1]^2` (x, y only).
pub fn c_cubic_spline__sq_mag2_d__454980(v: &[f32; 2]) -> f32 {
    v[0] * v[0] + v[1] * v[1]
}

#[cfg(test)]
mod tests_c_cubic_spline__sq_mag2_d__454980 {
    use super::c_cubic_spline__sq_mag2_d__454980 as sq;

    #[test]
    fn known_values() {
        assert_eq!(sq(&[3.0, 4.0]).to_bits(), 25.0f32.to_bits());
        assert_eq!(sq(&[0.0, 0.0]).to_bits(), 0.0f32.to_bits());
        assert_eq!(sq(&[1.0, 0.0]).to_bits(), 1.0f32.to_bits());
    }

    #[test]
    fn ignores_z_would_be() {
        // Only x and y participate; sign-independent.
        assert_eq!(sq(&[-2.0, -1.5]).to_bits(), (4.0f32 + 2.25).to_bits());
    }

    #[test]
    fn non_negative_and_symmetric() {
        for (x, y) in [(1.0f32, 2.0), (-3.0, 0.5), (7.5, -2.25), (0.0, 9.0)] {
            let a = sq(&[x, y]);
            assert!(a >= 0.0);
            // sign flip leaves the squared magnitude unchanged
            assert_eq!(a.to_bits(), sq(&[-x, -y]).to_bits());
            assert_eq!(a.to_bits(), sq(&[y, x]).to_bits()); // swap symmetry
        }
    }
}

/// Cubic-spline position eval: per control row, Horner-evaluate a 4-coefficient
/// cubic at `t`, weight by that row's 3-component basis vector, and sum.
///
/// `control` holds 4 contiguous polynomial rows (4 coefficients each, highest
/// degree first); `basis` holds the matching 4 rows of 3-component weights.
pub fn c_spline_eval_point__453580(control: &[f32; 16], basis: &[f32; 12], t: f32) -> [f32; 3] {
    // Seed the accumulator with the first row's contribution (autovec-friendly:
    // no zero-init then +=).
    let w0 = (((control[0] * t + control[1]) * t + control[2]) * t) + control[3];
    let mut out = [w0 * basis[0], w0 * basis[1], w0 * basis[2]];
    let mut i = 1;
    while i < 4 {
        let c = i * 4;
        let b = i * 3;
        let w = (((control[c] * t + control[c + 1]) * t + control[c + 2]) * t) + control[c + 3];
        out[0] += w * basis[b];
        out[1] += w * basis[b + 1];
        out[2] += w * basis[b + 2];
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests_c_spline_eval_point__453580 {
    use super::c_spline_eval_point__453580;

    fn horner(c: [f32; 4], t: f32) -> f32 {
        ((c[0] * t + c[1]) * t + c[2]) * t + c[3]
    }

    // Reference implementation mirroring the stock loop exactly.
    fn reference(control: &[f32; 16], basis: &[f32; 12], t: f32) -> [f32; 3] {
        let mut out = [0.0f32; 3];
        for i in 0..4 {
            let c = [
                control[i * 4],
                control[i * 4 + 1],
                control[i * 4 + 2],
                control[i * 4 + 3],
            ];
            let w = horner(c, t);
            out[0] += w * basis[i * 3];
            out[1] += w * basis[i * 3 + 1];
            out[2] += w * basis[i * 3 + 2];
        }
        out
    }

    fn approx(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
        (a[0] - b[0]).abs() <= tol && (a[1] - b[1]).abs() <= tol && (a[2] - b[2]).abs() <= tol
    }

    #[test]
    fn matches_reference_loop() {
        let control: [f32; 16] = [
            0.1, -0.2, 0.3, 1.0, 0.5, 0.4, -0.3, 0.2, -0.1, 0.6, 0.7, -0.8, 0.9, -0.05, 0.15, 0.25,
        ];
        let basis: [f32; 12] = [
            1.0, 2.0, 3.0, -1.0, 0.5, 0.25, 4.0, -2.0, 1.5, 0.0, 1.0, -1.0,
        ];
        for &t in &[0.0f32, 0.25, 0.5, 0.75, 1.0, -0.5, 2.0] {
            let got = c_spline_eval_point__453580(&control, &basis, t);
            let exp = reference(&control, &basis, t);
            assert!(approx(got, exp, 1e-4), "t={t}: got {got:?} exp {exp:?}");
        }
    }

    #[test]
    fn known_value_t_zero_picks_constant_terms() {
        // At t=0, each row's Horner result is its constant term control[i*4+3].
        let control: [f32; 16] = [
            7.0, 7.0, 7.0, 2.0, 9.0, 9.0, 9.0, 3.0, 1.0, 1.0, 1.0, -4.0, 5.0, 5.0, 5.0, 6.0,
        ];
        // Identity-ish basis: row i contributes its weight only to axis (i mod 3).
        let basis: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
        let got = c_spline_eval_point__453580(&control, &basis, 0.0);
        // w = [2, 3, -4, 6]
        // x = 2*1 + 6*1 = 8 ; y = 3*1 + 6*1 = 9 ; z = -4*1 + 6*1 = 2
        assert!((got[0] - 8.0).abs() < 1e-6);
        assert!((got[1] - 9.0).abs() < 1e-6);
        assert!((got[2] - 2.0).abs() < 1e-6);
    }

    #[test]
    fn zero_basis_yields_zero() {
        let control: [f32; 16] = [
            0.3, 0.1, 0.2, 0.9, 1.1, 1.2, 1.3, 1.4, 2.0, 2.1, 2.2, 2.3, 3.0, 3.1, 3.2, 3.3,
        ];
        let basis = [0.0f32; 12];
        let got = c_spline_eval_point__453580(&control, &basis, 0.42);
        assert_eq!(got, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn linearity_in_basis() {
        // Output is linear in the basis weights: scaling basis scales output.
        let control: [f32; 16] = [
            0.2, -0.1, 0.4, 0.5, -0.3, 0.6, 0.1, 0.2, 0.7, -0.2, 0.3, 0.1, 0.0, 0.5, -0.4, 0.8,
        ];
        let basis: [f32; 12] = [
            1.0, -2.0, 0.5, 3.0, 0.25, -1.0, 0.5, 2.0, -0.5, -1.5, 1.0, 0.75,
        ];
        let scaled: [f32; 12] = core::array::from_fn(|i| basis[i] * 2.5);
        let t = 0.6;
        let a = c_spline_eval_point__453580(&control, &basis, t);
        let b = c_spline_eval_point__453580(&control, &scaled, t);
        for k in 0..3 {
            assert!((b[k] - a[k] * 2.5).abs() <= 1e-4, "axis {k}: {a:?} {b:?}");
        }
    }
}

// No NEW pure kernel required: EvalSegmentTangent is a thin wrapper that
// loads a fixed degree-2 derivative basis and forwards to EvalBasisQuadratic,
// whose kernel `crate::math::spline::c_cubic_spline__eval_basis_quadratic__453640`
// is ALREADY shipped (src/math/spline.rs). This adapter reuses it verbatim.
// The basis-expansion + control-point selection are exercised by the adapter
// self-verify (3/3 passing against an independent tangent oracle).

/// Orientation rows of a position+orientation frame along a spline segment.
///
/// `cp` holds the segment's four contiguous vec3 control points (the window
/// starting at `segment*3`). The chord forward is `cp[6..9] - cp[3..6]`,
/// normalized when its length clears `2^-22`. With `deriv_basis` present
/// (cubic mode) the analytic tangent (degree-2 basis evaluation at
/// `local_t`) is preferred when it normalizes (length >= `0.01`) and agrees
/// with the chord (dot >= `0.5`); a degenerate tangent keeps `prev_fwd`
/// (the caller's pre-existing forward row, which the original leaves
/// untouched). Returns `(forward, right, up)` where
/// `right = normalize2d(-forward.y, forward.x, 0)` and
/// `up = forward x right`.
pub fn c_cubic_spline__eval_frame_at_distance__454580(
    cp: &[f32; 12],
    deriv_basis: Option<&[f32; 16]>,
    local_t: f32,
    prev_fwd: &[f32; 3],
) -> ([f32; 3], [f32; 3], [f32; 3]) {
    // Bit-exact host constants: 2^-22 (degenerate-length gate, 0x8029d4),
    // 0.01 (tangent-length gate, 0x8029d0), 0.5 (agreement dot, 0x7ffa24).
    const EPS_LEN: f32 = 2.384_185_791_015_625e-7;
    const EPS_TANGENT: f32 = 0.01;
    const DOT_AGREE: f32 = 0.5;

    let mut fwd = [cp[6] - cp[3], cp[7] - cp[4], cp[8] - cp[5]];
    let len = super::vector::c3_vector__squared_magnitude__4549f0(&fwd).sqrt();
    // `>=` is false on NaN: a non-finite length skips the normalize, as the
    // original's parity-flag branch does.
    if len.abs() >= EPS_LEN {
        fwd = super::vector::c3_vector__scale_by_inverse__4549c0(&fwd, len);
    }
    let chosen = match deriv_basis {
        None => fwd,
        Some(basis) => {
            let tan = c_cubic_spline__eval_basis_quadratic__453640(cp, basis, local_t);
            let tan_len = super::vector::c3_vector__squared_magnitude__4549f0(&tan).sqrt();
            if tan_len.abs() >= EPS_TANGENT {
                let inv = 1.0_f32 / tan_len;
                let tan = [tan[0] * inv, tan[1] * inv, tan[2] * inv];
                // Source x87 accumulation order: (z + y) + x.
                let dot = (tan[2] * fwd[2] + tan[1] * fwd[1]) + fwd[0] * tan[0];
                if dot >= DOT_AGREE { tan } else { fwd }
            } else {
                // Degenerate tangent: the original leaves the forward row
                // untouched and builds the basis from the caller's value.
                *prev_fwd
            }
        }
    };
    let mut right = [-chosen[1], chosen[0], 0.0_f32];
    let right_len = c_cubic_spline__sq_mag2_d__454980(&[right[0], right[1]]).sqrt();
    if right_len.abs() >= EPS_LEN {
        let inv = 1.0_f32 / right_len;
        right[0] *= inv;
        right[1] *= inv;
    }
    let up = [
        -(right[1] * chosen[2]),
        right[0] * chosen[2],
        chosen[0] * right[1] - chosen[1] * right[0],
    ];
    (chosen, right, up)
}

#[cfg(test)]
mod tests_c_cubic_spline__eval_frame_at_distance__454580 {
    use super::c_cubic_spline__eval_frame_at_distance__454580 as frame;

    const ZERO: [f32; 3] = [0.0, 0.0, 0.0];

    // cp window where the chord `cp[6..9] - cp[3..6]` equals `chord` and the
    // last control point is `p3` (the tangent under the P3-selecting basis).
    fn cp_with_chord(chord: [f32; 3], p3: [f32; 3]) -> [f32; 12] {
        [
            0.5,
            -0.5,
            0.25,
            1.0,
            2.0,
            3.0,
            1.0 + chord[0],
            2.0 + chord[1],
            3.0 + chord[2],
            p3[0],
            p3[1],
            p3[2],
        ]
    }

    // Derivative basis whose rows all evaluate to 0 except row 3 (constant
    // weight 1), so the tangent equals control point P3 at every t.
    fn p3_select_basis() -> [f32; 16] {
        let mut b = [0.0_f32; 16];
        b[14] = 1.0;
        b
    }

    #[test]
    fn host_constants_are_bit_exact() {
        // The embedded gates must match the host .rdata dwords.
        assert_eq!(2.384_185_791_015_625e-7_f32.to_bits(), 0x3480_0000);
        assert_eq!(0.01_f32.to_bits(), 0x3c23_d70a);
        assert_eq!(0.5_f32.to_bits(), 0x3f00_0000);
        assert_eq!(1.0_f32.to_bits(), 0x3f80_0000);
    }

    #[test]
    fn linear_axis_aligned_frame() {
        let cp = cp_with_chord([2.0, 0.0, 0.0], ZERO);
        let (fwd, right, up) = frame(&cp, None, 0.0, &ZERO);
        assert!((fwd[0] - 1.0).abs() <= 1e-6 && fwd[1].abs() <= 1e-6 && fwd[2].abs() <= 1e-6);
        assert!((right[1] - 1.0).abs() <= 1e-6 && right[0].abs() <= 1e-6);
        assert_eq!(right[2].to_bits(), 0.0_f32.to_bits());
        assert!((up[2] - 1.0).abs() <= 1e-6 && up[0].abs() <= 1e-6 && up[1].abs() <= 1e-6);
    }

    #[test]
    fn yaw_sweep_right_stays_orthonormal() {
        // Horizontal chords at several yaws: right = (-sin, cos, 0), up = +z.
        for &(c, s) in &[(1.0_f32, 0.0_f32), (0.6, 0.8), (-0.8, 0.6), (0.0, -1.0)] {
            let cp = cp_with_chord([3.0 * c, 3.0 * s, 0.0], ZERO);
            let (fwd, right, up) = frame(&cp, None, 0.5, &ZERO);
            assert!((fwd[0] - c).abs() <= 1e-5 && (fwd[1] - s).abs() <= 1e-5);
            assert!((right[0] + s).abs() <= 1e-5 && (right[1] - c).abs() <= 1e-5);
            assert!((up[2] - 1.0).abs() <= 1e-5);
            let dot_fr = fwd[0] * right[0] + fwd[1] * right[1] + fwd[2] * right[2];
            let dot_fu = fwd[0] * up[0] + fwd[1] * up[1] + fwd[2] * up[2];
            assert!(dot_fr.abs() <= 1e-5 && dot_fu.abs() <= 1e-5);
        }
    }

    #[test]
    fn vertical_chord_collapses_right_and_up_without_nan() {
        // A vertical chord zeroes the horizontal right vector; the 2^-22
        // gate skips its normalization (no 0/0 NaN) and the cross collapses.
        let cp = cp_with_chord([0.0, 0.0, 5.0], ZERO);
        let (fwd, right, up) = frame(&cp, None, 0.0, &ZERO);
        assert!((fwd[2] - 1.0).abs() <= 1e-6);
        for v in [right, up] {
            for c in v {
                assert!(c == 0.0, "expected zero, got {c}");
            }
        }
    }

    #[test]
    fn degenerate_chord_skips_normalize() {
        let cp = cp_with_chord(ZERO, ZERO);
        let (fwd, right, up) = frame(&cp, None, 0.0, &ZERO);
        assert_eq!(fwd, ZERO);
        assert!(right.iter().chain(up.iter()).all(|c| *c == 0.0));
    }

    #[test]
    fn cubic_agreeing_tangent_is_chosen() {
        // chord -> +x; tangent (1.6, 1.2, 0) normalizes to (0.8, 0.6, 0):
        // dot 0.8 >= 0.5 selects the tangent, not the chord.
        let cp = cp_with_chord([2.0, 0.0, 0.0], [1.6, 1.2, 0.0]);
        let basis = p3_select_basis();
        let (fwd, _, _) = frame(&cp, Some(&basis), 0.25, &ZERO);
        assert!((fwd[0] - 0.8).abs() <= 1e-6 && (fwd[1] - 0.6).abs() <= 1e-6);
    }

    #[test]
    fn cubic_disagreeing_tangent_falls_back_to_chord() {
        // tangent at ~63.4 deg from the chord: dot ~0.447 < 0.5 keeps the chord.
        let cp = cp_with_chord([2.0, 0.0, 0.0], [0.4, 0.8, 0.0]);
        let basis = p3_select_basis();
        let (fwd, _, _) = frame(&cp, Some(&basis), 0.25, &ZERO);
        assert!((fwd[0] - 1.0).abs() <= 1e-6 && fwd[1].abs() <= 1e-6);
    }

    #[test]
    fn cubic_degenerate_tangent_keeps_caller_forward() {
        // tangent = P3 = 0 (length 0 < 0.01): the caller's forward row is
        // kept and the basis is built from it: right = (-prev.y, prev.x, 0).
        let cp = cp_with_chord([2.0, 0.0, 0.0], ZERO);
        let basis = p3_select_basis();
        let prev = [0.0_f32, 1.0, 0.0];
        let (fwd, right, up) = frame(&cp, Some(&basis), 0.5, &prev);
        assert_eq!(fwd, prev);
        assert!((right[0] + 1.0).abs() <= 1e-6 && right[1].abs() <= 1e-6);
        // up.z = prev.x*right.y - prev.y*right.x = 0*0 - 1*(-1) = 1
        assert!((up[2] - 1.0).abs() <= 1e-6);
    }

    #[test]
    fn tiny_horizontal_forward_skips_right_normalize() {
        // forward ~ (1e-10, 0, 1): the planar right magnitude 1e-10 sits
        // below 2^-22, so right stays un-normalized (and finite).
        let cp = cp_with_chord([1e-10, 0.0, 1.0], ZERO);
        let (fwd, right, _) = frame(&cp, None, 0.0, &ZERO);
        assert!(fwd.iter().all(|c| c.is_finite()));
        assert!(right[1].abs() < 1e-7 && right[0] == 0.0);
    }
}
