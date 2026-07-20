//! `ui` family kernels.
#![allow(
    non_snake_case,
    // intentional NaN-reject via `!(a >= b)`
    // (differs from `a < b` on NaN), bit-exact source constants kept verbatim,
    // and ABI-dictated parameter counts.
    clippy::neg_cmp_op_on_partial_ord,
    clippy::excessive_precision,
    clippy::too_many_arguments
)]

/// Forward affine scale of a coordinate: `scale * x`.
pub fn os_gui_scale_x__41ae60(x: f32, scale: f32) -> f32 {
    scale * x
}

#[cfg(test)]
mod tests_os_gui_scale_x__41ae60 {
    use super::os_gui_scale_x__41ae60 as f;
    #[test]
    fn known_value() {
        assert_eq!(f(10.0, 0.8).to_bits(), 8.0f32.to_bits());
    }
    #[test]
    fn identity_scale_one() {
        assert_eq!(f(3.5, 1.0), 3.5);
    }
    #[test]
    fn zero_coord() {
        assert_eq!(f(0.0, 0.8), 0.0);
    }
    #[test]
    fn linear_in_coord() {
        // scale(2x) == 2*scale(x)
        let s = 0.6_f32;
        assert_eq!(f(4.0, s).to_bits(), (2.0 * f(2.0, s)).to_bits());
    }
}

/// Forward affine scale of a coordinate: `scale * y`.
pub fn os_gui_scale_y__41ae70(y: f32, scale: f32) -> f32 {
    scale * y
}

#[cfg(test)]
mod tests_os_gui_scale_y__41ae70 {
    use super::os_gui_scale_y__41ae70 as f;
    #[test]
    fn known_value() {
        assert_eq!(f(10.0, 0.6).to_bits(), 6.0f32.to_bits());
    }
    #[test]
    fn identity_scale_one() {
        assert_eq!(f(-2.25, 1.0), -2.25);
    }
    #[test]
    fn zero_coord() {
        assert_eq!(f(0.0, 0.6), 0.0);
    }
}

/// Inverse affine scale of a coordinate: `x / scale`.
pub fn os_gui_unscale_x__41ae40(x: f32, scale: f32) -> f32 {
    x / scale
}

#[cfg(test)]
mod tests_os_gui_unscale_x__41ae40 {
    use super::{os_gui_scale_x__41ae60 as scale, os_gui_unscale_x__41ae40 as unscale};
    #[test]
    fn known_value() {
        assert_eq!(unscale(8.0, 0.8).to_bits(), 10.0f32.to_bits());
    }
    #[test]
    fn inverse_of_scale() {
        // unscale(scale(x)) is x for a non-degenerate scale.
        let s = 0.8_f32;
        let x = 12.5_f32;
        let round_trip = unscale(scale(x, s), s);
        assert!((round_trip - x).abs() < 1e-3, "{round_trip}");
    }
    #[test]
    fn identity_scale_one() {
        assert_eq!(unscale(3.5, 1.0), 3.5);
    }
}

/// Inverse affine scale of a coordinate: `y / scale`.
pub fn os_gui_unscale_y__41ae50(y: f32, scale: f32) -> f32 {
    y / scale
}

#[cfg(test)]
mod tests_os_gui_unscale_y__41ae50 {
    use super::{os_gui_scale_y__41ae70 as scale, os_gui_unscale_y__41ae50 as unscale};
    #[test]
    fn known_value() {
        assert_eq!(unscale(6.0, 0.6).to_bits(), 10.0f32.to_bits());
    }
    #[test]
    fn inverse_of_scale() {
        let s = 0.6_f32;
        let y = -7.0_f32;
        let round_trip = unscale(scale(y, s), s);
        assert!((round_trip - y).abs() < 1e-3, "{round_trip}");
    }
    #[test]
    fn identity_scale_one() {
        assert_eq!(unscale(-2.25, 1.0), -2.25);
    }
}

/// Snaps a coordinate to the nearest integer after multiplying by an integer
/// device scale, rounding half away from zero, returning the rounded value as a
/// float. `scale` is the device pixel-scale read as a 32-bit integer.
///
/// `prod = scale * coord`; the result is `trunc(prod + 0.5)` when `prod > 0` and
/// `trunc(prod - 0.5)` otherwise — the add/sub-0.5-then-truncate-toward-zero
/// idiom (round half away from zero).
pub fn snap_coord_to_pixel_y__5c7010(coord: f32, scale: u32) -> f32 {
    let prod = scale as f32 * coord;
    let rounded = if prod > 0.0 {
        (prod + 0.5) as i32
    } else {
        (prod - 0.5) as i32
    };
    rounded as f32
}

#[cfg(test)]
mod tests_snap_coord_to_pixel_y__5c7010 {
    use super::snap_coord_to_pixel_y__5c7010 as snap;
    #[test]
    fn rounds_positive_half_up() {
        // scale 1: 2.5 -> trunc(2.5+0.5)=3
        assert_eq!(snap(2.5, 1), 3.0);
        assert_eq!(snap(2.4, 1), 2.0);
        assert_eq!(snap(2.6, 1), 3.0);
    }
    #[test]
    fn rounds_negative_half_away_from_zero() {
        assert_eq!(snap(-2.5, 1), -3.0);
        assert_eq!(snap(-2.4, 1), -2.0);
    }
    #[test]
    fn applies_integer_scale() {
        assert_eq!(snap(1.0, 3), 3.0);
        assert_eq!(snap(1.2, 2), 2.0);
    }
    #[test]
    fn zero_is_zero() {
        // prod == 0 takes the else branch: trunc(0 - 0.5) = trunc(-0.5) = 0
        assert_eq!(snap(0.0, 5), 0.0);
    }
    #[test]
    fn already_integer() {
        assert_eq!(snap(4.0, 1), 4.0);
        assert_eq!(snap(-4.0, 1), -4.0);
    }
}

/// Scans an obstacle-rectangle list for one that overlaps `rect` and, on the
/// first overlap, returns the clearance gap along `direction` (0..3). Returns
/// `None` when nothing overlaps (or the direction is out of range for every
/// overlapping rectangle).
///
/// Each rectangle is four floats `[a, b, c, d]`. Two rectangles overlap when
/// `rect[3] > obs[1]`, `rect[1] < obs[3]`, `rect[0] > obs[2]`, and
/// `rect[2] < obs[0]` (the bound-touching equality cases are NOT overlaps).
/// The clearance for each direction is: 0 -> `obs[0] - rect[2]`,
/// 1 -> `rect[3] - obs[1]`, 2 -> `obs[3] - rect[1]`, 3 -> `rect[0] - obs[2]`.
pub fn ui_frame_rect_overlap_distance__509220(
    rect: &[f32; 4],
    obstacles: &[[f32; 4]],
    direction: u32,
) -> Option<f32> {
    for obs in obstacles {
        let overlaps = rect[3] > obs[1] && rect[1] < obs[3] && rect[0] > obs[2] && rect[2] < obs[0];
        if !overlaps {
            continue;
        }
        match direction {
            0 => return Some(obs[0] - rect[2]),
            1 => return Some(rect[3] - obs[1]),
            2 => return Some(obs[3] - rect[1]),
            3 => return Some(rect[0] - obs[2]),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests_ui_frame_rect_overlap_distance__509220 {
    use super::ui_frame_rect_overlap_distance__509220 as f;

    // obs = [10, 2, 1, 8]; rect = [5, 5, 6, 5] satisfies all four strict
    // overlap inequalities (rect[3]=5>2, rect[1]=5<8, rect[0]=5>1, rect[2]=6<10).
    const OBS: [f32; 4] = [10.0, 2.0, 1.0, 8.0];
    const RECT: [f32; 4] = [5.0, 5.0, 6.0, 5.0];

    #[test]
    fn empty_list_no_overlap() {
        assert_eq!(f(&RECT, &[], 0), None);
    }
    #[test]
    fn non_overlapping_returns_none() {
        let obs = [[1.0_f32, 2.0, 1.0, 8.0]];
        assert_eq!(f(&RECT, &obs, 0), None);
    }
    #[test]
    fn overlap_distance_each_direction() {
        let obs = [OBS];
        assert_eq!(f(&RECT, &obs, 0), Some(4.0));
        assert_eq!(f(&RECT, &obs, 1), Some(3.0));
        assert_eq!(f(&RECT, &obs, 2), Some(3.0));
        assert_eq!(f(&RECT, &obs, 3), Some(4.0));
    }
    #[test]
    fn unknown_direction_falls_through() {
        let obs = [OBS];
        assert_eq!(f(&RECT, &obs, 7), None);
    }
    #[test]
    fn boundary_touch_is_not_overlap() {
        let mut rect = RECT;
        rect[0] = OBS[2]; // rect[0] > obs[2] becomes false (touch)
        let obs = [OBS];
        assert_eq!(f(&rect, &obs, 0), None);
        let mut rect2 = RECT;
        rect2[2] = OBS[0]; // rect[2] < obs[0] becomes false (touch)
        assert_eq!(f(&rect2, &obs, 0), None);
    }
    #[test]
    fn returns_first_overlap() {
        let obs = [OBS, [20.0, 2.0, 0.0, 9.0]];
        assert_eq!(f(&RECT, &obs, 0), Some(4.0));
    }
    #[test]
    fn skips_non_overlap_then_finds_overlap() {
        let far = [1.0_f32, 2.0, 1.0, 8.0];
        let obs = [far, OBS];
        assert_eq!(f(&RECT, &obs, 1), Some(3.0));
    }
}

/// Cohen-Sutherland-style clamp of a UI rect `(left, bottom, right, top)` to the
/// screen bounds, returning `(outcode, clamped)`.
///
/// `extent_x`/`extent_y` are the derived screen extents (cursor-scale minus the
/// screen-extent globals); `min_x`/`min_y` are the lower screen bounds. The
/// outcode bits are bit0 = `bottom < min_x`, bit1 = `extent_x < top`,
/// bit2 = `extent_y < left`, bit3 = `right < min_y` — all strict ordered
/// inequalities (matching the x87 `FCOMP`/`JP`/`JNZ` tests in the original).
///
/// `clamped` is the rect shifted to stay on-screen; the caller writes it only
/// when it actually performs the clamp (the outcode-only path discards it). The
/// height (`top - bottom`) and width (`left - right`) deltas come from the
/// original pre-clamp corners.
pub fn ui_frame_clamp_rect_to_screen__509bf0(
    left: f32,
    bottom: f32,
    right: f32,
    top: f32,
    extent_x: f32,
    extent_y: f32,
    min_x: f32,
    min_y: f32,
) -> (u8, [f32; 4]) {
    let height = top - bottom;
    let width = left - right;

    let mut outcode: u8 = 0;
    if bottom < min_x {
        outcode |= 1;
    }
    if extent_x < top {
        outcode |= 2;
    }
    if right < min_y {
        outcode |= 8;
    }
    if extent_y < left {
        outcode |= 4;
    }

    let mut l = left;
    let mut b = bottom;
    let mut r = right;
    let mut t = top;

    // Horizontal shift (bits 2|3): the left-out case takes priority.
    if outcode & 0xc != 0 {
        if outcode & 4 != 0 {
            r = extent_y - width;
            l = extent_y;
        } else if outcode & 8 != 0 {
            l = min_y + width;
            r = min_y;
        }
    }

    // Vertical shift (bits 0|1): the bottom-out case takes priority.
    if outcode & 0x3 != 0 {
        if outcode & 1 != 0 {
            b = min_x;
            t = min_x + height;
        } else if outcode & 2 != 0 {
            b = extent_x - height;
            t = extent_x;
        }
    }

    (outcode, [l, b, r, t])
}

#[cfg(test)]
mod tests_ui_frame_clamp_rect_to_screen__509bf0 {
    use super::ui_frame_clamp_rect_to_screen__509bf0 as clamp;

    /// Bit-exact equality of the four clamped corners.
    fn rect_bits_eq(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter()
            .zip(b.iter())
            .all(|(x, y)| x.to_bits() == y.to_bits())
    }

    /// A rect fully inside the screen bounds produces outcode 0 and is returned
    /// unchanged (no shift applied).
    #[test]
    fn inside_bounds_is_identity() {
        let rect = [10.0_f32, 10.0, 20.0, 20.0];
        let (outcode, clamped) = clamp(rect[0], rect[1], rect[2], rect[3], 100.0, 100.0, 0.0, 0.0);
        assert_eq!(outcode, 0);
        assert!(rect_bits_eq(clamped, rect));
    }

    /// Known value: bottom drops below `min_x`, so only bit0 is set and the rect
    /// is shifted up by exactly the bottom underflow, preserving its height.
    #[test]
    fn bottom_out_known_value() {
        let (outcode, clamped) = clamp(10.0, -5.0, 20.0, 5.0, 100.0, 100.0, 0.0, 0.0);
        assert_eq!(outcode, 1);
        assert!(rect_bits_eq(clamped, [10.0, 0.0, 20.0, 10.0]));
    }

    /// Known value: `left` exceeds `extent_y`, so only bit2 is set and the rect
    /// is shifted left to butt against the right extent, preserving its width.
    #[test]
    fn left_out_known_value() {
        let (outcode, clamped) = clamp(200.0, 10.0, 190.0, 20.0, 100.0, 100.0, 0.0, 0.0);
        assert_eq!(outcode, 4);
        assert!(rect_bits_eq(clamped, [100.0, 10.0, 90.0, 20.0]));
    }

    /// Outcode law: each bit is exactly its defining strict inequality, and only
    /// those four bits (1|2|4|8) can ever be set.
    #[test]
    fn outcode_bits_match_inequalities() {
        let cases = [
            (
                5.0_f32, 5.0_f32, 5.0_f32, 5.0_f32, 4.0_f32, 4.0_f32, 1.0_f32, 1.0_f32,
            ),
            (-1.0, -1.0, -1.0, -1.0, 10.0, 10.0, 0.0, 0.0),
            (20.0, 20.0, 20.0, 20.0, 1.0, 1.0, 0.0, 0.0),
            (3.0, 7.0, 2.0, 9.0, 8.0, 6.0, 4.0, 5.0),
        ];
        for &(left, bottom, right, top, ex, ey, mx, my) in &cases {
            let (outcode, _) = clamp(left, bottom, right, top, ex, ey, mx, my);
            let mut expected: u8 = 0;
            if bottom < mx {
                expected |= 1;
            }
            if ex < top {
                expected |= 2;
            }
            if ey < left {
                expected |= 4;
            }
            if right < my {
                expected |= 8;
            }
            assert_eq!(outcode, expected);
            assert_eq!(outcode & !0xf, 0);
        }
    }

    /// Metamorphic law: the shift operations are pure translations, so whenever a
    /// rect is clamped its width (`l - r`) and height (`t - b`) are preserved
    /// from the original corners on every shifted axis.
    #[test]
    fn shifts_preserve_extents() {
        let cases = [
            (
                10.0_f32, -5.0_f32, 20.0_f32, 5.0_f32, 100.0_f32, 100.0_f32, 0.0_f32, 0.0_f32,
            ),
            (200.0, 10.0, 190.0, 20.0, 100.0, 100.0, 0.0, 0.0),
            (-30.0, 50.0, -40.0, 60.0, 5.0, 5.0, 0.0, 0.0),
            (10.0, 80.0, 5.0, 95.0, 70.0, 70.0, 0.0, 0.0),
        ];
        for &(left, bottom, right, top, ex, ey, mx, my) in &cases {
            let width = left - right;
            let height = top - bottom;
            let (outcode, [l, b, r, t]) = clamp(left, bottom, right, top, ex, ey, mx, my);

            // Horizontal axis preserves width only when it was actually shifted.
            if outcode & 0xc != 0 {
                assert_eq!((l - r).to_bits(), width.to_bits());
            }
            // Vertical axis preserves height only when it was actually shifted.
            if outcode & 0x3 != 0 {
                assert_eq!((t - b).to_bits(), height.to_bits());
            }
        }
    }
}

/// Minimap blip projection (`Minimap__ProjectBlipDelta` @0x4eaa30): maps a
/// tracked object's world-XY delta from the minimap center into the blip's
/// 2-float local offset.
///
/// `k = 0.5·zoom_a·zoom_b` (the half-extent scale), `s = 1/range`;
/// `out.x = k − k·(obj_y − ctr_y)·s`, `out.y = k + k·(obj_x − ctr_x)·s`.
/// `ctr_z` (arg 3) is dead in the stock body (pushed, never read).
///
/// Rounding mirrors the stock x87 chain: `k` and the two-f32 subtractions are
/// exact in f64 (as they are in double-extended); the `out.y` operand is
/// rounded to f32 TWICE on its way through memory (`k·dx` stored, reloaded,
/// `·s` stored) while the `out.x` chain `k·dy·s` stays extended until the
/// final subtract — so the two lanes deliberately round differently. `s` and
/// the long products carry f64 (53-bit) where the FPU carried 64-bit; the
/// final f32 rounding absorbs that except in razor-edge cases (the accepted
/// f32/x87 precision class).
pub fn minimap__project_blip_delta__4eaa30(a: &[f32; 8]) -> [f32; 2] {
    let [ctr_x, ctr_y, _ctr_z, range, obj_x, obj_y, zoom_a, zoom_b] = *a;
    // FMUL·FMUL at extended precision, rounded once by the `FST` that
    // parks k in memory: f32·f32·0.5 is exact in f64, so one `as f32` matches.
    let k_d = f64::from(zoom_a) * f64::from(zoom_b) * 0.5;
    let k32 = k_d as f32;
    // out.y path: k·(obj_x − ctr_x) is FSTP'd to f32, reloaded, ·s, FSTP'd
    // again — two intermediate roundings, then the final two-f32 add.
    let m_y = (k_d * (f64::from(obj_x) - f64::from(ctr_x))) as f32;
    let s_d = 1.0 / f64::from(range);
    let t_y = (f64::from(m_y) * s_d) as f32;
    // out.x path: k·(obj_y − ctr_y)·s never touches memory — extended all the
    // way into the subtract, one rounding at the store.
    let t_x = k_d * (f64::from(obj_y) - f64::from(ctr_y)) * s_d;
    let out_x = (f64::from(k32) - t_x) as f32;
    let out_y = k32 + t_y;
    [out_x, out_y]
}

#[cfg(test)]
mod tests_minimap__project_blip_delta__4eaa30 {
    use super::minimap__project_blip_delta__4eaa30 as f;

    /// Object at the minimap center lands exactly on the blip-space center
    /// `(k, k)` regardless of range.
    #[test]
    fn centered_object_maps_to_k() {
        let k = 0.5f32 * 64.0 * 1.5;
        let out = f(&[100.0, 200.0, 7.0, 250.0, 100.0, 200.0, 64.0, 1.5]);
        assert_eq!(out[0].to_bits(), k.to_bits());
        assert_eq!(out[1].to_bits(), k.to_bits());
    }

    /// Known offsets: +y world delta moves out.x down by k·dy/range, +x world
    /// delta moves out.y up by k·dx/range.
    #[test]
    fn known_offsets() {
        // k = 0.5·10·2 = 10, s = 1/100. dy = +50 -> out.x = 10 − 10·50/100 = 5.
        let out = f(&[0.0, 0.0, 0.0, 100.0, 0.0, 50.0, 10.0, 2.0]);
        assert_eq!(out[0].to_bits(), 5.0f32.to_bits());
        assert_eq!(out[1].to_bits(), 10.0f32.to_bits());
        // dx = −25 -> out.y = 10 − 10·25/100 = 7.5.
        let out = f(&[0.0, 0.0, 0.0, 100.0, -25.0, 0.0, 10.0, 2.0]);
        assert_eq!(out[1].to_bits(), 7.5f32.to_bits());
    }

    /// arg 3 (`ctr_z`) is dead — any value, even NaN, must not affect the
    /// result (the stock body never reads its stack slot).
    #[test]
    fn ctr_z_is_dead() {
        let base = f(&[1.0, 2.0, 0.0, 30.0, 4.0, 5.0, 6.0, 7.0]);
        let nanz = f(&[1.0, 2.0, f32::NAN, 30.0, 4.0, 5.0, 6.0, 7.0]);
        assert_eq!(base[0].to_bits(), nanz[0].to_bits());
        assert_eq!(base[1].to_bits(), nanz[1].to_bits());
    }

    /// range = 0 divides to infinity (masked x87 semantics — no trap): the
    /// scaled terms become ±inf and the outputs follow IEEE through the
    /// add/sub.
    #[test]
    fn zero_range_flows_infinite() {
        let out = f(&[0.0, 0.0, 0.0, 0.0, 10.0, 10.0, 4.0, 1.0]);
        assert_eq!(out[0], f32::NEG_INFINITY);
        assert_eq!(out[1], f32::INFINITY);
    }

    /// NaN inputs propagate to both lanes (no compares anywhere in the body).
    #[test]
    fn nan_propagates() {
        let out = f(&[f32::NAN, 0.0, 0.0, 10.0, 1.0, 1.0, 2.0, 2.0]);
        assert!(out[1].is_nan());
        let out = f(&[0.0, f32::NAN, 0.0, 10.0, 1.0, 1.0, 2.0, 2.0]);
        assert!(out[0].is_nan());
    }

    /// The out.y intermediate really is double-rounded through f32 memory
    /// (matching the stock FSTP/reload), not carried extended like out.x:
    /// sweep inputs until the two chains disagree in the last ULP, proving
    /// the memory-store rounding is live in the kernel.
    #[test]
    fn out_y_uses_stored_f32_intermediate() {
        let mut found_divergence = false;
        for i in 1..5000u32 {
            let dx = 0.1f32 + i as f32 * 1.000_001e-3;
            let a = [0.0, 0.0, 0.0, 3.000_001, dx, 0.0, 1.000_000_1, 1.3];
            let k_d = f64::from(a[6]) * f64::from(a[7]) * 0.5;
            let s_d = 1.0 / f64::from(a[3]);
            // Fully-extended reference (what out.y would be WITHOUT the two
            // intermediate f32 stores).
            let single = ((k_d as f32) as f64 + k_d * f64::from(dx) * s_d) as f32;
            // The kernel's documented double-rounded chain.
            let m_y = (k_d * f64::from(dx)) as f32;
            let expect = (k_d as f32) + (f64::from(m_y) * s_d) as f32;
            assert_eq!(f(&a)[1].to_bits(), expect.to_bits(), "dx={dx}");
            if expect.to_bits() != single.to_bits() {
                found_divergence = true;
            }
        }
        assert!(
            found_divergence,
            "sweep never hit a double-rounding boundary — test inputs too tame"
        );
    }
}

/// Range gate of `Minimap__UnitBlipEnumCallback` 0x4eaa90 (block
/// 0x4eaae5): `d = pos − center`, then FCOMPP of `range²` vs
/// `(dz²+dy²)+dx²` (that FADDP order) with `TEST AH,0x5; JNP` — the
/// callback EXITS only on ordered `range² < dist²`; equality and
/// **unordered both continue** (NaN parity is 0x05 => PF=1 => no jump).
/// NB this corrects the wave-4 errata note that claimed NaN exits.
/// Subtractions/products/sums all run at x87 extended => f64 here.
pub fn blip_in_range__4eaa90(pos: &[f32; 3], center: &[f32; 3], range: f32) -> bool {
    let dx = f64::from(pos[0]) - f64::from(center[0]);
    let dy = f64::from(pos[1]) - f64::from(center[1]);
    let dz = f64::from(pos[2]) - f64::from(center[2]);
    let dist_sq = (dz * dz + dy * dy) + dx * dx;
    let r_sq = f64::from(range) * f64::from(range);
    !(r_sq < dist_sq)
}

#[cfg(test)]
mod tests_blip_in_range__4eaa90 {
    use super::blip_in_range__4eaa90 as in_range;

    #[test]
    fn range_polarity() {
        let c = [0.0f32, 0.0, 0.0];
        assert!(in_range(&[3.0, 0.0, 0.0], &c, 3.0)); // equal => continue
        assert!(in_range(&[1.0, 2.0, 2.0], &c, 3.0));
        assert!(!in_range(&[3.1, 0.0, 0.0], &c, 3.0));
    }

    #[test]
    fn nan_continues() {
        // Unordered FCOMPP leaves PF=1 => the exit jump is NOT taken.
        let c = [0.0f32, 0.0, 0.0];
        assert!(in_range(&[f32::NAN, 0.0, 0.0], &c, 3.0));
        assert!(in_range(&[1.0, 0.0, 0.0], &c, f32::NAN));
    }
}
