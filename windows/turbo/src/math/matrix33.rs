//! `matrix33` family kernels.
// Adapter and kernel names mirror the host's C++ symbols verbatim, with `__`
// standing in for the `::`, so the whole module is non-snake-case by construction.
#![allow(non_snake_case)]

// --- Euler composition helpers (per-axis row-major rotations; the host builds
// three single-axis matrices, multiplies them in the named order, and stores
// the transpose). ---
fn rx3(c: f32, s: f32) -> [f32; 9] {
    [1.0, 0.0, 0.0, 0.0, c, s, 0.0, -s, c]
}
fn ry3(c: f32, s: f32) -> [f32; 9] {
    [c, 0.0, s, 0.0, 1.0, 0.0, -s, 0.0, c]
}
fn rz3(c: f32, s: f32) -> [f32; 9] {
    [c, s, 0.0, -s, c, 0.0, 0.0, 0.0, 1.0]
}
fn mul3(a: &[f32; 9], b: &[f32; 9]) -> [f32; 9] {
    // The Euler builders do not inline a product of their own: each one `call`s
    // `C33Matrix::Multiply` twice (`0x7bf5d2` and `0x7bf5de` for the ZYX order),
    // so this has to be that kernel and not a second, tidier transcription of
    // the same algebra.
    c33_matrix__multiply__7bdfc0(a, b)
}
fn transpose3(m: &[f32; 9]) -> [f32; 9] {
    [m[0], m[3], m[6], m[1], m[4], m[7], m[2], m[5], m[8]]
}

/// 3x3 matrix determinant (rule of Sarrus) of row-major elements `m00..m22`.
// The nine row-major elements arrive one per stack slot — that is the client's
// own parameter list at `0x7bc040`, a fact about its calling convention rather
// than a design choice, so folding them into a matrix struct would take the
// signature off the shape the caller passes.
#[allow(clippy::too_many_arguments)]
pub fn c33_matrix__determinant__7bc040(
    m00: f32,
    m01: f32,
    m02: f32,
    m10: f32,
    m11: f32,
    m12: f32,
    m20: f32,
    m21: f32,
    m22: f32,
) -> f64 {
    // Stock's term order is `(((((A + B) + C) - D) - E) - F)`, and A is the
    // `m01·m12·m20` triple, not the leading diagonal — the diagonal is C. The
    // body contains no `FST`/`FSTP` at all: every triple is `FLD; FMUL; FMUL`
    // and the running total is only ever touched by `FADDP`/`FSUBP`, so the
    // result leaves in `ST(0)` un-narrowed. Its only caller multiplies it
    // straight into another `FMUL` without narrowing either, hence `f64` out.
    let a = f64::from(m01) * f64::from(m12) * f64::from(m20);
    let b = f64::from(m02) * f64::from(m10) * f64::from(m21);
    let c = f64::from(m00) * f64::from(m11) * f64::from(m22);
    let d = f64::from(m02) * f64::from(m11) * f64::from(m20);
    let e = f64::from(m01) * f64::from(m10) * f64::from(m22);
    let f = f64::from(m00) * f64::from(m12) * f64::from(m21);
    ((((a + b) + c) - d) - e) - f
}

#[cfg(test)]
mod tests_c33_matrix__determinant__7bc040 {
    use super::c33_matrix__determinant__7bc040 as det;
    #[test]
    fn identity_is_one() {
        assert_eq!(
            det(1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0).to_bits(),
            1.0f64.to_bits()
        );
    }
    #[test]
    fn known_value() {
        // det of [[6,1,1],[4,-2,5],[2,8,7]] = -306
        let d = det(6.0, 1.0, 1.0, 4.0, -2.0, 5.0, 2.0, 8.0, 7.0);
        assert!((d - (-306.0)).abs() < 1e-3, "got {d}");
    }
    #[test]
    fn singular_is_zero() {
        // two identical rows -> det 0
        let d = det(1.0, 2.0, 3.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0);
        assert!(d.abs() < 1e-4, "got {d}");
    }
    #[test]
    fn scaling_row_scales_det() {
        let base = det(2.0, -1.0, 0.5, 3.0, 1.0, 4.0, -2.0, 0.0, 1.0);
        // scale first row by 3 -> det *3
        let scaled = det(6.0, -3.0, 1.5, 3.0, 1.0, 4.0, -2.0, 0.0, 1.0);
        assert!((scaled - 3.0 * base).abs() < 1e-3, "{scaled} vs {base}");
    }
}

/// Builds a 3x3 rotation matrix (Rodrigues) of `angle` radians about `axis`.
///
/// When `normalize` is true the axis is normalized first (the original's flag runs
/// the other way: a non-zero byte *skips* it). Returns row-major 9 floats.
///
/// This is also the arithmetic behind `0x7bb860` and `0x7bdb00`, which are the
/// same routine emitted against wider destination layouts. All three bodies carry
/// the identical eighty floating-point instructions once the destination offsets
/// are masked off, so those two call this and only place the result.
///
/// Widths, read off the stores rather than the operand types. The `f32` slots the
/// original really does spill to are `cos`, `sin` and `1 - cos` (each written by an
/// `FSTP m32`), the renormalized axis components, and `x * sin` / `y * sin`
/// (`0x7be517` / `0x7be520`). Everything else stays on the x87 stack for the whole
/// body, which makes three things not narrow where a plain transcription narrows
/// them:
/// - `z * sin` never leaves the register, so `out[1]` and `out[3]` take it wide
///   while their `x` and `y` counterparts take a rounded product.
/// - the three axis pair products are exact (`f32 * f32`) and are multiplied by
///   `1 - cos` **after** the pair, not folded left to right: the original's
///   `FLD t; FMUL ST(4)` forms `t * (y * x)`, and `(t * y) * x` is other bits.
/// - `t * (z * x)` is used at two widths. `FST m32` at `0x7be554` stores an `f32`
///   copy and keeps the wide value, so `out[2]` subtracts from the wide one while
///   `out[6]` reloads the narrowed one.
///
/// `sin_cos` here is the shared polynomial and the original is the hardware
/// `FSINCOS`, so the two can never agree to the last bit; that residual is a
/// bounded deviation, separate from the shape facts above.
pub fn c33_matrix__from_axis_angle__7be490(
    axis: &[f32; 3],
    angle: f32,
    normalize: bool,
) -> [f32; 9] {
    let mut x = axis[0];
    let mut y = axis[1];
    let mut z = axis[2];

    if normalize {
        // FLD z, FMUL z, FLD y, FMUL y, FADDP, FLD x, FMUL x, FADDP: the squares
        // are summed z, then y, then x, and the sum stays in the register through
        // the FSQRT and the FDIVR. Only the three scaled components narrow.
        //
        // The order is the original's but nothing can observe it, so no test pins
        // it: over 3.4M random axes the two groupings gave a different sum on 1.2%
        // of them and a different reciprocal on 0.5%, and never once a different
        // scaled component. A one-ulp difference in an f64 reciprocal is ~2^-53
        // relative, far under the 2^-24 granularity of the f32 store that
        // follows, so it survives only on a rounding tie.
        let len2 = (f64::from(z) * f64::from(z) + f64::from(y) * f64::from(y))
            + f64::from(x) * f64::from(x);
        let inv = 1.0_f64 / len2.sqrt();
        x = super::f64_to_f32(f64::from(x) * inv);
        y = super::f64_to_f32(f64::from(y) * inv);
        z = super::f64_to_f32(f64::from(z) * inv);
    }

    let (s, c) = crate::math::trig::sin_cos(angle);

    // Exact: both factors are f32 and 24 + 24 bits fit the register.
    let yx = f64::from(y) * f64::from(x);
    let zy = f64::from(z) * f64::from(y);
    let zx = f64::from(z) * f64::from(x);

    let zs = f64::from(z) * f64::from(s);
    let xs = super::f64_to_f32(f64::from(x) * f64::from(s));
    let ys = super::f64_to_f32(f64::from(y) * f64::from(s));

    let t = f64::from(1.0f32 - c);
    let t_yx = t * yx;
    let t_zy = t * zy;
    let t_zx = t * zx;
    let t_zx_narrowed = super::f64_to_f32(t_zx);

    let diagonal = |v: f32| super::f64_to_f32(f64::from(v) * f64::from(v) * t + f64::from(c));

    [
        diagonal(x),
        super::f64_to_f32(t_yx + zs),
        super::f64_to_f32(t_zx - f64::from(ys)),
        super::f64_to_f32(t_yx - zs),
        diagonal(y),
        super::f64_to_f32(f64::from(xs) + t_zy),
        super::f64_to_f32(f64::from(t_zx_narrowed) + f64::from(ys)),
        super::f64_to_f32(t_zy - f64::from(xs)),
        diagonal(z),
    ]
}

/// Axis/angle pairs carrying full mantissas, so a reassociation lands apart.
///
/// Shared with the two wider entry points' tests. Built from an LCG rather than
/// written out: the shapes being separated differ on most inputs, and what the
/// asserts need is that *some* input separates them.
#[cfg(test)]
pub fn axis_angle_sweep() -> [([f32; 3], f32); 64] {
    // In [1, 2) with every mantissa bit in play, mapped to [-1, 1).
    let spread = |bits: u32| f32::from_bits(0x3f80_0000 | (bits & 0x007f_ffff)) * 2.0 - 3.0;
    let mut out = [([0.0f32; 3], 0.0f32); 64];
    let mut w = 0x3f2c_1d09_u32;
    let mut next = move || {
        w = w.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        w
    };
    for (axis, angle) in &mut out {
        for component in axis.iter_mut() {
            *component = spread(next());
        }
        *angle = spread(next()) * core::f32::consts::PI;
    }
    out
}

#[cfg(test)]
mod tests_c33_matrix__from_axis_angle__7be490 {
    use super::{c33_matrix__from_axis_angle__7be490 as rot, c33_matrix__multiply__7bdfc0 as mul};
    fn approx(a: &[f32; 9], b: &[f32; 9], tol: f32) {
        for i in 0..9 {
            assert!((a[i] - b[i]).abs() < tol, "i={i} {} vs {}", a[i], b[i]);
        }
    }
    #[test]
    fn zero_angle_is_identity() {
        let m = rot(&[0.0, 0.0, 1.0], 0.0, false);
        let i = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        approx(&m, &i, 1e-5);
    }
    #[test]
    fn z_axis_matches_rotz() {
        // about z by 90deg: cos=0,sin=1 -> [[0,1,0],[-1,0,0],[0,0,1]]
        let a = std::f32::consts::FRAC_PI_2;
        let m = rot(&[0.0, 0.0, 1.0], a, false);
        let expect = [0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        approx(&m, &expect, 1e-5);
    }
    #[test]
    fn orthonormal() {
        let m = rot(&[1.0, 2.0, 3.0], 0.7, true);
        for r in 0..3 {
            let len =
                (m[r * 3] * m[r * 3] + m[r * 3 + 1] * m[r * 3 + 1] + m[r * 3 + 2] * m[r * 3 + 2])
                    .sqrt();
            assert!((len - 1.0).abs() < 1e-4, "row {r} len {len}");
        }
        let d = m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6])
            + m[2] * (m[3] * m[7] - m[4] * m[6]);
        assert!((d - 1.0).abs() < 1e-3, "det {d}");
    }
    #[test]
    fn normalize_flag_unit_axis_equivalence() {
        let axis = [0.6, 0.0, 0.8];
        let m1 = rot(&axis, 1.1, false);
        let m2 = rot(&axis, 1.1, true);
        approx(&m1, &m2, 1e-5);
    }
    #[test]
    fn round_trip_inverse() {
        // R(angle) * R(-angle) = I
        let axis = [1.0, 1.0, 0.0];
        let r = rot(&axis, 0.9, true);
        let ri = rot(&axis, -0.9, true);
        let prod = mul(&r, &ri);
        let i = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        approx(&prod, &i, 1e-4);
    }

    /// One deviation from the byte-faithful shape, for the separation asserts.
    ///
    /// Each variant changes exactly one reading of the bytes, so an input that
    /// separates it separates that reading and nothing else.
    enum Deviation {
        /// Every intermediate rounded to `f32`, `1 - cos` folded in left to right.
        PerStepF32,
        /// `z * sin` narrowed like its `x` and `y` siblings.
        ZSinNarrowed,
        /// `out[6]` built from the wide `t * (z * x)`, not the stored `f32` copy.
        WideZxInRow2,
    }

    fn deviate(axis: &[f32; 3], angle: f32, deviation: &Deviation) -> [f32; 9] {
        let narrow = super::super::f64_to_f32;
        let (mut x, mut y, mut z) = (axis[0], axis[1], axis[2]);

        if matches!(deviation, Deviation::PerStepF32) {
            let inv = 1.0 / (x * x + y * y + z * z).sqrt();
            x *= inv;
            y *= inv;
            z *= inv;
            let (s, c) = crate::math::trig::sin_cos(angle);
            let t = 1.0 - c;
            let xy = t * y * x;
            let xz = t * z * x;
            let yz = t * z * y;
            return [
                x * x * t + c,
                xy + z * s,
                xz - y * s,
                xy - z * s,
                y * y * t + c,
                x * s + yz,
                xz + y * s,
                yz - x * s,
                z * z * t + c,
            ];
        }

        let len2 = (f64::from(z) * f64::from(z) + f64::from(y) * f64::from(y))
            + f64::from(x) * f64::from(x);
        let inv = 1.0_f64 / len2.sqrt();
        x = narrow(f64::from(x) * inv);
        y = narrow(f64::from(y) * inv);
        z = narrow(f64::from(z) * inv);

        let (s, c) = crate::math::trig::sin_cos(angle);
        let t = f64::from(1.0f32 - c);
        let zs = if matches!(deviation, Deviation::ZSinNarrowed) {
            f64::from(narrow(f64::from(z) * f64::from(s)))
        } else {
            f64::from(z) * f64::from(s)
        };
        let xs = narrow(f64::from(x) * f64::from(s));
        let ys = narrow(f64::from(y) * f64::from(s));
        let t_yx = t * (f64::from(y) * f64::from(x));
        let t_zy = t * (f64::from(z) * f64::from(y));
        let t_zx = t * (f64::from(z) * f64::from(x));
        let zx_for_row2 = if matches!(deviation, Deviation::WideZxInRow2) {
            t_zx
        } else {
            f64::from(narrow(t_zx))
        };
        let diagonal = |v: f32| narrow(f64::from(v) * f64::from(v) * t + f64::from(c));

        [
            diagonal(x),
            narrow(t_yx + zs),
            narrow(t_zx - f64::from(ys)),
            narrow(t_yx - zs),
            diagonal(y),
            narrow(f64::from(xs) + t_zy),
            narrow(zx_for_row2 + f64::from(ys)),
            narrow(t_zy - f64::from(xs)),
            diagonal(z),
        ]
    }

    fn separating_count(deviation: &Deviation) -> usize {
        super::axis_angle_sweep()
            .iter()
            .filter(|(axis, angle)| {
                let faithful = rot(axis, *angle, true);
                let other = deviate(axis, *angle, deviation);
                (0..9).any(|i| faithful[i].to_bits() != other[i].to_bits())
            })
            .count()
    }

    #[test]
    fn separates_from_the_per_step_f32_form() {
        // Without this the width half of the shape is unpinned: every tolerance
        // test above passes under either form.
        assert!(
            separating_count(&Deviation::PerStepF32) > 0,
            "sweep no longer separates narrow-once from narrow-per-step"
        );
    }

    #[test]
    fn separates_from_a_narrowed_z_sin() {
        assert!(
            separating_count(&Deviation::ZSinNarrowed) > 0,
            "sweep no longer separates the wide z * sin from a narrowed one"
        );
    }

    #[test]
    fn separates_from_a_wide_zx_in_row_two() {
        assert!(
            separating_count(&Deviation::WideZxInRow2) > 0,
            "sweep no longer separates out[6]'s stored f32 copy from the wide one"
        );
    }
}

/// Builds a 3x3 rotation-about-Z matrix from `angle`.
///
/// `[[cos,sin,0],[-sin,cos,0],[0,0,1]]` (row-major).
pub fn c33_matrix__from_rotation_z__7be5b0(angle: f32) -> [f32; 9] {
    let (s, c) = crate::math::trig::sin_cos(angle);
    [c, s, 0.0, -s, c, 0.0, 0.0, 0.0, 1.0]
}

#[cfg(test)]
mod tests_c33_matrix__from_rotation_z__7be5b0 {
    use super::c33_matrix__from_rotation_z__7be5b0 as rotz;
    #[test]
    fn zero_is_identity() {
        let m = rotz(0.0);
        let i = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        for k in 0..9 {
            assert!((m[k] - i[k]).abs() < 5e-6, "k={k}");
        }
    }
    #[test]
    fn ninety_deg() {
        let m = rotz(std::f32::consts::FRAC_PI_2);
        let expect = [0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        for k in 0..9 {
            assert!((m[k] - expect[k]).abs() < 1e-5, "k={k} {}", m[k]);
        }
    }
    #[test]
    fn orthonormal_rows() {
        let m = rotz(1.234);
        for r in 0..3 {
            let len =
                (m[r * 3] * m[r * 3] + m[r * 3 + 1] * m[r * 3 + 1] + m[r * 3 + 2] * m[r * 3 + 2])
                    .sqrt();
            assert!((len - 1.0).abs() < 1e-5, "row {r}");
        }
        assert_eq!(m[8].to_bits(), 1.0f32.to_bits());
    }
}

/// Per-element `k` summation order of `C33Matrix::Multiply`, indexed `r * 3 + c`.
///
/// As in the 4x4 product, the original is unrolled and its nine three-term dots
/// do not share one `k` order — there are four, read off the
/// `FLD`/`FMUL`/`FADDP` stream from `0x7bdfc4` to `0x7be09b`. Transcribed, not
/// derived.
const MULTIPLY_K_ORDER: [[usize; 3]; 9] = [
    [2, 1, 0],
    [0, 2, 1],
    [2, 0, 1],
    [0, 2, 1],
    [2, 1, 0],
    [2, 0, 1],
    [2, 1, 0],
    [1, 0, 2],
    [1, 0, 2],
];

/// 3x3 * 3x3 matrix product `out = a * b`, row-major: `out[r*3+c] = sum_k a[r*3+k]*b[k*3+c]`.
///
/// Each element sums its three products in that element's own
/// [`MULTIPLY_K_ORDER`], starting from the first product rather than a zero
/// seed, and narrows once. The original keeps seven of the nine elements on the
/// x87 stack until a run of trailing `FSTP m32` at `0x7be09d` and spills the
/// other two early (`0x7be061`, `0x7be079`) — different moments, but one
/// narrowing each either way.
pub fn c33_matrix__multiply__7bdfc0(a: &[f32; 9], b: &[f32; 9]) -> [f32; 9] {
    let mut out = [0.0f32; 9];
    for r in 0..3 {
        for c in 0..3 {
            let ks = MULTIPLY_K_ORDER[r * 3 + c];
            let term = |k: usize| f64::from(a[r * 3 + k]) * f64::from(b[k * 3 + c]);
            let mut acc = term(ks[0]);
            acc += term(ks[1]);
            acc += term(ks[2]);
            out[r * 3 + c] = super::f64_to_f32(acc);
        }
    }
    out
}

#[cfg(test)]
mod tests_c33_matrix__multiply__7bdfc0 {
    use super::c33_matrix__multiply__7bdfc0 as mul;

    #[test]
    fn per_element_order_and_single_narrowing() {
        // Six of the nine elements move between the original's per-element
        // orders at register width and the uniform k = 0..2 per-step f32 form
        // this used to compute. Bit patterns, not decimals.
        let a = [
            f32::from_bits(0xc03a_62ea),
            f32::from_bits(0xbda2_cc0d),
            f32::from_bits(0x4032_85af),
            f32::from_bits(0xc027_3545),
            f32::from_bits(0x3e7c_7221),
            f32::from_bits(0xbe51_84ee),
            f32::from_bits(0x3f1b_d90d),
            f32::from_bits(0xc01d_d9f1),
            f32::from_bits(0x3ef2_b23e),
        ];
        let b = [
            f32::from_bits(0xbfb0_f555),
            f32::from_bits(0x3ead_5c5f),
            f32::from_bits(0x3f5e_287f),
            f32::from_bits(0xbde9_066b),
            f32::from_bits(0xbf5e_5a49),
            f32::from_bits(0xbfc0_a6b3),
            f32::from_bits(0x4026_784f),
            f32::from_bits(0xbe8f_3127),
            f32::from_bits(0x3e39_4f7a),
        ];
        let got = mul(&a, &b);

        let mut moved = 0;
        for r in 0..3 {
            for c in 0..3 {
                let ks = super::MULTIPLY_K_ORDER[r * 3 + c];
                let term = |k: usize| f64::from(a[r * 3 + k]) * f64::from(b[k * 3 + c]);
                let mut acc = term(ks[0]);
                acc += term(ks[1]);
                acc += term(ks[2]);
                let want = super::super::f64_to_f32(acc);
                assert_eq!(got[r * 3 + c].to_bits(), want.to_bits(), "element {r},{c}");

                let uniform = (a[r * 3] * b[c] + a[r * 3 + 1] * b[3 + c]) + a[r * 3 + 2] * b[6 + c];
                if got[r * 3 + c].to_bits() != uniform.to_bits() {
                    moved += 1;
                }
            }
        }
        assert!(
            moved >= 5,
            "fixture no longer separates the orders (only {moved} elements move)"
        );
    }
    const I: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    #[test]
    fn a_times_identity() {
        let a = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
        let out = mul(&a, &I);
        for i in 0..9 {
            assert_eq!(out[i].to_bits(), a[i].to_bits());
        }
        let out2 = mul(&I, &a);
        for i in 0..9 {
            assert_eq!(out2[i].to_bits(), a[i].to_bits());
        }
    }
    #[test]
    fn known_product() {
        let a = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
        let b = [9.0, 8.0, 7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0];
        // row0: [1*9+2*6+3*3, 1*8+2*5+3*2, 1*7+2*4+3*1] = [30,24,18]
        let out = mul(&a, &b);
        let expect = [30.0, 24.0, 18.0, 84.0, 69.0, 54.0, 138.0, 114.0, 90.0];
        for i in 0..9 {
            assert!(
                (out[i] - expect[i]).abs() < 1e-3,
                "i={i} {} vs {}",
                out[i],
                expect[i]
            );
        }
    }
    #[test]
    fn associativity() {
        let a = [2.0, -1.0, 0.0, 1.0, 3.0, 2.0, 0.0, 1.0, 1.0];
        let b = [1.0, 0.0, 2.0, -1.0, 1.0, 0.0, 3.0, 2.0, 1.0];
        let c = [0.0, 1.0, -1.0, 2.0, 0.0, 1.0, 1.0, 1.0, 0.0];
        let lhs = mul(&mul(&a, &b), &c);
        let rhs = mul(&a, &mul(&b, &c));
        for i in 0..9 {
            assert!((lhs[i] - rhs[i]).abs() < 1e-3, "i={i}");
        }
    }
}

/// Multiplies all 9 elements of a 3x3 matrix by a scalar: `out[i] = m[i] * scale`.
pub fn c33_matrix__scale_by_scalar__672d80(m: &[f32; 9], scale: f32) -> [f32; 9] {
    let mut out = [m[0] * scale; 9];
    let mut i = 0;
    while i < 9 {
        out[i] = m[i] * scale;
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests_c33_matrix__scale_by_scalar__672d80 {
    use super::c33_matrix__scale_by_scalar__672d80 as f;
    #[test]
    fn scale_known() {
        let m = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
        let out = f(&m, 2.0);
        for i in 0..9 {
            assert_eq!(out[i].to_bits(), (m[i] * 2.0).to_bits());
        }
    }
    #[test]
    fn scale_identity() {
        let m = [1.5, -2.5, 3.0, 0.0, 9.0, -1.0, 4.0, 5.0, 6.0];
        let out = f(&m, 1.0);
        for i in 0..9 {
            assert_eq!(out[i].to_bits(), m[i].to_bits());
        }
    }
    #[test]
    fn scale_zero() {
        let m = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
        let out = f(&m, 0.0);
        for v in out {
            assert_eq!(v, 0.0);
        }
    }
}

/// Scales each row of a 3x3 matrix in place by the matching component of a 3-vector.
///
/// Row `i` (3 consecutive floats) is multiplied by `scale[i]`. Returns row-major 9 floats.
pub fn c33_matrix__scale_rows__7be6d0(m: &[f32; 9], scale: &[f32; 3]) -> [f32; 9] {
    let mut out = [0.0f32; 9];
    for r in 0..3 {
        let s = scale[r];
        out[r * 3] = s * m[r * 3];
        out[r * 3 + 1] = s * m[r * 3 + 1];
        out[r * 3 + 2] = s * m[r * 3 + 2];
    }
    out
}

#[cfg(test)]
mod tests_c33_matrix__scale_rows__7be6d0 {
    use super::c33_matrix__scale_rows__7be6d0 as sr;
    #[test]
    fn known() {
        let m = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
        let out = sr(&m, &[2.0, 3.0, 0.0]);
        let expect: [f32; 9] = [2.0, 4.0, 6.0, 12.0, 15.0, 18.0, 0.0, 0.0, 0.0];
        for i in 0..9 {
            assert_eq!(out[i].to_bits(), expect[i].to_bits(), "i={i}");
        }
    }
    #[test]
    fn ones_is_identity() {
        let m = [1.5, -2.0, 3.0, 0.0, 9.0, -1.0, 4.0, 5.0, 6.0];
        let out = sr(&m, &[1.0, 1.0, 1.0]);
        for i in 0..9 {
            assert_eq!(out[i].to_bits(), m[i].to_bits());
        }
    }
}

/// `C33Matrix::FromEulerAngles` — stack args in order `(angle_z, angle_y, angle_x)`.
///
/// Builds the per-axis rotations `Z(z)`, `Y(y)`, `X(x)`, forms the product
/// `Z · Y · X`, and stores its transpose — exactly the construction the 1.12
/// client emits (two 3x3 multiplies then a column-major copy). The first stack
/// arg drives the Z rotation, the third drives X.
pub fn c33_matrix__from_euler_angles__7bf4b0(angle_z: f32, angle_y: f32, angle_x: f32) -> [f32; 9] {
    let (sz, cz) = crate::math::trig::sin_cos(angle_z);
    let (sy, cy) = crate::math::trig::sin_cos(angle_y);
    let (sx, cx) = crate::math::trig::sin_cos(angle_x);
    let mz = [cz, -sz, 0.0, sz, cz, 0.0, 0.0, 0.0, 1.0];
    let my = [cy, 0.0, sy, 0.0, 1.0, 0.0, -sy, 0.0, cy];
    let mx = [1.0, 0.0, 0.0, 0.0, cx, -sx, 0.0, sx, cx];
    transpose3(&mul3(&mz, &mul3(&my, &mx)))
}

#[cfg(test)]
mod tests_c33_matrix__from_euler_angles__7bf4b0 {
    use super::c33_matrix__from_euler_angles__7bf4b0 as f;
    fn eq9(m: &[f32; 9], exp: &[f32; 9]) {
        for i in 0..9 {
            assert!(
                (m[i] - exp[i]).abs() < 5e-6,
                "idx {i}: {} vs {}",
                m[i],
                exp[i]
            );
        }
    }
    #[test]
    fn first_arg_is_transposed_z() {
        let a = 0.5f32;
        let (c, s) = (a.cos(), a.sin());
        eq9(&f(a, 0.0, 0.0), &[c, s, 0.0, -s, c, 0.0, 0.0, 0.0, 1.0]);
    }
    #[test]
    fn second_arg_is_transposed_y() {
        let a = 0.5f32;
        let (c, s) = (a.cos(), a.sin());
        eq9(&f(0.0, a, 0.0), &[c, 0.0, -s, 0.0, 1.0, 0.0, s, 0.0, c]);
    }
    #[test]
    fn third_arg_is_transposed_x() {
        let a = 0.5f32;
        let (c, s) = (a.cos(), a.sin());
        eq9(&f(0.0, 0.0, a), &[1.0, 0.0, 0.0, 0.0, c, s, 0.0, -s, c]);
    }
    #[test]
    fn z_and_x_compose_in_order() {
        // Two non-identity axes catch a swapped multiply order; `Y = I`, so the
        // result is `transpose(Z · X)`, hand-derived below.
        let (z, x) = (0.4f32, 0.9f32);
        let (cz, sz) = (z.cos(), z.sin());
        let (cx, sx) = (x.cos(), x.sin());
        eq9(
            &f(z, 0.0, x),
            &[cz, sz, 0.0, -sz * cx, cz * cx, sx, sz * sx, -cz * sx, cx],
        );
    }
    #[test]
    fn rows_orthonormal() {
        let m = f(0.3, -0.7, 1.1);
        for i in 0..3 {
            for j in 0..3 {
                let dot =
                    m[i * 3] * m[j * 3] + m[i * 3 + 1] * m[j * 3 + 1] + m[i * 3 + 2] * m[j * 3 + 2];
                let want = if i == j { 1.0 } else { 0.0 };
                assert!((dot - want).abs() < 1e-5, "row {i}·{j}");
            }
        }
    }
}

/// 3x3 rotation matrix from Euler angles, intrinsic axis order XYZ (row-major 9 floats).
pub fn c33_matrix__from_euler_xyz__7bed30(angle_x: f32, angle_y: f32, angle_z: f32) -> [f32; 9] {
    let (sx, cx) = crate::math::trig::sin_cos(angle_x);
    let (sy, cy) = crate::math::trig::sin_cos(angle_y);
    let (sz, cz) = crate::math::trig::sin_cos(angle_z);
    [
        cy * cz,
        -cx * sz - sx * sy * cz,
        -cx * sy * cz + sx * sz,
        cy * sz,
        cx * cz - sx * sy * sz,
        -cx * sy * sz - sx * cz,
        sy,
        sx * cy,
        cx * cy,
    ]
}

#[cfg(test)]
mod tests_c33_matrix__from_euler_xyz__7bed30 {
    use super::c33_matrix__from_euler_xyz__7bed30 as f;
    fn mul(a: &[f32; 9], b: &[f32; 9]) -> [f32; 9] {
        let mut out = [0.0f32; 9];
        for r in 0..3 {
            for c in 0..3 {
                out[r * 3 + c] =
                    a[r * 3] * b[c] + a[r * 3 + 1] * b[3 + c] + a[r * 3 + 2] * b[6 + c];
            }
        }
        out
    }
    fn transpose(m: &[f32; 9]) -> [f32; 9] {
        let mut t = [0.0f32; 9];
        for i in 0..9 {
            t[i] = m[(i % 3) * 3 + i / 3];
        }
        t
    }
    // Literal host construction for XYZ: transpose(b1 * (b2 * b3)) using the captured per-axis layouts.
    fn reference(ax: f32, ay: f32, az: f32) -> [f32; 9] {
        let (cx, sx) = (ax.cos(), ax.sin());
        let (cy, sy) = (ay.cos(), ay.sin());
        let (cz, sz) = (az.cos(), az.sin());
        let b1 = [1.0, 0.0, 0.0, 0.0, cx, sx, 0.0, -sx, cx];
        let b2 = [cy, 0.0, sy, 0.0, 1.0, 0.0, -sy, 0.0, cy];
        let b3 = [cz, sz, 0.0, -sz, cz, 0.0, 0.0, 0.0, 1.0];
        transpose(&mul(&b1, &mul(&b2, &b3)))
    }
    const ANGLES: [(f32, f32, f32); 6] = [
        (0.0, 0.0, 0.0),
        (0.31, -0.72, 1.13),
        (1.7, 0.4, -0.9),
        (-2.1, 1.05, 0.33),
        (2.0, -2.0, 2.0),
        (3.0, 1.0, -1.5),
    ];
    #[test]
    fn matches_literal_reference() {
        for &(ax, ay, az) in &ANGLES {
            let got = f(ax, ay, az);
            let exp = reference(ax, ay, az);
            for i in 0..9 {
                assert!(
                    (got[i] - exp[i]).abs() < 1e-5,
                    "i={i} {} vs {}",
                    got[i],
                    exp[i]
                );
            }
        }
    }
    #[test]
    fn orthonormal() {
        for &(ax, ay, az) in &ANGLES {
            let m = f(ax, ay, az);
            let p = mul(&m, &transpose(&m));
            let id = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
            for i in 0..9 {
                assert!((p[i] - id[i]).abs() < 2e-5, "i={i}");
            }
        }
    }
    #[test]
    fn zero_is_identity() {
        let m = f(0.0, 0.0, 0.0);
        let id = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        for i in 0..9 {
            assert!((m[i] - id[i]).abs() < 5e-6, "i={i}");
        }
    }
    #[test]
    fn x_only_is_transposed_rx() {
        let a = 0.5f32;
        let (c, s) = (a.cos(), a.sin());
        let m = f(a, 0.0, 0.0);
        let exp = [1.0, 0.0, 0.0, 0.0, c, -s, 0.0, s, c];
        for i in 0..9 {
            assert!((m[i] - exp[i]).abs() < 5e-6, "i={i}");
        }
    }
}

/// 3x3 rotation matrix from Euler angles, intrinsic axis order XZY (row-major 9 floats).
pub fn c33_matrix__from_euler_xzy__7beeb0(angle_x: f32, angle_y: f32, angle_z: f32) -> [f32; 9] {
    let (sx, cx) = crate::math::trig::sin_cos(angle_x);
    let (sy, cy) = crate::math::trig::sin_cos(angle_y);
    let (sz, cz) = crate::math::trig::sin_cos(angle_z);
    [
        cy * cz,
        cx * sy * cz - sx * sz,
        -cx * sz - sx * sy * cz,
        -sy,
        cx * cy,
        -sx * cy,
        cy * sz,
        cx * sy * sz + sx * cz,
        cx * cz - sx * sy * sz,
    ]
}

#[cfg(test)]
mod tests_c33_matrix__from_euler_xzy__7beeb0 {
    use super::c33_matrix__from_euler_xzy__7beeb0 as f;
    fn mul(a: &[f32; 9], b: &[f32; 9]) -> [f32; 9] {
        let mut out = [0.0f32; 9];
        for r in 0..3 {
            for c in 0..3 {
                out[r * 3 + c] =
                    a[r * 3] * b[c] + a[r * 3 + 1] * b[3 + c] + a[r * 3 + 2] * b[6 + c];
            }
        }
        out
    }
    fn transpose(m: &[f32; 9]) -> [f32; 9] {
        let mut t = [0.0f32; 9];
        for i in 0..9 {
            t[i] = m[(i % 3) * 3 + i / 3];
        }
        t
    }
    fn reference(ax: f32, ay: f32, az: f32) -> [f32; 9] {
        let (cx, sx) = (ax.cos(), ax.sin());
        let (cy, sy) = (ay.cos(), ay.sin());
        let (cz, sz) = (az.cos(), az.sin());
        let b1 = [1.0, 0.0, 0.0, 0.0, cx, sx, 0.0, -sx, cx];
        let b2 = [cy, -sy, 0.0, sy, cy, 0.0, 0.0, 0.0, 1.0];
        let b3 = [cz, 0.0, sz, 0.0, 1.0, 0.0, -sz, 0.0, cz];
        transpose(&mul(&b1, &mul(&b2, &b3)))
    }
    const ANGLES: [(f32, f32, f32); 6] = [
        (0.0, 0.0, 0.0),
        (0.31, -0.72, 1.13),
        (1.7, 0.4, -0.9),
        (-2.1, 1.05, 0.33),
        (2.0, -2.0, 2.0),
        (3.0, 1.0, -1.5),
    ];
    #[test]
    fn matches_literal_reference() {
        for &(ax, ay, az) in &ANGLES {
            let got = f(ax, ay, az);
            let exp = reference(ax, ay, az);
            for i in 0..9 {
                assert!(
                    (got[i] - exp[i]).abs() < 1e-5,
                    "i={i} {} vs {}",
                    got[i],
                    exp[i]
                );
            }
        }
    }
    #[test]
    fn orthonormal() {
        for &(ax, ay, az) in &ANGLES {
            let m = f(ax, ay, az);
            let p = mul(&m, &transpose(&m));
            let id = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
            for i in 0..9 {
                assert!((p[i] - id[i]).abs() < 2e-5, "i={i}");
            }
        }
    }
    #[test]
    fn zero_is_identity() {
        let m = f(0.0, 0.0, 0.0);
        let id = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        for i in 0..9 {
            assert!((m[i] - id[i]).abs() < 5e-6, "i={i}");
        }
    }
    #[test]
    fn x_only_is_transposed_rx() {
        let a = 0.5f32;
        let (c, s) = (a.cos(), a.sin());
        let m = f(a, 0.0, 0.0);
        let exp = [1.0, 0.0, 0.0, 0.0, c, -s, 0.0, s, c];
        for i in 0..9 {
            assert!((m[i] - exp[i]).abs() < 5e-6, "i={i}");
        }
    }
}

/// `C33Matrix::from_euler_yxz` — Euler order YXZ: stores `transpose(Rz · Rx · Ry)`.
pub fn c33_matrix__from_euler_yxz__7bf030(angle_x: f32, angle_y: f32, angle_z: f32) -> [f32; 9] {
    let (sx, cx) = crate::math::trig::sin_cos(angle_x);
    let (sy, cy) = crate::math::trig::sin_cos(angle_y);
    let (sz, cz) = crate::math::trig::sin_cos(angle_z);
    let rx = rx3(cx, sx);
    let ry = ry3(cy, sy);
    let rz = rz3(cz, sz);
    transpose3(&mul3(&rz, &mul3(&rx, &ry)))
}

#[cfg(test)]
mod tests_c33_matrix__from_euler_yxz__7bf030 {
    use super::c33_matrix__from_euler_yxz__7bf030 as f;
    fn eq9(m: &[f32; 9], exp: &[f32; 9]) {
        for i in 0..9 {
            assert!(
                (m[i] - exp[i]).abs() < 5e-6,
                "idx {i}: {} vs {}",
                m[i],
                exp[i]
            );
        }
    }
    #[test]
    fn x_only_is_transposed_rx() {
        let a = 0.5f32;
        let (c, s) = (a.cos(), a.sin());
        eq9(&f(a, 0.0, 0.0), &[1.0, 0.0, 0.0, 0.0, c, -s, 0.0, s, c]);
    }
    #[test]
    fn y_only_is_transposed_ry() {
        let a = 0.5f32;
        let (c, s) = (a.cos(), a.sin());
        eq9(&f(0.0, a, 0.0), &[c, 0.0, -s, 0.0, 1.0, 0.0, s, 0.0, c]);
    }
    #[test]
    fn z_only_is_transposed_rz() {
        let a = 0.5f32;
        let (c, s) = (a.cos(), a.sin());
        eq9(&f(0.0, 0.0, a), &[c, -s, 0.0, s, c, 0.0, 0.0, 0.0, 1.0]);
    }
    #[test]
    fn rows_orthonormal() {
        let m = f(0.3, -0.7, 1.1);
        for i in 0..3 {
            for j in 0..3 {
                let dot =
                    m[i * 3] * m[j * 3] + m[i * 3 + 1] * m[j * 3 + 1] + m[i * 3 + 2] * m[j * 3 + 2];
                let want = if i == j { 1.0 } else { 0.0 };
                assert!((dot - want).abs() < 1e-5, "row {i}·{j}");
            }
        }
    }
}

/// `C33Matrix::from_euler_yzx` — Euler order YZX: stores `transpose(Rx · Rz · Ry)`.
pub fn c33_matrix__from_euler_yzx__7bf1b0(angle_x: f32, angle_y: f32, angle_z: f32) -> [f32; 9] {
    let (sx, cx) = crate::math::trig::sin_cos(angle_x);
    let (sy, cy) = crate::math::trig::sin_cos(angle_y);
    let (sz, cz) = crate::math::trig::sin_cos(angle_z);
    let rx = rx3(cx, sx);
    let ry = ry3(cy, sy);
    let rz = rz3(cz, sz);
    transpose3(&mul3(&rx, &mul3(&rz, &ry)))
}

#[cfg(test)]
mod tests_c33_matrix__from_euler_yzx__7bf1b0 {
    use super::c33_matrix__from_euler_yzx__7bf1b0 as f;
    fn eq9(m: &[f32; 9], exp: &[f32; 9]) {
        for i in 0..9 {
            assert!(
                (m[i] - exp[i]).abs() < 5e-6,
                "idx {i}: {} vs {}",
                m[i],
                exp[i]
            );
        }
    }
    #[test]
    fn x_only_is_transposed_rx() {
        let a = 0.5f32;
        let (c, s) = (a.cos(), a.sin());
        eq9(&f(a, 0.0, 0.0), &[1.0, 0.0, 0.0, 0.0, c, -s, 0.0, s, c]);
    }
    #[test]
    fn y_only_is_transposed_ry() {
        let a = 0.5f32;
        let (c, s) = (a.cos(), a.sin());
        eq9(&f(0.0, a, 0.0), &[c, 0.0, -s, 0.0, 1.0, 0.0, s, 0.0, c]);
    }
    #[test]
    fn z_only_is_transposed_rz() {
        let a = 0.5f32;
        let (c, s) = (a.cos(), a.sin());
        eq9(&f(0.0, 0.0, a), &[c, -s, 0.0, s, c, 0.0, 0.0, 0.0, 1.0]);
    }
    #[test]
    fn rows_orthonormal() {
        let m = f(0.3, -0.7, 1.1);
        for i in 0..3 {
            for j in 0..3 {
                let dot =
                    m[i * 3] * m[j * 3] + m[i * 3 + 1] * m[j * 3 + 1] + m[i * 3 + 2] * m[j * 3 + 2];
                let want = if i == j { 1.0 } else { 0.0 };
                assert!((dot - want).abs() < 1e-5, "row {i}·{j}");
            }
        }
    }
}

/// `C33Matrix::from_euler_zxy` — Euler order ZXY: stores `transpose(Ry · Rx · Rz)`.
pub fn c33_matrix__from_euler_zxy__7bf330(angle_x: f32, angle_y: f32, angle_z: f32) -> [f32; 9] {
    let (sx, cx) = crate::math::trig::sin_cos(angle_x);
    let (sy, cy) = crate::math::trig::sin_cos(angle_y);
    let (sz, cz) = crate::math::trig::sin_cos(angle_z);
    let rx = rx3(cx, sx);
    let ry = ry3(cy, sy);
    let rz = rz3(cz, sz);
    transpose3(&mul3(&ry, &mul3(&rx, &rz)))
}

#[cfg(test)]
mod tests_c33_matrix__from_euler_zxy__7bf330 {
    use super::c33_matrix__from_euler_zxy__7bf330 as f;
    fn eq9(m: &[f32; 9], exp: &[f32; 9]) {
        for i in 0..9 {
            assert!(
                (m[i] - exp[i]).abs() < 5e-6,
                "idx {i}: {} vs {}",
                m[i],
                exp[i]
            );
        }
    }
    #[test]
    fn x_only_is_transposed_rx() {
        let a = 0.5f32;
        let (c, s) = (a.cos(), a.sin());
        eq9(&f(a, 0.0, 0.0), &[1.0, 0.0, 0.0, 0.0, c, -s, 0.0, s, c]);
    }
    #[test]
    fn y_only_is_transposed_ry() {
        let a = 0.5f32;
        let (c, s) = (a.cos(), a.sin());
        eq9(&f(0.0, a, 0.0), &[c, 0.0, -s, 0.0, 1.0, 0.0, s, 0.0, c]);
    }
    #[test]
    fn z_only_is_transposed_rz() {
        let a = 0.5f32;
        let (c, s) = (a.cos(), a.sin());
        eq9(&f(0.0, 0.0, a), &[c, -s, 0.0, s, c, 0.0, 0.0, 0.0, 1.0]);
    }
    #[test]
    fn rows_orthonormal() {
        let m = f(0.3, -0.7, 1.1);
        for i in 0..3 {
            for j in 0..3 {
                let dot =
                    m[i * 3] * m[j * 3] + m[i * 3 + 1] * m[j * 3 + 1] + m[i * 3 + 2] * m[j * 3 + 2];
                let want = if i == j { 1.0 } else { 0.0 };
                assert!((dot - want).abs() < 1e-5, "row {i}·{j}");
            }
        }
    }
}

/// `C33Matrix::SetTransposed`.
///
/// The pure transpose the x87 body performs before it forwards to the setter at
/// `0x7c0190`. Reads `m` as a row-major 3x3 (`m[3*r + c]`) and returns its
/// transpose `t[3*r + c] = m[3*c + r]`. This is a straight memory shuffle — no
/// arithmetic, hence no float compares and no NaN polarity to reproduce (any NaN
/// lane is copied through bit-for-bit).
pub fn c33_matrix__set_transposed__7c0120(m: &[f32; 9]) -> [f32; 9] {
    [
        m[0], m[3], m[6], // row 0 = source column 0
        m[1], m[4], m[7], // row 1 = source column 1
        m[2], m[5], m[8], // row 2 = source column 2
    ]
}

#[cfg(test)]
mod tests_c33_matrix__set_transposed__7c0120 {
    use super::c33_matrix__set_transposed__7c0120 as t;

    #[test]
    fn matches_declared_lane_map() {
        // Exact L[k]=p[?] mapping from the x87 body (0x7c0129–0x7c015e):
        // L0=p0 L1=p3 L2=p6 L3=p1 L4=p4 L5=p7 L6=p2 L7=p5 L8=p8.
        let p = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        assert_eq!(t(&p), [0.0, 3.0, 6.0, 1.0, 4.0, 7.0, 2.0, 5.0, 8.0]);
    }

    #[test]
    fn transpose_oracle_index_swap() {
        // Independent oracle: t[r][c] must equal m[c][r] for every (r,c).
        let m = [1.5, -2.0, 3.25, 0.0, 9.0, -1.0, 4.0, 5.5, -6.0];
        let out = t(&m);
        for r in 0..3 {
            for c in 0..3 {
                assert_eq!(
                    out[3 * r + c].to_bits(),
                    m[3 * c + r].to_bits(),
                    "r={r} c={c}"
                );
            }
        }
    }

    #[test]
    fn transpose_is_involution() {
        let m = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
        assert_eq!(t(&t(&m)), m);
    }

    #[test]
    fn symmetric_matrix_is_unchanged() {
        // Diagonal-symmetric input transposes to itself.
        let m = [1.0, 2.0, 3.0, 2.0, 4.0, 5.0, 3.0, 5.0, 6.0];
        assert_eq!(t(&m), m);
    }

    #[test]
    fn nan_lane_copies_through() {
        let mut m = [0.0f32; 9];
        m[5] = f32::NAN; // source (r=1,c=2) -> transposed index 3*2+1 = 7.
        let out = t(&m);
        assert!(out[7].is_nan());
        for (i, &v) in out.iter().enumerate() {
            if i != 7 {
                assert_eq!(v, 0.0);
            }
        }
    }
}
