//! `particle` family kernels.
#![allow(
    non_snake_case,
    // intentional NaN-reject via `!(a >= b)`
    // (differs from `a < b` on NaN), bit-exact source constants kept verbatim,
    // and ABI-dictated parameter counts.
    clippy::neg_cmp_op_on_partial_ord,
    clippy::excessive_precision,
    clippy::too_many_arguments
)]

/// `CParticleEmitter::SetAlpha` — fixed-point quantize of a float alpha to an 8-bit value.
///
/// `trunc(alpha * 255.0 + 0.5)`, returned as the i32 the original `__ftol`
/// (truncate-toward-zero) produces. The caller stores its low byte.
///
/// The original is `FLD alpha; FMUL 255.0; FADD 0.5; CALL __ftol`; adding 0.5
/// before a truncating cast yields round-half-up for the non-negative alphas
/// this is fed. The byte store (`MOV [esi+0x12f], AL`) keeps only the low 8
/// bits, so out-of-range inputs wrap exactly as the original does. The 255.0
/// scale lives at host global 0x7ffe58, the 0.5 bias at 0x7ffa24.
pub fn c_particle_emitter__set_alpha__7b7b10(alpha: f32) -> i32 {
    (alpha * 255.0 + 0.5) as i32
}

#[cfg(test)]
mod tests_c_particle_emitter__set_alpha__7b7b10 {
    use super::c_particle_emitter__set_alpha__7b7b10 as f;

    // Low byte, mirroring the original's `MOV byte ptr [...], AL`.
    fn byte(alpha: f32) -> u8 {
        (f(alpha) & 0xff) as u8
    }

    #[test]
    fn known_values() {
        assert_eq!(byte(0.0), 0);
        assert_eq!(byte(1.0), 255);
        assert_eq!(byte(0.5), 128); // trunc(127.5 + 0.5) = 128
    }

    #[test]
    fn endpoints_full_range() {
        // 0.0 -> 0, 1.0 -> 255 (alpha is a 0..1 fraction).
        assert_eq!(byte(0.0), 0);
        assert_eq!(byte(1.0), 255);
    }

    #[test]
    fn round_half_up() {
        // x*255 lands on .5 boundaries; the +0.5 bias rounds up under trunc.
        // 0.25*255 = 63.75 -> trunc(64.25) = 64
        assert_eq!(byte(0.25), 64);
        // 0.75*255 = 191.25 -> trunc(191.75) = 191
        assert_eq!(byte(0.75), 191);
    }

    #[test]
    fn monotonic_nondecreasing() {
        let mut prev = byte(0.0);
        let mut a = 0.0f32;
        while a <= 1.0 {
            let cur = byte(a);
            assert!(cur >= prev, "a={a} cur={cur} prev={prev}");
            prev = cur;
            a += 1.0 / 255.0;
        }
    }

    #[test]
    fn matches_reference_formula() {
        for a in [0.0f32, 0.1, 0.2, 0.333, 0.5, 0.6, 0.9, 1.0] {
            let expect = (a * 255.0 + 0.5) as i32;
            assert_eq!(f(a), expect, "a={a}");
        }
    }
}

/// `CParticleEmitter::SetColor` — packs the emitter's 32-bit color.
///
/// Packs three float color channels and the already-quantized alpha byte (set
/// earlier by `SetAlpha`) into the emitter's 32-bit color. Each channel is
/// fixed-point quantized `trunc(c * 255.0 + 0.5)` and masked to a byte; the
/// alpha byte is requantized `trunc(a * 255.0 * 255.0 + 0.5)` (a fixed-point
/// squaring, low byte kept) before being placed in the high byte.
///
/// Layout matches the original register packing
/// `(alpha << 24) | (c0 << 16) | (c1 << 8) | c2`, with `cvttss2si` truncation
/// and that byte placement. The 255.0 scale lives at host global 0x7ffe58 and
/// the 0.5 round bias at 0x7ffa24.
pub fn c_particle_emitter__set_color__7b7a80(alpha_byte: u8, c0: f32, c1: f32, c2: f32) -> u32 {
    // alpha: requantize the stored byte through *255*255, keep low 8 bits.
    let a = ((f32::from(alpha_byte) * 255.0 * 255.0 + 0.5) as i32 & 0xff) as u32;
    let r0 = ((c0 * 255.0 + 0.5) as i32 & 0xff) as u32;
    let r1 = ((c1 * 255.0 + 0.5) as i32 & 0xff) as u32;
    let r2 = ((c2 * 255.0 + 0.5) as i32 & 0xff) as u32;
    (a << 24) | (r0 << 16) | (r1 << 8) | r2
}

#[cfg(test)]
mod tests_c_particle_emitter__set_color__7b7a80 {
    use super::c_particle_emitter__set_color__7b7a80 as f;

    fn qbyte(c: f32) -> u32 {
        ((c * 255.0 + 0.5) as i32 & 0xff) as u32
    }

    #[test]
    fn byte_placement() {
        // c0 -> bits 16..23, c1 -> 8..15, c2 -> 0..7, alpha -> 24..31.
        let v = f(0, 1.0, 0.0, 0.0);
        assert_eq!((v >> 16) & 0xff, 255); // c0 channel
        assert_eq!((v >> 8) & 0xff, 0);
        assert_eq!(v & 0xff, 0);
        assert_eq!((v >> 24) & 0xff, 0);

        let v = f(0, 0.0, 1.0, 0.0);
        assert_eq!((v >> 8) & 0xff, 255); // c1 channel

        let v = f(0, 0.0, 0.0, 1.0);
        assert_eq!(v & 0xff, 255); // c2 channel
    }

    #[test]
    fn alpha_byte_requantize_low_byte() {
        // alpha_byte=0 -> trunc(0.5)=0; alpha_byte=1 -> trunc(65025.5)=65025,
        // low byte 65025 & 0xff = 1.
        assert_eq!((f(0, 0.0, 0.0, 0.0) >> 24) & 0xff, 0);
        assert_eq!((f(1, 0.0, 0.0, 0.0) >> 24) & 0xff, (65025 & 0xff) as u32);
        // alpha_byte=255 -> 255*255*255 = 16581375, +0.5 trunc, & 0xff.
        let expect = ((255.0f32 * 255.0 * 255.0 + 0.5) as i32 & 0xff) as u32;
        assert_eq!((f(255, 0.0, 0.0, 0.0) >> 24) & 0xff, expect);
    }

    #[test]
    fn known_value_full_white_zero_alpha() {
        // c0=c1=c2=1.0 -> each 255; alpha byte 0.
        assert_eq!(f(0, 1.0, 1.0, 1.0), 0x00_FF_FF_FF);
    }

    #[test]
    fn matches_reference_formula() {
        for (ab, c0, c1, c2) in [
            (0u8, 0.0f32, 0.0f32, 0.0f32),
            (1, 0.25, 0.5, 0.75),
            (128, 1.0, 0.0, 0.5),
            (255, 0.1, 0.9, 0.333),
        ] {
            let a = ((f32::from(ab) * 255.0 * 255.0 + 0.5) as i32 & 0xff) as u32;
            let expect = (a << 24) | (qbyte(c0) << 16) | (qbyte(c1) << 8) | qbyte(c2);
            assert_eq!(f(ab, c0, c1, c2), expect, "ab={ab}");
        }
    }

    #[test]
    fn channels_independent() {
        // Setting one channel does not disturb the others' bytes.
        let base = f(0, 0.0, 0.0, 0.0);
        assert_eq!(base, 0);
        let only_c0 = f(0, 0.5, 0.0, 0.0);
        assert_eq!((only_c0 >> 8) & 0xff, 0);
        assert_eq!(only_c0 & 0xff, 0);
    }
}

/// `CParticleEmitter::SetGravity` — clamps the incoming gravity to be non-negative.
///
/// Returns `gravity` when `gravity > 0.0`, else `0.0`.
///
/// Matches the original's `FLD gravity; FCOMP 0.0; if 0.0 < gravity keep else
/// store 0`, which is equivalently a branchless `cmpless`/`andps` clamp
/// against the same zero constant. A NaN input
/// compares false and is therefore flushed to `0.0`, exactly as the x87
/// `FCOMP`/`TEST AH,0x41` path does.
pub fn c_particle_emitter__set_gravity__7b4bf0(gravity: f32) -> f32 {
    if gravity > 0.0 { gravity } else { 0.0 }
}

#[cfg(test)]
mod tests_c_particle_emitter__set_gravity__7b4bf0 {
    use super::c_particle_emitter__set_gravity__7b4bf0 as f;

    #[test]
    fn known_values() {
        assert_eq!(f(9.81).to_bits(), 9.81f32.to_bits());
        assert_eq!(f(0.0).to_bits(), 0.0f32.to_bits());
        assert_eq!(f(-5.0).to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn negatives_and_zero_flush_to_zero() {
        for g in [-1.0e30f32, -1.0, -0.0, 0.0] {
            assert_eq!(f(g).to_bits(), 0.0f32.to_bits(), "g={g}");
        }
    }

    #[test]
    fn positives_pass_through_unchanged() {
        for g in [1.0e-30f32, 0.5, 1.0, 9.81, 1.0e30] {
            assert_eq!(f(g).to_bits(), g.to_bits(), "g={g}");
        }
    }

    #[test]
    fn nan_flushes_to_zero() {
        // `NaN > 0.0` is false, so the else-branch stores 0.0.
        assert_eq!(f(f32::NAN).to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn idempotent() {
        for g in [-3.0f32, 0.0, 2.0, 100.0] {
            assert_eq!(f(f(g)).to_bits(), f(g).to_bits(), "g={g}");
        }
    }

    #[test]
    fn output_never_negative() {
        for g in [-1.0e9f32, -0.1, 0.0, 0.1, 1.0e9] {
            assert!(f(g) >= 0.0, "g={g}");
        }
    }
}

/// Per-particle physics integration for one timestep.
///
/// All emitter parameters and the particle's mutable state are injected by value
/// (the original reads them from `this` and the particle record). Returns the
/// updated `(pos, vel, first_frame_flag, alive)`. Snap-to-zero uses a deadband
/// of magnitude `vel_eps` around exactly zero; the position term carries the
/// `½·g·dt²` kinematic correction (`half` is the source's literal `0.5`). The
/// drag coefficient field is read as an integer "enabled" flag by the original
/// (`drag_enabled`), separate from the float coefficient `drag_coeff`. The
/// alive/cull test multiplies the pre-drag velocity (scaled by `dt`) against the
/// updated position; a strictly-positive dot retires the particle.
pub fn c_particle_emitter__update_particle_physics__7b2680(
    pos: [f32; 3],
    vel: [f32; 3],
    first_frame: bool,
    age: f32,
    dt: f32,
    lifespan: f32,
    accel: [f32; 3],
    spawn_vel_bonus: [f32; 3],
    grav_z: f32,
    drag_coeff: f32,
    drag_enabled: bool,
    flags: u32,
    half: f32,
    drag_clamp: f32,
    vel_eps: f32,
) -> ([f32; 3], [f32; 3], bool, bool) {
    let mut p = pos;
    let mut v = vel;
    let mut ff = first_frame;

    // Acceleration while the particle is younger than its lifespan
    // (`age < lifespan`, NaN excluded), then snap each component to zero inside
    // the deadband.
    if age < lifespan {
        v = [
            dt * accel[0] + v[0],
            dt * accel[1] + v[1],
            dt * accel[2] + v[2],
        ];
        v = snap_deadband(v, vel_eps);
    }

    // Spawn-velocity bonus on the first frame the bit is observed (flag 0x40000);
    // the first observation consumes the per-particle first-frame bit instead of
    // applying the bonus.
    if flags & 0x4_0000 != 0 {
        if ff {
            ff = false;
        } else {
            p = [
                p[0] + spawn_vel_bonus[0],
                p[1] + spawn_vel_bonus[1],
                p[2] + spawn_vel_bonus[2],
            ];
        }
    }

    // Position integration. The pre-drag velocity (`v` here) is captured for the
    // cull dot below.
    let vdt = [dt * v[0], dt * v[1], dt * v[2]];
    p[0] += vdt[0];
    p[1] += vdt[1];
    // z carries the ½·g·dt² correction: pos.z += dt·vz − ½·g·dt².
    p[2] = (p[2] - dt * grav_z * dt * half) + vdt[2];
    // velocity z loses g·dt.
    v[2] -= dt * grav_z;

    // Drag (only when the integer "enabled" field is non-zero): drag factor is
    // `dt·coeff` clamped above by `drag_clamp` (1.0), applied as `v -= drag·v`.
    if drag_enabled {
        let mut drag = dt * drag_coeff;
        if drag_clamp < drag {
            drag = drag_clamp;
        }
        v = [v[0] - drag * v[0], v[1] - drag * v[1], v[2] - drag * v[2]];
    }

    v = snap_deadband(v, vel_eps);

    // Cull test (flag 0x800): the dot of the pre-drag, dt-scaled velocity with
    // the updated position; strictly positive => the particle moves away and
    // dies. NaN keeps it alive (matches the original's `JNZ`).
    let mut alive = true;
    if flags & 0x800 != 0 {
        let mut dot = vdt[2] * p[2];
        dot += vdt[1] * p[1];
        dot += vdt[0] * p[0];
        if dot > 0.0 {
            alive = false;
        }
    }

    (p, v, ff, alive)
}

/// Snap each component to exactly `0.0` when it is non-zero and its magnitude is below `eps`.
///
/// (A deadband around zero.) A component already equal to `0.0` is left
/// untouched, matching the original's `value != 0.0` guard.
fn snap_deadband(v: [f32; 3], eps: f32) -> [f32; 3] {
    v.map(|c| if c != 0.0 && c.abs() < eps { 0.0 } else { c })
}

#[cfg(test)]
mod tests_c_particle_emitter__update_particle_physics__7b2680 {
    use super::{c_particle_emitter__update_particle_physics__7b2680 as step, snap_deadband};

    fn approx(a: [f32; 3], b: [f32; 3]) {
        for (x, y) in a.iter().zip(b.iter()) {
            assert!((x - y).abs() < 1e-4, "{a:?} != {b:?}");
        }
    }

    #[test]
    fn freefall_integrates_position_and_gravity() {
        // dt=1, vel=[1,2,3], grav=10, no accel/drag/spawn/cull.
        let (p, v, ff, alive) = step(
            [0.0, 0.0, 0.0],
            [1.0, 2.0, 3.0],
            false,
            0.0,
            1.0,
            10.0,
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            10.0,
            0.0,
            false,
            0,
            0.5,
            1.0,
            1e-6,
        );
        // z carries -½·g·dt²: [1, 2, 3 - 5] = [1, 2, -2].
        approx(p, [1.0, 2.0, -2.0]);
        // v.z loses g·dt: 3 - 10 = -7.
        approx(v, [1.0, 2.0, -7.0]);
        assert!(!ff && alive);
    }

    #[test]
    fn deadband_snaps_subthreshold_velocity() {
        let (_, v, _, _) = step(
            [0.0, 0.0, 0.0],
            [5e-4, 5.0, 0.0],
            false,
            0.0,
            0.0,
            10.0,
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            0.0,
            0.0,
            false,
            0,
            0.5,
            1.0,
            1e-3,
        );
        assert_eq!(v[0].to_bits(), 0.0f32.to_bits());
        assert_eq!(v[1].to_bits(), 5.0f32.to_bits());
    }

    #[test]
    fn first_frame_consumes_bit_without_bonus() {
        let (p, _, ff, _) = step(
            [1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            true,
            0.0,
            0.0,
            10.0,
            [0.0, 0.0, 0.0],
            [100.0, 100.0, 100.0],
            0.0,
            0.0,
            false,
            0x4_0000,
            0.5,
            1.0,
            1e-6,
        );
        assert!(!ff);
        approx(p, [1.0, 1.0, 1.0]); // bonus not applied on the consuming frame
    }

    #[test]
    fn cull_retires_when_moving_away() {
        let (_, _, _, alive) = step(
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            false,
            0.0,
            1.0,
            10.0,
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            0.0,
            0.0,
            false,
            0x800,
            0.5,
            1.0,
            1e-6,
        );
        assert!(!alive);
    }

    #[test]
    fn deadband_helper_leaves_zero_and_large() {
        assert_eq!(snap_deadband([0.0, 2.0, 1e-9], 1e-3), [0.0, 2.0, 0.0]);
    }
}

/// Per-particle lifetime-fraction interpolation.
///
/// Produces a packed BGRA vertex color, the particle size, and a pair of
/// `u`/`v` atlas-cell indices.
///
/// `frac` is the normalized lifetime parameter `((age - t0) * inv_span) * cm0 + cb`.
/// Each output channel is `slope * frac + base (+ bias)` accumulated in 14.18 fixed
/// point, then shifted right by 14 and (for color/uv) masked to a byte. The alpha
/// channel additionally scales by `scale` before the bias. When `prec_sel != 1.0`
/// the `u`/`v` interpolation parameter is replaced by `pow(frac * prec_sel, pow_k)`
/// evaluated in extended precision; otherwise the linear `frac` is used directly.
#[allow(clippy::too_many_arguments)]
pub fn c_particle_emitter__compute_vertex_color_uv__7b9b10(
    age: f32,
    scale: f32,
    t0: f32,
    inv_span: f32,
    color_mul: f32,
    color_add: f32,
    bias: f32,
    pow_k: f64,
    slope_b: i32,
    base_b: u8,
    slope_g: i32,
    base_g: u8,
    slope_r: i32,
    base_r: u8,
    slope_a: i32,
    base_a: u8,
    size_base: f32,
    size_slope: f32,
    u_base: i32,
    u_slope: i32,
    v_base: i32,
    v_slope: i32,
    prec_sel: f32,
) -> ([u8; 4], f32, u32, u32) {
    let frac = (age - t0) * inv_span * color_mul + color_add;

    // Each channel packs its integer result into an `f32` mantissa via the magic
    // `bias` (512.0), then extracts it by reinterpreting the float's BIT PATTERN
    // and shifting right 14 — the original's `FADD bias; FSTP [f32]; MOV; SHR
    // 0xe` idiom, NOT a numeric float→int conversion (which would shift the small
    // `value + 512` below 2^14 and yield 0). `to_bits` replicates it exactly.
    let a = ((slope_a as f32 * frac + base_a as f32) * scale + bias).to_bits() >> 14;
    let r = (slope_r as f32 * frac + base_r as f32 + bias).to_bits() >> 14;
    let g = (slope_g as f32 * frac + base_g as f32 + bias).to_bits() >> 14;
    let b = (slope_b as f32 * frac + base_b as f32 + bias).to_bits() >> 14;

    let color = [b as u8, g as u8, r as u8, a as u8];

    let size = frac * size_slope + size_base;

    let (u, v) = if prec_sel.to_bits() == 0x3f80_0000 {
        let u = ((u_slope as f32 * frac + u_base as f32 + bias).to_bits() >> 14) & 0xff;
        let v = ((v_slope as f32 * frac + v_base as f32 + bias).to_bits() >> 14) & 0xff;
        (u, v)
    } else {
        // Higher-precision path: the exponent `pow_k` reshapes `frac`; the
        // original computes it on the x87 stack and still stores f32 before the
        // bit extract, so narrow to f32 first.
        let p = ((frac * prec_sel) as f64).powf(pow_k);
        let u =
            (((u_slope as f64 * p + u_base as f64 + bias as f64) as f32).to_bits() >> 14) & 0xff;
        let v =
            (((v_slope as f64 * p + v_base as f64 + bias as f64) as f32).to_bits() >> 14) & 0xff;
        (u, v)
    };

    (color, size, u, v)
}

#[cfg(test)]
mod tests_c_particle_emitter__compute_vertex_color_uv__7b9b10 {
    use super::c_particle_emitter__compute_vertex_color_uv__7b9b10 as f;

    // The host's magic bias constant `_DAT_008029cc` = 512.0. Adding it places a
    // channel's integer result in the f32 mantissa at the 2^14 scale, and the
    // original extracts that integer by reading the float's BIT PATTERN and
    // shifting right 14 — so for an integer payload `c < 512` the output byte is
    // simply `c & 0xff`. The earlier tests used `bias = 0` with slopes pre-scaled
    // by 2^14, which masked this by happening to agree with a numeric cast.
    const BIAS: f32 = 512.0;

    // Drive the kernel with a chosen lifetime `frac` by setting `age - t0 = frac`
    // and the linear remap to identity (`inv_span = 1`, `color_mul = 1`,
    // `color_add = 0`). All non-exercised channels are zeroed.
    #[allow(clippy::too_many_arguments)]
    fn with_frac(
        frac: f32,
        scale: f32,
        pow_k: f64,
        slope_b: i32,
        base_b: u8,
        slope_g: i32,
        base_g: u8,
        slope_r: i32,
        base_r: u8,
        slope_a: i32,
        base_a: u8,
        size_base: f32,
        size_slope: f32,
        u_base: i32,
        u_slope: i32,
        v_base: i32,
        v_slope: i32,
        prec_sel: f32,
    ) -> ([u8; 4], f32, u32, u32) {
        f(
            frac, scale, 0.0, 1.0, 1.0, 0.0, BIAS, pow_k, slope_b, base_b, slope_g, base_g,
            slope_r, base_r, slope_a, base_a, size_base, size_slope, u_base, u_slope, v_base,
            v_slope, prec_sel,
        )
    }

    /// Reference extractor: the bit-pattern shift the original performs.
    ///
    /// For an integer channel value `c` this returns `c` (low byte) — the closed
    /// form the assertions below check against.
    fn extract_byte(value: f32) -> u32 {
        ((value + BIAS).to_bits() >> 14) & 0xff
    }

    #[test]
    fn known_value_from_bases() {
        // frac = 1 with zero slopes: each channel value IS its base. The bit
        // trick maps an integer base `c` straight to `c` (verified vs the closed
        // form). bases chosen distinct: b=10, g=20, r=30, a=40.
        let (color, size, u, v) = with_frac(
            1.0, 1.0, 1.0, 0, 10, 0, 20, 0, 30, 0, 40, 5.0, 0.0, 5, 0, 6, 0, 1.0,
        );
        assert_eq!(color, [10, 20, 30, 40]);
        assert_eq!(u32::from(color[0]), extract_byte(10.0));
        assert_eq!(u32::from(color[3]), extract_byte(40.0));
        assert_eq!(size.to_bits(), 5.0f32.to_bits());
        assert_eq!(u, 5);
        assert_eq!(v, 6);
    }

    #[test]
    fn slope_advances_channel_with_frac() {
        // A non-zero slope makes the channel `slope*frac + base`. slope_b=7,
        // base_b=3, frac=2 => value 17 => byte 17. u_slope=4, u_base=1, frac=2 =>
        // 9 => byte 9.
        let (color, _, u, _) = with_frac(
            2.0, 1.0, 1.0, 7, 3, 0, 0, 0, 0, 0, 0, 0.0, 0.0, 1, 4, 0, 0, 1.0,
        );
        assert_eq!(u32::from(color[0]), 17);
        assert_eq!(u32::from(color[0]), extract_byte(7.0 * 2.0 + 3.0));
        assert_eq!(u, 9);
    }

    #[test]
    fn alpha_scales_before_bias() {
        // Alpha alone is `(slope_a*frac + base_a) * scale + bias`. With base_a=20,
        // slope_a=0: alpha = 20*scale, still under 256 for scale in 1..=8.
        for scale_i in 1u32..=8 {
            let scale = scale_i as f32;
            let (color, ..) = with_frac(
                1.0, scale, 1.0, 0, 0, 0, 0, 0, 0, 0, 20, 0.0, 0.0, 0, 0, 0, 0, 1.0,
            );
            assert_eq!(u32::from(color[3]), 20 * scale_i, "scale={scale}");
            assert_eq!(
                u32::from(color[3]),
                extract_byte(20.0 * scale),
                "scale={scale}"
            );
        }
    }

    #[test]
    fn pow_path_replaces_frac() {
        // prec_sel != 1.0 selects the extended-precision branch. With the host's
        // pow_k = 1.0, p = (frac*prec_sel)^1 = frac*prec_sel. frac=1, prec_sel=4
        // => p=4; u = u_slope*4 + u_base = 10*4 = 40; v = 20*4 = 80.
        let (_, _, u, v) = with_frac(
            1.0, 1.0, 1.0, 0, 0, 0, 0, 0, 0, 0, 0, 0.0, 0.0, 0, 10, 0, 20, 4.0,
        );
        assert_eq!(u, 40);
        assert_eq!(v, 80);
    }

    #[test]
    fn uv_masked_to_byte() {
        // The u/v results are `& 0xff`, so they never exceed 255 regardless of
        // slope magnitude.
        for slope in [0i32, 50, 300, 5000] {
            let (_, _, u, v) = with_frac(
                1.0, 1.0, 1.0, 0, 0, 0, 0, 0, 0, 0, 0, 0.0, 0.0, 0, slope, 0, slope, 1.0,
            );
            assert!(u <= 255, "u={u} slope={slope}");
            assert!(v <= 255, "v={v} slope={slope}");
        }
    }

    #[test]
    fn size_is_affine_in_frac() {
        // size = frac * size_slope + size_base is a plain affine float (no bias
        // trick); verify the closed form for a spread of fractions.
        for frac in [0.0f32, 0.25, 0.5, 1.0, 2.5] {
            let (_, size, _, _) = with_frac(
                frac, 1.0, 1.0, 0, 0, 0, 0, 0, 0, 0, 0, 1.5, 3.0, 0, 0, 0, 0, 1.0,
            );
            let expect = frac * 3.0 + 1.5;
            assert_eq!(size.to_bits(), expect.to_bits(), "frac={frac}");
        }
    }
}

/// `CParticleEmitter::BuildParticleQuad` fade/LOD slot index.
///
/// `fade = clamp(rate_num * age, 0.0, 255.0)`, then the LOD-array slot is
/// `((bits(fade + bias) >> 14) + (addr >> 5)) & 0x7f`. `bias` is the host
/// magic constant `512.0` (`_DAT_008029cc`): the original computes
/// `FADD bias; FSTP [f32]; MOV; SHR 14` — a bit-pattern read of `fade + bias`,
/// NOT a numeric float→int cast (the same magic-bias trap that, mis-coded as an
/// `as u32` cast, once blanked particles). `addr` is the particle record's host
/// address mixed in to decorrelate adjacent particles' LOD phase.
///
/// The clamp arms are ordered (`0.0 <= v`, then `255.0 <= v`), so a NaN product
/// collapses to `0.0` exactly as the original's `FCOM; FNSTSW; TEST AH; Jcc`.
pub fn c_particle_emitter__build_particle_quad_fade_index(
    rate_num: f32,
    age: f32,
    particle_addr: u32,
    bias: f32,
) -> u32 {
    let v = rate_num * age;
    let mut fade = 0.0f32;
    if 0.0 <= v {
        fade = v;
        if 255.0 <= v {
            fade = 255.0;
        }
    }
    let biased = (fade + bias).to_bits() >> 14;
    biased.wrapping_add(particle_addr >> 5) & 0x7f
}

#[cfg(test)]
mod tests_c_particle_emitter__build_particle_quad_fade_index {
    use super::c_particle_emitter__build_particle_quad_fade_index as f;

    // The host magic-bias constant `_DAT_008029cc`.
    const BIAS: f32 = 512.0;

    /// Closed form of the bit-pattern extract the original performs.
    ///
    /// Mirrored here so the assertions check the kernel against the trick, not
    /// itself.
    fn slot(fade: f32, addr: u32) -> u32 {
        (((fade + BIAS).to_bits() >> 14).wrapping_add(addr >> 5)) & 0x7f
    }

    #[test]
    fn matches_bit_trick_across_fades() {
        // rate_num * age sweeps below/within/above the [0, 255] clamp; addr held
        // fixed so the comparison isolates the fade-derived term.
        let addr = 0x0123_4560;
        for &age in &[0.0f32, 0.5, 1.0, 10.0, 300.0, 1.0e9] {
            let got = f(1.0, age, addr, BIAS);
            let want = slot(age.clamp(0.0, 255.0), addr);
            assert_eq!(got, want, "age={age}");
        }
    }

    #[test]
    fn negative_and_nan_clamp_to_zero_fade() {
        // `0.0 <= v` is false for negatives and NaN, so both pin fade to 0.0.
        let addr = 0xdead_be00;
        for &v in &[-1.0f32, -1.0e30, f32::NAN] {
            assert_eq!(f(1.0, v, addr, BIAS), slot(0.0, addr));
        }
    }

    #[test]
    fn address_term_mixes_in() {
        // The `addr >> 5` term shifts the slot; a one-step change at bit 5 of the
        // address moves the slot by one (mod 0x80).
        let a = f(1.0, 1.0, 0x0000_0000, BIAS);
        let b = f(1.0, 1.0, 0x0000_0020, BIAS);
        assert_eq!(b, (a + 1) & 0x7f);
    }

    #[test]
    fn result_is_seven_bits() {
        for addr in [0u32, 1, 0x20, 0xffff_ffe0, 0x8000_0000] {
            for &age in &[0.0f32, 1.0, 128.0, 255.0] {
                assert!(f(1.0, age, addr, BIAS) < 0x80);
            }
        }
    }
}

/// Mantissa of a 32-bit RNG word reinterpreted into `[0, 1)`.
///
/// `f32::from_bits(r & 0x7fffff | 0x3f800000)` lands in `[1, 2)`; subtracting
/// one normalises it to `[0, 1)`, matching the original's
/// `AND 0x7fffff; OR 0x3f800000; FLD; FSUB 1.0`.
fn rng_unit(r: u32, one: f32) -> f32 {
    f32::from_bits((r & 0x007f_ffff) | 0x3f80_0000) - one
}

/// `CParticleEmitter::SpawnParticle` initialiser math.
///
/// Builds a freshly spawned particle record from three RNG draws and the
/// emitter's spawn parameters, returning the nine 32-bit slots
/// `[life_remaining, life, pos.x, pos.y, pos.z, vel.x, vel.y, vel.z, p8]`.
///
/// `life` is a random fraction of `dt` clamped to zero once it reaches
/// `lifespan`; a random direction is built from two `(2·rand − 1)·spread` angles
/// fed through sin/cos at `dir_radius`. The original's folded first integration
/// step integrates the velocity into the position over `life` with a
/// half-`accel_z·t²` z correction and bleeds `accel_z·life` off the z velocity;
/// the spawn position is then offset by the emitter origin. `p8` is the raw
/// 4-byte field copied verbatim from `emitter + 0x20`. `one`/`half`/`zero` are
/// injected host constants (1.0, 0.5, 0.0). Slots are returned as bit patterns
/// so the adapter stores them without an extra round-trip.
pub fn c_particle_emitter__spawn_particle__7ba200(
    r1: u32,
    r2: u32,
    r3: u32,
    dt: f32,
    lifespan: f32,
    dir_radius: f32,
    accel_z: f32,
    spread_a: f32,
    spread_b: f32,
    emitter_pos: [f32; 3],
    p8_bits: u32,
    one: f32,
    half: f32,
    zero: f32,
) -> [u32; 9] {
    // Lifetime: a random fraction of dt, clamped to zero once it reaches lifespan.
    let mut life = rng_unit(r1, one) * dt;
    if lifespan <= life {
        life = zero;
    }
    let life_remaining = lifespan - life;

    // Position starts at the emitter local origin; offset by emitter_pos at the end.
    let mut pos = [zero, zero, zero];

    // Two symmetric random angles: (2·rand − 1)·spread.
    let ua = rng_unit(r2, one) * spread_a;
    let angle_a = (ua + ua) - spread_a;
    let ub = rng_unit(r3, one) * spread_b;
    let angle_b = (ub + ub) - spread_b;

    // Direction on a sphere of radius `dir_radius`.
    let (sa, ca) = crate::math::trig::sin_cos(angle_a);
    let vx0 = sa * dir_radius;
    let vz = ca * dir_radius;
    let (sb, cb) = crate::math::trig::sin_cos(angle_b);
    let vy = sb * vx0;
    let vx = cb * vx0;

    let mut vel = [vx - pos[0], vy - pos[1], vz - pos[2]];

    // Folded first integration step (the original `FUN_007ba380`): integrate the
    // velocity into the position over `life`, with the half-g·t² z correction, and
    // bleed `accel_z·life` off the z velocity. The global x/y gravity terms are
    // zero in stock data, kept explicit for exactness.
    let t = life;
    pos[0] += t * vel[0] + zero;
    pos[1] += t * vel[1] + zero;
    pos[2] += t * vel[2] - half * accel_z * t * t;
    vel[0] += zero;
    vel[1] += zero;
    vel[2] += -accel_z * t;

    // Offset the spawn position by the emitter world origin.
    pos[0] += emitter_pos[0];
    pos[1] += emitter_pos[1];
    pos[2] += emitter_pos[2];

    [
        life_remaining.to_bits(),
        life.to_bits(),
        pos[0].to_bits(),
        pos[1].to_bits(),
        pos[2].to_bits(),
        vel[0].to_bits(),
        vel[1].to_bits(),
        vel[2].to_bits(),
        p8_bits,
    ]
}

#[cfg(test)]
mod tests_c_particle_emitter__spawn_particle__7ba200 {
    use super::{c_particle_emitter__spawn_particle__7ba200 as spawn, rng_unit};

    const ONE: f32 = 1.0;
    const HALF: f32 = 0.5;
    const ZERO: f32 = 0.0;

    fn f(slot: u32) -> f32 {
        f32::from_bits(slot)
    }

    #[test]
    fn rng_unit_range() {
        assert_eq!(rng_unit(0, ONE).to_bits(), 0.0f32.to_bits());
        assert!(rng_unit(0xffff_ffff, ONE) < 1.0);
        assert!(rng_unit(0xffff_ffff, ONE) >= 0.0);
        // Top mantissa bit -> 1.5 in [1,2) -> 0.5 in [0,1).
        assert_eq!(rng_unit(0x0040_0000, ONE).to_bits(), 0.5f32.to_bits());
    }

    #[test]
    fn life_clamped_to_zero_at_lifespan() {
        // r1 mantissa-all-ones -> ~1.0 fraction; dt large enough that life >=
        // lifespan -> life clamps to 0, life_remaining == lifespan.
        let out = spawn(
            0x7fff_ffff,
            0,
            0,
            10.0,
            2.0,
            1.0,
            0.0,
            0.0,
            0.0,
            [0.0, 0.0, 0.0],
            0xdead_beef,
            ONE,
            HALF,
            ZERO,
        );
        assert_eq!(out[1], 0.0f32.to_bits()); // life
        assert_eq!(f(out[0]), 2.0); // life_remaining == lifespan
        assert_eq!(out[8], 0xdead_beef); // p8 copied verbatim
    }

    #[test]
    fn position_offset_by_emitter_origin_when_no_motion() {
        // life clamps to 0 (no integration), zero radius -> velocity 0; the only
        // change to position is the emitter-origin offset.
        let out = spawn(
            0x7fff_ffff,
            0,
            0,
            10.0,
            1.0,
            0.0,
            0.0,
            0.0,
            0.0,
            [3.0, -4.0, 5.0],
            0,
            ONE,
            HALF,
            ZERO,
        );
        assert_eq!(f(out[2]).to_bits(), 3.0f32.to_bits());
        assert_eq!(f(out[3]).to_bits(), (-4.0f32).to_bits());
        assert_eq!(f(out[4]).to_bits(), 5.0f32.to_bits());
        // Zero radius -> zero velocity.
        assert_eq!(f(out[5]), 0.0);
        assert_eq!(f(out[6]), 0.0);
        assert_eq!(f(out[7]), 0.0);
    }

    #[test]
    fn direction_magnitude_matches_radius_at_zero_spread() {
        // Zero spread -> both angles 0 -> sin=0, cos=1: vx0=0, vz=radius, vy=0,
        // vx=0. r=0 -> rng_unit=0 -> life=0 -> the t=0 integration case.
        let out = spawn(
            0,
            0,
            0,
            1.0e-6,
            1.0,
            7.0,
            0.0,
            0.0,
            0.0,
            [0.0, 0.0, 0.0],
            0,
            ONE,
            HALF,
            ZERO,
        );
        assert_eq!(f(out[1]).to_bits(), 0.0f32.to_bits());
        // velocity = (0, 0, radius)
        assert_eq!(f(out[5]).to_bits(), 0.0f32.to_bits());
        assert_eq!(f(out[6]).to_bits(), 0.0f32.to_bits());
        assert_eq!(f(out[7]).to_bits(), 7.0f32.to_bits());
    }

    #[test]
    fn gravity_pulls_z_down_over_life() {
        // Non-zero life and accel_z: z velocity loses accel_z*life; position z
        // gains t*vz - 0.5*accel_z*t^2. Radius 0 so initial vel=0.
        // rng_unit(0x0040_0000)=0.5 -> life = 0.5*dt.
        let dt = 0.4f32;
        let accel = 8.0f32;
        let out = spawn(
            0x0040_0000,
            0,
            0,
            dt,
            100.0,
            0.0,
            accel,
            0.0,
            0.0,
            [0.0, 0.0, 0.0],
            0,
            ONE,
            HALF,
            ZERO,
        );
        let life = 0.5 * dt;
        assert_eq!(f(out[1]).to_bits(), life.to_bits());
        assert_eq!(f(out[7]).to_bits(), (-accel * life).to_bits());
        let expected_z = -0.5 * accel * life * life;
        assert_eq!(f(out[4]).to_bits(), expected_z.to_bits());
    }
}

/// Copies a caller 4x4 matrix and accumulates a 3-component spawn offset into its translation row.
///
/// (Column-major elements 12, 13, 14.) Returns the combined 16-float matrix.
/// This is the pure prelude of the emitter spawn-transform update; the optional
/// bone-relative multiply and the double-buffered struct writes are handled by
/// the adapter.
pub fn c_particle_system__update_spawn_transform__7b76c0(
    matrix: &[f32; 16],
    position: &[f32; 3],
) -> [f32; 16] {
    let mut m = *matrix;
    m[12] += position[0];
    m[13] += position[1];
    m[14] += position[2];
    m
}

/// Extracts the three spawn-basis vec3s the emitter caches from a (possibly bone-transformed) 4x4.
///
/// The SECOND row `(m[4..7])`, row2 `(m[8..11])`, and the translation
/// `(m[12..15])`. Returned as `(row0, row2, translation)` to mirror the struct
/// layout (`this+0x7c`, `this+0x94`, `this+0x20`). Stock caches
/// `m[4],m[5],m[6]` into the `this+0x7c` slot, not the first row.
pub fn c_particle_system__spawn_basis__7b76c0(m: &[f32; 16]) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let row0 = [m[4], m[5], m[6]];
    let row2 = [m[8], m[9], m[10]];
    let translation = [m[12], m[13], m[14]];
    (row0, row2, translation)
}

#[cfg(test)]
mod tests_c_particle_system__update_spawn_transform__7b76c0 {
    use super::{
        c_particle_system__spawn_basis__7b76c0, c_particle_system__update_spawn_transform__7b76c0,
    };

    /// A 4x4 in column-major order with distinct entries.
    ///
    /// So a basis extractor cannot accidentally pass by reading the wrong slot.
    fn sample_matrix() -> [f32; 16] {
        [
            1.0, 2.0, 3.0, 0.0, // row0 candidates m[0..3]
            4.0, 5.0, 6.0, 0.0, // m[4..7]
            7.0, 8.0, 9.0, 0.0, // row2 candidates m[8..11]
            10.0, 11.0, 12.0, 1.0, // translation m[12..15]
        ]
    }

    /// Law: only the translation slots (12, 13, 14) change.
    ///
    /// Every other element is carried through untouched, and each translation
    /// slot is the source plus the matching position component.
    #[test]
    fn only_translation_accumulates() {
        let src = sample_matrix();
        let pos = [0.5, -1.5, 100.0];
        let out = c_particle_system__update_spawn_transform__7b76c0(&src, &pos);

        for (i, (&o, &s)) in out.iter().zip(src.iter()).enumerate() {
            if (12..15).contains(&i) {
                let expected = s + pos[i - 12];
                assert_eq!(o.to_bits(), expected.to_bits());
            } else {
                assert_eq!(o.to_bits(), s.to_bits());
            }
        }
    }

    /// Metamorphic law: accumulating the offset in two halves equals accumulating it once.
    ///
    /// Applying `(a + b)` in one shot must match applying `a` then `b`.
    #[test]
    fn offset_accumulation_is_additive() {
        let src = sample_matrix();
        let a = [1.0, 2.0, 3.0];
        let b = [10.0, 20.0, 30.0];

        let once = c_particle_system__update_spawn_transform__7b76c0(
            &src,
            &[a[0] + b[0], a[1] + b[1], a[2] + b[2]],
        );

        let step1 = c_particle_system__update_spawn_transform__7b76c0(&src, &a);
        let step2 = c_particle_system__update_spawn_transform__7b76c0(&step1, &b);

        for (&l, &r) in once.iter().zip(step2.iter()) {
            assert_eq!(l.to_bits(), r.to_bits());
        }
    }

    /// Law: a zero offset is the identity transform on the matrix.
    #[test]
    fn zero_offset_is_identity() {
        let src = sample_matrix();
        let out = c_particle_system__update_spawn_transform__7b76c0(&src, &[0.0, 0.0, 0.0]);
        for (&o, &s) in out.iter().zip(src.iter()) {
            assert_eq!(o.to_bits(), s.to_bits());
        }
    }

    /// Known value.
    ///
    /// With the sample matrix and offset `(0.5, -1.5, 100.0)` the translation
    /// row becomes `(10.5, 9.5, 112.0)` while the other elements hold.
    #[test]
    fn known_value() {
        let src = sample_matrix();
        let out = c_particle_system__update_spawn_transform__7b76c0(&src, &[0.5, -1.5, 100.0]);
        assert_eq!(out[12].to_bits(), 10.5f32.to_bits());
        assert_eq!(out[13].to_bits(), 9.5f32.to_bits());
        assert_eq!(out[14].to_bits(), 112.0f32.to_bits());
        assert_eq!(out[0].to_bits(), 1.0f32.to_bits());
        assert_eq!(out[15].to_bits(), 1.0f32.to_bits());
    }

    /// Law: the basis extractor pulls row0 from `m[0..3]`, row2 from `m[8..11]`.
    ///
    /// The translation comes from `m[12..15]` of the accumulated matrix —
    /// exactly the slots the adapter caches into the emitter struct.
    #[test]
    fn basis_pulls_expected_slots() {
        let src = sample_matrix();
        let pos = [0.5, -1.5, 100.0];
        let m = c_particle_system__update_spawn_transform__7b76c0(&src, &pos);
        let (row0, row2, translation) = c_particle_system__spawn_basis__7b76c0(&m);

        assert_eq!(row0[0].to_bits(), m[4].to_bits());
        assert_eq!(row0[1].to_bits(), m[5].to_bits());
        assert_eq!(row0[2].to_bits(), m[6].to_bits());

        assert_eq!(row2[0].to_bits(), m[8].to_bits());
        assert_eq!(row2[1].to_bits(), m[9].to_bits());
        assert_eq!(row2[2].to_bits(), m[10].to_bits());

        assert_eq!(translation[0].to_bits(), 10.5f32.to_bits());
        assert_eq!(translation[1].to_bits(), 9.5f32.to_bits());
        assert_eq!(translation[2].to_bits(), 112.0f32.to_bits());
    }
}

/// Floats per trail vertex-ring slot.
///
/// (`0x28` bytes: two edge vertices, each a position plus texture u/v.)
pub const TRAIL_VERT_FLOATS: usize = 10;

/// Trail-emitter state for `c_particle_emitter__advance_trail__7b7e60`.
///
/// The segment-ring indices plus every object field the advance reads, injected
/// by value. `head`, `tail`, `spawn_fraction` and `flags` are updated in place.
pub struct TrailAdvance {
    /// Ring capacity in slots (object `+0x8`).
    pub capacity: u32,
    /// Oldest live slot index (object `+0x18`).
    pub head: u32,
    /// Spawn slot index (object `+0x14`).
    pub tail: u32,
    /// Fractional spawn accumulator carried between calls (object `+0x1c`).
    pub spawn_fraction: f32,
    /// Emitter flag word (object `+0x148`).
    ///
    /// Bits 0 and 2 gate spawning; the advance sets bit 3 and clears bit 4 on
    /// exit.
    pub flags: u32,
    /// Maximum trail arc span (object `+0xf8`).
    ///
    /// Clamps the advance and retires slots whose age would exceed it.
    pub max_arc: f32,
    /// Segments spawned per unit of advance (object `+0xf4`).
    pub spawn_per_unit: f32,
    /// Parabolic z-sag rate per unit squared age (object `+0x164`).
    pub sag_rate: f32,
    /// Texture-u factor applied to the age first (object `+0x5c`).
    pub u_scale_a: f32,
    /// Texture-u factor applied second (object `+0x58`).
    pub u_scale_b: f32,
    /// Texture-u offset (object `+0x70`).
    pub u_origin: f32,
    /// Fixed texture-v row of the lower ribbon edge (object `+0x6c`).
    pub v_lo: f32,
    /// Fixed texture-v row of the upper ribbon edge (object `+0x74`).
    pub v_hi: f32,
    /// Current emitter sample position (object `+0x20`).
    pub pos: [f32; 3],
    /// Previous emitter sample position (object `+0x14c`).
    pub prev: [f32; 3],
    /// Ribbon edge direction at the current sample (object `+0x7c`).
    pub dir_cur: [f32; 3],
    /// Ribbon edge direction at the previous sample (object `+0x88`).
    pub dir_prev: [f32; 3],
    /// Unscaled drift vector for the current end (object `+0x94`).
    ///
    /// The frame scales it by the inter-sample distance.
    pub drift_cur_raw: [f32; 3],
    /// Unscaled drift vector for the previous end (object `+0xa0`).
    pub drift_prev_raw: [f32; 3],
    /// Edge extent above the sample point (object `+0x15c`).
    pub extent_hi: f32,
    /// Edge extent below the sample point (object `+0x160`).
    pub extent_lo: f32,
}

/// Spawn interpolation frame derived from the current / previous emitter samples.
///
/// (Object `+0xac..+0xf0`.) Refreshed once per spawning advance.
pub struct TrailFrame {
    /// Drift offset blended toward the current end (`+0xac`).
    pub drift_cur: [f32; 3],
    /// Drift offset blended toward the previous end (`+0xb8`).
    pub drift_prev: [f32; 3],
    /// Lower-edge point at the current sample (`+0xc4`).
    pub edge_lo_cur: [f32; 3],
    /// Lower-edge point at the previous sample (`+0xd0`).
    pub edge_lo_prev: [f32; 3],
    /// Upper-edge point at the current sample (`+0xdc`).
    pub edge_hi_cur: [f32; 3],
    /// Upper-edge point at the previous sample (`+0xe8`).
    pub edge_hi_prev: [f32; 3],
}

fn v3_scale(v: [f32; 3], k: f32) -> [f32; 3] {
    [v[0] * k, v[1] * k, v[2] * k]
}

fn v3_add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn v3_sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// One ring-index step.
///
/// `index + step`, minus `capacity` once the sum reaches it (the trail ring's
/// modular advance).
fn ring_step(index: u32, step: u32, capacity: u32) -> u32 {
    let next = index.wrapping_add(step);
    if next >= capacity {
        next.wrapping_sub(capacity)
    } else {
        next
    }
}

/// Rebuilds the spawn frame from the current / previous samples.
///
/// Edge points pulled in by `extent_lo` / pushed out by `extent_hi` along each
/// sample's edge direction, and the drift vectors scaled by the inter-sample
/// distance (times the injected `width_ref`, stock 1.0).
fn trail_frame(st: &TrailAdvance, width_ref: f32) -> TrailFrame {
    let delta = v3_sub(st.pos, st.prev);
    let span = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt() * width_ref;
    TrailFrame {
        drift_cur: v3_scale(st.drift_cur_raw, span),
        drift_prev: v3_scale(st.drift_prev_raw, span),
        edge_lo_cur: v3_sub(st.pos, v3_scale(st.dir_cur, st.extent_lo)),
        edge_lo_prev: v3_sub(st.prev, v3_scale(st.dir_prev, st.extent_lo)),
        edge_hi_cur: v3_add(v3_scale(st.dir_cur, st.extent_hi), st.pos),
        edge_hi_prev: v3_add(v3_scale(st.dir_prev, st.extent_hi), st.prev),
    }
}

/// Writes one ribbon slot at `tail`.
///
/// Both edge vertices interpolate between the frame's previous (`t = 1`) and
/// current (`t = 0`) ends with the drift terms blended in, the slot age is
/// seeded, and the tail advances by `step` slots (`step = 0` leaves the slot
/// uncommitted at the tail).
fn spawn_slot(
    st: &mut TrailAdvance,
    ages: &mut [f32],
    verts: &mut [f32],
    frame: &TrailFrame,
    age: f32,
    t: f32,
    step: u32,
    seg_ref: f32,
) {
    let s = seg_ref - t;
    let pulled = v3_scale(frame.drift_prev, s);
    let lo = v3_add(
        v3_scale(v3_sub(frame.edge_lo_prev, pulled), t),
        v3_scale(v3_add(v3_scale(frame.drift_cur, t), frame.edge_lo_cur), s),
    );
    let hi = v3_add(
        v3_scale(v3_add(v3_scale(frame.drift_cur, t), frame.edge_hi_cur), s),
        v3_scale(v3_sub(frame.edge_hi_prev, pulled), t),
    );
    let slot = &mut verts[st.tail as usize * TRAIL_VERT_FLOATS..][..TRAIL_VERT_FLOATS];
    slot[0] = lo[0];
    slot[1] = lo[1];
    slot[2] = lo[2];
    slot[5] = hi[0];
    slot[6] = hi[1];
    slot[7] = hi[2];
    ages[st.tail as usize] = age;
    st.tail = ring_step(st.tail, step, st.capacity);
}

/// Advances a trail-ribbon emitter by `advance` arc units.
///
/// Clamps the advance into `[0, max_arc]` (a strictly negative advance flushes
/// to zero; NaN or an overlong advance saturates at `max_arc`), retires ring
/// slots whose age would exceed the span, spawns interpolated segments when
/// flag bits 0 and 2 are set and `spawn_gate` is bit-pattern `+0.0` (the
/// original tests the raw argument word), ages every live slot — adding the
/// parabolic z sag `(2·age + d)·d·sag_rate` to both edge vertices and
/// refreshing texture u/v — then sets flag bit 3 and clears bit 4. `ages` holds
/// one float per ring slot and `verts` `TRAIL_VERT_FLOATS` floats per slot.
/// `seg_ref` / `min_advance` / `width_ref` are injected host constants (stock
/// 1.0 / 0.0 / 1.0). Returns the refreshed spawn frame when a spawn pass ran so
/// the adapter can write it back to the object.
pub fn c_particle_emitter__advance_trail__7b7e60(
    st: &mut TrailAdvance,
    ages: &mut [f32],
    verts: &mut [f32],
    advance: f32,
    spawn_gate: f32,
    seg_ref: f32,
    min_advance: f32,
    width_ref: f32,
) -> Option<TrailFrame> {
    let dist = if advance < min_advance {
        0.0
    } else if advance < st.max_arc {
        advance
    } else {
        st.max_arc
    };

    // Retire slots from the head whose advanced age leaves the arc span
    // (ordered strict `>`; a NaN sum exits, matching the original's C0|C3
    // status test).
    while st.head != st.tail && dist + ages[st.head as usize] > st.max_arc {
        st.head = ring_step(st.head, 1, st.capacity);
    }

    let mut frame_out = None;
    if spawn_gate.to_bits() == 0 && st.flags & 4 != 0 && st.flags & 1 != 0 {
        let emit_total = dist * st.spawn_per_unit + st.spawn_fraction;
        let frame = trail_frame(st, width_ref);
        if emit_total >= seg_ref {
            let inv_span = seg_ref / (emit_total - st.spawn_fraction);
            // The original floors `emit_total - seg_ref` in double precision,
            // truncates to 32 bits and adds one for the spawn count.
            let whole = f64::from(emit_total - seg_ref).floor();
            let count = (crate::math::misc::ftol__40a2b0(whole) as u32).wrapping_add(1);
            let mut marker = 1.0_f32;
            for _ in 0..count {
                let t = (marker - st.spawn_fraction) * inv_span;
                spawn_slot(st, ages, verts, &frame, -(t * dist), t, 1, seg_ref);
                marker += seg_ref;
            }
        }
        let total_wide = f64::from(emit_total);
        st.spawn_fraction = (total_wide - total_wide.floor()) as f32;
        // Uncommitted live-end slot at the tail (step 0); its texture
        // coordinates are pinned to the u origin and the two v rows.
        spawn_slot(st, ages, verts, &frame, 0.0, 1.0, 0, seg_ref);
        let slot = &mut verts[st.tail as usize * TRAIL_VERT_FLOATS..][..TRAIL_VERT_FLOATS];
        slot[3] = st.u_origin;
        slot[4] = st.v_lo;
        slot[8] = st.u_origin;
        slot[9] = st.v_hi;
        frame_out = Some(frame);
    }

    // Age every live slot: parabolic z sag on both edge vertices, age bump,
    // refreshed texture u/v.
    let mut cursor = st.head;
    while cursor != st.tail {
        let i = cursor as usize;
        let sag = (ages[i] + ages[i] + dist) * dist * st.sag_rate;
        let slot = &mut verts[i * TRAIL_VERT_FLOATS..][..TRAIL_VERT_FLOATS];
        slot[2] += sag;
        slot[7] += sag;
        ages[i] += dist;
        let u = ages[i] * st.u_scale_a * st.u_scale_b + st.u_origin;
        slot[3] = u;
        slot[4] = st.v_lo;
        slot[8] = u;
        slot[9] = st.v_hi;
        cursor = ring_step(cursor, 1, st.capacity);
    }

    st.flags = (st.flags & !0x10) | 8;
    frame_out
}

#[cfg(test)]
mod tests_c_particle_emitter__advance_trail__7b7e60 {
    use super::{
        TRAIL_VERT_FLOATS as N, TrailAdvance, c_particle_emitter__advance_trail__7b7e60 as f,
    };

    fn state(capacity: u32) -> TrailAdvance {
        TrailAdvance {
            capacity,
            head: 0,
            tail: 0,
            spawn_fraction: 0.0,
            flags: 0,
            max_arc: 10.0,
            spawn_per_unit: 0.0,
            sag_rate: 0.0,
            u_scale_a: 1.0,
            u_scale_b: 1.0,
            u_origin: 0.0,
            v_lo: 0.25,
            v_hi: 0.75,
            pos: [0.0; 3],
            prev: [0.0; 3],
            dir_cur: [0.0; 3],
            dir_prev: [0.0; 3],
            drift_cur_raw: [0.0; 3],
            drift_prev_raw: [0.0; 3],
            extent_hi: 0.0,
            extent_lo: 0.0,
        }
    }

    /// Geometry used by the frame tests: |pos − prev| = 5 on distinct axes.
    fn geometry(st: &mut TrailAdvance) {
        st.pos = [1.0, 2.0, 3.0];
        st.prev = [4.0, 6.0, 3.0];
        st.dir_cur = [0.0, 0.0, 1.0];
        st.dir_prev = [0.0, 1.0, 0.0];
        st.drift_cur_raw = [0.1, 0.2, 0.3];
        st.drift_prev_raw = [0.2, 0.0, 0.0];
        st.extent_lo = 2.0;
        st.extent_hi = 3.0;
    }

    #[test]
    fn flags_set_bit3_clear_bit4() {
        let mut st = state(4);
        st.flags = 0x10 | 0x2;
        let mut ages = [0.0f32; 4];
        let mut verts = [0.0f32; 4 * N];
        let frame = f(&mut st, &mut ages, &mut verts, 1.0, 0.0, 1.0, 0.0, 1.0);
        assert!(frame.is_none(), "spawn needs flag bits 0 and 2");
        assert_eq!(st.flags, 0x2 | 0x8);
    }

    #[test]
    fn negative_advance_flushes_to_zero() {
        let mut st = state(4);
        st.tail = 2;
        let mut ages = [3.0f32, 4.0, 0.0, 0.0];
        let mut verts = [0.0f32; 4 * N];
        f(&mut st, &mut ages, &mut verts, -5.0, 1.0, 1.0, 0.0, 1.0);
        assert_eq!(st.head, 0, "nothing retires at zero distance");
        assert_eq!(ages[0].to_bits(), 3.0f32.to_bits());
        assert_eq!(ages[1].to_bits(), 4.0f32.to_bits());
    }

    #[test]
    fn nan_advance_saturates_to_max_arc() {
        let mut st = state(4);
        st.max_arc = 2.5;
        st.tail = 1;
        let mut ages = [1.0f32, 0.0, 0.0, 0.0];
        let mut verts = [0.0f32; 4 * N];
        f(&mut st, &mut ages, &mut verts, f32::NAN, 1.0, 1.0, 0.0, 1.0);
        // dist == max_arc (2.5): the lone slot retires (2.5 + 1.0 > 2.5); a
        // zero or NaN dist would have left head at 0.
        assert_eq!(st.head, 1);
        assert_eq!(
            ages[0].to_bits(),
            1.0f32.to_bits(),
            "retired slots keep their age"
        );
    }

    #[test]
    fn retires_expired_head_slots_with_wrap() {
        let mut st = state(4);
        st.head = 3;
        st.tail = 2;
        st.max_arc = 5.0;
        let mut ages = [4.9f32, 1.0, 0.0, 4.5];
        let mut verts = [0.0f32; 4 * N];
        f(&mut st, &mut ages, &mut verts, 1.0, 1.0, 1.0, 0.0, 1.0);
        // 1 + 4.5 > 5 retires slot 3 (head wraps to 0), 1 + 4.9 > 5 retires
        // slot 0, 1 + 1 <= 5 keeps slot 1.
        assert_eq!(st.head, 1);
        assert_eq!(ages[1].to_bits(), 2.0f32.to_bits(), "survivor aged by dist");
        assert_eq!(
            ages[3].to_bits(),
            4.5f32.to_bits(),
            "retired slot untouched"
        );
    }

    #[test]
    fn spawn_frame_and_uncommitted_slot_geometry() {
        let mut st = state(8);
        st.flags = 5;
        st.spawn_fraction = 0.9;
        geometry(&mut st);
        let mut ages = [0.0f32; 8];
        let mut verts = [0.0f32; 8 * N];
        let frame =
            f(&mut st, &mut ages, &mut verts, 0.0, 0.0, 1.0, 0.0, 1.0).expect("spawn pass runs");
        // |pos − prev| = 5: drift = raw·5; edges pulled by extent_lo = 2 /
        // pushed by extent_hi = 3 along each sample's direction.
        assert_eq!(frame.drift_cur, [0.5, 1.0, 1.5]);
        assert_eq!(frame.drift_prev, [1.0, 0.0, 0.0]);
        assert_eq!(frame.edge_lo_cur, [1.0, 2.0, 1.0]);
        assert_eq!(frame.edge_lo_prev, [4.0, 4.0, 3.0]);
        assert_eq!(frame.edge_hi_cur, [1.0, 2.0, 6.0]);
        assert_eq!(frame.edge_hi_prev, [4.0, 9.0, 3.0]);
        // The uncommitted tail slot (t = 1) sits exactly on the previous edge
        // points, and its texture coordinates are pinned.
        assert_eq!(&verts[0..3], &frame.edge_lo_prev);
        assert_eq!(&verts[5..8], &frame.edge_hi_prev);
        assert_eq!(verts[3].to_bits(), st.u_origin.to_bits());
        assert_eq!(verts[4].to_bits(), st.v_lo.to_bits());
        assert_eq!(verts[8].to_bits(), st.u_origin.to_bits());
        assert_eq!(verts[9].to_bits(), st.v_hi.to_bits());
        assert_eq!(st.tail, 0, "step 0 leaves the slot uncommitted");
        // emit_total = 0.9 < 1: no committed spawns; fraction survives intact.
        assert_eq!(st.spawn_fraction.to_bits(), 0.9f32.to_bits());
    }

    #[test]
    fn midpoint_spawn_with_zero_drift() {
        let mut st = state(8);
        st.flags = 5;
        geometry(&mut st);
        st.drift_cur_raw = [0.0; 3];
        st.drift_prev_raw = [0.0; 3];
        st.spawn_per_unit = 2.0;
        let mut ages = [0.0f32; 8];
        let mut verts = [0.0f32; 8 * N];
        // dist 1 → emit_total 2.0 → two committed spawns at t = 0.5 and t = 1.
        f(&mut st, &mut ages, &mut verts, 1.0, 0.0, 1.0, 0.0, 1.0).expect("spawn pass runs");
        assert_eq!(st.tail, 2);
        // Slot 0 (t = 0.5, drift-free) is the exact midpoint of the edge pairs:
        // lo: (C + D)/2 = ([1,2,1] + [4,4,3])/2; hi: (E + F)/2.
        assert_eq!(&verts[0..3], &[2.5, 3.0, 2.0]);
        assert_eq!(&verts[5..8], &[2.5, 5.5, 4.5]);
        // Slot 1 (t = 1) lands on the previous edge points.
        assert_eq!(&verts[N..N + 3], &[4.0, 4.0, 3.0]);
        assert_eq!(&verts[N + 5..N + 8], &[4.0, 9.0, 3.0]);
    }

    #[test]
    fn spawn_pass_counts_ages_and_drift_blend() {
        let mut st = state(8);
        st.flags = 5;
        st.spawn_per_unit = 2.5;
        geometry(&mut st);
        let mut ages = [0.0f32; 8];
        let mut verts = [0.0f32; 8 * N];
        // dist 1 → emit_total 2.5 → count = floor(1.5) + 1 = 2 committed
        // spawns at t = 0.4 and t = 0.8, plus the uncommitted tail slot.
        let frame =
            f(&mut st, &mut ages, &mut verts, 1.0, 0.0, 1.0, 0.0, 1.0).expect("spawn pass runs");
        assert_eq!(st.tail, 2);
        assert_eq!(st.head, 0);
        assert_eq!(st.spawn_fraction.to_bits(), 0.5f32.to_bits());
        // Committed slot ages seed at −(t·dist), then the aging pass adds dist.
        let inv = 1.0f32 / 2.5;
        let t1 = 1.0f32 * inv;
        let t2 = 2.0f32 * inv;
        assert_eq!(ages[0].to_bits(), (-(t1 * 1.0) + 1.0).to_bits());
        assert_eq!(ages[1].to_bits(), (-(t2 * 1.0) + 1.0).to_bits());
        assert_eq!(
            ages[2].to_bits(),
            0.0f32.to_bits(),
            "uncommitted slot is not aged"
        );
        // Slot 0 lower edge mirrors the source op order:
        // (lo_prev − s·drift_prev)·t + (t·drift_cur + lo_cur)·s.
        let s1 = 1.0f32 - t1;
        for k in [0usize, 1, 2] {
            let want = (frame.edge_lo_prev[k] - frame.drift_prev[k] * s1) * t1
                + (frame.drift_cur[k] * t1 + frame.edge_lo_cur[k]) * s1;
            assert_eq!(verts[k].to_bits(), want.to_bits(), "lo[{k}]");
            let want_hi = (frame.drift_cur[k] * t1 + frame.edge_hi_cur[k]) * s1
                + (frame.edge_hi_prev[k] - frame.drift_prev[k] * s1) * t1;
            assert_eq!(verts[5 + k].to_bits(), want_hi.to_bits(), "hi[{k}]");
        }
    }

    #[test]
    fn spawn_gate_is_a_bit_pattern_test() {
        let mut st = state(4);
        st.flags = 5;
        st.spawn_per_unit = 2.5;
        let mut ages = [0.0f32; 4];
        let mut verts = [0.0f32; 4 * N];
        // -0.0 has a non-zero bit pattern: no spawn pass.
        let frame = f(&mut st, &mut ages, &mut verts, 1.0, -0.0, 1.0, 0.0, 1.0);
        assert!(frame.is_none());
        assert_eq!(st.tail, 0);
        // Flag bit 0 missing: no spawn pass either.
        let mut st = state(4);
        st.flags = 4;
        st.spawn_per_unit = 2.5;
        let frame = f(&mut st, &mut ages, &mut verts, 1.0, 0.0, 1.0, 0.0, 1.0);
        assert!(frame.is_none());
        assert_eq!(st.tail, 0);
    }

    #[test]
    fn spawn_wraps_the_ring() {
        let mut st = state(4);
        st.flags = 5;
        st.head = 2;
        st.tail = 2;
        st.spawn_per_unit = 3.5;
        let mut ages = [9.0f32; 4];
        let mut verts = [0.0f32; 4 * N];
        // dist 1 → emit_total 3.5 → three committed spawns: tail 2→3→0→1.
        f(&mut st, &mut ages, &mut verts, 1.0, 0.0, 1.0, 0.0, 1.0).expect("spawn pass runs");
        assert_eq!(st.tail, 1);
        assert_eq!(st.head, 2);
        assert_eq!(st.spawn_fraction.to_bits(), 0.5f32.to_bits());
        // The uncommitted slot at the wrapped tail was re-seeded to age 0 and
        // is excluded from aging.
        assert_eq!(ages[1].to_bits(), 0.0f32.to_bits());
        // Committed slots were seeded −(t·dist) and then aged by dist.
        let inv = 1.0f32 / 3.5;
        for (slot, marker) in [(2usize, 1.0f32), (3, 2.0), (0, 3.0)] {
            let t = marker * inv;
            assert_eq!(
                ages[slot].to_bits(),
                (-(t * 1.0) + 1.0).to_bits(),
                "slot {slot}"
            );
        }
    }

    #[test]
    fn aging_applies_parabolic_sag_and_u_refresh() {
        let mut st = state(4);
        st.head = 1;
        st.tail = 3;
        st.sag_rate = 0.5;
        st.u_scale_a = 2.0;
        st.u_scale_b = 4.0;
        st.u_origin = 10.0;
        st.max_arc = 100.0;
        let mut ages = [0.0f32, 3.0, 5.0, 0.0];
        let mut verts = [0.0f32; 4 * N];
        verts[N + 2] = 7.0;
        verts[N + 7] = 9.0;
        f(&mut st, &mut ages, &mut verts, 2.0, 1.0, 1.0, 0.0, 1.0);
        // Slot 1: sag = (3+3+2)·2·0.5 = 8 on both edge z; age 3+2 = 5;
        // u = 5·2·4 + 10 = 50.
        assert_eq!(verts[N + 2].to_bits(), 15.0f32.to_bits());
        assert_eq!(verts[N + 7].to_bits(), 17.0f32.to_bits());
        assert_eq!(ages[1].to_bits(), 5.0f32.to_bits());
        assert_eq!(verts[N + 3].to_bits(), 50.0f32.to_bits());
        assert_eq!(verts[N + 4].to_bits(), st.v_lo.to_bits());
        assert_eq!(verts[N + 8].to_bits(), 50.0f32.to_bits());
        assert_eq!(verts[N + 9].to_bits(), st.v_hi.to_bits());
        // Slot 2: sag = (5+5+2)·2·0.5 = 12; age 7; u = 7·8 + 10 = 66.
        assert_eq!(verts[2 * N + 2].to_bits(), 12.0f32.to_bits());
        assert_eq!(ages[2].to_bits(), 7.0f32.to_bits());
        assert_eq!(verts[2 * N + 3].to_bits(), 66.0f32.to_bits());
        // Slots outside head..tail untouched.
        assert_eq!(ages[0].to_bits(), 0.0f32.to_bits());
        assert_eq!(ages[3].to_bits(), 0.0f32.to_bits());
        assert_eq!(verts[3 * N + 3].to_bits(), 0.0f32.to_bits());
    }
}

/// Offsets a row-major 4x4 transform's translation row by a billboard vector.
///
/// The billboard vector is resolved through the transform's upper-3x3 column
/// basis.
///
/// Returns a copy of `m` whose translation row (`m[12..15]`) is advanced by the
/// billboard offset; the rest of the matrix is unchanged.
pub fn c_particle_emitter__draw_batch__70ca50(m: &[f32; 16], bv: &[f32; 3]) -> [f32; 16] {
    let mut out = *m;
    out[12] = m[0] * bv[0] + m[4] * bv[1] + m[8] * bv[2] + m[12];
    out[13] = m[1] * bv[0] + m[5] * bv[1] + m[9] * bv[2] + m[13];
    out[14] = m[2] * bv[0] + m[6] * bv[1] + m[10] * bv[2] + m[14];
    out
}

#[cfg(test)]
mod draw_batch_tests {
    use super::c_particle_emitter__draw_batch__70ca50 as draw_batch;

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() <= 1e-4 * (1.0 + b.abs()), "{a} != {b}");
    }

    #[test]
    fn zero_billboard_leaves_matrix_unchanged() {
        let m: [f32; 16] = [
            2.0, 3.0, 5.0, 7.0, 11.0, 13.0, 17.0, 19.0, 23.0, 29.0, 31.0, 37.0, 41.0, 43.0, 47.0,
            53.0,
        ];
        let out = draw_batch(&m, &[0.0, 0.0, 0.0]);
        for i in 0..16 {
            assert_eq!(out[i].to_bits(), m[i].to_bits());
        }
    }

    #[test]
    fn identity_basis_adds_billboard_to_translation() {
        let m: [f32; 16] = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 10.0, 20.0, 30.0, 1.0,
        ];
        let bv = [0.5, -1.5, 2.25];
        let out = draw_batch(&m, &bv);
        approx(out[12], 10.5);
        approx(out[13], 18.5);
        approx(out[14], 32.25);
        for i in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 15] {
            assert_eq!(out[i].to_bits(), m[i].to_bits());
        }
    }

    #[test]
    fn column_basis_is_transposed_application() {
        let m: [f32; 16] = [
            0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 100.0, 200.0, 300.0, 1.0,
        ];
        let bv = [3.0, 4.0, 5.0];
        let out = draw_batch(&m, &bv);
        approx(out[12], 96.0);
        approx(out[13], 203.0);
        approx(out[14], 310.0);
    }

    #[test]
    fn translation_offset_is_additive_in_billboard() {
        let m: [f32; 16] = [
            0.3, 1.1, -0.7, 0.0, 0.9, -0.2, 0.4, 0.0, -1.3, 0.6, 0.8, 0.0, 5.0, -5.0, 2.0, 1.0,
        ];
        let a = [0.2, 0.5, -0.9];
        let b = [-0.4, 1.2, 0.3];
        let sum = [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
        let oa = draw_batch(&m, &a);
        let ob = draw_batch(&m, &b);
        let osum = draw_batch(&m, &sum);
        for k in 12..15 {
            approx(osum[k], oa[k] + ob[k] - m[k]);
        }
    }
}

/// `CParticleEmitter::Update` entry guard + per-particle age/fade decisions.
///
/// The only host-testable logic in the otherwise-impure update driver.
///
/// Locks the three x87 `FNSTSW`/`TEST AH`/`Jcc` polarities the driver depends on
/// so they can be asserted bit-exactly off the live image. `dt_runs_body`
/// reproduces the entry compare `fcomp dt,[0.0]; test ah,0x41; jnp early_out`:
/// the masked bits are `C3|C0` (0x41), so `dt > 0` clears both (body runs),
/// `dt <= 0` sets exactly one (early-out), and **NaN sets both** (`C3+C0`, even
/// parity) so the body STILL runs — matching stock. `age_advance`
/// reproduces the per-particle `fadd dt,[age]; fst [age]` followed by the
/// lifespan cull (`fcom [lifespan]; test ah,0x5; jp cull`, `age >= lifespan`
/// culls) and the fade byte (`fcomp [fade_thresh]; test ah,0x41; jne byte=0`,
/// where `jne` is ZF-based on a mask that **includes** `C3`, so only a strict
/// `age > fade_thresh` sets the byte to 1; equal and NaN set 0). All in
/// single-rounding `f32` to match the original's `FADD dword`/`FCOM dword`.
///
/// Entry guard: stock runs the body when `dt > 0` AND when `dt` is NaN, and
/// early-outs only for `dt <= 0`. Encoded as `!(dt <= 0.0)` so NaN (where
/// `NaN <= 0.0 == false`) runs the body, faithful to the even-parity unordered
/// result of `test ah,0x41; jnp`.
pub fn c_particle_emitter__update__7b5a10_dt_runs_body(dt: f32) -> bool {
    // Stock: `jnp early_out` is NOT taken (body runs) when AH&(C3|C0)==0 (dt>0)
    // OR when both bits are set (NaN => even parity). Only dt<=0 takes the jump.
    !(dt <= 0.0)
}

/// Per-particle age advance plus the two cull/fade decisions.
///
/// Returns the new age and the `(culled, fade_byte)` outcome. The age add and
/// both compares are the only float math the driver does inline; physics,
/// position eval, and list mutation are all delegated.
pub fn c_particle_emitter__update__7b5a10_age_advance(
    age: f32,
    dt: f32,
    lifespan: f32,
    fade_threshold: f32,
) -> (f32, bool, u8) {
    // `fld dt; fadd [age]; fst [age]` — single-rounding f32 add, stored back.
    let new_age = dt + age;
    // Lifespan cull: `fcom [lifespan]; test ah,0x5(C2|C0); jp cull`.
    // age<lifespan => C0=1 (odd) => not taken => alive; age>=lifespan (incl.
    // equal: both bits clear, even) => taken => cull. Negated `<` keeps the
    // unordered (NaN) case on the cull side, matching the even-parity jump.
    let culled = !(new_age < lifespan);
    // Fade byte: `fcomp [fade_threshold]; test ah,0x41(C3|C0); jne byte=0`. Here
    // `jne` is ZF-based and the mask INCLUDES `C3`, so equality (C3 set), `<`
    // (C0 set), and NaN all take `jne` => byte=0; only a strict
    // `age > fade_threshold` leaves the masked bits clear => byte=1. Distinct
    // from the cull's parity idiom (mask 0x5, C3 excluded) — do not conflate.
    let fade_byte = u8::from(new_age > fade_threshold);
    (new_age, culled, fade_byte)
}

#[cfg(test)]
mod tests_c_particle_emitter__update__7b5a10 {
    use super::{
        c_particle_emitter__update__7b5a10_age_advance as age_advance,
        c_particle_emitter__update__7b5a10_dt_runs_body as dt_runs_body,
    };

    #[test]
    fn entry_guard_positive_dt_runs_body() {
        assert!(dt_runs_body(0.016));
        assert!(dt_runs_body(f32::from_bits(1))); // smallest positive subnormal
        assert!(dt_runs_body(f32::INFINITY));
    }

    #[test]
    fn entry_guard_nonpositive_dt_early_outs() {
        // dt == 0 and dt < 0 both early-out (one masked bit set => odd parity).
        assert!(!dt_runs_body(0.0));
        assert!(!dt_runs_body(-0.0));
        assert!(!dt_runs_body(-1.0));
        assert!(!dt_runs_body(f32::NEG_INFINITY));
    }

    #[test]
    fn entry_guard_nan_dt_runs_body_like_stock() {
        // Ship-blocking polarity: NaN sets BOTH C3|C0 (even parity) so `jnp` is
        // NOT taken and the body runs. The naive `!(dt > 0.0)` would early-out
        // here and diverge from stock; `!(dt <= 0.0)` matches.
        assert!(dt_runs_body(f32::NAN));
        assert!(dt_runs_body(-f32::NAN));
    }

    #[test]
    fn age_advance_is_single_rounding_add() {
        let (new_age, ..) = age_advance(1.5, 0.25, 10.0, 5.0);
        assert_eq!(new_age.to_bits(), (0.25f32 + 1.5f32).to_bits());
    }

    #[test]
    fn lifespan_cull_polarity() {
        // age < lifespan => alive (not culled).
        let (_, culled, _) = age_advance(4.0, 0.0, 5.0, 100.0);
        assert!(!culled);
        // age == lifespan => culled (both bits clear, even => jp taken).
        let (_, culled_eq, _) = age_advance(5.0, 0.0, 5.0, 100.0);
        assert!(culled_eq);
        // age > lifespan => culled.
        let (_, culled_gt, _) = age_advance(6.0, 0.0, 5.0, 100.0);
        assert!(culled_gt);
        // NaN age => culled (unordered lands on the even-parity cull side).
        let (_, culled_nan, _) = age_advance(f32::NAN, 0.0, 5.0, 100.0);
        assert!(culled_nan);
    }

    #[test]
    fn fade_byte_polarity() {
        // Stock `test ah,0x41; jne byte=0` is ZF-based with C3 in the mask:
        // byte=1 ONLY for a strict age > threshold; less, equal, and NaN => 0.
        let (_, _, b_lt) = age_advance(2.0, 0.0, 100.0, 5.0);
        assert_eq!(b_lt, 0);
        // age == threshold => C3 set => JNE taken => byte 0.
        let (_, _, b_eq) = age_advance(5.0, 0.0, 100.0, 5.0);
        assert_eq!(b_eq, 0);
        // age > threshold => masked bits clear => byte 1.
        let (_, _, b_gt) = age_advance(7.0, 0.0, 100.0, 5.0);
        assert_eq!(b_gt, 1);
        // NaN age => masked bits set => byte 0.
        let (_, _, b_nan) = age_advance(f32::NAN, 0.0, 100.0, 5.0);
        assert_eq!(b_nan, 0);
    }

    #[test]
    fn cull_mask_not_conflated_with_fade_mask() {
        // The cull compare (mask 0x5, parity) and the fade compare (mask 0x41,
        // ZF incl. C3) are independent idioms: at exact equality the cull culls
        // (`>=`) while the fade stays 0 (only a strict `>`). A single shared
        // idiom could not produce both.
        let (_, culled, fade) = age_advance(5.0, 0.0, 5.0, 5.0);
        assert!(culled);
        assert_eq!(fade, 0);
    }
}

/// Quantizes a color channel to its 8-bit value the way `CParticleEmitter::ApplyRenderState` does.
///
/// For every diffuse/emissive channel and alpha: clamp to `[0, 1]`, then
/// `trunc(c * 255.0 + 0.5)`, keeping the low byte the stock body stores from
/// `__ftol`'s `al`.
///
/// The clamp mirrors the x87 ladder exactly: `c < 0` floors to `0.0`; `c > 1`
/// ceils to `1.0`; otherwise (`c <= 1`, and any NaN) the value passes through.
/// A NaN therefore reaches `c * 255.0 + 0.5` and truncates to `0` (low byte
/// `0x00`), matching `__ftol`'s indefinite result. The x87 source evaluates the
/// `* 255.0 + 0.5` in 80-bit precision; this kernel uses `f32`, so the truncated
/// byte may differ by one LSB on the rare value that straddles a `.5` boundary —
/// the intended x87->SSE substitution this effort ships (see the sibling
/// `c_particle_emitter__set_alpha__7b7b10`). The `255.0` scale lives at host
/// global `0x7ffe58`, the `0.5` bias at `0x7ffa24`, the clamp bounds `0.0` at
/// `0x7ffd74` and `1.0` at `0x7ff9d8`.
// REASON: the explicit `< 0` / `> 1` ladder is NOT `f32::clamp`. The stock x87
// path passes a NaN through to `* 255.0 + 0.5` (truncating to the `0x00` low
// byte), whereas `c.clamp(0.0, 1.0)` returns NaN for a NaN input — a different
// result. Keeping the manual ladder preserves the source's NaN polarity.
#[allow(clippy::manual_clamp)]
pub fn clamp01_to_color_byte__70c190(c: f32) -> u32 {
    let clamped = if c < 0.0 {
        0.0
    } else if c > 1.0 {
        1.0
    } else {
        c
    };
    ((clamped * 255.0 + 0.5) as i32 & 0xff) as u32
}

/// Decides the `RenderState(5)` value from one basis row of the emitter's orientation.
///
/// `0` when the row is (near-)unit length, `1` otherwise.
///
/// Reproduces the squared-magnitude epsilon test at `0x70c36a`/`0x70c569`:
/// `((x*x + y*y) + z*z) - 1.0`, absolute value, compared against the epsilon
/// `0.001` (`0x3a83126f`). The stock keeps the strict-greater polarity
/// (`test ah,0x41; jne/je`): the row counts as non-unit only when the abs
/// difference is `> 0.001` and ordered, so a NaN difference yields `0` (unit).
/// The summation order `(x*x + y*y) + z*z` is the x87 `faddp` grouping and is
/// preserved verbatim. The constant `1.0` lives at host global `0x7ff9d8`, the
/// epsilon at `0x801360`.
pub fn rotation_row_render_state__70c190(x: f32, y: f32, z: f32, epsilon: f32) -> u32 {
    let sqmag = (x * x + y * y) + z * z;
    // `> epsilon` (not `<= epsilon` negated) makes a NaN difference fall to the
    // unit (`0`) arm, exactly as the stock `jne`/`je` around `fcomp epsilon` does.
    u32::from((sqmag - 1.0).abs() > epsilon)
}

/// `trunc(depth * 224.0)` blend/sort parameter at site `0x70c256`.
///
/// (`fld [ecx+0x10]; fmul 224.0; call __ftol`.) Returned as the i32 the stock
/// truncating `__ftol` produces for `RenderState(8)`. The `224.0` scale
/// (`0x43600000`) lives at host global `0x812034`.
pub fn blend_sort_param_224__70c190(depth: f32) -> i32 {
    (depth * 224.0) as i32
}

/// Packs four already-quantized channel bytes into the `0xAARRGGBB` D3DCOLOR dword.
///
/// The stock body assembles it from its `[ebp-4 .. ebp-1]` byte slots before
/// calling the scalar render-state setter (`B` at the low address, `A` at the
/// high). Each argument carries only its low 8 bits, as the source stores do.
pub fn pack_argb__70c190(a: u32, r: u32, g: u32, b: u32) -> u32 {
    (b & 0xff) | ((g & 0xff) << 8) | ((r & 0xff) << 16) | ((a & 0xff) << 24)
}

#[cfg(test)]
mod tests_apply_render_state__70c190 {
    use super::{
        blend_sort_param_224__70c190 as blend_sort, clamp01_to_color_byte__70c190 as quantize,
        pack_argb__70c190 as pack, rotation_row_render_state__70c190 as row_state,
    };

    const EPSILON: f32 = f32::from_bits(0x3a83_126f); // 0.0010000000474974513

    #[test]
    fn clamp_floor_ceil_and_round_half_up() {
        assert_eq!(quantize(-0.5), 0); // floored to 0.0
        assert_eq!(quantize(0.0), 0); // trunc(0.5) = 0
        assert_eq!(quantize(0.5), 128); // trunc(127.5 + 0.5) = 128
        assert_eq!(quantize(1.0), 255); // trunc(255.5) = 255
        assert_eq!(quantize(2.0), 255); // ceiled to 1.0 -> 255
    }

    #[test]
    fn clamp_low_is_strict_and_nan_passes_through_to_zero() {
        // Exactly 0.0 is not floored by a strict `< 0` test; trunc(0.5)=0 anyway.
        assert_eq!(quantize(0.0), 0);
        // A NaN reaches `*255+0.5` and truncates to the 0x00 low byte.
        assert_eq!(quantize(f32::NAN), 0);
    }

    #[test]
    fn row_state_unit_vs_nonunit_and_nan() {
        // Unit row -> 0.
        assert_eq!(row_state(1.0, 0.0, 0.0, EPSILON), 0);
        assert_eq!(row_state(0.0, 1.0, 0.0, EPSILON), 0);
        // Just inside the epsilon stays unit (sqmag-1 ~= 0.00049 <= 0.001).
        assert_eq!(row_state(f32::from_bits(0x3f80_0200), 0.0, 0.0, EPSILON), 0);
        // Clearly non-unit -> 1.
        assert_eq!(row_state(2.0, 0.0, 0.0, EPSILON), 1);
        // A NaN difference falls to the unit (0) arm via the `>` polarity.
        assert_eq!(row_state(f32::NAN, 0.0, 0.0, EPSILON), 0);
    }

    #[test]
    fn row_state_summation_order_is_left_assoc() {
        let (x, y, z) = (1.0e30f32, 1.0f32, -1.0e30f32);
        let want = u32::from((((x * x + y * y) + z * z) - 1.0).abs() > EPSILON);
        assert_eq!(row_state(x, y, z, EPSILON), want);
    }

    #[test]
    fn blend_sort_truncates_toward_zero() {
        assert_eq!(blend_sort(0.0), 0);
        assert_eq!(blend_sort(0.5), 112); // trunc(112.0)
        assert_eq!(blend_sort(0.999), 223); // trunc(223.776) = 223
        assert_eq!(blend_sort(-0.5), -112);
    }

    #[test]
    fn pack_argb_byte_order() {
        assert_eq!(pack(0x11, 0x22, 0x33, 0x44), 0x1122_3344);
        // Only the low byte of each argument survives.
        assert_eq!(pack(0x1ff, 0x2ff, 0x3ff, 0x4ff), 0xffff_ffff);
        assert_eq!(pack(0, 0, 0, 0), 0);
    }
}

/// Mantissa of a 32-bit RNG word reinterpreted into `[1, 2)`.
///
/// The raw `AND 0x7fffff; OR 0x3f800000` bit trick of `SpawnParticle` 0x7b8890
/// (unlike [`rng_unit`] above, the `- 1.0` normalisation is NOT folded in here
/// — the callers subtract a host-injected base/center instead).
fn spawn_rand_mantissa__7b8890(r: u32) -> f32 {
    f32::from_bits((r & 0x007f_ffff) | 0x3f80_0000)
}

/// Sign-centered random draw of `SpawnParticle` 0x7b8890.
///
/// The mantissa float `m ∈ [1, 2)` is folded about `center` (the host global
/// at 0x801628 — **live image value 2.0f**, so the spread is
/// `±(2 − m) ∈ (-1, 1)` mirrored by the RNG sign; an earlier 1.5/±0.5 reading
/// of that slot was wrong) using the SIGN BIT of the raw RNG word as an integer
/// test — `center - m` when `(int)r < 0`, else `m - center`. No x87 compares
/// are involved (stock uses `TEST EAX,EAX; JGE`), so there is no NaN-polarity
/// concern.
fn spawn_rand_centered__7b8890(r: u32, center: f32) -> f32 {
    let m = spawn_rand_mantissa__7b8890(r);
    if (r as i32) < 0 {
        center - m
    } else {
        m - center
    }
}

/// Particle age seed of `SpawnParticle` 0x7b8890.
///
/// `(m - base) * dt` where `m` is the `[1, 2)` mantissa float of the first draw
/// (NO sign fold) and `base` is the host global at 0x7ff9d8 (1.0), giving
/// `[0, 1) · dt`.
pub fn c_particle_emitter__spawn_age__7b8890(r: u32, base: f32, dt: f32) -> f32 {
    (spawn_rand_mantissa__7b8890(r) - base) * dt
}

/// Planar spawn offset of `SpawnParticle` 0x7b8890.
///
/// The emitter-local x/y position built from two sign-centered draws. Stock
/// draws the Y word FIRST (draw #2) and the X word second (draw #3):
/// `x = centered(r_x)·scale_x·half`, `y = centered(r_y)·scale_y·half` where
/// `scale_x`/`scale_y` are emitter fields +0x290/+0x294 and `half` is the host
/// global at 0x7ffa24 (0.5). Returns `[x, y]`; z is stored as literal `+0.0`
/// by the adapter.
pub fn c_particle_emitter__spawn_planar_offset__7b8890(
    r_y: u32,
    r_x: u32,
    center: f32,
    scale_x: f32,
    scale_y: f32,
    half: f32,
) -> [f32; 2] {
    let cy = spawn_rand_centered__7b8890(r_y, center);
    let cx = spawn_rand_centered__7b8890(r_x, center);
    [cx * scale_x * half, cy * scale_y * half]
}

/// Emission-cone velocity of `SpawnParticle` 0x7b8890 (mode selector +0x188 is zero).
///
/// Two sign-centered draws scaled by the emitter cone-angle fields
/// (+0x298 azimuth `scale_a`, +0x29c elevation `scale_b`) feed the classic
/// spherical direction at `radius`:
/// `[cos(b)·sin(a)·r, sin(b)·sin(a)·r, cos(a)·r]` with `sin(a)·r` computed
/// once and shared, matching the stock x87 op order.
///
/// The stock 2×`FSIN`+2×`FCOS` are replaced with `libm` `sinf`/`cosf` (the
/// landed `Math::Pow`/`Exp2` precedent): correctly-rounded SSE2, ULP-class
/// differences vs the x87 transcendentals — the accepted precision class for
/// this campaign. Arguments are small (`|angle| ≤ 0.5·scale`), far inside
/// every reduction's exact range.
pub fn c_particle_emitter__spawn_cone_velocity__7b8890(
    r_a: u32,
    r_b: u32,
    center: f32,
    scale_a: f32,
    scale_b: f32,
    radius: f32,
) -> [f32; 3] {
    let angle_a = spawn_rand_centered__7b8890(r_a, center) * scale_a;
    let angle_b = spawn_rand_centered__7b8890(r_b, center) * scale_b;
    let sin_a_r = libm::sinf(angle_a) * radius;
    let vz = libm::cosf(angle_a) * radius;
    let vy = libm::sinf(angle_b) * sin_a_r;
    let vx = libm::cosf(angle_b) * sin_a_r;
    [vx, vy, vz]
}

/// Directed velocity of `SpawnParticle` 0x7b8890 (mode selector +0x188 is non-zero).
///
/// Normalise `dir` — the vector from the target point to the spawn offset — to
/// length `radius`: `s = radius / sqrt(dir·dir)`, `v = dir · s`.
///
/// Stock performs `FSQRT; FDIVR` with NO zero guard: a zero-length `dir`
/// divides by zero (`s = ±inf`, or NaN when `radius` is also 0) and the
/// `0 · inf = NaN` products propagate into the velocity — reproduced verbatim
/// (IEEE f32 gives the identical inf/NaN behavior); do NOT add a guard. The
/// squared magnitude reuses the hooked `C3Vector::SquaredMagnitude` kernel,
/// exactly the call stock makes.
pub fn c_particle_emitter__spawn_directed_velocity__7b8890(dir: [f32; 3], radius: f32) -> [f32; 3] {
    let s = radius / crate::math::vector::c3_vector__squared_magnitude__4549f0(&dir).sqrt();
    [dir[0] * s, dir[1] * s, dir[2] * s]
}

/// Random velocity kick of `SpawnParticle` 0x7b8890 (flag 0x400).
///
/// One more sign-centered draw builds the kick magnitude
/// `k = centered·scale + base` (`scale` = emitter +0x184, `base` = the 1.0 host
/// global at 0x7ff9d8 — the same word the age math subtracts), which scales the
/// emitter velocity direction (+0x258/+0x25c/+0x260) and accumulates onto the
/// current record velocity: `vel + k·dir` per component.
pub fn c_particle_emitter__spawn_velocity_kick__7b8890(
    r: u32,
    center: f32,
    scale: f32,
    base: f32,
    dir: [f32; 3],
    vel: [f32; 3],
) -> [f32; 3] {
    let k = spawn_rand_centered__7b8890(r, center) * scale + base;
    [
        k * dir[0] + vel[0],
        k * dir[1] + vel[1],
        k * dir[2] + vel[2],
    ]
}

#[cfg(test)]
mod tests_c_particle_emitter__spawn_particle__7b8890 {
    use super::{
        c_particle_emitter__spawn_age__7b8890 as age,
        c_particle_emitter__spawn_cone_velocity__7b8890 as cone,
        c_particle_emitter__spawn_directed_velocity__7b8890 as directed,
        c_particle_emitter__spawn_planar_offset__7b8890 as planar,
        c_particle_emitter__spawn_velocity_kick__7b8890 as kick,
        spawn_rand_centered__7b8890 as centered,
    };

    /// Host-global stand-ins the adapter injects.
    ///
    /// The kernels are fully parameterized, so these are MOCK values for the
    /// tests; the live image holds `_DAT_00801628 = 2.0f` (CENTER here is
    /// deliberately different to exercise the fold generically),
    /// `_DAT_007ff9d8 = 1.0f`, `_DAT_007ffa24 = 0.5f`.
    const CENTER: f32 = 1.5; // mock; live _DAT_00801628 is 2.0f
    const BASE: f32 = 1.0; // _DAT_007ff9d8
    const HALF: f32 = 0.5; // _DAT_007ffa24

    /// Independent mantissa oracle: `1 + (r & 0x7fffff) / 2^23`, exact in f64.
    fn mantissa_oracle(r: u32) -> f32 {
        (1.0 + f64::from(r & 0x007f_ffff) / 8_388_608.0) as f32
    }

    /// Tiny deterministic LCG standing in for the stock table PRNG.
    ///
    /// (The real draw stream is stateful game memory — only in-game validation
    /// can check draw ORDER; these tests pin the pure math on arbitrary words.)
    fn mock_prng(state: &mut u32) -> u32 {
        *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *state
    }

    #[test]
    fn centered_sign_bit_folds_about_center() {
        // Sign bit clear: m - center. Mantissa 0 -> m = 1.0.
        assert_eq!(centered(0x0000_0000, CENTER), -0.5);
        // Sign bit set, same mantissa: center - m — the exact negation.
        assert_eq!(centered(0x8000_0000, CENTER), 0.5);
        // Exponent bits below the sign are ignored by the mask.
        assert_eq!(centered(0x4000_0000, CENTER), -0.5);
        // Top of the mantissa range: 1.99999988 - 1.5.
        let hi = mantissa_oracle(0x007f_ffff) - CENTER;
        assert_eq!(centered(0x007f_ffff, CENTER), hi);
        assert_eq!(centered(0x807f_ffff, CENTER), -hi);
    }

    #[test]
    fn centered_matches_oracle_and_stays_in_range() {
        let mut s = 0xdead_beef;
        for _ in 0..10_000 {
            let r = mock_prng(&mut s);
            let want = if (r as i32) < 0 {
                CENTER - mantissa_oracle(r)
            } else {
                mantissa_oracle(r) - CENTER
            };
            let got = centered(r, CENTER);
            assert_eq!(got.to_bits(), want.to_bits(), "r={r:#010x}");
            assert!((-0.5..=0.5).contains(&got), "r={r:#010x} -> {got}");
        }
    }

    #[test]
    fn age_is_unsigned_fraction_of_dt() {
        // Mantissa 0 -> (1.0 - 1.0)·dt = 0, regardless of the sign bit.
        assert_eq!(age(0x0000_0000, BASE, 0.25), 0.0);
        assert_eq!(age(0xff80_0000, BASE, 0.25), 0.0);
        // Oracle sweep: (m - 1)·dt in [0, dt).
        let mut s = 7;
        for _ in 0..1_000 {
            let r = mock_prng(&mut s);
            let want = (mantissa_oracle(r) - BASE) * 0.25;
            assert_eq!(age(r, BASE, 0.25).to_bits(), want.to_bits());
            assert!((0.0..0.25).contains(&age(r, BASE, 0.25)));
        }
    }

    #[test]
    fn planar_offset_scales_and_orders_draws() {
        // r_y (draw #2) feeds y via scale_y; r_x (draw #3) feeds x via scale_x.
        // centered(0x8000_0000) = +0.5, centered(0) = -0.5.
        let [x, y] = planar(0x8000_0000, 0x0000_0000, CENTER, 2.0, 4.0, HALF);
        assert_eq!(x, -0.5 * 2.0 * 0.5);
        assert_eq!(y, 0.5 * 4.0 * 0.5);
    }

    #[test]
    fn cone_zero_angles_point_up() {
        // Mantissa 0x400000 -> m = 1.5 -> centered = 0 -> both angles 0:
        // sin = 0, cos = 1 -> [0, 0, radius].
        let v = cone(0x0040_0000, 0x0040_0000, CENTER, 3.0, 7.0, 2.5);
        assert_eq!(v, [0.0, 0.0, 2.5]);
    }

    #[test]
    fn cone_matches_f64_oracle_and_radius() {
        let mut s = 42;
        for _ in 0..2_000 {
            let (ra, rb) = (mock_prng(&mut s), mock_prng(&mut s));
            let (sa, sb, r) = (0.9f32, 2.8f32, 5.0f32);
            let v = cone(ra, rb, CENTER, sa, sb, r);
            let a = f64::from(centered(ra, CENTER) * sa);
            let b = f64::from(centered(rb, CENTER) * sb);
            let want = [
                b.cos() * a.sin() * 5.0,
                b.sin() * a.sin() * 5.0,
                a.cos() * 5.0,
            ];
            for (got, want) in v.iter().zip(want) {
                assert!(
                    (f64::from(*got) - want).abs() < 1e-5,
                    "got {got}, want {want}"
                );
            }
            // The direction is exactly radius long (up to f32 rounding).
            let mag = (f64::from(v[0]).powi(2) + f64::from(v[1]).powi(2) + f64::from(v[2]).powi(2))
                .sqrt();
            assert!((mag - 5.0).abs() < 1e-5);
        }
    }

    #[test]
    fn directed_normalises_to_radius() {
        // |[3,4,0]| = 5, s = 10/5 = 2 — exact in f32.
        assert_eq!(directed([3.0, 4.0, 0.0], 10.0), [6.0, 8.0, 0.0]);
        // Negative components ride along.
        assert_eq!(directed([-3.0, 0.0, 4.0], 10.0), [-6.0, 0.0, 8.0]);
    }

    #[test]
    fn directed_zero_vector_divides_unguarded() {
        // Stock has NO zero guard: s = radius/0 = inf and 0·inf = NaN — the
        // faithful bug reproduction (velocity becomes NaN, exactly as x87).
        let v = directed([0.0, 0.0, 0.0], 5.0);
        assert!(v.iter().all(|c| c.is_nan()), "{v:?}");
        // radius 0 too: 0/0 = NaN propagates.
        let v = directed([0.0, 0.0, 0.0], 0.0);
        assert!(v.iter().all(|c| c.is_nan()), "{v:?}");
        // A zero radius over a valid direction scales to ±0, not NaN.
        assert_eq!(directed([3.0, 4.0, 0.0], 0.0), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn kick_accumulates_scaled_direction() {
        // centered(0x8000_0000) = +0.5 -> k = 0.5·2 + 1 = 2.
        let v = kick(
            0x8000_0000,
            CENTER,
            2.0,
            BASE,
            [1.0, 2.0, 3.0],
            [10.0, 20.0, 30.0],
        );
        assert_eq!(v, [12.0, 24.0, 36.0]);
        // Sign-bit-clear draw kicks the other way: k = -0.5·2 + 1 = 0.
        let v = kick(
            0x0000_0000,
            CENTER,
            2.0,
            BASE,
            [1.0, 2.0, 3.0],
            [10.0, 20.0, 30.0],
        );
        assert_eq!(v, [10.0, 20.0, 30.0]);
    }

    #[test]
    fn kick_preserves_nan_velocity() {
        // A NaN velocity (the directed zero-vector case upstream) stays NaN
        // through the accumulate — nothing in the kick masks it.
        let v = kick(
            0x1234_5678,
            CENTER,
            1.0,
            BASE,
            [1.0, 1.0, 1.0],
            [f32::NAN; 3],
        );
        assert!(v.iter().all(|c| c.is_nan()));
    }
}

/// Catch-up split for `CParticleEmitter::UpdateFixedStep` (@0x7b5880).
///
/// Breaks an oversized `dt` into `count` fixed-step Update calls plus a
/// residual, and pre-scales the three `+0x26c..+0x274` state floats by
/// `1/(count+1)`.
///
/// Returns `None` on the copy arm — the stock `FCOMP dt, step; TEST AH,0x41`
/// takes it for `dt <= step` AND for NaN `dt` (unordered sets C0/C3), so the
/// catch-up arm runs only on a strictly-greater ordered compare. On the
/// catch-up arm:
/// - `n = f32(rint(f64(dt/step)))` — the quotient is narrowed to double for
///   the CRT rint (`FSTP qword` before the call), rounded to nearest-even
///   (the CRT helper is an FRNDINT under the default control word), then
///   narrowed to f32 at the FSTP.
/// - `residual = f32(dt − step·n)` folded at extended precision, one narrow.
/// - `m = rint(f64(lifetime/step))` stays wide on the stack; `n` is replaced
///   by `f32(m)` only when `n > m` ordered (NaN `m` keeps `n`), then clamped
///   to the 255.0 immediate only when `n > cap` ordered.
/// - `count = ((n + magic).to_bits() >> 14) & 0xff` — the same magic-bias
///   mantissa extract as the color-track round (`magic` is the 512.0 global
///   at 0x8029cc), narrowed once at the FSTP.
/// - `k = one / (count+1)` divides wide (64-bit FILD of the incremented
///   count), and each source float multiplies against the WIDE quotient with
///   its own narrow (three FSTPs).
pub fn fixed_step_catchup__7b5880(
    dt: f32,
    lifetime: f32,
    src: [f32; 3],
    step: f32,
    cap: f32,
    magic: f32,
    one: f32,
) -> Option<(f32, u32, [f32; 3])> {
    if !(dt > step) {
        return None;
    }
    let mut n = super::f64_to_f32((f64::from(dt) / f64::from(step)).round_ties_even());
    let residual = super::f64_to_f32(f64::from(dt) - f64::from(step) * f64::from(n));
    let m = (f64::from(lifetime) / f64::from(step)).round_ties_even();
    if f64::from(n) > m {
        n = super::f64_to_f32(m);
    }
    if f64::from(n) > f64::from(cap) {
        n = 255.0;
    }
    let biased = super::f64_to_f32(f64::from(n) + f64::from(magic));
    let count = (biased.to_bits() >> 14) & 0xff;
    let k = f64::from(one) / f64::from(count + 1);
    let scaled = [
        super::f64_to_f32(k * f64::from(src[0])),
        super::f64_to_f32(k * f64::from(src[1])),
        super::f64_to_f32(k * f64::from(src[2])),
    ];
    Some((residual, count, scaled))
}

#[cfg(test)]
mod tests_fixed_step_catchup__7b5880 {
    use super::fixed_step_catchup__7b5880 as catchup;

    const STEP: f32 = 0.03;
    const CAP: f32 = 255.0;
    const MAGIC: f32 = 512.0;

    #[test]
    fn copy_arm_on_small_equal_and_nan_dt() {
        assert!(catchup(0.01, 10.0, [1.0; 3], STEP, CAP, MAGIC, 1.0).is_none());
        assert!(catchup(STEP, 10.0, [1.0; 3], STEP, CAP, MAGIC, 1.0).is_none());
        assert!(catchup(f32::NAN, 10.0, [1.0; 3], STEP, CAP, MAGIC, 1.0).is_none());
    }

    /// dt = 5.25 steps.
    ///
    /// rint rounds the quotient to 5, the residual keeps the quarter step, and
    /// the pre-scale divides by count+1 = 6.
    #[test]
    fn exact_split_and_prescale() {
        let (residual, count, scaled) =
            catchup(0.1575, 10.0, [6.0, 12.0, 18.0], STEP, CAP, MAGIC, 1.0).unwrap();
        assert_eq!(count, 5);
        let want_res = (f64::from(0.1575f32) - f64::from(STEP) * 5.0) as f32;
        assert_eq!(residual.to_bits(), want_res.to_bits());
        assert_eq!(scaled[0].to_bits(), 1.0f32.to_bits());
        assert_eq!(scaled[1].to_bits(), 2.0f32.to_bits());
        assert_eq!(scaled[2].to_bits(), 3.0f32.to_bits());
    }

    /// The rint is to-nearest-EVEN on a true .5 quotient (exact dyadic operands).
    ///
    /// 2.5 steps -> 2, and the residual keeps the half step.
    #[test]
    fn rint_ties_even_on_quotient() {
        let (residual, count, _) = catchup(0.625, 10.0, [1.0; 3], 0.25, CAP, MAGIC, 1.0).unwrap();
        assert_eq!(count, 2);
        assert_eq!(residual.to_bits(), 0.125f32.to_bits());
    }

    /// The lifetime quotient caps n (5 steps of dt but lifetime worth 3).
    #[test]
    fn lifetime_caps_count() {
        let (_, count, _) = catchup(0.15, 0.09, [1.0; 3], STEP, CAP, MAGIC, 1.0).unwrap();
        assert_eq!(count, 3);
    }

    /// NaN lifetime: the n > m compare is unordered, n survives.
    #[test]
    fn nan_lifetime_keeps_n() {
        let (_, count, _) = catchup(0.15, f32::NAN, [1.0; 3], STEP, CAP, MAGIC, 1.0).unwrap();
        assert_eq!(count, 5);
    }

    /// Oversized n clamps to the 255.0 immediate and extracts 255.
    #[test]
    fn cap_clamps_to_255() {
        let (_, count, _) = catchup(30.0, 1.0e9, [3.0; 3], STEP, CAP, MAGIC, 1.0).unwrap();
        assert_eq!(count, 255);
    }
}

/// Camera-fade scalar of `CParticleEmitter::EmitParticles` (0x7b5550 prologue).
///
/// `t = one − (cam − fade_base) × fade_scale`, computed wide.
/// The FIRST clamp compares the WIDE value against `quarter` (`FST` keeps
/// ST0; `TEST AH,5; JP` — only an ordered `t < quarter` takes 0.25, NaN
/// falls through), the SECOND compares the NARROWED store against `one`
/// (`JNP` — kept only when ordered `< one`), so a NaN chain lands on 1.0.
#[must_use]
pub fn emit_fade_clamp__7b5550(
    cam: f32,
    fade_base: f32,
    fade_scale: f32,
    one: f32,
    quarter: f32,
) -> f32 {
    let wide = f64::from(one) - (f64::from(cam) - f64::from(fade_base)) * f64::from(fade_scale);
    if wide < f64::from(quarter) {
        return quarter;
    }
    let narrow = super::f64_to_f32(wide);
    if narrow < one { narrow } else { one }
}

#[cfg(test)]
mod tests_emit_fade_clamp__7b5550 {
    use super::emit_fade_clamp__7b5550 as clamp;

    #[test]
    fn near_camera_is_full_rate() {
        // cam == fade_base → t = 1.0 → not < 1.0 → the 1.0 arm.
        assert_eq!(
            clamp(10.0, 10.0, 0.01, 1.0, 0.25).to_bits(),
            1.0f32.to_bits()
        );
    }

    #[test]
    fn far_camera_clamps_to_quarter() {
        // t = 1 − 90×0.01 = 0.1 < 0.25.
        assert_eq!(
            clamp(100.0, 10.0, 0.01, 1.0, 0.25).to_bits(),
            0.25f32.to_bits()
        );
    }

    #[test]
    fn mid_range_keeps_the_narrowed_fade() {
        // t = 1 − 50×0.01 = 0.5.
        assert_eq!(
            clamp(60.0, 10.0, 0.01, 1.0, 0.25).to_bits(),
            0.5f32.to_bits()
        );
    }

    #[test]
    fn nan_routes_to_one() {
        // Unordered falls through BOTH compares → 1.0 (stock JP/JNP pair).
        assert_eq!(
            clamp(f32::NAN, 10.0, 0.01, 1.0, 0.25).to_bits(),
            1.0f32.to_bits()
        );
    }
}

/// Emission rate of `CParticleEmitter::EmitParticles` (0x7b55a3..0x7b55bb).
///
/// The inlined `WowGlobals` getter float × the emitter rate field × the
/// fade scalar, all wide, one narrowing at the `FSTP`.
#[must_use]
pub fn emit_rate__7b5550(g: f32, emitter_rate: f32, fade: f32) -> f32 {
    super::f64_to_f32(f64::from(g) * f64::from(emitter_rate) * f64::from(fade))
}

#[cfg(test)]
mod tests_emit_rate__7b5550 {
    use super::emit_rate__7b5550 as rate;

    #[test]
    fn wide_product_narrows_once() {
        let expect = ((f64::from(0.3f32) * f64::from(7.0f32)) * f64::from(0.25f32)) as f32;
        assert_eq!(rate(0.3, 7.0, 0.25).to_bits(), expect.to_bits());
    }

    #[test]
    fn zero_fade_is_zero() {
        assert_eq!(rate(5.0, 3.0, 0.0).to_bits(), 0.0f32.to_bits());
    }
}

/// Accumulator step of `CParticleEmitter::EmitParticles` (0x7b565d..0x7b56b6 / 0x7b57d0).
///
/// `wide = rate × dt + accum` stays on the x87 stack — the
/// `FST` into `this+0x8` narrows the STORE but the `+half` count truncation
/// consumes the WIDE value. Returns `(stored, count)` with `count` = the
/// CRT `__ftol` low 32 of `wide + half`.
#[must_use]
pub fn emit_accum_count__7b5550(rate: f32, dt: f32, accum: f32, half: f32) -> (f32, u32) {
    let wide = f64::from(rate) * f64::from(dt) + f64::from(accum);
    let stored = super::f64_to_f32(wide);
    let count = crate::math::misc::ftol__40a2b0(wide + f64::from(half)) as u32;
    (stored, count)
}

#[cfg(test)]
mod tests_emit_accum_count__7b5550 {
    use super::emit_accum_count__7b5550 as step;

    #[test]
    fn rounds_half_up_for_positives() {
        let (stored, n) = step(2.5, 1.0, 0.0, 0.5);
        assert_eq!(stored.to_bits(), 2.5f32.to_bits());
        assert_eq!(n, 3);
        let (_, n) = step(2.4, 1.0, 0.0, 0.5);
        assert_eq!(n, 2);
    }

    #[test]
    fn count_uses_the_wide_sum_not_the_narrowed_store() {
        // accum = 2^24 − 2, rate×dt = 0.5: wide = 16777214.5 (ulp here is
        // 1.0). The f32 store rounds half-even DOWN to 16777214, but the
        // count truncates wide + 0.5 = 16777215 — one MORE than the
        // narrowed value implies.
        let (stored, n) = step(0.5, 1.0, 16_777_214.0, 0.5);
        assert_eq!(stored.to_bits(), 16_777_214.0f32.to_bits());
        assert_eq!(n, 16_777_215);
    }

    #[test]
    fn negative_accum_truncates_toward_zero() {
        let (_, n) = step(0.0, 1.0, -3.25, 0.5);
        // __ftol(-2.75) = -2 → EAX low 32.
        assert_eq!(n, (-2i32) as u32);
    }
}

/// Path-interpolation deltas of the flag-0x1000 arm (0x7b5686..0x7b56ad).
///
/// Per-component `FSUB` + `FSTP` — each `ctx` position minus the emitter
/// base narrows once.
#[must_use]
pub fn emit_path_deltas__7b5550(pos: &[f32; 3], base: &[f32; 3]) -> [f32; 3] {
    [
        super::f64_to_f32(f64::from(pos[0]) - f64::from(base[0])),
        super::f64_to_f32(f64::from(pos[1]) - f64::from(base[1])),
        super::f64_to_f32(f64::from(pos[2]) - f64::from(base[2])),
    ]
}

#[cfg(test)]
mod tests_emit_path_deltas__7b5550 {
    use super::emit_path_deltas__7b5550 as deltas;

    #[test]
    fn component_subtracts_narrow_once() {
        let d = deltas(&[3.0, 5.0, -1.0], &[1.0, 2.0, 1.0]);
        assert_eq!(d[0].to_bits(), 2.0f32.to_bits());
        assert_eq!(d[1].to_bits(), 3.0f32.to_bits());
        assert_eq!(d[2].to_bits(), (-2.0f32).to_bits());
    }
}

/// One interpolated spawn point of the flag-0x1000 arm (0x7b56e6..0x7b572b).
///
/// The RNG draw's mantissa bits build the `[1, 2)` float, `frac = m − one`
/// (exact) stays WIDE on the stack, and each component is `delta × frac +
/// base` with one narrowing. `base` and `one` are re-read live per spawn by
/// the adapter, exactly like the stock per-iteration loads.
#[must_use]
pub fn emit_path_point__7b5550(d: &[f32; 3], base: &[f32; 3], bits: u32, one: f32) -> [f32; 3] {
    let m = f32::from_bits(bits & 0x007f_ffff | 0x3f80_0000);
    let frac = f64::from(m) - f64::from(one);
    [
        super::f64_to_f32(f64::from(d[0]) * frac + f64::from(base[0])),
        super::f64_to_f32(f64::from(d[1]) * frac + f64::from(base[1])),
        super::f64_to_f32(f64::from(d[2]) * frac + f64::from(base[2])),
    ]
}

#[cfg(test)]
mod tests_emit_path_point__7b5550 {
    use super::emit_path_point__7b5550 as point;

    #[test]
    fn zero_mantissa_is_the_base() {
        let p = point(&[10.0, 20.0, 30.0], &[1.0, 2.0, 3.0], 0, 1.0);
        assert_eq!(p[0].to_bits(), 1.0f32.to_bits());
        assert_eq!(p[1].to_bits(), 2.0f32.to_bits());
        assert_eq!(p[2].to_bits(), 3.0f32.to_bits());
    }

    #[test]
    fn high_draw_bits_outside_the_mantissa_are_masked() {
        let a = point(&[8.0; 3], &[0.0; 3], 0x0040_0000, 1.0);
        let b = point(&[8.0; 3], &[0.0; 3], 0xff40_0000, 1.0);
        // frac = 0.5 either way → 4.0.
        assert_eq!(a[0].to_bits(), 4.0f32.to_bits());
        assert_eq!(b[0].to_bits(), a[0].to_bits());
    }

    #[test]
    fn full_mantissa_stays_below_one() {
        let p = point(&[1.0; 3], &[0.0; 3], 0x007f_ffff, 1.0);
        assert!(p[0] < 1.0);
    }
}

/// Accumulator settle of `CParticleEmitter::EmitParticles` (0x7b5861 tail).
///
/// `FILD` of the zero-extended spawn counter (exact in f64) subtracted from
/// the re-read accumulator, one narrowing at the store.
#[must_use]
pub fn emit_accum_settle__7b5550(accum: f32, emitted: u32) -> f32 {
    super::f64_to_f32(f64::from(accum) - f64::from(emitted))
}

#[cfg(test)]
mod tests_emit_accum_settle__7b5550 {
    use super::emit_accum_settle__7b5550 as settle;

    #[test]
    fn subtracts_the_spawn_count_exactly() {
        assert_eq!(settle(3.5, 3).to_bits(), 0.5f32.to_bits());
        assert_eq!(settle(0.25, 0).to_bits(), 0.25f32.to_bits());
    }

    #[test]
    fn large_counter_is_exact_in_f64() {
        // 2^24 + 1 is not an f32 integer but IS exact in the f64 FILD.
        let n = (1u32 << 24) + 1;
        let expect = ((f64::from(2.0f32.powi(25))) - f64::from(n)) as f32;
        assert_eq!(settle(2.0f32.powi(25), n).to_bits(), expect.to_bits());
    }
}

/// Negated translation column of `CParticleEmitter::Render`'s local matrix (0x7b3e51..0x7b3e8d).
///
/// Three inline `fld;fchs;fstp` of the emitter pivot.
/// Pure IEEE-754 sign flips — exact for every input.
#[must_use]
pub fn render_neg_translation__7b3d20(pos: &[f32; 3]) -> [f32; 3] {
    [-pos[0], -pos[1], -pos[2]]
}

#[cfg(test)]
mod tests_render_neg_translation__7b3d20 {
    use super::render_neg_translation__7b3d20 as neg;

    #[test]
    fn sign_flips_are_exact() {
        let r = neg(&[2.5, -0.0, f32::NAN]);
        assert_eq!(r[0].to_bits(), (-2.5f32).to_bits());
        assert_eq!(r[1].to_bits(), 0.0f32.to_bits());
        // FCHS flips only the sign bit of a NaN.
        assert_eq!(r[2].to_bits(), f32::NAN.to_bits() ^ 0x8000_0000);
    }
}

/// The 3x3 billboard-fixup transform of `CParticleEmitter::Render` (0x7b41a5..0x7b4243).
///
/// Each of the four fixup rows is rotated by the
/// billboard matrix's 3x3 block. Per component the x87 chain runs wide and
/// narrows once, with the exact stock sum trees:
/// `out.x = (m4·y + m8·z) + m0·x`, `out.y = (m5·y + m1·x) + m9·z`,
/// `out.z = (m6·y + m10·z) + m2·x`. `m` is the 16-float billboard matrix
/// (0xcf5888); `rows` is the 12-float fixup table (0xcf5af8).
#[must_use]
pub fn render_billboard_fixup__7b3d20(m: &[f32; 16], rows: &[f32; 12]) -> [f32; 12] {
    let mut out = [0f32; 12];
    for r in 0..4 {
        let x = f64::from(rows[r * 3]);
        let y = f64::from(rows[r * 3 + 1]);
        let z = f64::from(rows[r * 3 + 2]);
        out[r * 3] =
            super::f64_to_f32((f64::from(m[4]) * y + f64::from(m[8]) * z) + f64::from(m[0]) * x);
        out[r * 3 + 1] =
            super::f64_to_f32((f64::from(m[5]) * y + f64::from(m[1]) * x) + f64::from(m[9]) * z);
        out[r * 3 + 2] =
            super::f64_to_f32((f64::from(m[6]) * y + f64::from(m[10]) * z) + f64::from(m[2]) * x);
    }
    out
}

#[cfg(test)]
mod tests_render_billboard_fixup__7b3d20 {
    use super::render_billboard_fixup__7b3d20 as fixup;

    const IDENT: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];

    /// The stock one-time fixup table (0xcf5af8).
    const ROWS: [f32; 12] = [
        -1.0, 1.0, 0.0, -1.0, -1.0, 0.0, 1.0, 1.0, 0.0, 1.0, -1.0, 0.0,
    ];

    #[test]
    fn identity_matrix_passes_rows_through() {
        let out = fixup(&IDENT, &ROWS);
        for i in 0..12 {
            assert_eq!(out[i].to_bits(), ROWS[i].to_bits());
        }
    }

    #[test]
    fn column_pick_matches_the_stock_tree() {
        // A permutation matrix swapping x/y exercises every term slot.
        let mut m = [0f32; 16];
        m[1] = 1.0;
        m[4] = 1.0;
        m[10] = 1.0;
        let out = fixup(&m, &ROWS);
        // out.x = m4*y = y; out.y = m1*x = x; out.z = m10*z = z.
        assert_eq!(out[0].to_bits(), ROWS[1].to_bits());
        assert_eq!(out[1].to_bits(), ROWS[0].to_bits());
        assert_eq!(out[2].to_bits(), ROWS[2].to_bits());
    }
}

/// Facing-vector normalize of `CParticleEmitter::Render` (0x7b426b..0x7b429c).
///
/// The 0x4549f0 squared magnitude returns a WIDE ST0 (modeled in f64 with the
/// `(x²+y²)+z²` sum order), `FSQRT` stays wide, and the gate is
/// `TEST AH,0x5; JNP` — the normalize is SKIPPED only on an ordered
/// `|len| < eps`; **NaN falls through to the normalize** (an `eps <= |len|`
/// form would have the wrong unordered arm). On the normalize
/// path `inv = one / len` stays wide and each component multiply narrows
/// once. Returns `None` when the stock skips.
#[must_use]
pub fn render_facing_normalize__7b3d20(v: &[f32; 3], eps: f32, one: f32) -> Option<[f32; 3]> {
    let x = f64::from(v[0]);
    let y = f64::from(v[1]);
    let z = f64::from(v[2]);
    let len = ((x * x + y * y) + z * z).sqrt();
    if len.abs() < f64::from(eps) {
        return None;
    }
    let inv = f64::from(one) / len;
    Some([
        super::f64_to_f32(x * inv),
        super::f64_to_f32(y * inv),
        super::f64_to_f32(z * inv),
    ])
}

#[cfg(test)]
mod tests_render_facing_normalize__7b3d20 {
    use super::render_facing_normalize__7b3d20 as norm;

    const EPS: f32 = 1e-4;

    #[test]
    fn unit_result_for_a_plain_vector() {
        let n = norm(&[3.0, 0.0, 4.0], EPS, 1.0).unwrap();
        assert_eq!(n[0].to_bits(), 0.6f32.to_bits());
        assert_eq!(n[1].to_bits(), 0.0f32.to_bits());
        assert_eq!(n[2].to_bits(), 0.8f32.to_bits());
    }

    #[test]
    fn tiny_vector_skips() {
        assert!(norm(&[1e-6, 0.0, 0.0], EPS, 1.0).is_none());
    }

    #[test]
    fn nan_takes_the_normalize_path() {
        // Unordered |len| < eps is FALSE -> stock falls through and divides.
        let n = norm(&[f32::NAN, 0.0, 0.0], EPS, 1.0).unwrap();
        assert!(n[0].is_nan());
    }
}

/// Budget product of `Node::DistributeScaledBudget` (0x7b1dd9 and the per-child copy at 0x7b1e13).
///
/// `__ftol(a × b × K)` — two emitter f32
/// fields multiplied wide, scaled by the live K global, truncated by the
/// CRT `__ftol`; the caller keeps EAX (the low 32 bits).
#[must_use]
pub fn distribute_budget_count__7b1dd0(a: f32, b: f32, k: f32) -> u32 {
    let wide = f64::from(a) * f64::from(b) * f64::from(k);
    crate::math::misc::ftol__40a2b0(wide) as u32
}

#[cfg(test)]
mod tests_distribute_budget_count__7b1dd0 {
    use super::distribute_budget_count__7b1dd0 as count;

    #[test]
    fn truncates_toward_zero() {
        assert_eq!(count(1.45, 2.0, 1.0), 2); // 2.9 → 2
        assert_eq!(count(3.0, 0.5, 2.0), 3); // exact 3.0
    }

    #[test]
    fn negative_product_wraps_like_eax() {
        // __ftol(-2.9) = -2; EAX keeps the two's-complement low 32.
        assert_eq!(count(-1.45, 2.0, 1.0), (-2i32) as u32);
    }

    #[test]
    fn zero_scale_kills_the_budget() {
        assert_eq!(count(123.0, 456.0, 0.0), 0);
    }
}

/// Uniform-scale tail of `CObjectPlacement::SetRelativeTransform` (0x7b5219).
///
/// `|row0|` of the incoming transform. The x87 chain squares and
/// sums entirely wide as `(x·x + y·y) + z·z`, takes `FSQRT`, and narrows
/// ONCE at the `+0x264` store — modeled in f64 like the 0x7b5230 speed
/// chain (each f32 product is exact in f64; the sqrt's double rounding is
/// innocuous, 53 >= 2*24+2).
#[must_use]
pub fn set_relative_transform_scale__7b5160(row0: &[f32; 3]) -> f32 {
    let x = f64::from(row0[0]);
    let y = f64::from(row0[1]);
    let z = f64::from(row0[2]);
    super::f64_to_f32(((x * x + y * y) + z * z).sqrt())
}

#[cfg(test)]
mod tests_set_relative_transform_scale__7b5160 {
    use super::set_relative_transform_scale__7b5160 as scale;

    #[test]
    fn pythagorean_triple_is_exact() {
        assert_eq!(scale(&[3.0, 4.0, 0.0]).to_bits(), 5.0f32.to_bits());
        assert_eq!(scale(&[0.0, -3.0, 4.0]).to_bits(), 5.0f32.to_bits());
    }

    #[test]
    fn unit_row_is_one() {
        assert_eq!(scale(&[1.0, 0.0, 0.0]).to_bits(), 1.0f32.to_bits());
    }

    #[test]
    fn narrows_once_from_the_wide_sum() {
        // 0.1f32 squared three ways: the f64 sum differs from a per-step
        // f32 chain; pin the single-narrowing result.
        let v = 0.1f32;
        let w = f64::from(v);
        let expect = (((w * w + w * w) + w * w).sqrt()) as f32;
        assert_eq!(scale(&[v, v, v]).to_bits(), expect.to_bits());
    }

    #[test]
    fn nan_component_propagates() {
        assert!(scale(&[f32::NAN, 1.0, 1.0]).is_nan());
    }
}

/// Camera-relative distance for `CParticleEmitter::UpdateWithSubsteps` (@0x7b5230, prologue).
///
/// Per-component `FSUB`+`FSTP` narrows each delta,
/// squared magnitude via the shared 0x4549f0 kernel, then `FSQRT` with one
/// narrow into the `_DAT_00cf58e8` store (double rounding through f64 is
/// innocuous for a square root, 53 >= 2*24+2).
pub fn substep_camera_distance__7b5230(p: [f32; 3], q: [f32; 3]) -> f32 {
    let d = [
        super::f64_to_f32(f64::from(p[0]) - f64::from(q[0])),
        super::f64_to_f32(f64::from(p[1]) - f64::from(q[1])),
        super::f64_to_f32(f64::from(p[2]) - f64::from(q[2])),
    ];
    let sq = crate::math::vector::c3_vector__squared_magnitude__4549f0(&d);
    super::f64_to_f32(f64::from(sq).sqrt())
}

/// Velocity arm of `CParticleEmitter::UpdateWithSubsteps` (flag 0x40000, 0x7b530e–0x7b53c0).
///
/// Returns the narrowed frame deltas `cur − snap` (stored to `+0x26c` BEFORE
/// scaling) and the clamp-scaled copies that overwrite them.
///
/// The speed chain runs wide end to end: the three narrowed deltas re-load,
/// square, and sum as `(x·x + y·y) + z·z`, then `sqrt/dt·s1 + s0` with NO
/// intermediate store. The clamp is the FNSTSW pair at 0x7b5383/0x7b539a:
/// `speed < lo` (ordered) → `lo`; else `speed < hi` (ordered) → `speed`;
/// else → `hi`. **NaN speed lands on `hi`** — both parity branches fall
/// through on unordered (a form that keeps `lo` on NaN would be
/// wrong). The chosen factor stays wide (when it is `speed`, the 80-bit
/// value, never a narrowed copy) and each component multiply narrows once.
pub fn substep_velocity_scale__7b5230(
    cur: [f32; 3],
    snap: [f32; 3],
    dt: f32,
    s1: f32,
    s0: f32,
    lo: f32,
    hi: f32,
) -> ([f32; 3], [f32; 3]) {
    let d = [
        super::f64_to_f32(f64::from(cur[0]) - f64::from(snap[0])),
        super::f64_to_f32(f64::from(cur[1]) - f64::from(snap[1])),
        super::f64_to_f32(f64::from(cur[2]) - f64::from(snap[2])),
    ];
    let x = f64::from(d[0]);
    let y = f64::from(d[1]);
    let z = f64::from(d[2]);
    let speed = (x * x + y * y + z * z).sqrt() / f64::from(dt) * f64::from(s1) + f64::from(s0);
    let k = if speed < f64::from(lo) {
        f64::from(lo)
    } else if speed < f64::from(hi) {
        speed
    } else {
        // speed >= hi, and NaN: both FNSTSW branches fall through here.
        f64::from(hi)
    };
    let scaled = [
        super::f64_to_f32(k * x),
        super::f64_to_f32(k * y),
        super::f64_to_f32(k * z),
    ];
    (d, scaled)
}

/// Drift-window accumulate of `CParticleEmitter::UpdateWithSubsteps`.
///
/// (Flag 0x400, 0x7b53ce–0x7b53e8.) Returns the narrowed `+0x254` store value
/// and `Some(ratio)` when the window fires.
///
/// The `FST` keeps the WIDE sum on the stack: the `> window` gate and the
/// `ratio = acc/window` divide both consume the wide value, not the narrowed
/// store (a sub-f32-ulp overflow still fires). Gate polarity is
/// `TEST AH,0x41; JNZ skip` — skip on `acc <= window` AND on NaN.
pub fn substep_drift_accum__7b5230(dt: f32, acc_old: f32, window: f32) -> (f32, Option<f64>) {
    let acc = f64::from(dt) + f64::from(acc_old);
    let stored = super::f64_to_f32(acc);
    if acc > f64::from(window) {
        (stored, Some(acc / f64::from(window)))
    } else {
        (stored, None)
    }
}

/// Drift-scale tail (0x7b5467–0x7b54cd).
///
/// Narrowed deltas `cur − snap` (stored
/// to `+0x258` before scaling) and the scaled copies, with
/// `k = (one/ratio) · k_src` folded wide (`FDIVR`/`FMUL` straight off the
/// stack) and one narrow per component multiply.
pub fn substep_drift_scale__7b5230(
    cur: [f32; 3],
    snap: [f32; 3],
    ratio: f64,
    k_src: f32,
    one: f32,
) -> ([f32; 3], [f32; 3]) {
    let d = [
        super::f64_to_f32(f64::from(cur[0]) - f64::from(snap[0])),
        super::f64_to_f32(f64::from(cur[1]) - f64::from(snap[1])),
        super::f64_to_f32(f64::from(cur[2]) - f64::from(snap[2])),
    ];
    let k = f64::from(one) / ratio * f64::from(k_src);
    let scaled = [
        super::f64_to_f32(k * f64::from(d[0])),
        super::f64_to_f32(k * f64::from(d[1])),
        super::f64_to_f32(k * f64::from(d[2])),
    ];
    (d, scaled)
}

#[cfg(test)]
mod tests_update_with_substeps__7b5230 {
    use super::{
        substep_camera_distance__7b5230 as cam_dist, substep_drift_accum__7b5230 as drift_accum,
        substep_drift_scale__7b5230 as drift_scale, substep_velocity_scale__7b5230 as vel_scale,
    };

    #[test]
    fn camera_distance_known() {
        let d = cam_dist([4.0, 6.0, 3.0], [1.0, 2.0, 3.0]);
        assert_eq!(d.to_bits(), 5.0f32.to_bits());
    }

    #[test]
    fn velocity_deltas_and_midband_speed() {
        // deltas (3,4,0) -> |d|=5, dt=1, s1=0.1, s0=0 -> speed 0.5 in (lo,hi).
        let (d, scaled) = vel_scale([4.0, 6.0, 1.0], [1.0, 2.0, 1.0], 1.0, 0.1, 0.0, 0.0, 1.0);
        assert_eq!(d, [3.0, 4.0, 0.0]);
        let k = 0.5f64;
        assert_eq!(scaled[0].to_bits(), ((k * 3.0) as f32).to_bits());
        assert_eq!(scaled[1].to_bits(), ((k * 4.0) as f32).to_bits());
        assert_eq!(scaled[2].to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn velocity_clamp_polarities() {
        // Below lo -> lo.
        let (_, s) = vel_scale([1.0, 0.0, 0.0], [0.0; 3], 100.0, 1.0, 0.0, 0.5, 2.0);
        assert_eq!(s[0].to_bits(), 0.5f32.to_bits());
        // Above hi -> hi.
        let (_, s) = vel_scale([8.0, 0.0, 0.0], [0.0; 3], 1.0, 1.0, 0.0, 0.5, 2.0);
        assert_eq!(s[0].to_bits(), (8.0f32 * 2.0).to_bits());
        // NaN speed (dt = 0 -> sqrt(0)/0 = NaN via 0/0? use s0 = NaN) -> hi,
        // NOT lo: both FNSTSW parity branches fall through on unordered.
        let (_, s) = vel_scale([4.0, 0.0, 0.0], [0.0; 3], 1.0, 1.0, f32::NAN, 0.5, 2.0);
        assert_eq!(s[0].to_bits(), (4.0f32 * 2.0).to_bits());
    }

    #[test]
    fn drift_gate_uses_wide_sum() {
        // acc = 1 + 2^-30 exceeds the window in f64 even though the f32
        // store rounds back to exactly 1.0 — the gate must fire.
        let tiny = f32::from_bits(0x3080_0000); // 2^-30
        let (stored, fired) = drift_accum(tiny, 1.0, 1.0);
        assert_eq!(stored.to_bits(), 1.0f32.to_bits());
        let ratio = fired.expect("wide compare must fire");
        assert!(ratio > 1.0 && ratio < 1.0 + 1.0e-8);
    }

    #[test]
    fn drift_gate_skips_on_equal_and_nan() {
        assert!(drift_accum(0.0, 1.0, 1.0).1.is_none());
        let (stored, fired) = drift_accum(f32::NAN, 1.0, 1.0);
        assert!(stored.is_nan());
        assert!(fired.is_none());
    }

    #[test]
    fn drift_scale_known() {
        // ratio 2, k_src 4 -> k = 2; deltas (1,2,3) doubled.
        let (d, s) = drift_scale([2.0, 4.0, 6.0], [1.0, 2.0, 3.0], 2.0, 4.0, 1.0);
        assert_eq!(d, [1.0, 2.0, 3.0]);
        assert_eq!(s[0].to_bits(), 2.0f32.to_bits());
        assert_eq!(s[1].to_bits(), 4.0f32.to_bits());
        assert_eq!(s[2].to_bits(), 6.0f32.to_bits());
    }
}
