//! `matrix34` family kernels.
#![allow(
    non_snake_case,
    // intentional NaN-reject via `!(a >= b)`
    // (differs from `a < b` on NaN), bit-exact source constants kept verbatim,
    // and ABI-dictated parameter counts.
    clippy::neg_cmp_op_on_partial_ord,
    clippy::excessive_precision,
    clippy::too_many_arguments
)]

/// 3x4 affine matrix multiply `out = a * b` (row-major, 12 floats each).
///
/// Indices 0-8 are the product of the two 3x3 rotation blocks; indices 9/10/11
/// are `a`'s 3x3 applied to `b`'s translation row plus `a`'s own translation.
pub fn c34_matrix__multiply__7bae60(a: &[f32; 12], b: &[f32; 12]) -> [f32; 12] {
    let mut out = [0.0f32; 12];
    // Rotation rows 0..2: each output element dots an `a` row against a `b` column.
    for r in 0..3 {
        let a0 = a[3 * r];
        let a1 = a[3 * r + 1];
        let a2 = a[3 * r + 2];
        for c in 0..3 {
            out[3 * r + c] = a0 * b[c] + a1 * b[3 + c] + a2 * b[6 + c];
        }
    }
    // Translation row: a's 3x3 applied to b's translation, plus b's translation.
    let a9 = a[9];
    let a10 = a[10];
    let a11 = a[11];
    for c in 0..3 {
        out[9 + c] = a9 * b[c] + a10 * b[3 + c] + a11 * b[6 + c] + b[9 + c];
    }
    out
}

#[cfg(test)]
mod tests_c34_matrix__multiply__7bae60 {
    use super::c34_matrix__multiply__7bae60;

    const IDENTITY: [f32; 12] = [
        1.0, 0.0, 0.0, // row 0
        0.0, 1.0, 0.0, // row 1
        0.0, 0.0, 1.0, // row 2
        0.0, 0.0, 0.0, // translation
    ];

    fn approx(a: &[f32; 12], b: &[f32; 12], tol: f32) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() <= tol)
    }

    // Independent oracle: recompute via the affine 3x4 spec in f64.
    fn ref_mul(a: &[f32; 12], b: &[f32; 12]) -> [f32; 12] {
        let mut out = [0.0f32; 12];
        for r in 0..3 {
            for c in 0..3 {
                let v = f64::from(a[3 * r]) * f64::from(b[c])
                    + f64::from(a[3 * r + 1]) * f64::from(b[3 + c])
                    + f64::from(a[3 * r + 2]) * f64::from(b[6 + c]);
                out[3 * r + c] = v as f32;
            }
        }
        for c in 0..3 {
            let v = f64::from(a[9]) * f64::from(b[c])
                + f64::from(a[10]) * f64::from(b[3 + c])
                + f64::from(a[11]) * f64::from(b[6 + c])
                + f64::from(b[9 + c]);
            out[9 + c] = v as f32;
        }
        out
    }

    #[test]
    fn identity_left() {
        let m: [f32; 12] = [
            2.0, 3.0, 5.0, 7.0, 11.0, 13.0, 17.0, 19.0, 23.0, 29.0, 31.0, 37.0,
        ];
        let r = c34_matrix__multiply__7bae60(&IDENTITY, &m);
        assert!(approx(&r, &m, 0.0), "I*M must equal M, got {r:?}");
    }

    #[test]
    fn identity_right() {
        let m: [f32; 12] = [
            2.0, 3.0, 5.0, 7.0, 11.0, 13.0, 17.0, 19.0, 23.0, 29.0, 31.0, 37.0,
        ];
        let r = c34_matrix__multiply__7bae60(&m, &IDENTITY);
        assert!(approx(&r, &m, 1e-3), "M*I must equal M, got {r:?}");
    }

    #[test]
    fn matches_oracle_random() {
        let a: [f32; 12] = [
            0.5, -1.25, 2.0, 3.5, -0.75, 1.0, -2.5, 0.25, 4.0, 10.0, -5.0, 0.5,
        ];
        let b: [f32; 12] = [
            -1.0, 0.5, 2.25, 1.5, -3.0, 0.75, 2.0, -0.5, 1.25, -7.0, 8.0, -2.0,
        ];
        let got = c34_matrix__multiply__7bae60(&a, &b);
        let want = ref_mul(&a, &b);
        assert!(approx(&got, &want, 1e-3), "got {got:?} want {want:?}");
    }

    #[test]
    fn associativity() {
        let a: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 3.0];
        let b: [f32; 12] = [0.0, -1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 4.0, 5.0, 6.0];
        let c: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 1.0, 0.0, -1.0, 0.0, 2.0];
        let ab = c34_matrix__multiply__7bae60(&a, &b);
        let ab_c = c34_matrix__multiply__7bae60(&ab, &c);
        let bc = c34_matrix__multiply__7bae60(&b, &c);
        let a_bc = c34_matrix__multiply__7bae60(&a, &bc);
        assert!(
            approx(&ab_c, &a_bc, 1e-3),
            "(A*B)*C={ab_c:?} A*(B*C)={a_bc:?}"
        );
    }

    #[test]
    fn known_translation_compose() {
        let t1: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 3.0];
        let t2: [f32; 12] = [
            1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 10.0, 20.0, 30.0,
        ];
        let r = c34_matrix__multiply__7bae60(&t1, &t2);
        assert_eq!(r[9], 11.0);
        assert_eq!(r[10], 22.0);
        assert_eq!(r[11], 33.0);
        assert_eq!(r[0], 1.0);
        assert_eq!(r[4], 1.0);
        assert_eq!(r[8], 1.0);
    }
}
