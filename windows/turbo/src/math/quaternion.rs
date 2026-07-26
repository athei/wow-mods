//! `quaternion` family kernels.
// Adapter and kernel names mirror the host's C++ symbols verbatim, with `__`
// standing in for the `::`, so the whole module is non-snake-case by construction.
#![allow(non_snake_case)]

// Hamilton product a (x) b for quaternions stored `[x, y, z, w]`.
fn qmul(a: &[f32; 4], b: &[f32; 4]) -> [f32; 4] {
    let (ax, ay, az, aw) = (a[0], a[1], a[2], a[3]);
    let (bx, by, bz, bw) = (b[0], b[1], b[2], b[3]);
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by + ay * bw + az * bx - ax * bz,
        aw * bz + az * bw + ax * by - ay * bx,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}
fn qconj(q: &[f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}
fn qneg(q: &[f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], -q[3]]
}
fn dot4(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}
fn qscale(q: &[f32; 4], s: f32) -> [f32; 4] {
    [q[0] * s, q[1] * s, q[2] * s, q[3] * s]
}
fn qadd(a: &[f32; 4], b: &[f32; 4]) -> [f32; 4] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
}

/// Builds the inner SQUAD control (tangent) quaternions.
///
/// For cubic quaternion spline interpolation across three keyframes `q_prev`,
/// `q_cur`, `q_next` at times `t_prev < t_cur < t_next`.
///
/// `tension`/`continuity`/`bias` are TCB-style spline parameters. Returns `(tangent_in,
/// tangent_out)`, each `tangent = q_cur (x) Exp(weighted log-combination)`.
// Nine parameters because that is the original's argument list at this call:
// three keyframe quaternions, their three times and the three TCB weights, in
// the order the client pushes them. Regrouping them into structs would drop
// the correspondence to the routine this replaces.
#[allow(clippy::too_many_arguments)]
pub fn c4_quaternion__compute_squad_tangents__7c0b60(
    q_prev: &[f32; 4],
    q_cur: &[f32; 4],
    q_next: &[f32; 4],
    t_prev: f32,
    t_cur: f32,
    t_next: f32,
    tension: f32,
    continuity: f32,
    bias: f32,
) -> ([f32; 4], [f32; 4]) {
    let mut log_prev = [0.0f32; 4];
    let mut log_next = [0.0f32; 4];

    if t_prev < t_cur {
        let qp = if dot4(q_prev, q_cur) < 0.0 {
            qneg(q_prev)
        } else {
            *q_prev
        };
        log_prev = c4_quaternion__log__7c04f0(&qmul(&qconj(&qp), q_cur));
    }
    if t_cur < t_next {
        let qn = if dot4(q_cur, q_next) < 0.0 {
            qneg(q_next)
        } else {
            *q_next
        };
        log_next = c4_quaternion__log__7c04f0(&qmul(&qconj(q_cur), &qn));
    }
    if t_cur < t_prev {
        log_prev = log_next;
    }
    if t_next < t_cur {
        log_next = log_prev;
    }

    let mut f_in = 1.0f32;
    let mut f_out = 1.0f32;
    if t_prev < t_next {
        let adj = 1.0 / ((t_next - t_prev) * 0.5);
        f_in = (t_cur - t_prev) * adj;
        f_out = (t_next - t_cur) * adj;
        f_in = (1.0 - f_in) * continuity.abs() + f_in;
        f_out = (1.0 - f_out) * continuity.abs() + f_out;
    }

    let one_minus_tension = 1.0 - tension;
    let w_next_in = (bias + 1.0) * (continuity + 1.0) * one_minus_tension * f_out * 0.5;
    let cb = (1.0 - continuity) * one_minus_tension;
    let w_log_next_in = (1.0 - bias) * cb * f_out * 0.5 - 1.0;
    let w_log_next_out = 1.0 - cb * (bias + 1.0) * f_in * 0.5;
    let w_next_out = (1.0 - bias) * (continuity + 1.0) * one_minus_tension * f_in * (-0.5);

    let combo_in = qscale(
        &qadd(
            &qscale(&log_next, w_log_next_in),
            &qscale(&log_prev, w_next_in),
        ),
        0.5,
    );
    let exp_in = c4_quaternion__exp__7c0450(&[combo_in[0], combo_in[1], combo_in[2]]);
    let tangent_in = qmul(q_cur, &exp_in);

    let combo_out = qscale(
        &qadd(
            &qscale(&log_next, w_next_out),
            &qscale(&log_prev, w_log_next_out),
        ),
        0.5,
    );
    let exp_out = c4_quaternion__exp__7c0450(&[combo_out[0], combo_out[1], combo_out[2]]);
    let tangent_out = qmul(q_cur, &exp_out);

    (tangent_in, tangent_out)
}

#[cfg(test)]
mod tests_c4_quaternion__compute_squad_tangents__7c0b60 {
    use super::{c4_quaternion__compute_squad_tangents__7c0b60 as f, dot4, qconj, qmul};

    fn axis_angle(angle: f32, axis: [f32; 3]) -> [f32; 4] {
        let a = angle * 0.5;
        let s = a.sin();
        [s * axis[0], s * axis[1], s * axis[2], a.cos()]
    }
    fn norm(q: &[f32; 4]) -> f32 {
        (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt()
    }

    #[test]
    fn tangents_are_unit_quaternions() {
        let qp = axis_angle(0.2, [0.0, 0.0, 1.0]);
        let qc = axis_angle(0.9, [0.0, 0.0, 1.0]);
        let qn = axis_angle(1.7, [0.0, 0.0, 1.0]);
        let (ti, to) = f(&qp, &qc, &qn, 0.0, 1.0, 2.0, 0.0, 0.0, 0.0);
        assert!((norm(&ti) - 1.0).abs() < 1e-4, "|ti|={}", norm(&ti));
        assert!((norm(&to) - 1.0).abs() < 1e-4, "|to|={}", norm(&to));
    }

    #[test]
    fn all_equal_keyframes_give_identity_relative_tangents() {
        let qc = axis_angle(0.6, [0.0, 1.0, 0.0]);
        let (ti, to) = f(&qc, &qc, &qc, 0.0, 1.0, 2.0, 0.0, 0.0, 0.0);
        for i in 0..4 {
            assert!(
                (ti[i] - qc[i]).abs() < 1e-4,
                "ti i={} {} vs {}",
                i,
                ti[i],
                qc[i]
            );
            assert!(
                (to[i] - qc[i]).abs() < 1e-4,
                "to i={} {} vs {}",
                i,
                to[i],
                qc[i]
            );
        }
    }

    #[test]
    fn tangent_relative_rotation_stays_in_qcur_frame() {
        let qp = axis_angle(0.1, [1.0, 0.0, 0.0]);
        let qc = axis_angle(0.8, [0.0, 1.0, 0.0]);
        let qn = axis_angle(1.9, [0.0, 0.0, 1.0]);
        let (ti, _to) = f(&qp, &qc, &qn, 0.0, 1.0, 2.0, 0.2, -0.3, 0.4);
        let rel = qmul(&qconj(&qc), &ti);
        assert!((norm(&rel) - 1.0).abs() < 1e-3, "|rel|={}", norm(&rel));
    }

    #[test]
    fn symmetric_keyframes_default_tcb_are_finite() {
        let qp = axis_angle(0.3, [0.0, 0.0, 1.0]);
        let qc = axis_angle(0.6, [0.0, 0.0, 1.0]);
        let qn = axis_angle(0.9, [0.0, 0.0, 1.0]);
        let (ti, to) = f(&qp, &qc, &qn, 0.0, 1.0, 2.0, 0.0, 0.0, 0.0);
        assert!(ti.iter().all(|v| v.is_finite()));
        assert!(to.iter().all(|v| v.is_finite()));
        assert!((norm(&ti) - 1.0).abs() < 1e-4);
        assert!((norm(&to) - 1.0).abs() < 1e-4);
        assert!(ti[0].abs() < 1e-4 && ti[1].abs() < 1e-4, "ti={ti:?}");
    }

    #[test]
    fn dot4_self_is_norm_sq() {
        let q = axis_angle(0.5, [1.0, 0.0, 0.0]);
        assert!((dot4(&q, &q) - 1.0).abs() < 1e-5);
    }
}

/// Exponential map of a rotation vector `(x, y, z)` to a unit quaternion `[x, y, z, w]`.
///
/// `angle = |v|`; if `|sin(angle)| >= eps` scale `xyz` by `sin(angle)/angle`, else copy `xyz`;
/// `w = cos(angle)`. Inverse of [`c4_quaternion__log__7c04f0`].
pub fn c4_quaternion__exp__7c0450(v: &[f32; 3]) -> [f32; 4] {
    const EPS: f32 = 4.768_371_6e-7;
    let angle = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let (s, c) = crate::math::trig::sin_cos(angle);
    if s.abs() >= EPS {
        let k = s / angle;
        [k * v[0], k * v[1], k * v[2], c]
    } else {
        [v[0], v[1], v[2], c]
    }
}

#[cfg(test)]
mod tests_c4_quaternion__exp__7c0450 {
    use super::c4_quaternion__exp__7c0450 as f;

    fn norm(q: &[f32; 4]) -> f32 {
        (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt()
    }

    #[test]
    fn unit_norm_output() {
        for v in [[0.3, -0.7, 0.2], [1.5, 0.0, 0.0], [0.01, 0.02, 0.03]] {
            let q = f(&v);
            assert!((norm(&q) - 1.0).abs() < 1e-5, "v={:?} norm={}", v, norm(&q));
        }
    }

    #[test]
    fn zero_vector_is_identity() {
        let q = f(&[0.0, 0.0, 0.0]);
        assert!(q[0].abs() < 1e-6 && q[1].abs() < 1e-6 && q[2].abs() < 1e-6);
        assert!((q[3] - 1.0).abs() < 1e-6, "w={}", q[3]);
    }

    #[test]
    fn known_half_pi_z() {
        let q = f(&[0.0, 0.0, core::f32::consts::FRAC_PI_2]);
        assert!(q[0].abs() < 1e-6 && q[1].abs() < 1e-6);
        assert!((q[2] - 1.0).abs() < 1e-6, "z={}", q[2]);
        assert!(q[3].abs() < 1e-6, "w={}", q[3]);
    }
}

/// Quaternion logarithm: maps a unit quaternion `[x, y, z, w]` to a rotation vector `[x, y, z, 0]`.
///
/// If `|w| < 1`, let `angle = acos(w)`; if `|sin(angle)| >= eps` scale `xyz` by `angle/sin(angle)`,
/// else copy `xyz`. The `w` lane of the result is always `0`. Inverse of
/// [`c4_quaternion__exp__7c0450`].
pub fn c4_quaternion__log__7c04f0(q: &[f32; 4]) -> [f32; 4] {
    const EPS: f32 = 4.768_371_6e-7;
    let w = q[3];
    if w.abs() < 1.0 {
        let angle = crate::math::trig::acos(w);
        let s = crate::math::trig::sin(angle);
        if s.abs() >= EPS {
            let k = angle / s;
            return [k * q[0], k * q[1], k * q[2], 0.0];
        }
    }
    [q[0], q[1], q[2], 0.0]
}

#[cfg(test)]
mod tests_c4_quaternion__log__7c04f0 {
    use super::c4_quaternion__log__7c04f0 as f;

    fn exp(v: &[f32; 3]) -> [f32; 4] {
        const EPS: f32 = 4.768_371_6e-7;
        let angle = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        let c = angle.cos();
        let s = angle.sin();
        if s.abs() >= EPS {
            let k = s / angle;
            [k * v[0], k * v[1], k * v[2], c]
        } else {
            [v[0], v[1], v[2], c]
        }
    }

    #[test]
    fn w_lane_is_zero() {
        let r = f(&[0.1, 0.2, 0.3, 0.9]);
        assert_eq!(r[3].to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn identity_maps_to_zero_vec() {
        let r = f(&[0.0, 0.0, 0.0, 1.0]);
        assert!(r[0].abs() < 1e-7 && r[1].abs() < 1e-7 && r[2].abs() < 1e-7);
        assert_eq!(r[3].to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn round_trip_log_of_exp() {
        for v in [[0.3, -0.7, 0.2], [0.6, 0.0, 0.0], [0.1, 0.2, -0.15]] {
            let q = exp(&v);
            let back = f(&q);
            for i in 0..3 {
                assert!((back[i] - v[i]).abs() < 1e-4, "v={v:?} back={back:?}");
            }
        }
    }
}

/// Builds a rotation quaternion `[x, y, z, w]` from `angle` and a unit `axis`.
///
/// `a = angle * 0.5`; `w = cos(a)`; `xyz = sin(a) * axis`.
pub fn c4_quaternion__set_from_axis_angle__7c02c0(angle: f32, axis: &[f32; 3]) -> [f32; 4] {
    let a = angle * 0.5;
    let (s, c) = crate::math::trig::sin_cos(a);
    [s * axis[0], s * axis[1], s * axis[2], c]
}

#[cfg(test)]
mod tests_c4_quaternion__set_from_axis_angle__7c02c0 {
    use super::c4_quaternion__set_from_axis_angle__7c02c0 as f;

    fn norm(q: &[f32; 4]) -> f32 {
        (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt()
    }

    #[test]
    fn unit_axis_yields_unit_quat() {
        let q = f(1.2345, &[0.0, 1.0, 0.0]);
        assert!((norm(&q) - 1.0).abs() < 1e-5, "norm={}", norm(&q));
    }

    #[test]
    fn zero_angle_is_identity() {
        let q = f(0.0, &[1.0, 0.0, 0.0]);
        assert!(q[0].abs() < 1e-7);
        assert!(q[1].abs() < 1e-7);
        assert!(q[2].abs() < 1e-7);
        assert!((q[3] - 1.0).abs() < 1e-7);
    }

    #[test]
    fn known_180_about_z() {
        let q = f(core::f32::consts::PI, &[0.0, 0.0, 1.0]);
        assert!(q[0].abs() < 1e-6);
        assert!(q[1].abs() < 1e-6);
        assert!((q[2] - 1.0).abs() < 1e-6, "z={}", q[2]);
        assert!(q[3].abs() < 1e-6, "w={}", q[3]);
    }

    #[test]
    fn known_90_about_x() {
        let h = core::f32::consts::FRAC_1_SQRT_2;
        let q = f(core::f32::consts::FRAC_PI_2, &[1.0, 0.0, 0.0]);
        assert!((q[0] - h).abs() < 1e-6, "x={}", q[0]);
        assert!((q[3] - h).abs() < 1e-6, "w={}", q[3]);
    }
}

/// Spherical linear interpolation between unit quaternions `qa` and `qb` by `t`, `[x, y, z, w]`.
///
/// Takes the shortest arc (flips `qb`'s sign when `qa.qb < 0`). When the quaternions are nearly
/// (anti-)parallel (`sin(theta)` tiny) returns `qa` unchanged.
pub fn c4_quaternion__slerp__7c0570(qa: &[f32; 4], t: f32, qb: &[f32; 4]) -> [f32; 4] {
    const EPS: f32 = 4.768_371_6e-7;
    let mut sign = 1.0f32;
    let mut dot = qa[3] * qb[3] + qa[0] * qb[0] + qa[1] * qb[1] + qa[2] * qb[2];
    if dot < 0.0 {
        sign = -1.0;
        dot = -dot;
    }
    let s = (1.0 - dot * dot).abs().sqrt();
    if s.abs() < EPS {
        return *qa;
    }
    let theta = crate::math::trig::atan2(s, dot);
    let inv = 1.0 / s;
    let c1 = crate::math::trig::sin((1.0 - t) * theta) * inv;
    let c2 = crate::math::trig::sin(theta * t) * inv * sign;
    [
        c2 * qb[0] + c1 * qa[0],
        c2 * qb[1] + c1 * qa[1],
        c2 * qb[2] + c1 * qa[2],
        c2 * qb[3] + c1 * qa[3],
    ]
}

#[cfg(test)]
mod tests_c4_quaternion__slerp__7c0570 {
    use super::c4_quaternion__slerp__7c0570 as f;

    fn norm(q: &[f32; 4]) -> f32 {
        (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt()
    }

    fn axis_angle(angle: f32, axis: [f32; 3]) -> [f32; 4] {
        let a = angle * 0.5;
        let s = a.sin();
        [s * axis[0], s * axis[1], s * axis[2], a.cos()]
    }

    #[test]
    fn endpoints() {
        let qa = axis_angle(0.3, [0.0, 0.0, 1.0]);
        let qb = axis_angle(1.7, [0.0, 0.0, 1.0]);
        let r0 = f(&qa, 0.0, &qb);
        for i in 0..4 {
            assert!(
                (r0[i] - qa[i]).abs() < 1e-4,
                "t=0 i={} {} vs {}",
                i,
                r0[i],
                qa[i]
            );
        }
        let r1 = f(&qa, 1.0, &qb);
        for i in 0..4 {
            assert!(
                (r1[i] - qb[i]).abs() < 1e-4,
                "t=1 i={} {} vs {}",
                i,
                r1[i],
                qb[i]
            );
        }
    }

    #[test]
    fn stays_unit() {
        let qa = axis_angle(0.3, [1.0, 0.0, 0.0]);
        let qb = axis_angle(2.4, [0.0, 1.0, 0.0]);
        for t in [0.25f32, 0.5, 0.75] {
            let r = f(&qa, t, &qb);
            assert!((norm(&r) - 1.0).abs() < 1e-4, "t={} norm={}", t, norm(&r));
        }
    }

    #[test]
    fn midpoint_is_half_angle_about_same_axis() {
        let qa = axis_angle(0.4, [0.0, 0.0, 1.0]);
        let qb = axis_angle(1.6, [0.0, 0.0, 1.0]);
        let mid = f(&qa, 0.5, &qb);
        let expect = axis_angle(1.0, [0.0, 0.0, 1.0]);
        for i in 0..4 {
            assert!(
                (mid[i] - expect[i]).abs() < 1e-4,
                "i={} {} vs {}",
                i,
                mid[i],
                expect[i]
            );
        }
    }

    #[test]
    fn identical_returns_qa() {
        let qa = axis_angle(0.7, [0.0, 1.0, 0.0]);
        let r = f(&qa, 0.5, &qa);
        for i in 0..4 {
            assert!((r[i] - qa[i]).abs() < 1e-4);
        }
    }
}

/// D3DX-style spherical linear interpolation of `q1 -> q2` by `t`.
///
/// Quaternions stored `[x, y, z, w]`; flips `q2`'s weight for the shortest arc.
///
/// Differs from [`c4_quaternion__slerp__7c0570`] in three observable ways: the
/// near-parallel guard tests `1 - |dot|` against `2^-23` (non-strict, NaN
/// falls through to the fallback), the fallback is a plain un-normalized lerp
/// `(1-t)*q1 + sign*t*q2` rather than a copy of `q1`, and the shortest-arc
/// sign applies to the fallback weight too. The guard keeps `|dot| < 1` on the
/// slerp path, so `1 - dot^2 > 0` strictly — the square root cannot see a
/// negative radicand and no clamp is required.
pub fn d3dx_quaternion_slerp__74d11a(q1: &[f32; 4], q2: &[f32; 4], t: f32) -> [f32; 4] {
    // Source threshold constant: bit pattern 0x3400_0000 = 2^-23.
    const NEAR_PARALLEL: f32 = 1.192_092_9e-7;
    let dot = q1[0] * q2[0] + q1[1] * q2[1] + q1[2] * q2[2] + q1[3] * q2[3];
    let sign = if dot < 0.0 { -1.0_f32 } else { 1.0_f32 };
    let d = sign * dot;
    let (c1, c2) = if (1.0 - d) > NEAR_PARALLEL {
        let s = (1.0 - d * d).sqrt();
        let theta = crate::math::trig::atan2(s, d);
        let inv = 1.0 / s;
        (
            crate::math::trig::sin((1.0 - t) * theta) * inv,
            crate::math::trig::sin(theta * t) * inv * sign,
        )
    } else {
        (1.0 - t, sign * t)
    };
    [
        c1 * q1[0] + c2 * q2[0],
        c1 * q1[1] + c2 * q2[1],
        c1 * q1[2] + c2 * q2[2],
        c1 * q1[3] + c2 * q2[3],
    ]
}

#[cfg(test)]
mod tests_d3dx_quaternion_slerp__74d11a {
    use super::d3dx_quaternion_slerp__74d11a as slerp;

    const TOL: f32 = 1e-5;

    fn norm(q: [f32; 4]) -> [f32; 4] {
        let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
        [q[0] / n, q[1] / n, q[2] / n, q[3] / n]
    }

    fn approx4(a: [f32; 4], b: [f32; 4], tol: f32) {
        for i in 0..4 {
            assert!((a[i] - b[i]).abs() <= tol, "lane {i}: {a:?} vs {b:?}");
        }
    }

    /// Endpoints reproduce the inputs.
    #[test]
    fn endpoints() {
        let a = norm([0.2, -0.4, 0.1, 0.88]);
        let b = norm([-0.5, 0.3, 0.6, 0.55]);
        approx4(slerp(&a, &b, 0.0), a, TOL);
        approx4(slerp(&a, &b, 1.0), b, TOL);
    }

    /// Identical quaternions take the lerp fallback and pass through.
    #[test]
    fn parallel_falls_back_to_lerp() {
        let a = norm([0.2, -0.4, 0.1, 0.88]);
        approx4(slerp(&a, &a, 0.37), a, TOL);
    }

    /// Anti-parallel input flips the second weight (shortest arc).
    ///
    /// Blending a quat with its negation returns the first quat's hemisphere
    /// throughout.
    #[test]
    fn antiparallel_sign_flip() {
        let a = norm([0.2, -0.4, 0.1, 0.88]);
        let b = [-a[0], -a[1], -a[2], -a[3]];
        approx4(slerp(&a, &b, 0.37), a, TOL);
    }

    /// Midpoint of a 90-degree rotation pair stays unit-length and bisects.
    #[test]
    fn quarter_arc_midpoint() {
        let a = [0.0, 0.0, 0.0, 1.0];
        let s = core::f32::consts::FRAC_1_SQRT_2;
        let b = [0.0, 0.0, s, s];
        let m = slerp(&a, &b, 0.5);
        let len = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2] + m[3] * m[3]).sqrt();
        assert!((len - 1.0).abs() < 1e-4, "len={len}");
        // Bisector of 0 and 45 degrees in quat space: angle/2 = 22.5 degrees.
        let want = [
            0.0,
            0.0,
            (22.5_f32).to_radians().sin(),
            (22.5_f32).to_radians().cos(),
        ];
        approx4(m, want, 1e-4);
    }

    /// NaN dot routes to the fallback (no sqrt of garbage).
    ///
    /// Mirrors the original's unordered-compare branch.
    #[test]
    fn nan_routes_to_fallback() {
        let a = [f32::NAN, 0.0, 0.0, 1.0];
        let b = [0.0, 0.0, 0.0, 1.0];
        let m = slerp(&a, &b, 0.5);
        // Lane 0 is NaN (propagated); lane 3 is the finite lerp of w.
        assert!(m[0].is_nan());
        assert!((m[3] - 1.0).abs() <= TOL);
    }
}

/// `C4Quaternion::FromMatrix33` (Shoemake trace / largest-diagonal).
///
/// Converts a 3x3 **row-major** rotation matrix `m` (`m[r][c]` at `m[r*3 + c]`, so
/// the diagonal `m[i][i]` is `m[i*4]`) into the unit quaternion `[x, y, z, w]`.
///
/// The x87 original branches with `FCOM`/`FCOMP` + `TEST AH,0x41`, which is
/// unordered-aware; that polarity is load-bearing and reproduced exactly here:
/// - the **trace** branch is taken ONLY on a strictly ordered `trace > 0.0`
///   (equal, less-than, **and unordered/NaN** all fall through to the
///   largest-diagonal branch — `if trace > 0.0`, never `>=`);
/// - each of the two diagonal-pick compares advances the index ONLY on a strict
///   ordered `>` (a tie **or** NaN keeps the earlier index — `if a > b`, never a
///   NaN-naive `<`).
///
/// Constants from the original: the threshold `0.0` at `0x7ffd74`, `1.0` at
/// `0x7ff9d8`, `0.5` at `0x7ffa24`, and the `NEXT` index table `[1, 2, 0]` at
/// `0x81d994`. Pure leaf: no callees, writes only the quat.
///
/// Neither branch spills. The trace sum, the `+ 1.0`, the `FSQRT` and the
/// `FDIVR` reciprocal all stay on the x87 stack, and the only stores are the
/// four output `FSTP m32` (`0x7c01bf`..`0x7c01e5`, `0x7c025d`..`0x7c02a7`), so
/// every intermediate is wide and each lane narrows exactly once. In particular
/// the scale `0.5 / s` is taken from the **unnarrowed** root, not from the `f32`
/// component stored just before it, and the differences and sums feeding the
/// three off-diagonal lanes are `f32`-minus-`f32`, which is exact.
pub fn c4_quaternion__from_matrix33__7c0190(m: &[f32; 9]) -> [f32; 4] {
    // x87 accumulation order: FLD m11; FADD m00; FADD m22, all in-register, so
    // the compare below sees the wide sum.
    let trace = f64::from(m[4]) + f64::from(m[0]) + f64::from(m[8]);
    let mut q = [0.0f32; 4];
    if trace > 0.0 {
        // Ordered `trace > 0`: quaternion dominated by the scalar (w) part.
        let s = (trace + 1.0).sqrt();
        q[3] = super::f64_to_f32(0.5 * s); // w = 0.5 * sqrt(trace + 1)
        let t = 0.5 / s;
        // (m21 - m12) * t
        q[0] = super::f64_to_f32((f64::from(m[7]) - f64::from(m[5])) * t);
        // (m02 - m20) * t
        q[1] = super::f64_to_f32((f64::from(m[2]) - f64::from(m[6])) * t);
        // (m10 - m01) * t
        q[2] = super::f64_to_f32((f64::from(m[3]) - f64::from(m[1])) * t);
    } else {
        // NEXT[i] cycle used by the largest-diagonal branch.
        const NEXT: [usize; 3] = [1, 2, 0];
        // Pick the largest diagonal element; strict `>` so ties/NaN keep i. Both
        // compares are `f32` against `f32` with nothing accumulated, so their
        // width is not in question.
        let mut i = 0usize;
        if m[4] > m[0] {
            i = 1;
        }
        if m[8] > m[i * 4] {
            i = 2;
        }
        let j = NEXT[i];
        let k = NEXT[j];
        // s = sqrt(m[i][i] - m[j][j] - m[k][k] + 1), all four steps in-register.
        let s = (f64::from(m[i * 4]) - f64::from(m[j * 4]) - f64::from(m[k * 4]) + 1.0).sqrt();
        q[i] = super::f64_to_f32(0.5 * s);
        let t = 0.5 / s;
        // w
        q[3] = super::f64_to_f32((f64::from(m[k * 3 + j]) - f64::from(m[j * 3 + k])) * t);
        q[j] = super::f64_to_f32((f64::from(m[i * 3 + j]) + f64::from(m[j * 3 + i])) * t);
        q[k] = super::f64_to_f32((f64::from(m[i * 3 + k]) + f64::from(m[k * 3 + i])) * t);
    }
    q
}

#[cfg(test)]
mod tests_c4_quaternion__from_matrix33__7c0190 {
    use super::c4_quaternion__from_matrix33__7c0190 as from_matrix;

    /// Independent oracle: standard unit-quaternion -> row-major rotation matrix.
    ///
    /// Textbook form, matching the convention `from_matrix` inverts. Used to build
    /// known-rotation inputs and to round-trip the result back to a matrix
    /// (matrices are sign-invariant, sidestepping the ±q ambiguity).
    fn quat_to_mat(q: &[f32; 4]) -> [f32; 9] {
        let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - w * z),
            2.0 * (x * z + w * y),
            2.0 * (x * y + w * z),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - w * x),
            2.0 * (x * z - w * y),
            2.0 * (y * z + w * x),
            1.0 - 2.0 * (x * x + y * y),
        ]
    }

    fn normalize(q: [f32; 4]) -> [f32; 4] {
        let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
        [q[0] / n, q[1] / n, q[2] / n, q[3] / n]
    }

    fn approx9(a: &[f32; 9], b: &[f32; 9], tol: f32) {
        for i in 0..9 {
            assert!((a[i] - b[i]).abs() <= tol, "elem {i}: {} vs {}", a[i], b[i]);
        }
    }

    fn approx4(a: &[f32; 4], b: &[f32; 4], tol: f32) {
        for i in 0..4 {
            assert!((a[i] - b[i]).abs() <= tol, "lane {i}: {} vs {}", a[i], b[i]);
        }
    }

    #[test]
    fn identity_is_unit_w() {
        // trace = 3 > 0 -> trace branch, exact.
        let m = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        assert_eq!(from_matrix(&m), [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn half_turn_x_hits_diagonal_i0() {
        // 180° about X: diag(1,-1,-1), trace = -1 -> diagonal branch, i = 0.
        let m = [1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, -1.0];
        approx4(&from_matrix(&m), &[1.0, 0.0, 0.0, 0.0], 1e-6);
    }

    #[test]
    fn half_turn_y_hits_diagonal_i1() {
        // 180° about Y: diag(-1,1,-1), trace = -1 -> diagonal branch, i = 1.
        let m = [-1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, -1.0];
        approx4(&from_matrix(&m), &[0.0, 1.0, 0.0, 0.0], 1e-6);
    }

    #[test]
    fn half_turn_z_hits_diagonal_i2() {
        // 180° about Z: diag(-1,-1,1), trace = -1 -> diagonal branch, i = 2.
        let m = [-1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 1.0];
        approx4(&from_matrix(&m), &[0.0, 0.0, 1.0, 0.0], 1e-6);
    }

    #[test]
    fn round_trips_via_oracle_both_branches() {
        // Cover both branches: near-identity/small angles (trace > 0) and large
        // angles about each axis and a skew axis (trace <= 0).
        let quats = [
            [0.0, 0.0, 0.0, 1.0],
            normalize([0.2588, 0.0, 0.0, 0.9659]), // 30° about X (trace > 0)
            normalize([0.0, 0.3827, 0.0, 0.9239]), // 45° about Y
            normalize([0.0, 0.0, 0.6428, 0.7660]), // 80° about Z
            normalize([0.5, 0.5, 0.5, 0.5]),       // 120° about (1,1,1)
            normalize([0.9659, 0.0, 0.0, 0.2588]), // 150° about X (trace < 0)
            normalize([0.0, 0.9848, 0.1736, 0.0]), // ~180° skew (diagonal)
            normalize([0.1, -0.4, 0.3, 0.85]),     // arbitrary
        ];
        for q in &quats {
            let m = quat_to_mat(q);
            let out = from_matrix(&m);
            // Output must be a unit quaternion...
            let n = (out[0] * out[0] + out[1] * out[1] + out[2] * out[2] + out[3] * out[3]).sqrt();
            assert!((n - 1.0).abs() <= 1e-4, "not unit: {out:?}");
            // ...whose rotation matrix reproduces the input (sign-invariant).
            approx9(&quat_to_mat(&out), &m, 2e-4);
        }
    }

    #[test]
    fn equal_max_diagonals_keep_earlier_index() {
        // 180° about (1,1,0)/√2: R = [[0,1,0],[1,0,0],[0,0,-1]], diag (0,0,-1).
        // m00 == m11 (tie) -> strict `>` keeps i = 0 (a `>=` would advance to 1).
        // Both indices give the same valid quaternion here; the assertion pins
        // the correct rotation regardless of which was taken.
        let m = [0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, -1.0];
        let out = from_matrix(&m);
        let want = normalize([1.0, 1.0, 0.0, 0.0]);
        // ±q both valid; compare via the reconstructed matrix.
        approx9(&quat_to_mat(&out), &m, 1e-5);
        assert!((out[0].abs() - want[0]).abs() <= 1e-5);
    }

    #[test]
    fn nan_trace_routes_to_diagonal_branch() {
        // A NaN diagonal makes `trace` NaN; `trace > 0.0` is false, so the
        // unordered case falls through to the diagonal branch exactly like the
        // x87 original (never the trace branch). Result carries NaN; no panic.
        let m = [f32::NAN, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        let out = from_matrix(&m);
        assert!(out.iter().any(|v| v.is_nan()));
    }

    /// A rotation matrix that takes the trace branch, as bit patterns.
    ///
    /// Rounded decimal literals are different floats and stop separating the two
    /// widths, so both fixtures are spelled in bits.
    const TRACE_M: [u32; 9] = [
        0x3e9e_b27f,
        0x3f65_a9e0,
        0xbea1_2d1b,
        0xbf72_ae2b,
        0x3ea2_43b5,
        0xbcf7_7945,
        0x3d94_d16c,
        0x3e9d_9596,
        0x3f72_dc91,
    ];

    /// A rotation matrix that takes the largest-diagonal branch with `i = 1`.
    const DIAGONAL_M: [u32; 9] = [
        0xbed1_1c82,
        0x3ec9_722e,
        0xbf52_da5e,
        0x3f65_4f84,
        0xba59_0226,
        0xbee3_9e75,
        0xbe33_cfac,
        0xbf6b_5a42,
        0xbeb4_4565,
    ];

    /// The narrow-per-step form, kept only to prove the fixtures separate.
    ///
    /// Identical term order; every intermediate rounded to `f32` rather than left
    /// on the x87 stack. This is what the kernel computed before the widths were
    /// read off the stores.
    fn per_step(m: &[f32; 9]) -> [f32; 4] {
        const NEXT: [usize; 3] = [1, 2, 0];
        let trace = m[4] + m[0] + m[8];
        let mut q = [0.0f32; 4];
        if trace > 0.0 {
            let s = (trace + 1.0).sqrt();
            q[3] = 0.5 * s;
            let t = 0.5 / s;
            q[0] = (m[7] - m[5]) * t;
            q[1] = (m[2] - m[6]) * t;
            q[2] = (m[3] - m[1]) * t;
        } else {
            let mut i = 0usize;
            if m[4] > m[0] {
                i = 1;
            }
            if m[8] > m[i * 4] {
                i = 2;
            }
            let j = NEXT[i];
            let k = NEXT[j];
            let s = (m[i * 4] - m[j * 4] - m[k * 4] + 1.0).sqrt();
            q[i] = 0.5 * s;
            let t = 0.5 / s;
            q[3] = (m[k * 3 + j] - m[j * 3 + k]) * t;
            q[j] = (m[i * 3 + j] + m[j * 3 + i]) * t;
            q[k] = (m[i * 3 + k] + m[k * 3 + i]) * t;
        }
        q
    }

    /// Trace branch: the whole chain stays wide, each lane narrows at its store.
    #[test]
    fn trace_branch_narrows_once_at_each_store() {
        let m = TRACE_M.map(f32::from_bits);
        let want = [0x3dd7_a8f7u32, 0xbdf7_39d2, 0xbf13_2939, 0x3f4d_6bdb];
        let got = from_matrix(&m);
        for i in 0..4 {
            assert_eq!(got[i].to_bits(), want[i], "lane {i}");
        }
    }

    /// Largest-diagonal branch, `i = 1`: same, including `w` off the wide scale.
    #[test]
    fn diagonal_branch_narrows_once_at_each_store() {
        let m = DIAGONAL_M.map(f32::from_bits);
        let want = [0x3ef8_cb63u32, 0x3f29_cbc5, 0xbf03_9b71, 0xbe7a_2059];
        let got = from_matrix(&m);
        for i in 0..4 {
            assert_eq!(got[i].to_bits(), want[i], "lane {i}");
        }
    }

    #[test]
    fn fixtures_separate_from_the_per_step_f32_form() {
        // Without this the width half of the fix is unpinned: the tolerance-based
        // tests above pass under either form.
        for (name, bits) in [("trace", TRACE_M), ("diagonal", DIAGONAL_M)] {
            let m = bits.map(f32::from_bits);
            let stock = from_matrix(&m);
            let narrow = per_step(&m);
            assert!(
                (0..4).any(|i| stock[i].to_bits() != narrow[i].to_bits()),
                "{name} fixture no longer separates narrow-once from narrow-per-step"
            );
        }
    }
}
