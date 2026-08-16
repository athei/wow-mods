//! `ui` family kernels.
// Adapter and kernel names mirror the host's C++ symbols verbatim, with `__`
// standing in for the `::`, so the whole module is non-snake-case by construction.
#![allow(non_snake_case)]

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

/// Snaps a coordinate to the nearest integer after multiplying by an integer device scale.
///
/// Rounding half away from zero, returning the rounded value as a float.
/// `scale` is the device pixel-scale read as a 32-bit integer.
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

/// Scans an obstacle-rectangle list for one that overlaps `rect`.
///
/// On the first overlap, returns the clearance gap along `direction` (0..3).
/// Returns `None` when nothing overlaps, and `None` without scanning at all when
/// the direction is out of range.
///
/// Each rectangle is four floats `[a, b, c, d]`. Two rectangles overlap when
/// `rect[3] > obs[1]`, `rect[1] < obs[3]`, `rect[0] > obs[2]`, and
/// `rect[2] < obs[0]` (the bound-touching equality cases are NOT overlaps).
/// The clearance for each direction is: 0 -> `obs[0] - rect[2]`,
/// 1 -> `rect[3] - obs[1]`, 2 -> `obs[3] - rect[1]`, 3 -> `rect[0] - obs[2]`.
///
/// `direction` does not change across the scan, so the range test is answered
/// once at the entry rather than at each overlapping rectangle. Out of range,
/// the only exit stock's jump table leaves open is the `None` below: the scan
/// stores nothing on the way there (`out_distance` is written by the caller only
/// on the `Some` arm), so skipping it returns the same value with the same set
/// of stores.
pub fn ui_frame_rect_overlap_distance__509220(
    rect: &[f32; 4],
    obstacles: &[[f32; 4]],
    direction: u32,
) -> Option<f32> {
    if direction > 3 {
        return None;
    }
    for obs in obstacles {
        let overlaps = rect[3] > obs[1] && rect[1] < obs[3] && rect[0] > obs[2] && rect[2] < obs[0];
        if !overlaps {
            continue;
        }
        match direction {
            0 => return Some(obs[0] - rect[2]),
            1 => return Some(rect[3] - obs[1]),
            2 => return Some(obs[3] - rect[1]),
            // The guard above leaves 3 as the only remaining direction.
            _ => return Some(rect[0] - obs[2]),
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

/// Cohen-Sutherland-style clamp of a UI rect `(left, bottom, right, top)` to the screen bounds.
///
/// Returning `(outcode, clamped)`.
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
// The eight parameters are the client's own argument list at 0x509bf0: the four
// rect edges first, then the derived screen extents and the lower screen bounds.
// Folding them into a params struct would break the stack the client hands us.
#[allow(clippy::too_many_arguments)]
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

    /// A rect fully inside the screen bounds produces outcode 0 and is returned unchanged.
    ///
    /// (no shift applied).
    #[test]
    fn inside_bounds_is_identity() {
        let rect = [10.0_f32, 10.0, 20.0, 20.0];
        let (outcode, clamped) = clamp(rect[0], rect[1], rect[2], rect[3], 100.0, 100.0, 0.0, 0.0);
        assert_eq!(outcode, 0);
        assert!(rect_bits_eq(clamped, rect));
    }

    /// Known value: bottom drops below `min_x`, so only bit0 is set.
    ///
    /// The rect is shifted up by exactly the bottom underflow, preserving its
    /// height.
    #[test]
    fn bottom_out_known_value() {
        let (outcode, clamped) = clamp(10.0, -5.0, 20.0, 5.0, 100.0, 100.0, 0.0, 0.0);
        assert_eq!(outcode, 1);
        assert!(rect_bits_eq(clamped, [10.0, 0.0, 20.0, 10.0]));
    }

    /// Known value: `left` exceeds `extent_y`, so only bit2 is set.
    ///
    /// The rect is shifted left to butt against the right extent, preserving
    /// its width.
    #[test]
    fn left_out_known_value() {
        let (outcode, clamped) = clamp(200.0, 10.0, 190.0, 20.0, 100.0, 100.0, 0.0, 0.0);
        assert_eq!(outcode, 4);
        assert!(rect_bits_eq(clamped, [100.0, 10.0, 90.0, 20.0]));
    }

    /// Outcode law: each bit is exactly its defining strict inequality.
    ///
    /// Only those four bits (1|2|4|8) can ever be set.
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

    /// Metamorphic law: the shift operations are pure translations.
    ///
    /// Whenever a rect is clamped its width (`l - r`) and height (`t - b`) are
    /// preserved from the original corners on every shifted axis.
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

/// Minimap blip projection (`Minimap__ProjectBlipDelta` @0x4eaa30).
///
/// Maps a tracked object's world-XY delta from the minimap center into the
/// blip's 2-float local offset.
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

    /// Object at the minimap center lands exactly on the blip-space center `(k, k)`.
    ///
    /// Regardless of range.
    #[test]
    fn centered_object_maps_to_k() {
        let k = 0.5f32 * 64.0 * 1.5;
        let out = f(&[100.0, 200.0, 7.0, 250.0, 100.0, 200.0, 64.0, 1.5]);
        assert_eq!(out[0].to_bits(), k.to_bits());
        assert_eq!(out[1].to_bits(), k.to_bits());
    }

    /// Known offsets: +y world delta moves out.x down by k·dy/range.
    ///
    /// +x world delta moves out.y up by k·dx/range.
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

    /// arg 3 (`ctr_z`) is dead.
    ///
    /// Any value, even NaN, must not affect the result (the stock body never
    /// reads its stack slot).
    #[test]
    fn ctr_z_is_dead() {
        let base = f(&[1.0, 2.0, 0.0, 30.0, 4.0, 5.0, 6.0, 7.0]);
        let nanz = f(&[1.0, 2.0, f32::NAN, 30.0, 4.0, 5.0, 6.0, 7.0]);
        assert_eq!(base[0].to_bits(), nanz[0].to_bits());
        assert_eq!(base[1].to_bits(), nanz[1].to_bits());
    }

    /// range = 0 divides to infinity (masked x87 semantics — no trap).
    ///
    /// The scaled terms become ±inf and the outputs follow IEEE through the
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

    /// The out.y intermediate really is double-rounded through f32 memory.
    ///
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

/// Range gate of `Minimap__UnitBlipEnumCallback` 0x4eaa90 (block 0x4eaae5).
///
/// `d = pos − center`, then FCOMPP of `range²` vs
/// `(dz²+dy²)+dx²` (that FADDP order) with `TEST AH,0x5; JNP` — the
/// callback EXITS only on ordered `range² < dist²`; equality and
/// **unordered both continue** (NaN parity is 0x05 => PF=1 => no jump).
/// NB this corrects the wave-4 errata note that claimed NaN exits.
/// Subtractions/products/sums all run at x87 extended => f64 here.
// `!(r_sq < dist_sq)` is the ordered FCOMPP test the callback branches on: an
// unordered compare leaves the exit jump untaken, so a NaN operand stays in
// range. The suggested `r_sq >= dist_sq` is false there, inverting that.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
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

/// Outcode to preferred-push-direction row index (stock bit dispatch at `0x509d80`).
///
/// The outcode bits are bit0 = off-left, bit1 = off-right, bit2 = off-top,
/// bit3 = off-bottom. The result indexes the 9-row direction-preference table
/// at `0x8087e0` (rows of four direction codes, `-1` padding), a 3x3
/// classification of where the rect left the screen.
pub const fn ui_frame_outcode_row__509d80(outcode: u32) -> u32 {
    if outcode & 1 != 0 {
        if outcode & 4 != 0 {
            return 8;
        }
        return ((outcode & 8) | 0x10) >> 2;
    }
    if outcode & 4 != 0 {
        return if outcode & 2 != 0 { 7 } else { 2 };
    }
    if outcode & 2 != 0 {
        return if outcode & 8 != 0 { 5 } else { 3 };
    }
    ((outcode & 0xff) >> 3) & 1
}

#[cfg(test)]
mod tests_ui_frame_outcode_row__509d80 {
    use super::ui_frame_outcode_row__509d80 as f;

    #[test]
    fn on_screen_is_row_zero() {
        assert_eq!(f(0), 0);
    }
    #[test]
    fn single_edges() {
        assert_eq!(f(1), 4); // off-left
        assert_eq!(f(2), 3); // off-right
        assert_eq!(f(4), 2); // off-top
        assert_eq!(f(8), 1); // off-bottom
    }
    #[test]
    fn corners() {
        assert_eq!(f(1 | 4), 8); // left + top
        assert_eq!(f(1 | 8), 6); // left + bottom
        assert_eq!(f(2 | 4), 7); // right + top
        assert_eq!(f(2 | 8), 5); // right + bottom
    }
}

/// Slides a floating rect `(top, left, bottom, right)` back onto the screen edge by edge.
///
/// Transcribes the in-place fixup at `0x509e20`: the width and height are taken
/// from the incoming corners, then the four edge cases run in order (top past
/// `screen_h`, bottom below `floor`, left below `floor`, right past
/// `screen_w`), each later step seeing the earlier stores. All compares are
/// strict ordered x87 tests; a NaN corner skips its step.
pub fn ui_frame_slide_rect_onto_screen__509e20(
    rect: &mut [f32; 4],
    screen_w: f32,
    screen_h: f32,
    floor: f32,
) {
    let height = rect[0] - rect[2];
    let width = rect[3] - rect[1];
    if rect[0] >= screen_h {
        rect[0] = screen_h;
        rect[2] = screen_h - height;
    }
    if rect[2] < floor {
        rect[2] = 0.0;
        rect[0] = height;
    }
    if rect[1] < floor {
        rect[1] = 0.0;
        rect[3] = width;
    }
    if rect[3] > screen_w {
        rect[3] = screen_w;
        rect[1] = screen_w - width;
    }
}

#[cfg(test)]
mod tests_ui_frame_slide_rect_onto_screen__509e20 {
    use super::ui_frame_slide_rect_onto_screen__509e20 as f;

    #[test]
    fn on_screen_untouched() {
        let mut r = [5.0f32, 1.0, 1.0, 3.0];
        f(&mut r, 10.0, 10.0, 0.0);
        assert_eq!(r, [5.0, 1.0, 1.0, 3.0]);
    }
    #[test]
    fn top_overflow_slides_down() {
        let mut r = [12.0f32, 1.0, 8.0, 3.0];
        f(&mut r, 10.0, 10.0, 0.0);
        assert_eq!(r, [10.0, 1.0, 6.0, 3.0]);
    }
    #[test]
    fn bottom_underflow_slides_up_to_zero() {
        let mut r = [1.0f32, 1.0, -3.0, 3.0];
        f(&mut r, 10.0, 10.0, 0.0);
        assert_eq!(r, [4.0, 1.0, 0.0, 3.0]);
    }
    #[test]
    fn left_then_right_order() {
        // Wider than the screen: the right-edge fix runs last and wins.
        let mut r = [5.0f32, -2.0, 1.0, 14.0];
        f(&mut r, 10.0, 10.0, 0.0);
        assert_eq!(r, [5.0, -6.0, 1.0, 10.0]);
    }
    #[test]
    fn top_equal_still_stores() {
        // The top test is `>=`: an exactly-fitting top is re-stored unchanged.
        let mut r = [10.0f32, 1.0, 2.0, 3.0];
        f(&mut r, 10.0, 10.0, 0.0);
        assert_eq!(r, [10.0, 1.0, 2.0, 3.0]);
    }
}

/// Returns the lowest-index obstacle rect overlapping `rect`, if any.
///
/// Same strict-inequality overlap test and scan order as
/// [`ui_frame_rect_overlap_distance__509220`]; factored out so the floating
/// placement search runs one scan per frontier rect instead of one per push
/// direction (the overlap verdict and the hit obstacle do not depend on the
/// direction, only the returned clearance does).
pub fn ui_frame_first_overlap<'a>(
    rect: &[f32; 4],
    obstacles: &'a [[f32; 4]],
) -> Option<&'a [f32; 4]> {
    obstacles
        .iter()
        .find(|obs| rect[3] > obs[1] && rect[1] < obs[3] && rect[0] > obs[2] && rect[2] < obs[0])
}

/// Reusable frontier and per-call readings for the placement search.
///
/// The frontier is owned here rather than allocated per call: it grows by up to
/// four rects per expansion against a budget in the hundreds, so a call-local
/// buffer reallocated through the process heap several times per placement.
///
/// Two other things were tried here and measured worse, so they are absent by
/// evidence rather than by oversight. A visited table pruning duplicate
/// candidates, and a screen-space bucket index replacing the linear scan in
/// [`ui_frame_first_overlap`], together roughly doubled the search: 8.07 us per
/// placement against 3.68-4.92 for this shape, across three densities. The
/// crowd is around 36 rects, which a tight linear scan handles faster than a
/// bucket query, and hashing four children per expansion costs more than the
/// duplicate expansions it avoids. Pruning also converts a cheap early
/// give-up into an expensive thorough search, which is a placement-quality
/// argument, not a speed one.
pub struct PlacementSearch {
    /// Frontier of candidate rects, consumed head-first.
    queue: Vec<[f32; 4]>,
    /// Candidate expansions the last call performed.
    expansions: u32,
    /// Largest frontier length the last call reached.
    high_water: u32,
    /// Whether the last call gave up against the expansion budget.
    hit_cap: bool,
}

impl PlacementSearch {
    /// A search with no frontier and no history.
    #[must_use]
    pub fn new() -> Self {
        Self {
            queue: Vec::new(),
            expansions: 0,
            high_water: 0,
            hit_cap: false,
        }
    }

    /// Candidate expansions the last call performed.
    #[must_use]
    pub const fn expansions(&self) -> u32 {
        self.expansions
    }

    /// Largest frontier length the last call reached.
    #[must_use]
    pub const fn high_water(&self) -> u32 {
        self.high_water
    }

    /// Whether the last call gave up against the expansion budget.
    #[must_use]
    pub const fn hit_cap(&self) -> bool {
        self.hit_cap
    }

    /// Resets the readings and empties the frontier, keeping its capacity.
    fn begin(&mut self) {
        self.queue.clear();
        self.expansions = 0;
        self.high_water = 0;
        self.hit_cap = false;
    }
}

impl Default for PlacementSearch {
    fn default() -> Self {
        Self::new()
    }
}

/// Derived screen bounds for the floating placement search.
///
/// `extent_x`/`extent_y` are the derived upper bounds (cursor scale minus the
/// margin globals at `0x8087d8`/`0x8087cc`); `min_x`/`min_y` are the lower
/// bounds (`0x8087d4`/`0x8087d0`).
pub struct FloatingScreenBounds {
    /// Derived upper x bound.
    pub extent_x: f32,
    /// Derived upper y bound.
    pub extent_y: f32,
    /// Lower x bound.
    pub min_x: f32,
    /// Lower y bound.
    pub min_y: f32,
}

/// First-fit placement search for a floating rect among screen obstacles.
///
/// Reimplements the frontier search at `0x5097a0`: starting from the clamped
/// `seed`, repeatedly take a candidate rect; discard it if any screen outcode
/// bit is set; otherwise find its lowest-index overlapping obstacle. With no
/// overlap the candidate is accepted as soon as the preference row holds any
/// usable direction code (0..=3). With an overlap, each usable direction
/// produces a child candidate pushed clear of that obstacle by the clearance
/// along that axis; a push absorbed entirely by f32 rounding equals its parent
/// and is accepted as-is. After `cap` candidate expansions, or when the
/// frontier empties, the search gives up and returns `src` unchanged.
///
/// Deviation from stock, by design: the stock frontier order depends on
/// whether search nodes come from the recycler or from a fresh allocation
/// (recycled nodes are prepended, fresh ones appended), so which of several
/// valid positions wins depends on allocator history. This search is a plain
/// FIFO over children pushed in row order, which is deterministic and equally
/// valid under the same push rules. Stock's per-node direction tag is dead
/// (every allocation resets it to `-1` and nothing ever writes it), so it is
/// dropped here.
pub fn ui_frame_place_floating_rect(
    src: &[f32; 4],
    seed: &[f32; 4],
    obstacles: &[[f32; 4]],
    dirs: &[i32; 4],
    cap: u32,
    bounds: &FloatingScreenBounds,
    search: &mut PlacementSearch,
) -> [f32; 4] {
    search.begin();
    search.queue.push(*seed);
    let mut head = 0usize;
    while head < search.queue.len() {
        if search.expansions >= cap {
            search.hit_cap = true;
            return *src;
        }
        search.expansions += 1;
        let rect = search.queue[head];
        head += 1;
        let (outcode, _) = ui_frame_clamp_rect_to_screen__509bf0(
            rect[0],
            rect[1],
            rect[2],
            rect[3],
            bounds.extent_x,
            bounds.extent_y,
            bounds.min_x,
            bounds.min_y,
        );
        if outcode != 0 {
            continue;
        }
        let Some(obs) = ui_frame_first_overlap(&rect, obstacles) else {
            if dirs.iter().any(|d| (0..=3).contains(d)) {
                return rect;
            }
            continue;
        };
        for &dir in dirs {
            let Ok(dir) = u32::try_from(dir) else {
                continue;
            };
            if dir > 3 {
                continue;
            }
            let clearance = match dir {
                0 => obs[0] - rect[2],
                1 => rect[3] - obs[1],
                2 => obs[3] - rect[1],
                _ => rect[0] - obs[2],
            };
            let mut child = rect;
            match dir {
                0 => {
                    child[0] += clearance;
                    child[2] += clearance;
                }
                1 => {
                    child[1] -= clearance;
                    child[3] -= clearance;
                }
                2 => {
                    child[1] += clearance;
                    child[3] += clearance;
                }
                _ => {
                    child[0] -= clearance;
                    child[2] -= clearance;
                }
            }
            if child == rect {
                return child;
            }
            search.queue.push(child);
            let len = search.queue.len() as u32;
            if len > search.high_water {
                search.high_water = len;
            }
        }
    }
    *src
}

#[cfg(test)]
mod tests_ui_frame_place_floating_rect {
    use super::{FloatingScreenBounds, PlacementSearch, ui_frame_place_floating_rect};

    const BOUNDS: FloatingScreenBounds = FloatingScreenBounds {
        extent_x: 100.0,
        extent_y: 100.0,
        min_x: 0.0,
        min_y: 0.0,
    };
    // Push up, then left, then right, then down.
    const DIRS: [i32; 4] = [0, 1, 2, 3];

    fn f(
        src: &[f32; 4],
        seed: &[f32; 4],
        obstacles: &[[f32; 4]],
        dirs: &[i32; 4],
        cap: u32,
        bounds: &FloatingScreenBounds,
    ) -> [f32; 4] {
        let mut search = PlacementSearch::new();
        ui_frame_place_floating_rect(src, seed, obstacles, dirs, cap, bounds, &mut search)
    }

    #[test]
    fn no_obstacles_accepts_seed() {
        let seed = [10.0f32, 1.0, 5.0, 4.0];
        assert_eq!(
            f(&[9.0, 9.0, 9.0, 9.0], &seed, &[], &DIRS, 16, &BOUNDS),
            seed
        );
    }
    #[test]
    fn all_directions_unusable_returns_src() {
        let src = [9.0f32, 9.0, 9.0, 9.0];
        let seed = [10.0f32, 1.0, 5.0, 4.0];
        assert_eq!(f(&src, &seed, &[], &[-1, -1, -1, -1], 16, &BOUNDS), src);
    }
    #[test]
    fn cap_zero_returns_src() {
        let src = [9.0f32, 9.0, 9.0, 9.0];
        let seed = [10.0f32, 1.0, 5.0, 4.0];
        assert_eq!(f(&src, &seed, &[], &DIRS, 0, &BOUNDS), src);
    }
    #[test]
    fn single_obstacle_pushes_clear_in_first_direction() {
        // Obstacle occupies y 0..20 fully across; seed overlaps it. Direction 0
        // pushes up by `obs.top - rect.bottom = 20 - 5 = 15`.
        let obs = [[20.0f32, 0.0, 0.0, 100.0]];
        let seed = [10.0f32, 1.0, 5.0, 4.0];
        let placed = f(&[0.0; 4], &seed, &obs, &DIRS, 16, &BOUNDS);
        assert_eq!(placed, [25.0, 1.0, 20.0, 4.0]);
    }
    #[test]
    fn off_screen_candidate_discarded() {
        // The up-push lands past the screen top, so the search falls through to
        // the left-push child instead.
        let obs = [[120.0f32, 0.0, 0.0, 50.0]];
        let seed = [90.0f32, 40.0, 85.0, 45.0];
        let placed = f(&[0.0; 4], &seed, &obs, &DIRS, 16, &BOUNDS);
        // Left push: rect.right - obs.left = 45 - 0 = 45 => left column lands
        // at [90, -5, 85, 0], off screen; right push: obs.right - rect.left =
        // 50 - 40 = 10 => [90, 50, 85, 55], on screen and overlap-free.
        assert_eq!(placed, [90.0, 50.0, 85.0, 55.0]);
    }
    #[test]
    fn cap_hit_is_reported() {
        let obs = [[20.0f32, 0.0, 0.0, 100.0]];
        let seed = [10.0f32, 1.0, 5.0, 4.0];
        let mut search = PlacementSearch::new();
        let src = [9.0f32, 9.0, 9.0, 9.0];
        let placed =
            ui_frame_place_floating_rect(&src, &seed, &obs, &DIRS, 0, &BOUNDS, &mut search);
        assert_eq!(placed, src);
        assert!(search.hit_cap());
        assert_eq!(search.expansions(), 0);
    }
    #[test]
    fn readings_count_the_expansions() {
        let seed = [10.0f32, 1.0, 5.0, 4.0];
        let mut search = PlacementSearch::new();
        // No obstacles: the seed is accepted on the first expansion, and the
        // frontier never grows past the seed itself.
        ui_frame_place_floating_rect(&[0.0; 4], &seed, &[], &DIRS, 16, &BOUNDS, &mut search);
        assert_eq!(search.expansions(), 1);
        assert!(!search.hit_cap());
        assert_eq!(search.high_water(), 0);
        // One obstacle in the way: the seed expands into children, so the
        // frontier high-water mark records them.
        let obs = [[20.0f32, 0.0, 0.0, 100.0]];
        ui_frame_place_floating_rect(&[0.0; 4], &seed, &obs, &DIRS, 16, &BOUNDS, &mut search);
        assert!(search.high_water() > 1);
    }
    /// The search as it stood before the buckets and the visited table.
    ///
    /// Kept as the oracle for those two deviations: a plain FIFO frontier with
    /// duplicates pushed freely and a linear scan for the overlapping obstacle.
    /// Reports whether it gave up against the budget, because that is the one
    /// case where pruning is allowed to reach a different answer.
    fn reference(
        src: &[f32; 4],
        seed: &[f32; 4],
        obstacles: &[[f32; 4]],
        dirs: &[i32; 4],
        cap: u32,
        bounds: &FloatingScreenBounds,
    ) -> ([f32; 4], bool) {
        let mut queue: Vec<[f32; 4]> = vec![*seed];
        let mut head = 0usize;
        let mut expansions = 0u32;
        while head < queue.len() {
            if expansions >= cap {
                return (*src, true);
            }
            expansions += 1;
            let rect = queue[head];
            head += 1;
            let (outcode, _) = super::ui_frame_clamp_rect_to_screen__509bf0(
                rect[0],
                rect[1],
                rect[2],
                rect[3],
                bounds.extent_x,
                bounds.extent_y,
                bounds.min_x,
                bounds.min_y,
            );
            if outcode != 0 {
                continue;
            }
            let Some(obs) = super::ui_frame_first_overlap(&rect, obstacles) else {
                if dirs.iter().any(|d| (0..=3).contains(d)) {
                    return (rect, false);
                }
                continue;
            };
            for &dir in dirs {
                let Ok(dir) = u32::try_from(dir) else {
                    continue;
                };
                if dir > 3 {
                    continue;
                }
                let clearance = match dir {
                    0 => obs[0] - rect[2],
                    1 => rect[3] - obs[1],
                    2 => obs[3] - rect[1],
                    _ => rect[0] - obs[2],
                };
                let mut child = rect;
                match dir {
                    0 => {
                        child[0] += clearance;
                        child[2] += clearance;
                    }
                    1 => {
                        child[1] -= clearance;
                        child[3] -= clearance;
                    }
                    2 => {
                        child[1] += clearance;
                        child[3] += clearance;
                    }
                    _ => {
                        child[0] -= clearance;
                        child[2] -= clearance;
                    }
                }
                if child == rect {
                    return (child, false);
                }
                queue.push(child);
            }
        }
        (*src, false)
    }

    const CROWD_BOUNDS: FloatingScreenBounds = FloatingScreenBounds {
        extent_x: 1000.0,
        extent_y: 800.0,
        min_x: 0.0,
        min_y: 0.0,
    };

    fn next(state: &mut u32) -> f32 {
        *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        f32::from(((*state >> 16) & 0x3ff) as u16)
    }

    fn plate(state: &mut u32) -> [f32; 4] {
        let left = next(state);
        let bottom = next(state) * 0.7;
        [bottom + 24.0, left, bottom, left + 110.0]
    }

    #[test]
    fn a_reused_frontier_places_exactly_as_a_fresh_one() {
        // Carrying the frontier across calls is the one thing that could make a
        // placement depend on the call before it. Checked against a reference
        // that allocates a fresh queue every time, over a crowd the size of a
        // downtown scene, with the crowd growing as the frame appends each rect
        // it has placed. Equality is unconditional: the search shape is the
        // reference's, only the buffer's lifetime differs.
        let mut state = 0xa1b2_c3d4u32;
        let mut obstacles: Vec<[f32; 4]> = Vec::new();
        let mut search = PlacementSearch::new();
        for _ in 0..200 {
            let seed = plate(&mut state);
            let src = plate(&mut state);
            let (want, _) = reference(&src, &seed, &obstacles, &DIRS, 512, &CROWD_BOUNDS);
            let got = ui_frame_place_floating_rect(
                &src,
                &seed,
                &obstacles,
                &DIRS,
                512,
                &CROWD_BOUNDS,
                &mut search,
            );
            assert_eq!(got, want, "crowd of {}", obstacles.len());
            // The frame appends every rect it places, as the hook does.
            obstacles.push(got);
        }
    }
    #[test]
    fn state_is_reusable_across_calls() {
        let seed = [10.0f32, 1.0, 5.0, 4.0];
        let obs = [[20.0f32, 0.0, 0.0, 100.0]];
        let mut search = PlacementSearch::new();
        let first =
            ui_frame_place_floating_rect(&[0.0; 4], &seed, &obs, &DIRS, 16, &BOUNDS, &mut search);
        let second =
            ui_frame_place_floating_rect(&[0.0; 4], &seed, &obs, &DIRS, 16, &BOUNDS, &mut search);
        assert_eq!(first, second);
    }
}
