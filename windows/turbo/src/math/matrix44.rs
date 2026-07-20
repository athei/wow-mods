//! `C44Matrix` family kernels (4×4, 16 floats, row-major).
#![allow(
    non_snake_case,
    // intentional NaN-reject via `!(a >= b)`
    // (differs from `a < b` on NaN), bit-exact source constants kept verbatim,
    // and ABI-dictated parameter counts.
    clippy::neg_cmp_op_on_partial_ord,
    clippy::excessive_precision,
    clippy::too_many_arguments
)]

/// 4×4 row-major matrix product `out = a · b`
/// (`out[r][c] = Σ_k a[r][k] · b[k][c]`, indices `r*4 + c`).
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

/// Builds an orthonormal 3×3 basis (stored in the first three rows of a 4×4
/// matrix) from a direction vector `dir` and a seed "up" reference taken from
/// the matrix's existing row 0.
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

/// Builds a 3×3 rotation matrix (Rodrigues' formula) from an axis and angle,
/// laid out in the first 12 floats of a 4×4 matrix (rows of three, then a zeroed
/// fourth column at indices 9..12).
///
/// When `skip_normalize` is false the axis is normalized first; a true value
/// trusts the caller's axis to be unit length. The remaining matrix slots
/// (indices 12..16) are left untouched, matching the original which only writes
/// 0..12.
pub fn c44_matrix__set_rotation_axis_angle__7bb860(
    axis: &[f32; 3],
    angle: f32,
    skip_normalize: bool,
) -> [f32; 12] {
    let mut ax = axis[0];
    let mut ay = axis[1];
    let mut az = axis[2];

    if !skip_normalize {
        let inv = 1.0 / (ax * ax + ay * ay + az * az).sqrt();
        ax *= inv;
        ay *= inv;
        az *= inv;
    }

    let (s, c) = crate::math::trig::sin_cos(angle);
    let omc = 1.0 - c;

    let xy = omc * ay * ax;
    let xz = omc * az * ax;
    let yz = omc * az * ay;

    let mut out = [0.0f32; 12];
    out[0] = ax * ax * omc + c;
    out[1] = xy + az * s;
    out[2] = xz - ay * s;
    out[3] = xy - az * s;
    out[4] = ay * ay * omc + c;
    out[5] = ax * s + yz;
    out[6] = xz + ay * s;
    out[7] = yz - ax * s;
    out[8] = az * az * omc + c;
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

/// Transforms a 3D point by a 3×4 affine matrix stored column-major:
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

/// In-place post-translate concat: adds `M3x3 · point` into the matrix's
/// translation column.
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

/// `C44Matrix::InvertOrthonormal`: inverse of a rotation (orthonormal 3×3) plus
/// translation matrix. The inverse rotation is the transpose of the 3×3 block
/// (rows at indices 0/1/2, 4/5/6, 8/9/10); the inverse translation is `-t · R`
/// where `t` is the source translation (indices 12/13/14). Returns a 16-float
/// row-major matrix with last row `[it0, it1, it2, 1]`.
pub fn c44_matrix__invert_orthonormal__7bd700(src: &[f32; 16]) -> [f32; 16] {
    let r00 = src[0];
    let r01 = src[1];
    let r02 = src[2];
    let r10 = src[4];
    let r11 = src[5];
    let r12 = src[6];
    let r20 = src[8];
    let r21 = src[9];
    let r22 = src[10];

    let tx = src[12];
    let ty = src[13];
    let tz = src[14];

    // Inverse rotation is the transpose; inverse translation is -t·R
    // (component k = -(t · row_k of R)).
    let it0 = -(r00 * tx + r01 * ty + r02 * tz);
    let it1 = -(r10 * tx + r11 * ty + r12 * tz);
    let it2 = -(r20 * tx + r21 * ty + r22 * tz);

    [
        r00, r10, r20, 0.0, //
        r01, r11, r21, 0.0, //
        r02, r12, r22, 0.0, //
        it0, it1, it2, 1.0,
    ]
}

#[cfg(test)]
mod tests_c44_matrix__invert_orthonormal__7bd700 {
    use super::c44_matrix__invert_orthonormal__7bd700 as invert;

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

/// Pre-multiplies a 4x4 matrix by the rotation built from a quaternion:
/// `out = R(quat) * m`.
///
/// `quat` is `[x, y, z, w]`. The rotation `R` is the standard quaternion->matrix
/// in the original's row-major scratch layout, then composed with the incoming
/// matrix via the shared 4x4 product [`mul4x4`]. The original writes the 16-float
/// result back into the receiver; here it is returned by value.
pub fn c44_matrix__rotate_quaternion__7bddb0(m: &[f32; 16], quat: &[f32; 4]) -> [f32; 16] {
    let x = quat[0];
    let y = quat[1];
    let z = quat[2];
    let w = quat[3];

    let x2 = x + x;
    let y2 = y + y;
    let z2 = z + z;

    let xw = x2 * w;
    let yw = y2 * w;
    let zw = z2 * w;
    let xx = x2 * x;
    let yx = y2 * x;
    let zx = z2 * x;
    let zy = z2 * y;
    let yy = y2 * y;
    let zz = z2 * z;

    let r = [
        1.0 - (zz + yy), // [0]
        yx + zw,         // [1]
        zx - yw,         // [2]
        0.0,             // [3]
        yx - zw,         // [4]
        1.0 - (zz + xx), // [5]
        zy + xw,         // [6]
        0.0,             // [7]
        zx + yw,         // [8]
        zy - xw,         // [9]
        1.0 - (yy + xx), // [10]
        0.0,             // [11]
        0.0,             // [12]
        0.0,             // [13]
        0.0,             // [14]
        1.0,             // [15]
    ];

    super::matrix44::mul4x4(&r, m)
}

#[cfg(test)]
mod tests_c44_matrix__rotate_quaternion__7bddb0 {
    use super::{c44_matrix__rotate_quaternion__7bddb0 as rotq, mul4x4};

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
        let want = mul4x4(&r, &m);
        approx(&out, &want, 1e-5);
    }
}

/// Scales the upper-left 3x3 of a 4x4 matrix (4-float row stride) row-wise by a
/// 3-vector, in place.
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

/// Scales the leading 3x3 (stored contiguously in indices 0..9) of a matrix
/// row-wise by three scalars, in place.
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
pub fn c44_matrix__set_rotation_axis_angle__7bdb00(
    axis: &[f32; 3],
    angle: f32,
    skip_normalize: bool,
) -> [f32; 16] {
    let mut x = axis[0];
    let mut y = axis[1];
    let mut z = axis[2];

    if !skip_normalize {
        let inv = 1.0 / (x * x + y * y + z * z).sqrt();
        x *= inv;
        y *= inv;
        z *= inv;
    }

    let (s, c) = crate::math::trig::sin_cos(angle);
    let omc = 1.0 - c;

    let xy = omc * y * x;
    let xz = omc * z * x;
    let yz = omc * z * y;

    let mut out = [0.0f32; 16];
    out[0] = x * x * omc + c;
    out[1] = xy + z * s;
    out[2] = xz - y * s;
    out[3] = 0.0;
    out[4] = xy - z * s;
    out[5] = y * y * omc + c;
    out[6] = x * s + yz;
    out[7] = 0.0;
    out[8] = xz + y * s;
    out[9] = yz - x * s;
    out[10] = z * z * omc + c;
    out[11] = 0.0;
    out[12] = 0.0;
    out[13] = 0.0;
    out[14] = 0.0;
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
}

/// `C44Matrix::TransformPoint` (column-major variant): transforms a 3D point by
/// a 4×4 matrix with implied `w = 1`, applying the translation row at indices
/// 12/13/14. `out[j] = in.x·mat[j] + in.y·mat[4+j] + in.z·mat[8+j] + mat[12+j]`.
pub fn c44_matrix__transform_point__7bca80(input: &[f32; 3], mat: &[f32; 16]) -> [f32; 3] {
    let ix = input[0];
    let iy = input[1];
    let iz = input[2];

    let mut x = ix * mat[0];
    x += iy * mat[4];
    x += iz * mat[8];
    x += mat[12];

    let mut y = ix * mat[1];
    y += iy * mat[5];
    y += iz * mat[9];
    y += mat[13];

    let mut z = ix * mat[2];
    z += iy * mat[6];
    z += iz * mat[10];
    z += mat[14];

    [x, y, z]
}

#[cfg(test)]
mod tests_c44_matrix__transform_point__7bca80 {
    use super::c44_matrix__transform_point__7bca80 as xform;

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

/// `C44Matrix::TransformPoint` (row-major variant): transforms a 3D point by a
/// 4×4 matrix with implied `w = 1`, reading the translation from the last column
/// (indices 3/7/11). `out[i] = in.x·mat[4i] + in.y·mat[4i+1] + in.z·mat[4i+2] +
/// mat[4i+3]`.
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

/// `C44Matrix::TransformVector4`: transforms a homogeneous 4D vector by a 4×4
/// matrix, computing the full `w` component. `out[j] = in.x·mat[j] +
/// in.y·mat[4+j] + in.z·mat[8+j] + in.w·mat[12+j]` for `j = 0..3` (column-major).
pub fn c44_matrix__transform_vector4__7bcb40(input: &[f32; 4], mat: &[f32; 16]) -> [f32; 4] {
    let ix = input[0];
    let iy = input[1];
    let iz = input[2];
    let iw = input[3];

    let mut x = ix * mat[0];
    x += iy * mat[4];
    x += iz * mat[8];
    x += iw * mat[12];

    let mut y = ix * mat[1];
    y += iy * mat[5];
    y += iz * mat[9];
    y += iw * mat[13];

    let mut z = ix * mat[2];
    z += iy * mat[6];
    z += iz * mat[10];
    z += iw * mat[14];

    let mut w = ix * mat[3];
    w += iy * mat[7];
    w += iz * mat[11];
    w += iw * mat[15];

    [x, y, z, w]
}

#[cfg(test)]
mod tests_c44_matrix__transform_vector4__7bcb40 {
    use super::c44_matrix__transform_vector4__7bcb40 as xform;

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

/// `C44Matrix::TransformVertexInPlace`: transforms a 3D vertex by a 4×4 matrix
/// (column-major, implied `w = 1`, translation at indices 12/13/14) — identical
/// math to the column-major point transform. The original writes the result back
/// into the input vertex and copies it to `out`; this returns the value so the
/// adapter performs both writes.
pub fn c44_matrix__transform_vertex_in_place__7bcc60(vec: &[f32; 3], mat: &[f32; 16]) -> [f32; 3] {
    let ix = vec[0];
    let iy = vec[1];
    let iz = vec[2];

    let mut x = ix * mat[0];
    x += iy * mat[4];
    x += iz * mat[8];
    x += mat[12];

    let mut y = ix * mat[1];
    y += iy * mat[5];
    y += iz * mat[9];
    y += mat[13];

    let mut z = ix * mat[2];
    z += iy * mat[6];
    z += iz * mat[10];
    z += mat[14];

    [x, y, z]
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

/// In-place post-translate of a 4x4 matrix: adds `rot3x3 * trans` into the
/// matrix's translation column (indices 12,13,14).
///
/// The 4x4 has a 4-float row stride: the rotation columns scaling `trans.{x,y,z}`
/// are read from indices {0,4,8}, {1,5,9}, {2,6,10}; each translation component
/// `mat[12+i]` gains `mat[i]*tx + mat[4+i]*ty + mat[8+i]*tz`.
pub fn c44_matrix__translate__7bdc40(mat: &mut [f32; 16], trans: &[f32; 3]) {
    let tx = trans[0];
    let ty = trans[1];
    let tz = trans[2];

    mat[12] += mat[0] * tx + mat[4] * ty + mat[8] * tz;
    mat[13] += mat[1] * tx + mat[5] * ty + mat[9] * tz;
    mat[14] += mat[2] * tx + mat[6] * ty + mat[10] * tz;
}

#[cfg(test)]
mod tests_c44_matrix__translate__7bdc40 {
    use super::c44_matrix__translate__7bdc40 as translate;

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

/// `C44Matrix::InvertWithScale`: inverts a rotation + uniform-scale +
/// translation matrix. When `|scale - k| < eps` (the matrix is effectively
/// orthonormal, `k` ~ 1.0) this is the plain transpose inverse and delegates to
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

    // Transpose, then row-scale by k / scale².
    let f = k / (scale * scale);
    let i00 = m00 * f;
    let i01 = m10 * f;
    let i02 = m20 * f;
    let i10 = m01 * f;
    let i11 = m11 * f;
    let i12 = m21 * f;
    let i20 = m02 * f;
    let i21 = m12 * f;
    let i22 = m22 * f;

    // Inverse translation: scaled-transpose · (-t), reading the inverse rotation
    // by columns ({i00,i10,i20}, {i01,i11,i21}, {i02,i12,i22}).
    let tx = -src[12];
    let ty = -src[13];
    let tz = -src[14];

    let it0 = i00 * tx + i10 * ty + i20 * tz;
    let it1 = i01 * tx + i11 * ty + i21 * tz;
    let it2 = i02 * tx + i12 * ty + i22 * tz;

    [
        i00, i01, i02, 0.0, //
        i10, i11, i12, 0.0, //
        i20, i21, i22, 0.0, //
        it0, it1, it2, 1.0,
    ]
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

/// Builds the row-major rotation matrix of a quaternion `[x, y, z, w]`
/// (assumed normalized): zero translation row, `m[15] = 1`. Diagonals are
/// evaluated as `(1 - b) - c` with the `1 - 2xx` subterm computed once,
/// mirroring the original's operation order (which differs from the
/// `1 - (b + c)` association used by [`c44_matrix__rotate_quaternion__7bddb0`]).
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

    /// Matches the rotation block the landed `0x7bddb0` kernel builds (same
    /// expansion, different diagonal association) within a ulp-scale tolerance.
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

/// Pivot transformed by the row-vector convention including the translation
/// row, summed in the natural lane order `((x + y) + z) + w` (the billboard
/// pre-pass flavor; the post-rebuild flavor groups differently, see
/// [`bb_pivot_world_zyx__714260`]).
pub fn bb_pivot_world_xyz__714260(m: &[f32; 16], p: &[f32; 3]) -> [f32; 3] {
    [
        (m[0] * p[0] + m[4] * p[1] + m[8] * p[2]) + m[12],
        (m[1] * p[0] + m[5] * p[1] + m[9] * p[2]) + m[13],
        (m[2] * p[0] + m[6] * p[1] + m[10] * p[2]) + m[14],
    ]
}

/// Pivot world position as the billboard rebuild computes it: lane 0 sums
/// `(z + y) + x`, lanes 1-2 sum `(x + z) + y` — the per-lane x87 schedules
/// differ in the original and the groupings are load-bearing for bit
/// fidelity.
pub fn bb_pivot_world_zyx__714260(m: &[f32; 16], p: &[f32; 3]) -> [f32; 3] {
    [
        (m[8] * p[2] + m[4] * p[1] + m[0] * p[0]) + m[12],
        (m[1] * p[0] + m[9] * p[2] + m[5] * p[1]) + m[13],
        (m[2] * p[0] + m[10] * p[2] + m[6] * p[1]) + m[14],
    ]
}

/// Per-row Euclidean lengths of the three rotation rows, each summed
/// `(x² + y²) + z²`.
pub fn bb_row_lengths__714260(m: &[f32; 16]) -> [f32; 3] {
    [
        (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt(),
        (m[4] * m[4] + m[5] * m[5] + m[6] * m[6]).sqrt(),
        (m[8] * m[8] + m[9] * m[9] + m[10] * m[10]).sqrt(),
    ]
}

/// Normalize a basis row given its squared magnitude: skipped only when
/// `|sqrt(sqmag)|` is strictly below the 2⁻²² epsilon (an unordered compare —
/// NaN — still normalizes, poisoning the row like the original).
pub fn bb_normalize_row__714260(row: [f32; 3], sqmag: f32) -> [f32; 3] {
    const EPS: f32 = f32::from_bits(0x3480_0000);
    let s = sqmag.sqrt();
    if !(s.abs() < EPS) {
        let f = 1.0 / s;
        return [row[0] * f, row[1] * f, row[2] * f];
    }
    row
}

/// Row replacement ratio for the project-onto-model-rows billboard mode:
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

/// The local-row squared magnitude feeding [`bb_row_ratio__714260`]:
/// `(x² + z²) + y²` (the original pairs the x and z squares first).
pub fn bb_row_sqmag_xzy__714260(row: &[f32; 3]) -> f32 {
    row[0] * row[0] + row[2] * row[2] + row[1] * row[1]
}

/// Cross product in the swapped orientation the axis-locked billboard modes
/// use for their third row (`row1 × row0` expressed against `a = row0`,
/// `b = row1`).
pub fn bb_cross_swapped__714260(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    [
        a[2] * b[1] - a[1] * b[2],
        a[0] * b[2] - a[2] * b[0],
        a[1] * b[0] - a[0] * b[1],
    ]
}

/// Standard-orientation cross product (the z-locked billboard mode's first
/// row, `a = row2`, `b = row1`).
pub fn bb_cross_std__714260(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Billboard finalize: zero the fourth column, rescale the rebuilt rows by
/// the pre-rebuild lengths, set `m[15] = 1`, and recompute the translation
/// row as `pivot_world − (scaled-basis · pivot)` with the original's
/// `(y + z) + x` lane summation over the already-rescaled (f32-rounded) rows.
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

/// Translation-row fixup after the rotation/scale compose: `trans − (basis ·
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

/// `C44Matrix::Adjugate` — 4×4 adjugate (transpose of the cofactor matrix), the
/// numerator of the inverse. The original evaluates the sixteen 3×3 minors with
/// `C33Matrix::Determinant`, applies eight `fchs` sign flips, and stores each
/// cofactor into its transposed output slot.
///
/// Each minor selects nine source elements in the exact order the original
/// passes them to the determinant (matching the per-call push order of the
/// original), and the eight negations are reproduced as f32 negation:
/// stock negates the 80-bit x87 value before the `fstp` rounds it to f32, and
/// negation is an exact sign-bit flip that commutes with rounding, so `-det(..)`
/// is bit-identical. Input/output are row-major 16-float matrices.
pub fn c44_matrix__adjugate__7bd390(src: &[f32; 16]) -> [f32; 16] {
    let s = src;
    let det = crate::math::matrix33::c33_matrix__determinant__7bc040;
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

/// `C44Matrix::Determinant` (0x7bcf90) — `__fastcall(ecx = this)`, returns the
/// 4×4 determinant in `ST(0)`. (The return is in `ST(0)`, not a hidden
/// `float10*` out-pointer.)
/// Cofactor expansion along row 0: `det = m00·M0 − m01·M1 + m02·M2 − m03·M3`,
/// where each `Mk` is the 3×3 minor with row 0 and column `k` removed, evaluated
/// by `C33Matrix::Determinant` in the exact nine-element order the original
/// pushes (verified against the four `CALL 0x7bc040` push sequences). The
/// accumulation order `((t0 − t1) + t2) − t3` matches the stock
/// `FSUBR`/`FADD`/`FSUBR` chain; the result rounds to `f32` on return.
pub fn c44_matrix__determinant__7bcf90(m: &[f32; 16]) -> f32 {
    let det3 = crate::math::matrix33::c33_matrix__determinant__7bc040;
    // Minor for the m0j cofactor: row 0 and column j removed (3×3 of rows 1..=3).
    let m0 = det3(m[5], m[6], m[7], m[9], m[10], m[11], m[13], m[14], m[15]);
    let m1 = det3(m[4], m[6], m[7], m[8], m[10], m[11], m[12], m[14], m[15]);
    let m2 = det3(m[4], m[5], m[7], m[8], m[9], m[11], m[12], m[13], m[15]);
    let m3 = det3(m[4], m[5], m[6], m[8], m[9], m[10], m[12], m[13], m[14]);
    ((m[0] * m0 - m[1] * m1) + m[2] * m2) - m[3] * m3
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
            let want = det4_ref(m);
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

/// `C44Matrix::InverseScaledByDet` (0x7bd6c0) — `__thiscall(ecx = src, [out],
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
        let d = det4(&m);
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

/// Scales the upper-left 3x3 basis of a 4x4 matrix (4-float row stride) by a
/// single scalar `s`, in place.
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

    /// A 4x4 with distinct, recognisable values: the 3x3 basis, the translation
    /// column at 3/7/11, and the homogeneous row at 12..=15.
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

/// `BuildOrthoProjMatrix` (VA 0x5c3d90) — off-center **orthographic** 4×4
/// row-major projection matrix built into `out`. `k` is the runtime scale
/// constant `_DAT_00801628` (2.0 in the shipped image); read live by the
/// adapter so the kernel stays value-faithful.
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
        // K = 2.0 (the shipped `_DAT_00801628`) ⇒ a symmetric [-1,1] box maps to
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
