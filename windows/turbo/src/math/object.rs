// Adapter and kernel names mirror the host's C++ symbols verbatim, with `__`
// standing in for the `::`, so the whole module is non-snake-case by construction.
#![allow(non_snake_case)]

/// World-space position of a local point under a parent transform.
///
/// `t` is the parent transform's leading 12 floats (row-major rows of 4),
/// `local` is the object's local position `[x, y, z]`. Only the upper 3x3
/// rotation/scale block participates; the lane wiring matches the original's
/// `fmul` schedule, which pairs each output row with a transposed column of the
/// block.
pub fn cg_game_object_c__get_world_position__7c4b80(t: &[f32; 12], local: &[f32; 3]) -> [f32; 3] {
    let x = local[0];
    let y = local[1];
    let z = local[2];
    [
        t[2] * x + t[6] * y + t[10] * z,
        t[1] * x + t[5] * y + t[9] * z,
        t[0] * x + t[4] * y + t[8] * z,
    ]
}

#[cfg(test)]
mod tests_cg_game_object_c__get_world_position__7c4b80 {
    use super::cg_game_object_c__get_world_position__7c4b80 as world_pos;

    #[test]
    fn permutation_block_maps_axes() {
        let mut t = [0.0_f32; 12];
        t[10] = 1.0;
        t[5] = 1.0;
        t[0] = 1.0;
        let out = world_pos(&t, &[3.0, 5.0, 7.0]);
        assert_eq!(out[0].to_bits(), 7.0_f32.to_bits());
        assert_eq!(out[1].to_bits(), 5.0_f32.to_bits());
        assert_eq!(out[2].to_bits(), 3.0_f32.to_bits());
    }

    #[test]
    fn linear_in_local() {
        let t = [0.3, 0.1, 0.2, 0.0, 0.4, 0.5, 0.6, 0.0, 0.7, 0.8, 0.9, 0.0];
        let a = world_pos(&t, &[1.0, 2.0, 3.0]);
        let b = world_pos(&t, &[2.0, 4.0, 6.0]);
        for i in 0..3 {
            assert_eq!((a[i] * 2.0).to_bits(), b[i].to_bits());
        }
    }
}

/// Render-proxy scale factor.
///
/// The product of the two base axis scales with the active third-axis scale.
/// `use_alt_scale` selects the alternate third scale (the original picks it
/// when the object carries a secondary model handle).
pub fn cg_object__gather_render_proxy_by_guid__483340(
    scale_x: f32,
    scale_y: f32,
    scale_z: f32,
    scale_z_alt: f32,
    use_alt_scale: bool,
) -> f32 {
    let third = if use_alt_scale { scale_z_alt } else { scale_z };
    scale_x * scale_y * third
}

#[cfg(test)]
mod tests_cg_object__gather_render_proxy_by_guid__483340 {
    use super::cg_object__gather_render_proxy_by_guid__483340 as scale;

    #[test]
    fn base_and_alt_paths() {
        assert_eq!(
            scale(2.0, 3.0, 5.0, 7.0, false).to_bits(),
            30.0_f32.to_bits()
        );
        assert_eq!(
            scale(2.0, 3.0, 5.0, 7.0, true).to_bits(),
            42.0_f32.to_bits()
        );
    }

    #[test]
    fn commutes_in_base_axes() {
        let a = scale(1.5, 4.0, 2.0, 2.0, false);
        let b = scale(4.0, 1.5, 2.0, 2.0, false);
        assert_eq!(a.to_bits(), b.to_bits());
    }
}

/// Whether `world_z` sits within the near-surface band around a liquid height.
///
/// Returns `1` only when the liquid is flagged present (`flag != 0`), the point
/// is below the surface plus `band_above` (`world_z < liquid_height +
/// band_above`), and the surface is no more than `band_below` above the point
/// (`liquid_height - world_z < band_below`). Both comparisons are strict `<`,
/// matching the original's `fcomp` reject paths (a `NaN` takes neither branch).
pub fn cg_unit_c__is_footstep_near_liquid__60a740(
    flag: u32,
    liquid_height: f32,
    world_z: f32,
    band_above: f32,
    band_below: f32,
) -> u32 {
    let above_ok = world_z < liquid_height + band_above;
    let below_ok = liquid_height - world_z < band_below;
    u32::from(flag != 0 && above_ok && below_ok)
}

#[cfg(test)]
mod tests_cg_unit_c__is_footstep_near_liquid__60a740 {
    use super::cg_unit_c__is_footstep_near_liquid__60a740 as near;

    #[test]
    fn inside_band_passes() {
        assert_eq!(near(1, 100.0, 99.5, 1.0, 2.0), 1);
    }

    #[test]
    fn no_flag_rejects() {
        assert_eq!(near(0, 100.0, 99.5, 1.0, 2.0), 0);
    }

    #[test]
    fn above_boundary_is_strict() {
        assert_eq!(near(1, 100.0, 101.0, 1.0, 50.0), 0);
        assert_eq!(near(1, 100.0, 100.99, 1.0, 50.0), 1);
    }

    #[test]
    fn below_boundary_is_strict() {
        assert_eq!(near(1, 100.0, 98.0, 5.0, 2.0), 0);
        assert_eq!(near(1, 100.0, 98.1, 5.0, 2.0), 1);
    }
}

/// Segment-cursor advance with wrap.
///
/// `cur + 1`, or `0` once it reaches `count` (folded stock helper at
/// `0x5f6260`).
pub fn cg_game_object_display_transport__advance_segment__5f6260(cur: u32, count: u32) -> u32 {
    let next = cur.wrapping_add(1);
    if next == count { 0 } else { next }
}

/// Fraction of `path_time` through the keyframe segment `[t0, t1)`.
///
/// The numerator is the unsigned tick delta and the denominator the span
/// as a signed dword (the original's `fild`/`fidiv` pair); computed in
/// `f64` to track the x87 division and narrowed to `f32` at the store.
pub fn cg_game_object_display_transport__segment_frac__5f6280(
    path_time: u32,
    t0: u32,
    t1: u32,
) -> f32 {
    let num = f64::from(path_time.wrapping_sub(t0));
    let den = f64::from(t1.wrapping_sub(t0) as i32);
    super::f64_to_f32(num / den)
}

/// Spline sample: the keyframe-position lerp rotated by the orientation quaternion.
///
/// `quat` is `[x, y, z, w]`. The doubled-product rotation matrix is built
/// and narrowed to `f32` exactly where the original stores it, then
/// applied transposed (`out[j] = m[6+j]*fz + m[3+j]*fy + m[j]*fx`) to the
/// lerp `(one - frac)*seg + frac*nxt`. `one` is the host's `1.0` constant,
/// injected.
pub fn cg_game_object_display_transport__eval_spline_position__5f6280(
    quat: &[f32; 4],
    seg_pos: &[f32; 3],
    nxt_pos: &[f32; 3],
    frac: f32,
    one: f32,
) -> [f32; 3] {
    let x = f64::from(quat[0]);
    let y = f64::from(quat[1]);
    let z = f64::from(quat[2]);
    let w = f64::from(quat[3]);
    let one_w = f64::from(one);
    let (x2, y2, z2) = (x + x, y + y, z + z);
    let (xw2, yw2, zw2) = (x2 * w, y2 * w, z2 * w);
    let (xx2, xy2, xz2) = (x2 * x, y2 * x, z2 * x);
    let (yy2, yz2, zz2) = (y2 * y, z2 * y, z2 * z);
    // The original stores every matrix element to a 32-bit slot before use.
    let m: [f32; 9] = [
        super::f64_to_f32(one_w - (zz2 + yy2)),
        super::f64_to_f32(xy2 + zw2),
        super::f64_to_f32(xz2 - yw2),
        super::f64_to_f32(xy2 - zw2),
        super::f64_to_f32(one_w - (zz2 + xx2)),
        super::f64_to_f32(yz2 + xw2),
        super::f64_to_f32(xz2 + yw2),
        super::f64_to_f32(yz2 - xw2),
        super::f64_to_f32(one_w - (yy2 + xx2)),
    ];
    let fr = f64::from(frac);
    let omf = one_w - fr;
    let fx = f64::from(super::f64_to_f32(
        omf * f64::from(seg_pos[0]) + fr * f64::from(nxt_pos[0]),
    ));
    let fy = f64::from(super::f64_to_f32(
        omf * f64::from(seg_pos[1]) + fr * f64::from(nxt_pos[1]),
    ));
    let fz = f64::from(super::f64_to_f32(
        omf * f64::from(seg_pos[2]) + fr * f64::from(nxt_pos[2]),
    ));
    [
        super::f64_to_f32(f64::from(m[6]) * fz + f64::from(m[3]) * fy + f64::from(m[0]) * fx),
        super::f64_to_f32(f64::from(m[7]) * fz + f64::from(m[4]) * fy + f64::from(m[1]) * fx),
        super::f64_to_f32(f64::from(m[8]) * fz + f64::from(m[5]) * fy + f64::from(m[2]) * fx),
    ]
}

#[cfg(test)]
mod tests_cg_game_object_display_transport__eval_spline_position__5f6280 {
    use super::{
        cg_game_object_display_transport__advance_segment__5f6260 as advance,
        cg_game_object_display_transport__eval_spline_position__5f6280 as eval,
        cg_game_object_display_transport__segment_frac__5f6280 as frac,
    };

    const ID_Q: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    #[test]
    fn advance_wraps_only_on_count() {
        assert_eq!(advance(0, 3), 1);
        assert_eq!(advance(1, 3), 2);
        assert_eq!(advance(2, 3), 0);
        // No clamp beyond equality, exactly like the masked stock form.
        assert_eq!(advance(5, 3), 6);
    }

    #[test]
    fn frac_known_values() {
        assert_eq!(frac(50, 0, 100).to_bits(), 0.5f32.to_bits());
        assert_eq!(frac(10, 10, 20).to_bits(), 0.0f32.to_bits());
        // Unsigned wrap on both delta computations.
        assert_eq!(
            frac(0xffff_fff8, 0xffff_fff0, 0x10).to_bits(),
            0.25f32.to_bits()
        );
    }

    #[test]
    fn identity_quat_lerp_endpoints() {
        let a = [1.0f32, -2.0, 3.5];
        let b = [4.0f32, 5.0, -6.0];
        let p0 = eval(&ID_Q, &a, &b, 0.0, 1.0);
        let p1 = eval(&ID_Q, &a, &b, 1.0, 1.0);
        for i in 0..3 {
            assert_eq!(p0[i].to_bits(), a[i].to_bits());
            assert_eq!(p1[i].to_bits(), b[i].to_bits());
        }
    }

    #[test]
    fn quarter_turns_about_each_axis() {
        let s = core::f32::consts::FRAC_1_SQRT_2;
        let zero = [0.0f32; 3];
        // +90 deg about z maps +x to +y.
        let qz = [0.0f32, 0.0, s, s];
        let r = eval(&qz, &[1.0, 0.0, 0.0], &zero, 0.0, 1.0);
        assert!((r[0]).abs() < 1e-6 && (r[1] - 1.0).abs() < 1e-6 && r[2].abs() < 1e-6);
        // +90 deg about x maps +y to +z.
        let qx = [s, 0.0f32, 0.0, s];
        let r = eval(&qx, &[0.0, 1.0, 0.0], &zero, 0.0, 1.0);
        assert!(r[0].abs() < 1e-6 && r[1].abs() < 1e-6 && (r[2] - 1.0).abs() < 1e-6);
        // +90 deg about y maps +z to +x.
        let qy = [0.0f32, s, 0.0, s];
        let r = eval(&qy, &[0.0, 0.0, 1.0], &zero, 0.0, 1.0);
        assert!((r[0] - 1.0).abs() < 1e-6 && r[1].abs() < 1e-6 && r[2].abs() < 1e-6);
    }

    #[test]
    fn rotation_preserves_length() {
        let q = {
            // Normalized arbitrary-axis quaternion.
            let raw = [0.3f32, -0.5, 0.2, 0.79];
            let n = (raw[0] * raw[0] + raw[1] * raw[1] + raw[2] * raw[2] + raw[3] * raw[3]).sqrt();
            [raw[0] / n, raw[1] / n, raw[2] / n, raw[3] / n]
        };
        let v = [2.0f32, -1.0, 0.5];
        let r = eval(&q, &v, &v, 0.5, 1.0);
        let len_in = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        let len_out = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt();
        assert!((len_in - len_out).abs() < 1e-5, "{len_in} vs {len_out}");
    }
}

/// Rotates a local-space surface normal into world space.
///
/// By the upper 3x3 block of a 16-float (row stride 4) transform.
///
/// Lane wiring matches the original's x87 schedule: each output sums the
/// `z` lane first, then `y`, then `x`, against one matrix column.
pub fn cg_object__update_wmo_liquid_submersion__69dc30(
    normal: &[f32; 3],
    mat: &[f32; 16],
) -> [f32; 3] {
    [
        normal[2] * mat[8] + normal[1] * mat[4] + normal[0] * mat[0],
        normal[2] * mat[9] + normal[1] * mat[5] + normal[0] * mat[1],
        normal[2] * mat[10] + normal[1] * mat[6] + normal[0] * mat[2],
    ]
}

#[cfg(test)]
mod tests_cg_object__update_wmo_liquid_submersion__69dc30 {
    use super::cg_object__update_wmo_liquid_submersion__69dc30 as rotate;

    const ID: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ];

    #[test]
    fn identity_is_neutral() {
        let n = [0.0f32, 0.6, 0.8];
        let r = rotate(&n, &ID);
        for i in 0..3 {
            assert_eq!(r[i].to_bits(), n[i].to_bits());
        }
    }

    #[test]
    fn translation_row_is_ignored() {
        let mut m = ID;
        m[12] = 100.0;
        m[13] = 200.0;
        m[14] = 300.0;
        let n = [1.0f32, 2.0, 3.0];
        let r = rotate(&n, &m);
        assert_eq!(r, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn axis_permutation_rows() {
        // Row-major rows (x->y, y->z, z->x) under the kernel's wiring.
        let mut m = [0.0f32; 16];
        m[1] = 1.0; // row 0 -> y lane
        m[6] = 1.0; // row 1 -> z lane
        m[8] = 1.0; // row 2 -> x lane
        let r = rotate(&[1.0, 0.0, 0.0], &m);
        assert_eq!(r, [0.0, 1.0, 0.0]);
        let r = rotate(&[0.0, 1.0, 0.0], &m);
        assert_eq!(r, [0.0, 0.0, 1.0]);
        let r = rotate(&[0.0, 0.0, 1.0], &m);
        assert_eq!(r, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn doubling_input_doubles_output() {
        let m: [f32; 16] = [
            0.3, 0.1, 0.2, 0.0, //
            0.4, 0.5, 0.6, 0.0, //
            0.7, 0.8, 0.9, 0.0, //
            5.0, 6.0, 7.0, 1.0,
        ];
        let n = [1.0f32, -2.0, 3.0];
        let n2 = [2.0f32, -4.0, 6.0];
        let a = rotate(&n, &m);
        let b = rotate(&n2, &m);
        for i in 0..3 {
            assert_eq!((a[i] * 2.0).to_bits(), b[i].to_bits());
        }
    }
}

/// Strict depth-span gate: `top - bottom < threshold`.
///
/// Computed in `f64`, where the `f32` subtraction is exact, matching the
/// original's x87 subtract-then-compare; a `NaN` operand fails the gate.
pub fn cg_render_entity__accumulate_bounds__672a20(top: f32, bottom: f32, threshold: f32) -> bool {
    (f64::from(top) - f64::from(bottom)) < f64::from(threshold)
}

/// Expands three color bytes to a scaled `[x, y, z]` vector.
///
/// Each lane is `byte * scale`, exact in `f64` and narrowed to `f32` where
/// the original stores it.
pub fn cg_render_entity__accumulate_bounds_expand__672a20(
    bx: u8,
    by: u8,
    bz: u8,
    scale: f32,
) -> [f32; 3] {
    let s = f64::from(scale);
    [
        super::f64_to_f32(f64::from(bx) * s),
        super::f64_to_f32(f64::from(by) * s),
        super::f64_to_f32(f64::from(bz) * s),
    ]
}

#[cfg(test)]
mod tests_cg_render_entity__accumulate_bounds__672a20 {
    use super::{
        cg_render_entity__accumulate_bounds__672a20 as gate,
        cg_render_entity__accumulate_bounds_expand__672a20 as expand,
    };

    #[test]
    fn gate_is_strict() {
        assert!(gate(1.0, 0.5, 0.6));
        assert!(!gate(1.0, 0.5, 0.5));
        assert!(!gate(2.0, 0.5, 0.6));
    }

    #[test]
    fn gate_rejects_nan() {
        assert!(!gate(f32::NAN, 0.0, 1.0));
        assert!(!gate(0.0, f32::NAN, 1.0));
        assert!(!gate(0.0, 0.0, f32::NAN));
    }

    #[test]
    fn gate_subtraction_is_exact() {
        // f32 would round 16777216.0 + 1.0; the f64 difference is exact.
        let top = 16_777_216.0f32;
        let bottom = -1.0f32;
        assert!(!gate(top, bottom, 16_777_216.0));
        assert!(gate(top, bottom, 16_777_218.0));
    }

    #[test]
    fn expand_known_values() {
        assert_eq!(expand(255, 0, 128, 0.5), [127.5, 0.0, 64.0]);
        assert_eq!(expand(7, 9, 11, 0.0), [0.0, 0.0, 0.0]);
        let inv255 = 1.0f32 / 255.0;
        let v = expand(255, 51, 0, inv255);
        assert_eq!(
            v[0].to_bits(),
            ((255.0f64 * f64::from(inv255)) as f32).to_bits()
        );
        assert_eq!(
            v[1].to_bits(),
            ((51.0f64 * f64::from(inv255)) as f32).to_bits()
        );
        assert_eq!(v[2].to_bits(), 0.0f32.to_bits());
    }
}

/// Colour-derived direction and anchor/pull position for a coloured-vector submission.
///
/// `color` holds the raw colour bytes in output lane order; each lane expands
/// as `byte * inv255 * intensity`. With `pull = Some((target, weight))` the
/// position starts at `anchor`, moves toward `target` by the fraction
/// `weight * inv255`, and is rescaled to length `one`; otherwise it stays at
/// `anchor`. Returns `(direction, position)`.
pub fn object_draw_colored_vector__6a7300(
    color: [u8; 3],
    intensity: f32,
    inv255: f32,
    anchor: [f32; 3],
    pull: Option<([f32; 3], u8)>,
    one: f32,
) -> ([f32; 3], [f32; 3]) {
    let dir = color.map(|b| f32::from(b) * inv255 * intensity);
    let pos = match pull {
        None => anchor,
        Some((target, weight)) => {
            let w = f32::from(weight) * inv255;
            let t = [
                anchor[0] + (target[0] - anchor[0]) * w,
                anchor[1] + (target[1] - anchor[1]) * w,
                anchor[2] + (target[2] - anchor[2]) * w,
            ];
            let inv = one / (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt();
            t.map(|c| c * inv)
        }
    };
    (dir, pos)
}

#[cfg(test)]
mod tests_object_draw_colored_vector__6a7300 {
    use super::object_draw_colored_vector__6a7300 as draw;

    // 1/256 keeps every expansion product exact in f32.
    const SCALE: f32 = 0.003_906_25;

    #[test]
    fn byte_expansion_is_lane_aligned_and_exact() {
        let (dir, pos) = draw([128, 255, 0], 2.0, SCALE, [9.0, 8.0, 7.0], None, 1.0);
        // 128/256 * 2 = 1.0, 255/256 * 2 = 1.9921875, both exact.
        assert_eq!(dir[0].to_bits(), 1.0_f32.to_bits());
        assert_eq!(dir[1].to_bits(), 1.992_187_5_f32.to_bits());
        assert_eq!(dir[2].to_bits(), 0.0_f32.to_bits());
        assert_eq!(pos[0].to_bits(), 9.0_f32.to_bits());
    }

    #[test]
    fn anchor_passthrough_preserves_bits() {
        let anchor = [-0.0_f32, 5.5, 1.0e-20];
        let (_, pos) = draw([1, 2, 3], 0.5, SCALE, anchor, None, 1.0);
        for (p, a) in pos.iter().zip(anchor.iter()) {
            assert_eq!(p.to_bits(), a.to_bits());
        }
    }

    #[test]
    fn pull_renormalizes_to_unit_length() {
        let (_, pos) = draw(
            [0, 0, 0],
            1.0,
            SCALE,
            [0.0, 0.0, 0.0],
            Some(([0.0, 3.0, 4.0], 128)),
            1.0,
        );
        // w = 0.5, t = [0, 1.5, 2], |t| = 2.5, inv = 0.4.
        assert_eq!(pos[0].to_bits(), 0.0_f32.to_bits());
        assert_eq!(pos[1].to_bits(), 0.6_f32.to_bits());
        assert_eq!(pos[2].to_bits(), 0.8_f32.to_bits());
    }

    #[test]
    fn one_scales_the_pulled_length() {
        let (_, pos) = draw(
            [0, 0, 0],
            1.0,
            SCALE,
            [0.0, 0.0, 0.0],
            Some(([0.0, 3.0, 4.0], 128)),
            2.0,
        );
        // Same t as above; inv = 2 / 2.5 = 0.8.
        assert_eq!(pos[1].to_bits(), 1.2_f32.to_bits());
        assert_eq!(pos[2].to_bits(), 1.6_f32.to_bits());
    }

    #[test]
    fn zero_weight_keeps_anchor_direction_but_rescales() {
        let (dir, pos) = draw(
            [10, 20, 30],
            0.0,
            SCALE,
            [0.0, 0.0, 2.0],
            Some(([55.0, 66.0, 77.0], 0)),
            3.0,
        );
        // weight 0: t = anchor, rescaled to length `one` = 3.
        assert_eq!(pos[2].to_bits(), 3.0_f32.to_bits());
        assert_eq!(pos[0].to_bits(), 0.0_f32.to_bits());
        // zero intensity zeroes every direction lane.
        assert_eq!(dir, [0.0, 0.0, 0.0]);
    }
}

/// Expands three color bytes to a scaled `[x, y, z]` vector.
///
/// Each lane is `byte * scale`, exact in `f64` and narrowed to `f32` where
/// the original stores it.
pub fn object_draw_debug_box_recursive__6a8050(bx: u8, by: u8, bz: u8, scale: f32) -> [f32; 3] {
    let s = f64::from(scale);
    [
        super::f64_to_f32(f64::from(bx) * s),
        super::f64_to_f32(f64::from(by) * s),
        super::f64_to_f32(f64::from(bz) * s),
    ]
}

/// Scales a 3-float vector by a weight.
///
/// Each product exact in `f64` and narrowed to `f32` where the original
/// stores it.
pub fn object_draw_debug_box_recursive_weight__6a8050(v: &[f32; 3], w: f32) -> [f32; 3] {
    let wf = f64::from(w);
    [
        super::f64_to_f32(f64::from(v[0]) * wf),
        super::f64_to_f32(f64::from(v[1]) * wf),
        super::f64_to_f32(f64::from(v[2]) * wf),
    ]
}

#[cfg(test)]
mod tests_object_draw_debug_box_recursive__6a8050 {
    use super::{
        object_draw_debug_box_recursive__6a8050 as expand,
        object_draw_debug_box_recursive_weight__6a8050 as weight,
    };

    #[test]
    fn expand_known_values() {
        assert_eq!(expand(4, 8, 16, 0.25), [1.0, 2.0, 4.0]);
        assert_eq!(expand(255, 0, 1, 0.0), [0.0, 0.0, 0.0]);
        let inv255 = 1.0f32 / 255.0;
        let v = expand(255, 128, 64, inv255);
        for (lane, b) in v.iter().zip([255.0f64, 128.0, 64.0]) {
            assert_eq!(lane.to_bits(), ((b * f64::from(inv255)) as f32).to_bits());
        }
    }

    #[test]
    fn weight_known_values() {
        assert_eq!(weight(&[1.0, 3.0, 5.0], 0.5), [0.5, 1.5, 2.5]);
        assert_eq!(weight(&[1.0, -3.0, 5.0], 0.0), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn weight_doubling_is_exact() {
        let v = [0.3f32, -7.25, 1024.5];
        let a = weight(&v, 2.0);
        for i in 0..3 {
            assert_eq!(a[i].to_bits(), (v[i] * 2.0).to_bits());
        }
    }
}

/// Distance-fade gate plus scaled direction/colour for a debug-box draw node.
///
/// `pos` is the owner anchor (`z_bias` is added to its `z` first), `src` the
/// box source position, and `one` the host unit constant. Returns `None` once
/// the distance reaches `fade_end` (the node is culled); otherwise yields the
/// direction `d * (one / dist)` from the source toward the owner and the
/// colour scaled by the fade alpha — `1.0` inside `fade_start`, else
/// `one - (dist - fade_start) * fade_rate`.
// The eight parameters are the stock draw node's own operands: anchor, bias,
// source, the three fade constants, the unit global and the colour. The list
// mirrors what the client passes, so folding them into a params struct would
// describe a shape no caller on the hook path has.
#[allow(clippy::too_many_arguments)]
pub fn object_draw_debug_box__6a7ac0(
    pos: [f32; 3],
    z_bias: f32,
    src: [f32; 3],
    fade_start: f32,
    fade_end: f32,
    fade_rate: f32,
    one: f32,
    color: [f32; 3],
) -> Option<([f32; 3], [f32; 3])> {
    let d = [pos[0] - src[0], pos[1] - src[1], (pos[2] + z_bias) - src[2]];
    let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    let alpha = if dist < fade_start {
        1.0
    } else if dist < fade_end {
        one - (dist - fade_start) * fade_rate
    } else {
        return None;
    };
    let inv = one / dist;
    Some((d.map(|c| c * inv), color.map(|c| c * alpha)))
}

#[cfg(test)]
mod tests_object_draw_debug_box__6a7ac0 {
    use super::object_draw_debug_box__6a7ac0 as draw;

    #[test]
    fn full_alpha_inside_fade_start() {
        let (dir, color) = draw(
            [3.0, 4.0, 0.0],
            0.0,
            [0.0, 0.0, 0.0],
            10.0,
            20.0,
            0.5,
            1.0,
            [0.5, 0.25, 1.0],
        )
        .unwrap();
        // dist = 5, inv = 0.2 exactly.
        assert_eq!(dir[0].to_bits(), 0.6_f32.to_bits());
        assert_eq!(dir[1].to_bits(), 0.8_f32.to_bits());
        assert_eq!(dir[2].to_bits(), 0.0_f32.to_bits());
        // alpha is the exact literal 1.0: colour bits unchanged.
        assert_eq!(color[0].to_bits(), 0.5_f32.to_bits());
        assert_eq!(color[1].to_bits(), 0.25_f32.to_bits());
        assert_eq!(color[2].to_bits(), 1.0_f32.to_bits());
    }

    #[test]
    fn z_bias_folds_into_distance() {
        let (dir, _) = draw(
            [0.0, 0.0, 1.0],
            2.0,
            [0.0, 0.0, 0.0],
            10.0,
            20.0,
            0.5,
            1.0,
            [1.0, 1.0, 1.0],
        )
        .unwrap();
        // d = [0, 0, 3], dist = 3, dir.z = 3 * (1/3) -> 1.0 after f32 rounding.
        assert_eq!(dir[2].to_bits(), 1.0_f32.to_bits());
        assert_eq!(dir[0].to_bits(), 0.0_f32.to_bits());
    }

    #[test]
    fn linear_fade_band_uses_one_and_rate() {
        let (dir, color) = draw(
            [6.0, 0.0, 0.0],
            0.0,
            [0.0, 0.0, 0.0],
            2.0,
            10.0,
            0.125,
            2.0,
            [1.0, 1.0, 1.0],
        )
        .unwrap();
        // alpha = 2 - (6 - 2) * 0.125 = 1.5 exactly.
        assert_eq!(color[0].to_bits(), 1.5_f32.to_bits());
        // dir.x = 6 * (2/6) -> 2.0 after f32 rounding.
        assert_eq!(dir[0].to_bits(), 2.0_f32.to_bits());
    }

    #[test]
    fn boundary_at_fade_start_takes_band_formula() {
        let (_, color) = draw(
            [5.0, 0.0, 0.0],
            0.0,
            [0.0, 0.0, 0.0],
            5.0,
            10.0,
            0.25,
            0.75,
            [1.0, 1.0, 1.0],
        )
        .unwrap();
        // dist == fade_start: alpha = one - 0 * rate = one.
        assert_eq!(color[0].to_bits(), 0.75_f32.to_bits());
    }

    #[test]
    fn culled_at_and_past_fade_end() {
        let at = draw(
            [5.0, 0.0, 0.0],
            0.0,
            [0.0, 0.0, 0.0],
            1.0,
            5.0,
            0.1,
            1.0,
            [1.0, 1.0, 1.0],
        );
        assert!(at.is_none());
        let past = draw(
            [50.0, 0.0, 0.0],
            0.0,
            [0.0, 0.0, 0.0],
            1.0,
            5.0,
            0.1,
            1.0,
            [1.0, 1.0, 1.0],
        );
        assert!(past.is_none());
    }

    #[test]
    fn nan_distance_culls() {
        let out = draw(
            [f32::NAN, 0.0, 0.0],
            0.0,
            [0.0, 0.0, 0.0],
            1.0,
            5.0,
            0.1,
            1.0,
            [1.0, 1.0, 1.0],
        );
        assert!(out.is_none());
    }
}

/// Mask-gated strict point-in-box test for one child record.
///
/// `rec` is `[min_x, min_y, min_z, max_x, max_y, max_z]`; the point must
/// lie strictly inside on every axis and the record's flag word must share
/// no bit with `mask`. All comparisons are strict (`fcomp` reject paths),
/// so any `NaN` lane fails.
pub fn object_pick_child_at_point__6a4e00(
    rec_flags: u32,
    mask: u32,
    rec: &[f32; 6],
    point: &[f32; 3],
) -> bool {
    (rec_flags & mask) == 0
        && rec[0] < point[0]
        && rec[1] < point[1]
        && rec[2] < point[2]
        && point[0] < rec[3]
        && point[1] < rec[4]
        && point[2] < rec[5]
}

#[cfg(test)]
mod tests_object_pick_child_at_point__6a4e00 {
    use super::object_pick_child_at_point__6a4e00 as inside;

    const BOX: [f32; 6] = [0.0, 0.0, 0.0, 2.0, 2.0, 2.0];

    #[test]
    fn interior_point_passes() {
        assert!(inside(0, 0xffff_ffff, &BOX, &[1.0, 1.0, 1.0]));
    }

    #[test]
    fn boundaries_are_strict() {
        assert!(!inside(0, 0, &BOX, &[0.0, 1.0, 1.0]));
        assert!(!inside(0, 0, &BOX, &[2.0, 1.0, 1.0]));
        assert!(!inside(0, 0, &BOX, &[1.0, 1.0, 2.0]));
    }

    #[test]
    fn mask_overlap_rejects() {
        assert!(inside(0x8, 0x4, &BOX, &[1.0, 1.0, 1.0]));
        assert!(!inside(0x8, 0x8, &BOX, &[1.0, 1.0, 1.0]));
        assert!(!inside(0xff, 0x10, &BOX, &[1.0, 1.0, 1.0]));
    }

    #[test]
    fn nan_rejects() {
        assert!(!inside(0, 0, &BOX, &[f32::NAN, 1.0, 1.0]));
        let mut b = BOX;
        b[5] = f32::NAN;
        assert!(!inside(0, 0, &b, &[1.0, 1.0, 1.0]));
    }

    #[test]
    fn outside_each_axis_rejects() {
        for axis in 0..3 {
            let mut p = [1.0f32, 1.0, 1.0];
            p[axis] = -0.5;
            assert!(!inside(0, 0, &BOX, &p), "axis {axis} low");
            p[axis] = 2.5;
            assert!(!inside(0, 0, &BOX, &p), "axis {axis} high");
        }
    }
}

/// Ordered `<=` across the first three lanes of two 4-lane groups.
///
/// `cmpleps` is an ordered compare, so a `NaN` in either operand clears its
/// lane and the reduce is false exactly where a scalar `<=` chain is. Lane 3
/// is dropped from the mask, which is what lets a caller fill it with whatever
/// dword neighbours the three floats in memory.
fn lanes3_le(lhs: [f32; 4], rhs: [f32; 4]) -> bool {
    #[cfg(target_arch = "x86")]
    use core::arch::x86::{_mm_cmple_ps, _mm_loadu_ps, _mm_movemask_ps};
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::{_mm_cmple_ps, _mm_loadu_ps, _mm_movemask_ps};
    // Every intrinsic below is SSE1, available on every ISA baseline this
    // crate builds for.
    // SAFETY: the load covers the 16 in-bounds bytes of its `[f32; 4]` argument.
    let a = unsafe { _mm_loadu_ps(lhs.as_ptr()) };
    // SAFETY: as above.
    let b = unsafe { _mm_loadu_ps(rhs.as_ptr()) };
    // SAFETY: lane-wise ordered compare of two initialized vectors.
    let le = unsafe { _mm_cmple_ps(a, b) };
    // SAFETY: sign-bit extraction from an initialized vector.
    let mask = unsafe { _mm_movemask_ps(le) };
    (mask & 0b111) == 0b111
}

/// Inclusive AABB overlap test between a stored record box and a query box.
///
/// Both boxes are `[min_x, min_y, min_z, max_x, max_y, max_z]`, delivered as
/// two overlapping 4-lane groups so that each half is one 16-byte load: `_lo`
/// is elements `0..4` of its box, `_hi` is elements `3..6` plus whatever dword
/// follows them (in a stored record, the field at `+0x1c`). Lane 3 of either
/// group never reaches the answer.
///
/// Overlap requires `rec.min <= query.max` and `rec.max >= query.min` on every
/// axis (the original's `fcomp` chain accepts equality on both sides), which is
/// three lanes of `rec_lo <= query_hi` and three of `query_lo <= rec_hi`. The
/// two groups are separate reduces so a record failing the first never pays for
/// the second, and `query.min <= rec.max` is the same ordered predicate as
/// `rec.max >= query.min`, so any `NaN` lane still fails the test.
pub fn object_query_overlapping_boxes__6a3b10(
    rec_lo: [f32; 4],
    rec_hi: [f32; 4],
    query_lo: [f32; 4],
    query_hi: [f32; 4],
) -> bool {
    lanes3_le(rec_lo, query_hi) && lanes3_le(query_lo, rec_hi)
}

/// Collector field mask derived from the query flag bits.
///
/// Starts from `0xee` (`0xc6` when bit `0x10` is set), clears `0x24` when
/// bit `0x20` is set, clears `0x2` unless bit `0x40` is set, and clears
/// `0x40` unless bit `0x4000` is set.
pub fn object_query_overlapping_boxes_field_mask__6a3b10(flags: u32) -> u32 {
    let mut mask: u32 = if flags & 0x10 != 0 { 0xc6 } else { 0xee };
    if flags & 0x20 != 0 {
        mask &= !0x24;
    }
    if flags & 0x40 == 0 {
        mask &= !0x2;
    }
    if flags & 0x4000 == 0 {
        mask &= !0x40;
    }
    mask
}

#[cfg(test)]
mod tests_object_query_overlapping_boxes__6a3b10 {
    use super::{
        object_query_overlapping_boxes__6a3b10 as overlap4,
        object_query_overlapping_boxes_field_mask__6a3b10 as field_mask,
    };

    // Independent oracle: the six-term ordered chain the two 4-lane groups
    // stand in for. Every term is false on a NaN operand, in either grouping.
    fn oracle(rec: &[f32; 6], query: &[f32; 6]) -> bool {
        rec[0] <= query[3]
            && rec[1] <= query[4]
            && rec[2] <= query[5]
            && rec[3] >= query[0]
            && rec[4] >= query[1]
            && rec[5] >= query[2]
    }

    // Feeds the kernel the way the hook does: two overlapping 4-lane groups per
    // box, with `junk` standing in for the dword that neighbours a box in
    // memory and must not reach the answer.
    fn overlap_junk(rec: &[f32; 6], query: &[f32; 6], junk: f32) -> bool {
        overlap4(
            [rec[0], rec[1], rec[2], rec[3]],
            [rec[3], rec[4], rec[5], junk],
            [query[0], query[1], query[2], query[3]],
            [query[3], query[4], query[5], junk],
        )
    }

    fn overlap(rec: &[f32; 6], query: &[f32; 6]) -> bool {
        overlap_junk(rec, query, 0.0)
    }

    #[test]
    fn identical_boxes_overlap() {
        let b = [0.0f32, 1.0, 2.0, 3.0, 4.0, 5.0];
        assert!(overlap(&b, &b));
    }

    #[test]
    fn touching_faces_are_inclusive() {
        // rec.max.x == query.min.x: still an overlap (both sides <=/>=).
        let rec = [0.0f32, 0.0, 0.0, 1.0, 1.0, 1.0];
        let query = [1.0f32, 0.0, 0.0, 2.0, 1.0, 1.0];
        assert!(overlap(&rec, &query));
        // rec.min.x == query.max.x likewise.
        let query2 = [-2.0f32, 0.0, 0.0, 0.0, 1.0, 1.0];
        assert!(overlap(&rec, &query2));
    }

    #[test]
    fn disjoint_on_each_axis_fails() {
        let rec = [0.0f32, 0.0, 0.0, 1.0, 1.0, 1.0];
        for axis in 0..3 {
            let mut query = [0.0f32, 0.0, 0.0, 1.0, 1.0, 1.0];
            query[axis] = 1.5; // query.min beyond rec.max on one axis
            query[3 + axis] = 2.5;
            assert!(!overlap(&rec, &query), "axis {axis}");
        }
    }

    #[test]
    fn nan_lane_rejects() {
        let rec = [0.0f32, 0.0, 0.0, 1.0, 1.0, 1.0];
        let mut query = [0.0f32, 0.0, 0.0, 1.0, 1.0, 1.0];
        query[4] = f32::NAN;
        assert!(!overlap(&rec, &query));
        let mut rec2 = rec;
        rec2[0] = f32::NAN;
        let q = [0.0f32, 0.0, 0.0, 1.0, 1.0, 1.0];
        assert!(!overlap(&rec2, &q));
    }

    #[test]
    fn overlap_is_symmetric() {
        let a = [0.0f32, -1.0, 2.0, 4.0, 1.0, 6.0];
        let b = [3.0f32, 0.0, 1.0, 9.0, 0.5, 3.0];
        let c = [10.0f32, 0.0, 0.0, 11.0, 1.0, 1.0];
        assert_eq!(overlap(&a, &b), overlap(&b, &a));
        assert_eq!(overlap(&a, &c), overlap(&c, &a));
    }

    /// The grouped kernel must answer what the six-term chain answers.
    ///
    /// Sweeps one min and one max lane of each box through ordinary orderings,
    /// touching faces, negative zero, the infinities and NaN, with the ignored
    /// fourth lane set to values that would flip the answer if the mask ever
    /// let them through.
    #[test]
    fn grouped_matches_scalar_chain() {
        let vals = [
            -1.0f32,
            0.0,
            -0.0,
            1.0,
            2.0,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
        ];
        for &a in &vals {
            for &b in &vals {
                let rec = [a, 0.0, 0.0, 1.0, 1.0, b];
                let query = [0.0f32, b, -1.0, 1.0, a, 1.0];
                for &junk in &[f32::NAN, f32::NEG_INFINITY, f32::INFINITY, 0.0] {
                    assert_eq!(
                        oracle(&rec, &query),
                        overlap_junk(&rec, &query, junk),
                        "rec={rec:?} query={query:?} junk={junk}"
                    );
                    assert_eq!(
                        oracle(&query, &rec),
                        overlap_junk(&query, &rec, junk),
                        "rec={query:?} query={rec:?} junk={junk}"
                    );
                }
            }
        }
    }

    #[test]
    fn field_mask_known_values() {
        assert_eq!(field_mask(0), 0xac);
        assert_eq!(field_mask(0x10 | 0x40 | 0x4000), 0xc6);
        assert_eq!(field_mask(0x40 | 0x4000), 0xee);
        assert_eq!(field_mask(0x70), 0x82);
        assert_eq!(field_mask(0x20 | 0x40 | 0x4000), 0xca);
    }
}

/// `Orientation::SetPosition` change-test kernel (`0x7adcd0`).
///
/// True when the new position differs from the stored one on any of the three
/// components.
///
/// Stock compares each component with the x87 `FLD stored; FCOMP new; FNSTSW AX;
/// TEST AH,0x44; JP` idiom. After `FCOMP`, `AH` carries `C3` (bit `0x40`) and
/// `C2` (bit `0x04`); `TEST AH,0x44` then sets parity (PF=1, "differs") for both
/// an ordered-unequal compare (`AH&0x44 == 0x00`, even) and an unordered/NaN
/// compare (`AH&0x44 == 0x44`, even), but clears parity (PF=0, "equal") for an
/// ordered-equal compare (`AH&0x44 == 0x40`, odd). Rust's `!=` on `f32` matches
/// exactly: it is `false` only for an ordered-equal pair and `true` for both
/// ordered-unequal and any NaN operand.
#[must_use]
pub fn position_differs(old: &[f32; 3], new: &[f32; 3]) -> bool {
    old[0] != new[0] || old[1] != new[1] || old[2] != new[2]
}

#[cfg(test)]
mod tests_orientation__set_position__7adcd0 {
    use super::position_differs;

    // Independent oracle: the position differs unless every lane is ordered-equal
    // (a NaN operand never compares equal, so it forces "differs").
    fn oracle(old: &[f32; 3], new: &[f32; 3]) -> bool {
        let mut all_eq = true;
        for i in 0..3 {
            all_eq &= old[i] == new[i];
        }
        !all_eq
    }

    #[test]
    fn equal_does_not_differ() {
        let v = [1.0f32, -2.5, 3.25];
        assert!(!position_differs(&v, &v));
        assert_eq!(position_differs(&v, &v), oracle(&v, &v));
    }

    #[test]
    fn single_lane_change_differs() {
        let old = [1.0f32, 2.0, 3.0];
        for i in 0..3 {
            let mut new = old;
            new[i] += 1.0;
            assert!(position_differs(&old, &new), "lane {i}");
            assert_eq!(position_differs(&old, &new), oracle(&old, &new));
        }
    }

    #[test]
    fn nan_lane_differs() {
        // A NaN operand is unordered -> parity set -> differs, even "against itself".
        let old = [f32::NAN, 2.0, 3.0];
        let new = [f32::NAN, 2.0, 3.0];
        assert!(position_differs(&old, &new));
        assert_eq!(position_differs(&old, &new), oracle(&old, &new));
        let old2 = [1.0f32, 2.0, 3.0];
        let new2 = [1.0f32, f32::NAN, 3.0];
        assert!(position_differs(&old2, &new2));
        assert_eq!(position_differs(&old2, &new2), oracle(&old2, &new2));
    }

    #[test]
    fn zero_sign_does_not_differ() {
        // +0.0 and -0.0 are ordered-equal -> no change, like stock's FCOMP.
        let old = [0.0f32, 0.0, 0.0];
        let new = [-0.0f32, -0.0, -0.0];
        assert!(!position_differs(&old, &new));
        assert_eq!(position_differs(&old, &new), oracle(&old, &new));
    }
}

/// `CGRenderEntity::UpdateTransform` (0x6717d0) — move-delta kernel.
///
/// The world displacement `xform.translation - entity.position`. Each lane is
/// computed wide and narrowed to `f32` exactly where stock spills it (`FSUB` +
/// `FSTP dword` into the `C3Vector::Set` scratch), so the squared length below
/// runs on the narrowed deltas just like the original.
#[must_use]
pub fn cg_render_entity__update_transform_delta__6717d0(
    trans: &[f32; 3],
    pos: &[f32; 3],
) -> [f32; 3] {
    [
        super::f64_to_f32(f64::from(trans[0]) - f64::from(pos[0])),
        super::f64_to_f32(f64::from(trans[1]) - f64::from(pos[1])),
        super::f64_to_f32(f64::from(trans[2]) - f64::from(pos[2])),
    ]
}

/// Move test.
///
/// `|dx² + dy² + dz²|` (x87 sum order `(dz² + dy²) + dx²`, then `FABS`) against
/// the squared threshold (stock `.rdata` VA 0x801360). Stock is
/// `FCOMP; FNSTSW; TEST AH,5; JP` feeding a `SETZ` inversion: the parity jump
/// lands on the `EAX = 0` arm for `>`, `==` AND the unordered compare, and the
/// `SETZ` then raises the republish flag — so "moved" means **not strictly
/// below** the threshold, and a NaN displacement counts as moved (republish).
/// (The naive "NaN ⇒ moved=0" reading misses the `SETZ` inversion: a NaN
/// displacement counts as moved.)
// `!(len2.abs() < threshold)` is the polarity the stock parity jump plus `SETZ`
// produces. Clippy's `len2.abs() >= threshold` drops the unordered arm, and an
// unordered compare has to report "moved" so the republish still happens.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
#[must_use]
pub fn cg_render_entity__update_transform_moved__6717d0(delta: &[f32; 3], threshold: f32) -> bool {
    let (dx, dy, dz) = (
        f64::from(delta[0]),
        f64::from(delta[1]),
        f64::from(delta[2]),
    );
    let len2 = (dz * dz + dy * dy) + dx * dx;
    !(len2.abs() < f64::from(threshold))
}

/// Uniform scale = `sqrt(row0 · row0)` (x87 sum order `(m00² + m01²) + m02²`, then `FSQRT`).
///
/// Returns both the narrowed `f32` stock stores at
/// `entity+0x30` (`FST dword`, which keeps the value on the x87 stack) and the
/// wide value the subsequent unit-scale test consumes unrounded.
#[must_use]
pub fn cg_render_entity__update_transform_scale__6717d0(row0: &[f32; 3]) -> (f32, f64) {
    let (m00, m01, m02) = (f64::from(row0[0]), f64::from(row0[1]), f64::from(row0[2]));
    let wide = ((m00 * m00 + m01 * m01) + m02 * m02).sqrt();
    (super::f64_to_f32(wide), wide)
}

/// Unit-scale test on the wide (unrounded) scale.
///
/// `|scale - one| < eps` STRICTLY (`FCOMP eps; TEST AH,5; JNP` — the no-parity
/// jump fires only on the ordered `<`). A NaN scale compares unordered and
/// FALLS THROUGH to the rescale branch (which then divides `one / NaN`),
/// exactly like stock.
#[must_use]
pub fn cg_render_entity__update_transform_scale_is_unit__6717d0(
    scale_wide: f64,
    one: f32,
    eps: f32,
) -> bool {
    (scale_wide - f64::from(one)).abs() < f64::from(eps)
}

/// Rescale factor `one / scale`.
///
/// Where `scale` is the already-narrowed `f32` re-read from `entity+0x30`
/// (`FDIV dword [entity+0x30]`). Unguarded divide, exactly like stock: a zero
/// scale yields `inf`, a NaN scale yields `NaN`.
#[must_use]
pub fn cg_render_entity__update_transform_inv_scale__6717d0(one: f32, scale: f32) -> f32 {
    super::f64_to_f32(f64::from(one) / f64::from(scale))
}

/// Extent-degeneracy predicate (per lane `FLD min; FCOMP max; TEST AH,0x41; JP` → collapse).
///
/// The box collapses to the entity position iff any lane has
/// `min > max` STRICTLY or compares unordered (NaN). `min == max` does NOT
/// collapse — the box transforms normally (`AH&0x41 == 0x40`, odd parity).
/// Rust `!(min <= max)` reproduces the mask exactly. `ext` is
/// `{min.xyz, max.xyz}`; lanes are tested in x, y, z order (no side effects,
/// so short-circuiting is free).
// Each lane is written `!(min <= max)` so an unordered lane collapses the box,
// matching the stock `TEST AH,0x41; JP` mask. Clippy's `min > max` would leave
// a NaN lane uncollapsed and diverge from the original on exactly that input.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
#[must_use]
pub fn cg_render_entity__update_transform_extent_collapsed__6717d0(ext: &[f32; 6]) -> bool {
    !(ext[0] <= ext[3]) || !(ext[1] <= ext[4]) || !(ext[2] <= ext[5])
}

/// Box midpoint.
///
/// `(min' + max') * half` per lane (stock `.rdata` half at VA 0x7ffa24 = 0.5),
/// narrowed at the stock `FSTP dword` spill. The `f32 * f32` product is exact
/// in `f64`, so the single narrowing matches the x87 store rounding
/// bit-for-bit.
#[must_use]
pub fn cg_render_entity__update_transform_midpoint__6717d0(sum: &[f32; 3], half: f32) -> [f32; 3] {
    [
        super::f64_to_f32(f64::from(sum[0]) * f64::from(half)),
        super::f64_to_f32(f64::from(sum[1]) * f64::from(half)),
        super::f64_to_f32(f64::from(sum[2]) * f64::from(half)),
    ]
}

/// Bounding radius.
///
/// `sqrt(|max' - mid|²)` over the STORED `f32` max corner (`entity+0x50`) and
/// midpoint (`entity+0x5c`) — x87 sum order `(dz² + dy²) + dx²` — narrowed at
/// the final `FSTP dword [entity+0x68]`.
#[must_use]
pub fn cg_render_entity__update_transform_radius__6717d0(maxc: &[f32; 3], mid: &[f32; 3]) -> f32 {
    let dx = f64::from(maxc[0]) - f64::from(mid[0]);
    let dy = f64::from(maxc[1]) - f64::from(mid[1]);
    let dz = f64::from(maxc[2]) - f64::from(mid[2]);
    super::f64_to_f32(((dz * dz + dy * dy) + dx * dx).sqrt())
}

#[cfg(test)]
mod tests_cg_render_entity__update_transform__6717d0 {
    use super::{
        cg_render_entity__update_transform_delta__6717d0 as delta,
        cg_render_entity__update_transform_extent_collapsed__6717d0 as extent_collapsed,
        cg_render_entity__update_transform_inv_scale__6717d0 as inv_scale,
        cg_render_entity__update_transform_midpoint__6717d0 as midpoint,
        cg_render_entity__update_transform_moved__6717d0 as moved,
        cg_render_entity__update_transform_radius__6717d0 as radius,
        cg_render_entity__update_transform_scale__6717d0 as scale,
        cg_render_entity__update_transform_scale_is_unit__6717d0 as scale_is_unit,
    };

    // Stock `.rdata` constants (WoW.exe 1.12.1.5875, read from the image):
    // squared move threshold @0x801360, unit @0x7ff9d8, scale epsilon
    // @0x8029d4 (2^-22), midpoint half @0x7ffa24.
    const THRESHOLD: f32 = 0.001;
    const ONE: f32 = 1.0;
    const EPS: f32 = 2.384_185_8e-7;
    const HALF: f32 = 0.5;

    #[test]
    fn delta_matches_lane_oracle() {
        let trans = [10.5f32, -3.25, 100.0];
        let pos = [1.5f32, 0.75, -20.0];
        let d = delta(&trans, &pos);
        for i in 0..3 {
            assert_eq!(d[i], trans[i] - pos[i], "lane {i}");
        }
    }

    #[test]
    fn moved_strictly_below_threshold_is_false() {
        // |delta|² = 3 * (0.01)² = 3e-4 < 1e-3.
        assert!(!moved(&[0.01, 0.01, 0.01], THRESHOLD));
        assert!(!moved(&[0.0, 0.0, 0.0], THRESHOLD));
    }

    #[test]
    fn moved_at_or_above_threshold_is_true() {
        // Exactly-equal case (FCOMP `==` also lands on the SETZ arm): use an
        // exactly representable threshold, |delta|² = 4.0.
        assert!(moved(&[2.0, 0.0, 0.0], 4.0));
        // Strictly above the stock threshold.
        assert!(moved(&[1.0, 0.0, 0.0], THRESHOLD));
    }

    #[test]
    fn moved_nan_displacement_is_true() {
        // Polarity: the unordered FCOMP takes the same parity
        // arm as `>=`, and the SETZ inversion raises the republish flag — a
        // NaN displacement means MOVED (the naive "NaN => moved=0" is the
        // pre-SETZ value of EAX, not the stored flag).
        assert!(moved(&[f32::NAN, 0.0, 0.0], THRESHOLD));
        assert!(moved(&[0.0, f32::NAN, 0.0], THRESHOLD));
        assert!(moved(&[0.0, 0.0, f32::NAN], THRESHOLD));
    }

    #[test]
    fn scale_of_pythagorean_row_is_exact() {
        let (s, wide) = scale(&[3.0, 4.0, 0.0]);
        assert_eq!(s, 5.0);
        assert_eq!(wide, 5.0);
    }

    #[test]
    fn scale_unit_test_is_strict_and_nan_rescales() {
        // Within eps -> direct-copy branch.
        assert!(scale_is_unit(1.0, ONE, EPS));
        assert!(scale_is_unit(1.0 + 1e-8, ONE, EPS));
        // Exactly eps away -> NOT unit (strict `<`).
        assert!(!scale_is_unit(1.0 + f64::from(EPS), ONE, EPS));
        // Clearly non-unit.
        assert!(!scale_is_unit(5.0, ONE, EPS));
        // NaN scale falls through to the rescale branch.
        assert!(!scale_is_unit(f64::NAN, ONE, EPS));
    }

    #[test]
    fn inv_scale_is_an_unguarded_divide() {
        assert_eq!(inv_scale(ONE, 2.0), 0.5);
        // 1/NaN = NaN (the rescale branch runs it on a NaN scale).
        assert!(inv_scale(ONE, f32::NAN).is_nan());
        // 1/0 = inf, no guard, exactly like stock.
        assert_eq!(inv_scale(ONE, 0.0), f32::INFINITY);
    }

    #[test]
    fn extent_ordered_boxes_do_not_collapse() {
        assert!(!extent_collapsed(&[-1.0, -2.0, -3.0, 1.0, 2.0, 3.0]));
    }

    #[test]
    fn extent_min_eq_max_does_not_collapse() {
        // A zero-thickness box transforms normally (FCOMP `==` is odd parity).
        assert!(!extent_collapsed(&[1.0, -2.0, 3.0, 1.0, -2.0, 3.0]));
    }

    #[test]
    fn extent_min_strictly_above_max_collapses_per_lane() {
        for lane in 0..3 {
            let mut ext = [-1.0f32, -1.0, -1.0, 1.0, 1.0, 1.0];
            ext[lane] = 2.0; // min > max in this lane only
            assert!(extent_collapsed(&ext), "lane {lane}");
        }
    }

    #[test]
    fn extent_nan_collapses_per_lane() {
        for slot in 0..6 {
            let mut ext = [-1.0f32, -1.0, -1.0, 1.0, 1.0, 1.0];
            ext[slot] = f32::NAN;
            assert!(extent_collapsed(&ext), "slot {slot}");
        }
    }

    #[test]
    fn midpoint_halves_the_corner_sum() {
        assert_eq!(midpoint(&[2.0, -4.0, 7.0], HALF), [1.0, -2.0, 3.5]);
    }

    #[test]
    fn radius_matches_euclidean_oracle() {
        let maxc = [4.0f32, 6.0, 8.0];
        let mid = [1.0f32, 2.0, 4.0];
        let r = radius(&maxc, &mid);
        // Independent oracle: sqrt(3² + 4² + 4²) = sqrt(41).
        let oracle = ((3.0f64 * 3.0 + 4.0 * 4.0) + 4.0 * 4.0).sqrt() as f32;
        assert_eq!(r, oracle);
        assert!((f64::from(r) - 41.0f64.sqrt()).abs() < 1e-6);
    }

    #[test]
    fn radius_propagates_nan() {
        assert!(radius(&[f32::NAN, 0.0, 0.0], &[0.0, 0.0, 0.0]).is_nan());
    }
}

/// Selection-circle anchor smoothing.
///
/// `CGObject_C__UpdateSelectionCircleTransform` @0x614cd0, hot core:
/// exponential lerp of the stored anchor toward the ground point,
/// `anchor' = ground + (anchor − ground)·k` with `k = pow(K, dt)` computed
/// by the caller (retiring the x87 `_CIpow`).
///
/// Rounding mirrors the stock x87 chain: the three deltas are f32
/// single-rounded; the X lane's `d.x·k + ground.x` stays extended until one
/// final store (f64 here), while the Y/Z lanes park `k·d` in an f32 local
/// before the add — two roundings. `k` carries f64 where the FPU carried
/// 80-bit (the accepted precision class).
pub fn selection_circle_smooth__614cd0(anchor: &[f32; 3], ground: &[f32; 3], k: f64) -> [f32; 3] {
    let d = [
        anchor[0] - ground[0],
        anchor[1] - ground[1],
        anchor[2] - ground[2],
    ];
    let x = (f64::from(d[0]) * k + f64::from(ground[0])) as f32;
    let ky = (k * f64::from(d[1])) as f32;
    let kz = (f64::from(d[2]) * k) as f32;
    [x, ky + ground[1], kz + ground[2]]
}

/// Selection-circle isotropic scale (`@0x614cd0` tail).
///
/// `base · f94 · f90` multiplied in the stock order (`f94·f90` first — exact —
/// then `·base`), rounded once at the store like the x87 `FMULP`/`FST`.
pub fn selection_circle_scale__614cd0(base: f32, f94: f32, f90: f32) -> f32 {
    (f64::from(f94) * f64::from(f90) * f64::from(base)) as f32
}

#[cfg(test)]
mod tests_selection_circle__614cd0 {
    use super::{selection_circle_scale__614cd0, selection_circle_smooth__614cd0};

    /// k = 0 snaps the anchor onto the ground point exactly.
    #[test]
    fn k_zero_snaps_to_ground() {
        let out = selection_circle_smooth__614cd0(&[5.0, -3.0, 9.0], &[1.0, 2.0, 3.0], 0.0);
        assert_eq!(out[0].to_bits(), 1.0f32.to_bits());
        assert_eq!(out[1].to_bits(), 2.0f32.to_bits());
        assert_eq!(out[2].to_bits(), 3.0f32.to_bits());
    }

    /// k = 1 keeps the anchor unchanged.
    ///
    /// Delta + ground restores the input bit-exactly for representable values.
    #[test]
    fn k_one_keeps_anchor() {
        let a = [5.5f32, -3.25, 9.125];
        let out = selection_circle_smooth__614cd0(&a, &[1.0, 2.0, 3.0], 1.0);
        assert_eq!(out[0].to_bits(), a[0].to_bits());
        assert_eq!(out[1].to_bits(), a[1].to_bits());
        assert_eq!(out[2].to_bits(), a[2].to_bits());
    }

    /// Midpoint blend.
    ///
    /// k = 0.5 lands halfway (exact halves chosen so every step is representable).
    #[test]
    fn k_half_blends() {
        let out = selection_circle_smooth__614cd0(&[4.0, 8.0, -2.0], &[2.0, 4.0, 2.0], 0.5);
        assert_eq!(out[0].to_bits(), 3.0f32.to_bits());
        assert_eq!(out[1].to_bits(), 6.0f32.to_bits());
        assert_eq!(out[2].to_bits(), 0.0f32.to_bits());
    }

    /// The Y lane really is double-rounded through an f32 local.
    ///
    /// (Stock FSTP/reload) while X stays extended: sweep until the two chains
    /// disagree, proving the asymmetry is live.
    #[test]
    fn y_lane_uses_stored_f32_intermediate() {
        let mut found = false;
        for i in 1..5000u32 {
            let ay = 0.1f32 + i as f32 * 1.000_001e-3;
            let g = [0.0f32, 0.333_333_34, 0.0];
            let k = 0.700_000_1_f64;
            let d = ay - g[1];
            let expect = (k * f64::from(d)) as f32 + g[1];
            let single = (f64::from(d) * k + f64::from(g[1])) as f32;
            let out = selection_circle_smooth__614cd0(&[0.0, ay, 0.0], &g, k);
            assert_eq!(out[1].to_bits(), expect.to_bits(), "ay={ay}");
            if expect.to_bits() != single.to_bits() {
                found = true;
            }
        }
        assert!(found, "sweep never hit a double-rounding boundary");
    }

    /// Scale multiplies in the stock order and rounds once.
    #[test]
    fn scale_order_and_rounding() {
        let s = selection_circle_scale__614cd0(2.0, 3.0, 0.25);
        assert_eq!(s.to_bits(), 1.5f32.to_bits());
        // Non-trivial operands match the documented f64 chain bit-for-bit.
        let (b, f94, f90) = (1.234_567_9f32, 0.333_333_34f32, 7.777_778e-2f32);
        let want = (f64::from(f94) * f64::from(f90) * f64::from(b)) as f32;
        assert_eq!(
            selection_circle_scale__614cd0(b, f94, f90).to_bits(),
            want.to_bits()
        );
    }

    /// NaN ground.
    ///
    /// The caller's gate skips the smoothing entirely, but if a NaN sneaks
    /// into a non-gated lane it must propagate, not panic.
    #[test]
    fn nan_propagates() {
        let out = selection_circle_smooth__614cd0(&[1.0, f32::NAN, 3.0], &[0.0, 0.0, 0.0], 0.5);
        assert!(out[1].is_nan());
    }
}

/// Selection-circle fade for `CGObject_C::UpdateSelectionCircleAndVisuals`.
///
/// @0x614a90, 0x614b34–0x614b96: cubic-eased lerp of the circle alpha
/// animation `{from, to}` at `elapsed/dur`.
///
/// `FILD/FIDIV` divide the SIGNED dwords wide; `FST` narrows `t` but keeps
/// the wide quotient for the low compare (`< lo` ordered — NaN falls
/// through), while the high compare and the cube re-load the NARROWED `t`
/// (`> hi` ordered picks `hi`; `<=` and NaN cube). The lerp folds
/// `(to − from)·fade + from` wide with one narrow into the `+0xf4` store.
pub fn selection_fade__614a90(elapsed: i32, dur: i32, lo: f32, hi: f32, from: f32, to: f32) -> f32 {
    let t_wide = f64::from(elapsed) / f64::from(dur);
    let t = f64::from(super::f64_to_f32(t_wide));
    let fade = if t_wide < f64::from(lo) {
        f64::from(lo)
    } else if t > f64::from(hi) {
        f64::from(hi)
    } else {
        t * t * t
    };
    super::f64_to_f32((f64::from(to) - f64::from(from)) * fade + f64::from(from))
}

/// Two-second cosine scale pulse (0x614bdb–0x614c26).
///
/// Blends the recorded old scale toward the live `vfunc+0x1c` target.
///
/// The angle narrows once (`FSTP` after the two constant multiplies) before
/// the `FCOS`; the blend weight `(−cos)·half + half` and the lerp fold wide
/// with one narrow into the `+0x90` store. The cosine itself is native f64
/// (`FCOS` is not bit-reproducible; SSE trig by design, wave-4 precedent).
pub fn scale_pulse__614a90(
    elapsed: i32,
    k1: f32,
    k2: f32,
    half: f32,
    target: f32,
    old: f32,
) -> f32 {
    let angle = super::f64_to_f32(f64::from(elapsed) * f64::from(k1) * f64::from(k2));
    let c = f64::from(angle).cos();
    let w = -c * f64::from(half) + f64::from(half);
    super::f64_to_f32(w * (f64::from(target) - f64::from(old)) + f64::from(old))
}

/// Spell-visual survivor fade (0x614c58–0x614ca7).
///
/// Cubic ease of the remaining-lifetime fraction, clamped to `[0, 1]`.
///
/// The remaining time FILDs as a 64-bit integer whose HIGH DWORD is forced
/// zero (0x614c60) — an already-expired tail node's negative remainder
/// wraps to a huge positive and clamps to full alpha, exactly as stock.
/// Same wide-low/narrow-high compare split as [`selection_fade__614a90`]:
/// `< lo` wide → 0.0; `> hi` on the narrowed value → 1.0; else the narrowed
/// value cubes wide with one narrow.
pub fn visual_fade__614a90(remaining_raw: u32, k1: f32, lo: f32, hi: f32) -> f32 {
    let v_wide = f64::from(remaining_raw) * f64::from(k1);
    let v = f64::from(super::f64_to_f32(v_wide));
    if v_wide < f64::from(lo) {
        0.0
    } else if v > f64::from(hi) {
        1.0
    } else {
        super::f64_to_f32(v * v * v)
    }
}

/// Circle alpha product (0x614baa–0x614bb7).
///
/// `base(+0x100) · fade(+0xf4)`, one narrow into the pushed argument.
pub fn circle_alpha__614a90(base_alpha: f32, fade: f32) -> f32 {
    super::f64_to_f32(f64::from(base_alpha) * f64::from(fade))
}

#[cfg(test)]
mod tests_selection_visuals__614a90 {
    use super::{
        scale_pulse__614a90 as pulse, selection_fade__614a90 as sel_fade,
        visual_fade__614a90 as vis_fade,
    };

    #[test]
    fn circle_alpha_product() {
        use super::circle_alpha__614a90 as alpha;
        assert_eq!(alpha(0.5, 0.25).to_bits(), 0.125f32.to_bits());
        // Wide product, single narrow.
        let want = (f64::from(0.1f32) * f64::from(0.3f32)) as f32;
        assert_eq!(alpha(0.1, 0.3).to_bits(), want.to_bits());
    }

    #[test]
    fn selection_fade_cubic_midband() {
        // t = 0.5 -> fade = 0.125; from 0 to 1 -> 0.125.
        let got = sel_fade(500, 1000, 0.0, 1.0, 0.0, 1.0);
        assert_eq!(got.to_bits(), 0.125f32.to_bits());
        // Clamps: negative elapsed -> lo (0.0) -> from; t > 1 -> hi -> to.
        assert_eq!(
            sel_fade(-100, 1000, 0.0, 1.0, 2.0, 6.0).to_bits(),
            2.0f32.to_bits()
        );
        assert_eq!(
            sel_fade(1500, 1000, 0.0, 1.0, 2.0, 6.0).to_bits(),
            6.0f32.to_bits()
        );
    }

    #[test]
    fn scale_pulse_endpoints() {
        // angle 0 -> cos=1 -> weight 0 -> old.
        let got = pulse(0, 1.0, 1.0, 0.5, 3.0, 1.0);
        assert_eq!(got.to_bits(), 1.0f32.to_bits());
        // cos(pi) = -1 -> weight = 2*half = 1 -> target (k1*k2 scale pi in).
        let pi = core::f32::consts::PI;
        let got = pulse(1, pi, 1.0, 0.5, 3.0, 1.0);
        assert!((got - 3.0).abs() < 1.0e-6, "{got}");
    }

    #[test]
    fn visual_fade_clamps_and_cube() {
        // remaining 500ms at 1/1000 -> 0.5 -> 0.125.
        let got = vis_fade(500, 0.001, 0.0, 1.0);
        let want = {
            // v = f32(500 * k1) narrowed the kernel's way, then cubed wide.
            let vn = f64::from((500.0f64 * f64::from(0.001f32)) as f32);
            (vn * vn * vn) as f32
        };
        assert_eq!(got.to_bits(), want.to_bits());
        // Above the window -> 1.0.
        assert_eq!(vis_fade(5000, 0.001, 0.0, 1.0).to_bits(), 1.0f32.to_bits());
        // Expired tail node: negative i32 remainder arrives as wrapped u32
        // (high dword zeroed by stock) -> huge positive -> full alpha.
        assert_eq!(
            vis_fade((-100i32) as u32, 0.001, 0.0, 1.0).to_bits(),
            1.0f32.to_bits()
        );
        // Exactly zero remaining -> 0 * k1 = 0, not < lo -> cubes to 0.
        assert_eq!(vis_fade(0, 0.001, 0.0, 1.0).to_bits(), 0.0f32.to_bits());
    }
}

/// Facing delta + epsilon snap for `CGUnit_C::UpdateFacingInterpolation`.
///
/// @0x600cd0, 0x600ea8–0x600ed7: `d = target − orientation` with the
/// WIDE delta feeding the |d| FABS/FCOMP snap gate (snap iff `|d| <= eps`
/// ORDERED — `TEST AH,0x41; JP` routes NaN to smoothing) and one narrow
/// into the scratch slot. On snap the WIDE target narrows into the
/// orientation store. Returns `(d, f32(target), snap)`.
pub fn facing_delta_snap__600cd0(target: f64, current: f32, eps: f32) -> (f32, f32, bool) {
    let dw = target - f64::from(current);
    let d = super::f64_to_f32(dw);
    let snap = dw.abs() <= f64::from(eps);
    (d, super::f64_to_f32(target), snap)
}

/// ±2π wrap of the facing delta (0x600ed8–0x600f0b).
///
/// `d > hi` ordered subtracts a period, else `d < lo` ordered adds one (each a
/// wide fold, one narrow); anything else — including NaN — passes through.
pub fn facing_wrap__600cd0(d: f32, hi: f32, lo: f32, two_pi: f32) -> f32 {
    if d > hi {
        super::f64_to_f32(f64::from(d) - f64::from(two_pi))
    } else if d < lo {
        super::f64_to_f32(f64::from(d) + f64::from(two_pi))
    } else {
        d
    }
}

/// Four-sample history average + overshoot guard (0x600f7b–0x600fcb).
///
/// The sum seeds from the 0.0 global and folds the four history floats wide,
/// scales by the quarter constant, then the average is used only when it
/// does NOT overshoot the raw delta in the delta's direction:
/// `d > 0` ordered keeps the average iff NOT `d < avg` (a NaN average
/// survives); `d <= 0` and NaN-d keep it iff NOT `d > avg`. The average
/// narrows only when selected; the raw delta passes through unnarrowed.
// Both overshoot guards are negated ordered tests, so a NaN average survives
// the guard the way the stock branch leaves it selected. Clippy's `d >= avg` /
// `d <= avg` would discard the average on that input instead.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn facing_smooth__600cd0(d: f32, hist: [f32; 4], quarter: f32, zero: f32) -> f32 {
    let sum = f64::from(zero)
        + f64::from(hist[0])
        + f64::from(hist[1])
        + f64::from(hist[2])
        + f64::from(hist[3]);
    let avg = sum * f64::from(quarter);
    let use_avg = if d > zero {
        !(f64::from(d) < avg)
    } else {
        !(f64::from(d) > avg)
    };
    if use_avg { super::f64_to_f32(avg) } else { d }
}

/// Half-step apply + final [0, 2π) wrap (0x600fcb–0x601018).
///
/// `result·half + orientation` folds wide; `v < 0` ordered adds a period,
/// else `v >= 2π` ordered subtracts one, NaN stores as-is; one narrow into
/// the orientation store.
pub fn facing_apply__600cd0(result: f32, current: f32, half: f32, zero: f32, two_pi: f32) -> f32 {
    let v = f64::from(result) * f64::from(half) + f64::from(current);
    if v < f64::from(zero) {
        super::f64_to_f32(v + f64::from(two_pi))
    } else if v >= f64::from(two_pi) {
        super::f64_to_f32(v - f64::from(two_pi))
    } else {
        super::f64_to_f32(v)
    }
}

#[cfg(test)]
mod tests_facing_interpolation__600cd0 {
    use super::{
        facing_apply__600cd0 as apply, facing_delta_snap__600cd0 as delta,
        facing_smooth__600cd0 as smooth, facing_wrap__600cd0 as wrap,
    };

    const TWO_PI: f32 = core::f32::consts::TAU;

    #[test]
    fn delta_snap_polarity() {
        // |d| < eps and |d| == eps snap; NaN routes to smoothing.
        assert!(delta(1.0, 1.0005, 0.001).2);
        let (_, _, snap) = delta(1.001, 1.0, 0.001);
        assert!(snap, "exact eps snaps (C3 falls through)");
        assert!(!delta(1.01, 1.0, 0.001).2);
        assert!(!delta(f64::NAN, 1.0, 0.001).2);
        // The wide delta narrows once.
        let (d, t32, _) = delta(2.5, 1.0, 0.001);
        assert_eq!(d.to_bits(), 1.5f32.to_bits());
        assert_eq!(t32.to_bits(), 2.5f32.to_bits());
    }

    #[test]
    fn wrap_polarity() {
        // Above hi -> subtract a period; below lo -> add one.
        let got = wrap(4.0, core::f32::consts::PI, -core::f32::consts::PI, TWO_PI);
        assert_eq!(
            got.to_bits(),
            ((f64::from(4.0f32) - f64::from(TWO_PI)) as f32).to_bits()
        );
        let got = wrap(-4.0, core::f32::consts::PI, -core::f32::consts::PI, TWO_PI);
        assert_eq!(
            got.to_bits(),
            ((f64::from(-4.0f32) + f64::from(TWO_PI)) as f32).to_bits()
        );
        // In-band and NaN pass through.
        assert_eq!(wrap(1.0, 3.0, -3.0, TWO_PI).to_bits(), 1.0f32.to_bits());
        assert!(wrap(f32::NAN, 3.0, -3.0, TWO_PI).is_nan());
    }

    #[test]
    fn smooth_average_and_overshoot() {
        // Uniform history: avg == d -> avg selected (not an overshoot).
        let got = smooth(1.0, [1.0; 4], 0.25, 0.0);
        assert_eq!(got.to_bits(), 1.0f32.to_bits());
        // Positive d with smaller average -> average (no overshoot).
        let got = smooth(2.0, [1.0, 1.0, 1.0, 1.0], 0.25, 0.0);
        assert_eq!(got.to_bits(), 1.0f32.to_bits());
        // Positive d with larger average -> raw d (overshoot clamped).
        let got = smooth(0.5, [1.0; 4], 0.25, 0.0);
        assert_eq!(got.to_bits(), 0.5f32.to_bits());
        // Negative d mirrored: avg above d selected, below d clamped.
        let got = smooth(-2.0, [-1.0; 4], 0.25, 0.0);
        assert_eq!(got.to_bits(), (-1.0f32).to_bits());
        let got = smooth(-0.5, [-1.0; 4], 0.25, 0.0);
        assert_eq!(got.to_bits(), (-0.5f32).to_bits());
        // NaN average survives selection on the d > 0 arm.
        assert!(smooth(1.0, [f32::NAN; 4], 0.25, 0.0).is_nan());
    }

    #[test]
    fn apply_wraps_into_period() {
        // result/2 + current in-band.
        let got = apply(1.0, 1.0, 0.5, 0.0, TWO_PI);
        assert_eq!(got.to_bits(), 1.5f32.to_bits());
        // Negative -> + 2pi.
        let got = apply(-4.0, 1.0, 0.5, 0.0, TWO_PI);
        let want = (f64::from(-4.0f32) * 0.5 + 1.0 + f64::from(TWO_PI)) as f32;
        assert_eq!(got.to_bits(), want.to_bits());
        // At/above 2pi -> - 2pi (boundary == wraps).
        let got = apply(2.0 * TWO_PI, 0.0, 0.5, 0.0, TWO_PI);
        let want = (f64::from(TWO_PI) * f64::from(0.5f32) * 2.0 - f64::from(TWO_PI)) as f32;
        assert_eq!(got.to_bits(), want.to_bits());
        // NaN stores as-is.
        assert!(apply(f32::NAN, 0.0, 0.5, 0.0, TWO_PI).is_nan());
    }
}

/// Target-highlight blink byte of `CGPlayer_C::OnFrameUpdate` (0x607f9b..0x607fbe).
///
/// The remaining-ms `FILD` (exact) × the 1/500 scale, optionally folded through
/// `one − t` when the blink phase word is NON-zero (the stock `JZ` skips the
/// `FSUBR` on zero), × the byte scale, then the CRT `__ftol` — the caller keeps
/// AL (bits 8..15 of the packed color dword). The whole chain runs wide; no
/// intermediate narrows.
#[must_use]
pub fn blink_byte__607ed0(remaining: i32, phase: u32, k_inv: f32, one: f32, k_byte: f32) -> u8 {
    let mut t = f64::from(remaining) * f64::from(k_inv);
    if phase != 0 {
        t = f64::from(one) - t;
    }
    t *= f64::from(k_byte);
    (crate::math::misc::ftol__40a2b0(t) as u32 & 0xff) as u8
}

/// Turn-assist wrap argument of `CGPlayer_C::OnFrameUpdate` (0x6080c2..0x6080d1).
///
/// `(c98 − c94) + step` folds entirely wide and narrows ONCE into the
/// `NormalizeAngleToPi` stack argument (a per-op f32 model would round the
/// intermediate difference a second time). `step` arrives pre-negated by the
/// parity select (`FCHS`, exact).
#[must_use]
pub fn facing_assist_arg__607ed0(c98: f32, c94: f32, step: f32) -> f32 {
    super::f64_to_f32((f64::from(c98) - f64::from(c94)) + f64::from(step))
}

/// Turn-assist blend of `CGPlayer_C::OnFrameUpdate` (0x6080db..0x6080e8).
///
/// The wrapped delta × the quarter constant + the current facing, wide,
/// one narrow into the second `NormalizeAngleToPi` argument.
#[must_use]
pub fn facing_assist_blend__607ed0(wrapped: f32, quarter: f32, c94: f32) -> f32 {
    super::f64_to_f32(f64::from(wrapped) * f64::from(quarter) + f64::from(c94))
}

/// Turn-rate step of `CGPlayer_C::OnFrameUpdate` (0x6081dd..0x608204).
///
/// The ms delta `FILD`s as a ZERO-EXTENDED qword (exact), the three-factor
/// rate chain stays wide, and the `FCOM`+`TEST AH,0x41` pick keeps the
/// RATE on `rate <= |d|` **or unordered** (a NaN rate survives the pick),
/// switching to `|d|` only on an ordered `rate > |d|`; one narrow.
#[must_use]
pub fn turn_step__607ed0(dt_ms: u32, k_a: f32, unit_rate: f32, k_b: f32, abs_delta: f32) -> f32 {
    let rate = f64::from(dt_ms) * f64::from(k_a) * f64::from(unit_rate) * f64::from(k_b);
    let chosen = if rate > f64::from(abs_delta) {
        f64::from(abs_delta)
    } else {
        rate
    };
    super::f64_to_f32(chosen)
}

/// The sign-transfer idiom of `CGPlayer_C::OnFrameUpdate`.
///
/// Used at 0x60819b, 0x60820f, 0x6082e6, 0x608383: integer
/// `((mag ^ sign) & 0x7fffffff) ^ sign` — the magnitude bits of `mag` with
/// the sign bit of `sign_src`, exact for every payload including NaNs.
#[must_use]
pub fn copysign_bits__607ed0(mag: f32, sign_src: f32) -> f32 {
    let s = sign_src.to_bits();
    f32::from_bits((mag.to_bits() ^ s) & 0x7fff_ffff ^ s)
}

#[cfg(test)]
mod tests_on_frame_update__607ed0 {
    use super::{
        blink_byte__607ed0 as blink, copysign_bits__607ed0 as csign,
        facing_assist_arg__607ed0 as assist_arg, facing_assist_blend__607ed0 as blend,
        turn_step__607ed0 as step,
    };

    const INV_500: f32 = 0.002;
    const BYTE_SCALE: f32 = 255.0;

    #[test]
    fn blink_ramps_up_and_down() {
        // Phase 0: t = remaining/500 × 255, truncated.
        assert_eq!(blink(500, 0, INV_500, 1.0, BYTE_SCALE), 255);
        assert_eq!(blink(250, 0, INV_500, 1.0, BYTE_SCALE), 127);
        // Phase 1: inverted ramp.
        assert_eq!(blink(500, 1, INV_500, 1.0, BYTE_SCALE), 0);
        assert_eq!(blink(0, 1, INV_500, 1.0, BYTE_SCALE), 255);
    }

    #[test]
    fn blink_truncates_like_ftol() {
        // 100/500 × 255 = 51.0 exactly; 99/500 × 255 = 50.49 → 50.
        assert_eq!(blink(100, 0, INV_500, 1.0, BYTE_SCALE), 51);
        assert_eq!(blink(99, 0, INV_500, 1.0, BYTE_SCALE), 50);
    }

    #[test]
    fn assist_arg_narrows_once() {
        let expect = ((f64::from(6.5f32) - f64::from(0.25f32)) + f64::from(-3.0f32)) as f32;
        assert_eq!(assist_arg(6.5, 0.25, -3.0).to_bits(), expect.to_bits());
    }

    #[test]
    fn blend_is_mul_add_wide() {
        let expect = (f64::from(2.0f32) * f64::from(0.25f32) + f64::from(1.5f32)) as f32;
        assert_eq!(blend(2.0, 0.25, 1.5).to_bits(), expect.to_bits());
    }

    #[test]
    fn step_caps_at_the_abs_delta() {
        // Huge dt → rate far above |d| → capped.
        assert_eq!(
            step(100_000, 0.001, 1.0, 1.0, 0.5).to_bits(),
            0.5f32.to_bits()
        );
        // Tiny dt → rate below |d| → the wide rate, narrowed once.
        let expect = ((f64::from(2u32) * f64::from(0.001f32)) * 1.0 * 1.0) as f32;
        assert_eq!(step(2, 0.001, 1.0, 1.0, 0.5).to_bits(), expect.to_bits());
    }

    #[test]
    fn step_keeps_a_nan_rate() {
        // Unordered FCOM keeps the rate — NaN propagates to the store.
        assert!(step(1, f32::NAN, 1.0, 1.0, 0.5).is_nan());
    }

    #[test]
    fn copysign_matches_the_xor_trick() {
        assert_eq!(csign(1.5, -0.0).to_bits(), (-1.5f32).to_bits());
        assert_eq!(csign(-2.0, 1.0).to_bits(), 2.0f32.to_bits());
        // NaN payload of `mag` survives; only the sign transfers.
        let q = f32::from_bits(0x7fc0_0055);
        assert_eq!(csign(q, -1.0).to_bits(), 0xffc0_0055);
    }
}

/// Shadow-transform uniform scale (0x613fd7..0x613fe9).
///
/// `s94 × s90 × sel` where `sel` is the override/base scale selected by the
/// model flag. The x87 fold `(s94 × s90) × sel` stays 80-bit, one narrow.
pub fn shadow_scale__613ef0(s94: f32, s90: f32, sel: f32) -> f32 {
    super::f64_to_f32(f64::from(s94) * f64::from(s90) * f64::from(sel))
}

/// Shadow bounds-center pre-scale sums (0x614031..0x614050).
///
/// The three model-data pairings `(md3+md0, md1+md4, md2+md5)`, each a
/// single-narrow `FADD`. The caller then halves the vector via the Scale hook.
pub fn shadow_bounds_center__613ef0(md: [f32; 6]) -> [f32; 3] {
    [
        super::f64_to_f32(f64::from(md[3]) + f64::from(md[0])),
        super::f64_to_f32(f64::from(md[1]) + f64::from(md[4])),
        super::f64_to_f32(f64::from(md[2]) + f64::from(md[5])),
    ]
}

#[cfg(test)]
mod tests_shadow_transform__613ef0 {
    use super::{shadow_bounds_center__613ef0 as center, shadow_scale__613ef0 as scale};

    #[test]
    fn scale_folds_three_factors() {
        assert_eq!(scale(2.0, 3.0, 4.0), 24.0);
    }

    #[test]
    fn bounds_center_pairs_model_data() {
        // md = [0,1,2,3,4,5] -> [3+0, 1+4, 2+5] = [3, 5, 7].
        assert_eq!(center([0.0, 1.0, 2.0, 3.0, 4.0, 5.0]), [3.0, 5.0, 7.0]);
    }
}
