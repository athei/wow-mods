// Adapter and kernel names mirror the host's C++ symbols verbatim, with `__`
// standing in for the `::`, so the whole module is non-snake-case by construction.
#![allow(non_snake_case)]

/// 4×4 row-major matrix product `out = a · b`.
///
/// `out[r][c] = Σ_k a[r][k] · b[k][c]`, indices `r*4 + c`.
///
/// `ikj` accumulation order: each output row is built as a sum of `b` rows scaled
/// by the `a` entries, so the innermost work is over four contiguous `b`
/// elements — the shape most amenable to auto-vectorization. Returns the product
/// by value; the caller writes it out (the original requires `out` to alias
/// neither input, which a by-value result sidesteps entirely).
pub fn mul4x4(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0f32; 16];
    for r in 0..4 {
        for k in 0..4 {
            let aik = a[r * 4 + k];
            for c in 0..4 {
                out[r * 4 + c] += aik * b[k * 4 + c];
            }
        }
    }
    out
}

/// Per-element `k` summation order of `C44Matrix::Multiply`, indexed `r * 4 + c`.
///
/// The original is fully unrolled and its sixteen four-term dots do **not** share
/// one `k` order: there are six distinct ones, read off the `FLD`/`FMUL`/`FADDP`
/// sequence from `0x7bc6a8` to `0x7bc8dc`. Nothing about the algebra predicts
/// them — they are whatever the 1.12 compiler's scheduler emitted — so the table
/// is transcribed, not derived.
const MULTIPLY_K_ORDER: [[usize; 4]; 16] = [
    [3, 2, 1, 0],
    [3, 2, 1, 0],
    [3, 2, 1, 0],
    [2, 3, 0, 1],
    [3, 2, 0, 1],
    [3, 2, 1, 0],
    [3, 2, 1, 0],
    [2, 3, 0, 1],
    [3, 2, 0, 1],
    [3, 2, 1, 0],
    [3, 2, 1, 0],
    [2, 3, 0, 1],
    [2, 1, 3, 0],
    [2, 0, 1, 3],
    [2, 0, 1, 3],
    [2, 1, 0, 3],
];

/// 4×4 row-major product as `C44Matrix::Multiply` (`0x7bc6a0`) computes it.
///
/// Same algebra as [`mul4x4`] and a different summation: each element sums its
/// four products in that element's own [`MULTIPLY_K_ORDER`], starting from the
/// first product rather than from a zero seed, so there are three additions and
/// not four. Kept separate from [`mul4x4`] because that one has to keep agreeing
/// to the bit with the dispatched D3DX product the animation update also uses,
/// which sums `k = 0..3` from zero.
///
/// An element's four products and three sums never leave the x87 stack — the
/// only store in a block is the `FSTP m32` that ends it (`0x7bc6cc` for the
/// first) — so the chain carries an `f64` significand and narrows exactly once,
/// at the element's store.
pub fn c44_matrix__multiply__7bc6a0(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0f32; 16];
    for r in 0..4 {
        for c in 0..4 {
            let ks = MULTIPLY_K_ORDER[r * 4 + c];
            let term = |k: usize| f64::from(a[r * 4 + k]) * f64::from(b[k * 4 + c]);
            let mut acc = term(ks[0]);
            acc += term(ks[1]);
            acc += term(ks[2]);
            acc += term(ks[3]);
            out[r * 4 + c] = super::f64_to_f32(acc);
        }
    }
    out
}

#[cfg(test)]
mod tests_c44_matrix__multiply__7bc6a0 {
    use super::{c44_matrix__multiply__7bc6a0 as stock, mul4x4};

    /// Two matrices whose entries all carry a full non-terminating mantissa.
    ///
    /// Every reassociation of a four-term dot then lands on different bits.
    /// Given as bit patterns: a rounded decimal literal is a different float and
    /// would stop separating the orders.
    fn fixtures() -> ([f32; 16], [f32; 16]) {
        let mut a = [0.0f32; 16];
        let mut b = [0.0f32; 16];
        let mut aw = 0x3fc0_0001_u32;
        let mut bw = 0xbf91_0003_u32;
        for (slot_a, slot_b) in a.iter_mut().zip(b.iter_mut()) {
            *slot_a = f32::from_bits(aw);
            *slot_b = f32::from_bits(bw);
            aw += 0x0001_3579;
            bw += 0x0000_9adf;
        }
        (a, b)
    }

    #[test]
    fn matches_the_transcribed_per_element_order() {
        let (a, b) = fixtures();
        let got = stock(&a, &b);
        for r in 0..4 {
            for c in 0..4 {
                let ks = super::MULTIPLY_K_ORDER[r * 4 + c];
                let term = |k: usize| f64::from(a[r * 4 + k]) * f64::from(b[k * 4 + c]);
                let mut want = term(ks[0]);
                want += term(ks[1]);
                want += term(ks[2]);
                want += term(ks[3]);
                let want = super::super::f64_to_f32(want);
                assert_eq!(got[r * 4 + c].to_bits(), want.to_bits(), "element {r},{c}");
            }
        }
    }

    #[test]
    fn separates_from_the_per_step_f32_form() {
        // Same order, narrowed after every add instead of once at the store.
        // Without this the width half of the fix is unpinned.
        let (a, b) = fixtures();
        let got = stock(&a, &b);
        let mut per_step = [0.0f32; 16];
        for r in 0..4 {
            for c in 0..4 {
                let ks = super::MULTIPLY_K_ORDER[r * 4 + c];
                let mut acc = a[r * 4 + ks[0]] * b[ks[0] * 4 + c];
                acc += a[r * 4 + ks[1]] * b[ks[1] * 4 + c];
                acc += a[r * 4 + ks[2]] * b[ks[2] * 4 + c];
                acc += a[r * 4 + ks[3]] * b[ks[3] * 4 + c];
                per_step[r * 4 + c] = acc;
            }
        }
        assert!(
            (0..16).any(|i| got[i].to_bits() != per_step[i].to_bits()),
            "fixture no longer separates narrow-once from narrow-per-step"
        );
    }

    /// The live client case the differential harness reported, replayed.
    ///
    /// Two composed right-angle rotations, captured operand-for-operand off a real
    /// call that the harness flagged on lane 6. Every one of that lane's four
    /// products is `+0.0 * negative`, so the sum is `-0.0` under every IEEE
    /// rounding mode and no summation order can change it — a sum of zeros is
    /// negative exactly when all its addends are. The original reported `+0.0`,
    /// which those bytes cannot produce, so it did not read these bytes: the
    /// snapshot is compared against live memory only *after* the original returns,
    /// which catches a value that changed and stayed changed, not one that changed
    /// and changed back. A sign-bit flip on a zero is what a matrix being rebuilt
    /// on another thread looks like.
    ///
    /// So this pins the kernel against real client data rather than against a
    /// transcription, and it is why lane 6's divergence is a harness artifact
    /// rather than an arithmetic bug.
    #[test]
    fn matches_the_captured_client_case_on_the_signed_zero_lane() {
        const A: [u32; 16] = [
            0xb33b_bd2e,
            0x3f80_0000,
            0,
            0,
            0xbf80_0000,
            0xb33b_bd2e,
            0,
            0,
            0,
            0,
            0x3f7f_ffff,
            0,
            0,
            0,
            0,
            0x3f80_0000,
        ];
        const B: [u32; 16] = [
            0x3f62_2204,
            0xb280_0000,
            0,
            0,
            0xb280_0000,
            0xbf62_2204,
            0,
            0,
            0,
            0x3200_0000,
            0xbf62_2204,
            0,
            0xbf59_87d2,
            0x3f9c_ee05,
            0xbe81_714e,
            0x3f7f_ffff,
        ];
        let (a, b) = (A.map(f32::from_bits), B.map(f32::from_bits));
        let got = stock(&a, &b);

        // Every product feeding lane 6 is a positive zero times a negative value.
        for k in super::MULTIPLY_K_ORDER[6] {
            let (fa, fb) = (a[4 + k], b[k * 4 + 2]);
            assert_eq!(
                (fa * fb).to_bits(),
                (-0.0f32).to_bits(),
                "k={k}: {fa} * {fb} should be -0.0"
            );
        }
        assert_eq!(
            got[6].to_bits(),
            (-0.0f32).to_bits(),
            "lane 6 must be -0.0; a sum of negative zeros cannot be +0.0"
        );

        // And the sign survives any regrouping, which is what rules out the
        // summation order as an explanation.
        let mut ascending = f64::from(a[4]) * f64::from(b[2]);
        for k in [1usize, 2, 3] {
            ascending += f64::from(a[4 + k]) * f64::from(b[k * 4 + 2]);
        }
        assert_eq!(
            super::super::f64_to_f32(ascending).to_bits(),
            (-0.0f32).to_bits(),
            "k-ascending regrouping must also be -0.0"
        );
    }

    #[test]
    fn separates_from_the_zero_seeded_k_ascending_form() {
        // The fixture has to actually distinguish the two summations, otherwise
        // this pins nothing and neither does the diff harness.
        let (a, b) = fixtures();
        let stock_form = stock(&a, &b);
        let d3dx_form = mul4x4(&a, &b);
        assert!(
            (0..16).any(|i| stock_form[i].to_bits() != d3dx_form[i].to_bits()),
            "fixture no longer separates the two summation orders"
        );
    }

    #[test]
    fn agrees_with_the_zero_seeded_form_on_exact_inputs() {
        // Small integers make every intermediate exact, so both orders agree —
        // which is exactly why the pre-existing tolerance tests never saw this.
        let a: [f32; 16] = [
            0.0, 1.0, 2.0, 3.0, //
            4.0, 5.0, 6.0, 7.0, //
            8.0, 9.0, 10.0, 11.0, //
            12.0, 13.0, 14.0, 15.0,
        ];
        let b: [f32; 16] = [
            16.0, 15.0, 14.0, 13.0, //
            12.0, 11.0, 10.0, 9.0, //
            8.0, 7.0, 6.0, 5.0, //
            4.0, 3.0, 2.0, 1.0,
        ];
        assert_eq!(stock(&a, &b), mul4x4(&a, &b));
    }
}

#[cfg(test)]
mod tests {
    use super::mul4x4;

    const IDENTITY: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ];

    fn approx(got: &[f32; 16], want: &[f32; 16], tol: f32) {
        for i in 0..16 {
            assert!(
                (got[i] - want[i]).abs() <= tol,
                "idx {i}: {} vs {}",
                got[i],
                want[i]
            );
        }
    }

    const SEQ: [f32; 16] = [
        1.0, 2.0, 3.0, 4.0, //
        5.0, 6.0, 7.0, 8.0, //
        9.0, 10.0, 11.0, 12.0, //
        13.0, 14.0, 15.0, 16.0,
    ];

    #[test]
    fn identity_is_right_neutral() {
        approx(&mul4x4(&SEQ, &IDENTITY), &SEQ, 0.0);
    }

    #[test]
    fn identity_is_left_neutral() {
        approx(&mul4x4(&IDENTITY, &SEQ), &SEQ, 0.0);
    }

    #[test]
    fn known_asymmetric_product() {
        // A is row 0 = [1,2,3,4] over an identity tail; B is identity with last
        // row [5,6,7,8]. Hand-computed A·B catches any row/column-major transpose.
        let a: [f32; 16] = [
            1.0, 2.0, 3.0, 4.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ];
        let b: [f32; 16] = [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            5.0, 6.0, 7.0, 8.0,
        ];
        let want: [f32; 16] = [
            21.0, 26.0, 31.0, 32.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            5.0, 6.0, 7.0, 8.0,
        ];
        approx(&mul4x4(&a, &b), &want, 0.0);
    }

    #[test]
    fn associativity_within_tolerance() {
        // (A·B)·C ≈ A·(B·C); f32 accumulation differs by association only.
        let a: [f32; 16] = [
            1.0, -2.0, 3.0, 0.5, //
            -4.0, 5.0, -0.25, 2.0, //
            0.0, 1.5, -3.0, 4.0, //
            2.5, -1.0, 0.75, -2.0,
        ];
        let b: [f32; 16] = [
            -1.0, 0.5, 2.0, -3.0, //
            4.0, -0.5, 1.0, 0.25, //
            -2.0, 3.0, -1.5, 0.0, //
            1.0, -2.5, 0.75, 2.0,
        ];
        let c: [f32; 16] = [
            2.0, -1.0, 0.0, 1.5, //
            -0.5, 3.0, -2.0, 0.25, //
            1.0, -3.0, 4.0, -0.75, //
            -2.5, 0.5, 1.0, -1.0,
        ];
        let left = mul4x4(&mul4x4(&a, &b), &c);
        let right = mul4x4(&a, &mul4x4(&b, &c));
        approx(&left, &right, 1e-2);
    }

    #[test]
    fn ones_times_ones_is_four() {
        let ones = [1.0f32; 16];
        approx(&mul4x4(&ones, &ones), &[4.0f32; 16], 0.0);
    }

    /// Pin the exact bit pattern of a representative animation-shaped product.
    ///
    /// The `k = 0..3` left-to-right `f32` accumulation must not drift (e.g. a
    /// reassociation into `(p0+p1)+(p2+p3)` or an `f64` intermediate would
    /// change these bits), since this is also the order the dispatched D3DX
    /// matrix multiply uses — the in-process animation update relies on the
    /// two agreeing to the bit.
    #[test]
    fn accumulation_order_is_bit_stable() {
        // A rotation-ish row matrix times a translation — the bone-compose shape.
        let rot: [f32; 16] = [
            0.36, 0.48, -0.8, 0.0, //
            -0.8, 0.6, 0.0, 0.0, //
            0.48, 0.64, 0.6, 0.0, //
            1.5, -2.25, 3.125, 1.0,
        ];
        let trans: [f32; 16] = [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            10.5, -20.25, 30.125, 1.0,
        ];
        let p = mul4x4(&rot, &trans);
        // Reference: the identical left-to-right k sum, computed independently.
        let mut want = [0.0f32; 16];
        for r in 0..4 {
            for c in 0..4 {
                want[r * 4 + c] = ((rot[r * 4] * trans[c] + rot[r * 4 + 1] * trans[4 + c])
                    + rot[r * 4 + 2] * trans[8 + c])
                    + rot[r * 4 + 3] * trans[12 + c];
            }
        }
        for i in 0..16 {
            assert_eq!(p[i].to_bits(), want[i].to_bits(), "lane {i}");
        }
    }
}

/// Element-wise sum of two 4×4 matrices (16 floats): `out = a + b`.
pub fn c44_matrix__add__7bc290(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0f32; 16];
    for ((o, &x), &y) in out.iter_mut().zip(a).zip(b) {
        *o = x + y;
    }
    out
}

#[cfg(test)]
mod tests_c44_matrix__add__7bc290 {
    use super::c44_matrix__add__7bc290 as add;

    fn seq(base: f32) -> [f32; 16] {
        core::array::from_fn(|i| base + i as f32)
    }

    #[test]
    fn zero_is_neutral() {
        let a = seq(1.0);
        let z = [0.0f32; 16];
        assert_eq!(add(&a, &z), a);
    }

    #[test]
    fn commutative() {
        let a = seq(1.0);
        let b = seq(100.0);
        assert_eq!(add(&a, &b), add(&b, &a));
    }

    #[test]
    fn known_value() {
        let a = [1.0f32; 16];
        let b = [2.0f32; 16];
        assert_eq!(add(&a, &b), [3.0f32; 16]);
    }
}

/// Builds an orthonormal 3×3 basis (stored in the first three rows of a 4×4 matrix).
///
/// From a direction vector `dir` and a seed "up" reference taken from the
/// matrix's existing row 0.
///
/// Mirrors the original layout: row 1 (`out[4..7]`) = `up × dir`, normalized;
/// row 0 (`out[0..3]`) = `dir × row1`; row 2 (`out[8..11]`) = `dir`. `up` is the
/// incoming `out[0..3]`. The normalize is skipped when the cross length falls
/// below an epsilon (degenerate `up`/`dir` near-parallel).
pub fn c44_matrix__build_basis_from_dir__710a80(dir: &[f32; 3], out: &mut [f32; 16]) {
    const EPS: f32 = 1.0e-8;

    let dx = dir[0];
    let dy = dir[1];
    let dz = dir[2];

    // Seed "up" = current row 0.
    let ux = out[0];
    let uy = out[1];
    let uz = out[2];

    // row1 = up × dir
    let mut r1x = uz * dy - uy * dz;
    let mut r1y = ux * dz - uz * dx;
    let mut r1z = uy * dx - ux * dy;

    let len = (r1x * r1x + r1y * r1y + r1z * r1z).sqrt();
    if len.abs() >= EPS {
        let inv = 1.0 / len;
        r1x *= inv;
        r1y *= inv;
        r1z *= inv;
    }
    out[4] = r1x;
    out[5] = r1y;
    out[6] = r1z;

    // row0 = dir × row1
    out[0] = r1y * dz - dy * r1z;
    out[1] = dx * r1z - r1x * dz;
    out[2] = r1x * dy - dx * r1y;

    // row2 = dir
    out[8] = dx;
    out[9] = dy;
    out[10] = dz;
}

#[cfg(test)]
mod tests_c44_matrix__build_basis_from_dir__710a80 {
    use super::c44_matrix__build_basis_from_dir__710a80 as build;

    fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }
    fn norm(a: [f32; 3]) -> f32 {
        dot(a, a).sqrt()
    }

    #[test]
    fn orthonormal_basis_from_axis() {
        // dir = +Z, up seed = +Y.
        let dir = [0.0f32, 0.0, 1.0];
        let mut m = [0.0f32; 16];
        m[0] = 0.0;
        m[1] = 1.0;
        m[2] = 0.0;
        build(&dir, &mut m);

        let r0 = [m[0], m[1], m[2]];
        let r1 = [m[4], m[5], m[6]];
        let r2 = [m[8], m[9], m[10]];

        // row2 == dir exactly.
        assert_eq!(r2, [0.0, 0.0, 1.0]);
        // row1 unit length.
        assert!((norm(r1) - 1.0).abs() < 1e-5, "r1 len {}", norm(r1));
        // row0 unit length (dir and row1 are orthogonal unit vectors).
        assert!((norm(r0) - 1.0).abs() < 1e-5, "r0 len {}", norm(r0));
        // mutual orthogonality.
        assert!(dot(r0, r1).abs() < 1e-5);
        assert!(dot(r1, r2).abs() < 1e-5);
        assert!(dot(r0, r2).abs() < 1e-5);
    }

    #[test]
    fn known_value_z_dir_y_up() {
        // up=+Y, dir=+Z. The original forms row1 = dir×up = (-1,0,0); then
        // row0 = dir×row1 = (0,1,0).
        let dir = [0.0f32, 0.0, 1.0];
        let mut m = [0.0f32; 16];
        m[1] = 1.0;
        build(&dir, &mut m);
        let r1 = [m[4], m[5], m[6]];
        let r0 = [m[0], m[1], m[2]];
        assert!((r1[0] + 1.0).abs() < 1e-6 && r1[1].abs() < 1e-6 && r1[2].abs() < 1e-6);
        assert!(r0[0].abs() < 1e-6 && (r0[1] - 1.0).abs() < 1e-6 && r0[2].abs() < 1e-6);
    }

    #[test]
    fn degenerate_parallel_skips_normalize() {
        // up parallel to dir => cross is zero => normalize skipped, row1 stays 0.
        let dir = [0.0f32, 0.0, 1.0];
        let mut m = [0.0f32; 16];
        m[2] = 1.0; // up = +Z parallel to dir
        build(&dir, &mut m);
        assert_eq!([m[4], m[5], m[6]], [0.0, 0.0, 0.0]);
    }
}

/// Scales a 4×4 matrix (16 floats) by a scalar: `out = src · s`.
pub fn c44_matrix__multiply_scalar__7bc8e0(src: &[f32; 16], s: f32) -> [f32; 16] {
    src.map(|x| x * s)
}

#[cfg(test)]
mod tests_c44_matrix__multiply_scalar__7bc8e0 {
    use super::c44_matrix__multiply_scalar__7bc8e0 as scale;

    fn seq(base: f32) -> [f32; 16] {
        core::array::from_fn(|i| base + i as f32)
    }

    #[test]
    fn one_is_identity() {
        let a = seq(1.0);
        assert_eq!(scale(&a, 1.0), a);
    }

    #[test]
    fn zero_zeros_all() {
        let a = seq(1.0);
        assert_eq!(scale(&a, 0.0), [0.0f32; 16]);
    }

    #[test]
    fn distributes_over_double() {
        let a = seq(3.0);
        let twice = scale(&a, 2.0);
        for i in 0..16 {
            assert_eq!(twice[i], a[i] + a[i]);
        }
    }

    #[test]
    fn known_value() {
        let a = [2.0f32; 16];
        assert_eq!(scale(&a, 2.5), [5.0f32; 16]);
    }
}

/// Builds a 3×3 rotation matrix (Rodrigues' formula) from an axis and angle.
///
/// Laid out in the first 12 floats of a 4×4 matrix (rows of three, then a zeroed
/// fourth column at indices 9..12).
///
/// When `skip_normalize` is false the axis is normalized first; a true value
/// trusts the caller's axis to be unit length. The remaining matrix slots
/// (indices 12..16) are left untouched, matching the original which only writes
/// 0..12.
///
/// The arithmetic is `0x7be490`'s — the same routine against a narrower
/// destination, its eighty floating-point instructions identical once the
/// destination offsets are masked off — so it lives in
/// `c33_matrix__from_axis_angle__7be490` and this entry point only places the
/// result. That kernel's flag runs the other way round from this one's.
pub fn c44_matrix__set_rotation_axis_angle__7bb860(
    axis: &[f32; 3],
    angle: f32,
    skip_normalize: bool,
) -> [f32; 12] {
    let r =
        crate::math::matrix33::c33_matrix__from_axis_angle__7be490(axis, angle, !skip_normalize);
    let mut out = [0.0f32; 12];
    out[0..9].copy_from_slice(&r);
    // out[9..12] stay zero (translation column).
    out
}

#[cfg(test)]
mod tests_c44_matrix__set_rotation_axis_angle__7bb860 {
    use super::c44_matrix__set_rotation_axis_angle__7bb860 as rot;

    fn row(m: &[f32; 12], r: usize) -> [f32; 3] {
        [m[r * 3], m[r * 3 + 1], m[r * 3 + 2]]
    }
    fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    #[test]
    fn zero_angle_is_identity() {
        let m = rot(&[0.0, 0.0, 1.0], 0.0, true);
        let id: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        for i in 0..9 {
            assert!((m[i] - id[i]).abs() < 5e-6, "i {i}: {}", m[i]);
        }
        assert_eq!([m[9], m[10], m[11]], [0.0, 0.0, 0.0]);
    }

    #[test]
    fn orthonormal_rows() {
        // arbitrary axis + angle; result must be orthonormal.
        let axis = [1.0f32, 2.0, -0.5];
        let m = rot(&axis, 0.73, false);
        for r in 0..3 {
            let v = row(&m, r);
            assert!((dot(v, v) - 1.0).abs() < 1e-4, "row {r} not unit");
        }
        assert!(dot(row(&m, 0), row(&m, 1)).abs() < 1e-4);
        assert!(dot(row(&m, 1), row(&m, 2)).abs() < 1e-4);
        assert!(dot(row(&m, 0), row(&m, 2)).abs() < 1e-4);
    }

    #[test]
    fn rotation_about_z_quarter_turn() {
        // 90° about +Z. As a column-major 3x3 the standard +Z rotation has
        // off-diagonal sin terms; check the diagonal cos terms and the structure.
        let m = rot(&[0.0, 0.0, 1.0], std::f32::consts::FRAC_PI_2, true);
        // cos(90)=0 on the xy diagonal, 1 on zz.
        assert!(m[0].abs() < 5e-6 && m[4].abs() < 5e-6);
        assert!((m[8] - 1.0).abs() < 5e-6);
        // ax=ay=0 so xy,xz,yz vanish; out[1]=az*s=+1, out[3]=-az*s=-1.
        assert!((m[1] - 1.0).abs() < 5e-6);
        assert!((m[3] + 1.0).abs() < 5e-6);
    }

    #[test]
    fn determinant_is_one() {
        let m = rot(&[0.3f32, -1.0, 0.8], -1.2, false);
        let det = m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6])
            + m[2] * (m[3] * m[7] - m[4] * m[6]);
        assert!((det - 1.0).abs() < 1e-4, "det {det}");
    }
}

/// Transforms a 3D point by a 3×4 affine matrix stored column-major.
///
/// `out = M3x3 · point + translation`.
///
/// Row `r` of the output reads `mat[r]`, `mat[3+r]`, `mat[6+r]` as the basis
/// columns scaling `point.{x,y,z}` and `mat[9+r]` as the translation. Returns
/// the transformed point by value; the original writes it to both an output
/// vector and back into the input point.
pub fn c44_matrix__transform_point__7bb290(point: &[f32; 3], mat: &[f32; 12]) -> [f32; 3] {
    let px = point[0];
    let py = point[1];
    let pz = point[2];

    let x = px * mat[0] + py * mat[3] + pz * mat[6] + mat[9];
    let y = px * mat[1] + py * mat[4] + pz * mat[7] + mat[10];
    let z = px * mat[2] + py * mat[5] + pz * mat[8] + mat[11];
    [x, y, z]
}

#[cfg(test)]
mod tests_c44_matrix__transform_point__7bb290 {
    use super::c44_matrix__transform_point__7bb290 as xform;

    // Column-major identity 3x4 (no translation): columns are the standard basis.
    const ID: [f32; 12] = [
        1.0, 0.0, 0.0, // col 0 (scales point.x) -> contributes to x
        0.0, 1.0, 0.0, // col 1 (scales point.y) -> contributes to y
        0.0, 0.0, 1.0, // col 2 (scales point.z) -> contributes to z
        0.0, 0.0, 0.0, // translation
    ];

    #[test]
    fn identity_is_neutral() {
        let p = [3.0f32, -2.0, 7.5];
        assert_eq!(xform(&p, &ID), p);
    }

    #[test]
    fn translation_only() {
        let mut m = ID;
        m[9] = 10.0;
        m[10] = 20.0;
        m[11] = 30.0;
        let p = [1.0f32, 2.0, 3.0];
        assert_eq!(xform(&p, &m), [11.0, 22.0, 33.0]);
    }

    #[test]
    fn linearity_affine() {
        // For an affine map, T(p) - T(0) is linear: T(a+b) - T(0) == (T(a)-T(0)) + (T(b)-T(0)).
        let m: [f32; 12] = [
            0.5, 1.0, -0.5, // col0
            2.0, 0.0, 1.0, // col1
            -1.0, 0.5, 0.25, // col2
            4.0, -3.0, 2.0, // translation
        ];
        let a = [1.0f32, 2.0, 3.0];
        let b = [-2.0f32, 0.5, 1.0];
        let ab = [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
        let t0 = xform(&[0.0, 0.0, 0.0], &m);
        let ta = xform(&a, &m);
        let tb = xform(&b, &m);
        let tab = xform(&ab, &m);
        for i in 0..3 {
            let lhs = tab[i] - t0[i];
            let rhs = (ta[i] - t0[i]) + (tb[i] - t0[i]);
            assert!((lhs - rhs).abs() < 1e-4, "i {i}: {lhs} vs {rhs}");
        }
    }

    #[test]
    fn known_value() {
        let m: [f32; 12] = [
            2.0, 0.0, 0.0, // x scaled by 2
            0.0, 3.0, 0.0, // y scaled by 3
            0.0, 0.0, 4.0, // z scaled by 4
            1.0, 1.0, 1.0, // +(1,1,1)
        ];
        let p = [1.0f32, 1.0, 1.0];
        assert_eq!(xform(&p, &m), [3.0, 4.0, 5.0]);
    }
}

/// In-place post-translate concat: adds `M3x3 · point` into the matrix's translation column.
///
/// The matrix is a column-major 3×4 affine: rotation in indices 0..9,
/// translation in indices 9..12. Each translation component `t[r]` gains
/// `mat[r]·px + mat[3+r]·py + mat[6+r]·pz`, the same column convention used by
/// the point transform.
pub fn c44_matrix__translate_local__7bb990(mat: &mut [f32; 12], point: &[f32; 3]) {
    let px = point[0];
    let py = point[1];
    let pz = point[2];

    mat[9] += mat[0] * px + mat[3] * py + mat[6] * pz;
    mat[10] += mat[1] * px + mat[4] * py + mat[7] * pz;
    mat[11] += mat[2] * px + mat[5] * py + mat[8] * pz;
}

#[cfg(test)]
mod tests_c44_matrix__translate_local__7bb990 {
    use super::c44_matrix__translate_local__7bb990 as translate;

    // Column-major identity rotation, translation = (5,6,7).
    fn id_with_t(t: [f32; 3]) -> [f32; 12] {
        [
            1.0, 0.0, 0.0, // col0
            0.0, 1.0, 0.0, // col1
            0.0, 0.0, 1.0, // col2
            t[0], t[1], t[2],
        ]
    }

    #[test]
    fn identity_adds_point_to_translation() {
        let mut m = id_with_t([5.0, 6.0, 7.0]);
        translate(&mut m, &[1.0, 2.0, 3.0]);
        assert_eq!([m[9], m[10], m[11]], [6.0, 8.0, 10.0]);
    }

    #[test]
    fn zero_point_no_change() {
        let mut m = id_with_t([5.0, 6.0, 7.0]);
        let before = m;
        translate(&mut m, &[0.0, 0.0, 0.0]);
        assert_eq!(m, before);
    }

    #[test]
    fn rotation_applies_to_point() {
        // diagonal scale rotation = diag(2,3,4); point (1,1,1) => add (2,3,4).
        let mut m: [f32; 12] = [
            2.0, 0.0, 0.0, //
            0.0, 3.0, 0.0, //
            0.0, 0.0, 4.0, //
            0.0, 0.0, 0.0,
        ];
        translate(&mut m, &[1.0, 1.0, 1.0]);
        assert_eq!([m[9], m[10], m[11]], [2.0, 3.0, 4.0]);
    }

    #[test]
    fn rotation_untouched() {
        let mut m = id_with_t([0.0, 0.0, 0.0]);
        m[1] = 0.5;
        let rot_before: [f32; 9] = [m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7], m[8]];
        translate(&mut m, &[9.0, 8.0, 7.0]);
        let rot_after: [f32; 9] = [m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7], m[8]];
        assert_eq!(rot_before, rot_after);
    }
}

/// `C44Matrix::InvertOrthonormal`.
///
/// Inverse of a rotation (orthonormal 3×3) plus translation matrix. The inverse
/// rotation is the transpose of the 3×3 block (rows at indices 0/1/2, 4/5/6,
/// 8/9/10); the inverse translation is `-t · R` where `t` is the source
/// translation (indices 12/13/14).
///
/// The original does not compute that last row as three dot products. It
/// transposes the 3×3 into a fresh matrix whose translation row is **zero**,
/// negates each source translation component into an `f32` slot
/// (`0x7bd767`..`0x7bd7bb`), and then `call`s `C44Matrix::Translate` at
/// `0x7bd7fc`. Doing the same here is what makes the last row match: it
/// inherits Translate's `z, y, x` summation and its single narrowing instead of
/// a left-to-right per-step `f32` dot, and it inherits the trailing add of the
/// zeroed translation, which is not a no-op — it turns a `-0.0` row into `+0.0`.
pub fn c44_matrix__invert_orthonormal__7bd700(src: &[f32; 16]) -> [f32; 16] {
    let mut out = [
        src[0], src[4], src[8], 0.0, //
        src[1], src[5], src[9], 0.0, //
        src[2], src[6], src[10], 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ];
    c44_matrix__translate__7bdc40(&mut out, &[-src[12], -src[13], -src[14]]);
    out
}

#[cfg(test)]
mod tests_c44_matrix__invert_orthonormal__7bd700 {
    use super::c44_matrix__invert_orthonormal__7bd700 as invert;

    #[test]
    fn last_row_goes_through_translate_not_a_dot_product() {
        // Separates the original's shape - transpose, zero translation, then
        // Translate with the negated translation - from the three left-to-right
        // per-step f32 dot products this used to compute. All three components
        // differ on this fixture. Bit patterns, not decimals.
        let src = [
            f32::from_bits(0x3fa9_7fda),
            f32::from_bits(0x3fc4_e0e7),
            f32::from_bits(0xbec7_e3e8),
            f32::from_bits(0xbfc1_7f6e),
            f32::from_bits(0x3f16_16a3),
            f32::from_bits(0x3f9f_27ba),
            f32::from_bits(0x3fee_fb64),
            f32::from_bits(0x3fb1_eb2b),
            f32::from_bits(0x3f5a_9220),
            f32::from_bits(0xbf6f_66dc),
            f32::from_bits(0xbf7f_26d3),
            f32::from_bits(0xbfaf_ec4d),
            f32::from_bits(0x41be_a747),
            f32::from_bits(0xc1b0_6e1e),
            f32::from_bits(0x3f0f_5abc),
            f32::from_bits(0x3f82_8002),
        ];
        let got = invert(&src);

        // The rotation block is the plain transpose and must be bit-identical.
        assert_eq!(
            [
                got[0], got[1], got[2], got[4], got[5], got[6], got[8], got[9], got[10]
            ],
            [
                src[0], src[4], src[8], src[1], src[5], src[9], src[2], src[6], src[10]
            ]
        );

        let transposed = [
            src[0], src[4], src[8], 0.0, //
            src[1], src[5], src[9], 0.0, //
            src[2], src[6], src[10], 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ];
        let neg = [
            f64::from(-src[12]),
            f64::from(-src[13]),
            f64::from(-src[14]),
        ];
        for i in 0..3 {
            let elem = |j: usize| f64::from(transposed[j]);
            let want = super::super::f64_to_f32(
                ((elem(8 + i) * neg[2] + elem(4 + i) * neg[1]) + elem(i) * neg[0]) + elem(12 + i),
            );
            assert_eq!(got[12 + i].to_bits(), want.to_bits(), "component {i}");

            let dot =
                -((src[4 * i] * src[12] + src[4 * i + 1] * src[13]) + src[4 * i + 2] * src[14]);
            assert_ne!(
                got[12 + i].to_bits(),
                dot.to_bits(),
                "fixture no longer separates component {i}"
            );
        }
    }

    // Row-major 4x4 multiply for verifying inv·M == I.
    fn mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
        let mut out = [0.0f32; 16];
        for r in 0..4 {
            for c in 0..4 {
                let mut s = a[r * 4] * b[c];
                s += a[r * 4 + 1] * b[4 + c];
                s += a[r * 4 + 2] * b[8 + c];
                s += a[r * 4 + 3] * b[12 + c];
                out[r * 4 + c] = s;
            }
        }
        out
    }

    fn approx_id(m: &[f32; 16]) {
        let id: [f32; 16] = [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ];
        for i in 0..16 {
            assert!((m[i] - id[i]).abs() < 1e-4, "idx {i}: {}", m[i]);
        }
    }

    // Row-major rotation-about-Z + translation.
    fn rot_z_trans(theta: f32, t: [f32; 3]) -> [f32; 16] {
        let c = theta.cos();
        let s = theta.sin();
        [
            c, s, 0.0, 0.0, //
            -s, c, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            t[0], t[1], t[2], 1.0,
        ]
    }

    #[test]
    fn identity_inverts_to_identity() {
        let id: [f32; 16] = [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ];
        assert_eq!(invert(&id), id);
    }

    #[test]
    fn inverse_composes_to_identity_rowmajor() {
        let m = rot_z_trans(0.7, [3.0, -4.0, 5.0]);
        let inv = invert(&m);
        approx_id(&mul(&inv, &m));
        approx_id(&mul(&m, &inv));
    }

    #[test]
    fn known_pure_translation() {
        // No rotation: inverse just negates the translation.
        let m: [f32; 16] = [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            10.0, -20.0, 30.0, 1.0,
        ];
        let inv = invert(&m);
        assert_eq!([inv[12], inv[13], inv[14]], [-10.0, 20.0, -30.0]);
    }

    #[test]
    fn transpose_of_rotation_block() {
        let m: [f32; 16] = [
            1.0, 2.0, 3.0, 0.0, //
            4.0, 5.0, 6.0, 0.0, //
            7.0, 8.0, 9.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ];
        let inv = invert(&m);
        // rows become columns of the original 3x3.
        assert_eq!([inv[0], inv[1], inv[2]], [1.0, 4.0, 7.0]);
        assert_eq!([inv[4], inv[5], inv[6]], [2.0, 5.0, 8.0]);
        assert_eq!([inv[8], inv[9], inv[10]], [3.0, 6.0, 9.0]);
    }
}

/// Pre-multiplies a 4x4 matrix by the rotation built from a quaternion: `out = R(quat) * m`.
///
/// `quat` is `[x, y, z, w]`. The rotation `R` is the standard quaternion->matrix
/// in the original's row-major scratch layout, then composed with the incoming
/// matrix. The original does not inline that product — it `call`s `0x7bc6a0` at
/// `0x7bded7` — so the composition goes through
/// [`c44_matrix__multiply__7bc6a0`] and inherits its per-element summation
/// order, not [`mul4x4`]'s. The original writes the 16-float result back into
/// the receiver; here it is returned by value.
pub fn c44_matrix__rotate_quaternion__7bddb0(m: &[f32; 16], quat: &[f32; 4]) -> [f32; 16] {
    let x = quat[0];
    let y = quat[1];
    let z = quat[2];
    let w = quat[3];

    // `q + q` is exact at any precision, so the doubled components need no
    // width care even though the original leaves two of them on the x87 stack.
    let x2 = x + x;
    let y2 = y + y;
    let z2 = z + z;

    let mul = |a: f32, b: f32| f64::from(a) * f64::from(b);

    // Seven products are narrowed at their own `FSTP m32` and read back as f32.
    let xw = super::f64_to_f32(mul(x2, w));
    let yw = super::f64_to_f32(mul(y2, w));
    let zw = super::f64_to_f32(mul(z2, w));
    let xx = super::f64_to_f32(mul(x2, x));
    let yx = super::f64_to_f32(mul(y2, x));
    let zx = super::f64_to_f32(mul(z2, x));
    let zy = super::f64_to_f32(mul(z2, y));

    // Two are not, and this is where the diagonal stops being symmetric.
    // `yy` never leaves the x87 stack at all, and `zz` is stored by `FST m32`
    // at 0x7bde1f - which writes an f32 copy but *keeps* the wide value live.
    // So each diagonal term mixes the two differently:
    //   [0]  uses the wide zz and the wide yy   (0x7bde22)
    //   [5]  reloads the narrowed zz, plus xx   (0x7bde4b)
    //   [10] uses the wide yy, plus xx          (0x7bde8d)
    let yy = mul(y2, y);
    let zz_wide = mul(z2, z);
    let zz = super::f64_to_f32(zz_wide);

    let one = 1.0_f64;
    let sub = |a: f32, b: f32| f64::from(a) - f64::from(b);
    let add = |a: f32, b: f32| f64::from(a) + f64::from(b);

    let r = [
        super::f64_to_f32(one - (zz_wide + yy)),       // [0]
        super::f64_to_f32(add(yx, zw)),                // [1]
        super::f64_to_f32(sub(zx, yw)),                // [2]
        0.0,                                           // [3]
        super::f64_to_f32(sub(yx, zw)),                // [4]
        super::f64_to_f32(one - add(zz, xx)),          // [5]
        super::f64_to_f32(add(zy, xw)),                // [6]
        0.0,                                           // [7]
        super::f64_to_f32(add(zx, yw)),                // [8]
        super::f64_to_f32(sub(zy, xw)),                // [9]
        super::f64_to_f32(one - (yy + f64::from(xx))), // [10]
        0.0,                                           // [11]
        0.0,                                           // [12]
        0.0,                                           // [13]
        0.0,                                           // [14]
        1.0,                                           // [15]
    ];

    super::matrix44::c44_matrix__multiply__7bc6a0(&r, m)
}

#[cfg(test)]
mod tests_c44_matrix__rotate_quaternion__7bddb0 {
    use super::c44_matrix__rotate_quaternion__7bddb0 as rotq;

    #[test]
    fn diagonal_terms_do_not_share_one_width() {
        // Composed against the identity so the product passes the rotation
        // through unchanged and the diagonal is read directly. Diagonal [0]
        // takes both squares wide, [10] takes yy wide against a narrowed xx,
        // and [5] takes both narrowed - so computing all three in f32, as this
        // used to, moves [0] and [10] but not [5]. This fixture separates
        // exactly those two.
        let quat = [
            f32::from_bits(0xbf5d_00ca),
            f32::from_bits(0x3f06_3c15),
            f32::from_bits(0xbf6a_c781),
            f32::from_bits(0xbf49_ff68),
        ];
        let got = rotq(&IDENTITY, &quat);

        let (x, y, z) = (quat[0], quat[1], quat[2]);
        let (x2, y2, z2) = (x + x, y + y, z + z);
        let xx = super::super::f64_to_f32(f64::from(x2) * f64::from(x));
        let yy = f64::from(y2) * f64::from(y);
        let zz_wide = f64::from(z2) * f64::from(z);
        let zz = super::super::f64_to_f32(zz_wide);

        let want0 = super::super::f64_to_f32(1.0_f64 - (zz_wide + yy));
        let want5 = super::super::f64_to_f32(1.0_f64 - (f64::from(zz) + f64::from(xx)));
        let want10 = super::super::f64_to_f32(1.0_f64 - (yy + f64::from(xx)));
        assert_eq!(got[0].to_bits(), want0.to_bits(), "diagonal 0");
        assert_eq!(got[5].to_bits(), want5.to_bits(), "diagonal 5");
        assert_eq!(got[10].to_bits(), want10.to_bits(), "diagonal 10");

        // The all-f32 form this replaced, for comparison.
        let (yy32, zz32) = (y2 * y, z2 * z);
        assert_ne!(
            got[0].to_bits(),
            (1.0f32 - (zz32 + yy32)).to_bits(),
            "fixture no longer separates diagonal 0"
        );
        assert_ne!(
            got[10].to_bits(),
            (1.0f32 - (yy32 + xx)).to_bits(),
            "fixture no longer separates diagonal 10"
        );
    }

    const IDENTITY: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];

    fn approx(a: &[f32; 16], b: &[f32; 16], tol: f32) {
        for i in 0..16 {
            assert!((a[i] - b[i]).abs() <= tol, "idx {i}: {} vs {}", a[i], b[i]);
        }
    }

    #[test]
    fn identity_quat_on_identity_is_identity() {
        let out = rotq(&IDENTITY, &[0.0, 0.0, 0.0, 1.0]);
        approx(&out, &IDENTITY, 1e-6);
    }

    #[test]
    fn rotation_block_is_orthonormal() {
        let q = {
            let raw = [0.3f32, -0.5, 0.2, 0.8];
            let n = (raw[0] * raw[0] + raw[1] * raw[1] + raw[2] * raw[2] + raw[3] * raw[3]).sqrt();
            [raw[0] / n, raw[1] / n, raw[2] / n, raw[3] / n]
        };
        let r = rotq(&IDENTITY, &q); // R * I = R
        let rows = [[r[0], r[1], r[2]], [r[4], r[5], r[6]], [r[8], r[9], r[10]]];
        let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        for row in rows {
            assert!((dot(row, row) - 1.0).abs() < 1e-4);
        }
        assert!(dot(rows[0], rows[1]).abs() < 1e-4);
        assert!(dot(rows[1], rows[2]).abs() < 1e-4);
        assert!(dot(rows[0], rows[2]).abs() < 1e-4);
    }

    #[test]
    fn ninety_deg_about_z() {
        let s = (std::f32::consts::FRAC_PI_4).sin();
        let c = (std::f32::consts::FRAC_PI_4).cos();
        let r = rotq(&IDENTITY, &[0.0, 0.0, s, c]);
        assert!(r[0].abs() < 1e-5 && r[5].abs() < 1e-5);
        assert!((r[10] - 1.0).abs() < 1e-5);
        assert!((r[1].abs() - 1.0).abs() < 1e-5);
        assert!((r[4].abs() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn composition_left_multiplies() {
        let q = [0.1f32, 0.2, 0.3, 0.9];
        let m = [
            2.0f32, 0.0, 1.0, 0.0, -1.0, 3.0, 0.0, 0.0, 0.5, 0.0, 4.0, 0.0, 1.0, 2.0, 3.0, 1.0,
        ];
        let out = rotq(&m, &q);
        let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
        let (x2, y2, z2) = (x + x, y + y, z + z);
        let r = [
            1.0 - (z2 * z + y2 * y),
            y2 * x + z2 * w,
            z2 * x - y2 * w,
            0.0,
            y2 * x - z2 * w,
            1.0 - (z2 * z + x2 * x),
            z2 * y + x2 * w,
            0.0,
            z2 * x + y2 * w,
            z2 * y - x2 * w,
            1.0 - (y2 * y + x2 * x),
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
        ];
        // The original composes through `0x7bc6a0`, so the reference is that
        // product and the comparison can be exact rather than tolerant.
        let want = super::c44_matrix__multiply__7bc6a0(&r, &m);
        for i in 0..16 {
            assert_eq!(out[i].to_bits(), want[i].to_bits(), "element {i}");
        }
    }
}

/// Scales the upper-left 3x3 of a 4x4 matrix (4-float row stride) row-wise by a 3-vector, in place.
///
/// Row 0 (indices 0,1,2) is multiplied by `s.x`, row 1 (indices 4,5,6) by `s.y`,
/// row 2 (indices 8,9,10) by `s.z`. The translation column and fourth row are
/// untouched.
pub fn c44_matrix__scale_rows_vec__7bdca0(mat: &mut [f32; 16], s: &[f32; 3]) {
    let sx = s[0];
    mat[0] *= sx;
    mat[1] *= sx;
    mat[2] *= sx;
    let sy = s[1];
    mat[4] *= sy;
    mat[5] *= sy;
    mat[6] *= sy;
    let sz = s[2];
    mat[8] *= sz;
    mat[9] *= sz;
    mat[10] *= sz;
}

#[cfg(test)]
mod tests_c44_matrix__scale_rows_vec__7bdca0 {
    use super::c44_matrix__scale_rows_vec__7bdca0 as scale;

    fn full() -> [f32; 16] {
        [
            1.0, 2.0, 3.0, 99.0, 4.0, 5.0, 6.0, 99.0, 7.0, 8.0, 9.0, 99.0, 10.0, 11.0, 12.0, 1.0,
        ]
    }

    #[test]
    fn ones_is_neutral() {
        let mut m = full();
        let before = m;
        scale(&mut m, &[1.0, 1.0, 1.0]);
        assert_eq!(m, before);
    }

    #[test]
    fn per_row_scale() {
        let mut m = full();
        scale(&mut m, &[2.0, 3.0, 0.5]);
        assert_eq!([m[0], m[1], m[2]], [2.0, 4.0, 6.0]);
        assert_eq!([m[4], m[5], m[6]], [12.0, 15.0, 18.0]);
        assert_eq!([m[8], m[9], m[10]], [3.5, 4.0, 4.5]);
    }

    #[test]
    fn translation_and_pads_untouched() {
        let mut m = full();
        scale(&mut m, &[2.0, 3.0, 0.5]);
        assert_eq!([m[3], m[7], m[11]], [99.0, 99.0, 99.0]);
        assert_eq!([m[12], m[13], m[14], m[15]], [10.0, 11.0, 12.0, 1.0]);
    }

    #[test]
    fn zero_zeros_rows() {
        let mut m = full();
        scale(&mut m, &[0.0, 0.0, 0.0]);
        assert_eq!([m[0], m[1], m[2]], [0.0, 0.0, 0.0]);
        assert_eq!([m[4], m[5], m[6]], [0.0, 0.0, 0.0]);
        assert_eq!([m[8], m[9], m[10]], [0.0, 0.0, 0.0]);
    }
}

/// Scales the leading 3x3 (stored contiguously in indices 0..9) of a matrix.
///
/// Row-wise by three scalars, in place.
///
/// Unlike [`c44_matrix__scale_rows_vec__7bdca0`], this overload reads a
/// contiguous 3x3 block (no 4-float row stride): indices 0,1,2 x`sx`; 3,4,5
/// x`sy`; 6,7,8 x`sz`. The original touches only the first nine floats.
pub fn c44_matrix__scale_rows__7be670(mat: &mut [f32; 9], sx: f32, sy: f32, sz: f32) {
    mat[0] *= sx;
    mat[1] *= sx;
    mat[2] *= sx;
    mat[3] *= sy;
    mat[4] *= sy;
    mat[5] *= sy;
    mat[6] *= sz;
    mat[7] *= sz;
    mat[8] *= sz;
}

#[cfg(test)]
mod tests_c44_matrix__scale_rows__7be670 {
    use super::c44_matrix__scale_rows__7be670 as scale;

    fn seq() -> [f32; 9] {
        [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]
    }

    #[test]
    fn ones_is_neutral() {
        let mut m = seq();
        let before = m;
        scale(&mut m, 1.0, 1.0, 1.0);
        assert_eq!(m, before);
    }

    #[test]
    fn per_row_scale() {
        let mut m = seq();
        scale(&mut m, 2.0, 3.0, 0.5);
        assert_eq!([m[0], m[1], m[2]], [2.0, 4.0, 6.0]);
        assert_eq!([m[3], m[4], m[5]], [12.0, 15.0, 18.0]);
        assert_eq!([m[6], m[7], m[8]], [3.5, 4.0, 4.5]);
    }

    #[test]
    fn zero_zeros_rows() {
        let mut m = seq();
        scale(&mut m, 0.0, 0.0, 0.0);
        assert_eq!(m, [0.0f32; 9]);
    }

    #[test]
    fn known_value() {
        let mut m = [10.0f32; 9];
        scale(&mut m, 0.1, 0.2, 0.3);
        for &v in &m[0..3] {
            assert!((v - 1.0).abs() < 1e-5);
        }
        for &v in &m[3..6] {
            assert!((v - 2.0).abs() < 1e-5);
        }
        for &v in &m[6..9] {
            assert!((v - 3.0).abs() < 1e-5);
        }
    }
}

/// Builds a full 4x4 Rodrigues axis-angle rotation matrix, row-major.
///
/// The upper-left 3x3 is the rotation; the fourth row/column form the affine
/// identity (zeros with `out[15] == 1`). When `skip_normalize` is false the axis
/// is normalized first. Mirrors the original's exact element layout (the off-
/// diagonal sin terms are placed per the original's `out[1]/out[2]/...` writes).
///
/// The arithmetic is `0x7be490`'s, so it lives in
/// `c33_matrix__from_axis_angle__7be490` (whose flag runs the other way round) and
/// this entry point differs from it only in where the nine elements land and in
/// the six zeros and the `1.0` it writes around them (`0x7bdb76`..`0x7bdb9a`).
pub fn c44_matrix__set_rotation_axis_angle__7bdb00(
    axis: &[f32; 3],
    angle: f32,
    skip_normalize: bool,
) -> [f32; 16] {
    let r =
        crate::math::matrix33::c33_matrix__from_axis_angle__7be490(axis, angle, !skip_normalize);
    let mut out = [0.0f32; 16];
    for row in 0..3 {
        out[row * 4..row * 4 + 3].copy_from_slice(&r[row * 3..row * 3 + 3]);
    }
    out[15] = 1.0;
    out
}

#[cfg(test)]
mod tests_c44_matrix__set_rotation_axis_angle__7bdb00 {
    use super::c44_matrix__set_rotation_axis_angle__7bdb00 as rot;

    fn row(m: &[f32; 16], r: usize) -> [f32; 3] {
        [m[r * 4], m[r * 4 + 1], m[r * 4 + 2]]
    }
    fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    #[test]
    fn zero_angle_is_identity() {
        let m = rot(&[0.0, 0.0, 1.0], 0.0, true);
        let id: [f32; 16] = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        for i in 0..16 {
            assert!((m[i] - id[i]).abs() < 5e-6, "i {i}: {}", m[i]);
        }
    }

    #[test]
    fn affine_tail_is_identity() {
        let m = rot(&[1.0, 2.0, -0.5], 0.7, false);
        assert_eq!([m[3], m[7], m[11]], [0.0, 0.0, 0.0]);
        assert_eq!([m[12], m[13], m[14], m[15]], [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn orthonormal_rotation_block() {
        let axis = [1.0f32, 2.0, -0.5];
        let m = rot(&axis, 0.73, false);
        for r in 0..3 {
            let v = row(&m, r);
            assert!((dot(v, v) - 1.0).abs() < 1e-4, "row {r} not unit");
        }
        assert!(dot(row(&m, 0), row(&m, 1)).abs() < 1e-4);
        assert!(dot(row(&m, 1), row(&m, 2)).abs() < 1e-4);
        assert!(dot(row(&m, 0), row(&m, 2)).abs() < 1e-4);
    }

    #[test]
    fn quarter_turn_about_z() {
        let m = rot(&[0.0, 0.0, 1.0], std::f32::consts::FRAC_PI_2, true);
        assert!(m[0].abs() < 5e-6 && m[5].abs() < 5e-6);
        assert!((m[10] - 1.0).abs() < 5e-6);
        assert!((m[1] - 1.0).abs() < 5e-6);
        assert!((m[4] + 1.0).abs() < 5e-6);
    }

    #[test]
    fn determinant_is_one() {
        let m = rot(&[0.3f32, -1.0, 0.8], -1.2, false);
        let det = m[0] * (m[5] * m[10] - m[6] * m[9]) - m[1] * (m[4] * m[10] - m[6] * m[8])
            + m[2] * (m[4] * m[9] - m[5] * m[8]);
        assert!((det - 1.0).abs() < 1e-4, "det {det}");
    }

    #[test]
    fn all_three_entry_points_place_the_same_elements() {
        // The three originals' floating-point instruction streams are identical
        // once the destination offsets are masked off, so the nine rotation
        // elements have to agree to the bit and only the layout may differ. Both
        // wider entry points take `skip_normalize`, the 3x3 one takes its inverse.
        for (axis, angle) in crate::math::matrix33::axis_angle_sweep() {
            let square = rot(&axis, angle, false);
            let rows = super::c44_matrix__set_rotation_axis_angle__7bb860(&axis, angle, false);
            let three = crate::math::matrix33::c33_matrix__from_axis_angle__7be490(
                &axis, angle, true, // normalize: the same act as skip_normalize = false
            );
            for r in 0..3 {
                for c in 0..3 {
                    let want = three[r * 3 + c].to_bits();
                    assert_eq!(square[r * 4 + c].to_bits(), want, "4x4 element {r},{c}");
                    assert_eq!(rows[r * 3 + c].to_bits(), want, "3x4 element {r},{c}");
                }
            }
            assert_eq!([rows[9], rows[10], rows[11]], [0.0, 0.0, 0.0]);
            assert_eq!([square[3], square[7], square[11]], [0.0, 0.0, 0.0]);
            assert_eq!(
                [square[12], square[13], square[14], square[15]],
                [0.0, 0.0, 0.0, 1.0]
            );
        }
    }
}

/// The angle a quarter turn arrives with: `PI * 0.5` in `f32`, which is exact.
///
/// Pinned as bits so the recogniser below is a comparison against a constant
/// rather than a call into [`crate::math::m2::half_pi_spin__718960`], which is
/// where the value comes from; a test holds the two together.
const Z_QUARTER_TURN_ANGLE_BITS: u32 = 0x3fc9_0fdb;

/// [`c44_matrix__set_rotation_axis_angle__7bdb00`] for a quarter turn about `+Z`.
///
/// Written out rather than rebuilt because the M2 particle walk asks for this
/// one rotation on every spline emitter of every node of every frame.
///
/// **These are this reimplementation's bits, not the client's.** `0x7bdb00`
/// builds its rotation from the shared polynomial `sin_cos` where the original
/// runs `FSINCOS`, and the two differ in the last place — the entry's diff
/// annotation carries the bound. Pinning what the kernel returns is what keeps
/// the walk's behaviour unchanged; the test beside this holds the two together
/// if the trig ever moves.
///
/// Three lanes are not what a textbook quarter turn writes, and none of them may
/// be rounded off in a cleanup. The two diagonal cosines are `-4.371139e-8`
/// rather than zero, and `[10]` is one ulp below `1.0`: the residual is under
/// the half-ulp at one, so `1.0f32 - cos` rounds back to exactly `1.0`, and the
/// diagonal `z * z * t + c` then lands at `1.0 - 4.371139e-8`.
const Z_QUARTER_TURN: [f32; 16] = [
    f32::from_bits(0xb33b_bd2e), // cos
    f32::from_bits(0x3f80_0000), // sin
    f32::from_bits(0x0000_0000),
    f32::from_bits(0x0000_0000),
    f32::from_bits(0xbf80_0000), // -sin
    f32::from_bits(0xb33b_bd2e), // cos
    f32::from_bits(0x0000_0000),
    f32::from_bits(0x0000_0000),
    f32::from_bits(0x0000_0000),
    f32::from_bits(0x0000_0000),
    f32::from_bits(0x3f7f_ffff), // z diagonal, one ulp below 1.0
    f32::from_bits(0x0000_0000),
    f32::from_bits(0x0000_0000),
    f32::from_bits(0x0000_0000),
    f32::from_bits(0x0000_0000),
    f32::from_bits(0x3f80_0000),
];

/// Whether the axis and angle are the exact pair the emitter spin passes.
///
/// Bit tests, not value compares. `-0.0 == 0.0` holds, and a negative zero in
/// the axis reaches the signs of the products the general body forms, so a
/// value compare would answer for an input the constant was not built from.
/// `skip_normalize` is deliberately not part of the test: normalizing an exact
/// unit `+Z` scales by `1.0 / sqrt(1.0)` and returns every component unchanged,
/// so both flag values build the same matrix — pinned by a test rather than
/// left as an argument.
pub const fn is_z_quarter_turn(axis: &[f32; 3], angle: f32) -> bool {
    axis[0].to_bits() == 0
        && axis[1].to_bits() == 0
        && axis[2].to_bits() == 0x3f80_0000
        && angle.to_bits() == Z_QUARTER_TURN_ANGLE_BITS
}

/// `C44Matrix::RotateAxisAngle` — pre-multiplies a 4x4 by an axis-angle rotation.
///
/// `out = R(axis, angle) * m`. The original at `0x7bdd60` is a shell over two
/// functions reimplemented here: it builds `R` into a stack local through
/// `0x7bdb00`, calls `0x7bc6a0` with `(R, this)` — the receiver is pushed
/// before the build, so it is still the multiply's second operand — and copies
/// all sixteen floats of the product back over the receiver. Composing at
/// kernel level replaces two detour crossings with none. It also bypasses the
/// non-finite probe the multiply adapter carries, which is the trade
/// [`c44_matrix__rotate_quaternion__7bddb0`] already makes for the same reason.
///
/// A quarter turn about `+Z` is answered from [`Z_QUARTER_TURN`]
/// instead of rebuilt. That is not a general memo: it is the one argument pair
/// the M2 particle walk passes, and it passes it per spline emitter per node
/// per frame.
pub fn c44_matrix__rotate_axis_angle__7bdd60(
    m: &[f32; 16],
    axis: &[f32; 3],
    angle: f32,
    skip_normalize: bool,
) -> [f32; 16] {
    let r = if is_z_quarter_turn(axis, angle) {
        Z_QUARTER_TURN
    } else {
        c44_matrix__set_rotation_axis_angle__7bdb00(axis, angle, skip_normalize)
    };
    c44_matrix__multiply__7bc6a0(&r, m)
}

#[cfg(test)]
mod tests_c44_matrix__rotate_axis_angle__7bdd60 {
    use super::{
        Z_QUARTER_TURN, Z_QUARTER_TURN_ANGLE_BITS, c44_matrix__rotate_axis_angle__7bdd60 as rotate,
        c44_matrix__set_rotation_axis_angle__7bdb00 as build,
    };

    #[test]
    fn the_constant_is_what_the_builder_returns() {
        // Both flag values, because the recogniser ignores `skip_normalize`:
        // normalizing an exact unit axis has to leave every component alone.
        for skip in [false, true] {
            let want = build(
                &[0.0, 0.0, 1.0],
                f32::from_bits(Z_QUARTER_TURN_ANGLE_BITS),
                skip,
            );
            for i in 0..16 {
                assert_eq!(
                    Z_QUARTER_TURN[i].to_bits(),
                    want[i].to_bits(),
                    "element {i}, skip_normalize {skip}"
                );
            }
        }
    }

    #[test]
    fn the_constant_is_not_the_textbook_quarter_turn() {
        // Three lanes a cleanup would "fix". Asserted separately from the
        // builder comparison so a failure says which reading moved.
        assert_eq!(Z_QUARTER_TURN[0].to_bits(), 0xb33b_bd2e);
        assert_eq!(Z_QUARTER_TURN[5].to_bits(), 0xb33b_bd2e);
        assert_eq!(Z_QUARTER_TURN[10].to_bits(), 0x3f7f_ffff);
    }

    #[test]
    fn the_angle_is_the_one_the_walk_passes() {
        assert_eq!(
            crate::math::m2::half_pi_spin__718960().to_bits(),
            Z_QUARTER_TURN_ANGLE_BITS
        );
    }

    /// A receiver whose entries all carry a full non-terminating mantissa.
    ///
    /// Given as bit patterns for the reason the multiply's own fixtures are: a
    /// rounded decimal literal is a different float and would stop separating
    /// the summation the product runs.
    fn receiver() -> [f32; 16] {
        let mut w = 0x3f8c_0d31_u32;
        let mut out = [0.0f32; 16];
        for cell in &mut out {
            w = w.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *cell = f32::from_bits((w & 0x807f_ffff) | 0x3fa0_0000);
        }
        out
    }

    #[test]
    fn the_product_is_rotation_times_receiver_not_the_other_way_round() {
        // Both orders compile and both look plausible, so the operand order is
        // the way a future edit breaks this silently. The original pushes its
        // receiver before building the rotation, leaving it as the multiply's
        // second operand.
        let m = receiver();
        let r = build(&[0.3, -1.0, 0.8], -1.2, false);
        let got = rotate(&m, &[0.3, -1.0, 0.8], -1.2, false);
        let forward = super::c44_matrix__multiply__7bc6a0(&r, &m);
        let reversed = super::c44_matrix__multiply__7bc6a0(&m, &r);
        for i in 0..16 {
            assert_eq!(got[i].to_bits(), forward[i].to_bits(), "element {i}");
        }
        assert!((0..16).any(|i| forward[i].to_bits() != reversed[i].to_bits()));
    }

    #[test]
    fn rotate_matches_the_shell_it_replaces() {
        let m = receiver();
        let constant = (
            [0.0f32, 0.0, 1.0],
            f32::from_bits(Z_QUARTER_TURN_ANGLE_BITS),
        );
        for (axis, angle) in crate::math::matrix33::axis_angle_sweep()
            .into_iter()
            .chain(core::iter::once(constant))
        {
            for skip in [false, true] {
                let want = super::c44_matrix__multiply__7bc6a0(&build(&axis, angle, skip), &m);
                let got = rotate(&m, &axis, angle, skip);
                for i in 0..16 {
                    assert_eq!(got[i].to_bits(), want[i].to_bits(), "element {i}");
                }
            }
        }
    }
}

/// `C44Matrix::TransformPoint` (column-major variant).
///
/// Transforms a 3D point by a 4×4 matrix with implied `w = 1`, applying the
/// translation row at indices 12/13/14.
/// `out[j] = in.x·mat[j] + in.y·mat[4+j] + in.z·mat[8+j] + mat[12+j]`.
///
/// The summation order is not uniform across the three components, and nothing
/// about the algebra says it should be — it is what the original emitted. `out.x`
/// runs `z, y, x` then the translation (`0x7bcab9`); `out.y` and `out.z` run
/// `z, x, y` then the translation (`0x7bcaa1`, `0x7bca87`). Left to right over
/// `x, y, z` is a different number in all three.
///
/// Nothing is stored until the three trailing `FSTP m32` at `0x7bcad1`, so each
/// component carries an `f64` significand through its three multiplies, two adds
/// and the translation add, and narrows once.
///
/// A caller holding one matrix over a run of points widens it once and calls
/// [`c44_matrix__transform_point__7bca80_pre`] instead.
pub fn c44_matrix__transform_point__7bca80(input: &[f32; 3], mat: &[f32; 16]) -> [f32; 3] {
    c44_matrix__transform_point__7bca80_pre(input, &mat.map(f64::from))
}

/// `C44Matrix::TransformPoint` over a matrix already widened to `f64`.
///
/// The body of [`c44_matrix__transform_point__7bca80`] with the twelve
/// `f64::from(mat[i])` widenings lifted out of it. `f32 → f64` is exact, so a
/// caller transforming a run of points against one matrix widens once and gets
/// the same operands, the same per-component order and the same single
/// narrowing. Elements 3, 7, 11 and 15 are never read.
pub fn c44_matrix__transform_point__7bca80_pre(input: &[f32; 3], mat: &[f64; 16]) -> [f32; 3] {
    let (ix, iy, iz) = (
        f64::from(input[0]),
        f64::from(input[1]),
        f64::from(input[2]),
    );
    let elem = |i: usize| mat[i];

    let x = ((iz * elem(8) + iy * elem(4)) + ix * elem(0)) + elem(12);
    let y = ((iz * elem(9) + ix * elem(1)) + iy * elem(5)) + elem(13);
    let z = ((iz * elem(10) + ix * elem(2)) + iy * elem(6)) + elem(14);

    [
        super::f64_to_f32(x),
        super::f64_to_f32(y),
        super::f64_to_f32(z),
    ]
}

#[cfg(test)]
mod tests_c44_matrix__transform_point__7bca80 {
    use super::c44_matrix__transform_point__7bca80 as xform;

    #[test]
    fn per_component_order_and_single_narrowing() {
        // Separates the stock shape from the left-to-right per-step f32 form on
        // all three components at once. Bit patterns, not decimals.
        let v = [
            f32::from_bits(0xc1b4_400e),
            f32::from_bits(0x41bd_d201),
            f32::from_bits(0x4193_156c),
        ];
        let m = [
            f32::from_bits(0xbfeb_2709),
            f32::from_bits(0x3f58_6664),
            f32::from_bits(0xbfd7_aad3),
            f32::from_bits(0xc030_18e5),
            f32::from_bits(0xbf7b_e37a),
            f32::from_bits(0xbf7e_7312),
            f32::from_bits(0xbf1f_7d2c),
            f32::from_bits(0x3f61_6dce),
            f32::from_bits(0x4001_172c),
            f32::from_bits(0x4031_25f2),
            f32::from_bits(0x4036_f30d),
            f32::from_bits(0x403d_819e),
            f32::from_bits(0x3fde_9a5a),
            f32::from_bits(0xbf94_c063),
            f32::from_bits(0xbff8_9a61),
            f32::from_bits(0x3f3e_1530),
        ];
        let got = xform(&v, &m);

        // `out.x` runs z,y,x; `out.y` and `out.z` run z,x,y. All narrow once.
        let (ix, iy, iz) = (f64::from(v[0]), f64::from(v[1]), f64::from(v[2]));
        let elem = |i: usize| f64::from(m[i]);
        let want = [
            super::super::f64_to_f32(((iz * elem(8) + iy * elem(4)) + ix * elem(0)) + elem(12)),
            super::super::f64_to_f32(((iz * elem(9) + ix * elem(1)) + iy * elem(5)) + elem(13)),
            super::super::f64_to_f32(((iz * elem(10) + ix * elem(2)) + iy * elem(6)) + elem(14)),
        ];
        for i in 0..3 {
            assert_eq!(got[i].to_bits(), want[i].to_bits(), "component {i}");
        }

        let mut per_step = [0.0f32; 3];
        for (i, slot) in per_step.iter_mut().enumerate() {
            let mut acc = v[0] * m[i];
            acc += v[1] * m[4 + i];
            acc += v[2] * m[8 + i];
            acc += m[12 + i];
            *slot = acc;
        }
        for i in 0..3 {
            assert_ne!(
                got[i].to_bits(),
                per_step[i].to_bits(),
                "fixture no longer separates component {i}"
            );
        }
    }

    #[test]
    fn vertex_in_place_is_the_same_arithmetic() {
        // The two originals are the same instruction sequence over the same
        // operands, differing only in where the results are stored.
        let v = [
            f32::from_bits(0xc1b4_400e),
            f32::from_bits(0x41bd_d201),
            f32::from_bits(0x4193_156c),
        ];
        let mut m = [0.0f32; 16];
        let mut w = 0x3fc0_0001_u32;
        for slot in &mut m {
            *slot = f32::from_bits(w);
            w += 7919;
        }
        assert_eq!(
            xform(&v, &m),
            super::c44_matrix__transform_vertex_in_place__7bcc60(&v, &m)
        );
    }

    const ID: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ];

    #[test]
    fn identity_is_neutral() {
        let p = [3.0f32, -2.0, 7.5];
        assert_eq!(xform(&p, &ID), p);
    }

    #[test]
    fn translation_in_last_row() {
        let mut m = ID;
        m[12] = 10.0;
        m[13] = 20.0;
        m[14] = 30.0;
        let p = [1.0f32, 2.0, 3.0];
        assert_eq!(xform(&p, &m), [11.0, 22.0, 33.0]);
    }

    #[test]
    fn diagonal_scale_plus_translate() {
        let m: [f32; 16] = [
            2.0, 0.0, 0.0, 0.0, //
            0.0, 3.0, 0.0, 0.0, //
            0.0, 0.0, 4.0, 0.0, //
            1.0, 1.0, 1.0, 1.0,
        ];
        let p = [1.0f32, 1.0, 1.0];
        assert_eq!(xform(&p, &m), [3.0, 4.0, 5.0]);
    }

    #[test]
    fn affine_linearity() {
        // T(a+b) - T(0) == (T(a)-T(0)) + (T(b)-T(0)).
        let m: [f32; 16] = [
            0.5, 1.0, -0.5, 0.0, //
            2.0, 0.0, 1.0, 0.0, //
            -1.0, 0.5, 0.25, 0.0, //
            4.0, -3.0, 2.0, 1.0,
        ];
        let a = [1.0f32, 2.0, 3.0];
        let b = [-2.0f32, 0.5, 1.0];
        let ab = [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
        let t0 = xform(&[0.0, 0.0, 0.0], &m);
        let ta = xform(&a, &m);
        let tb = xform(&b, &m);
        let tab = xform(&ab, &m);
        for i in 0..3 {
            let lhs = tab[i] - t0[i];
            let rhs = (ta[i] - t0[i]) + (tb[i] - t0[i]);
            assert!((lhs - rhs).abs() < 1e-4, "i {i}: {lhs} vs {rhs}");
        }
    }
}

/// `C44Matrix::TransformPoint` (row-major variant).
///
/// Transforms a 3D point by a 4×4 matrix with implied `w = 1`, reading the
/// translation from the last column (indices 3/7/11). `out[i] = in.x·mat[4i] +
/// in.y·mat[4i+1] + in.z·mat[4i+2] + mat[4i+3]`.
pub fn c44_matrix__transform_point__7bcae0(mat: &[f32; 16], input: &[f32; 3]) -> [f32; 3] {
    let ix = input[0];
    let iy = input[1];
    let iz = input[2];

    let mut x = mat[0] * ix;
    x += mat[1] * iy;
    x += mat[2] * iz;
    x += mat[3];

    let mut y = mat[4] * ix;
    y += mat[5] * iy;
    y += mat[6] * iz;
    y += mat[7];

    let mut z = mat[8] * ix;
    z += mat[9] * iy;
    z += mat[10] * iz;
    z += mat[11];

    [x, y, z]
}

#[cfg(test)]
mod tests_c44_matrix__transform_point__7bcae0 {
    use super::c44_matrix__transform_point__7bcae0 as xform;

    const ID: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ];

    #[test]
    fn identity_is_neutral() {
        let p = [3.0f32, -2.0, 7.5];
        assert_eq!(xform(&ID, &p), p);
    }

    #[test]
    fn translation_in_last_column() {
        // row-major: translation lives at indices 3,7,11.
        let mut m = ID;
        m[3] = 10.0;
        m[7] = 20.0;
        m[11] = 30.0;
        let p = [1.0f32, 2.0, 3.0];
        assert_eq!(xform(&m, &p), [11.0, 22.0, 33.0]);
    }

    #[test]
    fn diagonal_scale_plus_translate() {
        let m: [f32; 16] = [
            2.0, 0.0, 0.0, 1.0, //
            0.0, 3.0, 0.0, 1.0, //
            0.0, 0.0, 4.0, 1.0, //
            0.0, 0.0, 0.0, 1.0,
        ];
        let p = [1.0f32, 1.0, 1.0];
        assert_eq!(xform(&m, &p), [3.0, 4.0, 5.0]);
    }

    #[test]
    fn is_transpose_of_column_major_variant() {
        // Row-major M * p == column-major M^T * p; check against an explicit ref.
        let row: [f32; 16] = [
            1.0, 2.0, 3.0, 0.5, //
            4.0, 5.0, 6.0, -1.0, //
            7.0, 8.0, 9.0, 2.0, //
            0.0, 0.0, 0.0, 1.0,
        ];
        let p = [0.7f32, -1.3, 2.1];
        let got = xform(&row, &p);
        let want = [
            1.0 * p[0] + 2.0 * p[1] + 3.0 * p[2] + 0.5,
            4.0 * p[0] + 5.0 * p[1] + 6.0 * p[2] - 1.0,
            7.0 * p[0] + 8.0 * p[1] + 9.0 * p[2] + 2.0,
        ];
        for i in 0..3 {
            assert!((got[i] - want[i]).abs() < 1e-4, "i {i}");
        }
    }
}

/// `C44Matrix::TransformVector4`.
///
/// Transforms a homogeneous 4D vector by a 4×4 matrix, computing the full `w`
/// component. `out[j] = in.x·mat[j] + in.y·mat[4+j] + in.z·mat[8+j] +
/// in.w·mat[12+j]` for `j = 0..3` (column-major).
///
/// As in [`c44_matrix__transform_point__7bca80`] the order is not uniform.
/// Three components run `w, x, z, y`; `out.x` alone runs `w, z, y, x`
/// (`0x7bcba0`, and it is also the one that loads the vector lane before the
/// matrix lane on its last product). Nothing is stored until the four trailing
/// `FSTP m32` at `0x7bcbbd`, so every component narrows exactly once.
pub fn c44_matrix__transform_vector4__7bcb40(input: &[f32; 4], mat: &[f32; 16]) -> [f32; 4] {
    let (ix, iy, iz, iw) = (
        f64::from(input[0]),
        f64::from(input[1]),
        f64::from(input[2]),
        f64::from(input[3]),
    );
    let elem = |i: usize| f64::from(mat[i]);

    let x = ((iw * elem(12) + iz * elem(8)) + iy * elem(4)) + ix * elem(0);
    let y = ((iw * elem(13) + ix * elem(1)) + iz * elem(9)) + iy * elem(5);
    let z = ((iw * elem(14) + ix * elem(2)) + iz * elem(10)) + iy * elem(6);
    let w = ((iw * elem(15) + ix * elem(3)) + iz * elem(11)) + iy * elem(7);

    [
        super::f64_to_f32(x),
        super::f64_to_f32(y),
        super::f64_to_f32(z),
        super::f64_to_f32(w),
    ]
}

#[cfg(test)]
mod tests_c44_matrix__transform_vector4__7bcb40 {
    use super::c44_matrix__transform_vector4__7bcb40 as xform;

    #[test]
    fn per_component_order_and_single_narrowing() {
        // Three components sum w,x,z,y; `out.x` alone sums w,z,y,x. Every one
        // narrows only at its trailing store. The fixture has to separate that
        // from the left-to-right per-step f32 form on all four lanes.
        let mut v = [0.0f32; 4];
        let mut m = [0.0f32; 16];
        let mut w = 0x4021_3c0f_u32;
        for slot in v.iter_mut().chain(m.iter_mut()) {
            *slot = f32::from_bits(w);
            w = w.wrapping_add(0x0002_4c9d) ^ 0x0000_0b1d;
        }
        let got = xform(&v, &m);

        let (ix, iy, iz, iw) = (
            f64::from(v[0]),
            f64::from(v[1]),
            f64::from(v[2]),
            f64::from(v[3]),
        );
        let elem = |i: usize| f64::from(m[i]);
        let want = [
            super::super::f64_to_f32(
                ((iw * elem(12) + iz * elem(8)) + iy * elem(4)) + ix * elem(0),
            ),
            super::super::f64_to_f32(
                ((iw * elem(13) + ix * elem(1)) + iz * elem(9)) + iy * elem(5),
            ),
            super::super::f64_to_f32(
                ((iw * elem(14) + ix * elem(2)) + iz * elem(10)) + iy * elem(6),
            ),
            super::super::f64_to_f32(
                ((iw * elem(15) + ix * elem(3)) + iz * elem(11)) + iy * elem(7),
            ),
        ];
        for i in 0..4 {
            assert_eq!(got[i].to_bits(), want[i].to_bits(), "component {i}");
        }

        let mut per_step = [0.0f32; 4];
        for (i, slot) in per_step.iter_mut().enumerate() {
            let mut acc = v[0] * m[i];
            acc += v[1] * m[4 + i];
            acc += v[2] * m[8 + i];
            acc += v[3] * m[12 + i];
            *slot = acc;
        }
        assert!(
            (0..4).any(|i| got[i].to_bits() != per_step[i].to_bits()),
            "fixture no longer separates the stock shape from the per-step form"
        );
    }

    const ID: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ];

    #[test]
    fn identity_is_neutral() {
        let v = [3.0f32, -2.0, 7.5, 1.0];
        assert_eq!(xform(&v, &ID), v);
    }

    #[test]
    fn w_one_matches_point_transform() {
        // With w=1 the 4D transform reproduces the affine point transform.
        let m: [f32; 16] = [
            2.0, 0.0, 0.0, 0.0, //
            0.0, 3.0, 0.0, 0.0, //
            0.0, 0.0, 4.0, 0.0, //
            1.0, 1.0, 1.0, 1.0,
        ];
        let v = [1.0f32, 1.0, 1.0, 1.0];
        assert_eq!(xform(&v, &m), [3.0, 4.0, 5.0, 1.0]);
    }

    #[test]
    fn full_w_is_computed() {
        // last matrix column drives w; pick a non-trivial bottom column.
        let mut m = ID;
        m[3] = 0.5;
        m[7] = 0.25;
        m[11] = -0.5;
        let v = [2.0f32, 4.0, 8.0, 1.0];
        let r = xform(&v, &m);
        let want_w = 0.5 * 2.0 + 0.25 * 4.0 + (-0.5) * 8.0 + 1.0 * 1.0;
        assert!((r[3] - want_w).abs() < 1e-5, "w {} vs {}", r[3], want_w);
    }

    #[test]
    fn linearity() {
        // full 4D transform is linear: T(a+b) == T(a) + T(b).
        let m: [f32; 16] = [
            0.5, 1.0, -0.5, 0.3, //
            2.0, 0.0, 1.0, -0.2, //
            -1.0, 0.5, 0.25, 0.1, //
            4.0, -3.0, 2.0, 1.0,
        ];
        let a = [1.0f32, 2.0, 3.0, 1.0];
        let b = [-2.0f32, 0.5, 1.0, 0.5];
        let ab = [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]];
        let ta = xform(&a, &m);
        let tb = xform(&b, &m);
        let tab = xform(&ab, &m);
        for i in 0..4 {
            assert!((tab[i] - (ta[i] + tb[i])).abs() < 1e-4, "i {i}");
        }
    }
}

/// `C44Matrix::TransformVertexInPlace`.
///
/// Transforms a 3D vertex by a 4×4 matrix (column-major, implied `w = 1`,
/// translation at indices 12/13/14) — identical math to the column-major point
/// transform. The original writes the result back into the input vertex and
/// copies it to `out`; this returns the value so the adapter performs both
/// writes.
///
/// "Identical math" is exact rather than approximate here: `0x7bcc67`..`0x7bccad`
/// is the same instruction sequence over the same operands as `0x7bca87`
/// onwards, down to the per-component summation order, and the two differ only
/// in which pointer the results are stored through. So this defers to the point
/// transform rather than carrying a second copy that could drift away from it.
pub fn c44_matrix__transform_vertex_in_place__7bcc60(vec: &[f32; 3], mat: &[f32; 16]) -> [f32; 3] {
    c44_matrix__transform_point__7bca80(vec, mat)
}

#[cfg(test)]
mod tests_c44_matrix__transform_vertex_in_place__7bcc60 {
    use super::c44_matrix__transform_vertex_in_place__7bcc60 as xform;

    const ID: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ];

    #[test]
    fn identity_is_neutral() {
        let p = [3.0f32, -2.0, 7.5];
        assert_eq!(xform(&p, &ID), p);
    }

    #[test]
    fn diagonal_scale_plus_translate() {
        // Same column-major math as the 7bca80 point transform.
        let m: [f32; 16] = [
            2.0, 0.0, 0.0, 0.0, //
            0.0, 3.0, 0.0, 0.0, //
            0.0, 0.0, 4.0, 0.0, //
            1.0, 1.0, 1.0, 1.0,
        ];
        let p = [1.0f32, 1.0, 1.0];
        assert_eq!(xform(&p, &m), [3.0, 4.0, 5.0]);
    }

    #[test]
    fn translation_in_last_row() {
        let mut m = ID;
        m[12] = -5.0;
        m[13] = 6.0;
        m[14] = -7.0;
        let p = [1.0f32, 2.0, 3.0];
        assert_eq!(xform(&p, &m), [-4.0, 8.0, -4.0]);
    }

    #[test]
    fn affine_linearity() {
        let m: [f32; 16] = [
            0.5, 1.0, -0.5, 0.0, //
            2.0, 0.0, 1.0, 0.0, //
            -1.0, 0.5, 0.25, 0.0, //
            4.0, -3.0, 2.0, 1.0,
        ];
        let a = [1.0f32, 2.0, 3.0];
        let b = [-2.0f32, 0.5, 1.0];
        let ab = [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
        let t0 = xform(&[0.0, 0.0, 0.0], &m);
        let ta = xform(&a, &m);
        let tb = xform(&b, &m);
        let tab = xform(&ab, &m);
        for i in 0..3 {
            let lhs = tab[i] - t0[i];
            let rhs = (ta[i] - t0[i]) + (tb[i] - t0[i]);
            assert!((lhs - rhs).abs() < 1e-4, "i {i}: {lhs} vs {rhs}");
        }
    }
}

/// In-place post-translate of a 4x4 matrix.
///
/// Adds `rot3x3 * trans` into the matrix's translation column (indices 12,13,14).
///
/// The 4x4 has a 4-float row stride: the rotation columns scaling `trans.{x,y,z}`
/// are read from indices {0,4,8}, {1,5,9}, {2,6,10}; each translation component
/// `mat[12+i]` gains `mat[i]*tx + mat[4+i]*ty + mat[8+i]*tz`.
///
/// All three rows sum `z, y, x` and fold the existing translation in last
/// (`0x7bdc46` onwards). Unlike the point transform the order *is* uniform here.
/// Each row's three products, two adds and the translation add stay on the x87
/// stack until that row's `FSTP m32` (`0x7bdc5d`, `0x7bdc78`, `0x7bdc93`), so a
/// row narrows once. Note the rows are written one at a time and each reads only
/// its own column, so the in-place store cannot feed a later row.
pub fn c44_matrix__translate__7bdc40(mat: &mut [f32; 16], trans: &[f32; 3]) {
    let (tx, ty, tz) = (
        f64::from(trans[0]),
        f64::from(trans[1]),
        f64::from(trans[2]),
    );
    let elem = |i: usize| f64::from(mat[i]);

    let r0 = ((elem(8) * tz + elem(4) * ty) + elem(0) * tx) + elem(12);
    let r1 = ((elem(9) * tz + elem(5) * ty) + elem(1) * tx) + elem(13);
    let r2 = ((elem(10) * tz + elem(6) * ty) + elem(2) * tx) + elem(14);

    mat[12] = super::f64_to_f32(r0);
    mat[13] = super::f64_to_f32(r1);
    mat[14] = super::f64_to_f32(r2);
}

#[cfg(test)]
mod tests_c44_matrix__translate__7bdc40 {
    use super::c44_matrix__translate__7bdc40 as translate;

    #[test]
    fn rows_sum_z_then_y_then_x_and_narrow_once() {
        let t = [
            f32::from_bits(0xc200_5ce7),
            f32::from_bits(0x421a_8f04),
            f32::from_bits(0xc1f3_a2bc),
        ];
        let m = [
            f32::from_bits(0xbfea_ecf7),
            f32::from_bits(0xbf7d_1ccb),
            f32::from_bits(0x3fa3_efe5),
            f32::from_bits(0x4007_f85d),
            f32::from_bits(0xbe15_61d2),
            f32::from_bits(0xbc36_1883),
            f32::from_bits(0x3fdd_8511),
            f32::from_bits(0xbfaa_7eac),
            f32::from_bits(0xbf43_fe30),
            f32::from_bits(0x3f2d_23d4),
            f32::from_bits(0x3f51_3dc1),
            f32::from_bits(0xc031_60f8),
            f32::from_bits(0xc014_fbff),
            f32::from_bits(0xc00b_08d5),
            f32::from_bits(0x3fac_e606),
            f32::from_bits(0x3f94_29b1),
        ];

        let mut got = m;
        translate(&mut got, &t);

        let (tx, ty, tz) = (f64::from(t[0]), f64::from(t[1]), f64::from(t[2]));
        let elem = |i: usize| f64::from(m[i]);
        for i in 0..3 {
            let want = super::super::f64_to_f32(
                ((elem(8 + i) * tz + elem(4 + i) * ty) + elem(i) * tx) + elem(12 + i),
            );
            assert_eq!(got[12 + i].to_bits(), want.to_bits(), "row {i}");

            let per_step = m[12 + i] + (m[i] * t[0] + m[4 + i] * t[1] + m[8 + i] * t[2]);
            assert_ne!(
                got[12 + i].to_bits(),
                per_step.to_bits(),
                "fixture no longer separates row {i}"
            );
        }

        // The rest of the matrix is untouched.
        for i in 0..12 {
            assert_eq!(got[i].to_bits(), m[i].to_bits(), "element {i} moved");
        }
    }

    fn id_with_t(t: [f32; 3]) -> [f32; 16] {
        [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, t[0], t[1], t[2], 1.0,
        ]
    }

    #[test]
    fn identity_adds_point() {
        let mut m = id_with_t([5.0, 6.0, 7.0]);
        translate(&mut m, &[1.0, 2.0, 3.0]);
        assert_eq!([m[12], m[13], m[14]], [6.0, 8.0, 10.0]);
    }

    #[test]
    fn zero_trans_no_change() {
        let mut m = id_with_t([5.0, 6.0, 7.0]);
        let before = m;
        translate(&mut m, &[0.0, 0.0, 0.0]);
        assert_eq!(m, before);
    }

    #[test]
    fn rotation_block_untouched() {
        let mut m = id_with_t([0.0, 0.0, 0.0]);
        m[1] = 0.5;
        let rb: [f32; 11] = [
            m[0], m[1], m[2], m[4], m[5], m[6], m[8], m[9], m[10], m[15], m[3],
        ];
        translate(&mut m, &[9.0, 8.0, 7.0]);
        let ra: [f32; 11] = [
            m[0], m[1], m[2], m[4], m[5], m[6], m[8], m[9], m[10], m[15], m[3],
        ];
        assert_eq!(rb, ra);
    }

    #[test]
    fn diagonal_scale_applies() {
        let mut m = [
            2.0f32, 0.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 0.0, 4.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        translate(&mut m, &[1.0, 1.0, 1.0]);
        assert_eq!([m[12], m[13], m[14]], [2.0, 3.0, 4.0]);
    }
}

/// Builds a box-projection light-space matrix pair from an axis-aligned box.
///
/// Returns `(primary, secondary)` row-major 4x4 matrices, or `None` when the box
/// is degenerate (width or height below `eps`). The box origin (a live world
/// global in the host image) is injected as `origin`; the optional `post_mul`
/// 4x4, the `arg4` element value, the `centered` selector, and the source float
/// constants (`eps`, `one`, `half`, `rot_angle`) are likewise injected so the
/// kernel stays pure and host-testable. `aabb` is `min.{x,y,z}` then `max.{x,y,z}`.
// The nine parameters are the original's own arguments plus the image globals and
// source float constants it read, lifted out so the kernel stays pure and testable;
// bundling them into a struct would hide which values come from the image. The extent
// guard is written `!(a >= b)` so a NaN width or height rejects, which the `a < b`
// clippy suggests would not.
#[allow(clippy::neg_cmp_op_on_partial_ord, clippy::too_many_arguments)]
pub fn build_box_projection_matrix__6d6fa0(
    origin: &[f32; 3],
    aabb: &[f32; 6],
    post_mul: Option<&[f32; 16]>,
    arg4: f32,
    centered: bool,
    eps: f32,
    one: f32,
    half: f32,
    rot_angle: f32,
) -> Option<([f32; 16], [f32; 16])> {
    let min = [aabb[0], aabb[1], aabb[2]];
    let max = [aabb[3], aabb[4], aabb[5]];

    // center = 0.5 * (min + max); translation is origin-relative when centered,
    // otherwise the plain box-corner negation.
    let center = [
        (min[0] + max[0]) * half,
        (min[1] + max[1]) * half,
        (min[2] + max[2]) * half,
    ];
    let trans = if centered {
        [
            origin[0] - center[0],
            origin[1] - center[1],
            origin[2] - center[2],
        ]
    } else {
        [-center[0], -center[1], -center[2]]
    };

    // Identity post-translated by `trans` (the shared pre-bias basis).
    let mut basis = [
        one, 0.0, 0.0, 0.0, //
        0.0, one, 0.0, 0.0, //
        0.0, 0.0, one, 0.0, //
        0.0, 0.0, 0.0, one,
    ];
    c44_matrix__translate__7bdc40(&mut basis, &trans);

    let width = max[0] - min[0];
    let height = max[1] - min[1];
    let depth = max[2] - min[2];

    // Degenerate-extent guard: proceed only when width >= eps && height >= eps.
    // Written as `!(a >= b)` so NaN extents reject (matching the x87 `FCOMP`
    // followed by `TEST AH,0x5; JNP`, which is a strict-less-than reject).
    if !(width >= eps) || !(height >= eps) {
        return None;
    }

    // diag(1/width, 1/height, 1, 1)
    let scale = [
        one / width,
        0.0,
        0.0,
        0.0, //
        0.0,
        one / height,
        0.0,
        0.0, //
        0.0,
        0.0,
        one,
        0.0, //
        0.0,
        0.0,
        0.0,
        one,
    ];
    let rot = c44_matrix__set_rotation_axis_angle__7bdb00(&[0.0, 0.0, one], rot_angle, true);

    // primary = (basis * scale) * rot, optional post-multiply, then +half texel
    // bias on the translation x/y.
    let mut primary = mul4x4(&mul4x4(&basis, &scale), &rot);
    if let Some(pm) = post_mul {
        primary = mul4x4(&primary, pm);
    }
    primary[12] += half;
    primary[13] += half;

    let inv_depth = one / depth;
    let secondary_src = [
        0.0, 0.0, 0.0, 0.0, //
        0.0, 0.0, 0.0, 0.0, //
        inv_depth, 0.0, inv_depth, 0.0, //
        arg4, half, arg4, one,
    ];
    let secondary = mul4x4(&basis, &secondary_src);

    Some((primary, secondary))
}

#[cfg(test)]
mod tests_build_box_projection_matrix__6d6fa0 {
    use super::build_box_projection_matrix__6d6fa0 as build;

    const EPS: f32 = 0.001;
    const ONE: f32 = 1.0;
    const HALF: f32 = 0.5;
    const ANGLE: f32 = core::f32::consts::PI * -0.5;

    fn approx(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn degenerate_width_returns_none() {
        let r = build(
            &[0.0, 0.0, 0.0],
            &[1.0, 0.0, 0.0, 1.0, 2.0, 3.0],
            None,
            0.0,
            true,
            EPS,
            ONE,
            HALF,
            ANGLE,
        );
        assert!(r.is_none());
    }

    #[test]
    fn degenerate_height_returns_none() {
        let r = build(
            &[0.0, 0.0, 0.0],
            &[0.0, 5.0, 0.0, 4.0, 5.0, 3.0],
            None,
            0.0,
            true,
            EPS,
            ONE,
            HALF,
            ANGLE,
        );
        assert!(r.is_none());
    }

    #[test]
    fn nan_extent_returns_none() {
        let r = build(
            &[0.0, 0.0, 0.0],
            &[0.0, 0.0, 0.0, f32::NAN, 2.0, 3.0],
            None,
            0.0,
            true,
            EPS,
            ONE,
            HALF,
            ANGLE,
        );
        assert!(r.is_none());
    }

    #[test]
    fn valid_box_produces_pair() {
        let (primary, secondary) = build(
            &[10.0, 20.0, 30.0],
            &[0.0, 0.0, 0.0, 4.0, 2.0, 8.0],
            None,
            7.0,
            true,
            EPS,
            ONE,
            HALF,
            ANGLE,
        )
        .unwrap();
        assert!(approx(secondary[8], 1.0 / 8.0, 1e-6), "{}", secondary[8]);
        assert!(approx(secondary[10], 1.0 / 8.0, 1e-6), "{}", secondary[10]);
        assert!(secondary.iter().all(|v| v.is_finite()));
        assert!(primary.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn centered_vs_corner_translation_differs() {
        let aabb = [0.0, 0.0, 0.0, 4.0, 2.0, 8.0];
        let (c, _) = build(
            &[10.0, 20.0, 30.0],
            &aabb,
            None,
            0.0,
            true,
            EPS,
            ONE,
            HALF,
            ANGLE,
        )
        .unwrap();
        let (k, _) = build(
            &[10.0, 20.0, 30.0],
            &aabb,
            None,
            0.0,
            false,
            EPS,
            ONE,
            HALF,
            ANGLE,
        )
        .unwrap();
        assert!((0..16).any(|i| (c[i] - k[i]).abs() > 1e-4));
    }

    #[test]
    fn post_mul_identity_is_noop_modulo_bias() {
        let id: [f32; 16] = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let aabb = [0.0, 0.0, 0.0, 4.0, 2.0, 8.0];
        let (a, _) = build(
            &[1.0, 2.0, 3.0],
            &aabb,
            None,
            0.0,
            true,
            EPS,
            ONE,
            HALF,
            ANGLE,
        )
        .unwrap();
        let (b, _) = build(
            &[1.0, 2.0, 3.0],
            &aabb,
            Some(&id),
            0.0,
            true,
            EPS,
            ONE,
            HALF,
            ANGLE,
        )
        .unwrap();
        for i in 0..16 {
            assert!((a[i] - b[i]).abs() <= 1e-5, "idx {i}: {} vs {}", a[i], b[i]);
        }
    }
}

/// `C44Matrix::InvertWithScale`: inverts a rotation + uniform-scale + translation matrix.
///
/// When `|scale - k| < eps` (the matrix is effectively orthonormal, `k` ~ 1.0)
/// this is the plain transpose inverse and delegates to
/// [`c44_matrix__invert_orthonormal__7bd700`]. Otherwise the inverse rotation
/// block is the transpose of the source 3×3 scaled row-wise by `k / scale²`
/// (the inverse-transpose / normal matrix for a uniform scale), and the inverse
/// translation is `R_inv · (-t)`.
///
/// `src` and the result are 16-float row-major 4×4; the source 3×3 occupies
/// indices 0,1,2 / 4,5,6 / 8,9,10 and the translation indices 12,13,14. `k` is
/// the host reference constant and `eps` the host comparison epsilon, injected
/// by the adapter.
pub fn c44_matrix__invert_with_scale__7bd820(
    src: &[f32; 16],
    scale: f32,
    k: f32,
    eps: f32,
) -> [f32; 16] {
    // Orthonormal fast path: scale ~= k => no row scaling needed.
    if (scale - k).abs() < eps {
        return c44_matrix__invert_orthonormal__7bd700(src);
    }

    // Source 3×3 (4-float row stride).
    let m00 = src[0];
    let m01 = src[1];
    let m02 = src[2];
    let m10 = src[4];
    let m11 = src[5];
    let m12 = src[6];
    let m20 = src[8];
    let m21 = src[9];
    let m22 = src[10];

    // Transpose, then row-scale by k / scale². The reciprocal is built wide:
    // `scale * scale` is formed on the x87 stack (0x7bd8b2), divided under
    // `FDIVR` (0x7bd8ca) and only narrowed by the `FSTP m32` that passes it as
    // the row-scale argument (0x7bd938). Squaring in `f32` first rounds a
    // product that is exact at this precision, and gives a different `f`.
    let f = super::f64_to_f32(f64::from(k) / (f64::from(scale) * f64::from(scale)));

    // Each element is `f * m` narrowed at its own store (0x7bdd00 onwards).
    // A product of two `f32` needs 48 bits and is exact here, so plain `f32`
    // multiplies reproduce those nine stores exactly.
    let i00 = m00 * f;
    let i01 = m10 * f;
    let i02 = m20 * f;
    let i10 = m01 * f;
    let i11 = m11 * f;
    let i12 = m21 * f;
    let i20 = m02 * f;
    let i21 = m12 * f;
    let i22 = m22 * f;

    // As on the orthonormal path, the original does not dot the last row out.
    // It negates the source translation into `f32` slots (0x7bd940..0x7bd959)
    // and `call`s Translate at 0x7bd95f, so the row inherits that kernel's
    // `z, y, x` order and single narrowing.
    let mut out = [
        i00, i01, i02, 0.0, //
        i10, i11, i12, 0.0, //
        i20, i21, i22, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ];
    c44_matrix__translate__7bdc40(&mut out, &[-src[12], -src[13], -src[14]]);
    out
}

#[cfg(test)]
mod tests_c44_matrix__invert_with_scale__7bd820 {
    use super::c44_matrix__invert_with_scale__7bd820 as invert;

    const K: f32 = 1.0;
    const EPS: f32 = 1.0e-6;

    // Row-major 4x4 multiply (last row [.,.,.,1] convention).
    fn mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
        let mut out = [0.0f32; 16];
        for r in 0..4 {
            for c in 0..4 {
                let mut s = 0.0;
                for k in 0..4 {
                    s += a[r * 4 + k] * b[k * 4 + c];
                }
                out[r * 4 + c] = s;
            }
        }
        out
    }

    fn approx_id(m: &[f32; 16], tol: f32) {
        let id: [f32; 16] = [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ];
        for i in 0..16 {
            assert!((m[i] - id[i]).abs() < tol, "idx {i}: {} (tol {tol})", m[i]);
        }
    }

    // Row-major rotation-about-Z * uniform-scale, plus translation.
    fn rot_z_scale_trans(theta: f32, s: f32, t: [f32; 3]) -> [f32; 16] {
        let c = theta.cos();
        let sn = theta.sin();
        [
            c * s,
            sn * s,
            0.0,
            0.0, //
            -sn * s,
            c * s,
            0.0,
            0.0, //
            0.0,
            0.0,
            s,
            0.0, //
            t[0],
            t[1],
            t[2],
            1.0,
        ]
    }

    #[test]
    fn orthonormal_path_identity() {
        let id: [f32; 16] = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        assert_eq!(invert(&id, 1.0, K, EPS), id);
    }

    #[test]
    fn orthonormal_path_rot_trans_composes_to_identity() {
        let m = rot_z_scale_trans(0.6, 1.0, [3.0, -4.0, 5.0]);
        let inv = invert(&m, 1.0, K, EPS);
        approx_id(&mul(&inv, &m), 1e-4);
        approx_id(&mul(&m, &inv), 1e-4);
    }

    #[test]
    fn scaled_path_composes_to_identity() {
        let s = 2.5;
        let m = rot_z_scale_trans(0.9, s, [1.0, 2.0, -3.0]);
        let inv = invert(&m, s, K, EPS);
        approx_id(&mul(&inv, &m), 1e-3);
        approx_id(&mul(&m, &inv), 1e-3);
    }

    #[test]
    fn scaled_path_pure_scale_known_value() {
        let s = 4.0;
        let m: [f32; 16] = [
            s, 0.0, 0.0, 0.0, //
            0.0, s, 0.0, 0.0, //
            0.0, 0.0, s, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ];
        let inv = invert(&m, s, K, EPS);
        let want = 1.0 / s;
        assert!((inv[0] - want).abs() < 1e-6);
        assert!((inv[5] - want).abs() < 1e-6);
        assert!((inv[10] - want).abs() < 1e-6);
        assert_eq!([inv[12], inv[13], inv[14], inv[15]], [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn scaled_path_transposes_block() {
        let m: [f32; 16] = [
            1.0, 2.0, 3.0, 0.0, //
            4.0, 5.0, 6.0, 0.0, //
            7.0, 8.0, 9.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ];
        let f = 0.25; // k=1 / scale^2 (scale=2)
        let inv = invert(&m, 2.0, K, EPS);
        assert!((inv[0] - 1.0 * f).abs() < 1e-6);
        assert!((inv[1] - 4.0 * f).abs() < 1e-6);
        assert!((inv[2] - 7.0 * f).abs() < 1e-6);
        assert!((inv[4] - 2.0 * f).abs() < 1e-6);
        assert!((inv[5] - 5.0 * f).abs() < 1e-6);
        assert!((inv[6] - 8.0 * f).abs() < 1e-6);
        assert!((inv[8] - 3.0 * f).abs() < 1e-6);
        assert!((inv[9] - 6.0 * f).abs() < 1e-6);
        assert!((inv[10] - 9.0 * f).abs() < 1e-6);
    }

    #[test]
    fn scaled_path_translation_negates_under_inverse() {
        let s = 3.0;
        let m: [f32; 16] = [
            s, 0.0, 0.0, 0.0, //
            0.0, s, 0.0, 0.0, //
            0.0, 0.0, s, 0.0, //
            6.0, -9.0, 12.0, 1.0,
        ];
        let inv = invert(&m, s, K, EPS);
        assert!((inv[12] - (-6.0 / s)).abs() < 1e-5, "{}", inv[12]);
        assert!((inv[13] - (9.0 / s)).abs() < 1e-5, "{}", inv[13]);
        assert!((inv[14] - (-12.0 / s)).abs() < 1e-5, "{}", inv[14]);
    }

    #[test]
    fn boundary_just_inside_eps_takes_fast_path() {
        let m = rot_z_scale_trans(0.3, 1.0, [1.0, 1.0, 1.0]);
        let scale = K + EPS * 0.5;
        let got = invert(&m, scale, K, EPS);
        let want = super::c44_matrix__invert_orthonormal__7bd700(&m);
        assert_eq!(got, want);
    }

    #[test]
    fn boundary_just_outside_eps_takes_scaled_path() {
        let m: [f32; 16] = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let scale = K + EPS * 2.0;
        let got = invert(&m, scale, K, EPS);
        assert!(got[0] < 1.0 && got[0] > 0.999);
    }
}

/// Builds the row-major rotation matrix of a quaternion `[x, y, z, w]` (assumed normalized).
///
/// Zero translation row, `m[15] = 1`. Diagonals are evaluated as `(1 - b) - c`
/// with the `1 - 2xx` subterm computed once, mirroring the original's operation
/// order (which differs from the `1 - (b + c)` association used by
/// [`c44_matrix__rotate_quaternion__7bddb0`]).
pub fn d3dx_matrix_rotation_quaternion__74b6bb(quat: &[f32; 4]) -> [f32; 16] {
    let [x, y, z, w] = *quat;
    let (x2, y2, z2) = (x + x, y + y, z + z);
    let (xw, yw, zw) = (x2 * w, y2 * w, z2 * w);
    let (xx, xy, xz) = (x2 * x, y2 * x, z2 * x);
    let (yy, yz, zz) = (y2 * y, z2 * y, z2 * z);
    let dx = 1.0 - xx;
    [
        (1.0 - yy) - zz,
        xy + zw,
        xz - yw,
        0.0,
        xy - zw,
        dx - zz,
        yz + xw,
        0.0,
        xz + yw,
        yz - xw,
        dx - yy,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ]
}

#[cfg(test)]
mod tests_d3dx_matrix_rotation_quaternion__74b6bb {
    use super::d3dx_matrix_rotation_quaternion__74b6bb as rot;

    const TOL: f32 = 1e-6;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() <= TOL
    }

    /// Identity quaternion produces the identity matrix exactly.
    #[test]
    fn identity_quat() {
        let m = rot(&[0.0, 0.0, 0.0, 1.0]);
        for (i, &v) in m.iter().enumerate() {
            let want: f32 = if i % 5 == 0 { 1.0 } else { 0.0 };
            assert_eq!(v.to_bits(), want.to_bits(), "m[{i}]");
        }
    }

    /// 90 degrees about Z maps x->y and y->-x for row vectors (`v * M`).
    #[test]
    fn quarter_turn_z() {
        let s = core::f32::consts::FRAC_1_SQRT_2;
        let m = rot(&[0.0, 0.0, s, s]);
        // Row 0 = image of +X, row 1 = image of +Y.
        assert!(approx(m[0], 0.0) && approx(m[1], 1.0) && approx(m[2], 0.0));
        assert!(approx(m[4], -1.0) && approx(m[5], 0.0) && approx(m[6], 0.0));
        assert!(approx(m[8], 0.0) && approx(m[9], 0.0) && approx(m[10], 1.0));
        assert_eq!(m[15].to_bits(), 1.0_f32.to_bits());
    }

    /// Rotation rows stay orthonormal for an arbitrary unit quaternion.
    #[test]
    fn orthonormal_rows() {
        let q = {
            let raw = [0.3_f32, -0.5, 0.2, 0.78];
            let n = (raw[0] * raw[0] + raw[1] * raw[1] + raw[2] * raw[2] + raw[3] * raw[3]).sqrt();
            [raw[0] / n, raw[1] / n, raw[2] / n, raw[3] / n]
        };
        let m = rot(&q);
        for r in 0..3 {
            for c in 0..3 {
                let dot = (0..3).map(|k| m[4 * r + k] * m[4 * c + k]).sum::<f32>();
                let want = if r == c { 1.0 } else { 0.0 };
                assert!((dot - want).abs() < 1e-5, "rows {r},{c}: dot={dot}");
            }
        }
    }

    /// Matches the rotation block the landed `0x7bddb0` kernel builds.
    ///
    /// Same expansion, different diagonal association; within a ulp-scale
    /// tolerance.
    #[test]
    fn matches_c44_rotation_block() {
        let q = [0.18_f32, 0.41, -0.32, 0.83];
        let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
        let q = [q[0] / n, q[1] / n, q[2] / n, q[3] / n];
        let m = rot(&q);
        let [x, y, z, w] = q;
        let (xx, yy, zz) = (2.0 * x * x, 2.0 * y * y, 2.0 * z * z);
        let (xy, xz, yz) = (2.0 * x * y, 2.0 * x * z, 2.0 * y * z);
        let (xw, yw, zw) = (2.0 * x * w, 2.0 * y * w, 2.0 * z * w);
        let want = [
            1.0 - (yy + zz),
            xy + zw,
            xz - yw,
            xy - zw,
            1.0 - (xx + zz),
            yz + xw,
            xz + yw,
            yz - xw,
            1.0 - (xx + yy),
        ];
        let got = [m[0], m[1], m[2], m[4], m[5], m[6], m[8], m[9], m[10]];
        for (i, (g, w)) in got.iter().zip(want.iter()).enumerate() {
            assert!((g - w).abs() <= 2e-7, "cell {i}: {g} vs {w}");
        }
    }
}

/// Pivot transformed by the row-vector convention including the translation row.
///
/// Summed in the natural lane order `((x + y) + z) + w` (the billboard
/// pre-pass flavor; the post-rebuild flavor groups differently, see
/// [`bb_pivot_world_zyx__714260`]).
pub fn bb_pivot_world_xyz__714260(m: &[f32; 16], p: &[f32; 3]) -> [f32; 3] {
    [
        (m[0] * p[0] + m[4] * p[1] + m[8] * p[2]) + m[12],
        (m[1] * p[0] + m[5] * p[1] + m[9] * p[2]) + m[13],
        (m[2] * p[0] + m[6] * p[1] + m[10] * p[2]) + m[14],
    ]
}

/// Pivot world position as the billboard rebuild computes it.
///
/// Lane 0 sums `(z + y) + x`, lanes 1-2 sum `(x + z) + y` — the per-lane x87
/// schedules differ in the original and the groupings are load-bearing for bit
/// fidelity.
pub fn bb_pivot_world_zyx__714260(m: &[f32; 16], p: &[f32; 3]) -> [f32; 3] {
    [
        (m[8] * p[2] + m[4] * p[1] + m[0] * p[0]) + m[12],
        (m[1] * p[0] + m[9] * p[2] + m[5] * p[1]) + m[13],
        (m[2] * p[0] + m[10] * p[2] + m[6] * p[1]) + m[14],
    ]
}

/// Per-row Euclidean lengths of the three rotation rows, each summed `(x² + y²) + z²`.
pub fn bb_row_lengths__714260(m: &[f32; 16]) -> [f32; 3] {
    [
        (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt(),
        (m[4] * m[4] + m[5] * m[5] + m[6] * m[6]).sqrt(),
        (m[8] * m[8] + m[9] * m[9] + m[10] * m[10]).sqrt(),
    ]
}

/// Normalize a basis row given its squared magnitude.
///
/// Skipped only when `|sqrt(sqmag)|` is strictly below the 2⁻²² epsilon (an
/// unordered compare — NaN — still normalizes, poisoning the row like the
/// original).
// `!(s.abs() < EPS)` keeps the unordered case on the normalize arm; the `>=` form
// clippy suggests would send NaN to the skip arm and lose the original's poisoning.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn bb_normalize_row__714260(row: [f32; 3], sqmag: f32) -> [f32; 3] {
    const EPS: f32 = f32::from_bits(0x3480_0000);
    let s = sqmag.sqrt();
    if !(s.abs() < EPS) {
        let f = 1.0 / s;
        return [row[0] * f, row[1] * f, row[2] * f];
    }
    row
}

/// Row replacement ratio for the project-onto-model-rows billboard mode.
///
/// `sqrt(local² / model²)` when the model row clears the 1e-5 squared-length
/// floor (ordered compare — NaN falls to the 1.0 arm), else 1.0. `local_sqmag`
/// is computed by the caller with the mode's own `(x² + z²) + y²` grouping.
pub fn bb_row_ratio__714260(model_sqmag: f32, local_sqmag: f32) -> f32 {
    const EPS_SQ: f32 = f32::from_bits(0x3727_c5ac);
    if model_sqmag > EPS_SQ {
        (local_sqmag / model_sqmag).sqrt()
    } else {
        1.0
    }
}

/// The local-row squared magnitude feeding [`bb_row_ratio__714260`].
///
/// `(x² + z²) + y²` (the original pairs the x and z squares first).
pub fn bb_row_sqmag_xzy__714260(row: &[f32; 3]) -> f32 {
    row[0] * row[0] + row[2] * row[2] + row[1] * row[1]
}

/// Cross product in the swapped orientation the axis-locked billboard modes use.
///
/// For their third row (`row1 × row0` expressed against `a = row0`,
/// `b = row1`).
pub fn bb_cross_swapped__714260(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    [
        a[2] * b[1] - a[1] * b[2],
        a[0] * b[2] - a[2] * b[0],
        a[1] * b[0] - a[0] * b[1],
    ]
}

/// Standard-orientation cross product.
///
/// The z-locked billboard mode's first row, `a = row2`, `b = row1`.
pub fn bb_cross_std__714260(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Billboard finalize.
///
/// Zero the fourth column, rescale the rebuilt rows by the pre-rebuild lengths,
/// set `m[15] = 1`, and recompute the translation row as
/// `pivot_world − (scaled-basis · pivot)` with the original's `(y + z) + x` lane
/// summation over the already-rescaled (f32-rounded) rows.
pub fn bb_finalize__714260(m: &mut [f32; 16], l: &[f32; 3], pivot: &[f32; 3], pw: &[f32; 3]) {
    m[3] = 0.0;
    m[7] = 0.0;
    m[11] = 0.0;
    m[0] *= l[0];
    m[1] *= l[0];
    m[2] *= l[0];
    m[4] *= l[1];
    m[5] *= l[1];
    m[6] *= l[1];
    m[8] *= l[2];
    m[9] *= l[2];
    m[10] *= l[2];
    m[15] = 1.0;
    m[12] = pw[0] - (m[4] * pivot[1] + m[8] * pivot[2] + m[0] * pivot[0]);
    m[13] = pw[1] - (m[5] * pivot[1] + m[9] * pivot[2] + m[1] * pivot[0]);
    m[14] = pw[2] - (m[6] * pivot[1] + m[10] * pivot[2] + m[2] * pivot[0]);
}

/// Translation-row fixup after the rotation/scale compose.
///
/// `trans − (basis ·
/// pivot)` with the `(z + y) + x` lane summation of the original.
pub fn bb_trans_fixup__714260(trans: &[f32; 3], m: &[f32; 16], pivot: &[f32; 3]) -> [f32; 3] {
    [
        trans[0] - (m[8] * pivot[2] + m[4] * pivot[1] + m[0] * pivot[0]),
        trans[1] - (m[9] * pivot[2] + m[5] * pivot[1] + m[1] * pivot[0]),
        trans[2] - (m[10] * pivot[2] + m[6] * pivot[1] + m[2] * pivot[0]),
    ]
}

#[cfg(test)]
mod tests_bb__714260 {
    use super::{
        bb_cross_std__714260 as cross_std, bb_cross_swapped__714260 as cross_swapped,
        bb_finalize__714260 as finalize, bb_normalize_row__714260 as norm,
        bb_pivot_world_xyz__714260 as pw_xyz, bb_pivot_world_zyx__714260 as pw_zyx,
        bb_row_lengths__714260 as lengths, bb_row_ratio__714260 as ratio,
        bb_row_sqmag_xzy__714260 as sqmag_xzy, bb_trans_fixup__714260 as fixup,
    };

    const M: [f32; 16] = [
        0.6, 0.8, 0.0, 0.0, //
        -0.8, 0.6, 0.0, 0.0, //
        0.0, 0.0, 2.0, 0.0, //
        10.0, 20.0, 30.0, 1.0,
    ];
    const P: [f32; 3] = [1.0, 2.0, 3.0];

    #[test]
    fn pivot_world_groupings_bit_exact() {
        let a = pw_xyz(&M, &P);
        assert_eq!(
            a[0].to_bits(),
            ((M[0] * P[0] + M[4] * P[1] + M[8] * P[2]) + M[12]).to_bits()
        );
        let b = pw_zyx(&M, &P);
        assert_eq!(
            b[0].to_bits(),
            ((M[8] * P[2] + M[4] * P[1] + M[0] * P[0]) + M[12]).to_bits()
        );
        assert_eq!(
            b[1].to_bits(),
            ((M[1] * P[0] + M[9] * P[2] + M[5] * P[1]) + M[13]).to_bits()
        );
    }

    #[test]
    fn row_lengths_known() {
        let l = lengths(&M);
        assert_eq!(l[0].to_bits(), 1.0f32.to_bits());
        assert_eq!(l[2].to_bits(), 2.0f32.to_bits());
    }

    #[test]
    fn normalize_guard_boundaries() {
        // Length exactly 0: sqrt(0) = 0 < eps -> untouched.
        assert_eq!(norm([0.0, 0.0, 0.0], 0.0), [0.0, 0.0, 0.0]);
        // Tiny but above eps after sqrt: normalized.
        let r = norm([2.0, 0.0, 0.0], 4.0);
        assert_eq!(r[0].to_bits(), 1.0f32.to_bits());
        // NaN sqmag still normalizes (poisons the row).
        assert!(norm([1.0, 0.0, 0.0], f32::NAN)[0].is_nan());
        // Negative sqmag -> sqrt NaN -> normalize -> NaN.
        assert!(norm([1.0, 0.0, 0.0], -1.0)[0].is_nan());
    }

    #[test]
    fn ratio_floor() {
        assert_eq!(ratio(1.0, 4.0).to_bits(), 2.0f32.to_bits());
        // At/below the floor (and NaN): the 1.0 arm.
        assert_eq!(
            ratio(f32::from_bits(0x3727_c5ac), 4.0).to_bits(),
            1.0f32.to_bits()
        );
        assert_eq!(ratio(0.0, 4.0).to_bits(), 1.0f32.to_bits());
        assert_eq!(ratio(f32::NAN, 4.0).to_bits(), 1.0f32.to_bits());
    }

    #[test]
    fn sqmag_grouping_is_x_z_y() {
        let row = [3.0f32, 5.0, 7.0];
        assert_eq!(
            sqmag_xzy(&row).to_bits(),
            (row[0] * row[0] + row[2] * row[2] + row[1] * row[1]).to_bits()
        );
    }

    #[test]
    fn cross_orientations_oppose() {
        let a = [1.0f32, 0.0, 0.0];
        let b = [0.0f32, 1.0, 0.0];
        assert_eq!(cross_std(&a, &b), [0.0, 0.0, 1.0]);
        assert_eq!(cross_swapped(&a, &b), [0.0, -0.0, -1.0]);
    }

    #[test]
    fn finalize_rescales_and_solves_translation() {
        let mut m = M;
        let l = [2.0f32, 3.0, 4.0];
        let pw = [100.0f32, 200.0, 300.0];
        finalize(&mut m, &l, &P, &pw);
        assert_eq!(m[3].to_bits(), 0.0f32.to_bits());
        assert_eq!(m[15].to_bits(), 1.0f32.to_bits());
        assert_eq!(m[0].to_bits(), (2.0f32 * 0.6).to_bits());
        assert_eq!(
            m[12].to_bits(),
            (100.0f32 - (m[4] * P[1] + m[8] * P[2] + m[0] * P[0])).to_bits()
        );
    }

    #[test]
    fn fixup_grouping_is_z_y_x() {
        let t = [1.0f32, 2.0, 3.0];
        let f = fixup(&t, &M, &P);
        assert_eq!(
            f[0].to_bits(),
            (t[0] - (M[8] * P[2] + M[4] * P[1] + M[0] * P[0])).to_bits()
        );
    }
}

/// A 3x3 minor as a caller that stores it into an `f32` slot sees it.
///
/// `C33Matrix::Determinant` leaves its result at register width; this is the
/// narrowing that the storing callers perform and the 4x4 determinant does not.
// The nine elements are the client's own parameter list at `0x7bc040`, so the
// signature has to keep that shape to forward them unchanged.
#[allow(clippy::too_many_arguments)]
#[inline]
fn minor3_narrowed(
    m00: f32,
    m01: f32,
    m02: f32,
    m10: f32,
    m11: f32,
    m12: f32,
    m20: f32,
    m21: f32,
    m22: f32,
) -> f32 {
    super::f64_to_f32(crate::math::matrix33::c33_matrix__determinant__7bc040(
        m00, m01, m02, m10, m11, m12, m20, m21, m22,
    ))
}

/// `C44Matrix::Adjugate` — 4×4 adjugate (transpose of the cofactor matrix).
///
/// The numerator of the inverse. The original evaluates the sixteen 3×3 minors
/// with `C33Matrix::Determinant`, applies eight `fchs` sign flips, and stores
/// each cofactor into its transposed output slot.
///
/// Each minor selects nine source elements in the exact order the original
/// passes them to the determinant (matching the per-call push order of the
/// original), and the eight negations are reproduced as f32 negation:
/// stock negates the value before the `fstp` rounds it to f32, and
/// negation is an exact sign-bit flip that commutes with rounding, so `-det(..)`
/// is bit-identical. Input/output are row-major 16-float matrices.
pub fn c44_matrix__adjugate__7bd390(src: &[f32; 16]) -> [f32; 16] {
    let s = src;
    // Unlike the 4x4 determinant, which keeps its minors at register width and
    // never stores the last term, every slot here reaches an `f32` element of
    // the output, so each minor narrows at its own store. The negations stay
    // outside the narrowing: a sign flip is exact and commutes with rounding.
    let det = minor3_narrowed;
    [
        // out[0]  = +minor(rows 1,2,3 / cols 1,2,3)
        det(s[5], s[6], s[7], s[9], s[10], s[11], s[13], s[14], s[15]),
        // out[1]  = -minor(rows 0,2,3 / cols 1,2,3)
        -det(s[1], s[2], s[3], s[9], s[10], s[11], s[13], s[14], s[15]),
        // out[2]  = +minor(rows 0,1,3 / cols 1,2,3)
        det(s[1], s[2], s[3], s[5], s[6], s[7], s[13], s[14], s[15]),
        // out[3]  = -minor(rows 0,1,2 / cols 1,2,3)
        -det(s[1], s[2], s[3], s[5], s[6], s[7], s[9], s[10], s[11]),
        // out[4]  = -minor(rows 1,2,3 / cols 0,2,3)
        -det(s[4], s[6], s[7], s[8], s[10], s[11], s[12], s[14], s[15]),
        // out[5]  = +minor(rows 0,2,3 / cols 0,2,3)
        det(s[0], s[2], s[3], s[8], s[10], s[11], s[12], s[14], s[15]),
        // out[6]  = -minor(rows 0,1,3 / cols 0,2,3)
        -det(s[0], s[2], s[3], s[4], s[6], s[7], s[12], s[14], s[15]),
        // out[7]  = +minor(rows 0,1,2 / cols 0,2,3)
        det(s[0], s[2], s[3], s[4], s[6], s[7], s[8], s[10], s[11]),
        // out[8]  = +minor(rows 1,2,3 / cols 0,1,3)
        det(s[4], s[5], s[7], s[8], s[9], s[11], s[12], s[13], s[15]),
        // out[9]  = -minor(rows 0,2,3 / cols 0,1,3)
        -det(s[0], s[1], s[3], s[8], s[9], s[11], s[12], s[13], s[15]),
        // out[10] = +minor(rows 0,1,3 / cols 0,1,3)
        det(s[0], s[1], s[3], s[4], s[5], s[7], s[12], s[13], s[15]),
        // out[11] = -minor(rows 0,1,2 / cols 0,1,3)
        -det(s[0], s[1], s[3], s[4], s[5], s[7], s[8], s[9], s[11]),
        // out[12] = -minor(rows 1,2,3 / cols 0,1,2)
        -det(s[4], s[5], s[6], s[8], s[9], s[10], s[12], s[13], s[14]),
        // out[13] = +minor(rows 0,2,3 / cols 0,1,2)
        det(s[0], s[1], s[2], s[8], s[9], s[10], s[12], s[13], s[14]),
        // out[14] = -minor(rows 0,1,3 / cols 0,1,2)
        -det(s[0], s[1], s[2], s[4], s[5], s[6], s[12], s[13], s[14]),
        // out[15] = +minor(rows 0,1,2 / cols 0,1,2)
        det(s[0], s[1], s[2], s[4], s[5], s[6], s[8], s[9], s[10]),
    ]
}

#[cfg(test)]
mod tests_c44_matrix__adjugate__7bd390 {
    use super::c44_matrix__adjugate__7bd390 as adjugate;

    // Independent 3x3 determinant (different term order from the kernel's det,
    // so this is a genuine oracle, not a copy).
    fn det3(m: &[f32; 9]) -> f32 {
        m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6])
            + m[2] * (m[3] * m[7] - m[4] * m[6])
    }

    // Minor: 3x3 block of `src` with row `skip_r` and col `skip_c` removed.
    fn minor(src: &[f32; 16], skip_r: usize, skip_c: usize) -> [f32; 9] {
        let mut m = [0.0f32; 9];
        let mut k = 0;
        for r in 0..4 {
            if r == skip_r {
                continue;
            }
            for c in 0..4 {
                if c == skip_c {
                    continue;
                }
                m[k] = src[r * 4 + c];
                k += 1;
            }
        }
        m
    }

    // Textbook adjugate: adj[i][j] = (-1)^(i+j) * minor(j, i)  (cofactor, transposed).
    fn adjugate_ref(src: &[f32; 16]) -> [f32; 16] {
        let mut out = [0.0f32; 16];
        for i in 0..4 {
            for j in 0..4 {
                let sign = if (i + j) % 2 == 0 { 1.0 } else { -1.0 };
                out[i * 4 + j] = sign * det3(&minor(src, j, i));
            }
        }
        out
    }

    fn det4(m: &[f32; 16]) -> f32 {
        let mut d = 0.0f32;
        for j in 0..4 {
            let sign = if j % 2 == 0 { 1.0 } else { -1.0 };
            d += sign * m[j] * det3(&minor(m, 0, j));
        }
        d
    }

    fn mul4(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
        let mut out = [0.0f32; 16];
        for r in 0..4 {
            for c in 0..4 {
                let mut s = 0.0f32;
                for k in 0..4 {
                    s += a[r * 4 + k] * b[k * 4 + c];
                }
                out[r * 4 + c] = s;
            }
        }
        out
    }

    const IDENTITY: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ];

    #[test]
    fn identity_adjugate_is_identity() {
        // adj(I) == I numerically. The eight negated cofactors of a zero minor
        // produce -0.0 (faithful: stock's FCHS of a 0 determinant), so compare by
        // value (-0.0 == 0.0), not by raw bits.
        let a = adjugate(&IDENTITY);
        for i in 0..16 {
            assert_eq!(a[i], IDENTITY[i], "idx {i}");
        }
    }

    #[test]
    fn matches_textbook_adjugate() {
        let m: [f32; 16] = [
            2.0, -1.0, 0.0, 3.0, //
            1.0, 4.0, -2.0, 0.5, //
            0.0, 2.0, 5.0, -1.0, //
            -3.0, 1.0, 2.0, 4.0,
        ];
        let got = adjugate(&m);
        let want = adjugate_ref(&m);
        for i in 0..16 {
            assert!(
                (got[i] - want[i]).abs() < 1e-2,
                "idx {i}: {} vs {}",
                got[i],
                want[i]
            );
        }
    }

    #[test]
    fn m_times_adj_is_det_times_identity() {
        let m: [f32; 16] = [
            1.0, 2.0, 0.5, -1.0, //
            0.0, 3.0, 1.0, 2.0, //
            -2.0, 1.0, 4.0, 0.0, //
            1.0, -1.0, 2.0, 3.0,
        ];
        let adj = adjugate(&m);
        let prod = mul4(&m, &adj);
        let d = det4(&m);
        let id_scaled: [f32; 16] = {
            let mut s = [0.0f32; 16];
            for i in 0..4 {
                s[i * 4 + i] = d;
            }
            s
        };
        for i in 0..16 {
            assert!(
                (prod[i] - id_scaled[i]).abs() < 1e-1,
                "idx {i}: {} vs {}",
                prod[i],
                id_scaled[i]
            );
        }
    }

    #[test]
    fn sign_pattern_pinned_on_basis() {
        // A matrix whose minors are easy to reason about: a checkerboard pin of
        // the eight negated slots. Use the textbook reference as ground truth and
        // confirm the kernel agrees slot-for-slot (catches a transposed write or
        // a missing/extra fchs).
        let m: [f32; 16] = [
            3.0, 1.0, -2.0, 0.0, //
            -1.0, 2.0, 1.0, 4.0, //
            2.0, 0.0, 3.0, -1.0, //
            1.0, -3.0, 0.0, 2.0,
        ];
        let got = adjugate(&m);
        let want = adjugate_ref(&m);
        for i in 0..16 {
            // Same sign in every slot (the load-bearing property of the layout).
            assert!(
                got[i].signum() == want[i].signum()
                    || (got[i].abs() < 1e-3 && want[i].abs() < 1e-3),
                "sign mismatch idx {i}: {} vs {}",
                got[i],
                want[i]
            );
            assert!(
                (got[i] - want[i]).abs() < 1e-2,
                "idx {i}: {} vs {}",
                got[i],
                want[i]
            );
        }
    }
}

/// `C44Matrix::Determinant` (0x7bcf90).
///
/// `__fastcall(ecx = this)`, returns the 4×4 determinant in `ST(0)`. (The return
/// is in `ST(0)`, not a hidden `float10*` out-pointer.) Cofactor expansion along
/// row 0: `det = m00·M0 − m01·M1 + m02·M2 − m03·M3`, where each `Mk` is the 3×3
/// minor with row 0 and column `k` removed, evaluated by `C33Matrix::Determinant`
/// in the exact nine-element order the original pushes (verified against the four
/// `CALL 0x7bc040` push sequences). The accumulation order `((t0 − t1) + t2) − t3`
/// matches the stock `FSUBR`/`FADD`/`FSUBR` chain.
///
/// The widths are mixed and each one is deliberate. The running total **is**
/// spilled, by three `FSTP dword` to one frame slot after the first, second and
/// third terms, so those three narrow to `f32`. Nothing else does: each minor
/// arrives from `0x7bc040` at register width and is multiplied by an `f32`
/// element without being rounded first, and the fourth term is never stored, so
/// the result leaves in `ST(0)` and each caller narrows it or does not. One
/// caller uses `FST` and keeps the wide value live, which is why the narrowing
/// belongs at the call site rather than here.
pub fn c44_matrix__determinant__7bcf90(m: &[f32; 16]) -> f64 {
    let det3 = crate::math::matrix33::c33_matrix__determinant__7bc040;
    // Minor for the m0j cofactor: row 0 and column j removed (3×3 of rows 1..=3).
    let m0 = det3(m[5], m[6], m[7], m[9], m[10], m[11], m[13], m[14], m[15]);
    let m1 = det3(m[4], m[6], m[7], m[8], m[10], m[11], m[12], m[14], m[15]);
    let m2 = det3(m[4], m[5], m[7], m[8], m[9], m[11], m[12], m[13], m[15]);
    let m3 = det3(m[4], m[5], m[6], m[8], m[9], m[10], m[12], m[13], m[14]);
    let t0 = super::f64_to_f32(f64::from(m[0]) * m0);
    let t1 = super::f64_to_f32(f64::from(t0) - f64::from(m[1]) * m1);
    let t2 = super::f64_to_f32(f64::from(t1) + f64::from(m[2]) * m2);
    f64::from(t2) - f64::from(m[3]) * m3
}

#[cfg(test)]
mod tests_c44_matrix__determinant__7bcf90 {
    use super::c44_matrix__determinant__7bcf90 as det4;

    // Independent Laplace-expansion oracle (textbook minors, different term order).
    fn det4_ref(m: &[f32; 16]) -> f32 {
        fn minor3(m: &[f32; 16], r: usize, c: usize) -> f32 {
            let mut v = [0.0f32; 9];
            let mut k = 0;
            for rr in 0..4 {
                if rr == r {
                    continue;
                }
                for cc in 0..4 {
                    if cc == c {
                        continue;
                    }
                    v[k] = m[rr * 4 + cc];
                    k += 1;
                }
            }
            v[0] * (v[4] * v[8] - v[5] * v[7]) - v[1] * (v[3] * v[8] - v[5] * v[6])
                + v[2] * (v[3] * v[7] - v[4] * v[6])
        }
        m[0] * minor3(m, 0, 0) - m[1] * minor3(m, 0, 1) + m[2] * minor3(m, 0, 2)
            - m[3] * minor3(m, 0, 3)
    }

    #[test]
    fn identity_is_one() {
        let mut m = [0.0f32; 16];
        for i in 0..4 {
            m[i * 4 + i] = 1.0;
        }
        assert!((det4(&m) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn singular_is_zero() {
        // two identical rows -> det 0
        let m = [
            1.0, 2.0, 3.0, 4.0, //
            1.0, 2.0, 3.0, 4.0, //
            5.0, 6.0, 7.0, 8.0, //
            9.0, 1.0, 2.0, 3.0,
        ];
        assert!(det4(&m).abs() < 1e-3);
    }

    #[test]
    fn matches_reference() {
        let mats: [[f32; 16]; 3] = [
            [
                3.0, -2.0, 0.5, 1.0, 0.0, 4.0, -1.0, 2.0, 1.5, 0.0, 2.0, -3.0, -1.0, 1.0, 0.0, 5.0,
            ],
            [
                1.0, 0.0, 0.0, 2.0, 0.0, 1.0, 0.0, 3.0, 0.0, 0.0, 1.0, 4.0, 0.0, 0.0, 0.0, 1.0,
            ],
            [
                2.0, 1.0, 1.0, 0.0, 1.0, 3.0, 0.0, 1.0, 0.0, 1.0, 2.0, 1.0, 1.0, 0.0, 1.0, 2.0,
            ],
        ];
        for m in &mats {
            let got = det4(m);
            let want = f64::from(det4_ref(m));
            assert!(
                (got - want).abs() <= want.abs() * 1e-4 + 1e-3,
                "{got} vs {want}"
            );
        }
    }

    #[test]
    fn scaling_row_scales_det() {
        let base = [
            2.0, -1.0, 0.5, 1.0, 3.0, 1.0, 4.0, -2.0, -2.0, 0.0, 1.0, 1.0, 0.5, 2.0, -1.0, 3.0,
        ];
        let mut scaled = base;
        for s in scaled.iter_mut().take(4) {
            *s *= 3.0; // scale row 0 by 3 -> det *3
        }
        let want = 3.0 * det4(&base);
        assert!((det4(&scaled) - want).abs() <= want.abs() * 1e-4 + 1e-3);
    }
}

/// `C44Matrix::InverseScaledByDet` (0x7bd6c0).
///
/// `__thiscall(ecx = src, [out],
/// [det])`. Builds the inverse of `src` given its precomputed determinant:
/// `out = adjugate(src) · (1/det)`, then returns `out`. The reciprocal numerator
/// is the stock `1.0f` constant at `0x7ff9d8` (`FLD; FDIV det`). Reuses the
/// `Adjugate` and `MultiplyScalar` kernels, so it inherits their verified
/// element order.
pub fn c44_matrix__inverse_scaled_by_det__7bd6c0(src: &[f32; 16], determinant: f32) -> [f32; 16] {
    let inv_det = 1.0f32 / determinant;
    let adj = c44_matrix__adjugate__7bd390(src);
    c44_matrix__multiply_scalar__7bc8e0(&adj, inv_det)
}

#[cfg(test)]
mod tests_c44_matrix__inverse_scaled_by_det__7bd6c0 {
    use super::{
        c44_matrix__determinant__7bcf90 as det4,
        c44_matrix__inverse_scaled_by_det__7bd6c0 as inverse,
    };

    fn mul4(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
        let mut o = [0.0f32; 16];
        for r in 0..4 {
            for c in 0..4 {
                let mut s = 0.0f32;
                for k in 0..4 {
                    s += a[r * 4 + k] * b[k * 4 + c];
                }
                o[r * 4 + c] = s;
            }
        }
        o
    }

    #[test]
    fn inverse_times_original_is_identity() {
        let m = [
            4.0, 7.0, 2.0, 0.0, 3.0, 6.0, 1.0, 0.0, 2.0, 5.0, 9.0, 0.0, 1.0, 2.0, 3.0, 1.0,
        ];
        // The inverse takes the determinant as the f32 its caller stores.
        let d = super::super::f64_to_f32(det4(&m));
        let inv = inverse(&m, d);
        let prod = mul4(&inv, &m);
        for r in 0..4 {
            for c in 0..4 {
                let want = if r == c { 1.0 } else { 0.0 };
                assert!(
                    (prod[r * 4 + c] - want).abs() < 1e-2,
                    "[{r}][{c}] = {}",
                    prod[r * 4 + c]
                );
            }
        }
    }

    #[test]
    fn identity_inverse_is_identity() {
        let mut id = [0.0f32; 16];
        for i in 0..4 {
            id[i * 4 + i] = 1.0;
        }
        let inv = inverse(&id, 1.0);
        for i in 0..16 {
            assert!((inv[i] - id[i]).abs() < 1e-6, "i={i}");
        }
    }
}

/// Scales the upper-left 3x3 basis of a 4x4 matrix (4-float row stride).
///
/// By a single scalar `s`, in place.
///
/// The nine basis elements — row 0 (indices 0,1,2), row 1 (indices 4,5,6), row
/// 2 (indices 8,9,10) — become `s * m[i]`. The translation column (indices
/// 3,7,11) and the homogeneous fourth row (indices 12..=15) are untouched: the
/// x87 original only issues nine `FLD s; FMUL m[i]; FSTP m[i]` triples. There
/// are no compares, so a NaN (in `s` or any element) simply propagates through
/// the multiply — no ordered-vs-unordered polarity to reproduce.
pub fn c44_matrix__scale_rotation3x3__7bdd00(mat: &mut [f32; 16], s: f32) {
    mat[0] *= s;
    mat[1] *= s;
    mat[2] *= s;
    mat[4] *= s;
    mat[5] *= s;
    mat[6] *= s;
    mat[8] *= s;
    mat[9] *= s;
    mat[10] *= s;
}

#[cfg(test)]
mod tests_c44_matrix__scale_rotation3x3__7bdd00 {
    use super::c44_matrix__scale_rotation3x3__7bdd00 as scale;

    /// A 4x4 with distinct, recognisable values.
    ///
    /// The 3x3 basis, the translation column at 3/7/11, and the homogeneous row
    /// at 12..=15.
    fn full() -> [f32; 16] {
        [
            1.0, 2.0, 3.0, 91.0, 4.0, 5.0, 6.0, 92.0, 7.0, 8.0, 9.0, 93.0, 94.0, 95.0, 96.0, 1.0,
        ]
    }

    /// The nine basis float indices, in the stock write order.
    const BASIS: [usize; 9] = [0, 1, 2, 4, 5, 6, 8, 9, 10];
    /// Everything else: translation column + homogeneous row (stock never touches).
    const UNTOUCHED: [usize; 7] = [3, 7, 11, 12, 13, 14, 15];

    #[test]
    fn scales_only_the_basis() {
        let before = full();
        let mut m = before;
        scale(&mut m, 2.0);
        // Independent oracle: each basis element doubled, the rest identical.
        for i in BASIS {
            assert_eq!(m[i], before[i] * 2.0, "basis index {i}");
        }
        for i in UNTOUCHED {
            assert_eq!(m[i], before[i], "untouched index {i}");
        }
    }

    #[test]
    fn one_is_neutral() {
        let before = full();
        let mut m = before;
        scale(&mut m, 1.0);
        assert_eq!(m, before);
    }

    #[test]
    fn zero_zeros_the_basis_only() {
        let before = full();
        let mut m = before;
        scale(&mut m, 0.0);
        for i in BASIS {
            assert_eq!(m[i], 0.0, "basis index {i}");
        }
        for i in UNTOUCHED {
            assert_eq!(m[i], before[i], "untouched index {i}");
        }
    }

    #[test]
    fn negative_flips_sign_over_grid() {
        // Independent oracle across a value grid: result == s * original.
        for &s in &[-3.5f32, -1.0, 0.25, 7.0] {
            let before = full();
            let mut m = before;
            scale(&mut m, s);
            for i in BASIS {
                assert_eq!(m[i], before[i] * s, "s={s} index {i}");
            }
        }
    }

    #[test]
    fn nan_scalar_propagates_to_basis_only() {
        // No compares in the stock body ⇒ NaN just flows through the multiply;
        // the untouched slots (never multiplied) keep their finite values.
        let before = full();
        let mut m = before;
        scale(&mut m, f32::NAN);
        for i in BASIS {
            assert!(m[i].is_nan(), "basis index {i} should be NaN");
        }
        for i in UNTOUCHED {
            assert_eq!(m[i], before[i], "untouched index {i}");
        }
    }

    #[test]
    fn nan_element_stays_nan() {
        let mut m = full();
        m[5] = f32::NAN;
        scale(&mut m, 2.0);
        assert!(m[5].is_nan());
        // A finite neighbour in the same basis is scaled normally.
        assert_eq!(m[4], 4.0 * 2.0);
    }
}

/// `BuildOrthoProjMatrix` (VA 0x5c3d90).
///
/// Off-center **orthographic** 4×4 row-major projection matrix built into `out`.
/// `k` is the runtime scale constant at `0x801628` (2.0 in the shipped image);
/// read live by the adapter so the kernel stays value-faithful.
///
/// Diagonal scales `k/(r−l)`, `k/(t−b)`, `k/(f−n)`; the translation row
/// (indices 12/13/14) `−(l+r)/(r−l)`, `−(b+t)/(t−b)`, `−(n+f)/(f−n)`; `m33 = 1`;
/// all other cells 0. The subtracted denominator is computed once and reused for
/// both the scale and the translation of each axis, matching the x87 code's
/// single `FSUB` + leftover `st(1)` (at 0x5c3dc5).
///
/// Returns `false` **without writing `out`** when the range is degenerate,
/// reproducing the stock guard's *exact* x87 NaN polarity:
///   * `left != right` (FCOMP/`TEST AH,0x44`/`JNP` — only the ordered-equal case
///     trips the error; a NaN operand *passes*, matching C `!=`),
///   * `bottom != top` (same),
///   * `zNear < zFar` (FCOMP/`TEST AH,0x5`/`JNP` to the build — strict `<`; both
///     `zNear >= zFar` *and* NaN fall to the error, matching C `<`).
///
/// On `false` the caller raises Storm error 0x57; do NOT normalize these
/// comparisons — the IEEE `!=`/`<` forms reproduce all three polarities.
// The eight parameters are the original's argument list at this entry point — the `out`
// matrix and the seven frustum scalars. That count is a fact about the client's ABI,
// not a design choice.
#[allow(clippy::too_many_arguments)]
pub fn build_ortho_proj_matrix__5c3d90(
    out: &mut [f32; 16],
    left: f32,
    right: f32,
    bottom: f32,
    top: f32,
    z_near: f32,
    z_far: f32,
    k: f32,
) -> bool {
    // Exact x87 guard polarity: equal ⇒ reject, NaN passes the two `!=`, and
    // NaN (or n>=f) fails the `<`. Any false ⇒ degenerate, leave `out` untouched.
    if !(left != right && bottom != top && z_near < z_far) {
        return false;
    }
    let rl = right - left;
    let tb = top - bottom;
    let nf = z_far - z_near;
    *out = [
        k / rl,
        0.0,
        0.0,
        0.0,
        0.0,
        k / tb,
        0.0,
        0.0,
        0.0,
        0.0,
        k / nf,
        0.0,
        -((left + right) / rl),
        -((bottom + top) / tb),
        -((z_near + z_far) / nf),
        1.0,
    ];
    true
}

#[cfg(test)]
mod tests_build_ortho_proj_matrix__5c3d90 {
    use super::build_ortho_proj_matrix__5c3d90 as ortho;

    // Independent oracle: standard off-center orthographic build with scale `k`.
    fn oracle(l: f32, r: f32, b: f32, t: f32, n: f32, f: f32, k: f32) -> [f32; 16] {
        [
            k / (r - l),
            0.0,
            0.0,
            0.0,
            0.0,
            k / (t - b),
            0.0,
            0.0,
            0.0,
            0.0,
            k / (f - n),
            0.0,
            -(l + r) / (r - l),
            -(b + t) / (t - b),
            -(n + f) / (f - n),
            1.0,
        ]
    }

    #[test]
    fn builds_expected_cells_k2() {
        // K = 2.0 (the shipped value at `0x801628`) ⇒ a symmetric [-1,1] box maps to
        // the OpenGL-style ortho: scale 1 on each axis, zero translation.
        let mut out = [f32::NAN; 16];
        assert!(ortho(&mut out, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, 2.0));
        let want = [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ];
        assert_eq!(out, want);
    }

    #[test]
    fn matches_oracle_offcenter() {
        let (l, r, b, t, n, f, k) = (2.0f32, 10.0, -3.0, 5.0, 0.5, 100.0, 2.0);
        let mut out = [0.0f32; 16];
        assert!(ortho(&mut out, l, r, b, t, n, f, k));
        assert_eq!(out, oracle(l, r, b, t, n, f, k));
    }

    #[test]
    fn honours_runtime_k() {
        // A non-2.0 `k` scales the diagonal but leaves the translation row and
        // m33 unchanged (translation is `-(l+r)/(r-l)`, no `k`).
        let mut out = [0.0f32; 16];
        assert!(ortho(&mut out, 0.0, 4.0, 0.0, 8.0, 1.0, 3.0, 6.0));
        assert_eq!(out[0], 6.0 / 4.0);
        assert_eq!(out[5], 6.0 / 8.0);
        assert_eq!(out[10], 6.0 / 2.0);
        assert_eq!(out[12], -(0.0 + 4.0) / 4.0);
        assert_eq!(out[13], -(0.0 + 8.0) / 8.0);
        assert_eq!(out[14], -(1.0 + 3.0) / 2.0);
        assert_eq!(out[15], 1.0);
    }

    #[test]
    // `0.30000001` is a source constant reproduced bit-exactly; trimming it to `0.3`
    // would move the rounded denominator this test exists to pin down.
    #[allow(clippy::excessive_precision)]
    fn reuses_denominator_bit_exactly() {
        // The scale and translation of each axis share the *same* rounded
        // denominator (single f32 subtract), matching the x87 `FSUB`+leftover.
        let mut out = [0.0f32; 16];
        assert!(ortho(&mut out, 0.1, 0.30000001, -0.2, 0.7, 1.5, 9.5, 2.0));
        let rl = 0.30000001f32 - 0.1;
        let tb = 0.7f32 - (-0.2);
        let nf = 9.5f32 - 1.5;
        assert_eq!(out[0], 2.0f32 / rl);
        assert_eq!(out[12], -((0.1f32 + 0.30000001) / rl));
        assert_eq!(out[5], 2.0f32 / tb);
        assert_eq!(out[13], -(((-0.2f32) + 0.7) / tb));
        assert_eq!(out[10], 2.0f32 / nf);
        assert_eq!(out[14], -((1.5f32 + 9.5) / nf));
    }

    #[test]
    fn degenerate_equal_left_right_rejects() {
        let mut out = [7.0f32; 16];
        assert!(!ortho(&mut out, 5.0, 5.0, -1.0, 1.0, 0.0, 1.0, 2.0));
        // `out` left untouched on the reject path.
        assert_eq!(out, [7.0f32; 16]);
    }

    #[test]
    fn degenerate_equal_bottom_top_rejects() {
        let mut out = [7.0f32; 16];
        assert!(!ortho(&mut out, -1.0, 1.0, 3.0, 3.0, 0.0, 1.0, 2.0));
        assert_eq!(out, [7.0f32; 16]);
    }

    #[test]
    fn z_range_requires_strict_near_lt_far() {
        // n == f ⇒ reject; n > f ⇒ reject; only n < f builds.
        let mut out = [0.0f32; 16];
        assert!(!ortho(&mut out, -1.0, 1.0, -1.0, 1.0, 2.0, 2.0, 2.0));
        assert!(!ortho(&mut out, -1.0, 1.0, -1.0, 1.0, 5.0, 3.0, 2.0));
        assert!(ortho(&mut out, -1.0, 1.0, -1.0, 1.0, 3.0, 5.0, 2.0));
    }

    #[test]
    fn nan_polarity_matches_x87() {
        let nan = f32::NAN;
        let mut out = [0.0f32; 16];
        // NaN in left/right *passes* the `!=` guard (NaN != x is true), so the
        // build proceeds (the other axes are valid).
        assert!(ortho(&mut out, nan, 1.0, -1.0, 1.0, 0.0, 1.0, 2.0));
        // NaN in bottom/top likewise passes the `!=` guard.
        assert!(ortho(&mut out, -1.0, 1.0, nan, 1.0, 0.0, 1.0, 2.0));
        // NaN in the z-range *fails* the strict `<` ⇒ reject (leaves out).
        let mut z = [4.0f32; 16];
        assert!(!ortho(&mut z, -1.0, 1.0, -1.0, 1.0, nan, 1.0, 2.0));
        assert_eq!(z, [4.0f32; 16]);
        assert!(!ortho(&mut z, -1.0, 1.0, -1.0, 1.0, 0.0, nan, 2.0));
        assert_eq!(z, [4.0f32; 16]);
    }
}
