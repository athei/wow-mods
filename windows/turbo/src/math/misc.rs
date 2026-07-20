//! `misc` family kernels.
#![allow(
    non_snake_case,
    // intentional NaN-reject via `!(a >= b)`
    // (differs from `a < b` on NaN), bit-exact source constants kept verbatim,
    // and ABI-dictated parameter counts.
    clippy::neg_cmp_op_on_partial_ord,
    clippy::excessive_precision,
    clippy::too_many_arguments
)]

/// Builds a 2x2 CCW rotation matrix from `angle` (radians), row-major: `[cos, sin, -sin, cos]`.
pub fn build_rotation_matrix2x2__7c42f0(angle: f32) -> [f32; 4] {
    let (s, c) = crate::math::trig::sin_cos(angle);
    [c, s, -s, c]
}

#[cfg(test)]
mod tests_build_rotation_matrix2x2__7c42f0 {
    use super::build_rotation_matrix2x2__7c42f0 as build;

    const TOL: f32 = 1e-5;

    fn approx(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn zero_angle_is_identity() {
        let m = build(0.0);
        assert!(approx(m[0], 1.0, TOL));
        assert!(approx(m[1], 0.0, TOL));
        assert!(approx(m[2], 0.0, TOL));
        assert!(approx(m[3], 1.0, TOL));
    }

    #[test]
    fn quarter_turn_known_value() {
        // 90 degrees: cos=0, sin=1 -> [0, 1, -1, 0]
        let m = build(core::f32::consts::FRAC_PI_2);
        assert!(approx(m[0], 0.0, TOL));
        assert!(approx(m[1], 1.0, TOL));
        assert!(approx(m[2], -1.0, TOL));
        assert!(approx(m[3], 0.0, TOL));
    }

    #[test]
    fn structure_cos_sin_negsin_cos() {
        // Verify the [c, s, -s, c] packing for an arbitrary angle.
        let a = 0.7_f32;
        let m = build(a);
        assert!(approx(m[0], a.cos(), TOL));
        assert!(approx(m[1], a.sin(), TOL));
        assert!(approx(m[2], -a.sin(), TOL));
        assert!(approx(m[3], a.cos(), TOL));
        assert!(approx(m[0], m[3], TOL));
        assert!(approx(m[1], -m[2], TOL));
    }

    #[test]
    fn orthonormal_rows() {
        // Rows of a rotation matrix are unit length, orthogonal, det = +1.
        for &a in &[0.0_f32, 0.3, 1.1, 2.5, -1.7, 3.0] {
            let m = build(a);
            let r0 = (m[0] * m[0] + m[1] * m[1]).sqrt();
            let r1 = (m[2] * m[2] + m[3] * m[3]).sqrt();
            assert!(approx(r0, 1.0, TOL), "row0 norm at {a}");
            assert!(approx(r1, 1.0, TOL), "row1 norm at {a}");
            let dot = m[0] * m[2] + m[1] * m[3];
            assert!(approx(dot, 0.0, TOL), "row dot at {a}");
            let det = m[0] * m[3] - m[1] * m[2];
            assert!(approx(det, 1.0, TOL), "det at {a}");
        }
    }

    #[test]
    fn applies_as_expected() {
        // M (row-major) applied to v=(1,0) at 90deg gives (cos, -sin) = (0, -1).
        let m = build(core::f32::consts::FRAC_PI_2);
        let v = [1.0_f32, 0.0_f32];
        let out = [m[0] * v[0] + m[1] * v[1], m[2] * v[0] + m[3] * v[1]];
        assert!(approx(out[0], 0.0, TOL));
        assert!(approx(out[1], -1.0, TOL));
    }
}

/// Maps a travel `distance` to an output displacement along a transport path.
///
/// Using a trapezoidal speed profile defined by `cruise` speed and `accel`.
///
/// `accel_enabled` / `decel_enabled` select ramp-up / ramp-down phases. Returns
/// the resulting displacement (as `f64`, matching the original x87 return) and a
/// phase code: `0xa2` accelerate, `0xa3` cruise, `0xa4` decelerate.
pub fn cg_game_object_display_mo_transport__eval_move_distance__5f8dc0(
    cruise: f32,
    accel: f32,
    distance: f32,
    total: f32,
    accel_enabled: bool,
    decel_enabled: bool,
) -> (f64, u32) {
    const HALF: f32 = 0.5;

    if !accel_enabled {
        if !decel_enabled {
            // Pure cruise: constant speed over the whole path.
            return (f64::from(distance * cruise), 0xa3);
        }
        // Decelerate-only profile.
        let mut ramp = cruise / accel;
        if total < ramp {
            ramp = total;
        }
        let cruise_end = total - ramp;
        if distance <= cruise_end {
            return (f64::from(distance * cruise), 0xa3);
        }
        let v0 = ramp * accel;
        let rem = distance - cruise_end;
        let avg = ((v0 + v0) - rem * accel) * HALF;
        return (f64::from(cruise_end * cruise + avg * rem), 0xa4);
    }

    let ramp = cruise / accel;

    if !decel_enabled {
        // Accelerate-only profile.
        let mut accel_end = ramp;
        if total < ramp {
            accel_end = total;
        }
        if distance <= accel_end {
            return (f64::from(distance * accel * HALF * distance), 0xa2);
        }
        return (
            f64::from(cruise * HALF * ramp + (distance - ramp) * cruise),
            0xa3,
        );
    }

    // Full trapezoid: accelerate, cruise, decelerate.
    let half_total = total * HALF;
    let mut ramp_len = half_total;
    if ramp <= half_total {
        ramp_len = ramp;
    }
    if distance <= total - ramp_len {
        if distance <= ramp_len {
            return (f64::from(distance * accel * HALF * distance), 0xa2);
        }
        return (
            f64::from(cruise * HALF * ramp + (distance - ramp) * cruise),
            0xa3,
        );
    }
    let v0 = ramp_len * accel;
    let rem = distance - (total - ramp_len);
    let avg = ((v0 + v0) - rem * accel) * HALF;
    (
        f64::from(v0 * HALF * ramp_len + (total - (ramp_len + ramp_len)) * cruise + avg * rem),
        0xa4,
    )
}

#[cfg(test)]
mod tests_cg_game_object_display_mo_transport__eval_move_distance__5f8dc0 {
    use super::cg_game_object_display_mo_transport__eval_move_distance__5f8dc0 as eval;

    const TOL: f64 = 1e-3;

    #[test]
    fn pure_cruise_phase_and_value() {
        // No accel, no decel: linear distance * cruise, phase 0xa3.
        let (d, p) = eval(4.0, 2.0, 3.0, 100.0, false, false);
        assert_eq!(p, 0xa3);
        assert!((d - 12.0).abs() < TOL, "got {d}");
    }

    #[test]
    fn accel_only_quadratic_phase() {
        // distance well inside the accel ramp -> phase 0xa2, value = d*a*0.5*d.
        let cruise = 10.0;
        let accel = 2.0;
        let dist = 1.0; // < ramp (cruise/accel = 5.0)
        let (d, p) = eval(cruise, accel, dist, 100.0, true, false);
        assert_eq!(p, 0xa2);
        let expect = f64::from(dist * accel * 0.5 * dist);
        assert!((d - expect).abs() < TOL, "got {d} want {expect}");
    }

    #[test]
    fn accel_only_cruise_segment_phase() {
        // distance past the accel ramp -> phase 0xa3.
        let cruise = 10.0;
        let accel = 2.0;
        let ramp = cruise / accel; // 5.0
        let dist = 8.0; // > ramp
        let (d, p) = eval(cruise, accel, dist, 100.0, true, false);
        assert_eq!(p, 0xa3);
        let expect = f64::from(cruise * 0.5 * ramp + (dist - ramp) * cruise);
        assert!((d - expect).abs() < TOL, "got {d} want {expect}");
    }

    #[test]
    fn decel_only_cruise_then_decel() {
        let cruise = 10.0;
        let accel = 2.0;
        let total = 100.0;
        // cruise region: distance small -> phase 0xa3 linear.
        let (d1, p1) = eval(cruise, accel, 3.0, total, false, true);
        assert_eq!(p1, 0xa3);
        assert!((d1 - f64::from(3.0_f32 * cruise)).abs() < TOL);
        // decel region: distance near the end -> phase 0xa4.
        let (_d2, p2) = eval(cruise, accel, 99.0, total, false, true);
        assert_eq!(p2, 0xa4);
    }

    #[test]
    fn full_trapezoid_three_phases() {
        let cruise = 10.0;
        let accel = 2.0;
        let total = 100.0;
        let (_a, pa) = eval(cruise, accel, 1.0, total, true, true);
        assert_eq!(pa, 0xa2, "early -> accel");
        let (_c, pc) = eval(cruise, accel, 50.0, total, true, true);
        assert_eq!(pc, 0xa3, "middle -> cruise");
        let (_d, pd) = eval(cruise, accel, 99.0, total, true, true);
        assert_eq!(pd, 0xa4, "late -> decel");
    }

    #[test]
    fn monotonic_displacement_over_distance() {
        // Output displacement is non-decreasing as input distance grows.
        let cruise = 10.0;
        let accel = 2.0;
        let total = 100.0;
        let mut prev = f64::NEG_INFINITY;
        let mut x = 0.0_f32;
        while x <= total {
            let (d, _p) = eval(cruise, accel, x, total, true, true);
            assert!(d >= prev - TOL, "non-monotonic at x={x}: {d} < {prev}");
            prev = d;
            x += 0.5;
        }
    }

    #[test]
    fn accel_value_continuous_at_ramp_boundary() {
        // The accel-only quadratic and cruise formulas should agree at d == ramp.
        let cruise = 12.0;
        let accel = 3.0;
        let ramp = cruise / accel; // 4.0
        // Just below and at the boundary, values should be close (continuity).
        let (lo, _) = eval(cruise, accel, ramp - 1e-3, 1000.0, true, false);
        let (hi, _) = eval(cruise, accel, ramp + 1e-3, 1000.0, true, false);
        assert!((hi - lo).abs() < 0.1, "discontinuity: lo={lo} hi={hi}");
    }

    #[test]
    fn full_trapezoid_endpoint_equals_total_times_cruise_when_no_ramp() {
        // If accel is huge, ramps collapse; total displacement ~ total*cruise.
        let cruise = 5.0;
        let accel = 100000.0; // ramp ~ 0
        let total = 50.0;
        let (d, _p) = eval(cruise, accel, total, total, true, true);
        let expect = f64::from(total * cruise);
        assert!((d - expect).abs() < 0.5, "got {d} want {expect}");
    }
}

/// Piecewise-linear sample of a circular gradient of `(position, value)` stop pairs.
///
/// 8-byte stride: `stops[i][0]` is the position in `[0,1)`, `stops[i][1]` the
/// value. `t` is clamped to `[0, 1]`, the bracketing stops are found (wrapping
/// around the array ends), and the value is lerped; if the bracket straddles the
/// wrap seam the positions are unwrapped by `+1.0`. A degenerate bracket
/// (positions within `EPS`) returns the lower stop's value unchanged.
pub fn cloud_gradient_sample__6cf6c0(stops: &[[f32; 2]], t: f32) -> f32 {
    // Span below which the bracket is treated as degenerate — the host's `.data`
    // guard constant at 0x801360 (0.001), compared with `FCOMP` before the divide.
    const EPS: f32 = f32::from_bits(0x3a83_126f);

    let count = stops.len();
    if count == 0 {
        return 0.0;
    }

    // Clamp `t` to [0, 1] with the original's exact branch order.
    let t = if t >= 0.0 {
        if t >= 1.0 { 1.0 } else { t }
    } else {
        0.0
    };

    // First stop whose position is >= t (the original's `t <= pos` break).
    let mut hi = 0usize;
    while hi < count {
        if t <= stops[hi][0] {
            break;
        }
        hi += 1;
    }

    // Resolve the bracketing (lo, hi) indices, wrapping at both ends.
    let (lo, hi) = if hi == count || hi == 0 {
        (count - 1, 0)
    } else {
        (hi - 1, hi)
    };

    // Position span hi - lo, unwrapped across the seam if it went negative.
    let mut span_pos = stops[hi][0] - stops[lo][0];
    if span_pos < 0.0 {
        span_pos += 1.0;
    }

    if span_pos.abs() < EPS {
        return stops[lo][1];
    }

    // Offset of t from the lower stop, unwrapped likewise.
    let mut offset = t - stops[lo][0];
    if offset < 0.0 {
        offset += 1.0;
    }

    let lo_val = stops[lo][1];
    let hi_val = stops[hi][1];
    let frac = offset / span_pos;
    if hi_val < lo_val {
        lo_val - (lo_val - hi_val) * frac
    } else {
        (hi_val - lo_val) * frac + lo_val
    }
}

#[cfg(test)]
mod tests_cloud_gradient_sample__6cf6c0 {
    use super::cloud_gradient_sample__6cf6c0 as sample;

    fn approx(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    // Three evenly spaced stops with distinct values; the gradient is monotone
    // increasing on [0, 0.5] then decreasing back, with a wrap segment from the
    // last stop (pos 0.75) around to the first (pos 0.0).
    const STOPS: [[f32; 2]; 3] = [[0.0, 10.0], [0.5, 30.0], [0.75, 20.0]];

    #[test]
    fn hits_stop_values_exactly() {
        // At a stop position t == pos, t <= pos picks that stop as `hi`, so the
        // lerp lands on its value (offset spans the whole segment, frac == 1).
        assert!(approx(sample(&STOPS, 0.5), 30.0, 1e-4));
        assert!(approx(sample(&STOPS, 0.75), 20.0, 1e-4));
    }

    #[test]
    fn midpoint_is_average_within_segment() {
        // Midpoint of [0.0,10.0]..[0.5,30.0] => 20.0.
        assert!(approx(sample(&STOPS, 0.25), 20.0, 1e-4));
        // Midpoint of [0.5,30.0]..[0.75,20.0] => 25.0.
        assert!(approx(sample(&STOPS, 0.625), 25.0, 1e-4));
    }

    #[test]
    fn clamps_below_zero_and_above_one() {
        // t < 0 clamps to 0.0; t > 1 clamps to 1.0.
        let at_zero = sample(&STOPS, 0.0);
        assert!(approx(sample(&STOPS, -5.0), at_zero, 1e-4));
        // t=0.0: lo=2 (pos .75 val 20), hi=0 (pos 0 val 10). offset = 0-.75 -> .25,
        // span = .25, frac = 1 => value hi = 10.0.
        assert!(approx(at_zero, 10.0, 1e-4));
        let at_one = sample(&STOPS, 1.0);
        assert!(approx(sample(&STOPS, 2.0), at_one, 1e-4));
    }

    #[test]
    fn wrap_segment_is_continuous_and_bounded() {
        // Sweep the wrap segment t in (0.75, 1.0]; values must stay within the
        // [10, 20] envelope of its two endpoints and move monotonically.
        let mut prev = sample(&STOPS, 0.75);
        for k in 1..=20u32 {
            let t = 0.75 + 0.25 * f32::from(k as u16) / 20.0;
            let v = sample(&STOPS, t);
            assert!((10.0 - 1e-4..=20.0 + 1e-4).contains(&v), "t={t} v={v}");
            // value goes from 20 down to 10, so non-increasing.
            assert!(v <= prev + 1e-4, "t={t} v={v} prev={prev}");
            prev = v;
        }
    }

    #[test]
    fn degenerate_span_returns_lower_value() {
        // Two stops at the same position: span_pos == 0 < EPS, returns lo value.
        let stops = [[0.3, 7.0], [0.3, 99.0]];
        let v = sample(&stops, 0.3);
        assert!(approx(v, 99.0, 1e-4) || approx(v, 7.0, 1e-4), "{v}");
    }

    #[test]
    fn single_stop_is_constant() {
        let stops = [[0.4, 42.0]];
        for &t in &[0.0f32, 0.4, 0.9, 1.0] {
            assert!(approx(sample(&stops, t), 42.0, 1e-4), "t={t}");
        }
    }

    #[test]
    fn empty_is_zero() {
        let stops: [[f32; 2]; 0] = [];
        assert_eq!(sample(&stops, 0.5), 0.0);
    }
}

/// Heading (radians) of the 2D vector from `from` to `to`: `atan2(dy, dx)`.
///
/// Mirrors the original axis-snapping: when a component is within `EPS` of
/// zero the result is one of the exact constants `0`, `pi/2`, `pi`, `3pi/2`
/// instead of routing through `atan2`. The general path uses the same
/// `atan2(dy, dx)` (range `(-pi, pi]`) as the original x87 `FPATAN`.
pub fn heading_from_delta2_d__47f220(from: &[f32; 2], to: &[f32; 2]) -> f64 {
    // f32 epsilon == 2^-22, the original near-axis snap threshold.
    const EPS: f32 = 2.384_185_791_015_625e-7;
    const PI: f64 = 3.141_592_741_012_573_2; // pi rounded to f32, then widened.

    let dx = to[0] - from[0];
    let dy = to[1] - from[1];

    if dx.abs() < EPS {
        // Near-vertical: snap to +pi/2 (up) or 3pi/2 (down).
        if dy < 0.0 {
            return 1.5 * PI;
        }
        return 0.5 * PI;
    }

    if dy.abs() < EPS {
        // Near-horizontal: snap to pi (left) or 0 (right).
        if to[0] < from[0] {
            return PI;
        }
        return 0.0;
    }

    // General case: standard atan2, computed in f64 to track the original
    // x87 80-bit FPATAN precision.
    (f64::from(dy)).atan2(f64::from(dx))
}

#[cfg(test)]
mod tests_heading_from_delta2_d__47f220 {
    use std::f64::consts::PI;

    use super::heading_from_delta2_d__47f220 as h;

    const TOL: f64 = 1e-6;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < TOL
    }

    #[test]
    fn axis_east_is_zero() {
        // due +x: dy ~ 0, dx > 0 -> 0
        assert!(close(h(&[0.0, 0.0], &[5.0, 0.0]), 0.0));
    }

    #[test]
    fn axis_west_is_pi() {
        // due -x: dy ~ 0, dx < 0 -> pi
        assert!(close(h(&[0.0, 0.0], &[-5.0, 0.0]), PI));
    }

    #[test]
    fn axis_north_is_half_pi() {
        // dx ~ 0, dy > 0 -> pi/2
        assert!(close(h(&[0.0, 0.0], &[0.0, 5.0]), 0.5 * PI));
    }

    #[test]
    fn axis_south_is_three_half_pi() {
        // dx ~ 0, dy < 0 -> 3pi/2 (NOT -pi/2; original uses 1.5*pi here)
        assert!(close(h(&[0.0, 0.0], &[0.0, -5.0]), 1.5 * PI));
    }

    #[test]
    fn general_quadrant1_is_atan2() {
        // dx=1, dy=1 -> pi/4
        let r = h(&[0.0, 0.0], &[1.0, 1.0]);
        assert!(close(r, PI / 4.0), "got {r}");
    }

    #[test]
    fn general_quadrant2() {
        // dx=-1, dy=1 -> 3pi/4
        let r = h(&[0.0, 0.0], &[-1.0, 1.0]);
        assert!(close(r, 3.0 * PI / 4.0), "got {r}");
    }

    #[test]
    fn general_quadrant3_negative_range() {
        // dx=-1, dy=-1 -> atan2 range is (-pi,pi] => -3pi/4
        let r = h(&[0.0, 0.0], &[-1.0, -1.0]);
        assert!(close(r, -3.0 * PI / 4.0), "got {r}");
    }

    #[test]
    fn general_quadrant4_negative() {
        // dx=1, dy=-1 -> -pi/4
        let r = h(&[0.0, 0.0], &[1.0, -1.0]);
        assert!(close(r, -PI / 4.0), "got {r}");
    }

    #[test]
    fn translation_invariant() {
        // Heading depends only on the delta, not absolute position.
        let a = h(&[10.0, 20.0], &[13.0, 24.0]);
        let b = h(&[0.0, 0.0], &[3.0, 4.0]);
        assert!(close(a, b), "{a} vs {b}");
    }

    #[test]
    fn known_value_3_4() {
        // dx=3, dy=4 -> atan2(4,3) ~= 0.927295218
        let r = h(&[0.0, 0.0], &[3.0, 4.0]);
        assert!(close(r, 0.927_295_218_001_612), "got {r}");
    }

    #[test]
    fn reverse_adds_pi_magnitude() {
        // heading(a->b) and heading(b->a) differ by pi (mod 2pi) in general case.
        let fwd = h(&[0.0, 0.0], &[2.0, 3.0]);
        let rev = h(&[2.0, 3.0], &[0.0, 0.0]);
        let diff = (fwd - rev).abs();
        assert!(close(diff, PI), "diff={diff}");
    }
}

/// Clamped linear interpolation: `a + (b - a) * clamp(t, 0.0, 1.0)`.
pub fn lerp_clamp01__674ba0(a: f32, b: f32, t: f32) -> f32 {
    let c = t.clamp(0.0, 1.0);
    (b - a) * c + a
}

#[cfg(test)]
mod tests_lerp_clamp01__674ba0 {
    use super::lerp_clamp01__674ba0 as lc;

    fn close(x: f32, y: f32) -> bool {
        (x - y).abs() <= 1e-5 * (1.0 + x.abs().max(y.abs()))
    }

    #[test]
    fn endpoints() {
        // t=0 -> a, t=1 -> b
        assert!(close(lc(2.0, 5.0, 0.0), 2.0));
        assert!(close(lc(2.0, 5.0, 1.0), 5.0));
    }

    #[test]
    fn midpoint() {
        assert!(close(lc(2.0, 6.0, 0.5), 4.0));
        assert!(close(lc(-1.0, 1.0, 0.5), 0.0));
    }

    #[test]
    fn clamp_below() {
        // negative t clamps to 0 -> returns a
        assert!(close(lc(3.0, 9.0, -2.5), 3.0));
        assert!(close(lc(3.0, 9.0, -0.001), 3.0));
    }

    #[test]
    fn clamp_above() {
        // t>1 clamps to 1 -> returns b
        assert!(close(lc(3.0, 9.0, 4.0), 9.0));
        assert!(close(lc(3.0, 9.0, 1.0001), 9.0));
    }

    #[test]
    fn monotone_in_clamped_range() {
        // within [0,1], result moves monotonically from a toward b
        let (a, b) = (1.0f32, 10.0f32);
        let mut prev = lc(a, b, 0.0);
        let mut t = 0.05f32;
        while t <= 1.0 {
            let cur = lc(a, b, t);
            assert!(cur >= prev - 1e-5);
            prev = cur;
            t += 0.05;
        }
    }

    #[test]
    fn equal_endpoints_constant() {
        // a == b -> always a regardless of t
        for &tt in &[-1.0f32, 0.0, 0.3, 1.0, 2.0] {
            assert!(close(lc(7.5, 7.5, tt), 7.5));
        }
    }

    #[test]
    fn known_values() {
        assert!(close(lc(0.0, 100.0, 0.25), 25.0));
        assert!(close(lc(10.0, 20.0, 0.75), 17.5));
    }
}

/// Branch-balanced linear interpolation `a + (b - a) * t`.
///
/// Mirroring the reference's two operand orderings.
///
/// The original branches purely to keep the x87 operand order identical in both
/// directions (`a <= b` computes `(b - a) * t + a`; otherwise `a - (a - b) * t`),
/// which keeps rounding stable when `t` is at the endpoints. The two forms are
/// algebraically the lerp; the branch is preserved so the rounding matches.
pub fn math_lerp_float__6d15f0(a: f32, b: f32, t: f32) -> f32 {
    if a <= b {
        (b - a) * t + a
    } else {
        a - (a - b) * t
    }
}

#[cfg(test)]
mod tests_math_lerp_float__6d15f0 {
    use super::math_lerp_float__6d15f0 as lerp;

    #[test]
    fn endpoints_are_exact() {
        // t = 0 returns a, t = 1 returns b, in both branch directions.
        assert_eq!(lerp(2.0, 5.0, 0.0).to_bits(), 2.0f32.to_bits());
        assert_eq!(lerp(2.0, 5.0, 1.0).to_bits(), 5.0f32.to_bits());
        assert_eq!(lerp(5.0, 2.0, 0.0).to_bits(), 5.0f32.to_bits());
        assert_eq!(lerp(5.0, 2.0, 1.0).to_bits(), 2.0f32.to_bits());
    }

    #[test]
    fn midpoint() {
        assert!((lerp(0.0, 10.0, 0.5) - 5.0).abs() < 1e-5);
        assert!((lerp(10.0, 0.0, 0.5) - 5.0).abs() < 1e-5);
        assert!((lerp(-4.0, 4.0, 0.5) - 0.0).abs() < 1e-5);
    }

    #[test]
    fn known_values() {
        // a=1, b=3, t=0.25 -> 1 + 2*0.25 = 1.5
        assert!((lerp(1.0, 3.0, 0.25) - 1.5).abs() < 1e-5);
        // descending branch: a=3, b=1, t=0.25 -> 3 - 2*0.25 = 2.5
        assert!((lerp(3.0, 1.0, 0.25) - 2.5).abs() < 1e-5);
    }

    #[test]
    fn extrapolation_monotonic() {
        // For a <= b and increasing t, output is non-decreasing.
        let (a, b) = (1.0f32, 4.0f32);
        let mut prev = lerp(a, b, -0.5);
        let mut tt = -0.4f32;
        while tt <= 1.5 {
            let cur = lerp(a, b, tt);
            assert!(cur >= prev - 1e-5, "non-monotonic at t={tt}");
            prev = cur;
            tt += 0.1;
        }
    }

    #[test]
    fn equal_endpoints_constant() {
        // a == b takes the `a <= b` branch and returns a for any t.
        for &t in &[-1.0f32, 0.0, 0.5, 1.0, 2.0] {
            assert_eq!(lerp(7.0, 7.0, t).to_bits(), 7.0f32.to_bits());
        }
    }
}

/// `x / 3`, computed as `x * 0.33333334` (the `f32` nearest `1/3`).
///
/// The reference multiplies `x` by a fixed 4-byte float constant
/// `0.33333334` (`0x3eaaaaab`), the single-precision rounding of `1/3`, rather
/// than dividing. Computing in `f32` matches the original's lane width and the
/// stored constant exactly.
pub fn math_scale_one_third__6d6e50(x: f32) -> f32 {
    const ONE_THIRD: f32 = 0.333_333_34;
    x * ONE_THIRD
}

#[cfg(test)]
mod tests_math_scale_one_third__6d6e50 {
    use super::math_scale_one_third__6d6e50 as scale;

    #[test]
    fn constant_is_one_third_f32() {
        // The literal is bit-exact the f32 rounding of 1/3 (0x3eaaaaab).
        assert_eq!(0.333_333_34f32.to_bits(), 0x3eaa_aaab);
    }

    #[test]
    fn known_values() {
        assert_eq!(scale(0.0).to_bits(), 0.0f32.to_bits());
        assert!((scale(3.0) - 1.0).abs() < 1e-6);
        assert!((scale(9.0) - 3.0).abs() < 1e-6);
        assert!((scale(-6.0) + 2.0).abs() < 1e-6);
    }

    #[test]
    fn linear_scaling() {
        // f(k*x) = k*f(x) for finite k (exact under the same single multiply
        // up to rounding).
        let x = 1.5f32;
        assert!((scale(2.0 * x) - 2.0 * scale(x)).abs() < 1e-6);
    }

    #[test]
    fn additive_over_sum() {
        // f(a) + f(b) ≈ f(a + b) within single-precision tolerance.
        let (a, b) = (4.0f32, 5.0f32);
        assert!((scale(a) + scale(b) - scale(a + b)).abs() < 1e-5);
    }

    #[test]
    fn matches_naive_mul() {
        for &v in &[1.0f32, 2.0, 7.5, -3.25, 100.0, 0.001] {
            assert_eq!(scale(v).to_bits(), (v * 0.333_333_34f32).to_bits());
        }
    }
}

/// Half-open AABB overlap predicate.
///
/// The query's `max` corner must lie strictly below the object's stored `max`
/// corner on every axis, and the object's stored `min` corner must not exceed
/// the query's `min` corner on every axis.
///
/// Mirrors the reference predicate's exact comparison polarity. Both groups are
/// **inclusive** (`<=`): the original compares each axis with `comiss` and the
/// upper-bound branches are `TEST AH,0x41; JP` — the `C0|C3` (less-or-equal)
/// parity idiom, *not* strict less — so a query corner exactly on the object's
/// `max` face still counts (touching boxes overlap). `query` packs the upper
/// corner in lanes `0..=2` and the lower corner in lanes `3..=5`. A NaN input
/// makes every comparison false, so a NaN misses (`false`), as the original does.
pub fn object_test_value_threshold__6a4670(
    obj_min: &[f32; 3],
    obj_max: &[f32; 3],
    query: &[f32; 6],
) -> bool {
    query[0] <= obj_max[0]
        && query[1] <= obj_max[1]
        && query[2] <= obj_max[2]
        && obj_min[0] <= query[3]
        && obj_min[1] <= query[4]
        && obj_min[2] <= query[5]
}

#[cfg(test)]
mod tests_object_test_value_threshold__6a4670 {
    use super::object_test_value_threshold__6a4670 as f;

    // A query strictly inside the object's bounds passes (all tests inclusive).
    #[test]
    fn strictly_inside_passes() {
        let obj_min = [0.0f32, 0.0, 0.0];
        let obj_max = [10.0f32, 10.0, 10.0];
        let query = [5.0f32, 5.0, 5.0, 1.0, 2.0, 3.0];
        assert!(f(&obj_min, &obj_max, &query));
    }

    // Equality on an upper-bound axis passes — the test is inclusive (`<=`), so a
    // query corner touching the object's max face still overlaps.
    #[test]
    fn upper_bound_equality_passes() {
        let obj_min = [0.0f32, 0.0, 0.0];
        let obj_max = [10.0f32, 10.0, 10.0];
        let query = [10.0f32, 5.0, 5.0, 1.0, 2.0, 3.0];
        assert!(f(&obj_min, &obj_max, &query));
    }

    // Equality on a lower-bound axis passes (the lower test is inclusive).
    #[test]
    fn lower_bound_equality_passes() {
        let obj_min = [1.0f32, 2.0, 3.0];
        let obj_max = [10.0f32, 10.0, 10.0];
        let query = [5.0f32, 5.0, 5.0, 1.0, 2.0, 3.0];
        assert!(f(&obj_min, &obj_max, &query));
    }

    // A query above an upper bound fails.
    #[test]
    fn above_upper_bound_fails() {
        let obj_min = [0.0f32, 0.0, 0.0];
        let obj_max = [10.0f32, 10.0, 10.0];
        let query = [5.0f32, 11.0, 5.0, 1.0, 2.0, 3.0];
        assert!(!f(&obj_min, &obj_max, &query));
    }

    // A query below a lower bound fails (obj_min[i] > query[3+i]).
    #[test]
    fn below_lower_bound_fails() {
        let obj_min = [5.0f32, 5.0, 5.0];
        let obj_max = [10.0f32, 10.0, 10.0];
        let query = [6.0f32, 6.0, 6.0, 4.0, 6.0, 6.0];
        assert!(!f(&obj_min, &obj_max, &query));
    }

    // Each axis independently gates the result: flip exactly one of the six
    // tests out of band and the predicate must drop to false.
    #[test]
    fn each_axis_independently_gates() {
        let obj_min = [0.0f32, 0.0, 0.0];
        let obj_max = [10.0f32, 10.0, 10.0];
        let pass = [5.0f32, 5.0, 5.0, 1.0, 2.0, 3.0];
        assert!(f(&obj_min, &obj_max, &pass));
        for i in 0..3 {
            let mut q = pass;
            // Strictly above the max face fails; equality is inclusive (tested in
            // `upper_bound_equality_passes`), so push past it to gate.
            q[i] = obj_max[i] + 1.0;
            assert!(!f(&obj_min, &obj_max, &q), "upper lane {i}");
        }
        for i in 0..3 {
            let mut q = pass;
            q[3 + i] = obj_min[i] - 1.0;
            assert!(!f(&obj_min, &obj_max, &q), "lower lane {i}");
        }
    }

    // NaN in any participating lane makes that comparison false => overall miss.
    #[test]
    fn nan_query_misses() {
        let obj_min = [0.0f32, 0.0, 0.0];
        let obj_max = [10.0f32, 10.0, 10.0];
        let base = [5.0f32, 5.0, 5.0, 1.0, 2.0, 3.0];
        for i in 0..6 {
            let mut q = base;
            q[i] = f32::NAN;
            assert!(!f(&obj_min, &obj_max, &q), "nan lane {i}");
        }
    }
}

/// Seeds the 13 `f32` (52-byte `0x34`) fields of a `QuadElement` from a single `fill`.
///
/// The header is `{0, 0, 1.0, center}`, the nine bound/extent floats
/// (indices 4..=12) are all `fill`, and the center (index 3) is derived from the
/// bounds as `(extent[1] + extent[0]) * K - extent[2]`.
///
/// `K` is the host `.rdata` constant `DAT_007ffd74`, which is `0.0` in the stock
/// image; keeping it as a named factor mirrors the original's read-back-then-
/// derive sequence (the center is computed from the just-written bound floats, so
/// `(fill + fill) * 0.0 - fill == -fill`).
pub fn quad_element__seed_from_fill__7bf7b0(fill: f32) -> [f32; 13] {
    const K: f32 = 0.0;
    let mut e = [0.0f32; 13];
    e[0] = 0.0;
    e[1] = 0.0;
    e[2] = 1.0;
    // The nine bound/extent floats are splatted from `fill`.
    e[4] = fill;
    e[5] = fill;
    e[6] = fill;
    e[7] = fill;
    e[8] = fill;
    e[9] = fill;
    e[10] = fill;
    e[11] = fill;
    e[12] = fill;
    // Center derived from the bound floats just written.
    e[3] = (e[5] + e[4]) * K - e[6];
    e
}

#[cfg(test)]
mod tests_quad_element__seed_from_fill__7bf7b0 {
    use super::quad_element__seed_from_fill__7bf7b0 as f;

    #[test]
    fn known_value() {
        let e = f(2.5);
        assert_eq!(e[0], 0.0);
        assert_eq!(e[1], 0.0);
        assert_eq!(e[2], 1.0);
        // K == 0.0 ⇒ center == -fill.
        assert_eq!(e[3], -2.5);
        for (i, &v) in e.iter().enumerate().skip(4) {
            assert_eq!(v, 2.5, "field {i}");
        }
    }

    #[test]
    fn zero_fill_is_all_header_only() {
        let e = f(0.0);
        assert_eq!(e[2], 1.0);
        // Every other field is 0.0 (including center, since -0.0 == 0.0).
        for (i, &v) in e.iter().enumerate() {
            if i == 2 {
                continue;
            }
            assert_eq!(v, 0.0, "field {i}");
        }
    }

    #[test]
    fn center_is_negated_fill() {
        // Metamorphic: center is exactly the negation of every bound float.
        for fill in [-7.0f32, -1.0, 0.5, 3.0, 42.0, 1000.0] {
            let e = f(fill);
            assert_eq!(e[3], -fill, "fill {fill}");
            // And it equals -e[6] regardless of the K factor.
            assert_eq!(e[3], -e[6], "fill {fill}");
        }
    }

    #[test]
    fn header_is_constant_across_fill() {
        // The {0, 0, 1.0} header never depends on `fill`.
        let a = f(1.0);
        let b = f(-99.0);
        assert_eq!(a[0], b[0]);
        assert_eq!(a[1], b[1]);
        assert_eq!(a[2], b[2]);
    }

    #[test]
    fn bounds_all_equal_fill() {
        // All nine bound/extent floats carry the identical fill value.
        let e = f(13.5);
        let bounds = &e[4..13];
        assert!(bounds.iter().all(|&x| x == 13.5), "{bounds:?}");
    }
}

/// Lua 5.0 `luaH_arrayindex`: resolve a `TValue` number key to its array-part index.
///
/// Returns the integer key when `tag` is `LUA_TNUMBER` (3), the number is an exact
/// integer round-trip of its truncation, and the resulting index lies in
/// `[1, 0x100_0000]` (the `MAXASIZE` guard `(key-1) & 0xff00_0000 == 0`). Returns
/// `-1` otherwise. `f64` is required: the value is a Lua number and the integrality
/// round-trip must track full double precision.
pub fn lua_h_arrayindex__6fa4a0(tag: i32, number: f64) -> i32 {
    if tag != 3 {
        return -1;
    }
    // `__ftol` truncates toward zero into an i64; the caller then takes the low
    // 32 bits via `(int)`. Replicate both steps.
    let truncated = ftol__40a2b0(number) as i32;
    // Exact integer round-trip `(double)truncated == number`, positivity, and the
    // MAXASIZE window `(truncated - 1) & 0xff00_0000 == 0`.
    if f64::from(truncated) == number
        && truncated >= 1
        && (truncated - 1) & 0xff00_0000u32 as i32 == 0
    {
        return truncated;
    }
    -1
}

#[cfg(test)]
mod tests_lua_h_arrayindex__6fa4a0 {
    use super::lua_h_arrayindex__6fa4a0;

    const LUA_TNUMBER: i32 = 3;

    #[test]
    fn non_number_tag_rejected() {
        // Any tag other than 3 returns -1 regardless of the number.
        assert_eq!(lua_h_arrayindex__6fa4a0(0, 1.0), -1);
        assert_eq!(lua_h_arrayindex__6fa4a0(1, 5.0), -1);
        assert_eq!(lua_h_arrayindex__6fa4a0(4, 5.0), -1);
    }

    #[test]
    fn valid_small_integers() {
        // In-range positive integers map to themselves.
        assert_eq!(lua_h_arrayindex__6fa4a0(LUA_TNUMBER, 1.0), 1);
        assert_eq!(lua_h_arrayindex__6fa4a0(LUA_TNUMBER, 2.0), 2);
        assert_eq!(lua_h_arrayindex__6fa4a0(LUA_TNUMBER, 100.0), 100);
    }

    #[test]
    fn zero_and_negative_rejected() {
        // Must be >= 1.
        assert_eq!(lua_h_arrayindex__6fa4a0(LUA_TNUMBER, 0.0), -1);
        assert_eq!(lua_h_arrayindex__6fa4a0(LUA_TNUMBER, -1.0), -1);
        assert_eq!(lua_h_arrayindex__6fa4a0(LUA_TNUMBER, -100.0), -1);
    }

    #[test]
    fn non_integral_rejected() {
        // Fractional numbers fail the exact round-trip.
        assert_eq!(lua_h_arrayindex__6fa4a0(LUA_TNUMBER, 1.5), -1);
        assert_eq!(lua_h_arrayindex__6fa4a0(LUA_TNUMBER, 2.0001), -1);
        assert_eq!(lua_h_arrayindex__6fa4a0(LUA_TNUMBER, 0.5), -1);
    }

    #[test]
    fn maxasize_boundary() {
        // (key-1) must satisfy & 0xff000000 == 0, i.e. key in [1, 0x1000000].
        // 0x1000000 == 16777216 is the last valid index.
        assert_eq!(
            lua_h_arrayindex__6fa4a0(LUA_TNUMBER, 16_777_216.0),
            16_777_216
        );
        // 0x1000001: (key-1) = 0x1000000, & 0xff000000 != 0 -> reject.
        assert_eq!(lua_h_arrayindex__6fa4a0(LUA_TNUMBER, 16_777_217.0), -1);
    }

    #[test]
    fn round_trip_property() {
        // For every in-range positive integer, the function is the identity.
        let mut k: i32 = 1;
        while k <= 16_777_216 {
            assert_eq!(lua_h_arrayindex__6fa4a0(LUA_TNUMBER, f64::from(k)), k);
            k = k.saturating_mul(2);
        }
    }

    #[test]
    fn large_out_of_range_rejected() {
        // Values above the MAXASIZE window are rejected.
        assert_eq!(lua_h_arrayindex__6fa4a0(LUA_TNUMBER, 100_000_000.0), -1);
    }
}

/// Whether appending a 4-byte value needs the backing window grown first.
///
/// The byte-stream writer grows its window when the write `cursor` is below the
/// current window origin, or when the next write (`cursor + 4`) would run past
/// the mapped end (`window + capacity`). All values are unsigned byte offsets,
/// matching the original's `JC`/`JBE` unsigned comparisons.
pub fn c_data_store__put_int32_dup__4183d0(cursor: u32, window: u32, capacity: u32) -> bool {
    cursor < window || window.wrapping_add(capacity) < cursor.wrapping_add(4)
}

#[cfg(test)]
mod tests_c_data_store__put_int32_dup__4183d0 {
    use super::c_data_store__put_int32_dup__4183d0 as needs_grow;

    #[test]
    fn fits_inside_window_no_grow() {
        assert!(!needs_grow(10, 0, 100));
    }

    #[test]
    fn cursor_below_window_grows() {
        assert!(needs_grow(5, 8, 100));
    }

    #[test]
    fn next_write_past_end_grows() {
        assert!(needs_grow(10, 0, 12));
    }

    #[test]
    fn exact_fit_no_grow() {
        assert!(!needs_grow(8, 0, 12));
    }

    #[test]
    fn one_past_exact_grows() {
        assert!(needs_grow(9, 0, 12));
    }

    #[test]
    fn windowed_region() {
        assert!(!needs_grow(120, 100, 50));
        assert!(needs_grow(99, 100, 50));
        assert!(needs_grow(147, 100, 50));
    }

    #[test]
    fn monotone_in_cursor_within_window() {
        let (window, capacity) = (0u32, 40u32);
        let mut seen_grow = false;
        for cursor in window..=window + capacity {
            let g = needs_grow(cursor, window, capacity);
            if g {
                seen_grow = true;
            } else {
                assert!(!seen_grow, "non-monotone at cursor={cursor}");
            }
        }
        assert!(seen_grow, "expected a grow point within the window");
    }
}

/// fmod-style angle wrap into a canonical period.
///
/// Computes `angle - trunc(angle * recip) * period`, then subtracts one more
/// `period` when `angle < threshold` (keeping the result in a half-open range).
/// `recip` is the host's `1/period` reciprocal; `period` and `threshold` are the
/// fixed host constants, injected so the kernel stays pure. The truncation
/// matches the original's `__ftol` (round toward zero into a 32-bit int).
pub fn c_math_wrap_angle__7abbe0(angle: f32, recip: f32, period: f32, threshold: f32) -> f32 {
    let trunc = ftol__40a2b0(f64::from(angle * recip)) as i32;
    let mut reduced = trunc as f32 * period;
    if angle < threshold {
        reduced -= period;
    }
    angle - reduced
}

#[cfg(test)]
mod tests_c_math_wrap_angle__7abbe0 {
    use super::c_math_wrap_angle__7abbe0 as wrap;

    // The stock client constants: period = 2*pi, recip = 1/(2*pi), threshold = 0.
    const TAU: f32 = core::f32::consts::TAU;
    const RECIP: f32 = 1.0 / TAU;
    const THRESH: f32 = 0.0;

    fn approx(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn already_in_range_is_unchanged() {
        for &a in &[0.0f32, 0.5, 1.0, 3.0, 6.0] {
            assert!(approx(wrap(a, RECIP, TAU, THRESH), a, 1e-4), "a={a}");
        }
    }

    #[test]
    fn reduces_above_period() {
        let a = TAU + 0.5;
        assert!(approx(wrap(a, RECIP, TAU, THRESH), 0.5, 1e-3));
    }

    #[test]
    fn negative_wraps_into_positive_range() {
        let a = -0.5f32;
        let r = wrap(a, RECIP, TAU, THRESH);
        assert!(approx(r, TAU - 0.5, 1e-3), "got {r}");
        assert!(r >= 0.0, "result should be non-negative, got {r}");
    }

    #[test]
    fn result_in_canonical_range_for_threshold_zero() {
        let mut x = -50.0f32;
        while x <= 50.0 {
            let r = wrap(x, RECIP, TAU, THRESH);
            assert!(
                (-1e-3..TAU + 1e-3).contains(&r),
                "x={x} -> r={r} out of range"
            );
            x += 0.37;
        }
    }

    #[test]
    fn periodicity_metamorphic() {
        for &a in &[0.3f32, 2.1, 5.5, -1.2] {
            let base = wrap(a, RECIP, TAU, THRESH);
            for k in 1..=4 {
                let shifted = wrap(a + k as f32 * TAU, RECIP, TAU, THRESH);
                assert!(
                    approx(base, shifted, 2e-2),
                    "a={a} k={k}: {base} vs {shifted}"
                );
            }
        }
    }

    #[test]
    fn threshold_controls_extra_period() {
        let a = 1.0f32;
        let with = wrap(a, RECIP, TAU, 2.0);
        assert!(approx(with, a + TAU, 1e-3), "got {with}");
        let without = wrap(a, RECIP, TAU, 0.5);
        assert!(approx(without, a, 1e-4), "got {without}");
    }
}

/// `NormalizeAngleToPi` (0x6084f0).
///
/// Wrap a scalar angle into the canonical half-open interval `(-PI, PI]` via an
/// fmod-by-2PI, reproducing the stock x87 range-reduction.
/// `__stdcall(float) -> float` (result in ST0). There is no `this` receiver
/// despite the x87-stack fmod intrinsic; ECX is never touched and `RET 4` means
/// a single float arg.
///
/// Constants are the exact image values: `HI = +PI` / `LO = -PI` as f32
/// (`0x40490fdb` / `0xc0490fdb`), and the fmod divisor is `2*PI_f32` stored as an
/// f64 (`0x401921fb60000000` = 6.283_185_482_025_146 — twice the *f32* PI, not
/// true 2*PI). The stock loads the divisor as a double and mixes the f32 angle
/// with an f64 fmod, so we widen to f64, fmod, then narrow; the small f32-derived
/// intermediates are exact in f64, making the reduction bit-faithful.
///
/// NaN polarity: both compares are ordered (the x87 `FCOMP` chain falls to the
/// identity tail on unordered), so a NaN angle returns unchanged — matched by
/// Rust `>`/`<` both being false on NaN. Do NOT reorder into `<=`/`>=`.
pub fn normalize_angle_to_pi__6084f0(angle: f32) -> f32 {
    const HI: f32 = f32::from_bits(0x4049_0fdb); //  PI as f32
    const LO: f32 = f32::from_bits(0xc049_0fdb); // -PI as f32
    const TWO_PI: f64 = f64::from_bits(0x4019_21fb_6000_0000); // 2 * PI_f32
    if angle > HI {
        (((f64::from(angle) + f64::from(HI)) % TWO_PI) - f64::from(HI)) as f32
    } else if angle < LO {
        (((f64::from(angle) - f64::from(HI)) % TWO_PI) + f64::from(HI)) as f32
    } else {
        // In `(-PI, PI]`, or NaN — returned unchanged.
        angle
    }
}

#[cfg(test)]
mod tests_normalize_angle_to_pi__6084f0 {
    use super::normalize_angle_to_pi__6084f0 as norm;

    const HI: f32 = f32::from_bits(0x4049_0fdb); // PI as f32
    const TWO_PI: f64 = f64::from_bits(0x4019_21fb_6000_0000);

    #[test]
    fn in_range_is_unchanged() {
        // Anything already in (-PI, PI] (strictly inside) is returned bit-for-bit.
        for &a in &[0.0f32, -0.0, 0.5, -0.5, 1.5, -1.5, 3.0, -3.0] {
            assert_eq!(norm(a), a, "a={a}");
        }
    }

    #[test]
    fn boundary_pi_is_unchanged() {
        // angle == +PI: `angle > HI` is false -> identity. angle == -PI: `< LO`
        // false -> identity. Both boundaries pass through.
        assert_eq!(norm(HI), HI);
        assert_eq!(norm(-HI), -HI);
    }

    #[test]
    fn wraps_one_period_each_side() {
        // Just above +PI subtracts 2*PI; just below -PI adds 2*PI.
        let above = norm(HI + 0.5);
        assert!(
            (f64::from(above) - (0.5 - f64::from(HI))).abs() < 1e-3,
            "above={above}"
        );
        let below = norm(-HI - 0.5);
        assert!(
            (f64::from(below) - (f64::from(HI) - 0.5)).abs() < 1e-3,
            "below={below}"
        );
    }

    #[test]
    fn nan_returned_unchanged() {
        assert!(norm(f32::NAN).is_nan());
    }

    #[test]
    fn congruent_mod_two_pi_and_in_range() {
        // Independent oracle: the result must be congruent to the input modulo
        // 2*PI_f32 and lie within [-PI, PI] (either boundary allowed). This checks
        // the wrap without assuming which boundary representative is chosen.
        let mut a = -40.0f64;
        while a <= 40.0 {
            let r = norm(a as f32);
            assert!(
                f64::from(r) >= -f64::from(HI) - 1e-3 && f64::from(r) <= f64::from(HI) + 1e-3,
                "a={a} r={r} out of range"
            );
            let k = (a - f64::from(r)) / TWO_PI;
            assert!(
                (k - k.round()).abs() < 1e-4,
                "a={a} r={r} not congruent mod 2PI (k={k})"
            );
            a += 0.37;
        }
    }
}

/// In-place heapsort of a `u32` array using a caller-supplied comparator.
///
/// `cmp(a, b)` returns `> 0` when element `a` should be ordered *after* element
/// `b` (i.e. `a` is the larger in heap order). Builds a max-heap by sifting down
/// from the last internal node, then repeatedly moves the root to the end and
/// re-sifts the shrunken heap, yielding ascending order. The index arithmetic and
/// comparison polarity mirror the original; `cmp` is invoked exactly where the
/// reference calls its function-pointer predicate.
pub fn heap_sort_u_int32__71f860<F: FnMut(u32, u32) -> i32>(array: &mut [u32], mut cmp: F) {
    let count = array.len();
    if count <= 1 {
        return;
    }
    let sift = |array: &mut [u32], start: usize, len: usize, cmp: &mut F| {
        let held = array[start];
        let mut parent = start;
        let mut child = start * 2 + 1;
        while child < len {
            let mut larger = child;
            if child + 1 < len && cmp(array[child + 1], array[child]) > 0 {
                larger = child + 1;
            }
            if cmp(held, array[larger]) >= 0 {
                break;
            }
            array[parent] = array[larger];
            parent = larger;
            child = larger * 2 + 1;
        }
        array[parent] = held;
    };
    let mut node = count / 2;
    while node != 0 {
        node -= 1;
        sift(array, node, count, &mut cmp);
    }
    let mut end = count - 1;
    while end != 0 {
        array.swap(0, end);
        sift(array, 0, end, &mut cmp);
        end -= 1;
    }
}

#[cfg(test)]
mod tests_heap_sort_u_int32__71f860 {
    use super::heap_sort_u_int32__71f860;

    /// Ascending comparator: `a` is ordered after `b` exactly when `a > b`.
    fn ascending(a: u32, b: u32) -> i32 {
        a.cmp(&b) as i32
    }

    /// Reference oracle: a clone sorted ascending by the standard library.
    fn oracle(data: &[u32]) -> Vec<u32> {
        let mut v = data.to_vec();
        v.sort_unstable();
        v
    }

    #[test]
    fn known_value_small() {
        let mut data = [5u32, 1, 4, 2, 8];
        heap_sort_u_int32__71f860(&mut data, ascending);
        assert_eq!(data, [1u32, 2, 4, 5, 8]);
    }

    #[test]
    fn known_value_reversed_input() {
        let mut data = [9u32, 7, 5, 3, 1];
        heap_sort_u_int32__71f860(&mut data, ascending);
        assert_eq!(data, [1u32, 3, 5, 7, 9]);
    }

    #[test]
    fn empty_and_single_are_noops() {
        let mut empty: [u32; 0] = [];
        heap_sort_u_int32__71f860(&mut empty, ascending);
        assert_eq!(empty, []);

        let mut single = [42u32];
        heap_sort_u_int32__71f860(&mut single, ascending);
        assert_eq!(single, [42u32]);
    }

    #[test]
    fn duplicates_kept_as_multiset() {
        let mut data = [3u32, 3, 1, 2, 1, 3, 2];
        heap_sort_u_int32__71f860(&mut data, ascending);
        assert_eq!(data, [1u32, 1, 2, 2, 3, 3, 3]);
    }

    /// Property: output equals the standard-library sort across a spread of inputs.
    #[test]
    fn matches_oracle_over_many_inputs() {
        let cases: [&[u32]; 6] = [
            &[],
            &[7],
            &[2, 1],
            &[1, 2, 3, 4, 5, 6],
            &[6, 5, 4, 3, 2, 1],
            &[100, 0, 4_000_000_000, 1, 0xFFFF_FFFF, 17, 17, 256],
        ];
        for case in &cases {
            let mut data = case.to_vec();
            heap_sort_u_int32__71f860(&mut data, ascending);
            assert_eq!(data, oracle(case));
        }
    }

    /// Property: a deterministic pseudo-random sequence sorts nondecreasing.
    ///
    /// Matches the oracle exactly.
    #[test]
    fn sorted_is_nondecreasing() {
        let mut state: u32 = 0x1234_5678;
        let mut data = Vec::with_capacity(257);
        for _ in 0..257u32 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            data.push(state);
        }
        let expected = oracle(&data);
        heap_sort_u_int32__71f860(&mut data, ascending);
        assert_eq!(data, expected);
        assert!(data.windows(2).all(|w| w[0] <= w[1]));
    }

    /// Metamorphic: reversing the comparator polarity yields descending order.
    #[test]
    fn reversed_comparator_yields_descending() {
        let mut data = [5u32, 1, 4, 2, 8, 8, 0];
        heap_sort_u_int32__71f860(&mut data, |a, b| b.cmp(&a) as i32);
        let mut expected = oracle(&[5, 1, 4, 2, 8, 8, 0]);
        expected.reverse();
        assert_eq!(data, expected.as_slice());
    }
}

/// Cached `(cos, sin)` of a yaw angle (radians), in the original's store order.
///
/// The orientation object caches `cos(yaw)` and `sin(yaw)` immediately after a
/// yaw change; the source uses the x87 `fsincos` which yields both from one
/// angle. Returned as `[cos, sin]` matching the `this+0x20`/`this+0x24` slots.
pub fn orientation__set_yaw__7acc40(yaw: f32) -> [f32; 2] {
    let (s, c) = crate::math::trig::sin_cos(yaw);
    [c, s]
}

#[cfg(test)]
mod tests_orientation__set_yaw__7acc40 {
    use super::orientation__set_yaw__7acc40 as cs;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() <= 1e-5
    }

    #[test]
    fn zero_is_one_zero() {
        let r = cs(0.0);
        assert!(approx(r[0], 1.0));
        assert!(approx(r[1], 0.0));
    }

    #[test]
    fn quarter_turn() {
        let r = cs(core::f32::consts::FRAC_PI_2);
        assert!(approx(r[0], 0.0), "cos={}", r[0]);
        assert!(approx(r[1], 1.0), "sin={}", r[1]);
    }

    #[test]
    fn pythagorean_identity() {
        let mut a = -10.0f32;
        while a <= 10.0 {
            let r = cs(a);
            assert!(approx(r[0] * r[0] + r[1] * r[1], 1.0), "a={a}");
            a += 0.31;
        }
    }

    #[test]
    fn order_is_cos_then_sin() {
        let r = cs(1.0);
        assert!(approx(r[0], 1.0f32.cos()));
        assert!(approx(r[1], 1.0f32.sin()));
    }
}

/// Saturates `value` into `[0, 1]`.
///
/// Returns `0.0` when `value < zero_lo`, `1.0` when `value >= one_hi`, else
/// `value` unchanged.
///
/// `zero_lo`/`one_hi` are the host saturation thresholds (`0.0` and `1.0` in the
/// stock image). The branch order and comparison polarity mirror the original x87
/// `FCOMP` saturate idiom: a `NaN` input takes the low-saturate branch and yields
/// `0.0`, matching the unordered-compare path.
pub fn set_clamped_value01__453480(value: f32, zero_lo: f32, one_hi: f32) -> f32 {
    if zero_lo <= value {
        if one_hi <= value {
            return 1.0;
        }
        value
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests_set_clamped_value01__453480 {
    use super::set_clamped_value01__453480 as clamp;

    // Stock saturation thresholds from the image (`DAT_007ffd74` / `_DAT_007ff9d8`).
    const ZERO_LO: f32 = 0.0;
    const ONE_HI: f32 = 1.0;

    /// Below the low threshold saturates to `0.0`.
    #[test]
    fn below_low_saturates_to_zero() {
        for v in [-100.0_f32, -1.0, -0.001, f32::NEG_INFINITY] {
            assert_eq!(clamp(v, ZERO_LO, ONE_HI).to_bits(), 0.0_f32.to_bits());
        }
    }

    /// At or above the high threshold saturates to `1.0`.
    #[test]
    fn at_or_above_high_saturates_to_one() {
        for v in [1.0_f32, 1.5, 100.0, f32::INFINITY] {
            assert_eq!(clamp(v, ZERO_LO, ONE_HI).to_bits(), 1.0_f32.to_bits());
        }
    }

    /// Values strictly inside the band pass through unchanged.
    #[test]
    fn inside_band_passthrough() {
        for v in [0.0_f32, 0.25, 0.5, 0.75, 0.999] {
            assert_eq!(clamp(v, ZERO_LO, ONE_HI).to_bits(), v.to_bits());
        }
    }

    /// Output is always within `[0.0, 1.0]` for the stock thresholds.
    #[test]
    fn output_within_unit_interval() {
        for v in [-5.0_f32, -0.1, 0.0, 0.3, 0.5, 0.9, 1.0, 2.0, 50.0] {
            let out = clamp(v, ZERO_LO, ONE_HI);
            assert!((0.0..=1.0).contains(&out), "out={out} for v={v}");
        }
    }

    /// Idempotence: clamping an already-clamped result is a fixed point.
    #[test]
    fn idempotent() {
        for v in [-3.0_f32, -0.2, 0.0, 0.4, 0.8, 1.0, 7.0] {
            let once = clamp(v, ZERO_LO, ONE_HI);
            let twice = clamp(once, ZERO_LO, ONE_HI);
            assert_eq!(twice.to_bits(), once.to_bits());
        }
    }

    /// `NaN` takes the low-saturate branch (`zero_lo <= NaN` is false) and yields `0.0`.
    #[test]
    fn nan_saturates_to_zero() {
        assert_eq!(
            clamp(f32::NAN, ZERO_LO, ONE_HI).to_bits(),
            0.0_f32.to_bits()
        );
    }

    /// Known values computed directly from the branch logic.
    #[test]
    fn known_values() {
        // value == zero_lo: not below, not above -> passthrough (0.0).
        assert_eq!(clamp(0.0, ZERO_LO, ONE_HI).to_bits(), 0.0_f32.to_bits());
        // value == one_hi: one_hi <= value holds -> 1.0.
        assert_eq!(clamp(1.0, ZERO_LO, ONE_HI).to_bits(), 1.0_f32.to_bits());
        // midpoint passthrough.
        assert_eq!(clamp(0.5, ZERO_LO, ONE_HI).to_bits(), 0.5_f32.to_bits());
    }

    /// With custom thresholds the band boundaries move accordingly.
    #[test]
    fn custom_thresholds() {
        let lo = 2.0_f32;
        let hi = 5.0_f32;
        assert_eq!(clamp(1.0, lo, hi).to_bits(), 0.0_f32.to_bits());
        assert_eq!(clamp(3.0, lo, hi).to_bits(), 3.0_f32.to_bits());
        assert_eq!(clamp(6.0, lo, hi).to_bits(), 1.0_f32.to_bits());
        // boundary: value == lo -> passthrough.
        assert_eq!(clamp(2.0, lo, hi).to_bits(), 2.0_f32.to_bits());
        // boundary: value == hi -> 1.0.
        assert_eq!(clamp(5.0, lo, hi).to_bits(), 1.0_f32.to_bits());
    }
}

/// CRT float-to-i64 truncation (`__ftol`).
///
/// Kernel and tests live in `wow_shared::ftol` (shared with the d3d9 side);
/// re-exported here to keep the `crate::math::misc::ftol__40a2b0` kernel/hook
/// paths stable.
pub use wow_shared::ftol::ftol__40a2b0;

/// One rotate-xor pass over the 64-entry hash table.
///
/// Pre-mix the input with two of its rotations, then XOR four rotated table
/// entries selected by 6-bit slices of the mix.
fn hash_lo(perm: &[u32; 64], v: u32) -> u32 {
    let m = v ^ v.rotate_left(21) ^ v.rotate_left(11);
    perm[((m >> 18) & 0x3f) as usize].rotate_left(3)
        ^ perm[((m >> 12) & 0x3f) as usize].rotate_left(2)
        ^ perm[((m >> 6) & 0x3f) as usize].rotate_left(1)
        ^ perm[(m & 0x3f) as usize]
}

/// Chained form of `hash_lo` with the final 4-bit rotate.
///
/// Applied between axis levels of the corner-hash cascade.
fn hash_hi(perm: &[u32; 64], v: u32) -> u32 {
    hash_lo(perm, v).rotate_left(4)
}

/// Hashes the eight unit-cube corner lattice points of cell `(ix, iy, iz)`.
///
/// A z -> y -> x cascade where each level XORs the lattice coordinate into the
/// previous level's hash. Order: `[x0, x1]` innermost, then y, then z.
fn corner_hashes(perm: &[u32; 64], ix: u32, iy: u32, iz: u32) -> [u32; 8] {
    let z0 = hash_hi(perm, iz);
    let z1 = hash_hi(perm, iz.wrapping_add(1));
    let iy1 = iy.wrapping_add(1);
    let y00 = hash_hi(perm, z0 ^ iy);
    let y10 = hash_hi(perm, z0 ^ iy1);
    let y01 = hash_hi(perm, z1 ^ iy);
    let y11 = hash_hi(perm, z1 ^ iy1);
    let ix1 = ix.wrapping_add(1);
    [
        hash_lo(perm, y00 ^ ix),
        hash_lo(perm, y00 ^ ix1),
        hash_lo(perm, y10 ^ ix),
        hash_lo(perm, y10 ^ ix1),
        hash_lo(perm, y01 ^ ix),
        hash_lo(perm, y01 ^ ix1),
        hash_lo(perm, y11 ^ ix),
        hash_lo(perm, y11 ^ ix1),
    ]
}

/// Truncate-toward-zero cell index (`__ftol`, low 32 bits).
///
/// With the negative-side floor fix-up (strict `v < cutoff` decrements, so exact
/// negative integers land in the cell below with fractional part 1.0 —
/// matching the source).
fn floor_cell(v: f64, cutoff: f64) -> u32 {
    let mut cell = ftol__40a2b0(v) as u32;
    if v < cutoff {
        cell = cell.wrapping_sub(1);
    }
    cell
}

/// 4-bit lane of `h` at bit `s`, as a signed table index.
fn nib(h: u32, s: u32) -> i32 {
    ((h >> s) & 0xf) as i32
}

/// Signed ramp lookup.
///
/// `idx` in `-15..=15` (a nibble or a nibble difference), anchored at the table
/// midpoint.
fn ramp(t: &[f32; 31], idx: i32) -> f32 {
    t[(idx + 15) as usize]
}

/// Gradient-ramp entry selected by the 4-bit lane of `h` at bit `s`.
fn gn(grad: &[f32; 16], h: u32, s: u32) -> f32 {
    grad[((h >> s) & 0xf) as usize]
}

/// One Hermite lane.
///
/// Pre-tabulated end-minus-start difference `dif`, start value `base`, endpoint
/// derivatives `d0`/`d1`, basis weights `w`/`v`/`u`.
fn lane(dif: f32, base: f32, d0: f32, d1: f32, w: f32, v: f32, u: f32) -> f32 {
    dif * w + d0 * v + d1 * u + base
}

/// Hermite blend of four already-blended lanes picked from a 16-lane pass output.
///
/// Start `base`, end `end`, endpoint derivatives `d0`/`d1`.
fn blend(
    o: &[f32; 16],
    base: usize,
    end: usize,
    d0: usize,
    d1: usize,
    w: f32,
    v: f32,
    u: f32,
) -> f32 {
    (o[end] - o[base]) * w + o[d0] * v + o[d1] * u + o[base]
}

/// Tiled cubic-Hermite gradient-noise sample at `(x, y, z)` with analytic gradient.
///
/// Hashes the eight unit-cube corner lattice points through the 64-entry
/// rotate-xor `perm` table; each corner hash packs eight 4-bit
/// channels (value + derivative lanes) indexing the `grad`/`val_tab`/`dif_tab`
/// ramps. Two 16-lane Hermite passes (x, then z) plus a y-blend tail produce
/// the scalar and its three partial derivatives; the scalar is remapped
/// through `m = n * scale + bias` and smoothstep `m^2 * (3 - 2m)`.
pub fn perlin_noise3_d__452960(
    x: f64,
    y: f64,
    z: f64,
    perm: &[u32; 64],
    grad: &[f32; 16],
    val_tab: &[f32; 31],
    dif_tab: &[f32; 31],
    floor_cutoff: f64,
    two: f32,
    three: f32,
    six: f32,
    one: f32,
    scale: f32,
    bias: f32,
) -> (f64, [f32; 3]) {
    let ix = floor_cell(x, floor_cutoff);
    let iy = floor_cell(y, floor_cutoff);
    let iz = floor_cell(z, floor_cutoff);
    let fx = (x - f64::from(ix as i32)) as f32;
    let fy = (y - f64::from(iy as i32)) as f32;
    let fz = (z - f64::from(iz as i32)) as f32;

    let ch = corner_hashes(perm, ix, iy, iz);

    // Hermite basis weights per axis: `u = t^3 - t^2`, `v = (t - 2) t^2 + t`,
    // `w = (3 - 2t) t^2` (endpoint-derivative pair + smoothstep blend).
    let fx2 = fx * fx;
    let fy2 = fy * fy;
    let fz2 = fz * fz;
    let u_x = fx2 * fx - fx2;
    let u_y = fy2 * fy - fy2;
    let u_z = fz2 * fz - fz2;
    let v_x = (fx - two) * fx2 + fx;
    let v_y = (fy - two) * fy2 + fy;
    let v_z = (fz - two) * fz2 + fz;
    let w_x = (three - (fx + fx)) * fx2;
    let w_y = (three - (fy + fy)) * fy2;
    let w_z = (three - (fz + fz)) * fz2;

    // Pass 1 — x-axis Hermite over the four (y, z) corner pairs. Channel
    // layout per pair (h0 = x0 corner, h1 = x1 corner): value channel from
    // the value ramp (nibble 28) with derivative nibble 24, then three
    // gradient channels (base nibbles 20/12/4, derivative nibbles 16/8/0).
    let mut o1 = [0.0_f32; 16];
    for (dst, pair) in o1.chunks_exact_mut(4).zip(ch.chunks_exact(2)) {
        let (h0, h1) = (pair[0], pair[1]);
        dst[0] = lane(
            ramp(val_tab, nib(h1, 28) - nib(h0, 28)),
            ramp(val_tab, nib(h0, 28)),
            gn(grad, h0, 24),
            gn(grad, h1, 24),
            w_x,
            v_x,
            u_x,
        );
        dst[1] = lane(
            ramp(dif_tab, nib(h1, 20) - nib(h0, 20)),
            gn(grad, h0, 20),
            gn(grad, h0, 16),
            gn(grad, h1, 16),
            w_x,
            v_x,
            u_x,
        );
        dst[2] = lane(
            ramp(dif_tab, nib(h1, 12) - nib(h0, 12)),
            gn(grad, h0, 12),
            gn(grad, h0, 8),
            gn(grad, h1, 8),
            w_x,
            v_x,
            u_x,
        );
        dst[3] = lane(
            ramp(dif_tab, nib(h1, 4) - nib(h0, 4)),
            gn(grad, h0, 4),
            gn(grad, h0, 0),
            gn(grad, h1, 0),
            w_x,
            v_x,
            u_x,
        );
    }

    // Pass 2 — z-axis Hermite pairing corner k (z0) with corner k+4 (z1).
    // Value channel (nibble 28) takes its z-derivative from nibble 12; the
    // three derivative channels (base nibbles 24/20/16) from nibbles 8/4/0.
    let mut o2 = [0.0_f32; 16];
    for ((dst, &h0), &h1) in o2.chunks_exact_mut(4).zip(&ch[..4]).zip(&ch[4..]) {
        dst[0] = lane(
            ramp(val_tab, nib(h1, 28) - nib(h0, 28)),
            ramp(val_tab, nib(h0, 28)),
            gn(grad, h0, 12),
            gn(grad, h1, 12),
            w_z,
            v_z,
            u_z,
        );
        dst[1] = lane(
            ramp(dif_tab, nib(h1, 24) - nib(h0, 24)),
            gn(grad, h0, 24),
            gn(grad, h0, 8),
            gn(grad, h1, 8),
            w_z,
            v_z,
            u_z,
        );
        dst[2] = lane(
            ramp(dif_tab, nib(h1, 20) - nib(h0, 20)),
            gn(grad, h0, 20),
            gn(grad, h0, 4),
            gn(grad, h1, 4),
            w_z,
            v_z,
            u_z,
        );
        dst[3] = lane(
            ramp(dif_tab, nib(h1, 16) - nib(h0, 16)),
            gn(grad, h0, 16),
            gn(grad, h0, 0),
            gn(grad, h1, 0),
            w_z,
            v_z,
            u_z,
        );
    }

    // z-blend of the x-blended lanes: value track (f7/f8) and y-derivative
    // track (f9/f4), each pairing a z0 lane with its z1 sibling.
    let f7 = blend(&o1, 0, 8, 2, 10, w_z, v_z, u_z);
    let f4 = blend(&o1, 5, 13, 7, 15, w_z, v_z, u_z);
    let f9 = blend(&o1, 1, 9, 3, 11, w_z, v_z, u_z);
    let f8 = blend(&o1, 4, 12, 6, 14, w_z, v_z, u_z) - f7;

    // Smoothstep-derivative factors per axis (chain rule through `w`).
    let d_x = w_x - w_x * w_x;
    let d_y = w_y - w_y * w_y;
    let d_z = w_z - w_z * w_z;
    let e_x = w_x - three * d_x;
    let e_y = w_y - d_y * three;
    let e_z = w_z - d_z * three;

    // Analytic gradient, one component per axis.
    let t1 = blend(&o2, 2, 10, 3, 11, w_y, v_y, u_y);
    let t2 = blend(&o2, 4, 12, 5, 13, w_y, v_y, u_y);
    let t3 = blend(&o2, 6, 14, 7, 15, w_y, v_y, u_y);
    let mid_x = t2 - o2[0] - (o2[8] - o2[0]) * w_y - o2[1] * v_y - o2[9] * u_y;
    let k_x = e_x - (w_x + w_x) + one;
    let g_x = t1 * k_x + mid_x * six * d_x + t3 * e_x;

    let k_y = e_y - (w_y + w_y) + one;
    let g_y = k_y * f9 + f8 * six * d_y + e_y * f4;

    let s1 = blend(&o1, 2, 6, 3, 7, w_y, v_y, u_y);
    let s2 = blend(&o1, 8, 12, 9, 13, w_y, v_y, u_y);
    let s3 = blend(&o1, 10, 14, 11, 15, w_y, v_y, u_y);
    let mid_z = s2 - o1[0] - (o1[4] - o1[0]) * w_y - o1[1] * v_y - o1[5] * u_y;
    let k_z = e_z - (w_z + w_z) + one;
    let g_z = s1 * k_z + mid_z * six * d_z + s3 * e_z;

    // Final y-blend of the value track, then the smoothstep remap.
    let n = f9 * v_y + f8 * w_y + f4 * u_y + f7;
    let m = n * scale + bias;
    let value = (three - (m + m)) * (m * m);
    (f64::from(value), [g_x, g_y, g_z])
}

#[cfg(test)]
mod tests_perlin_noise3_d__452960 {
    use super::perlin_noise3_d__452960 as noise;

    const TWO: f32 = 2.0;
    const THREE: f32 = 3.0;
    const SIX: f32 = 6.0;
    const ONE: f32 = 1.0;
    const SCALE: f32 = 0.625;
    const BIAS: f32 = 0.1875;

    /// Host-shaped ramps.
    ///
    /// 16-entry gradient ramp -1..1 (step 2/15), a 31-entry value ramp anchored
    /// mid-table (step 1/15), and its difference ramp (step 2/15).
    fn ramp_tables() -> ([f32; 16], [f32; 31], [f32; 31]) {
        let mut grad = [0.0_f32; 16];
        let mut g = -1.0_f32;
        for slot in &mut grad {
            *slot = g;
            g += 2.0 / 15.0;
        }
        let mut val = [0.0_f32; 31];
        let mut v = -1.0_f32;
        for slot in &mut val {
            *slot = v;
            v += 1.0 / 15.0;
        }
        let mut dif = [0.0_f32; 31];
        let mut d = -2.0_f32;
        for slot in &mut dif {
            *slot = d;
            d += 2.0 / 15.0;
        }
        (grad, val, dif)
    }

    fn lcg_perm() -> [u32; 64] {
        let mut s = 0x1234_5678_u32;
        let mut perm = [0_u32; 64];
        for slot in &mut perm {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *slot = s;
        }
        perm
    }

    fn sample(
        perm: &[u32; 64],
        tabs: &([f32; 16], [f32; 31], [f32; 31]),
        x: f64,
        y: f64,
        z: f64,
    ) -> (f64, [f32; 3]) {
        noise(
            x, y, z, perm, &tabs.0, &tabs.1, &tabs.2, 0.0, TWO, THREE, SIX, ONE, SCALE, BIAS,
        )
    }

    /// With an all-zero hash table every corner hash is zero.
    ///
    /// So the 16 lanes of each pass collapse to a value-lane and a gradient-lane
    /// scalar; the whole pipeline reduces to closed-form algebra mirrored here.
    #[test]
    fn zero_perm_known_value() {
        let tabs = ramp_tables();
        let perm = [0_u32; 64];
        let (got, got_grad) = sample(&perm, &tabs, 0.25, 0.5, 0.75);

        let (fx, fy, fz) = (0.25_f32, 0.5_f32, 0.75_f32);
        let (fx2, fy2, fz2) = (fx * fx, fy * fy, fz * fz);
        let (u_x, u_y, u_z) = (fx2 * fx - fx2, fy2 * fy - fy2, fz2 * fz - fz2);
        let (v_x, v_y, v_z) = (
            (fx - TWO) * fx2 + fx,
            (fy - TWO) * fy2 + fy,
            (fz - TWO) * fz2 + fz,
        );
        let (w_x, w_y, w_z) = (
            (THREE - (fx + fx)) * fx2,
            (THREE - (fy + fy)) * fy2,
            (THREE - (fz + fz)) * fz2,
        );

        let a = tabs.1[15]; // value-ramp anchor (index 0)
        let e = tabs.2[15]; // difference-ramp anchor (index 0)
        let g0 = tabs.0[0]; // gradient entry 0 (all nibbles are zero)

        // Per-pass collapsed lanes.
        let o1_val = a * w_x + g0 * v_x + g0 * u_x + a;
        let o1_grd = e * w_x + g0 * v_x + g0 * u_x + g0;
        let o2_val = a * w_z + g0 * v_z + g0 * u_z + a;
        let o2_grd = e * w_z + g0 * v_z + g0 * u_z + g0;

        // z-blend tail (value track f7/f8, y-derivative track f9/f4).
        let f7 = (o1_val - o1_val) * w_z + o1_grd * v_z + o1_grd * u_z + o1_val;
        let f4 = (o1_grd - o1_grd) * w_z + o1_grd * v_z + o1_grd * u_z + o1_grd;
        let f9 = f4;
        let f8 = ((o1_val - o1_val) * w_z + o1_grd * v_z + o1_grd * u_z + o1_val) - f7;

        let d_x = w_x - w_x * w_x;
        let d_y = w_y - w_y * w_y;
        let d_z = w_z - w_z * w_z;
        let e_x = w_x - THREE * d_x;
        let e_y = w_y - d_y * THREE;
        let e_z = w_z - d_z * THREE;

        let t1 = (o2_grd - o2_grd) * w_y + o2_grd * v_y + o2_grd * u_y + o2_grd;
        let t2 = (o2_val - o2_val) * w_y + o2_grd * v_y + o2_grd * u_y + o2_val;
        let t3 = t1;
        let mid_x = t2 - o2_val - (o2_val - o2_val) * w_y - o2_grd * v_y - o2_grd * u_y;
        let k_x = e_x - (w_x + w_x) + ONE;
        let g_x = t1 * k_x + mid_x * SIX * d_x + t3 * e_x;

        let k_y = e_y - (w_y + w_y) + ONE;
        let g_y = k_y * f9 + f8 * SIX * d_y + e_y * f4;

        let s1 = (o1_grd - o1_grd) * w_y + o1_grd * v_y + o1_grd * u_y + o1_grd;
        let s2 = (o1_val - o1_val) * w_y + o1_grd * v_y + o1_grd * u_y + o1_val;
        let s3 = s1;
        let mid_z = s2 - o1_val - (o1_val - o1_val) * w_y - o1_grd * v_y - o1_grd * u_y;
        let k_z = e_z - (w_z + w_z) + ONE;
        let g_z = s1 * k_z + mid_z * SIX * d_z + s3 * e_z;

        let n = f9 * v_y + f8 * w_y + f4 * u_y + f7;
        let m = n * SCALE + BIAS;
        let want = f64::from((THREE - (m + m)) * (m * m));

        assert_eq!(got.to_bits(), want.to_bits());
        assert_eq!(got_grad[0].to_bits(), g_x.to_bits());
        assert_eq!(got_grad[1].to_bits(), g_y.to_bits());
        assert_eq!(got_grad[2].to_bits(), g_z.to_bits());
    }

    /// Zero hash table => the sample depends only on the fractional parts.
    ///
    /// So integer translation (and the negative-side floor fix-up landing on
    /// the same fractions) reproduces the value bit-for-bit.
    #[test]
    fn zero_perm_translation_and_negative_floor() {
        let tabs = ramp_tables();
        let perm = [0_u32; 64];
        let base = sample(&perm, &tabs, 0.25, 0.5, 0.75);
        let shifted = sample(&perm, &tabs, 5.25, 7.5, 9.75);
        let negative = sample(&perm, &tabs, -0.75, -1.5, -3.25);
        assert_eq!(base.0.to_bits(), shifted.0.to_bits());
        assert_eq!(base.0.to_bits(), negative.0.to_bits());
        for i in 0..3 {
            assert_eq!(base.1[i].to_bits(), shifted.1[i].to_bits());
            assert_eq!(base.1[i].to_bits(), negative.1[i].to_bits());
        }
    }

    /// The Hermite construction is C1 across lattice planes.
    ///
    /// Corner hashes are shared between adjacent cells (the cascade re-derives
    /// them), the blend weight hits 1 and both derivative weights hit 0 at t=1.
    /// Sample just below and exactly on integer planes on every axis (including
    /// a negative crossing exercising the floor fix-up) and require value and
    /// gradient to agree within a slope bound.
    #[test]
    fn lattice_continuity_all_axes() {
        let tabs = ramp_tables();
        let perm = lcg_perm();
        let eps = 1e-4_f64;
        let cases = [
            ((2.0 - eps, 0.3, 0.7), (2.0, 0.3, 0.7)),
            ((0.3, -1.0, 0.7), (0.3, -1.0 + eps, 0.7)),
            ((1.5, 2.5, 3.0 - eps), (1.5, 2.5, 3.0)),
        ];
        for (lo, hi) in cases {
            let a = sample(&perm, &tabs, lo.0, lo.1, lo.2);
            let b = sample(&perm, &tabs, hi.0, hi.1, hi.2);
            assert!(
                (a.0 - b.0).abs() < 1e-2,
                "value jump at {lo:?}/{hi:?}: {} vs {}",
                a.0,
                b.0
            );
            for i in 0..3 {
                assert!(
                    (a.1[i] - b.1[i]).abs() < 5e-2,
                    "gradient lane {i} jump at {lo:?}/{hi:?}: {} vs {}",
                    a.1[i],
                    b.1[i]
                );
            }
        }
    }

    /// Same inputs => bit-identical outputs.
    ///
    /// Everything stays finite over a spread of cells with a pseudo-random hash
    /// table.
    #[test]
    fn deterministic_and_finite() {
        let tabs = ramp_tables();
        let perm = lcg_perm();
        let probes = [(0.1, 0.2, 0.3), (-7.6, 12.4, -0.5), (123.9, -456.1, 789.5)];
        for (x, y, z) in probes {
            let a = sample(&perm, &tabs, x, y, z);
            let b = sample(&perm, &tabs, x, y, z);
            assert_eq!(a.0.to_bits(), b.0.to_bits());
            assert!(a.0.is_finite());
            for i in 0..3 {
                assert_eq!(a.1[i].to_bits(), b.1[i].to_bits());
                assert!(a.1[i].is_finite());
            }
        }
    }
}

/// `Math::Pow` — the CRT `_CIpow(base, exp)` intrinsic (thunk 0x73f940 + core 0x73f962).
///
/// In the 1.12 client an x87 `fyl2x`/`f2xm1`/`fscale` chain wrapped in
/// `FSAVE`/`FRSTOR`, hot under Rosetta because those transcendentals trap to
/// microcode helpers. Computed in f64 via the pure-Rust `libm` (SSE2, no x87
/// libm call); the `x87pow` shim keeps the intrinsic's register contract (args
/// in `ST(1)`/`ST(0)`, both consumed; result in `ST(0)`) balanced. Hooking the
/// thunk also makes the core and `Math::Exp2` (0x744b00) dead — the core is the
/// thunk's only callee, and Exp2's only caller is the core.
///
/// `libm::pow` follows IEEE-754 / C99 `pow`, which is precisely what the
/// original's `_pow_handle_inf` special-case table reproduces (|x| ≷ 1 with
/// y = ±INF, x = ±INF, and the negative-base integer-exponent parity sign), so
/// the special cases match without porting that table by hand.
pub fn math__pow_x87(base: f64, exp: f64) -> f64 {
    libm::pow(base, exp)
}

#[cfg(test)]
mod tests_math__pow_x87 {
    use super::math__pow_x87 as pow;

    // Cross-check against the host libm (`f64::powf`) over a sweep of finite
    // pairs — both compute correctly-rounded `pow`, so they agree to < 1 ulp.
    #[test]
    fn matches_host_powf_finite() {
        let bases = [0.0f64, 0.5, 1.0, 2.0, 2.5, 10.0, 0.1, 123.456, 1e-8, 1e8];
        let exps = [0.0f64, 1.0, 2.0, 3.0, 0.5, -1.0, -2.5, 7.0, 0.333, -0.75];
        for &b in &bases {
            for &e in &exps {
                let got = pow(b, e);
                let want = b.powf(e);
                if want.is_finite() {
                    let tol = want.abs() * 1e-12 + 1e-300;
                    assert!((got - want).abs() <= tol, "pow({b},{e}) = {got} vs {want}");
                } else {
                    assert_eq!(got.to_bits(), want.to_bits(), "pow({b},{e}) nonfinite");
                }
            }
        }
    }

    #[test]
    fn ieee_special_cases() {
        assert_eq!(pow(1.0, f64::INFINITY).to_bits(), 1.0f64.to_bits()); // 1^anything = 1
        assert_eq!(pow(123.0, 0.0).to_bits(), 1.0f64.to_bits()); // x^0 = 1
        assert_eq!(pow(0.0, 3.0), 0.0);
        assert_eq!(pow(2.0, 10.0), 1024.0);
        // negative base, integer exponent: sign by parity
        assert_eq!(pow(-2.0, 3.0), -8.0);
        assert_eq!(pow(-2.0, 2.0), 4.0);
        // |x| < 1 ^ +INF -> 0 ; |x| > 1 ^ +INF -> +INF
        assert_eq!(pow(0.5, f64::INFINITY), 0.0);
        assert_eq!(pow(2.0, f64::INFINITY), f64::INFINITY);
        assert!(pow(-1.0, 0.5).is_nan()); // negative base, non-integer exp -> NaN
    }
}
