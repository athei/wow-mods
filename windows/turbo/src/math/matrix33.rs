//! `matrix33` family kernels.
#![allow(
    non_snake_case,
    // intentional NaN-reject via `!(a >= b)`
    // (differs from `a < b` on NaN), bit-exact source constants kept verbatim,
    // and ABI-dictated parameter counts.
    clippy::neg_cmp_op_on_partial_ord,
    clippy::excessive_precision,
    clippy::too_many_arguments
)]

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
    let mut out = [0.0f32; 9];
    for r in 0..3 {
        for col in 0..3 {
            out[r * 3 + col] =
                a[r * 3] * b[col] + a[r * 3 + 1] * b[3 + col] + a[r * 3 + 2] * b[6 + col];
        }
    }
    out
}
fn transpose3(m: &[f32; 9]) -> [f32; 9] {
    [m[0], m[3], m[6], m[1], m[4], m[7], m[2], m[5], m[8]]
}

/// 3x3 matrix determinant (rule of Sarrus) of row-major elements `m00..m22`.
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
) -> f32 {
    m00 * m11 * m22 + m02 * m10 * m21 + m01 * m12 * m20
        - m02 * m11 * m20
        - m01 * m10 * m22
        - m00 * m12 * m21
}

#[cfg(test)]
mod tests_c33_matrix__determinant__7bc040 {
    use super::c33_matrix__determinant__7bc040 as det;
    #[test]
    fn identity_is_one() {
        assert_eq!(
            det(1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0).to_bits(),
            1.0f32.to_bits()
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
/// When `normalize` is true the axis is normalized first. Returns row-major 9 floats.
pub fn c33_matrix__from_axis_angle__7be490(
    axis: &[f32; 3],
    angle: f32,
    normalize: bool,
) -> [f32; 9] {
    let mut ax = axis[0];
    let mut ay = axis[1];
    let mut az = axis[2];
    if normalize {
        let inv = 1.0 / (ax * ax + ay * ay + az * az).sqrt();
        ax *= inv;
        ay *= inv;
        az *= inv;
    }
    let (s, c) = crate::math::trig::sin_cos(angle);
    let t = 1.0 - c;
    let mut out = [0.0f32; 9];
    out[0] = ax * ax * t + c;
    let xy = t * ay * ax;
    out[1] = xy + az * s;
    let xz = t * az * ax;
    out[2] = xz - ay * s;
    out[3] = xy - az * s;
    out[4] = ay * ay * t + c;
    let yz = t * az * ay;
    out[5] = ax * s + yz;
    out[6] = xz + ay * s;
    out[7] = yz - ax * s;
    out[8] = az * az * t + c;
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
}

/// Builds a 3x3 rotation-about-Z matrix from `angle`:
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

/// 3x3 * 3x3 matrix product `out = a * b`, row-major: `out[r*3+c] = sum_k a[r*3+k]*b[k*3+c]`.
pub fn c33_matrix__multiply__7bdfc0(a: &[f32; 9], b: &[f32; 9]) -> [f32; 9] {
    let mut out = [0.0f32; 9];
    for r in 0..3 {
        for c in 0..3 {
            let mut acc = a[r * 3] * b[c];
            acc += a[r * 3 + 1] * b[3 + c];
            acc += a[r * 3 + 2] * b[6 + c];
            out[r * 3 + c] = acc;
        }
    }
    out
}

#[cfg(test)]
mod tests_c33_matrix__multiply__7bdfc0 {
    use super::c33_matrix__multiply__7bdfc0 as mul;
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

/// Scales each row of a 3x3 matrix in place by the matching component of a 3-vector:
/// row `i` (3 consecutive floats) is multiplied by `scale[i]`. Returns row-major 9 floats.
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

/// `C33Matrix::FromEulerAngles` — stack args in order `(angle_z, angle_y,
/// angle_x)`. Builds the per-axis rotations `Z(z)`, `Y(y)`, `X(x)`, forms the
/// product `Z · Y · X`, and stores its transpose — exactly the construction the
/// 1.12 client emits (two 3x3 multiplies then a column-major copy). The first
/// stack arg drives the Z rotation, the third drives X.
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

/// `C33Matrix::SetTransposed` — the pure transpose the x87 body performs before
/// it forwards to the setter `FUN_007c0190`. Reads `m` as a row-major 3x3
/// (`m[3*r + c]`) and returns its transpose `t[3*r + c] = m[3*c + r]`. This is a
/// straight memory shuffle — no arithmetic, hence no float compares and no NaN
/// polarity to reproduce (any NaN lane is copied through bit-for-bit).
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
