//! `C3Vector` family kernels.
//!
//! These are the predictable lints for hand-reconstructed vector/matrix
//! arithmetic that mirrors a fixed reference; allowing them module-wide (rather
//! than per line across every assembled kernel) keeps the family files readable
//! while leaving every other lint live. `suboptimal_flops` in particular is
//! deliberate: the baseline has no hardware FMA, so `mul_add` lowers to a slow
//! libm call and changes rounding versus the scalar reference.
#![allow(
    non_snake_case,
    // intentional NaN-reject via `!(a >= b)`
    // (differs from `a < b` on NaN), bit-exact source constants kept verbatim,
    // and ABI-dictated parameter counts.
    clippy::neg_cmp_op_on_partial_ord,
    clippy::excessive_precision,
    clippy::too_many_arguments
)]

use super::f64_to_f32;

/// Normalize a 3-component vector in place to `target_len`:
/// `v *= target_len / |v|`.
///
/// Computed in f64 (sum of squares, `sqrt`, the reciprocal scale, and the
/// per-component product) to stay close to the original's x87 80-bit
/// intermediate and match the double-precision reference replacement; a
/// degenerate (zero or non-finite) input propagates `inf`/`NaN` exactly as the
/// original does.
pub fn normalize3(v: &mut [f32; 3], target_len: f64) {
    let x = f64::from(v[0]);
    let y = f64::from(v[1]);
    let z = f64::from(v[2]);
    let scale = target_len / (x * x + y * y + z * z).sqrt();
    v[0] = f64_to_f32(x * scale);
    v[1] = f64_to_f32(y * scale);
    v[2] = f64_to_f32(z * scale);
}

#[cfg(test)]
mod tests {
    use super::normalize3;

    // Independent length oracle; no `mul_add` for the same no-FMA reason as the
    // kernel (and to keep its rounding comparable).
    fn length(v: &[f32; 3]) -> f64 {
        let x = f64::from(v[0]);
        let y = f64::from(v[1]);
        let z = f64::from(v[2]);
        (x * x + y * y + z * z).sqrt()
    }

    #[test]
    fn normalizes_to_unit_length() {
        for mut v in [
            [1.0f32, 2.0, 3.0],
            [-5.0, 0.1, 4.2],
            [100.0, -200.0, 50.0],
            [0.001, 0.002, 0.003],
        ] {
            normalize3(&mut v, 1.0);
            assert!(
                (length(&v) - 1.0).abs() < 1e-6,
                "len {} for {v:?}",
                length(&v)
            );
        }
    }

    #[test]
    fn known_unit_triangle() {
        let mut v = [3.0f32, 4.0, 0.0];
        normalize3(&mut v, 1.0);
        assert!((v[0] - 0.6).abs() < 1e-6);
        assert!((v[1] - 0.8).abs() < 1e-6);
        assert!(v[2].abs() < 1e-12);
    }

    #[test]
    fn preserves_direction() {
        let orig = [2.0f32, -3.0, 6.0];
        let mut v = orig;
        normalize3(&mut v, 1.0);
        // Same direction ⇒ cross product with the original is ~0.
        let cross = [
            f64::from(orig[1]) * f64::from(v[2]) - f64::from(orig[2]) * f64::from(v[1]),
            f64::from(orig[2]) * f64::from(v[0]) - f64::from(orig[0]) * f64::from(v[2]),
            f64::from(orig[0]) * f64::from(v[1]) - f64::from(orig[1]) * f64::from(v[0]),
        ];
        for c in cross {
            assert!(c.abs() < 1e-5, "cross component {c} too large");
        }
    }

    #[test]
    fn scales_to_k() {
        let mut v = [1.0f32, 0.0, 0.0];
        normalize3(&mut v, 2.5);
        assert!((length(&v) - 2.5).abs() < 1e-6);
    }

    #[test]
    fn zero_vector_propagates_nan() {
        let mut v = [0.0f32, 0.0, 0.0];
        normalize3(&mut v, 1.0);
        assert!(v.iter().all(|c| c.is_nan()), "{v:?}");
    }

    #[test]
    fn large_magnitude_no_f32_overflow() {
        // (2e19)² overflows f32 (> 3.4e38) but not the f64 intermediate.
        let mut v = [2.0e19f32, 0.0, 0.0];
        normalize3(&mut v, 1.0);
        assert!((v[0] - 1.0).abs() < 1e-6, "{v:?}");
        assert!(v[1].abs() < 1e-12 && v[2].abs() < 1e-12, "{v:?}");
    }
}

/// Adds `rhs` into `acc` component-wise, returning the sum (out = acc + rhs).
pub fn c3_vector__add_in_place__4841c0(acc: &[f32; 3], rhs: &[f32; 3]) -> [f32; 3] {
    [acc[0] + rhs[0], acc[1] + rhs[1], acc[2] + rhs[2]]
}

#[cfg(test)]
mod tests_c3_vector__add_in_place__4841c0 {
    use super::c3_vector__add_in_place__4841c0 as f;
    #[test]
    fn known_value() {
        assert_eq!(f(&[1.0, 2.0, 3.0], &[4.0, 5.0, 6.0]), [5.0, 7.0, 9.0]);
    }
    #[test]
    fn add_zero_identity() {
        let v = [1.5, -2.0, 9.0];
        assert_eq!(f(&v, &[0.0, 0.0, 0.0]), v);
    }
    #[test]
    fn commutative() {
        let a = [1.0, 2.0, 3.0];
        let b = [-7.0, 0.5, 11.0];
        assert_eq!(f(&a, &b), f(&b, &a));
    }
}

/// Scales a vec3 by the inverse of `divisor` (each component divided by `divisor`).
pub fn c3_vector__scale_by_inverse__4549c0(v: &[f32; 3], divisor: f32) -> [f32; 3] {
    let inv = 1.0_f32 / divisor;
    [v[0] * inv, v[1] * inv, v[2] * inv]
}

#[cfg(test)]
mod tests_c3_vector__scale_by_inverse__4549c0 {
    use super::c3_vector__scale_by_inverse__4549c0 as f;
    #[test]
    fn known_value() {
        let out = f(&[2.0, 4.0, 8.0], 2.0);
        assert_eq!(out, [1.0, 2.0, 4.0]);
    }
    #[test]
    fn divide_by_one_identity() {
        let v = [1.5, -3.25, 7.0];
        assert_eq!(f(&v, 1.0), v);
    }
    #[test]
    fn normalize_by_length_gives_unit() {
        let v = [3.0_f32, 4.0, 0.0];
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        let u = f(&v, len);
        let mag = (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt();
        assert!((mag - 1.0).abs() < 1e-6);
    }
}

/// Scales a vec3 in place by a scalar: out = vec * scalar.
pub fn c3_vector__scale_in_place__5132f0(v: &[f32; 3], scalar: f32) -> [f32; 3] {
    [v[0] * scalar, v[1] * scalar, v[2] * scalar]
}

#[cfg(test)]
mod tests_c3_vector__scale_in_place__5132f0 {
    use super::c3_vector__scale_in_place__5132f0 as f;
    #[test]
    fn known_value() {
        assert_eq!(f(&[1.0, 2.0, 3.0], 2.0), [2.0, 4.0, 6.0]);
    }
    #[test]
    fn scale_by_one_identity() {
        let v = [1.5, -2.0, 9.0];
        assert_eq!(f(&v, 1.0), v);
    }
    #[test]
    fn scale_by_zero() {
        assert_eq!(f(&[1.0, 2.0, 3.0], 0.0), [0.0, 0.0, 0.0]);
    }
}

/// Scales `src` by a scalar into a fresh vec3: out = src * scalar.
pub fn c3_vector__scale__5f8cf0(src: &[f32; 3], scalar: f32) -> [f32; 3] {
    [src[0] * scalar, src[1] * scalar, src[2] * scalar]
}

#[cfg(test)]
mod tests_c3_vector__scale__5f8cf0 {
    use super::c3_vector__scale__5f8cf0 as f;
    #[test]
    fn known_value() {
        assert_eq!(f(&[1.0, 2.0, 3.0], 3.0), [3.0, 6.0, 9.0]);
    }
    #[test]
    fn scale_by_one_identity() {
        let v = [4.0, -5.0, 6.0];
        assert_eq!(f(&v, 1.0), v);
    }
    #[test]
    fn matches_known_scale() {
        let v = [2.0_f32, 3.0, 4.0];
        let s = 2.5_f32;
        assert_eq!(f(&v, s), [v[0] * s, v[1] * s, v[2] * s]);
    }
}

/// Returns the squared magnitude `x*x + y*y + z*z` of a vec3.
pub fn c3_vector__squared_magnitude__4549f0(v: &[f32; 3]) -> f32 {
    v[0] * v[0] + v[1] * v[1] + v[2] * v[2]
}

#[cfg(test)]
mod tests_c3_vector__squared_magnitude__4549f0 {
    use super::c3_vector__squared_magnitude__4549f0 as f;
    #[test]
    fn known_value() {
        assert_eq!(f(&[3.0, 4.0, 0.0]), 25.0);
        assert_eq!(f(&[1.0, 2.0, 2.0]), 9.0);
    }
    #[test]
    fn non_negative() {
        assert!(f(&[-5.0, -6.0, -7.0]) >= 0.0);
    }
    #[test]
    fn zero_vector() {
        assert_eq!(f(&[0.0, 0.0, 0.0]), 0.0);
    }
}

/// Component-wise subtract: out = a - b.
pub fn c3_vector__subtract__4841f0(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[cfg(test)]
mod tests_c3_vector__subtract__4841f0 {
    use super::c3_vector__subtract__4841f0 as f;
    #[test]
    fn known_value() {
        assert_eq!(f(&[5.0, 7.0, 9.0], &[4.0, 5.0, 6.0]), [1.0, 2.0, 3.0]);
    }
    #[test]
    fn subtract_self_is_zero() {
        let v = [3.3, -1.0, 8.0];
        assert_eq!(f(&v, &v), [0.0, 0.0, 0.0]);
    }
    #[test]
    fn anticommutative() {
        let a = [1.0, 2.0, 3.0];
        let b = [9.0, -4.0, 0.5];
        let ab = f(&a, &b);
        let ba = f(&b, &a);
        assert_eq!(ab, [-ba[0], -ba[1], -ba[2]]);
    }
}

/// Adds a scalar to every component of a 3-component vector.
pub fn c3_vector__add_scalar__6372d0(v: &[f32; 3], s: f32) -> [f32; 3] {
    [v[0] + s, v[1] + s, v[2] + s]
}

#[cfg(test)]
mod tests_c3_vector__add_scalar__6372d0 {
    use super::c3_vector__add_scalar__6372d0 as add_scalar;

    #[test]
    fn known_value() {
        assert_eq!(add_scalar(&[1.0, 2.0, 3.0], 10.0), [11.0, 12.0, 13.0]);
    }

    #[test]
    fn identity_zero() {
        let v = [1.5, -2.5, 7.25];
        assert_eq!(add_scalar(&v, 0.0), v);
    }

    #[test]
    fn negative_scalar() {
        assert_eq!(add_scalar(&[5.0, 5.0, 5.0], -2.0), [3.0, 3.0, 3.0]);
    }
}

/// Componentwise sum of two 3-component vectors.
pub fn c3_vector__add__621160(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

#[cfg(test)]
mod tests_c3_vector__add__621160 {
    use super::c3_vector__add__621160 as add;

    #[test]
    fn known_value() {
        assert_eq!(add(&[1.0, 2.0, 3.0], &[4.0, 5.0, 6.0]), [5.0, 7.0, 9.0]);
    }

    #[test]
    fn identity_zero() {
        let a = [1.5, -2.5, 7.25];
        assert_eq!(add(&a, &[0.0, 0.0, 0.0]), a);
    }

    #[test]
    fn commutative() {
        let a = [1.5, -2.25, 0.75];
        let b = [-3.0, 4.5, 2.0];
        assert_eq!(add(&a, &b), add(&b, &a));
    }
}

/// Cross product of two 3-component vectors (`a x b`).
pub fn c3_vector__cross__672130(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[cfg(test)]
mod tests_c3_vector__cross__672130 {
    use super::{c3_vector__cross__672130 as cross, c3_vector__dot__602630 as dot};

    #[test]
    fn known_value_basis() {
        // x cross y = z
        assert_eq!(cross(&[1.0, 0.0, 0.0], &[0.0, 1.0, 0.0]), [0.0, 0.0, 1.0]);
    }

    #[test]
    fn anticommutative() {
        let a = [1.5, -2.25, 0.75];
        let b = [-3.0, 4.5, 2.0];
        let ab = cross(&a, &b);
        let ba = cross(&b, &a);
        assert_eq!(ab, [-ba[0], -ba[1], -ba[2]]);
    }

    #[test]
    fn orthogonal_to_inputs() {
        let a = [1.5, -2.25, 0.75];
        let b = [-3.0, 4.5, 2.0];
        let c = cross(&a, &b);
        assert!(dot(&c, &a).abs() < 1e-4);
        assert!(dot(&c, &b).abs() < 1e-4);
    }

    #[test]
    fn parallel_is_zero() {
        let a = [2.0, 4.0, 6.0];
        let b = [1.0, 2.0, 3.0];
        let c = cross(&a, &b);
        assert!(c[0].abs() < 1e-5 && c[1].abs() < 1e-5 && c[2].abs() < 1e-5);
    }
}

/// Dot product of two 3-component vectors.
pub fn c3_vector__dot__602630(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    let mut acc = a[0] * b[0];
    acc += a[1] * b[1];
    acc += a[2] * b[2];
    acc
}

#[cfg(test)]
mod tests_c3_vector__dot__602630 {
    use super::c3_vector__dot__602630 as dot;

    #[test]
    fn known_value() {
        assert_eq!(
            dot(&[1.0, 2.0, 3.0], &[4.0, 5.0, 6.0]).to_bits(),
            32.0f32.to_bits()
        );
    }

    #[test]
    fn orthogonal_is_zero() {
        assert_eq!(dot(&[1.0, 0.0, 0.0], &[0.0, 1.0, 0.0]), 0.0);
    }

    #[test]
    fn commutative() {
        let a = [1.5, -2.25, 0.75];
        let b = [-3.0, 4.5, 2.0];
        assert_eq!(dot(&a, &b).to_bits(), dot(&b, &a).to_bits());
    }

    #[test]
    fn self_dot_is_squared_length() {
        let a = [3.0, 4.0, 12.0];
        assert_eq!(dot(&a, &a), 9.0 + 16.0 + 144.0);
    }
}

/// Componentwise negation of a 3-component vector.
pub fn c3_vector__negate__637330(v: &[f32; 3]) -> [f32; 3] {
    [-v[0], -v[1], -v[2]]
}

#[cfg(test)]
mod tests_c3_vector__negate__637330 {
    use super::c3_vector__negate__637330 as negate;

    #[test]
    fn known_value() {
        assert_eq!(negate(&[1.0, -2.0, 3.0]), [-1.0, 2.0, -3.0]);
    }

    #[test]
    fn double_negation_is_identity() {
        let v = [1.5, -2.5, 7.25];
        assert_eq!(negate(&negate(&v)), v);
    }

    #[test]
    fn zero_negates_to_zero() {
        assert_eq!(negate(&[0.0, 0.0, 0.0]), [0.0, 0.0, 0.0]);
    }
}

/// Subtracts a scalar from every component of a 3-component vector.
pub fn c3_vector__subtract_scalar__637300(v: &[f32; 3], s: f32) -> [f32; 3] {
    [v[0] - s, v[1] - s, v[2] - s]
}

#[cfg(test)]
mod tests_c3_vector__subtract_scalar__637300 {
    use super::{
        c3_vector__add_scalar__6372d0 as add_scalar,
        c3_vector__subtract_scalar__637300 as sub_scalar,
    };

    #[test]
    fn known_value() {
        assert_eq!(sub_scalar(&[11.0, 12.0, 13.0], 10.0), [1.0, 2.0, 3.0]);
    }

    #[test]
    fn identity_zero() {
        let v = [1.5, -2.5, 7.25];
        assert_eq!(sub_scalar(&v, 0.0), v);
    }

    #[test]
    fn inverse_of_add() {
        let v = [1.5, -2.5, 7.25];
        assert_eq!(sub_scalar(&add_scalar(&v, 3.0), 3.0), v);
    }
}

/// Returns `1` iff `a` dominates `b` component-wise (`a.x>=b.x && a.y>=b.y &&
/// a.z>=b.z`), else `0`. The hardware rejects each lane on the unordered/`a<b`
/// flag, so a NaN in either operand makes that lane fail; the `>=` comparison
/// reproduces this exactly (it is `false` for NaN).
pub fn c3_vector__greater_equal_all__699330(a: &[f32; 3], b: &[f32; 3]) -> i32 {
    let ge = a[0] >= b[0] && a[1] >= b[1] && a[2] >= b[2];
    i32::from(ge)
}

#[cfg(test)]
mod tests_c3_vector__greater_equal_all__699330 {
    use super::c3_vector__greater_equal_all__699330 as ge;
    #[test]
    fn equal_dominates() {
        assert_eq!(ge(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0]), 1);
    }
    #[test]
    fn strictly_greater() {
        assert_eq!(ge(&[2.0, 3.0, 4.0], &[1.0, 2.0, 3.0]), 1);
    }
    #[test]
    fn one_component_less_fails() {
        assert_eq!(ge(&[2.0, 1.9, 4.0], &[1.0, 2.0, 3.0]), 0);
        assert_eq!(ge(&[0.9, 3.0, 4.0], &[1.0, 2.0, 3.0]), 0);
        assert_eq!(ge(&[2.0, 3.0, 2.9], &[1.0, 2.0, 3.0]), 0);
    }
    #[test]
    fn nan_rejects_each_axis() {
        assert_eq!(ge(&[f32::NAN, 2.0, 3.0], &[1.0, 2.0, 3.0]), 0);
        assert_eq!(ge(&[2.0, f32::NAN, 3.0], &[1.0, 2.0, 3.0]), 0);
        assert_eq!(ge(&[2.0, 3.0, f32::NAN], &[1.0, 2.0, 3.0]), 0);
        assert_eq!(ge(&[2.0, 3.0, 4.0], &[f32::NAN, 2.0, 3.0]), 0);
    }
    #[test]
    fn boundary_equality_each_axis() {
        for axis in 0..3 {
            let mut a = [5.0f32, 5.0, 5.0];
            let mut b = [4.0f32, 4.0, 4.0];
            a[axis] = 4.0;
            b[axis] = 4.0;
            assert_eq!(ge(&a, &b), 1, "axis {axis}");
        }
    }
}

/// Returns `1` iff every lane of a 3-vector is finite (neither NaN nor
/// infinite), else `0`. Each lane is promoted to f64 and its 11 exponent bits
/// tested against the all-ones pattern, exactly mirroring the original's
/// per-lane `(exp_bits != 0x7ff)` check on the widened double (an inlined
/// 2-instruction pure leaf). The test short-circuits, matching the original.
pub fn c3_vector__is_valid__6337d0(v: &[f32; 3]) -> u8 {
    fn finite(x: f32) -> bool {
        // Promote to f64; the exponent field is all-ones only for Inf/NaN.
        let exp = ((f64::from(x).to_bits() >> 52) & 0x7ff) as u16;
        exp != 0x7ff
    }
    u8::from(finite(v[0]) && finite(v[1]) && finite(v[2]))
}

#[cfg(test)]
mod tests_c3_vector__is_valid__6337d0 {
    use super::c3_vector__is_valid__6337d0 as valid;
    #[test]
    fn all_finite_is_valid() {
        assert_eq!(valid(&[1.0, -2.5, 3.0e10]), 1);
        assert_eq!(valid(&[0.0, -0.0, 1.0e-30]), 1);
    }
    #[test]
    fn nan_invalid_each_axis() {
        assert_eq!(valid(&[f32::NAN, 1.0, 2.0]), 0);
        assert_eq!(valid(&[1.0, f32::NAN, 2.0]), 0);
        assert_eq!(valid(&[1.0, 2.0, f32::NAN]), 0);
    }
    #[test]
    fn inf_invalid_each_axis() {
        assert_eq!(valid(&[f32::INFINITY, 1.0, 2.0]), 0);
        assert_eq!(valid(&[1.0, f32::NEG_INFINITY, 2.0]), 0);
        assert_eq!(valid(&[1.0, 2.0, f32::INFINITY]), 0);
    }
    #[test]
    fn matches_is_finite() {
        for v in [
            [1.0f32, 2.0, 3.0],
            [f32::MAX, f32::MIN, 0.0],
            [f32::NAN, 0.0, 0.0],
            [0.0, f32::INFINITY, 0.0],
        ] {
            let want = u8::from(v[0].is_finite() && v[1].is_finite() && v[2].is_finite());
            assert_eq!(valid(&v), want, "{v:?}");
        }
    }
}

/// In-place component-wise maximum: `dst[i] = max(dst[i], src[i])` for the three
/// lanes. Stock keeps `dst` ONLY when `src` is strictly-ordered-less: the z/y
/// lanes are `FCOMPP; FNSTSW; TEST AH,0x5; JP src` and the x lane is
/// `FCOMP; FNSTSW; TEST AH,0x41; JNZ src` (0x6b11f6..0x6b1236), all of which
/// route equal AND unordered to the `src` load — so a `src` NaN sticks, a `dst`
/// NaN is replaced, and a ±0 tie takes `src`'s bit pattern. Encoded as
/// `if src < dst { dst } else { src }`; do NOT flip this into
/// `if dst < src { src }` (that keeps `dst` on ties/unordered — the polarity
/// this kernel originally shipped with, caught during the
/// BuildVerticesAndBounds review).
pub fn c3_vector__max_in_place__6b11f0(dst: &mut [f32; 3], src: &[f32; 3]) {
    dst[0] = if src[0] < dst[0] { dst[0] } else { src[0] };
    dst[1] = if src[1] < dst[1] { dst[1] } else { src[1] };
    dst[2] = if src[2] < dst[2] { dst[2] } else { src[2] };
}

#[cfg(test)]
mod tests_c3_vector__max_in_place__6b11f0 {
    use super::{c3_vector__max__6992c0 as max, c3_vector__max_in_place__6b11f0 as maxip};
    #[test]
    fn known_value() {
        let mut d = [1.0, 5.0, 3.0];
        maxip(&mut d, &[4.0, 2.0, 6.0]);
        assert_eq!(d, [4.0, 5.0, 6.0]);
    }
    #[test]
    fn agrees_with_out_of_place() {
        let a = [1.0f32, 7.0, -3.0];
        let b = [4.0f32, 2.0, 8.0];
        let mut d = a;
        maxip(&mut d, &b);
        assert_eq!(d, max(&a, &b));
    }
    #[test]
    fn idempotent_with_self_copy() {
        let v = [1.5f32, -2.0, 9.0];
        let mut d = v;
        maxip(&mut d, &v);
        assert_eq!(d, v);
    }
    #[test]
    fn per_axis_independent() {
        let mut d = [9.0f32, 0.0, 0.0];
        maxip(&mut d, &[0.0, 9.0, 9.0]);
        assert_eq!(d, [9.0, 9.0, 9.0]);
    }
    #[test]
    fn nan_src_sticks() {
        // Unordered lanes take SRC (stock JP/JNZ route NaN to the src load).
        let mut d = [1.0f32, 2.0, 3.0];
        maxip(&mut d, &[f32::NAN, 5.0, f32::NAN]);
        assert!(d[0].is_nan());
        assert_eq!(d[1], 5.0);
        assert!(d[2].is_nan());
    }
    #[test]
    fn nan_dst_is_replaced() {
        // Unordered lanes take SRC even when the NaN sits in dst.
        let mut d = [f32::NAN, f32::NAN, 7.0];
        maxip(&mut d, &[1.0, 2.0, 3.0]);
        assert_eq!(d, [1.0, 2.0, 7.0]);
    }
    #[test]
    fn signed_zero_tie_takes_src_bits() {
        // x87 compares -0.0 == +0.0 as EQUAL; equal lanes take SRC's bits.
        let mut d = [-0.0f32, 0.0, -0.0];
        maxip(&mut d, &[0.0, -0.0, -0.0]);
        assert_eq!(d[0].to_bits(), 0.0f32.to_bits());
        assert_eq!(d[1].to_bits(), (-0.0f32).to_bits());
        assert_eq!(d[2].to_bits(), (-0.0f32).to_bits());
    }
}

/// Component-wise maximum of two 3-vectors into a fresh vector:
/// `out[i] = max(a[i], b[i])`. Stock (0x6992c3..0x699311, same FCOMPP/`TEST
/// AH,0x5`/JP and FCOMP/`TEST AH,0x41`/JNZ shape as `MaxInPlace` 0x6b11f0)
/// keeps `a` (EDX) ONLY when `b` (the stack arg) is strictly-ordered-less:
/// equal and unordered lanes take `b`, so a `b` NaN sticks, an `a` NaN is
/// replaced, and a ±0 tie takes `b`'s bit pattern.
pub fn c3_vector__max__6992c0(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    [
        if b[0] < a[0] { a[0] } else { b[0] },
        if b[1] < a[1] { a[1] } else { b[1] },
        if b[2] < a[2] { a[2] } else { b[2] },
    ]
}

#[cfg(test)]
mod tests_c3_vector__max__6992c0 {
    use super::c3_vector__max__6992c0 as max;
    #[test]
    fn known_value() {
        assert_eq!(max(&[1.0, 5.0, 3.0], &[4.0, 2.0, 6.0]), [4.0, 5.0, 6.0]);
    }
    #[test]
    fn idempotent() {
        let v = [1.5, -2.0, 9.0];
        assert_eq!(max(&v, &v), v);
    }
    #[test]
    fn commutative_value() {
        let a = [1.0, 7.0, -3.0];
        let b = [4.0, 2.0, 8.0];
        assert_eq!(max(&a, &b), max(&b, &a));
    }
    #[test]
    fn per_axis_independent() {
        assert_eq!(max(&[9.0, 0.0, 0.0], &[0.0, 9.0, 9.0]), [9.0, 9.0, 9.0]);
    }
    #[test]
    fn negative_values() {
        assert_eq!(
            max(&[-5.0, -1.0, -9.0], &[-3.0, -2.0, -1.0]),
            [-3.0, -1.0, -1.0]
        );
    }
    #[test]
    fn nan_in_b_sticks() {
        // Unordered lanes take B (stock routes NaN to the stack-arg load).
        let out = max(&[1.0, 2.0, 3.0], &[f32::NAN, 5.0, f32::NAN]);
        assert!(out[0].is_nan());
        assert_eq!(out[1], 5.0);
        assert!(out[2].is_nan());
    }
    #[test]
    fn nan_in_a_is_replaced() {
        let out = max(&[f32::NAN, f32::NAN, 7.0], &[1.0, 2.0, 3.0]);
        assert_eq!(out, [1.0, 2.0, 7.0]);
    }
    #[test]
    fn signed_zero_tie_takes_b_bits() {
        // x87 compares -0.0 == +0.0 as EQUAL; equal lanes take B's bits.
        let out = max(&[-0.0, 0.0, -0.0], &[0.0, -0.0, -0.0]);
        assert_eq!(out[0].to_bits(), 0.0f32.to_bits());
        assert_eq!(out[1].to_bits(), (-0.0f32).to_bits());
        assert_eq!(out[2].to_bits(), (-0.0f32).to_bits());
    }
}

/// `ScaleStoredVec3ByFactor` (0x7c4ed0) — scales a base vec3 by an external
/// `scale` times an internal `mag`, one FMUL chain per lane.
///
/// The stock x87 evaluates each lane as `(scale * base[i]) * mag`
/// (`fld scale; fmul base[i]; fmul mag`), so the grouping is reproduced
/// verbatim: `scale * base[i]` first, then `* mag`. Computed in f32 to match
/// the single-rounded per-lane store (the z lane rounds its intermediate to f32
/// identically; the x/y lanes' 80-bit intermediate is treated as accepted
/// precision). NaN/inf propagate through the products unchanged — there are no
/// compares or branches in the original.
pub fn scale_stored_vec3_by_factor__7c4ed0(base: &[f32; 3], scale: f32, mag: f32) -> [f32; 3] {
    [
        (scale * base[0]) * mag,
        (scale * base[1]) * mag,
        (scale * base[2]) * mag,
    ]
}

#[cfg(test)]
mod tests_scale_stored_vec3_by_factor__7c4ed0 {
    use super::scale_stored_vec3_by_factor__7c4ed0 as f;

    // Independent oracle: the exact instruction grouping `(scale*base[i])*mag`.
    fn oracle(base: &[f32; 3], scale: f32, mag: f32) -> [f32; 3] {
        let mut out = [0.0f32; 3];
        for i in 0..3 {
            out[i] = (scale * base[i]) * mag;
        }
        out
    }

    #[test]
    fn known_value() {
        // 2 * [1,2,3] * 3 = [6, 12, 18].
        assert_eq!(f(&[1.0, 2.0, 3.0], 2.0, 3.0), [6.0, 12.0, 18.0]);
    }

    #[test]
    fn identity_scalars() {
        let v = [1.5, -2.0, 9.0];
        assert_eq!(f(&v, 1.0, 1.0), v);
    }

    #[test]
    fn scale_zero_zeros_all() {
        assert_eq!(f(&[1.0, 2.0, 3.0], 0.0, 5.0), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn mag_zero_zeros_all() {
        assert_eq!(f(&[1.0, 2.0, 3.0], 5.0, 0.0), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn negatives() {
        assert_eq!(f(&[-1.0, 2.0, -3.0], -2.0, 1.5), [3.0, -6.0, 9.0]);
    }

    #[test]
    fn grouping_matches_oracle() {
        // A fractional case where (scale*base)*mag vs scale*(base*mag) could
        // differ in the last ULP — the reimpl must match the left-grouped oracle.
        let base = [0.1_f32, 0.2, 0.3];
        let (scale, mag) = (7.0_f32, 0.30000001_f32);
        assert_eq!(f(&base, scale, mag), oracle(&base, scale, mag));
    }

    #[test]
    fn nan_scale_propagates() {
        let out = f(&[1.0, 2.0, 3.0], f32::NAN, 4.0);
        assert!(out.iter().all(|x| x.is_nan()));
    }

    #[test]
    fn nan_base_lane_only() {
        // NaN in one base lane must taint only that lane (no cross-lane compares).
        let out = f(&[1.0, f32::NAN, 3.0], 2.0, 1.0);
        assert!(!out[0].is_nan() && out[1].is_nan() && !out[2].is_nan());
        assert_eq!(out[0], 2.0);
        assert_eq!(out[2], 6.0);
    }

    #[test]
    fn inf_propagates() {
        let out = f(&[1.0, 0.0, -1.0], f32::INFINITY, 2.0);
        assert_eq!(out[0], f32::INFINITY);
        assert!(out[1].is_nan()); // inf * 0 = NaN
        assert_eq!(out[2], f32::NEG_INFINITY);
    }
}
