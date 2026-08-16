//! `matrix33` family kernels.
// Adapter and kernel names mirror the host's C++ symbols verbatim, with `__`
// standing in for the `::`, so the whole module is non-snake-case by construction.
#![allow(non_snake_case)]

// --- Euler composition helpers ---
//
// All six Euler builders share one body: three inline `FSINCOS`, three
// single-axis matrices written to stack slots, two `call C33Matrix::Multiply`,
// and a nine-argument transposing copy. The three matrices below are the ones
// the originals actually store, read off the `FSTP m32` stream rather than
// inferred from a convention. For the ZYX entry at 0x7bf4b0 the Z block writes
// `cos` at 0x7bf4db, `-sin` after the `FCHS` at 0x7bf4e9, `sin` at 0x7bf4f6 and
// `cos` at 0x7bf503 into consecutive slots, giving [c,-s,0, s,c,0, 0,0,1], and
// the five siblings write the same three layouts. The X and Z helpers this
// replaces were the TRANSPOSE of these, which is one of the two ways five of
// the six builders came out a whole sign wrong on their off-diagonal lanes; the
// other was reading argument 0 as the X angle in entries whose first FSINCOS
// feeds a Y or Z matrix.
fn euler_x3(c: f32, s: f32) -> [f32; 9] {
    [1.0, 0.0, 0.0, 0.0, c, -s, 0.0, s, c]
}
fn euler_y3(c: f32, s: f32) -> [f32; 9] {
    [c, 0.0, s, 0.0, 1.0, 0.0, -s, 0.0, c]
}
fn euler_z3(c: f32, s: f32) -> [f32; 9] {
    [c, -s, 0.0, s, c, 0.0, 0.0, 0.0, 1.0]
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

/// The composition every Euler builder ends in: `transpose(m1 * (m2 * m3))`.
///
/// The grouping is not a choice. The first `call` takes the second and third
/// matrices (`edx` = the second slot, the stack argument = the third) and the
/// second `call` takes the first against that result, so the product is
/// right-associated; the nine arguments then handed to the transposing copy at
/// 0x5f8d20 are pushed in the order that reads the result column by column.
/// `m1`, `m2`, `m3` are the axes in the entry's NAME order, each built from the
/// stack argument in the same position.
fn euler3(m1: &[f32; 9], m2: &[f32; 9], m3: &[f32; 9]) -> [f32; 9] {
    transpose3(&mul3(m1, &mul3(m2, m3)))
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

/// The by-value form of `c33_matrix__from_axis_angle__7be490_into`.
///
/// For the callers that place the nine floats somewhere other than a bare 3x3:
/// `0x7bb860`, `0x7bdb00` and `0x7c5490`'s turn-toward-target forward. It holds no
/// arithmetic of its own, and the return slot the caller supplied is what the
/// kernel writes into, so the by-value shape costs no copy.
pub fn c33_matrix__from_axis_angle__7be490(
    axis: &[f32; 3],
    angle: f32,
    normalize: bool,
) -> [f32; 9] {
    let mut m = core::mem::MaybeUninit::<[f32; 9]>::uninit();
    c33_matrix__from_axis_angle__7be490_into(&mut m, axis, angle, normalize);
    // SAFETY: `_into` writes all nine floats through `out` on every path: its one
    // store is unconditional and covers the whole array.
    unsafe { m.assume_init() }
}

/// Builds a 3x3 rotation matrix (Rodrigues) of `angle` radians about `axis`.
///
/// When `normalize` is true the axis is normalized first (the original's flag runs
/// the other way: a non-zero byte *skips* it). Writes row-major 9 floats.
///
/// The nine floats land in a caller-supplied slot rather than being returned: a
/// 36-byte return travels by hidden pointer, so the installed adapter at `0x7be490`
/// would have to copy them out of the callee's temp into the client's matrix,
/// six instructions whose 16-byte reload straddles three narrower stores this body
/// just made. Nothing elides that copy for it, because the adapter's destination is
/// a raw guest pointer nothing can prove writable ahead of the call.
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
pub fn c33_matrix__from_axis_angle__7be490_into(
    out: &mut core::mem::MaybeUninit<[f32; 9]>,
    axis: &[f32; 3],
    angle: f32,
    normalize: bool,
) {
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

    out.write([
        diagonal(x),
        super::f64_to_f32(t_yx + zs),
        super::f64_to_f32(t_zx - f64::from(ys)),
        super::f64_to_f32(t_yx - zs),
        diagonal(y),
        super::f64_to_f32(f64::from(xs) + t_zy),
        super::f64_to_f32(f64::from(t_zx_narrowed) + f64::from(ys)),
        super::f64_to_f32(t_zy - f64::from(xs)),
        diagonal(z),
    ]);
}

/// Axis/angle pairs carrying full mantissas, so a reassociation lands apart.
///
/// Shared with the two wider entry points' tests. Built from an LCG rather than
/// written out: the shapes being separated differ on most inputs, and what the
/// asserts need is that *some* input separates them.
#[cfg(test)]
pub fn axis_angle_sweep() -> [([f32; 3], f32); 64] {
    // Sign and all 23 mantissa bits kept, exponent forced: values in +/-[1, 2).
    // Mapping [1, 2) onto [-1, 1) by `v * 2 - 3` would leave every value a multiple
    // of 2^-22, and sums of those are exact in f32.
    let spread = |bits: u32| f32::from_bits((bits & 0x807f_ffff) | 0x3f80_0000);
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

/// `C33Matrix::FromEulerAngles` — stores `transpose(Z(a0) * (Y(a1) * X(a2)))`.
///
/// The only entry of the six whose name does not spell its axis order, and the
/// only one with live callers (0x50ab62 and 0x60a184). Its first stack argument
/// drives the Z-shaped matrix at `[ebp-0x7c]` (0x7bf4db..0x7bf503), the second
/// the Y-shaped one and the third the X-shaped one.
pub fn c33_matrix__from_euler_angles__7bf4b0(angle_z: f32, angle_y: f32, angle_x: f32) -> [f32; 9] {
    let ([sz, sy, sx], [cz, cy, cx]) = crate::math::trig::sin_cos3([angle_z, angle_y, angle_x]);
    euler3(&euler_z3(cz, sz), &euler_y3(cy, sy), &euler_x3(cx, sx))
}

/// `C33Matrix::FromEulerXYZ` — stores `transpose(X(a0) * (Y(a1) * Z(a2)))`.
///
/// The stack arguments are the axes in the name's order, which is what the three
/// `FSINCOS` blocks at 0x7bed52, 0x7bedaa and 0x7bee02 say: the first argument
/// feeds the X-shaped matrix, the second the Y-shaped one, the third the
/// Z-shaped one.
pub fn c33_matrix__from_euler_xyz__7bed30(angle_x: f32, angle_y: f32, angle_z: f32) -> [f32; 9] {
    let ([sx, sy, sz], [cx, cy, cz]) = crate::math::trig::sin_cos3([angle_x, angle_y, angle_z]);
    euler3(&euler_x3(cx, sx), &euler_y3(cy, sy), &euler_z3(cz, sz))
}

/// `C33Matrix::FromEulerXZY` — stores `transpose(X(a0) * (Z(a1) * Y(a2)))`.
///
/// The SECOND stack argument is the Z angle and the third the Y angle: the
/// matrix at `[ebp-0x34]` gets the Z layout (0x7bef33..0x7bef5b) and the one at
/// `[ebp-0x58]` the Y layout (0x7bef8b..0x7befac).
pub fn c33_matrix__from_euler_xzy__7beeb0(angle_x: f32, angle_z: f32, angle_y: f32) -> [f32; 9] {
    let ([sx, sz, sy], [cx, cz, cy]) = crate::math::trig::sin_cos3([angle_x, angle_z, angle_y]);
    euler3(&euler_x3(cx, sx), &euler_z3(cz, sz), &euler_y3(cy, sy))
}

/// `C33Matrix::FromEulerYXZ` — stores `transpose(Y(a0) * (X(a1) * Z(a2)))`.
///
/// The FIRST stack argument is the Y angle: `[ebp-0x7c]` takes the Y layout
/// (0x7bf05b..0x7bf083), `[ebp-0x34]` the X layout and `[ebp-0x58]` the Z one.
pub fn c33_matrix__from_euler_yxz__7bf030(angle_y: f32, angle_x: f32, angle_z: f32) -> [f32; 9] {
    let ([sy, sx, sz], [cy, cx, cz]) = crate::math::trig::sin_cos3([angle_y, angle_x, angle_z]);
    euler3(&euler_y3(cy, sy), &euler_x3(cx, sx), &euler_z3(cz, sz))
}

/// `C33Matrix::FromEulerYZX` — stores `transpose(Y(a0) * (Z(a1) * X(a2)))`.
///
/// Argument order follows the name: `[ebp-0x7c]` takes the Y layout
/// (0x7bf1db..0x7bf203), `[ebp-0x34]` the Z one and `[ebp-0x58]` the X one.
pub fn c33_matrix__from_euler_yzx__7bf1b0(angle_y: f32, angle_z: f32, angle_x: f32) -> [f32; 9] {
    let ([sy, sz, sx], [cy, cz, cx]) = crate::math::trig::sin_cos3([angle_y, angle_z, angle_x]);
    euler3(&euler_y3(cy, sy), &euler_z3(cz, sz), &euler_x3(cx, sx))
}

/// `C33Matrix::FromEulerZXY` — stores `transpose(Z(a0) * (X(a1) * Y(a2)))`.
///
/// Argument order follows the name: `[ebp-0x7c]` takes the Z layout
/// (0x7bf35b..0x7bf383), `[ebp-0x34]` the X one and `[ebp-0x58]` the Y one.
pub fn c33_matrix__from_euler_zxy__7bf330(angle_z: f32, angle_x: f32, angle_y: f32) -> [f32; 9] {
    let ([sz, sx, sy], [cz, cx, cy]) = crate::math::trig::sin_cos3([angle_z, angle_x, angle_y]);
    euler3(&euler_z3(cz, sz), &euler_x3(cx, sx), &euler_y3(cy, sy))
}

#[cfg(test)]
mod tests_euler_builders {
    use super::{
        c33_matrix__from_euler_angles__7bf4b0, c33_matrix__from_euler_xyz__7bed30,
        c33_matrix__from_euler_xzy__7beeb0, c33_matrix__from_euler_yxz__7bf030,
        c33_matrix__from_euler_yzx__7bf1b0, c33_matrix__from_euler_zxy__7bf330, euler_x3, euler_y3,
        euler_z3, mul3, transpose3,
    };
    use crate::math::trig::sin_cos;

    /// One builder: its address, its entry point, and its three axis letters.
    type Entry = (&'static str, fn(f32, f32, f32) -> [f32; 9], [u8; 3]);

    /// The six builders, each with the axis letters its stack arguments carry.
    ///
    /// The letters are read off the store stream of each original, not off the
    /// symbol name: they agree, but only because the name orders the axes the
    /// way the arguments do, and that is the fact under test.
    const ENTRIES: [Entry; 6] = [
        ("7bf4b0", c33_matrix__from_euler_angles__7bf4b0, *b"zyx"),
        ("7bed30", c33_matrix__from_euler_xyz__7bed30, *b"xyz"),
        ("7beeb0", c33_matrix__from_euler_xzy__7beeb0, *b"xzy"),
        ("7bf030", c33_matrix__from_euler_yxz__7bf030, *b"yxz"),
        ("7bf1b0", c33_matrix__from_euler_yzx__7bf1b0, *b"yzx"),
        ("7bf330", c33_matrix__from_euler_zxy__7bf330, *b"zxy"),
    ];

    fn axis(letter: u8, angle: f32) -> [f32; 9] {
        let (s, c) = sin_cos(angle);
        match letter {
            b'x' => euler_x3(c, s),
            b'y' => euler_y3(c, s),
            _ => euler_z3(c, s),
        }
    }

    /// Per-step `f32` expansion of a 3x3 product, the closed-form alternative.
    ///
    /// Two of these builders used to be written this way instead of calling the
    /// multiply kernel twice, and the difference survives to the stored lane.
    fn expand_f32(a: &[f32; 9], b: &[f32; 9]) -> [f32; 9] {
        let mut out = [0.0f32; 9];
        for r in 0..3 {
            for c in 0..3 {
                out[r * 3 + c] =
                    a[r * 3] * b[c] + a[r * 3 + 1] * b[3 + c] + a[r * 3 + 2] * b[6 + c];
            }
        }
        out
    }

    fn eq9(name: &str, what: &str, got: &[f32; 9], want: &[f32; 9]) {
        for i in 0..9 {
            assert_eq!(
                got[i].to_bits(),
                want[i].to_bits(),
                "{name} {what} lane {i}: {} vs {}",
                got[i],
                want[i]
            );
        }
    }

    #[test]
    fn each_argument_drives_the_axis_the_original_gives_it() {
        // One non-zero argument at a time: the other two matrices are exact
        // identities, so the product narrows to the single axis matrix and the
        // compare can be on bits. This is the pin that five of the six entries
        // were failing: they read argument 0 as the X angle whatever the name
        // said, and built two of the three axes transposed.
        let a = 0.5f32;
        assert!(
            sin_cos(a).0 != 0.0,
            "a degenerate fixture would not separate a layout from its transpose"
        );
        for (name, f, letters) in ENTRIES {
            for pos in 0..3 {
                let mut args = [0.0f32; 3];
                args[pos] = a;
                let want = transpose3(&axis(letters[pos], a));
                eq9(name, "axis", &f(args[0], args[1], args[2]), &want);
            }
        }
    }

    /// Angle triples separating the right-associated product from the left one.
    ///
    /// Searched rather than chosen: the two groupings agree on most inputs, and
    /// a rounded decimal literal is a different `f32` that may stop separating
    /// them, so these are bit patterns.
    const ASSOC: [[u32; 3]; 6] = [
        [0xbfb4_d600, 0x3eb4_ada7, 0x3fbb_5a0d],
        [0xbf3d_e08d, 0xc01e_6a1b, 0x3e58_e61a],
        [0xbed9_2a33, 0xbfab_a91d, 0x4041_ea60],
        [0xbf54_a61a, 0xbe7c_1700, 0xbfe4_23d0],
        [0x3e78_cf33, 0x4015_54b5, 0xc000_2faa],
        [0xbf9c_56c3, 0x3fb7_daed, 0x3f9a_55ad],
    ];

    #[test]
    fn the_product_is_right_associated() {
        // The first `call C33Matrix::Multiply` takes the second and third
        // matrices and the second takes the first against that result, so the
        // grouping is `m1 * (m2 * m3)`. Regrouping perturbs the chain by about
        // 2^-53 and the f32 store is granular to 2^-24, so only a searched
        // fixture separates the two; each assert carries its own separation
        // check so a fixture that stops separating fails loudly instead of
        // passing under either form.
        for (k, (name, f, letters)) in ENTRIES.iter().enumerate() {
            let a = ASSOC[k].map(f32::from_bits);
            let m = [
                axis(letters[0], a[0]),
                axis(letters[1], a[1]),
                axis(letters[2], a[2]),
            ];
            let right = transpose3(&mul3(&m[0], &mul3(&m[1], &m[2])));
            let left = transpose3(&mul3(&mul3(&m[0], &m[1]), &m[2]));
            assert_ne!(right, left, "{name}: fixture stopped separating groupings");
            eq9(name, "assoc", &f(a[0], a[1], a[2]), &right);
        }
    }

    /// Angle triples separating the multiply kernel from an algebraic expansion.
    ///
    /// Same reasoning as [`ASSOC`]: the kernel keeps each three-term dot on the
    /// x87 stack and narrows once, an expansion rounds every step, and only
    /// some inputs carry that difference to a stored lane.
    const EXPAND: [[u32; 3]; 6] = [
        [0xbfb4_d600, 0x3eb4_ada7, 0x3fbb_5a0d],
        [0x404b_0080, 0xbf19_3fed, 0x3f69_f167],
        [0xbed9_2a33, 0xbfab_a91d, 0x4041_ea60],
        [0xbf36_8ac7, 0xc04a_c588, 0x3f06_11ad],
        [0x3e78_cf33, 0x4015_54b5, 0xc000_2faa],
        [0xbf9c_56c3, 0x3fb7_daed, 0x3f9a_55ad],
    ];

    #[test]
    fn the_product_runs_through_the_multiply_kernel() {
        for (k, (name, f, letters)) in ENTRIES.iter().enumerate() {
            let a = EXPAND[k].map(f32::from_bits);
            let m = [
                axis(letters[0], a[0]),
                axis(letters[1], a[1]),
                axis(letters[2], a[2]),
            ];
            let kernel = transpose3(&mul3(&m[0], &mul3(&m[1], &m[2])));
            let expanded = transpose3(&expand_f32(&m[0], &expand_f32(&m[1], &m[2])));
            assert_ne!(
                kernel, expanded,
                "{name}: fixture stopped separating shapes"
            );
            eq9(name, "kernel", &f(a[0], a[1], a[2]), &kernel);
        }
    }

    #[test]
    fn all_zero_angles_give_the_identity() {
        let id = [1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        for (name, f, _) in ENTRIES {
            eq9(name, "identity", &f(0.0, 0.0, 0.0), &id);
        }
    }

    #[test]
    fn rows_stay_orthonormal() {
        for (name, f, _) in ENTRIES {
            for &(a0, a1, a2) in &[
                (0.31f32, -0.72f32, 1.13f32),
                (1.7, 0.4, -0.9),
                (-2.1, 1.05, 0.33),
                (3.0, 1.0, -1.5),
            ] {
                let m = f(a0, a1, a2);
                for i in 0..3 {
                    for j in 0..3 {
                        let dot = m[i * 3] * m[j * 3]
                            + m[i * 3 + 1] * m[j * 3 + 1]
                            + m[i * 3 + 2] * m[j * 3 + 2];
                        let want = if i == j { 1.0 } else { 0.0 };
                        assert!((dot - want).abs() < 1e-5, "{name} row {i}.{j}");
                    }
                }
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
