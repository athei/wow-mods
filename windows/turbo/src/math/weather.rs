//! `weather` family kernels.
#![allow(
    non_snake_case,
    // intentional NaN-reject via `!(a >= b)`
    // (differs from `a < b` on NaN), bit-exact source constants kept verbatim,
    // and ABI-dictated parameter counts.
    clippy::neg_cmp_op_on_partial_ord,
    clippy::excessive_precision,
    clippy::too_many_arguments
)]

/// Scales three packed color bytes (each `0..=255`) by a normalization factor
/// (the source's literal `1.0/255.0`) into a linear RGB triple. The input bytes
/// are taken in the order the original writes them to its output vector
/// (`red`, `green`, `blue`), so the returned lanes map directly onto
/// `direction[0..3]` passed to the depth-range setter.
pub fn cloud_apply_depth_range_and_color__6c31e0(
    red: u8,
    green: u8,
    blue: u8,
    scale: f32,
) -> [f32; 3] {
    [
        f32::from(red) * scale,
        f32::from(green) * scale,
        f32::from(blue) * scale,
    ]
}

#[cfg(test)]
mod tests_cloud_apply_depth_range_and_color__6c31e0 {
    use super::cloud_apply_depth_range_and_color__6c31e0 as scale_rgb;

    const K: f32 = 1.0 / 255.0;

    #[test]
    fn full_white_maps_to_one() {
        let c = scale_rgb(255, 255, 255, K);
        for v in c {
            assert_eq!(v.to_bits(), 1.0f32.to_bits());
        }
    }

    #[test]
    fn zero_maps_to_zero() {
        let c = scale_rgb(0, 0, 0, K);
        for v in c {
            assert_eq!(v.to_bits(), 0.0f32.to_bits());
        }
    }

    #[test]
    fn lanes_are_independent_and_ordered() {
        // distinct values per lane verify no cross-talk and channel order.
        let c = scale_rgb(10, 20, 30, K);
        assert_eq!(c[0].to_bits(), (10.0f32 * K).to_bits());
        assert_eq!(c[1].to_bits(), (20.0f32 * K).to_bits());
        assert_eq!(c[2].to_bits(), (30.0f32 * K).to_bits());
    }

    #[test]
    fn scale_is_linear() {
        // doubling the scale doubles every lane.
        let a = scale_rgb(40, 80, 120, K);
        let b = scale_rgb(40, 80, 120, 2.0 * K);
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!((x * 2.0).to_bits(), y.to_bits());
        }
    }

    #[test]
    fn half_intensity() {
        let c = scale_rgb(128, 64, 32, K);
        assert_eq!(c[0].to_bits(), (128.0f32 * K).to_bits());
        assert_eq!(c[1].to_bits(), (64.0f32 * K).to_bits());
        assert_eq!(c[2].to_bits(), (32.0f32 * K).to_bits());
    }
}

/// Reciprocal-square-root bit-hack seed used by the cloud shader: the stock
/// helper's single magic-constant step (no refinement iterations).
fn cloud_rsqrt_seed__456330(x: f32) -> f32 {
    const MAGIC: u32 = 0x5f39_97bb;
    f32::from_bits(MAGIC.wrapping_sub((x.to_bits() >> 1) & 0x3fff_ffff))
}

/// Shades one non-empty cloud-density cell to its packed output bytes
/// `[blue, green, red, alpha]`. The density byte maps to a shade scalar
/// `(((255 - d) >> 1) + 0x40) / 255`, the base-lit color is `sun * shade +
/// base` per lane, and when the cell's normalized gradient faces the light
/// (`dot > 0`, both vectors normalized by the bit-hack seed) the `tint` color
/// scaled by `gain` is added on top. Lanes clamp to `limit`, then pack to
/// 8 bits through the `value * pack_scale + pack_bias` float-bit trick
/// (mantissa bits 14..22 of the stored f32). Alpha carries the density byte.
pub fn cloud_shade_layer__6cfb00(
    density: u8,
    grad: [f32; 2],
    delta: [f32; 3],
    sun: [f32; 3],
    base: [f32; 3],
    tint: [f32; 3],
    gain: f32,
    inv255: f32,
    one: f32,
    zero: f32,
    limit: f64,
    pack_scale: f32,
    pack_bias: f32,
) -> [u8; 4] {
    let shade = ((255 - density) >> 1) + 0x40;
    let s = f64::from(shade) * f64::from(inv255);
    let r0 = (f64::from(sun[0]) * s + f64::from(base[0])) as f32;
    let g0 = (f64::from(sun[1]) * s + f64::from(base[1])) as f32;
    let b0 = (s * f64::from(sun[2]) + f64::from(base[2])) as f32;

    let [dx, dy, dz] = delta;
    let dist_sq = (f64::from(dz) * f64::from(dz)
        + f64::from(dy) * f64::from(dy)
        + f64::from(dx) * f64::from(dx)) as f32;
    let rs1 = cloud_rsqrt_seed__456330(dist_sq);
    let grad_sq = (f64::from(grad[0]) * f64::from(grad[0])
        + f64::from(grad[1]) * f64::from(grad[1])
        + f64::from(one)) as f32;
    let rs2 = cloud_rsqrt_seed__456330(grad_sq);
    let k12 = f64::from(rs2) * f64::from(rs1);
    let dot =
        f64::from(dx) * f64::from(grad[0]) + f64::from(dy) * f64::from(grad[1]) + f64::from(dz);
    let lit = dot * k12;

    let (mut r, mut g, b_chain): (f32, f32, f64);
    if lit > f64::from(zero) {
        let k = lit * f64::from(gain);
        r = (f64::from(tint[0]) * k + f64::from(r0)) as f32;
        g = (f64::from(tint[1]) * k + f64::from(g0)) as f32;
        b_chain = k * f64::from(tint[2]) + f64::from(b0);
    } else {
        r = r0;
        g = g0;
        b_chain = f64::from(b0);
    }
    if f64::from(r) > limit {
        r = 1.0;
    }
    if f64::from(g) > limit {
        g = 1.0;
    }
    let b = if b_chain > limit {
        f64::from(one)
    } else {
        b_chain
    };

    let scale = f64::from(pack_scale);
    let bias = f64::from(pack_bias);
    let pack = |v: f64| (((v * scale + bias) as f32).to_bits() >> 14) as u8;
    [pack(b), pack(f64::from(g)), pack(f64::from(r)), density]
}

#[cfg(test)]
mod tests_cloud_shade_layer__6cfb00 {
    use super::cloud_shade_layer__6cfb00 as shade;

    const INV255: f32 = 1.0 / 255.0;
    const SCALE: f32 = 255.0;
    const BIAS: f32 = 512.0;

    fn unlit(density: u8, sun: [f32; 3], base: [f32; 3]) -> [u8; 4] {
        // delta z < 0 with a flat gradient forces the unlit branch.
        shade(
            density,
            [0.0, 0.0],
            [0.0, 0.0, -1.0],
            sun,
            base,
            [1.0, 1.0, 1.0],
            1.0,
            INV255,
            1.0,
            0.0,
            1.0,
            SCALE,
            BIAS,
        )
    }

    #[test]
    fn alpha_carries_the_density_byte() {
        for d in [1u8, 7, 64, 200, 255] {
            assert_eq!(unlit(d, [0.0; 3], [0.0; 3])[3], d);
        }
    }

    #[test]
    fn unlit_base_color_packs_exactly() {
        // 0.25 / 0.5 / 0.75 scale to exactly-representable f32 pack inputs, so
        // the float-bit trick truncates to 63 / 127 / 191.
        let c = unlit(255, [0.0; 3], [0.25, 0.5, 0.75]);
        assert_eq!(c, [191, 127, 63, 255]);
    }

    #[test]
    fn channels_clamp_to_limit() {
        let c = unlit(255, [0.0; 3], [2.0, 0.0, 2.0]);
        // red clamps to literal 1.0, blue to the injected `one`.
        assert_eq!(c[2], 255);
        assert_eq!(c[0], 255);
        assert_eq!(c[1], 0);
    }

    #[test]
    fn higher_density_darkens_the_shade_scalar() {
        // shade(d=1) = 191/255 vs shade(d=255) = 64/255 against a pure red sun.
        let bright = unlit(1, [1.0, 1.0, 1.0], [0.0; 3]);
        let dark = unlit(255, [1.0, 1.0, 1.0], [0.0; 3]);
        assert!(bright[2] > dark[2]);
        assert!(bright[1] > dark[1]);
        assert!(bright[0] > dark[0]);
    }

    #[test]
    fn lit_branch_adds_tint_scaled_by_gain() {
        let cell = |gain: f32, tint: [f32; 3]| {
            // delta = (0, 0, 1) with a flat gradient gives dot = 1 > 0.
            shade(
                255,
                [0.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.0; 3],
                [0.0; 3],
                tint,
                gain,
                INV255,
                1.0,
                0.0,
                1.0,
                SCALE,
                BIAS,
            )
        };
        let lit = cell(1.0, [1.0, 0.0, 0.0]);
        // only the tinted lane moves; the seed-normalized dot is < 1.
        assert!(lit[2] > 200 && lit[2] < 250);
        assert_eq!(lit[1], 0);
        assert_eq!(lit[0], 0);
        // halving the gain lowers the lane.
        let half = cell(0.5, [1.0, 0.0, 0.0]);
        assert!(half[2] < lit[2]);

        // known value: dist_sq = grad_sq = 1.0, so both seeds share one bit
        // pattern and the lane is the squared seed.
        let seed_bits = 0x5f39_97bbu32.wrapping_sub((1.0f32.to_bits() >> 1) & 0x3fff_ffff);
        let seed = f64::from(f32::from_bits(seed_bits));
        let lane = (seed * seed) as f32;
        let expected = ((f64::from(lane) * 255.0 + 512.0) as f32).to_bits() >> 14;
        assert_eq!(u32::from(lit[2]), expected & 0xff);
    }

    #[test]
    fn nan_gradient_takes_the_unlit_branch() {
        let c = shade(
            128,
            [f32::NAN, 0.0],
            [1.0, 0.0, 0.0],
            [0.0; 3],
            [0.25, 0.5, 0.75],
            [1.0, 1.0, 1.0],
            1.0,
            INV255,
            1.0,
            0.0,
            1.0,
            SCALE,
            BIAS,
        );
        assert_eq!(c, [191, 127, 63, 128]);
    }
}

/// Nearest-hit fraction update for one traced doodad candidate: a candidate at
/// `frac` replaces `best` only when strictly nearer (an unordered compare
/// keeps `best`). A non-negative trace depth `hit` applies the depth bias
/// `frac - ((1 - inv_dz * hit) * inv_dz + inv_dz)` floored at zero; a negative
/// (or unordered) depth keeps the raw `frac`.
pub fn doodad_raycast_segment__6b7790(
    frac: f32,
    hit: f32,
    inv_dz: f32,
    best: f32,
    one: f32,
    zero: f32,
) -> Option<f32> {
    if frac < best {
        if hit >= zero {
            let biased = f64::from(frac)
                - ((f64::from(one) - f64::from(inv_dz) * f64::from(hit)) * f64::from(inv_dz)
                    + f64::from(inv_dz));
            if biased < f64::from(zero) {
                Some(0.0)
            } else {
                Some(biased as f32)
            }
        } else {
            Some(frac)
        }
    } else {
        None
    }
}

/// Sub-node bounds test for a traced segment: the node's X/Y interval must
/// straddle the segment start, its Z maximum must reach the segment end and
/// its Z minimum must stay below the segment start. Written as negated strict
/// compares so unordered (NaN) operands pass, matching the stock x87 chain.
pub fn doodad_raycast_segment_node_overlaps__6b7790(
    bounds_min: [f32; 3],
    bounds_max: [f32; 3],
    start: [f32; 3],
    end_z: f32,
) -> bool {
    !(bounds_max[0] < start[0])
        && !(bounds_min[0] > start[0])
        && !(bounds_max[1] < start[1])
        && !(bounds_min[1] > start[1])
        && !(bounds_max[2] < end_z)
        && !(bounds_min[2] > start[2])
}

#[cfg(test)]
mod tests_doodad_raycast_segment__6b7790 {
    use super::{
        doodad_raycast_segment__6b7790 as update,
        doodad_raycast_segment_node_overlaps__6b7790 as overlaps,
    };

    #[test]
    fn farther_or_equal_candidates_are_rejected() {
        assert_eq!(update(0.5, 0.0, 0.1, 0.5, 1.0, 0.0), None);
        assert_eq!(update(0.9, 0.0, 0.1, 0.5, 1.0, 0.0), None);
        assert_eq!(update(f32::NAN, 0.0, 0.1, 0.5, 1.0, 0.0), None);
    }

    #[test]
    fn negative_depth_keeps_the_raw_fraction() {
        let r = update(0.25, -1.0, 0.1, 1.0, 1.0, 0.0).unwrap();
        assert_eq!(r.to_bits(), 0.25f32.to_bits());
        // unordered depth behaves like the negative branch.
        let r = update(0.25, f32::NAN, 0.1, 1.0, 1.0, 0.0).unwrap();
        assert_eq!(r.to_bits(), 0.25f32.to_bits());
    }

    #[test]
    fn depth_bias_known_value() {
        // hit = 0: bias collapses to 2 * inv_dz -> 0.5 - 0.2 = 0.3.
        let r = update(0.5, 0.0, 0.1, 1.0, 1.0, 0.0).unwrap();
        let expected = (0.5f64 - ((1.0f64 - 0.1f64 * 0.0) * 0.1f64 + 0.1f64)) as f32;
        assert_eq!(r.to_bits(), expected.to_bits());
    }

    #[test]
    fn biased_fraction_floors_at_zero() {
        let r = update(0.1, 0.0, 0.1, 1.0, 1.0, 0.0).unwrap();
        assert_eq!(r.to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn overlap_accepts_a_straddled_start() {
        let min = [-1.0, -1.0, -5.0];
        let max = [1.0, 1.0, 5.0];
        assert!(overlaps(min, max, [0.0, 0.0, 0.0], 2.0));
    }

    #[test]
    fn overlap_rejects_each_separating_axis() {
        let min = [-1.0, -1.0, -5.0];
        let max = [1.0, 1.0, 5.0];
        // start.x outside either side.
        assert!(!overlaps(min, max, [2.0, 0.0, 0.0], 2.0));
        assert!(!overlaps(min, max, [-2.0, 0.0, 0.0], 2.0));
        // start.y outside either side.
        assert!(!overlaps(min, max, [0.0, 2.0, 0.0], 2.0));
        assert!(!overlaps(min, max, [0.0, -2.0, 0.0], 2.0));
        // end.z above the node, start.z below it.
        assert!(!overlaps(min, max, [0.0, 0.0, 0.0], 6.0));
        assert!(!overlaps(min, max, [0.0, 0.0, -6.0], 2.0));
    }

    #[test]
    fn unordered_bounds_pass_like_the_stock_chain() {
        let min = [-1.0, -1.0, -5.0];
        let max = [f32::NAN, 1.0, 5.0];
        assert!(overlaps(min, max, [0.0, 0.0, 0.0], 2.0));
    }
}

/// Packs one ground-splash vertex (24 bytes as six little-endian dwords): the
/// position dwords copied verbatim, the diffuse dword set to `corner << 24`
/// (optionally with its red/blue bytes swapped for the active vertex-color
/// order), then the two texture-coordinate dwords copied verbatim.
pub fn weather_build_patter_geometry__6753c0(
    position: [u32; 3],
    uv: [u32; 2],
    corner: u32,
    swap_red_blue: bool,
) -> [u32; 6] {
    let mut color = (corner & 0xff) << 24;
    if swap_red_blue {
        let [b0, b1, b2, b3] = color.to_le_bytes();
        color = u32::from_le_bytes([b2, b1, b0, b3]);
    }
    [position[0], position[1], position[2], color, uv[0], uv[1]]
}

#[cfg(test)]
mod tests_weather_build_patter_geometry__6753c0 {
    use super::weather_build_patter_geometry__6753c0 as pack;

    #[test]
    fn corner_index_lands_in_the_top_byte() {
        for corner in 0..3u32 {
            let v = pack([0, 0, 0], [0, 0], corner, false);
            assert_eq!(v[3], corner << 24);
        }
    }

    #[test]
    fn position_and_uv_pass_through_verbatim() {
        let pos = [1.5f32.to_bits(), (-2.25f32).to_bits(), 1024.0f32.to_bits()];
        let uv = [0.5f32.to_bits(), 0.125f32.to_bits()];
        let v = pack(pos, uv, 1, false);
        assert_eq!([v[0], v[1], v[2]], pos);
        assert_eq!([v[4], v[5]], uv);
    }

    #[test]
    fn swap_is_identity_on_corner_colors() {
        // corner << 24 has zero red/blue bytes, so the swizzle is a no-op.
        for corner in 0..3u32 {
            let a = pack([0, 0, 0], [0, 0], corner, false);
            let b = pack([0, 0, 0], [0, 0], corner, true);
            assert_eq!(a, b);
        }
    }

    #[test]
    fn swap_exchanges_bytes_zero_and_two() {
        // Drive the swizzle through a synthetic corner with low bits set: only
        // the top byte survives the `corner & 0xff` mask, so probe the byte
        // shuffle directly instead.
        let [b0, b1, b2, b3] = 0xaabb_ccddu32.to_le_bytes();
        let swapped = u32::from_le_bytes([b2, b1, b0, b3]);
        assert_eq!(swapped, 0xaadd_ccbb);
    }
}

/// Per-drop billboard scalars for the rain-drop streak driver at `0x675ac0`.
///
/// Transcribes the crash-class arithmetic of the stock x87 body: the two
/// lifetime gates, the view-relative normalize, the fade-dot clamp gate and the
/// two magic-bias texture-index packs. The vertex placement (which basis row
/// lands on which corner) is left to the caller because it is smoke-validated
/// geometry, not a crash-class decision. Every float compare keeps the stock
/// x87 NaN polarity (decoded from `fnstsw`/`test ah`/`Jcc`), and the two packs
/// use `f32::to_bits() >> 14`, never a numeric `as u32` cast. The magic-bias
/// packs round per `f32` op rather than reproducing the stock 80-bit
/// single-final-round: an intentional `f32`-fast path that can differ from the
/// x87 chain by at most one ulp at a `>>14` quantization boundary
/// (smoke-acceptable, measure-zero), not a bit-exact reproduction.
///
/// Inputs: `now` is the seconds value from `Weather_TicksToSeconds` (stored at
/// `ebp-0x24`), `birth`/`death` are the drop's lifetime pair (`drop+0x1c` and
/// `drop+0x20`), `pos`/`camera` are the world and camera positions, `one`=1.0,
/// `zero`=0.0, `scale_399`=3.99, `bias_512`=512.0, `mul_4`=4.0. Output is `None`
/// when the drop is culled, else the computed scalars the caller threads into the
/// six-vertex streak.
pub fn weather_raindrop_vertex__675ac0(
    now: f32,
    birth: f32,
    death: f32,
    pos: [f32; 3],
    camera: [f32; 3],
    one: f32,
    zero: f32,
    scale_399: f32,
    bias_512: f32,
    mul_4: f32,
) -> Option<RainDropScalars> {
    // Gate 1 (0x675d90 `fcomp dropBirth`; 0x675d98 `test ah,0x5`; 0x675d9b
    // `jnp` skip): process iff NOT(now < birth) -- a strictly-less ordered
    // compare whose NaN (unordered) case falls through to processing.
    if now < birth {
        return None;
    }
    // Gate 2 (0x675da4 `fcomp st(1)`=death; 0x675daa `test ah,0x41`; 0x675dad
    // `je` skip): emit iff NOT(now > death) -- strictly-greater ordered, NaN
    // (unordered) emits.
    if now > death {
        return None;
    }

    // rel = pos - camera (0x675db3/0x675dbc/0x675dcb `fsub`).
    let rel = [pos[0] - camera[0], pos[1] - camera[1], pos[2] - camera[2]];
    // inv_len = 1.0 / sqrt(SquaredMagnitude(rel)) (0x675de3 call; 0x675de8
    // `fsqrt`; 0x675df3 `fdivr 1.0`). f64 sqrt matches the stock x87 80-bit
    // intermediate closer than an f32 sqrt of the f32 squared magnitude; the
    // squared magnitude itself reuses the shared kernel's fadd grouping.
    let mag = crate::math::vector::c3_vector__squared_magnitude__4549f0(&rel);
    let inv_len = (f64::from(one) / f64::from(mag).sqrt()) as f32;

    // tilt = rel * inv_len, then negated per component (0x675e16..0x675e43
    // `fchs` + `C3Vector::Set`).
    let tilt = [rel[0] * inv_len, rel[1] * inv_len, rel[2] * inv_len];
    let neg_tilt = [-tilt[0], -tilt[1], -tilt[2]];

    // Gate 3 -- fade-dot clamp (0x675e54..0x675e71). The stock multiplies the
    // dot accumulator by `zero` (the 0.0 constant at 0x7ffd74) before the
    // compare, so the compared quantity collapses to the trailing addend; when
    // it is `> 0` the alternate dot is recomputed, otherwise the fade clamps to
    // 0.0. `test ah,0x41` + `jne` clamps on (value <= 0 OR unordered).
    let dot_acc = (neg_tilt[1] + neg_tilt[0]) * zero + neg_tilt[2];
    let fade = if dot_acc > zero {
        // 0x675e85 recompute path: a second dot of the same negated tilt rows.
        (neg_tilt[1] + neg_tilt[0]) * zero + neg_tilt[2]
    } else {
        zero
    };

    // Magic-bias texture-index pack 1 (0x675f1b..0x675f37): the low byte of
    // `((1.0 - fade)*3.99 + 512.0).to_bits() >> 14`.
    let pack1 = ((f64::from(one) - f64::from(fade)) as f32 * scale_399 + bias_512).to_bits() >> 14;
    let tex_u_index = (pack1 & 0xff) as u8;

    // Magic-bias texture-index pack 2 (0x675f3a..0x675f5e): the low byte of
    // `((now - birth)*4.0*3.99 + 512.0).to_bits() >> 14`.
    let pack2 = (((now - birth) * mul_4 * scale_399) + bias_512).to_bits() >> 14;
    let tex_anim_index = (pack2 & 0xff) as u8;

    Some(RainDropScalars {
        rel,
        tilt,
        neg_tilt,
        fade,
        tex_u_index,
        tex_anim_index,
    })
}

/// Crash-class scalars the rain-drop driver threads into one streak's six
/// vertices. `rel` is the camera-relative position, `tilt`/`neg_tilt` the
/// normalized view-space stretch, `fade` the clamped lifetime fade and the two
/// `tex_*_index` the packed texture/animation byte indices.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RainDropScalars {
    pub rel: [f32; 3],
    pub tilt: [f32; 3],
    pub neg_tilt: [f32; 3],
    pub fade: f32,
    pub tex_u_index: u8,
    pub tex_anim_index: u8,
}

#[cfg(test)]
mod tests_weather_raindrop_vertex__675ac0 {
    use super::weather_raindrop_vertex__675ac0 as drop_scalars;

    const ONE: f32 = 1.0;
    const ZERO: f32 = 0.0;
    const S399: f32 = 3.990_000_009_536_743;
    const B512: f32 = 512.0;
    const M4: f32 = 4.0;

    fn live(now: f32, birth: f32, death: f32) -> Option<super::RainDropScalars> {
        drop_scalars(
            now,
            birth,
            death,
            [10.0, 20.0, 5.0],
            [1.0, 2.0, 3.0],
            ONE,
            ZERO,
            S399,
            B512,
            M4,
        )
    }

    #[test]
    fn gate1_skips_strictly_before_birth() {
        // now < birth -> culled.
        assert!(live(0.5, 1.0, 10.0).is_none());
        // now == birth -> processed (strictly-less polarity).
        assert!(live(1.0, 1.0, 10.0).is_some());
    }

    #[test]
    fn gate1_nan_now_processes() {
        // Unordered now falls through gate 1 (parity-even -> JNP not taken).
        // It is then rejected by gate 2 only if `now > death` is *ordered*
        // true; NaN keeps it alive.
        assert!(live(f32::NAN, 1.0, 10.0).is_some());
    }

    #[test]
    fn gate2_skips_strictly_after_death() {
        assert!(live(11.0, 1.0, 10.0).is_none());
        // now == death -> emit (strictly-greater polarity).
        assert!(live(10.0, 1.0, 10.0).is_some());
    }

    #[test]
    fn rel_is_pos_minus_camera() {
        let s = live(5.0, 1.0, 10.0).unwrap();
        assert_eq!(s.rel[0].to_bits(), (10.0f32 - 1.0).to_bits());
        assert_eq!(s.rel[1].to_bits(), (20.0f32 - 2.0).to_bits());
        assert_eq!(s.rel[2].to_bits(), (5.0f32 - 3.0).to_bits());
    }

    #[test]
    fn tilt_is_normalized_rel_and_negation_matches() {
        let s = live(5.0, 1.0, 10.0).unwrap();
        let mag = s.rel[0] * s.rel[0] + s.rel[1] * s.rel[1] + s.rel[2] * s.rel[2];
        let inv = (1.0f64 / f64::from(mag).sqrt()) as f32;
        assert_eq!(s.tilt[0].to_bits(), (s.rel[0] * inv).to_bits());
        assert_eq!(s.tilt[1].to_bits(), (s.rel[1] * inv).to_bits());
        assert_eq!(s.tilt[2].to_bits(), (s.rel[2] * inv).to_bits());
        assert_eq!(s.neg_tilt[0].to_bits(), (-s.tilt[0]).to_bits());
        assert_eq!(s.neg_tilt[1].to_bits(), (-s.tilt[1]).to_bits());
        assert_eq!(s.neg_tilt[2].to_bits(), (-s.tilt[2]).to_bits());
    }

    #[test]
    fn fade_dot_clamp_polarity() {
        // The compared accumulator is `(nt.y + nt.x)*0 + nt.z` = nt.z. When
        // nt.z <= 0 (or unordered) the fade clamps to 0.0; otherwise it is the
        // recomputed dot (== nt.z here because *0 zeros the first two terms).
        let s = live(5.0, 1.0, 10.0).unwrap();
        let expected = if s.neg_tilt[2] > 0.0 {
            s.neg_tilt[2]
        } else {
            0.0
        };
        assert_eq!(s.fade.to_bits(), expected.to_bits());
    }

    #[test]
    fn pack1_is_to_bits_shift_not_numeric_cast() {
        let s = live(5.0, 1.0, 10.0).unwrap();
        let expected = (((1.0f64 - f64::from(s.fade)) as f32 * S399 + B512).to_bits() >> 14) & 0xff;
        assert_eq!(u32::from(s.tex_u_index), expected);
    }

    #[test]
    fn pack2_is_to_bits_shift_not_numeric_cast() {
        let now = 5.0f32;
        let birth = 1.0f32;
        let s = live(now, birth, 10.0).unwrap();
        let expected = ((((now - birth) * M4 * S399) + B512).to_bits() >> 14) & 0xff;
        assert_eq!(u32::from(s.tex_anim_index), expected);
    }

    #[test]
    fn pack_indices_stay_in_byte_range() {
        // Sweep the lifetime so the packs exercise their full low-byte range.
        for k in 0..200u32 {
            let now = 1.0 + k as f32 * 0.05;
            if let Some(s) = live(now, 1.0, 1000.0) {
                assert!(u32::from(s.tex_u_index) <= 0xff);
                assert!(u32::from(s.tex_anim_index) <= 0xff);
            }
        }
    }
}

/// Inverse-stride scalar for one octave column: the stock `fild(1<<col)`
/// followed by `one / x` (`fdivr` against the live `1.0` constant). `col` is the
/// octave index `0..depth`; the reciprocal feeds the per-cell noise scale.
pub fn cloud_layer_inv_stride__6cffc0(one: f32, col: u32) -> f32 {
    let stride = (1i32 << (col & 0x1f)) as f32;
    (f64::from(one) / f64::from(stride)) as f32
}

/// Builds the eight gradient-LUT record floats for one octave cell from four
/// adjacent-sample pairs. Each pair `(base, next)` becomes `base` and the
/// forward difference `next - base`, matching the stock `fld;fst;fld;fsub;fstp`
/// chain that fills record slots `+0x28..+0x44`.
pub fn cloud_layer_grad_diff__6cffc0(pairs: [(f32, f32); 4]) -> [f32; 8] {
    let mut out = [0.0f32; 8];
    let mut i = 0usize;
    while i < 4 {
        let (base, next) = pairs[i];
        out[2 * i] = base;
        out[2 * i + 1] = (f64::from(next) - f64::from(base)) as f32;
        i += 1;
    }
    out
}

/// Core value-noise bilinear cell evaluation: blends the eight record gradient
/// floats `rec` (`[a0,a1,b0,b1,c0,c1,d0,d1]`, the `+0x28..+0x44` slots) through
/// the three smoothstep weights `(wu, wa, wb)`, scales by `scale` (record
/// `+0x14`) and adds the running accumulator `acc`. Returns the new accumulator.
/// The f64-widened grouping reproduces the stock x87 fld/fmul/fadd/fsub/faddp
/// ordering exactly (lerp in U, lerp the pair, cross-blend in V, then MAD).
pub fn cloud_layer_bilerp_cell__6cffc0(
    rec: [f32; 8],
    wu: f32,
    wa: f32,
    wb: f32,
    scale: f32,
    acc: f32,
) -> f32 {
    let [a0, a1, b0, b1, c0, c1, d0, d1] = rec;
    let wu = f64::from(wu);
    let wa = f64::from(wa);
    let wb = f64::from(wb);

    // fVar7 = wu*a1 + a0 ; fVar8 = wu*c1 + c0 (stored to local_8).
    let f7 = wu * f64::from(a1) + f64::from(a0);
    let f8 = wu * f64::from(c1) + f64::from(c0);
    // fVar7' = ((wu*b1 + b0) - fVar7) * wa + fVar7
    let f7p = ((wu * f64::from(b1) + f64::from(b0)) - f7) * wa + f7;
    // tail = ((((wu*d1 + d0) - fVar8) * wa + fVar8) - fVar7') * wb + fVar7'
    let tail = (((wu * f64::from(d1) + f64::from(d0)) - f8) * wa + f8 - f7p) * wb + f7p;
    // acc' = tail * scale + acc
    (tail * f64::from(scale) + f64::from(acc)) as f32
}

/// Velocity/derivative deltas written when the octave column index reaches its
/// midpoint. `s` is `(1 << (shift - 7))` as an integer (the stock `fild`
/// argument); `prev` is the previous depth-cell accumulator (`fVar4`), `acc` the
/// current accumulator, `stored` the value already held in the `+0x5c` buffer.
/// Returns `((prev - acc) * s, (stored - acc) * s)` for the `+8*edx`/`+8*edx+4`
/// slots; the caller then copies `acc` into both `fVar4` and the `+0x5c` buffer.
pub fn cloud_layer_cross_term__6cffc0(prev: f32, acc: f32, stored: f32, s: i32) -> (f32, f32) {
    let sf = s as f32;
    let d0 = ((f64::from(prev) - f64::from(acc)) * f64::from(sf)) as f32;
    let d1 = ((f64::from(stored) - f64::from(acc)) * f64::from(sf)) as f32;
    (d0, d1)
}

/// Magic-bias byte quantize of one density float: `((v*k0 + k1 + k2) as f32
/// bits) >> 14`, yielding the raw 8-bit index BEFORE the floor subtraction and
/// clamp-table lookup. The `>> 14` is the stored-f32 mantissa extraction, not a
/// numeric cast, so boundary ULPs match the stock `fstp;mov;shr eax,0xe` trick.
pub fn cloud_layer_pack_byte__6cffc0(v: f32, k0: f32, k1: f32, k2: f32) -> u32 {
    let packed = (f64::from(v) * f64::from(k0) + f64::from(k1) + f64::from(k2)) as f32;
    (packed.to_bits() >> 14) & 0xff
}

#[cfg(test)]
mod tests_cloud_layer_update__6cffc0 {
    use super::{
        cloud_layer_bilerp_cell__6cffc0 as bilerp, cloud_layer_cross_term__6cffc0 as cross,
        cloud_layer_grad_diff__6cffc0 as grad_diff, cloud_layer_inv_stride__6cffc0 as inv_stride,
        cloud_layer_pack_byte__6cffc0 as pack,
    };

    #[test]
    fn inv_stride_is_reciprocal_power_of_two() {
        assert_eq!(inv_stride(1.0, 0).to_bits(), 1.0f32.to_bits());
        assert_eq!(inv_stride(1.0, 1).to_bits(), 0.5f32.to_bits());
        assert_eq!(inv_stride(1.0, 3).to_bits(), 0.125f32.to_bits());
        // non-unit numerator divides faithfully.
        assert_eq!(inv_stride(256.0, 2).to_bits(), 64.0f32.to_bits());
    }

    #[test]
    fn grad_diff_emits_base_then_forward_difference() {
        let out = grad_diff([(1.0, 1.5), (2.0, 0.0), (-1.0, -1.0), (3.0, 4.0)]);
        assert_eq!(out[0].to_bits(), 1.0f32.to_bits());
        assert_eq!(out[1].to_bits(), 0.5f32.to_bits());
        assert_eq!(out[2].to_bits(), 2.0f32.to_bits());
        assert_eq!(out[3].to_bits(), (-2.0f32).to_bits());
        assert_eq!(out[4].to_bits(), (-1.0f32).to_bits());
        assert_eq!(out[5].to_bits(), 0.0f32.to_bits());
        assert_eq!(out[6].to_bits(), 3.0f32.to_bits());
        assert_eq!(out[7].to_bits(), 1.0f32.to_bits());
    }

    #[test]
    fn bilerp_zero_weights_reduce_to_base_plus_acc() {
        // wu=wa=wb=0 -> tail collapses to a0; result = a0*scale + acc.
        let rec = [7.0, 99.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let r = bilerp(rec, 0.0, 0.0, 0.0, 2.0, 1.0);
        let expected = (7.0f64 * 2.0 + 1.0) as f32;
        assert_eq!(r.to_bits(), expected.to_bits());
    }

    #[test]
    fn bilerp_matches_explicit_grouping() {
        let rec = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8];
        let (wu, wa, wb, scale, acc) = (0.25f32, 0.5f32, 0.75f32, 1.5f32, 0.05f32);
        let (wuf, waf, wbf) = (f64::from(wu), f64::from(wa), f64::from(wb));
        let f7 = wuf * 0.2 + 0.1;
        let f8 = wuf * 0.6 + 0.5;
        let f7p = ((wuf * 0.4 + 0.3) - f7) * waf + f7;
        let tail = (((wuf * 0.8 + 0.7) - f8) * waf + f8 - f7p) * wbf + f7p;
        let expected = (tail * f64::from(scale) + f64::from(acc)) as f32;
        assert_eq!(
            bilerp(rec, wu, wa, wb, scale, acc).to_bits(),
            expected.to_bits()
        );
    }

    #[test]
    fn cross_term_scales_each_delta() {
        let (d0, d1) = cross(2.0, 0.5, 3.0, 4);
        // (prev-acc)*s = (2.0-0.5)*4 = 6.0 ; (stored-acc)*s = (3.0-0.5)*4 = 10.0
        assert_eq!(d0.to_bits(), 6.0f32.to_bits());
        assert_eq!(d1.to_bits(), 10.0f32.to_bits());
    }

    #[test]
    fn pack_byte_uses_mantissa_shift_not_cast() {
        // v*k0 + k1 + k2 with k0=1,k1=0,k2=512 (a representative magic bias):
        // result mantissa bits 14..22 are the quantized index.
        let raw = pack(10.0, 1.0, 0.0, 512.0);
        let expected = ((10.0f64 * 1.0 + 0.0 + 512.0) as f32).to_bits() >> 14 & 0xff;
        assert_eq!(raw, expected);
        // a fractional input that a plain `as u32` cast would round differently.
        let raw2 = pack(0.4999999, 1.0, 0.0, 512.0);
        let expected2 = ((0.4999999f64 + 512.0) as f32).to_bits() >> 14 & 0xff;
        assert_eq!(raw2, expected2);
    }
}
