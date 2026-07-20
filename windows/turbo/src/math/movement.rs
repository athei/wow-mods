//! `movement` family kernels.
#![allow(
    non_snake_case,
    // intentional NaN-reject via `!(a >= b)`
    // (differs from `a < b` on NaN), bit-exact source constants kept verbatim,
    // and ABI-dictated parameter counts.
    clippy::neg_cmp_op_on_partial_ord,
    clippy::excessive_precision,
    clippy::too_many_arguments
)]

/// `CMovement::AccelCurveOffset` — eased speed for a `delta_ms` step plus the
/// receiver's positional bias (`+0x18`).
///
/// Converts `delta_ms` to seconds (`* 0.001`), runs the acceleration curve over
/// that interval (see [`c_movement__apply_accel_curve__7c5e70`]) using the live
/// speed `cur_speed` (`+0xa0`) and `flags`, then adds the bias.
pub fn c_movement__accel_curve_offset__7c5f30(
    flags: u32,
    cur_speed: f32,
    delta_ms: u32,
    bias: f32,
) -> f32 {
    // ms -> seconds (DAT 0x801360 = 0.001).
    const SEC_PER_MS: f32 = 0.001;
    let dt = delta_ms as f32 * SEC_PER_MS;
    c_movement__apply_accel_curve__7c5e70(flags, cur_speed, dt) + bias
}

#[cfg(test)]
mod tests_c_movement__accel_curve_offset__7c5f30 {
    use super::{
        c_movement__accel_curve_offset__7c5f30 as f, c_movement__apply_accel_curve__7c5e70 as acc,
    };
    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn equals_curve_plus_bias() {
        let dt = 250.0_f32 * 0.001;
        assert!(close(f(0, 0.0, 250, 12.5), acc(0, 0.0, dt) + 12.5, 1e-5));
    }
    #[test]
    fn bias_is_additive() {
        assert!(close(f(0, 1.0, 100, 5.0) - f(0, 1.0, 100, 0.0), 5.0, 1e-4));
    }
    #[test]
    fn zero_delta_is_just_bias() {
        assert!(close(f(0, 4.0, 0, 7.25), 7.25, 1e-5));
    }
    #[test]
    fn ms_to_seconds_scale() {
        // 1000 ms == 1.0 s.
        assert!(close(f(0, 0.0, 1000, 0.0), acc(0, 0.0, 1.0), 1e-4));
    }
}

/// `CMovement::ApplyAccelCurve` — integrate the current speed toward its capped
/// maximum over `dt`, returning the eased speed.
///
/// `cur_speed` is the receiver's live speed (`+0xa0`); `flags & 0x2000_0000`
/// selects the cap (7.0, else 60.148). The current speed is first clamped down
/// to the cap, then a constant-acceleration ramp is applied: while the linear
/// projection `cur + accel*dt` stays under the cap the eased speed grows
/// quadratically; once it would overshoot, the curve splices the remaining time
/// at the capped speed.
pub fn c_movement__apply_accel_curve__7c5e70(flags: u32, cur_speed: f32, dt: f32) -> f32 {
    const ALT_CAP_FLAG: u32 = 0x2000_0000;
    // Speed caps (DAT 0x87d894 / 0x87d898).
    const CAP_DEFAULT: f32 = 60.148_003;
    const CAP_ALT: f32 = 7.0;
    // ACCEL (0x81da58) = 19.291105, HALF_ACCEL (0x81da60) = 9.645553 = ACCEL/2,
    // RECIP_ACCEL (0x80e020) = 0.05183736 = 1/ACCEL.
    const ACCEL: f32 = 19.291_105;
    const HALF_ACCEL: f32 = 9.645_553;
    const RECIP_ACCEL: f32 = 0.051_837_362;

    let cap = if (flags & ALT_CAP_FLAG) != 0 {
        CAP_ALT
    } else {
        CAP_DEFAULT
    };
    // Clamp current speed down to the cap.
    let cur = if cap < cur_speed { cap } else { cur_speed };

    if dt * ACCEL + cur < cap {
        // Still ramping: quadratic in dt.
        (dt * HALF_ACCEL + cur) * dt
    } else {
        // Splice: ramp to the cap over `t`, then cruise the remaining `dt - t`.
        let t = (cap - cur) * RECIP_ACCEL;
        (dt - t) * cap + (HALF_ACCEL * t + cur) * t
    }
}

#[cfg(test)]
mod tests_c_movement__apply_accel_curve__7c5e70 {
    use super::c_movement__apply_accel_curve__7c5e70 as f;
    const ALT: u32 = 0x2000_0000;
    const CAP_DEFAULT: f32 = 60.148_003;
    const HALF_ACCEL: f32 = 9.645_553;
    const RECIP_ACCEL: f32 = 0.051_837_362;
    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn ramp_branch_from_rest_small_dt() {
        let dt = 0.5_f32; // 0.5*19.29 = 9.6 < 60.15
        assert!(close(f(0, 0.0, dt), (dt * HALF_ACCEL) * dt, 1e-4));
    }
    #[test]
    fn splice_branch_large_dt() {
        let (dt, cur, cap) = (5.0_f32, 0.0_f32, CAP_DEFAULT);
        let t = (cap - cur) * RECIP_ACCEL;
        let want = (dt - t) * cap + (HALF_ACCEL * t + cur) * t;
        assert!(close(f(0, cur, dt), want, 1e-2));
    }
    #[test]
    fn alt_cap_selected_by_flag() {
        let dt = 0.5_f32;
        let (alt, def) = (f(ALT, 0.0, dt), f(0, 0.0, dt));
        assert!((alt - def).abs() > 1e-3 && alt <= 7.0 * dt + 1e-3);
    }
    #[test]
    fn clamps_current_speed_to_cap() {
        let dt = 0.1_f32;
        // cur clamped to cap => splice with t=0 => dt*cap (pure cruise).
        assert!(close(f(0, 1000.0, dt), dt * CAP_DEFAULT, 1e-2));
    }
    #[test]
    fn monotonic_in_dt() {
        let mut prev = f(0, 0.0, 0.0);
        for i in 1..50 {
            let cur = f(0, 0.0, i as f32 * 0.1);
            assert!(cur >= prev - 1e-4);
            prev = cur;
        }
    }
    #[test]
    fn zero_dt_zero_distance() {
        assert!(close(f(0, 3.0, 0.0), 0.0, 1e-6));
    }
}

/// `CMovement::ComputeForwardVector` — derive the cached facing/heading direction
/// fields from the scalar `facing` (yaw) and `pitch` angles.
///
/// Returns the seven cached floats in struct order `[heading_x(+0x5c),
/// heading_y(+0x60), heading_z(+0x64), cos_facing(+0x68), sin_facing(+0x6c),
/// cos_pitch(+0x70), sin_pitch(+0x74)]`.
///
/// When the receiver is swim-capable (`flags & 0x0020_0000 != 0`) and the pitch
/// is non-negligible (`|pitch| >= 2^-20`), the heading is the full 3-D unit
/// vector tilted by pitch; otherwise it collapses to the flat 2-D facing
/// direction with an upright pitch basis `(cos, sin) = (1, 0)`.
pub fn c_movement__compute_forward_vector__7c5880(flags: u32, facing: f32, pitch: f32) -> [f32; 7] {
    const SWIM_FLAG: u32 = 0x0020_0000;
    // 2^-20, the engine's "pitch is significant" epsilon (DAT at VA 0x8026bc).
    const PITCH_EPS: f32 = 9.536_743_16e-7;

    let (sin_f, cos_f) = crate::math::trig::sin_cos(facing);

    if (flags & SWIM_FLAG) != 0 && pitch.abs() >= PITCH_EPS {
        let (sin_p, cos_p) = crate::math::trig::sin_cos(pitch);
        // heading = (cos_f*cos_p, sin_f*cos_p, sin_p).
        [
            cos_f * cos_p,
            sin_f * cos_p,
            sin_p,
            cos_f,
            sin_f,
            cos_p,
            sin_p,
        ]
    } else {
        // flat: heading = (cos_f, sin_f, 0); upright pitch basis (1, 0).
        [cos_f, sin_f, 0.0, cos_f, sin_f, 1.0, 0.0]
    }
}

#[cfg(test)]
mod tests_c_movement__compute_forward_vector__7c5880 {
    use super::c_movement__compute_forward_vector__7c5880 as f;
    const SWIM: u32 = 0x0020_0000;
    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn flat_zero_facing() {
        let o = f(0, 0.0, 0.0);
        assert!(close(o[0], 1.0, 5e-6) && close(o[1], 0.0, 5e-6) && close(o[2], 0.0, 5e-6));
        assert!(close(o[3], 1.0, 5e-6) && close(o[4], 0.0, 5e-6));
        assert!(close(o[5], 1.0, 5e-6) && close(o[6], 0.0, 5e-6));
    }
    #[test]
    fn flat_ignores_pitch_without_swim_flag() {
        let o = f(0, 0.0, 0.7);
        assert!(close(o[2], 0.0, 5e-6) && close(o[5], 1.0, 5e-6) && close(o[6], 0.0, 5e-6));
    }
    #[test]
    fn flat_ignores_tiny_pitch_with_swim_flag() {
        let o = f(SWIM, 1.0, 1e-9);
        assert!(close(o[2], 0.0, 5e-6) && close(o[5], 1.0, 5e-6));
    }
    #[test]
    fn full_3d_unit_heading_when_swimming() {
        let (facing, pitch) = (0.6_f32, 0.4_f32);
        let o = f(SWIM, facing, pitch);
        let len2 = o[0] * o[0] + o[1] * o[1] + o[2] * o[2];
        assert!(close(len2, 1.0, 1e-5), "len2 = {len2}");
        assert!(close(o[3], facing.cos(), 5e-6) && close(o[4], facing.sin(), 5e-6));
        assert!(close(o[5], pitch.cos(), 5e-6) && close(o[6], pitch.sin(), 5e-6));
        assert!(close(o[2], pitch.sin(), 5e-6));
    }
    #[test]
    fn cos_sin_facing_always_present() {
        for &facing in &[-2.0_f32, -0.3, 0.0, 1.1, 3.0] {
            let o = f(0, facing, 0.0);
            assert!(close(o[3] * o[3] + o[4] * o[4], 1.0, 1e-5));
        }
    }
}

/// Gate predicate for the active spline/velocity source: the per-step source
/// block is active iff the gating scalar is not bit-exactly equal to the host
/// zero sentinel (`x != sentinel`, matching the original's `fcomp`/`jne`; a NaN
/// scalar is unordered and so counts as active).
pub fn c_movement__get_active_spline_source__618920(scalar: f32, sentinel: f32) -> bool {
    scalar != sentinel
}

#[cfg(test)]
mod tests_c_movement__get_active_spline_source__618920 {
    use super::c_movement__get_active_spline_source__618920 as g;
    #[test]
    fn equal_sentinel_inactive() {
        assert!(!g(0.0, 0.0));
        assert!(!g(2.5, 2.5));
    }
    #[test]
    fn nonzero_scalar_active() {
        assert!(g(1.0, 0.0));
        assert!(g(-3.0, 0.0));
    }
    #[test]
    fn nan_scalar_active() {
        assert!(g(f32::NAN, 0.0));
    }
}

/// Resolves the forward-speed source scalar from a movement flag word and the
/// raw `+0x9c` value: bit `0x10` selects `+raw`, bit `0x20` selects `-raw`, else
/// the host zero sentinel; the result is scaled by `mult` when any low/bias bit
/// (`0x200f`) is set.
pub fn c_movement__resolve_forward_speed__7c5c50(
    flags: u32,
    raw: f32,
    zero: f32,
    mult: f32,
) -> f32 {
    let base = if (flags & 0x10) != 0 {
        raw
    } else if (flags & 0x20) != 0 {
        -raw
    } else {
        zero
    };
    if (flags & 0x200f) != 0 {
        base * mult
    } else {
        base
    }
}

#[cfg(test)]
mod tests_c_movement__resolve_forward_speed__7c5c50 {
    use super::c_movement__resolve_forward_speed__7c5c50 as fwd;
    #[test]
    fn positive_bit() {
        assert_eq!(fwd(0x10, 3.5, -99.0, 2.0), 3.5);
    }
    #[test]
    fn negative_bit() {
        assert_eq!(fwd(0x20, 3.5, -99.0, 2.0), -3.5);
    }
    #[test]
    fn zero_sentinel() {
        assert_eq!(fwd(0x0, 3.5, 12.0, 2.0), 12.0);
    }
    #[test]
    fn bias_multiplies() {
        assert_eq!(fwd(0x11, 3.0, 0.0, 2.0), 6.0);
    }
    #[test]
    fn bit10_takes_priority_over_20() {
        assert_eq!(fwd(0x30, 4.0, 0.0, 1.0), 4.0);
    }
}

/// Resolves the turn-rate source scalar: bit `0x40` selects `+raw`, the sign bit
/// (`0x8000_0000`) selects `-raw`, else the host zero sentinel; scaled by `mult`
/// when any low/bias bit (`0x200f`) is set.
pub fn c_movement__resolve_turn_rate__7c5c90(flags: u32, raw: f32, zero: f32, mult: f32) -> f32 {
    let base = if (flags & 0x40) != 0 {
        raw
    } else if (flags & 0x8000_0000) != 0 {
        -raw
    } else {
        zero
    };
    if (flags & 0x200f) != 0 {
        base * mult
    } else {
        base
    }
}

#[cfg(test)]
mod tests_c_movement__resolve_turn_rate__7c5c90 {
    use super::c_movement__resolve_turn_rate__7c5c90 as turn;
    #[test]
    fn positive_bit() {
        assert_eq!(turn(0x40, 2.0, -1.0, 3.0), 2.0);
    }
    #[test]
    fn sign_bit_negates() {
        assert_eq!(turn(0x8000_0000, 2.0, -1.0, 3.0), -2.0);
    }
    #[test]
    fn zero_sentinel() {
        assert_eq!(turn(0x0, 2.0, 7.5, 3.0), 7.5);
    }
    #[test]
    fn bias_multiplies() {
        assert_eq!(turn(0x42, 2.0, 0.0, 3.0), 6.0);
    }
    #[test]
    fn bit40_priority_over_sign() {
        assert_eq!(turn(0x8000_0040, 5.0, 0.0, 1.0), 5.0);
    }
}

/// Planar arc integration (forward + yaw turn, no pitch). Given the per-step
/// forward `speed` (`+0x84`), the resolved turn `rate`, the timestep `dt`, the
/// facing cosine/sine `cf`/`sf` (`+0x68`/`+0x6c`) and the planar scales `s0`/`s1`
/// (`+0x70`/`+0x74`), returns the local-space displacement `[dx, dy, dz]`.
///
/// `versine = inv*(1 - cos)` where `inv = speed/rate`; the horizontal offset is
/// the planar arc chord rotated by facing and scaled by `s0`, and the vertical
/// component is the straight-line `speed*s1*dt`.
pub fn c_movement__integrate_arc_flat_turn__7c52c0(
    speed: f32,
    rate: f32,
    dt: f32,
    cf: f32,
    sf: f32,
    s0: f32,
    s1: f32,
) -> [f32; 3] {
    let inv = speed / rate;
    let theta = rate * dt;
    let (s, c) = crate::math::trig::sin_cos(theta);
    let versine = inv - c * inv;
    let dx = (s * inv * cf - versine * sf) * s0;
    let dy = (s * inv * sf + versine * cf) * s0;
    let dz = speed * s1 * dt;
    [dx, dy, dz]
}

#[cfg(test)]
mod tests_c_movement__integrate_arc_flat_turn__7c52c0 {
    use super::c_movement__integrate_arc_flat_turn__7c52c0 as f;
    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }
    #[test]
    fn zero_dt_zero_delta() {
        let o = f(5.0, 2.0, 0.0, 1.0, 0.0, 1.0, 1.0);
        assert!(close(o[0], 0.0, 1e-6) && close(o[1], 0.0, 1e-6) && close(o[2], 0.0, 1e-6));
    }
    #[test]
    fn vertical_is_speed_scale_dt() {
        let o = f(4.0, 1.0, 0.5, 0.3, 0.7, 2.0, 1.5);
        assert!(close(o[2], 4.0 * 1.5 * 0.5, 1e-5));
    }
    #[test]
    fn facing_rotation_preserves_radius() {
        let (speed, rate, dt, s0, s1) = (3.0f32, 1.5f32, 0.4f32, 1.0f32, 1.0f32);
        let mag = |cf: f32, sf: f32| {
            let o = f(speed, rate, dt, cf, sf, s0, s1);
            (o[0] * o[0] + o[1] * o[1]).sqrt()
        };
        let m0 = mag(1.0, 0.0);
        let m1 = mag(0.6, 0.8);
        let m2 = mag(-0.5, 0.75f32.sqrt());
        assert!(close(m0, m1, 1e-4) && close(m1, m2, 1e-4), "{m0} {m1} {m2}");
    }
    #[test]
    fn straight_line_limit_small_turn() {
        let (speed, dt) = (2.0f32, 0.1f32);
        let rate = 1e-3f32;
        let (cf, sf, s0) = (1.0f32, 0.0f32, 1.0f32);
        let o = f(speed, rate, dt, cf, sf, s0, 1.0);
        assert!(close(o[0], speed * dt * cf * s0, 1e-3), "{}", o[0]);
        assert!(o[1].abs() < 1e-3, "{}", o[1]);
    }
}

/// Constant-turn-rate arc integration (forward speed + yaw turn + pitch).
///
/// `speed` (`+0x84`); `fwd` is the resolved forward speed; `rate` the resolved
/// turn rate; `cur_pitch` is `+0x54`; `cf`/`sf` are the yaw cos/sin
/// (`+0x68`/`+0x6c`); `p0`/`p1` the pitch basis (`+0x70`/`+0x74`). `num`
/// (`_DAT_007ff9d8`) is the inverse-rate numerator, `upper`/`lower` the pitch
/// clamp bounds, `eps` the bottom-out epsilon, `kx` (`_DAT_007ffa24`) the yaw
/// cross-coupling scale. The pitch is advanced by `rate*dt` and clamped; when it
/// crosses a bound the leftover `overshoot` is spliced back into the vertical
/// component. Returns the local displacement `[dx, dy, dz]`.
pub fn c_movement__integrate_arc_fwd_turn_pitch__7c4fd0(
    speed: f32,
    fwd: f32,
    rate: f32,
    dt: f32,
    cur_pitch: f32,
    cf: f32,
    sf: f32,
    p0: f32,
    p1: f32,
    num: f32,
    upper: f32,
    lower: f32,
    eps: f32,
    kx: f32,
) -> [f32; 3] {
    let ratio = speed / fwd;
    let inv_rate = num / rate;
    let scaled = inv_rate * speed;

    let mut dt_local = rate * dt;
    let pitch_new = rate * dt + cur_pitch;
    let overshoot;
    if pitch_new > upper {
        overshoot = (pitch_new - upper) * inv_rate;
        dt_local = upper - cur_pitch;
    } else if pitch_new < lower {
        // the original adds `upper` (not `lower`) into the overshoot term.
        overshoot = (pitch_new + upper) * inv_rate;
        dt_local = lower - cur_pitch;
    } else {
        overshoot = 0.0;
    }

    let turn = (dt - overshoot) * fwd;
    let (s1, c1) = crate::math::trig::sin_cos(turn);
    let (s2, c2) = crate::math::trig::sin_cos(dt_local);

    let yaw = (s1 * ratio + s2 * scaled) * kx;
    let r_ver = c1 * ratio - ratio;
    let p_ver = scaled - c2 * scaled;

    let mix = yaw * cf - r_ver * sf;
    let dx = mix * p0 - p_ver * p1;
    let dy = yaw * sf + r_ver * cf;
    let mut dz = mix * p1 + p_ver * p0;

    if overshoot.abs() >= eps {
        let ov = overshoot * speed;
        if ((dt_local + cur_pitch) - upper).abs() < eps {
            dz += ov;
        } else {
            dz -= ov;
        }
    }
    [dx, dy, dz]
}

#[cfg(test)]
mod tests_c_movement__integrate_arc_fwd_turn_pitch__7c4fd0 {
    use super::c_movement__integrate_arc_fwd_turn_pitch__7c4fd0 as f;
    const UP: f32 = core::f32::consts::FRAC_PI_2;
    const LO: f32 = -core::f32::consts::FRAC_PI_2;
    const NUM: f32 = 1.0;
    const EPS: f32 = 9.5e-7;
    const KX: f32 = 1.0;
    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }
    #[test]
    fn zero_dt_in_range_zero_delta() {
        let o = f(
            5.0, 5.0, 2.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, NUM, UP, LO, EPS, KX,
        );
        assert!(close(o[0], 0.0, 1e-6) && close(o[1], 0.0, 1e-6) && close(o[2], 0.0, 1e-6));
    }
    #[test]
    fn dy_uses_yaw_and_radial() {
        let (speed, fwd, rate, dt, cur, cf, sf, p0, p1) = (
            4.0f32, 5.0f32, 1.0f32, 0.3f32, 0.0f32, 0.6f32, 0.8f32, 0.9f32, 0.4f32,
        );
        let o = f(
            speed, fwd, rate, dt, cur, cf, sf, p0, p1, NUM, UP, LO, EPS, KX,
        );
        let ratio = speed / fwd;
        let inv_rate = NUM / rate;
        let scaled = inv_rate * speed;
        let dtl = rate * dt;
        let turn = (dt - 0.0) * fwd;
        let yaw = (turn.sin() * ratio + dtl.sin() * scaled) * KX;
        let r_ver = turn.cos() * ratio - ratio;
        let want_dy = yaw * sf + r_ver * cf;
        assert!(close(o[1], want_dy, 1e-4), "{} {}", o[1], want_dy);
    }
    #[test]
    fn finite_under_clamp() {
        let o = f(
            4.0, 5.0, 6.0, 2.0, 1.2, 0.6, 0.8, 0.9, 0.4, NUM, UP, LO, EPS, KX,
        );
        assert!(o[0].is_finite() && o[1].is_finite() && o[2].is_finite());
    }
    #[test]
    fn vertical_includes_overshoot_when_clamped() {
        let a = (
            4.0f32, 5.0f32, 6.0f32, 2.0f32, 1.2f32, 0.6f32, 0.8f32, 0.9f32, 0.4f32,
        );
        let with = f(
            a.0, a.1, a.2, a.3, a.4, a.5, a.6, a.7, a.8, NUM, UP, LO, EPS, KX,
        );
        let without = f(
            a.0, a.1, a.2, a.3, a.4, a.5, a.6, a.7, a.8, NUM, UP, LO, 1e9, KX,
        );
        assert!(
            (with[2] - without[2]).abs() > 1e-4,
            "{} {}",
            with[2],
            without[2]
        );
    }
}

/// Arc integration with pitch advance/clamp (turn + pitch, implicit speed).
///
/// `speed` is `+0x84`; `rate` is the resolved turn rate; `cur_pitch` is `+0x54`;
/// `cf`/`sf` are the yaw cos/sin (`+0x68`/`+0x6c`); `p0`/`p1` are the pitch basis
/// (`+0x70`/`+0x74`). `num` (`_DAT_007ff9d8`) is the inverse-rate numerator,
/// `upper`/`lower` the pitch clamp bounds, `eps` the bottom-out epsilon. The
/// pitch is advanced by `rate*dt` and clamped; the leftover `overshoot` is
/// spliced back into the vertical component. Returns the local displacement
/// `[dx, dy, dz]`.
pub fn c_movement__integrate_arc_turn_pitch__7c5180(
    speed: f32,
    rate: f32,
    dt: f32,
    cur_pitch: f32,
    cf: f32,
    sf: f32,
    p0: f32,
    p1: f32,
    num: f32,
    upper: f32,
    lower: f32,
    eps: f32,
) -> [f32; 3] {
    let inv_rate = num / rate;
    let scaled = inv_rate * speed;

    let mut dt_local = rate * dt;
    let pitch_new = rate * dt + cur_pitch;
    let overshoot;
    if pitch_new > upper {
        overshoot = (pitch_new - upper) * inv_rate;
        dt_local = upper - cur_pitch;
    } else if pitch_new < lower {
        // the original adds `upper` (not `lower`) into the overshoot term.
        overshoot = (pitch_new + upper) * inv_rate;
        dt_local = lower - cur_pitch;
    } else {
        overshoot = 0.0;
    }

    let (s, c) = crate::math::trig::sin_cos(dt_local);
    let a = s * scaled;
    let b = scaled - c * scaled;

    let dx = (a * p0 - b * p1) * cf;
    let dy = (a * p0 - b * p1) * sf;
    let mut dz = a * p1 + b * p0;

    if overshoot.abs() >= eps {
        let ov = overshoot * speed;
        if ((dt_local + cur_pitch) - upper).abs() < eps {
            dz += ov;
        } else {
            dz -= ov;
        }
    }
    [dx, dy, dz]
}

#[cfg(test)]
mod tests_c_movement__integrate_arc_turn_pitch__7c5180 {
    use super::c_movement__integrate_arc_turn_pitch__7c5180 as f;
    const UP: f32 = core::f32::consts::FRAC_PI_2;
    const LO: f32 = -core::f32::consts::FRAC_PI_2;
    const NUM: f32 = 1.0;
    const EPS: f32 = 9.5e-7;
    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }
    #[test]
    fn zero_dt_zero_delta() {
        let o = f(5.0, 2.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, NUM, UP, LO, EPS);
        assert!(close(o[0], 0.0, 1e-6) && close(o[1], 0.0, 1e-6) && close(o[2], 0.0, 1e-6));
    }
    #[test]
    fn no_clamp_in_range_no_bottom_out() {
        let o = f(3.0, 1.0, 0.2, 0.0, 1.0, 0.0, 1.0, 1.0, NUM, UP, LO, EPS);
        let inv_rate = NUM / 1.0;
        let scaled = inv_rate * 3.0;
        let dtl = 1.0f32 * 0.2;
        let a = dtl.sin() * scaled;
        let b = scaled - dtl.cos() * scaled;
        assert!(close(o[2], a * 1.0 + b * 1.0, 1e-4), "{}", o[2]);
    }
    #[test]
    fn dx_dy_share_yaw_factor() {
        let (cf, sf) = (0.6f32, 0.8f32);
        let o = f(3.0, 1.2, 0.3, 0.1, cf, sf, 0.9, 0.4, NUM, UP, LO, EPS);
        assert!(close(o[0] * sf, o[1] * cf, 1e-4), "{} {}", o[0], o[1]);
    }
    #[test]
    fn upper_clamp_engages_on_large_pitch() {
        let cur_pitch = 1.0f32;
        let o = f(
            3.0, 5.0, 2.0, cur_pitch, 1.0, 0.0, 1.0, 0.0, NUM, UP, LO, EPS,
        );
        assert!(o[2].is_finite());
    }
}

/// `CMovement::ComputeBoxGroundNormal` (plane-assembly kernel).
///
/// Builds the six axis-aligned clip planes of the mover's collision box from its
/// horizontal half-extents `x_he`/`y_he`, vertical half-extent `z_he`, the
/// horizontal contact radius `h_rad` and the vertical contact radius `v_rad`.
///
/// Each plane is four floats `[nx, ny, nz, d]` (outward unit normal + signed
/// offset). The `+X`/`-X` and `+Y`/`-Y` faces are inset by `h_rad`, the bottom
/// (`+Z`) face by `v_rad`, and the top (`-Z`) face uses the bare vertical
/// half-extent.
pub fn c_movement__compute_box_ground_normal__6332a0(
    x_he: f32,
    y_he: f32,
    z_he: f32,
    h_rad: f32,
    v_rad: f32,
) -> [f32; 24] {
    [
        // +X face
        1.0,
        0.0,
        0.0,
        -x_he - h_rad,
        // -X face
        -1.0,
        0.0,
        0.0,
        x_he - h_rad,
        // +Y face
        0.0,
        1.0,
        0.0,
        -y_he - h_rad,
        // -Y face
        0.0,
        -1.0,
        0.0,
        y_he - h_rad,
        // +Z (bottom) face
        0.0,
        0.0,
        1.0,
        -z_he - v_rad,
        // -Z (top) face
        0.0,
        0.0,
        -1.0,
        z_he,
    ]
}

#[cfg(test)]
mod tests_c_movement__compute_box_ground_normal__6332a0 {
    use super::c_movement__compute_box_ground_normal__6332a0 as f;

    #[test]
    fn unit_normals_are_axis_aligned() {
        let p = f(2.0, 3.0, 4.0, 0.5, 0.25);
        for i in 0..6 {
            let n = [p[i * 4], p[i * 4 + 1], p[i * 4 + 2]];
            let len2 = n[0] * n[0] + n[1] * n[1] + n[2] * n[2];
            assert!((len2 - 1.0).abs() < 1e-6, "plane {i} normal not unit");
        }
    }
    #[test]
    fn opposite_face_pairs_have_opposite_normals() {
        let p = f(2.0, 3.0, 4.0, 0.5, 0.25);
        for &(a, b) in &[(0usize, 1usize), (2, 3), (4, 5)] {
            for k in 0..3 {
                assert_eq!(p[a * 4 + k], -p[b * 4 + k]);
            }
        }
    }
    #[test]
    fn horizontal_offsets_inset_by_h_rad() {
        let (x, y, _z, h, _v) = (2.0_f32, 3.0_f32, 4.0_f32, 0.5_f32, 0.25_f32);
        let p = f(x, y, 4.0, h, 0.25);
        assert_eq!(p[3], -x - h);
        assert_eq!(p[7], x - h);
        assert_eq!(p[11], -y - h);
        assert_eq!(p[15], y - h);
    }
    #[test]
    fn vertical_offsets_use_v_rad_and_bare_extent() {
        let (z, v) = (4.0_f32, 0.25_f32);
        let p = f(2.0, 3.0, z, 0.5, v);
        assert_eq!(p[19], -z - v);
        assert_eq!(p[23], z);
    }
    #[test]
    fn z_axis_plane_normals() {
        let p = f(2.0, 3.0, 4.0, 0.5, 0.25);
        assert_eq!([p[16], p[17], p[18]], [0.0, 0.0, 1.0]);
        assert_eq!([p[20], p[21], p[22]], [0.0, 0.0, -1.0]);
    }
}

/// `CMovement::IsPositionFinite` — validate that the mover position `(x, y, z)`
/// is finite and that its world-plane coordinates lie inside the playable world
/// rectangle.
///
/// Each component must be a finite float (not Inf/NaN). The X/Y coordinates are
/// re-expressed relative to the world origin (`-(coord - bias)`) and each must
/// satisfy `min <= v < max`. Returns `1` when finite and in bounds, else `0`.
pub fn c_movement__is_position_finite__616bf0(
    x: f32,
    y: f32,
    z: f32,
    bias: f32,
    min: f32,
    max: f32,
) -> i32 {
    if !x.is_finite() || !y.is_finite() || !z.is_finite() {
        return 0;
    }
    // World-relative X/Y: `-(coord - bias)` (the original negates after biasing).
    let ny = -(y - bias);
    let nx = -(x - bias);
    // Each axis: `min <= v` (inclusive low) and `v < max` (strict high).
    if min <= ny && ny < max && min <= nx && nx < max {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests_c_movement__is_position_finite__616bf0 {
    use super::c_movement__is_position_finite__616bf0 as f;

    const BIAS: f32 = 17066.666;
    const MIN: f32 = -17066.666;
    const MAX: f32 = 17066.666;

    #[test]
    fn center_is_in_bounds() {
        assert_eq!(f(BIAS, BIAS, 0.0, BIAS, MIN, MAX), 1);
    }
    #[test]
    fn nan_or_inf_component_rejected() {
        assert_eq!(f(f32::NAN, BIAS, 0.0, BIAS, MIN, MAX), 0);
        assert_eq!(f(BIAS, f32::INFINITY, 0.0, BIAS, MIN, MAX), 0);
        assert_eq!(f(BIAS, BIAS, f32::NEG_INFINITY, BIAS, MIN, MAX), 0);
    }
    #[test]
    fn z_must_be_finite_but_is_not_range_checked() {
        assert_eq!(f(BIAS, BIAS, 1.0e30, BIAS, MIN, MAX), 1);
    }
    #[test]
    fn low_bound_is_inclusive() {
        let y2 = BIAS - MIN;
        assert_eq!(f(BIAS, y2, 0.0, BIAS, MIN, MAX), 1);
    }
    #[test]
    fn high_bound_is_exclusive_at_max() {
        let y = BIAS - MAX;
        assert_eq!(f(BIAS, y, 0.0, BIAS, MIN, MAX), 0);
    }
    #[test]
    fn both_axes_must_pass() {
        let x = BIAS - MAX;
        assert_eq!(f(x, BIAS, 0.0, BIAS, MIN, MAX), 0);
    }
}

/// `CMovement::StepSplineAndSnapPosition` (candidate-position + snap-decision
/// kernel).
///
/// Given the mover's current planar position `pos` (x at `+0x10`, y at `+0x14`),
/// the spline base point `base` (`+0x44..+0x4c`) and the per-step spline
/// displacement `delta`, this builds the candidate world position and decides
/// whether to snap to it.
///
/// The candidate is `base + delta`. The snap is a desync fail-safe: the live
/// position is overwritten with the candidate only when the squared planar
/// deviation `dist2` EXCEEDS a tolerance — the normalised form
/// `dist2 / step^2 > tol_norm` (where `step = packed_speed * sec_per_ms`) OR
/// the raw form `dist2 > tol_raw`; a deviation within both tolerances leaves
/// the (ground-clamped) live position untouched. Unordered compares snap too
/// (a NaN deviation recovers onto the spline), hence the negated `<=` forms.
/// Returns `(candidate[x,y,z], snap)`.
pub fn c_movement__step_spline_and_snap_position__616af0(
    pos_x: f32,
    pos_y: f32,
    base: [f32; 3],
    delta: [f32; 3],
    packed_speed: u32,
    sec_per_ms: f32,
    tol_norm: f32,
    tol_raw: f32,
) -> ([f32; 3], bool) {
    let candidate = [base[0] + delta[0], base[1] + delta[1], base[2] + delta[2]];

    let dx = candidate[0] - pos_x;
    let dy = candidate[1] - pos_y;
    let dist2 = dx * dx + dy * dy;

    // step = packed_speed * sec_per_ms; normalised distance = dist2 / step^2.
    let step = packed_speed as f32 * sec_per_ms;
    let norm = dist2 / (step * step);

    // Snap when either tolerance is exceeded or unordered; both boundary
    // equalities stay put.
    let snap = !(norm <= tol_norm) || !(dist2 <= tol_raw);
    (candidate, snap)
}

#[cfg(test)]
mod tests_c_movement__step_spline_and_snap_position__616af0 {
    use super::c_movement__step_spline_and_snap_position__616af0 as f;

    #[test]
    fn candidate_is_base_plus_delta() {
        let (c, _) = f(
            0.0,
            0.0,
            [10.0, 20.0, 30.0],
            [1.0, 2.0, 3.0],
            100,
            0.001,
            1.0,
            1e9,
        );
        assert_eq!(c, [11.0, 22.0, 33.0]);
    }
    #[test]
    fn deviation_within_both_tolerances_stays_put() {
        // dist2 = 1e-6, step = 0.001 => norm = 1.0; both tolerances cover.
        let (_c, snap) = f(
            0.0,
            0.0,
            [0.001, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            1,
            0.001,
            2.0,
            1e-3,
        );
        assert!(!snap);
    }
    #[test]
    fn large_deviation_snaps_when_both_tolerances_tight() {
        let (_c, snap) = f(
            0.0,
            0.0,
            [100.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            1,
            0.001,
            1e-6,
            1e-6,
        );
        assert!(snap);
    }
    #[test]
    fn exceeding_normalised_tolerance_alone_snaps() {
        // norm = 1.0 > 0.5 while dist2 = 1e-6 <= tol_raw: OR still snaps.
        let (_c, snap) = f(
            0.0,
            0.0,
            [0.001, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            1,
            0.001,
            0.5,
            1.0,
        );
        assert!(snap);
    }
    #[test]
    fn exceeding_raw_tolerance_alone_snaps() {
        // dist2 = 4.0 > 1.0 while norm = 400 <= tol_norm: OR still snaps.
        let (_c, snap) = f(
            0.0,
            0.0,
            [2.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            100,
            0.001,
            1000.0,
            1.0,
        );
        assert!(snap);
    }
    #[test]
    fn boundary_equality_is_inclusive_and_stays_put() {
        // step = 2 * 0.5 = 1.0 exactly => norm == tol_norm and dist2 == tol_raw.
        let (_c, snap) = f(0.0, 0.0, [1.0, 0.0, 0.0], [0.0, 0.0, 0.0], 2, 0.5, 1.0, 1.0);
        assert!(!snap);
    }
    #[test]
    fn nan_deviation_snaps() {
        let (_c, snap) = f(
            f32::NAN,
            0.0,
            [0.001, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            1,
            0.001,
            2.0,
            1e-3,
        );
        assert!(snap);
    }
    #[test]
    fn distance_is_planar_only_z_ignored() {
        let (_c, snap) = f(
            0.0,
            0.0,
            [0.0, 0.0, 1e6],
            [0.0, 0.0, 0.0],
            1,
            0.001,
            1.0,
            1e-3,
        );
        assert!(!snap, "z should not contribute to planar distance");
    }
}

/// `CMovement::UpdateSplinePath` — track progress: parameter `t` in
/// `[0, 1]` for `elapsed_ms` against `duration_ms`, plus a completion
/// latch (raised only when a non-zero duration has fully elapsed).
///
/// Integer semantics mirror the source: a zero duration pins `t = 1`
/// without latching, a negative elapsed pins `t = 0`, the
/// elapsed-vs-duration compare is unsigned, and both int→float
/// conversions are signed.
pub fn c_movement__update_spline_path__7c5490_track_param(
    elapsed_ms: i32,
    duration_ms: i32,
) -> (f32, bool) {
    if duration_ms == 0 {
        return (1.0, false);
    }
    if elapsed_ms < 0 {
        return (0.0, false);
    }
    if (elapsed_ms as u32) < (duration_ms as u32) {
        (elapsed_ms as f32 / duration_ms as f32, false)
    } else {
        (1.0, true)
    }
}

/// `CMovement::UpdateSplinePath` — facing yaw from the evaluated 2D
/// direction: `atan2(y, x)` wrapped from `(-pi, pi]` into `[0, two_pi)`
/// by adding `two_pi` to strictly negative results.
///
/// `atan2(0, 0)` is `0` (the hardware `fpatan` result for a degenerate
/// vertical-tangent direction), so a zero direction yields yaw `0`
/// rather than a non-finite value.
pub fn c_movement__update_spline_path__7c5490_yaw(
    dir_x: f32,
    dir_y: f32,
    zero: f32,
    two_pi: f32,
) -> f32 {
    let yaw = crate::math::trig::atan2(dir_y, dir_x);
    if yaw < zero { yaw + two_pi } else { yaw }
}

/// `CMovement::UpdateSplinePath` — pitch from the evaluated direction's
/// vertical component: `asin(z)` via the source's x87 identity
/// `atan2(z, sqrt((1-z)(1+z)))`; the product is floored at zero so a
/// `|z|` slightly above 1 saturates to `±pi/2` instead of going
/// non-finite.
pub fn c_movement__update_spline_path__7c5490_pitch(dir_z: f32) -> f32 {
    crate::math::trig::atan2(dir_z, ((1.0 - dir_z) * (1.0 + dir_z)).max(0.0).sqrt())
}

/// `CMovement::UpdateSplinePath` — look-ahead track parameter:
/// `advanced_ms / duration_ms` (signed int→float) clamped to `[0, 1]`.
///
/// Comparison polarity mirrors the source's x87 compares: only a
/// strictly negative quotient pins 0, so an unordered (NaN) quotient
/// from a zero duration saturates to 1.
pub fn c_movement__update_spline_path__7c5490_look_param(
    advanced_ms: i32,
    duration_ms: i32,
    zero: f32,
    one: f32,
) -> f32 {
    let t = advanced_ms as f32 / duration_ms as f32;
    if t < zero {
        0.0
    } else if t < one {
        t
    } else {
        1.0
    }
}

/// `CMovement::UpdateSplinePath` — turn-toward-target forward vector.
///
/// Both the evaluated 2D direction `dir2d` and the target offset
/// `look - resolved` are rescaled to length `one` unless their length is
/// strictly under `eps`; the turn is `±2*acos(dot)` (positive when the
/// 2D cross is below `zero` or unordered) clamped to `[-pi/2, pi/2]`;
/// the forward is `frame_row2` row-multiplied by the Rodrigues rotation
/// of the clamped turn about `axis` (axis normalized by the rotation
/// builder, as in the source).
pub fn c_movement__update_spline_path__7c5490_turn_forward(
    axis: [f32; 3],
    dir2d: [f32; 2],
    look: [f32; 2],
    resolved: [f32; 2],
    frame_row2: [f32; 3],
    zero: f32,
    one: f32,
    eps: f32,
    neg_one: f32,
    neg_half_pi: f32,
    half_pi: f32,
) -> [f32; 3] {
    // Conditional unit rescale (folded `CCubicSpline::SqMag2D` + sqrt):
    // vectors strictly shorter than `eps` pass through unscaled.
    fn rescale(v: [f32; 2], eps: f32, one: f32) -> [f32; 2] {
        let len = (v[0] * v[0] + v[1] * v[1]).sqrt();
        if len.abs() < eps {
            v
        } else {
            let s = one / len;
            [v[0] * s, v[1] * s]
        }
    }
    let v = rescale(dir2d, eps, one);
    let d = rescale([look[0] - resolved[0], look[1] - resolved[1]], eps, one);

    let dot = d[1] * v[1] + d[0] * v[0];
    let cross = d[1] * v[0] - v[1] * d[0];
    let dot = if dot < neg_one {
        -1.0
    } else if dot < one {
        dot
    } else {
        1.0
    };
    // Source polarity: the negated branch is taken only when
    // `cross >= zero`; an unordered compare falls into the positive one.
    let half = crate::math::trig::acos(dot);
    let angle = if !(cross >= zero) { half } else { -half };
    let doubled = angle + angle;
    let turn = if doubled < neg_half_pi {
        -1.570_796_4
    } else if doubled < half_pi {
        doubled
    } else {
        1.570_796_4
    };

    let rot = crate::math::matrix33::c33_matrix__from_axis_angle__7be490(&axis, turn, true);
    let a = frame_row2;
    [
        a[2] * rot[6] + a[1] * rot[3] + a[0] * rot[0],
        a[2] * rot[7] + a[1] * rot[4] + a[0] * rot[1],
        a[2] * rot[8] + a[1] * rot[5] + a[0] * rot[2],
    ]
}

#[cfg(test)]
mod tests_c_movement__update_spline_path__7c5490 {
    use core::f32::consts::{FRAC_PI_2, FRAC_PI_6, PI, TAU};

    use super::{
        c_movement__update_spline_path__7c5490_look_param as look_param,
        c_movement__update_spline_path__7c5490_pitch as pitch,
        c_movement__update_spline_path__7c5490_track_param as track_param,
        c_movement__update_spline_path__7c5490_turn_forward as turn_forward,
        c_movement__update_spline_path__7c5490_yaw as yaw,
    };

    const ZERO: f32 = 0.0;
    const ONE: f32 = 1.0;
    const EPS: f32 = 2.384_185_8e-7;
    const NEG_ONE: f32 = -1.0;
    const NEG_HALF_PI: f32 = -1.570_796_4;
    const HALF_PI: f32 = 1.570_796_4;

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() < tol
    }

    #[test]
    fn track_param_progress_and_latch() {
        assert_eq!(track_param(0, 1000), (0.0, false));
        assert_eq!(track_param(500, 1000), (0.5, false));
        assert_eq!(track_param(250, 1000), (0.25, false));
        assert_eq!(track_param(1000, 1000), (1.0, true));
        assert_eq!(track_param(5000, 1000), (1.0, true));
    }

    #[test]
    fn track_param_zero_duration_pins_one_without_latch() {
        assert_eq!(track_param(123, 0), (1.0, false));
    }

    #[test]
    fn track_param_negative_elapsed_pins_zero() {
        assert_eq!(track_param(-5, 1000), (0.0, false));
    }

    #[test]
    fn track_param_unsigned_compare_signed_convert() {
        // elapsed 100 is unsigned-below i32::MIN, but the int→float
        // conversion of the duration stays signed.
        let (t, done) = track_param(100, i32::MIN);
        assert!(!done);
        assert_eq!(t.to_bits(), (100.0f32 / -2_147_483_648.0f32).to_bits());
    }

    #[test]
    fn yaw_quadrants_wrap_into_zero_tau() {
        assert!(close(yaw(1.0, 0.0, ZERO, TAU), 0.0, 1e-6));
        assert!(close(yaw(0.0, 1.0, ZERO, TAU), FRAC_PI_2, 1e-6));
        assert!(close(yaw(-1.0, 0.0, ZERO, TAU), PI, 1e-5));
        // negative atan2 result wraps by +tau
        assert!(close(yaw(0.0, -1.0, ZERO, TAU), 3.0 * FRAC_PI_2, 1e-5));
        assert!(close(yaw(1.0, -1.0, ZERO, TAU), TAU - PI / 4.0, 1e-5));
    }

    #[test]
    fn yaw_zero_direction_is_zero_not_nan() {
        // Degenerate vertical tangent: both horizontal components zero.
        let y = yaw(0.0, 0.0, ZERO, TAU);
        assert_eq!(y.to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn yaw_range_property() {
        let dirs = [
            [1.0f32, 0.5],
            [-0.3, 0.8],
            [-0.9, -0.1],
            [0.2, -0.7],
            [0.0, 1.0],
            [0.0, -1.0],
        ];
        for d in dirs {
            let y = yaw(d[0], d[1], ZERO, TAU);
            assert!((0.0..TAU).contains(&y), "yaw {y} out of [0, tau)");
        }
    }

    #[test]
    fn pitch_matches_asin() {
        assert!(close(pitch(0.0), 0.0, 1e-6));
        assert!(close(pitch(0.5), FRAC_PI_6, 1e-5));
        assert!(close(pitch(-0.5), -FRAC_PI_6, 1e-5));
        assert!(close(pitch(1.0), FRAC_PI_2, 1e-5));
        assert!(close(pitch(-1.0), -FRAC_PI_2, 1e-5));
    }

    #[test]
    fn pitch_saturates_above_unit() {
        assert!(close(pitch(1.000_001), FRAC_PI_2, 1e-5));
        assert!(close(pitch(-1.000_001), -FRAC_PI_2, 1e-5));
    }

    #[test]
    fn look_param_clamps() {
        assert!(close(look_param(500, 1000, ZERO, ONE), 0.5, 1e-6));
        assert_eq!(look_param(-100, 1000, ZERO, ONE), 0.0);
        assert_eq!(look_param(2000, 1000, ZERO, ONE), 1.0);
    }

    #[test]
    fn look_param_zero_duration_saturates_high() {
        // +inf and NaN quotients both land on the 1.0 branch (the
        // source's unordered-compare polarity); -inf pins 0.
        assert_eq!(look_param(500, 0, ZERO, ONE), 1.0);
        assert_eq!(look_param(0, 0, ZERO, ONE), 1.0);
        assert_eq!(look_param(-500, 0, ZERO, ONE), 0.0);
    }

    #[test]
    fn turn_forward_aligned_target_keeps_frame_row() {
        // Target dead ahead: zero turn, rotation is identity, the
        // forward is the frame row unchanged — for any axis.
        for axis in [[0.0f32, 0.0, 1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]] {
            let fwd = turn_forward(
                axis,
                [1.0, 0.0],
                [2.0, 0.0],
                [0.0, 0.0],
                [0.25, -0.5, 0.75],
                ZERO,
                ONE,
                EPS,
                NEG_ONE,
                NEG_HALF_PI,
                HALF_PI,
            );
            assert!(close(fwd[0], 0.25, 1e-5));
            assert!(close(fwd[1], -0.5, 1e-5));
            assert!(close(fwd[2], 0.75, 1e-5));
        }
    }

    #[test]
    fn turn_forward_left_target_clamps_to_neg_half_pi() {
        // Target 90° left of facing: cross >= 0 → angle = -acos(0),
        // doubled = -pi → clamped to -pi/2 about +z. Row1 of that
        // rotation is (1, 0, 0).
        let fwd = turn_forward(
            [0.0, 0.0, 1.0],
            [1.0, 0.0],
            [0.0, 1.0],
            [0.0, 0.0],
            [0.0, 1.0, 0.0],
            ZERO,
            ONE,
            EPS,
            NEG_ONE,
            NEG_HALF_PI,
            HALF_PI,
        );
        assert!(close(fwd[0], 1.0, 1e-5), "{fwd:?}");
        assert!(close(fwd[1], 0.0, 1e-5), "{fwd:?}");
        assert!(close(fwd[2], 0.0, 1e-5), "{fwd:?}");
    }

    #[test]
    fn turn_forward_right_target_clamps_to_pos_half_pi() {
        // Mirror case: cross < 0 → angle = +acos(0), doubled = +pi →
        // clamped to +pi/2 about +z. Row1 of that rotation is (-1, 0, 0).
        let fwd = turn_forward(
            [0.0, 0.0, 1.0],
            [1.0, 0.0],
            [0.0, -1.0],
            [0.0, 0.0],
            [0.0, 1.0, 0.0],
            ZERO,
            ONE,
            EPS,
            NEG_ONE,
            NEG_HALF_PI,
            HALF_PI,
        );
        assert!(close(fwd[0], -1.0, 1e-5), "{fwd:?}");
        assert!(close(fwd[1], 0.0, 1e-5), "{fwd:?}");
        assert!(close(fwd[2], 0.0, 1e-5), "{fwd:?}");
    }

    #[test]
    fn turn_forward_opposite_target_dot_clamps_low() {
        // Anti-parallel: dot clamps to -1 (acos = pi), doubled = -2pi →
        // clamped turn; the result stays finite and unit-ish.
        let fwd = turn_forward(
            [0.0, 0.0, 1.0],
            [1.0, 0.0],
            [-3.0, 0.0],
            [0.0, 0.0],
            [0.0, 1.0, 0.0],
            ZERO,
            ONE,
            EPS,
            NEG_ONE,
            NEG_HALF_PI,
            HALF_PI,
        );
        let len = (fwd[0] * fwd[0] + fwd[1] * fwd[1] + fwd[2] * fwd[2]).sqrt();
        assert!(close(len, 1.0, 1e-5), "{fwd:?}");
    }

    #[test]
    fn turn_forward_target_scale_invariance() {
        // Metamorphic: scaling the target offset must not change the
        // result (it is rescaled to unit length first).
        let base = turn_forward(
            [0.1, 0.2, 0.97],
            [0.6, 0.8],
            [3.0, 4.0],
            [1.0, 1.5],
            [0.0, 0.0, 1.0],
            ZERO,
            ONE,
            EPS,
            NEG_ONE,
            NEG_HALF_PI,
            HALF_PI,
        );
        let scaled = turn_forward(
            [0.1, 0.2, 0.97],
            [0.6, 0.8],
            [11.0, 14.0],
            [1.0, 1.5],
            [0.0, 0.0, 1.0],
            ZERO,
            ONE,
            EPS,
            NEG_ONE,
            NEG_HALF_PI,
            HALF_PI,
        );
        for i in [0usize, 1, 2] {
            assert!(close(base[i], scaled[i], 1e-5), "{base:?} vs {scaled:?}");
        }
    }

    #[test]
    fn turn_forward_sub_epsilon_vector_passes_unscaled() {
        // A direction shorter than eps is used raw: dot ≈ 0 → acos ≈
        // pi/2 → doubled ±pi → clamp; the path stays finite.
        let fwd = turn_forward(
            [0.0, 0.0, 1.0],
            [1.0e-9, 0.0],
            [0.0, 1.0],
            [0.0, 0.0],
            [0.0, 1.0, 0.0],
            ZERO,
            ONE,
            EPS,
            NEG_ONE,
            NEG_HALF_PI,
            HALF_PI,
        );
        assert!(fwd.iter().all(|c| c.is_finite()), "{fwd:?}");
    }
}

/// `CMovement::ComputeSlopeClimbDelta` (0x635c00) — the per-move slide/step
/// collision-response helper: given a contact-plane normal `n = plane_n`, a
/// horizontal move `dir * dist_in`, and the mover's step height, compute the
/// vertical displacement `dz = -dot(n, dir) * dist / n.z` that keeps the mover
/// riding the plane. Writes `out_vec = (0*dz, 0*dz, dz)` and (when clamped)
/// rescales the translate distance, returned as `dist_out`.
///
/// Faithful port of the x87 body, including the exact `FCOMP`/`FNSTSW`/`Jcc`
/// NaN/boundary polarity of the six branches (Rust `<`/`<=`/`>` reproduce the
/// unordered arms — see each comment). `combined_n` is dereferenced
/// unconditionally by the stock code (the `|cn|²` dot runs before the
/// `useCombinedNormal` test), matching here. The `f64` intermediates track the
/// x87 80-bit stack; each memory-store point narrows through
/// [`super::f64_to_f32`].
///
/// `step_height` is the stock `FUN_00617430(this)` result, hoisted by the
/// adapter (invariant across this call — it reads only `this` state, never the
/// mutated translate distance — so the up-to-three stock calls collapse to one).
pub fn c_movement__compute_slope_climb_delta__635c00(
    dir: [f32; 3],
    dist_in: f32,
    plane_n: [f32; 3],
    use_combined: bool,
    combined_n: [f32; 3],
    step_up_mode: bool,
    step_height: f32,
) -> ([f32; 3], f32) {
    // _DAT_0080dffc ≈ 0.6427876 (cos 50°, steep-plane threshold).
    const STEEP_THRESH: f32 = f32::from_bits(0x3f24_8dbb);
    // _DAT_008029d4 = 2^-22 ≈ 2.384e-7 (shared |cn|² and |n.z| epsilon).
    const EPS: f32 = f32::from_bits(0x3480_0000);
    // Saturation sentinels: 0xff7fffff = -FLT_MAX, 0x7f7fffff = +FLT_MAX.
    const NEG_FLT_MAX: f32 = f32::from_bits(0xff7f_ffff);
    const POS_FLT_MAX: f32 = f32::from_bits(0x7f7f_ffff);

    // |combined_n|² in x87 order `(x² + y²) + z²`, narrowed to f32 (the stock
    // stores it to a f32 slot, then reloads it for the eps compare). Read
    // unconditionally, exactly like the original.
    let cx = combined_n[0] as f64;
    let cy = combined_n[1] as f64;
    let cz = combined_n[2] as f64;
    let cn_len_sq = super::f64_to_f32((cx * cx + cy * cy) + cz * cz);

    // Steep-plane combined-normal swap (only when useCombinedNormal != 0):
    //   @0x635c38 `FCOMP steep; JP` skips the swap on `planeNz > steep` OR NaN
    //     ⇒ swap iff `planeNz <= steep` (Rust `<=` is false for NaN → skip, and
    //     false for `planeNz > steep` → skip, matching the JP arm exactly).
    //   @0x635c52 `FCOMP eps; JNP` skips only on `|cnLenSq| < eps` strictly; the
    //     `== eps` and NaN cases proceed ⇒ `!(|cnLenSq| < eps)`.
    // Both jumps target the same skip label, so the swap needs both to fall
    // through (logical AND).
    let mut n = plane_n;
    if use_combined && plane_n[2] <= STEEP_THRESH && !(cn_len_sq.abs() < EPS) {
        n = [-combined_n[0], -combined_n[1], -combined_n[2]];
    }

    // fVar1 = -dot(n, dir) * dist, accumulated in the x87 term order:
    // `((-n.z*dir.z) + (-n.y*dir.y)) + (-n.x*dir.x)`, then `* dist`.
    let tz = -(n[2] as f64) * dir[2] as f64;
    let ty = -(n[1] as f64) * dir[1] as f64;
    let tx = -(n[0] as f64) * dir[0] as f64;
    let f_var1 = ((tz + ty) + tx) * dist_in as f64;

    // dz = fVar1 / n.z, narrowed to f32 (stock stores it to a f32 slot and all
    // downstream uses reload that f32). The `|n.z| < eps` saturation:
    //   @0x635cb4 `FCOMP eps; JNP` diverts to the saturate branch only on
    //     `|n.z| < eps` strictly (`== eps` and NaN divide normally).
    //   @0x635cd0 `FCOMP 0.0; JNP`: keep -FLT_MAX only on `fVar1 < 0` strictly;
    //     `fVar1 >= 0` and NaN both fall through to +FLT_MAX.
    let dz: f32 = if !((n[2] as f64).abs() < EPS as f64) {
        super::f64_to_f32(f_var1 / n[2] as f64)
    } else if f_var1 < 0.0 {
        NEG_FLT_MAX
    } else {
        POS_FLT_MAX
    };

    let mut dist_out = dist_in;
    let final_dz: f32;
    if step_up_mode && dz < 0.0 {
        // Step-up mode short-circuit: `this+0x40 & 0x4000000` set AND the
        // `FCOMP 0.0; JP` @0x635cf0 falls through only on a strictly-negative dz
        // (`dz >= 0` or NaN takes the general clamp path). Zero the translate
        // distance and return `stepHeight` as dz.
        dist_out = 0.0;
        final_dz = step_height;
    } else if step_height < dz {
        // dz > stepHeight — @0x635d12 `FCOMP stepHeight,dz; JP` is NOT taken only
        // when `stepHeight < dz`; `>=` and NaN keep dz. Clamp high and rescale
        // the translate distance by `stepHeight/dz`.
        dist_out = super::f64_to_f32((step_height as f64 / dz as f64) * dist_in as f64);
        final_dz = step_height;
    } else if -step_height > dz {
        // dz < -stepHeight — @0x635d38 `FCOMP -stepHeight,dz; JNE` is NOT taken
        // only when `-stepHeight > dz` strictly; `==` and NaN keep dz. Clamp low
        // and rescale by `-(stepHeight/dz)`.
        dist_out = super::f64_to_f32(-((step_height as f64 / dz as f64) * dist_in as f64));
        final_dz = -step_height;
    } else {
        // -stepHeight <= dz <= stepHeight (or NaN): keep dz unchanged.
        final_dz = dz;
    }

    // out_vec = (0*dz, 0*dz, dz): the x/y lanes are the x87 `0.0 * finalDz`
    // splat (exactly ±0.0 for finite dz; NaN only if dz is non-finite), not a
    // hard zero.
    let xy = 0.0_f32 * final_dz;
    ([xy, xy, final_dz], dist_out)
}

#[cfg(test)]
mod tests_c_movement__compute_slope_climb_delta__635c00 {
    use super::c_movement__compute_slope_climb_delta__635c00 as climb;

    const NEG_FLT_MAX: f32 = f32::from_bits(0xff7f_ffff);
    const POS_FLT_MAX: f32 = f32::from_bits(0x7f7f_ffff);

    // Independent f64 oracle for the un-clamped, un-swapped divide path.
    fn oracle_dz(n: [f32; 3], dir: [f32; 3], dist: f32) -> f32 {
        let dot = -(f64::from(n[2]) * f64::from(dir[2]))
            - f64::from(n[1]) * f64::from(dir[1])
            - f64::from(n[0]) * f64::from(dir[0]);
        (dot * f64::from(dist) / f64::from(n[2])) as f32
    }

    #[test]
    fn basic_divide_in_range_keeps_dz() {
        // Gentle plane, small move → dz well within ±stepHeight; no clamp, no
        // swap. out = (0, 0, dz); dist unchanged.
        let dir = [1.0, 0.0, 0.0];
        let n = [0.2, 0.0, 0.98];
        let (out, dist_out) = climb(dir, 3.0, n, false, [0.0; 3], false, 2.0);
        let dz = oracle_dz(n, dir, 3.0);
        assert!(dz.abs() <= 2.0, "precondition: dz {dz} in range");
        assert_eq!(out, [0.0 * dz, 0.0 * dz, dz]);
        assert_eq!(dist_out, 3.0);
    }

    #[test]
    fn clamp_high_rescales_distance() {
        // dz > stepHeight → dz clamps to +stepHeight; dist rescales by
        // stepHeight/dz. `dir.x < 0` vs `n.x > 0` makes dot < 0 → fVar1 > 0 →
        // dz > 0.
        let dir = [-1.0, 0.0, 0.0];
        let n = [0.9, 0.0, 0.05]; // steep-ish: large -n.x*dir.x / small n.z
        let dist = 4.0_f32;
        let sh = 2.0_f32;
        let dz_raw = oracle_dz(n, dir, dist);
        assert!(dz_raw > sh, "precondition: dz_raw {dz_raw} > {sh}");
        let (out, dist_out) = climb(dir, dist, n, false, [0.0; 3], false, sh);
        assert_eq!(out[2], sh);
        let expect_dist = ((f64::from(sh) / f64::from(dz_raw)) * f64::from(dist)) as f32;
        assert_eq!(dist_out, expect_dist);
    }

    #[test]
    fn clamp_low_rescales_and_negates() {
        // dz < -stepHeight → dz clamps to -stepHeight; dist = -(stepHeight/dz)*dist.
        // `dir.x > 0` vs `n.x > 0` makes dot > 0 → fVar1 < 0 → dz < 0.
        let dir = [1.0, 0.0, 0.0];
        let n = [0.9, 0.0, 0.05];
        let dist = 4.0_f32;
        let sh = 2.0_f32;
        let dz_raw = oracle_dz(n, dir, dist);
        assert!(dz_raw < -sh, "precondition: dz_raw {dz_raw} < {}", -sh);
        let (out, dist_out) = climb(dir, dist, n, false, [0.0; 3], false, sh);
        assert_eq!(out[2], -sh);
        let expect = (-((f64::from(sh) / f64::from(dz_raw)) * f64::from(dist))) as f32;
        assert_eq!(dist_out, expect);
    }

    #[test]
    fn near_zero_nz_saturates_by_sign() {
        // |n.z| < eps → dz saturates to ±FLT_MAX by sign(fVar1). fVar1 =
        // -dot(n,dir)*dist. With n.x>0, dir.x>0, dist>0 → dot>0 → fVar1<0 →
        // -FLT_MAX; flip dir.x → +FLT_MAX. Then the ±FLT_MAX drives the clamp:
        // dz beyond ±stepHeight so out[2] = ∓stepHeight.
        let n = [0.5, 0.0, 1e-9];
        let sh = 2.0_f32;
        // fVar1 < 0 → dz = -FLT_MAX → dz < -sh → clamp low → out[2] = -sh.
        let (out_neg, _) = climb([1.0, 0.0, 0.0], 3.0, n, false, [0.0; 3], false, sh);
        assert_eq!(out_neg[2], -sh);
        // fVar1 > 0 → dz = +FLT_MAX → dz > sh → clamp high → out[2] = +sh.
        let (out_pos, _) = climb([-1.0, 0.0, 0.0], 3.0, n, false, [0.0; 3], false, sh);
        assert_eq!(out_pos[2], sh);
        // Direct sentinel check with `stepHeight = FLT_MAX` (so the ±FLT_MAX dz
        // is not caught by the clamp): dz passes through as the raw saturated
        // value.
        let (raw_neg, _) = climb([1.0, 0.0, 0.0], 3.0, n, false, [0.0; 3], false, f32::MAX);
        assert_eq!(raw_neg[2], NEG_FLT_MAX);
        let (raw_pos, _) = climb([-1.0, 0.0, 0.0], 3.0, n, false, [0.0; 3], false, f32::MAX);
        assert_eq!(raw_pos[2], POS_FLT_MAX);
    }

    #[test]
    fn saturate_zero_fvar1_takes_positive() {
        // |n.z| < eps and fVar1 == 0 (dir ⟂ n horizontally) → +FLT_MAX (the
        // `fVar1 < 0` test is strict; 0 falls through to +FLT_MAX). Use
        // `stepHeight = FLT_MAX` so the clamp does not swallow the sentinel.
        let n = [0.0, 0.0, 1e-9];
        let (out, _) = climb([1.0, 0.0, 0.0], 3.0, n, false, [0.0; 3], false, f32::MAX);
        assert_eq!(out[2], POS_FLT_MAX);
    }

    #[test]
    fn step_up_downward_short_circuits() {
        // Step-up mode + dz < 0 → dist zeroed, dz = stepHeight (upward).
        let dir = [1.0, 0.0, 0.0]; // dot>0 → fVar1<0 → dz<0
        let n = [0.2, 0.0, 0.98];
        let sh = 2.0_f32;
        assert!(oracle_dz(n, dir, 3.0) < 0.0, "precondition dz<0");
        let (out, dist_out) = climb(dir, 3.0, n, false, [0.0; 3], true, sh);
        assert_eq!(out[2], sh);
        assert_eq!(dist_out, 0.0);
    }

    #[test]
    fn step_up_upward_takes_general_path() {
        // Step-up mode but dz >= 0 → general clamp path (not short-circuited).
        // Here dz is small positive and in range → dz kept, dist unchanged.
        let dir = [-1.0, 0.0, 0.0]; // dot<0 → fVar1>0 → dz>0
        let n = [0.2, 0.0, 0.98];
        let sh = 2.0_f32;
        let dz = oracle_dz(n, dir, 3.0);
        assert!(dz > 0.0 && dz <= sh, "precondition 0<dz<=sh: {dz}");
        let (out, dist_out) = climb(dir, 3.0, n, false, [0.0; 3], true, sh);
        assert_eq!(out[2], dz);
        assert_eq!(dist_out, 3.0);
    }

    #[test]
    fn steep_plane_swaps_to_negated_combined_normal() {
        // useCombined + steep plane (planeNz <= 0.6428) + |cn|² >= eps → n is
        // replaced by -combined_n. Verify by comparing against a direct call
        // that passes -combined_n as the plane normal with no swap.
        let dir = [0.3, 0.4, 0.1];
        let plane = [0.1, 0.1, 0.5]; // nz 0.5 <= threshold → steep
        let cn = [0.2, 0.3, 0.9];
        let sh = 100.0_f32; // large → no clamp, isolate the swap
        let (swapped, _) = climb(dir, 2.0, plane, true, cn, false, sh);
        let neg_cn = [-cn[0], -cn[1], -cn[2]];
        let (manual, _) = climb(dir, 2.0, neg_cn, false, [0.0; 3], false, sh);
        assert_eq!(swapped[2], manual[2]);
        // And it must differ from the un-swapped plane result.
        let (unswapped, _) = climb(dir, 2.0, plane, false, [0.0; 3], false, sh);
        assert_ne!(swapped[2], unswapped[2]);
    }

    #[test]
    fn no_swap_when_plane_not_steep() {
        // planeNz > threshold → no swap even with useCombined set.
        let dir = [0.3, 0.4, 0.1];
        let plane = [0.05, 0.05, 0.99]; // nz 0.99 > threshold → not steep
        let cn = [0.2, 0.3, 0.9];
        let sh = 100.0_f32;
        let (with_flag, _) = climb(dir, 2.0, plane, true, cn, false, sh);
        let (without, _) = climb(dir, 2.0, plane, false, [0.0; 3], false, sh);
        assert_eq!(with_flag[2], without[2]);
    }

    #[test]
    fn no_swap_when_combined_degenerate() {
        // Steep plane but |cn|² < eps → no swap (the eps gate blocks it).
        let dir = [0.3, 0.4, 0.1];
        let plane = [0.1, 0.1, 0.5];
        let cn = [1e-6, 0.0, 0.0]; // |cn|² = 1e-12 << eps
        let sh = 100.0_f32;
        let (with_flag, _) = climb(dir, 2.0, plane, true, cn, false, sh);
        let (without, _) = climb(dir, 2.0, plane, false, [0.0; 3], false, sh);
        assert_eq!(with_flag[2], without[2]);
    }

    #[test]
    fn nan_dz_keeps_dz_through_clamps() {
        // A NaN dz (fVar1 NaN via NaN dir) must survive both clamp compares
        // (`stepHeight < dz` and `-stepHeight > dz` are both false for NaN →
        // keep). out[2] is NaN; out.xy = 0*NaN = NaN.
        let dir = [f32::NAN, 0.0, 0.0];
        let n = [0.2, 0.0, 0.98];
        let (out, _) = climb(dir, 3.0, n, false, [0.0; 3], false, 2.0);
        assert!(out[2].is_nan());
        assert!(out[0].is_nan() && out[1].is_nan());
    }

    #[test]
    fn xy_lanes_carry_sign_of_zero_times_dz() {
        // out.xy = 0.0 * dz → +0.0 for dz>0, -0.0 for dz<0 (bit-distinct).
        let dir_up = [-1.0, 0.0, 0.0]; // dz > 0
        let n = [0.2, 0.0, 0.98];
        let (up, _) = climb(dir_up, 3.0, n, false, [0.0; 3], false, 100.0);
        assert!(up[2] > 0.0);
        assert_eq!(up[0].to_bits(), 0.0_f32.to_bits(), "+0.0");
        let dir_dn = [1.0, 0.0, 0.0]; // dz < 0
        let (dn, _) = climb(dir_dn, 3.0, n, false, [0.0; 3], false, 100.0);
        assert!(dn[2] < 0.0);
        assert_eq!(dn[0].to_bits(), (-0.0_f32).to_bits(), "-0.0");
    }
}

/// Facing gate of `CMovement__ComputeGroundNormal` 0x637140 (block
/// 0x6371a0): a contact record is PROCESSED iff its `normal.z > threshold`
/// **or the compare is unordered** — stock `FCOMP; TEST AH,0x41; JNP`
/// skips only on ordered `<=` (a NaN facing value falls through to
/// processing). Port as `!(z <= thr)`, never "normalize" to `>`.
pub fn ground_normal_process__637140(z: f32, thr: f32) -> bool {
    !(z <= thr)
}

/// Normalize tail of 0x637140 (block 0x637248), reproducing the stock
/// mixed-precision x87 structure exactly:
/// * `k = 1/hitCount` stays extended (f64 here);
/// * `x' = sumX·k` STAYS extended all the way to the final store;
/// * `y' = sumY·k` and `z' = sumZ·k` are narrowed to f32 (FSTP/FST m32),
///   but the z² term multiplies the EXTENDED z' by its NARROWED copy;
/// * `len² = (z'ₑ·z'f + y'f²) + x'ₑ²` (that FADDP order);
/// * `inv = 1/√len²` extended; `out.x = f32(inv·x'ₑ)` but `out.y/out.z`
///   multiply the NARROWED lanes by the NARROWED `inv` (plain f32 muls).
///
/// Stock has NO zero-length or zero-count guard — division by zero yields
/// inf/NaN exactly like the original; do not add one.
pub fn ground_normal_normalize__637140(sum: &[f32; 3], hit_count: u32) -> [f32; 3] {
    let k = 1.0f64 / f64::from(hit_count);
    let x_e = f64::from(sum[0]) * k;
    let y_f = super::f64_to_f32(f64::from(sum[1]) * k);
    let z_e = f64::from(sum[2]) * k;
    let z_f = super::f64_to_f32(z_e);
    let len_sq = (z_e * f64::from(z_f) + f64::from(y_f) * f64::from(y_f)) + x_e * x_e;
    let inv = 1.0f64 / len_sq.sqrt();
    let inv_f = super::f64_to_f32(inv);
    [super::f64_to_f32(inv * x_e), y_f * inv_f, z_f * inv_f]
}

#[cfg(test)]
mod tests_compute_ground_normal__637140 {
    use super::*;

    #[test]
    fn facing_polarity() {
        assert!(ground_normal_process__637140(0.9, 0.5));
        // <= skips (both strict less and equal).
        assert!(!ground_normal_process__637140(0.5, 0.5));
        assert!(!ground_normal_process__637140(0.1, 0.5));
        // NaN facing PROCESSES (unordered falls through the JNP).
        assert!(ground_normal_process__637140(f32::NAN, 0.5));
        assert!(ground_normal_process__637140(0.9, f32::NAN));
    }

    #[test]
    fn normalize_unit_vector_roundtrips() {
        let n = ground_normal_normalize__637140(&[0.0, 0.0, 3.0], 3);
        assert!((n[2] - 1.0).abs() < 1e-6 && n[0] == 0.0 && n[1] == 0.0);
    }

    #[test]
    fn normalize_matches_f64_reference() {
        // Dense sweep vs a straight f64 normalize; the mixed-precision
        // structure only diverges at ulp scale.
        let cases: [([f32; 3], u32); 4] = [
            ([1.0, 2.0, 2.0], 1),
            ([3.0, -4.0, 12.0], 5),
            ([0.1, 0.2, 0.97], 2),
            ([-7.5, 0.5, 3.25], 7),
        ];
        for (s, c) in cases {
            let got = ground_normal_normalize__637140(&s, c);
            let len = (f64::from(s[0]).powi(2) + f64::from(s[1]).powi(2) + f64::from(s[2]).powi(2))
                .sqrt();
            for lane in 0..3 {
                let want = f64::from(s[lane]) / len;
                assert!(
                    (f64::from(got[lane]) - want).abs() <= 1e-6 * (1.0 + want.abs()),
                    "sum={s:?} count={c} lane={lane} got={} want={want}",
                    got[lane]
                );
            }
        }
    }

    #[test]
    fn normalize_zero_sum_is_unguarded() {
        // Stock divides by sqrt(0) with no guard => non-finite lanes.
        let n = ground_normal_normalize__637140(&[0.0, 0.0, 0.0], 2);
        assert!(!n[0].is_finite() || n[0].is_nan());
        assert!(n[1].is_nan() || !n[1].is_finite());
    }
}

/// Facing interpolation wrap (0x7c4afd..0x7c4b2a): `facing + base` folded
/// wide, then wrapped into `[low, 2π)` — subtract `2π` when the sum is
/// STRICTLY greater (ordered), add `2π` when it is below `low` OR unordered
/// (NaN), else pass through. One narrow at the return.
pub fn interpolate_facing__7c4ae0(facing: f32, base: f32, two_pi: f32, low: f32) -> f32 {
    let sum = f64::from(facing) + f64::from(base);
    let tp = f64::from(two_pi);
    let wrapped = if sum > tp {
        sum - tp
    } else if !(sum >= f64::from(low)) {
        sum + tp
    } else {
        sum
    };
    super::f64_to_f32(wrapped)
}

#[cfg(test)]
mod tests_interpolate_facing__7c4ae0 {
    use super::interpolate_facing__7c4ae0 as facing;
    const TP: f32 = 6.2831855;

    #[test]
    fn wraps_high_and_low() {
        // 5 + 3 = 8 > 2pi -> 8 - 2pi.
        assert_eq!(
            facing(5.0, 3.0, TP, 0.0),
            super::super::f64_to_f32(8.0 - f64::from(TP))
        );
        // -1 + 0 = -1 < 0 -> -1 + 2pi.
        assert_eq!(
            facing(-1.0, 0.0, TP, 0.0),
            super::super::f64_to_f32(-1.0 + f64::from(TP))
        );
        // 1 + 1 = 2 in range -> unchanged.
        assert_eq!(facing(1.0, 1.0, TP, 0.0), 2.0);
    }

    #[test]
    fn nan_takes_the_add_branch() {
        assert!(facing(f32::NAN, 0.0, TP, 0.0).is_nan());
    }
}

/// Path-node position lerp (0x619217..0x619264): `new[i] = (pos[i] −
/// node[i]) × frac + node[i]` where `frac = delta / period` (integer divide,
/// 80-bit). The three lanes narrow differently — X pre-narrows the product,
/// Y folds fully wide, Z pre-narrows both the delta and the product.
pub fn path_node_lerp__6191c0(pos: [f32; 3], node: [f32; 3], delta: i32, period: i32) -> [f32; 3] {
    let frac = f64::from(delta) / f64::from(period);
    let px = super::f64_to_f32((f64::from(pos[0]) - f64::from(node[0])) * frac);
    let x = super::f64_to_f32(f64::from(px) + f64::from(node[0]));
    let y = super::f64_to_f32((f64::from(pos[1]) - f64::from(node[1])) * frac + f64::from(node[1]));
    let dz = super::f64_to_f32(f64::from(pos[2]) - f64::from(node[2]));
    let pz = super::f64_to_f32(f64::from(dz) * frac);
    let z = super::f64_to_f32(f64::from(pz) + f64::from(node[2]));
    [x, y, z]
}

#[cfg(test)]
mod tests_path_node_lerp__6191c0 {
    use super::path_node_lerp__6191c0 as lerp;

    #[test]
    fn lerps_toward_node() {
        // frac = 1/2; new = (pos - node)/2 + node = midpoint.
        let p = lerp([10.0, 4.0, 6.0], [0.0, 0.0, 0.0], 1, 2);
        assert_eq!(p, [5.0, 2.0, 3.0]);
    }

    #[test]
    fn frac_zero_returns_node() {
        assert_eq!(
            lerp([9.0, 9.0, 9.0], [1.0, 2.0, 3.0], 0, 5),
            [1.0, 2.0, 3.0]
        );
    }
}
