// Adapter and kernel names mirror the host's C++ symbols verbatim, with `__`
// standing in for the `::`, so the whole module is non-snake-case by construction.
#![allow(non_snake_case)]

/// Builds a 9x9 grid of vertex positions for a terrain chunk cell block.
///
/// X is the row index scaled by `row_scale`, Y the column index scaled by
/// `col_scale`, and Z the per-vertex sampled height minus a chunk base height.
/// The row/column scales are derived from `cell`, `origin`, and `divisor` exactly
/// as the reference does (an `origin`-anchored difference rather than the
/// algebraically-equal `cell` directly), preserving the f32 rounding of the large
/// `origin` constant. The 81 height samples are supplied one per vertex in
/// row-major order.
pub fn c_map_chunk__build_grid_vertices__68d540(
    count_a: i32,
    count_b: i32,
    cell: f32,
    origin: f32,
    divisor: f32,
    base_height: f32,
    heights: &[f32; 81],
) -> [[f32; 3]; 81] {
    // row_scale from count_a, col_scale from count_b — each is
    // -((origin - i*cell) - (origin - (i+1)*cell)) / divisor, computed via the
    // origin-anchored intermediates to match the reference's f32 rounding.
    let row_scale = grid_scale(count_a, cell, origin, divisor);
    let col_scale = grid_scale(count_b, cell, origin, divisor);

    core::array::from_fn(|i| {
        let row = (i / 9) as i32;
        let col = (i % 9) as i32;
        let x = (row as f32) * row_scale;
        let y = (col as f32) * col_scale;
        let z = heights[i] - base_height;
        [x, y, z]
    })
}

/// Per-axis grid spacing: `-((origin - i*cell) - (origin - (i+1)*cell)) / divisor`.
fn grid_scale(i: i32, cell: f32, origin: f32, divisor: f32) -> f32 {
    let lo = -((i as f32) * cell) + origin;
    let hi = -(((i + 1) as f32) * cell) + origin;
    -(lo - hi) / divisor
}

#[cfg(test)]
mod tests_c_map_chunk__build_grid_vertices__68d540 {
    use super::{c_map_chunk__build_grid_vertices__68d540 as build, grid_scale};

    fn flat_heights(v: f32) -> [f32; 81] {
        [v; 81]
    }

    #[test]
    fn scale_is_negative_cell_over_divisor() {
        // For modest indices the origin-anchored difference is exactly `cell`.
        let s = grid_scale(3, 2.0, 100.0, 4.0);
        assert!((s - (-2.0 / 4.0)).abs() < 1e-6, "{s}");
    }

    #[test]
    fn z_is_height_minus_base() {
        let mut h = [0.0f32; 81];
        for (k, e) in h.iter_mut().enumerate() {
            *e = k as f32;
        }
        let g = build(0, 0, 1.0, 0.0, 1.0, 10.0, &h);
        for (k, v) in g.iter().enumerate() {
            assert!((v[2] - (k as f32 - 10.0)).abs() < 1e-6, "idx {k}");
        }
    }

    #[test]
    fn xy_row_col_structure() {
        // With unit cell, zero origin, unit divisor: scale = -1.
        let g = build(0, 0, 1.0, 0.0, 1.0, 0.0, &flat_heights(0.0));
        for (i, gi) in g.iter().enumerate() {
            let row = (i / 9) as f32;
            let col = (i % 9) as f32;
            assert!((gi[0] - (-row)).abs() < 1e-6, "x idx {i}");
            assert!((gi[1] - (-col)).abs() < 1e-6, "y idx {i}");
        }
    }

    #[test]
    fn x_constant_per_row_y_constant_per_col() {
        let g = build(5, 7, 3.0, 17066.666, 8.0, 1.0, &flat_heights(2.0));
        // Within a row, X identical; within a column, Y identical.
        for row in 0..9 {
            let base = g[row * 9][0];
            for col in 0..9 {
                assert_eq!(g[row * 9 + col][0].to_bits(), base.to_bits(), "row {row}");
            }
        }
        for col in 0..9 {
            let base = g[col][1];
            for row in 0..9 {
                assert_eq!(g[row * 9 + col][1].to_bits(), base.to_bits(), "col {col}");
            }
        }
    }

    #[test]
    fn corner_zero_vertex() {
        // Vertex (0,0): X = 0*scale = 0, Y = 0*scale = 0, Z = h[0]-base.
        let mut h = flat_heights(0.0);
        h[0] = 42.0;
        let g = build(2, 3, 5.0, 100.0, 2.0, 8.0, &h);
        // 0 * scale is +/-0.0 depending on the scale's sign; value-equal to zero.
        assert_eq!(g[0][0], 0.0);
        assert_eq!(g[0][1], 0.0);
        assert!((g[0][2] - (42.0 - 8.0)).abs() < 1e-6);
    }

    #[test]
    fn row_scale_independent_of_count_when_exact() {
        // Both scales reduce to -cell/divisor for small counts regardless of count.
        let g1 = build(1, 1, 4.0, 0.0, 2.0, 0.0, &flat_heights(0.0));
        let g2 = build(9, 13, 4.0, 0.0, 2.0, 0.0, &flat_heights(0.0));
        // last row/col index 8 -> X/Y identical across the two builds.
        assert!((g1[80][0] - g2[80][0]).abs() < 1e-4);
        assert!((g1[80][1] - g2[80][1]).abs() < 1e-4);
    }
}

/// Result of mapping a world XY into the liquid chunk grid.
///
/// The flat grid index, the per-chunk sub-cell index, and the row/bit position
/// into that chunk's liquid bitfield.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LiquidGridLookup {
    pub grid_index: usize,
    pub sub_index: usize,
    pub liquid_row: u32,
    pub bit_index: u32,
}

/// Maps a world-space XY into the loaded-terrain liquid chunk grid.
///
/// Returns `None` when the point falls outside the grid's valid band
/// (`lo_bound <= d < hi_bound` on each axis, where `d = origin - coord`).
/// Otherwise returns the flat 64x64 grid index, the per-chunk 16x16 sub-cell
/// index, and the row/bit indices into the chunk's 64x64 liquid bitfield. The
/// caller performs the (live, mutable) grid and chunk dereferences and the final
/// bit test.
// The eight scalars are the reference's argument list, not a design choice. The
// band tests are written `!(d >= lo_bound)` / `!(d < hi_bound)` so a NaN
// coordinate rejects exactly as the x87 compare chain did; the positive forms
// would accept it.
#[allow(clippy::too_many_arguments, clippy::neg_cmp_op_on_partial_ord)]
pub fn c_world__point_in_liquid_grid__69b350(
    px: f32,
    py: f32,
    origin: f32,
    lo_bound: f32,
    hi_bound: f32,
    scale_cell: f32,
    scale_sub: f32,
    bias: f32,
) -> Option<LiquidGridLookup> {
    // d_y from py, d_x from px; both anchored at `origin`.
    let dy = -(py - origin);
    let dx = -(px - origin);

    // Inclusive lower bound, exclusive upper bound, on both axes.
    if !(dy >= lo_bound) || !(dx >= lo_bound) {
        return None;
    }
    if !(dy < hi_bound) || !(dx < hi_bound) {
        return None;
    }

    // round-to-nearest (FISTP) then bias-subtract, as integers.
    let idx_y = round_i32(scale_cell * dy - bias);
    let idx_x = round_i32(scale_cell * dx - bias);

    let grid_index = (((idx_x >> 4) & 0x3f) * 0x40 + ((idx_y >> 4) & 0x3f)) as usize;
    let sub_index = ((idx_x & 0xf) * 0x10 + (idx_y & 0xf)) as usize;

    let sub_y = round_i32(scale_sub * dx - bias);
    let sub_x = round_i32(scale_sub * dy - bias);

    Some(LiquidGridLookup {
        grid_index,
        sub_index,
        liquid_row: (sub_y & 0x3f) as u32,
        bit_index: (sub_x & 0x3f) as u32,
    })
}

/// Round-half-to-even to i32, matching x87 `FISTP` under the default rounding mode.
fn round_i32(v: f32) -> i32 {
    v.round_ties_even() as i32
}

/// Tests a single liquid bit given the chunk's bitfield byte at the computed offset.
///
/// `byte` is `bitfield[liquid_row*8 + (bit_index >> 3)]`; the result is
/// `(byte & (1 << (bit_index & 7))) != 0`.
pub fn liquid_bit_set(byte: u8, bit_index: u32) -> bool {
    (byte & (1u8 << (bit_index & 7))) != 0
}

#[cfg(test)]
mod tests_c_world__point_in_liquid_grid__69b350 {
    use super::{
        LiquidGridLookup, c_world__point_in_liquid_grid__69b350 as lookup, liquid_bit_set,
    };

    // Realistic-ish constants: origin large, small scales.
    const ORIGIN: f32 = 17066.666;
    const LO: f32 = 0.0;
    const HI: f32 = 34133.332;
    const SCALE_CELL: f32 = 0.0009375;
    const SCALE_SUB: f32 = 0.015;
    const BIAS: f32 = 0.5;

    #[test]
    fn out_of_band_low_returns_none() {
        // dx = origin - px; choose px so dx < LO (px > origin).
        assert_eq!(
            lookup(
                ORIGIN + 1.0,
                ORIGIN,
                ORIGIN,
                LO,
                HI,
                SCALE_CELL,
                SCALE_SUB,
                BIAS
            ),
            None
        );
    }

    #[test]
    fn out_of_band_high_returns_none() {
        // dx = origin - px >= HI when px <= origin - HI.
        let px = ORIGIN - HI - 1.0;
        assert_eq!(
            lookup(px, ORIGIN, ORIGIN, LO, HI, SCALE_CELL, SCALE_SUB, BIAS),
            None
        );
    }

    #[test]
    fn in_band_returns_some() {
        // Point at the origin: d = 0, in [LO, HI).
        let r = lookup(ORIGIN, ORIGIN, ORIGIN, LO, HI, SCALE_CELL, SCALE_SUB, BIAS);
        assert!(r.is_some(), "{r:?}");
    }

    #[test]
    fn lower_bound_inclusive_upper_exclusive() {
        // d == LO is in-band (inclusive); d == HI is out (exclusive).
        let at_lo = lookup(
            ORIGIN - LO,
            ORIGIN,
            ORIGIN,
            LO,
            HI,
            SCALE_CELL,
            SCALE_SUB,
            BIAS,
        );
        assert!(at_lo.is_some());
        let at_hi = lookup(
            ORIGIN - HI,
            ORIGIN,
            ORIGIN,
            LO,
            HI,
            SCALE_CELL,
            SCALE_SUB,
            BIAS,
        );
        assert_eq!(at_hi, None);
    }

    #[test]
    fn indices_in_expected_ranges() {
        let r = lookup(
            ORIGIN - 1000.0,
            ORIGIN - 2000.0,
            ORIGIN,
            LO,
            HI,
            SCALE_CELL,
            SCALE_SUB,
            BIAS,
        )
        .unwrap();
        assert!(r.grid_index < 0x40 * 0x40);
        assert!(r.sub_index < 0x10 * 0x10);
        assert!(r.liquid_row < 0x40);
        assert!(r.bit_index < 0x40);
    }

    #[test]
    fn known_index_decomposition() {
        // scale_cell=1, bias=0, origin=0 -> idx = round(-coord).
        let r = lookup(-50.0, -33.0, 0.0, -1e9, 1e9, 1.0, 1.0, 0.0).unwrap();
        // dx = 50, dy = 33; idx_x = 50, idx_y = 33.
        let idx_x = 50i32;
        let idx_y = 33i32;
        let want_grid = (((idx_x >> 4) & 0x3f) * 0x40 + ((idx_y >> 4) & 0x3f)) as usize;
        let want_sub = ((idx_x & 0xf) * 0x10 + (idx_y & 0xf)) as usize;
        assert_eq!(r.grid_index, want_grid);
        assert_eq!(r.sub_index, want_sub);
        // sub_x = round(scale_sub*dy) = 33 -> bit_index; sub_y = round(scale_sub*dx)=50 -> liquid_row.
        assert_eq!(r.liquid_row, (50i32 & 0x3f) as u32);
        assert_eq!(r.bit_index, (33i32 & 0x3f) as u32);
    }

    #[test]
    fn bit_test_known() {
        assert!(liquid_bit_set(0b0000_1000, 3));
        assert!(!liquid_bit_set(0b0000_1000, 2));
        // bit_index uses only low 3 bits for the mask.
        assert!(liquid_bit_set(0b0000_0001, 8));
    }

    #[test]
    fn lookup_struct_eq() {
        let a = LiquidGridLookup {
            grid_index: 1,
            sub_index: 2,
            liquid_row: 3,
            bit_index: 4,
        };
        let b = a;
        assert_eq!(a, b);
    }
}

/// Normalizes a screen-space cursor into NDC against the viewport rect.
///
/// Gates it against the inclusive `[lo, hi]` bounds. `None` when outside the
/// viewport.
// The cursor pair, the four viewport edges and the two NDC bounds are the
// reference's argument list. The gate is written as negated ordered compares so
// a NaN cursor or viewport rejects, which the positive forms would not.
#[allow(clippy::too_many_arguments, clippy::neg_cmp_op_on_partial_ord)]
pub fn c_world_frame__unproject_cursor_to_world_ray__4813b0(
    screen_x: f32,
    screen_y: f32,
    vp_x0: f32,
    vp_x1: f32,
    vp_y0: f32,
    vp_y1: f32,
    lo: f32,
    hi: f32,
) -> Option<NdcCursor> {
    let ndc_x = (screen_x - vp_x0) / (vp_x1 - vp_x0);
    let ndc_y = (screen_y - vp_y0) / (vp_y1 - vp_y0);
    // Reject when ndc < lo (strict) or ndc > hi, so a pass requires lo<=ndc<=hi
    // on both axes (matches the x87 `FCOMP`/`TEST AH,0x5`/`TEST AH,0x41` tests).
    if !(ndc_x >= lo) || !(ndc_y >= lo) || !(ndc_x <= hi) || !(ndc_y <= hi) {
        return None;
    }
    Some(NdcCursor { ndc_x, ndc_y })
}

/// Normalized-device cursor coordinates produced by the unprojection gate.
pub struct NdcCursor {
    pub ndc_x: f32,
    pub ndc_y: f32,
}

/// Translates a ray endpoint (3 floats) by the camera origin in place.
pub fn unproject_ray_translate(ray: &mut [f32; 3], origin: &[f32; 3]) {
    ray[0] += origin[0];
    ray[1] += origin[1];
    ray[2] += origin[2];
}

#[cfg(test)]
mod tests_c_world_frame__unproject_cursor_to_world_ray__4813b0 {
    use super::{
        NdcCursor, c_world_frame__unproject_cursor_to_world_ray__4813b0, unproject_ray_translate,
    };

    // A symmetric `[-1, 1]` NDC gate over a 200x100 viewport at the origin.
    const LO: f32 = -1.0;
    const HI: f32 = 1.0;
    const VP_X0: f32 = 0.0;
    const VP_X1: f32 = 200.0;
    const VP_Y0: f32 = 0.0;
    const VP_Y1: f32 = 100.0;

    fn gate(sx: f32, sy: f32) -> Option<NdcCursor> {
        c_world_frame__unproject_cursor_to_world_ray__4813b0(
            sx, sy, VP_X0, VP_X1, VP_Y0, VP_Y1, LO, HI,
        )
    }

    // Known value: the viewport center maps to NDC (0.5, 0.5), inside [-1, 1].
    #[test]
    fn center_is_inside_with_expected_ndc() {
        let ndc = gate(100.0, 50.0).expect("center is inside the gate");
        assert_eq!(ndc.ndc_x.to_bits(), 0.5_f32.to_bits());
        assert_eq!(ndc.ndc_y.to_bits(), 0.5_f32.to_bits());
    }

    // Known value: screen (0, 0) maps to NDC (0, 0), the lower-left corner.
    #[test]
    fn origin_maps_to_zero_ndc() {
        let ndc = gate(0.0, 0.0).expect("origin is inside the gate");
        assert_eq!(ndc.ndc_x.to_bits(), 0.0_f32.to_bits());
        assert_eq!(ndc.ndc_y.to_bits(), 0.0_f32.to_bits());
    }

    // Boundary law: the inclusive bounds pass exactly at `lo` and `hi`.
    // screen (-200, -100) -> ndc (-1, -1) = lo; screen (200, 100) -> ndc (1, 1) = hi.
    #[test]
    fn inclusive_bounds_pass_at_edges() {
        // ndc = lo exactly on both axes.
        assert!(gate(-200.0, -100.0).is_some());
        // ndc = hi exactly on both axes.
        assert!(gate(200.0, 100.0).is_some());
    }

    // Metamorphic law: a point just below `lo` on either axis is rejected.
    #[test]
    fn below_lo_rejected_per_axis() {
        // ndc_x < lo (-1): screen_x = -210 -> ndc_x = -1.05.
        assert!(gate(-210.0, 50.0).is_none());
        // ndc_y < lo (-1): screen_y = -110 -> ndc_y = -1.1.
        assert!(gate(100.0, -110.0).is_none());
    }

    // Metamorphic law: a point just above `hi` on either axis is rejected.
    #[test]
    fn above_hi_rejected_per_axis() {
        // ndc_x > hi (1): screen_x = 410 -> ndc_x = 2.05.
        assert!(gate(410.0, 50.0).is_none());
        // ndc_y > hi (1): screen_y = 210 -> ndc_y = 2.1.
        assert!(gate(100.0, 210.0).is_none());
    }

    // Metamorphic law: a degenerate zero-width viewport divides by zero, so the
    // gate never finds the result within bounds and rejects it.
    #[test]
    fn degenerate_viewport_rejected() {
        // vp_x1 == vp_x0 -> division by zero -> NaN/inf, never within bounds.
        let out = c_world_frame__unproject_cursor_to_world_ray__4813b0(
            10.0, 50.0, 0.0, 0.0, VP_Y0, VP_Y1, LO, HI,
        );
        assert!(out.is_none());
    }

    // Translation law: adding the origin shifts each component by that amount;
    // adding a zero origin is the identity.
    #[test]
    fn translate_adds_origin_componentwise() {
        let mut ray = [1.0_f32, 2.0_f32, 3.0_f32];
        let origin = [10.0_f32, 20.0_f32, 30.0_f32];
        unproject_ray_translate(&mut ray, &origin);
        for (got, want) in ray.iter().zip([11.0_f32, 22.0_f32, 33.0_f32].iter()) {
            assert_eq!(got.to_bits(), want.to_bits());
        }

        let mut id = [-4.5_f32, 0.0_f32, 7.25_f32];
        let before = id;
        unproject_ray_translate(&mut id, &[0.0_f32, 0.0_f32, 0.0_f32]);
        for (got, want) in id.iter().zip(before.iter()) {
            assert_eq!(got.to_bits(), want.to_bits());
        }
    }
}

/// Maps a world-space XY point into a liquid heightfield cell.
///
/// `scale` and the chunk grid origin define the affine map; the integer cell is
/// the truncation of the scaled offset (matching the reference's `__ftol`), and
/// `fx`/`fy` are the in-cell fractional weights. Returns `None` when the cell
/// falls outside the `dim_x` x `dim_y` grid.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LiquidCell {
    pub ix: i32,
    pub iy: i32,
    pub fx: f32,
    pub fy: f32,
}

pub fn liquid_cell_map(
    px: f32,
    py: f32,
    origin_x: f32,
    origin_y: f32,
    scale: f32,
    dim_x: i32,
    dim_y: i32,
) -> Option<LiquidCell> {
    let gx = scale * (px - origin_x);
    let gy = scale * (py - origin_y);
    // `__ftol` truncates toward zero.
    let ix = gx as i32;
    let iy = gy as i32;
    if ix < 0 || iy < 0 || ix >= dim_x || iy >= dim_y {
        return None;
    }
    Some(LiquidCell {
        ix,
        iy,
        fx: gx - ix as f32,
        fy: gy - iy as f32,
    })
}

/// Bilinear blend of four heightfield samples.
///
/// `row0` is the cell's near-row `(lo, hi)` height pair, `row1` the far row;
/// `fx`/`fy` are the in-cell weights. Mirrors the reference's two horizontal
/// lerps followed by one vertical lerp.
#[inline]
pub fn liquid_blend_height(row0: (f32, f32), row1: (f32, f32), fx: f32, fy: f32) -> f32 {
    let h0 = (row0.1 - row0.0) * fx + row0.0;
    let h1 = (row1.1 - row1.0) * fx + row1.0;
    (h1 - h0) * fy + h0
}

/// Result of a liquid height-at-point test for one cell `kind` (`cellFlags & 3`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LiquidHit {
    /// The interpolated surface lies above `point.z`.
    ///
    /// Carries the (unbiased) surface height to report.
    Hit(f32),
    /// The surface is at or below `point.z` (or the kind has no surface arm).
    Miss,
}

/// Tests one liquid cell kind.
///
/// Kind 0 biases the surface by `bias` only for the comparison; kinds 2 and 3
/// compare the raw height. Any other kind misses. A hit requires the (compared)
/// surface to be strictly above `point_z`.
pub fn liquid_test_kind(kind: u32, height: f32, bias: f32, point_z: f32) -> LiquidHit {
    let compared = match kind {
        0 => height + bias,
        2 | 3 => height,
        _ => return LiquidHit::Miss,
    };
    if compared > point_z {
        LiquidHit::Hit(height)
    } else {
        LiquidHit::Miss
    }
}

#[cfg(test)]
mod c_world_liquid__query_height_at_point_tests {
    use super::{LiquidHit, liquid_blend_height, liquid_cell_map, liquid_test_kind};

    #[test]
    fn cell_map_truncates_and_bounds() {
        let c = liquid_cell_map(3.25, 5.75, 0.0, 0.0, 1.0, 8, 8).unwrap();
        assert_eq!((c.ix, c.iy), (3, 5));
        assert!((c.fx - 0.25).abs() < 1e-6 && (c.fy - 0.75).abs() < 1e-6);
        assert!(liquid_cell_map(9.0, 0.0, 0.0, 0.0, 1.0, 8, 8).is_none());
        assert!(liquid_cell_map(-1.0, 0.0, 0.0, 0.0, 1.0, 8, 8).is_none());
    }

    #[test]
    fn blend_corners_are_exact() {
        let r0 = (10.0, 20.0);
        let r1 = (30.0, 40.0);
        assert_eq!(
            liquid_blend_height(r0, r1, 0.0, 0.0).to_bits(),
            10.0f32.to_bits()
        );
        assert_eq!(
            liquid_blend_height(r0, r1, 1.0, 0.0).to_bits(),
            20.0f32.to_bits()
        );
        assert_eq!(
            liquid_blend_height(r0, r1, 0.0, 1.0).to_bits(),
            30.0f32.to_bits()
        );
        assert_eq!(
            liquid_blend_height(r0, r1, 1.0, 1.0).to_bits(),
            40.0f32.to_bits()
        );
        assert_eq!(
            liquid_blend_height(r0, r1, 0.5, 0.5).to_bits(),
            25.0f32.to_bits()
        );
    }

    #[test]
    fn kind0_uses_bias_for_compare_only() {
        assert_eq!(liquid_test_kind(0, 5.0, 10.0, 12.0), LiquidHit::Hit(5.0));
        assert_eq!(liquid_test_kind(0, 5.0, 10.0, 20.0), LiquidHit::Miss);
    }

    #[test]
    fn kind2_3_compare_raw_height() {
        assert_eq!(liquid_test_kind(2, 8.0, 100.0, 7.0), LiquidHit::Hit(8.0));
        assert_eq!(liquid_test_kind(3, 8.0, 100.0, 8.0), LiquidHit::Miss);
        assert_eq!(liquid_test_kind(1, 8.0, 0.0, 0.0), LiquidHit::Miss);
    }
}

/// One render-list element's sort-key fields, read from a 0x40-byte draw element.
#[derive(Clone, Copy)]
pub struct DrawElem {
    /// `elem[0]` (+0x00): geometry type, signed ascending; also the priority index.
    pub type_idx: i32,
    /// `elem[1]` (+0x04): unsigned ascending key.
    pub key_u: u32,
    /// `elem[2]` (+0x08): signed descending depth.
    pub depth_a: i32,
    /// `elem[5]` (+0x14): float descending.
    pub f14: f32,
    /// `elem[6]` (+0x18): float descending.
    pub f18: f32,
    /// `elem[10]` (+0x28): signed ascending.
    pub i28: i32,
    /// `elem[14]` (+0x38): unsigned ascending texture key (gated).
    pub tex_key: u32,
    /// `*(u16)(elem[11]+0xc)`: sub-key, only consulted when `type_idx < 3`.
    pub sub_u16: u16,
}

/// Three-valued opaque draw-list comparator.
///
/// Returns `Some(-1|1)` on a decided order or `None` when every keyed field ties
/// (the caller then falls through to the extended comparator). `prio_a`/`prio_b`
/// are the priority-table values for the two elements (only meaningful when
/// `prio_enabled`); `tex_enabled` gates the texture-key tier.
pub fn c_world_view__compare_draw_list_opaque__70ae10(
    a: &DrawElem,
    b: &DrawElem,
    prio_enabled: bool,
    prio_a: u32,
    prio_b: u32,
    tex_enabled: bool,
) -> Option<i32> {
    if prio_enabled {
        if prio_a < prio_b {
            return Some(-1);
        }
        if prio_b < prio_a {
            return Some(1);
        }
    }
    if b.f14 < a.f14 {
        return Some(-1);
    }
    if a.f14 < b.f14 {
        return Some(1);
    }
    if b.depth_a < a.depth_a {
        return Some(-1);
    }
    if a.depth_a < b.depth_a {
        return Some(1);
    }
    if a.i28 < b.i28 {
        return Some(-1);
    }
    if b.i28 < a.i28 {
        return Some(1);
    }
    if tex_enabled {
        if a.tex_key < b.tex_key {
            return Some(-1);
        }
        if b.tex_key < a.tex_key {
            return Some(1);
        }
    }
    if b.f18 < a.f18 {
        return Some(-1);
    }
    if a.f18 < b.f18 {
        return Some(1);
    }
    if a.key_u < b.key_u {
        return Some(-1);
    }
    if b.key_u < a.key_u {
        return Some(1);
    }
    if a.type_idx < b.type_idx {
        return Some(-1);
    }
    if b.type_idx < a.type_idx {
        return Some(1);
    }
    if a.type_idx < 3 {
        if a.sub_u16 < b.sub_u16 {
            return Some(-1);
        }
        if b.sub_u16 < a.sub_u16 {
            return Some(1);
        }
    }
    None
}

/// Packed total-order sort key for one element, equivalent to the opaque comparator chain.
///
/// The arrays order lexicographically, one lane per tier in chain order, so for
/// any two elements the array comparison is `Less`/`Greater` exactly where
/// [`c_world_view__compare_draw_list_opaque__70ae10`] returns `Some(-1)`/`Some(1)`
/// and `Equal` exactly where it returns `None`. A sort can therefore compute
/// every element's key once, compare keys, and consult the extended comparator
/// only on full key equality, where the chain itself falls through. Descending
/// tiers (`f14`, `depth_a`, `f18`) are stored inverted; disabled gated tiers
/// (`prio`, `tex_key`) are stored as zero on every element so they cannot
/// decide; the `sub_u16` tier is stored as zero for `type_idx >= 3`, which is
/// faithful because that tier is only reached when the `type_idx` lane already
/// ties. Returns `None` when `f14` or `f18` is NaN: the chain's ordered `<`
/// tests make a NaN tie with every value, which is not a total order and
/// cannot be expressed by any key, so the caller must fall back to sorting
/// with the comparator chain itself.
pub fn draw_elem_sort_key(
    e: &DrawElem,
    prio_enabled: bool,
    prio: u32,
    tex_enabled: bool,
) -> Option<[u32; 9]> {
    if e.f14.is_nan() || e.f18.is_nan() {
        return None;
    }
    Some([
        if prio_enabled { prio } else { 0 },
        !float_key_ascending(e.f14),
        !(e.depth_a.cast_unsigned() ^ 0x8000_0000),
        e.i28.cast_unsigned() ^ 0x8000_0000,
        if tex_enabled { e.tex_key } else { 0 },
        !float_key_ascending(e.f18),
        e.key_u,
        e.type_idx.cast_unsigned() ^ 0x8000_0000,
        if e.type_idx < 3 {
            u32::from(e.sub_u16)
        } else {
            0
        },
    ])
}

/// Order-preserving bit map from a non-NaN `f32` to a `u32`.
///
/// `a < b` on the floats iff `map(a) < map(b)` on the integers, and equal
/// floats map equal: `-0.0` is canonicalised to `+0.0` first (the comparator's
/// `<` tests treat the two zeros as equal, their bit patterns are not).
/// Non-negative floats set the sign bit, negative floats invert wholesale, so
/// the integer order runs -inf .. -0/+0 .. +inf.
const fn float_key_ascending(f: f32) -> u32 {
    let bits = f.to_bits();
    // Both zeros have all 31 value bits clear; everything else keeps its bits.
    let canonical = if bits.trailing_zeros() >= 31 { 0 } else { bits };
    if canonical & 0x8000_0000 == 0 {
        canonical | 0x8000_0000
    } else {
        !canonical
    }
}

#[cfg(test)]
mod tests_c_world_view__compare_draw_list_opaque__70ae10 {
    use super::{DrawElem, c_world_view__compare_draw_list_opaque__70ae10 as cmp};

    /// All-zero element.
    ///
    /// Every keyed field ties, so two of them compare as `None` regardless of
    /// which tier is enabled.
    fn zero_elem() -> DrawElem {
        DrawElem {
            type_idx: 0,
            key_u: 0,
            depth_a: 0,
            f14: 0.0,
            f18: 0.0,
            i28: 0,
            tex_key: 0,
            sub_u16: 0,
        }
    }

    #[test]
    fn equal_elements_are_undecided() {
        let a = zero_elem();
        let b = zero_elem();
        // No tier can decide between two identical elements.
        assert_eq!(cmp(&a, &b, false, 0, 0, false), None);
        assert_eq!(cmp(&a, &b, true, 7, 7, true), None);
    }

    #[test]
    fn antisymmetry_when_decided() {
        // Swapping the two operands must flip the sign of a decided result, for
        // every distinct field across the tier chain (priority off, texture off).
        let mut cases: Vec<(DrawElem, DrawElem)> = Vec::new();

        let mut f14 = zero_elem();
        f14.f14 = 1.0;
        cases.push((zero_elem(), f14));

        let mut depth = zero_elem();
        depth.depth_a = 5;
        cases.push((zero_elem(), depth));

        let mut i28 = zero_elem();
        i28.i28 = 9;
        cases.push((zero_elem(), i28));

        let mut f18 = zero_elem();
        f18.f18 = 2.0;
        cases.push((zero_elem(), f18));

        let mut key = zero_elem();
        key.key_u = 11;
        cases.push((zero_elem(), key));

        let mut ty = zero_elem();
        ty.type_idx = 4;
        cases.push((zero_elem(), ty));

        for (lo, hi) in &cases {
            let forward = cmp(lo, hi, false, 0, 0, false);
            let reverse = cmp(hi, lo, false, 0, 0, false);
            let f = forward.expect("case should be decided");
            let r = reverse.expect("reverse should be decided");
            assert_eq!(f, -r);
            assert!(f == -1 || f == 1);
        }
    }

    #[test]
    fn priority_tier_dominates() {
        // When priority is enabled and the two priority values differ, the
        // decision is taken on priority alone even though every other field
        // would have forced the opposite order.
        let mut a = zero_elem();
        let mut b = zero_elem();
        // Make b sort first on f14 (descending) so a later tier would pick b.
        a.f14 = 1.0;
        b.f14 = 9.0;
        // Lower priority value sorts first (ascending).
        assert_eq!(cmp(&a, &b, true, 2, 8, false), Some(-1));
        assert_eq!(cmp(&a, &b, true, 8, 2, false), Some(1));
        // Equal priority falls through to the f14 tier (b is larger -> b first).
        assert_eq!(cmp(&a, &b, true, 5, 5, false), Some(1));
    }

    #[test]
    fn f14_is_descending() {
        // Larger f14 sorts first (returns -1 for the larger operand on the left).
        let mut big = zero_elem();
        let mut small = zero_elem();
        big.f14 = 10.0;
        small.f14 = 3.0;
        assert_eq!(cmp(&big, &small, false, 0, 0, false), Some(-1));
        assert_eq!(cmp(&small, &big, false, 0, 0, false), Some(1));
    }

    #[test]
    fn i28_is_ascending() {
        // Smaller i28 sorts first; this tier sits below the depth tier.
        let mut lo = zero_elem();
        let mut hi = zero_elem();
        lo.i28 = -4;
        hi.i28 = 7;
        assert_eq!(cmp(&lo, &hi, false, 0, 0, false), Some(-1));
        assert_eq!(cmp(&hi, &lo, false, 0, 0, false), Some(1));
    }

    #[test]
    fn texture_tier_is_gated() {
        // The texture key only participates when `tex_enabled`; with it off the
        // pair is undecided, with it on the smaller key sorts first.
        let mut a = zero_elem();
        let mut b = zero_elem();
        a.tex_key = 1;
        b.tex_key = 4;
        assert_eq!(cmp(&a, &b, false, 0, 0, false), None);
        assert_eq!(cmp(&a, &b, false, 0, 0, true), Some(-1));
        assert_eq!(cmp(&b, &a, false, 0, 0, true), Some(1));
    }

    #[test]
    fn sub_u16_only_when_type_below_three() {
        // sub_u16 is the final tier and is consulted only when type_idx < 3.
        // type_idx must be equal for the chain to reach it (it is an earlier tier).
        let mut a = zero_elem();
        let mut b = zero_elem();
        a.type_idx = 2;
        b.type_idx = 2;
        a.sub_u16 = 1;
        b.sub_u16 = 9;
        assert_eq!(cmp(&a, &b, false, 0, 0, false), Some(-1));
        assert_eq!(cmp(&b, &a, false, 0, 0, false), Some(1));

        // type_idx == 3 is outside the gate -> sub_u16 is ignored -> undecided.
        let mut c = a;
        let mut d = b;
        c.type_idx = 3;
        d.type_idx = 3;
        assert_eq!(cmp(&c, &d, false, 0, 0, false), None);
    }

    #[test]
    fn tier_precedence_across_chain() {
        // For each adjacent pair of decided fields, the earlier tier must win
        // even when the later tier disagrees. We walk a list of (setter) pairs.
        // f14 (descending) outranks depth (descending): give a the larger f14
        // but the smaller depth -> f14 decides -> a sorts first.
        let mut a = zero_elem();
        let mut b = zero_elem();
        a.f14 = 5.0;
        b.f14 = 1.0;
        a.depth_a = 1;
        b.depth_a = 9;
        assert_eq!(cmp(&a, &b, false, 0, 0, false), Some(-1));

        // depth (descending) outranks i28 (ascending).
        let mut c = zero_elem();
        let mut d = zero_elem();
        c.depth_a = 9;
        d.depth_a = 1;
        c.i28 = 100;
        d.i28 = -100;
        assert_eq!(cmp(&c, &d, false, 0, 0, false), Some(-1));
    }
}

#[cfg(test)]
mod tests_draw_elem_sort_key {
    use super::{
        DrawElem, c_world_view__compare_draw_list_opaque__70ae10 as cmp, draw_elem_sort_key,
    };

    /// Xorshift step, the whole PRNG the generator needs.
    fn next(state: &mut u32) -> u32 {
        let mut x = *state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *state = x;
        x
    }

    /// Element with every field drawn from the PRNG over a small value pool.
    ///
    /// Fields collide often on purpose: the equivalence claim is about ties as
    /// much as about decided orders, and independent 32-bit draws would never
    /// tie. The float pool covers both zeros, both infinities and both signs.
    fn random_elem(state: &mut u32) -> DrawElem {
        const FLOATS: [f32; 9] = [
            0.0,
            -0.0,
            1.0,
            -1.0,
            0.5,
            -3.25,
            f32::INFINITY,
            f32::NEG_INFINITY,
            123.0,
        ];
        let mut pick = |n: u32| next(state) % n;
        DrawElem {
            type_idx: pick(6).cast_signed() - 1,
            key_u: pick(4),
            depth_a: pick(5).cast_signed() - 2,
            f14: FLOATS[pick(9) as usize],
            f18: FLOATS[pick(9) as usize],
            i28: pick(5).cast_signed() - 2,
            tex_key: pick(4),
            sub_u16: u16::try_from(pick(3)).expect("pool bound is 3"),
        }
    }

    #[test]
    fn key_order_matches_comparator_chain() {
        // For every pair and every gate combination: the key comparison is
        // Less/Greater exactly where the chain decides -1/1, Equal exactly
        // where the chain is undecided (None). Priority values are drawn from
        // a small pool per element, fixed across the pair's two orderings.
        let mut state = 0x9e37_79b9;
        for _ in 0..4000 {
            let a = random_elem(&mut state);
            let b = random_elem(&mut state);
            let prio_a = next(&mut state) % 3;
            let prio_b = next(&mut state) % 3;
            for (prio_enabled, tex_enabled) in
                [(false, false), (false, true), (true, false), (true, true)]
            {
                let ka = draw_elem_sort_key(&a, prio_enabled, prio_a, tex_enabled)
                    .expect("pool has no NaN");
                let kb = draw_elem_sort_key(&b, prio_enabled, prio_b, tex_enabled)
                    .expect("pool has no NaN");
                let chain = cmp(&a, &b, prio_enabled, prio_a, prio_b, tex_enabled);
                let expected = match chain {
                    Some(-1) => core::cmp::Ordering::Less,
                    Some(1) => core::cmp::Ordering::Greater,
                    None => core::cmp::Ordering::Equal,
                    Some(other) => panic!("comparator returned {other}"),
                };
                assert_eq!(
                    ka.cmp(&kb),
                    expected,
                    "gates prio={prio_enabled} tex={tex_enabled} prio_a={prio_a} prio_b={prio_b}"
                );
            }
        }
    }

    #[test]
    fn nan_yields_no_key() {
        // A NaN in either float tier makes the chain's `<` tests tie with
        // everything, which no total key order can express, so the builder
        // must refuse rather than emit a key.
        let mut e = DrawElem {
            type_idx: 0,
            key_u: 0,
            depth_a: 0,
            f14: f32::NAN,
            f18: 0.0,
            i28: 0,
            tex_key: 0,
            sub_u16: 0,
        };
        assert!(draw_elem_sort_key(&e, false, 0, false).is_none());
        e.f14 = 0.0;
        e.f18 = f32::NAN;
        assert!(draw_elem_sort_key(&e, false, 0, false).is_none());
        e.f18 = 0.0;
        assert!(draw_elem_sort_key(&e, false, 0, false).is_some());
    }

    #[test]
    fn zero_signs_tie_in_the_float_tiers() {
        // -0.0 and +0.0 differ in bits but tie under the chain's `<` tests;
        // the canonicalisation must make their keys equal too.
        let mut a = DrawElem {
            type_idx: 0,
            key_u: 0,
            depth_a: 0,
            f14: 0.0,
            f18: -0.0,
            i28: 0,
            tex_key: 0,
            sub_u16: 0,
        };
        let b = DrawElem {
            f14: -0.0,
            f18: 0.0,
            ..a
        };
        let ka = draw_elem_sort_key(&a, false, 0, false).expect("no NaN");
        let kb = draw_elem_sort_key(&b, false, 0, false).expect("no NaN");
        assert_eq!(ka, kb);
        a.f14 = f32::MIN_POSITIVE;
        let kc = draw_elem_sort_key(&a, false, 0, false).expect("no NaN");
        // A positive value must still sort after either zero in a descending tier.
        assert_eq!(kc.cmp(&kb), core::cmp::Ordering::Less);
    }
}

/// Slot index for `HeightBucket_InsertNodeAtPosSub` (`0x681970`).
///
/// The same 0..=31 Z-bucket as `height_bucket_insert_node_at_pos__6818b0`,
/// composed with the caller `sub_index` into the flat slot index
/// `(sub_index + bucket*9)*0xc` bytes of the 2-D bucket-pair array. Returns the
/// int-element offset (already divided by 4, i.e. `(sub_index + bucket*9)*3`),
/// or `None` when the bucket is rejected.
pub fn height_bucket_insert_node_at_pos_sub__681970(
    pos: &[f32; 3],
    coeff: &[f32; 4],
    scale: f32,
    bias: f32,
    sub_index: i32,
) -> Option<i32> {
    let bucket = i32::try_from(bucket_index(pos, coeff, scale, bias)?).unwrap_or(0);
    // (sub_index + bucket*9) * 0xc bytes -> *3 in int elements.
    Some((sub_index + bucket * 9) * 3)
}

#[cfg(test)]
mod tests_height_bucket_insert_node_at_pos_sub__681970 {
    use super::height_bucket_insert_node_at_pos_sub__681970 as slot_index;

    // Identity X-axis projection: `plane = pos.x`, so `bucket == round(pos.x)`
    // for `0 <= pos.x <= 31`. Lets each test pin an exact bucket via pos.x.
    const COEFF: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

    fn at(bucket_x: f32, sub_index: i32) -> Option<i32> {
        slot_index(&[bucket_x, 0.0, 0.0], &COEFF, 1.0, 0.0, sub_index)
    }

    #[test]
    fn known_values() {
        // bucket 3, sub_index 2 -> (2 + 3*9)*3 = 29*3 = 87.
        assert_eq!(at(3.0, 2), Some(87));
        // bucket 0, sub_index 0 -> 0.
        assert_eq!(at(0.0, 0), Some(0));
    }

    #[test]
    fn rejects_at_or_past_last_bucket() {
        // raw == 32 is out of the 0..=31 range -> None.
        assert_eq!(at(32.0, 0), None);
        assert_eq!(at(100.0, 5), None);
    }

    #[test]
    fn negative_projection_clamps_to_bucket_zero() {
        // A negative plane clamps the bucket to 0, so the result is sub_index*3.
        for sub_index in 0..9 {
            assert_eq!(at(-10.0, sub_index), Some(sub_index * 3));
        }
    }

    #[test]
    fn metamorphic_bucket_step_adds_27() {
        // Holding sub_index fixed, stepping the bucket up by one adds 9*3 == 27.
        let sub_index = 4;
        let buckets = [0.0_f32, 1.0, 2.0, 3.0, 10.0, 29.0, 30.0];
        for &bucket_x in &buckets {
            let here = at(bucket_x, sub_index).expect("in-range bucket");
            let next = at(bucket_x + 1.0, sub_index).expect("in-range bucket");
            assert_eq!(next - here, 27);
        }
    }

    #[test]
    fn metamorphic_sub_index_step_adds_3() {
        // Holding the bucket fixed, stepping sub_index up by one adds 3.
        let bucket_x = 7.0;
        for sub_index in 0..8 {
            let here = at(bucket_x, sub_index).expect("in-range bucket");
            let next = at(bucket_x, sub_index + 1).expect("in-range bucket");
            assert_eq!(next - here, 3);
        }
    }

    #[test]
    fn result_is_divisible_by_three() {
        // The returned offset is `(sub_index + bucket*9)*3`, always a multiple of 3.
        let cases = [(0.0, 0), (5.0, 2), (31.0, 8), (12.0, 6)];
        for (bucket_x, sub_index) in cases {
            let v = at(bucket_x, sub_index).expect("in-range bucket");
            assert_eq!(v % 3, 0);
        }
    }
}

/// Projects a world position onto the scene Z-bucket plane and maps it to a 0..=31 bucket index.
///
/// The plane is `scale * (c0*x + c1*y + c2*z + c3) - bias`, rounded to nearest
/// (x87 round-half-to-even). A negative result clamps to bucket 0; a result
/// `>= 32` is rejected (`None`). This is the only arithmetic shared by the
/// height-bucket insert routines; the intrusive linked-list surgery they perform
/// lives in the FFI adapter.
fn bucket_index(pos: &[f32; 3], coeff: &[f32; 4], scale: f32, bias: f32) -> Option<usize> {
    let plane = coeff[0] * pos[0] + coeff[1] * pos[1] + coeff[2] * pos[2] + coeff[3];
    // x87 `FISTP` rounds the scaled-then-biased f32 to int via round-half-to-even.
    let raw = (scale * plane - bias).round_ties_even() as i32;
    if raw >= 0x20 {
        None
    } else if raw < 0 {
        Some(0)
    } else {
        Some(raw as usize)
    }
}

/// Bucket index for `HeightBucket_InsertNodeAtPos` (`0x6818b0`).
///
/// The projected 0..=31 Z-bucket for `pos`, or `None` when the projection lands
/// at or past the last bucket. The adapter uses it to pick the global head slot
/// to relink at.
pub fn height_bucket_insert_node_at_pos__6818b0(
    pos: &[f32; 3],
    coeff: &[f32; 4],
    scale: f32,
    bias: f32,
) -> Option<usize> {
    bucket_index(pos, coeff, scale, bias)
}

#[cfg(test)]
mod tests_height_bucket_insert_node_at_pos__6818b0 {
    use super::height_bucket_insert_node_at_pos__6818b0 as bucket;

    /// Identity projection: `coeff = [1,0,0,0]`, unit scale, zero bias.
    ///
    /// The bucket is the rounded x coordinate.
    const ID: [f32; 4] = [1.0, 0.0, 0.0, 0.0];

    #[test]
    fn known_values() {
        // plane = 5.0, raw = round(5.0) = 5.
        assert_eq!(bucket(&[5.0, 0.0, 0.0], &ID, 1.0, 0.0), Some(5));
        // plane = 0.0, raw = 0.
        assert_eq!(bucket(&[0.0, 0.0, 0.0], &ID, 1.0, 0.0), Some(0));
        // plane = 31.0 lands on the last valid bucket.
        assert_eq!(bucket(&[31.0, 0.0, 0.0], &ID, 1.0, 0.0), Some(31));
    }

    #[test]
    fn negative_clamps_to_zero() {
        // A negative projection clamps to bucket 0, never wraps.
        for &x in &[-1.0_f32, -3.0, -100.0, -0.6] {
            assert_eq!(bucket(&[x, 0.0, 0.0], &ID, 1.0, 0.0), Some(0));
        }
    }

    #[test]
    fn at_or_past_last_bucket_is_none() {
        // raw == 32 and anything beyond is rejected.
        for &x in &[32.0_f32, 33.0, 64.0, 1000.0] {
            assert_eq!(bucket(&[x, 0.0, 0.0], &ID, 1.0, 0.0), None);
        }
    }

    #[test]
    fn round_half_to_even() {
        // 2.5 -> 2 (even), 3.5 -> 4 (even): banker's rounding.
        assert_eq!(bucket(&[2.5, 0.0, 0.0], &ID, 1.0, 0.0), Some(2));
        assert_eq!(bucket(&[3.5, 0.0, 0.0], &ID, 1.0, 0.0), Some(4));
        // 0.5 -> 0 (even), 1.5 -> 2 (even).
        assert_eq!(bucket(&[0.5, 0.0, 0.0], &ID, 1.0, 0.0), Some(0));
        assert_eq!(bucket(&[1.5, 0.0, 0.0], &ID, 1.0, 0.0), Some(2));
    }

    #[test]
    fn full_coefficient_dot_product() {
        // plane = 2*x + 3*y + 4*z + 1; with x=1,y=1,z=1 -> 10.
        let coeff = [2.0, 3.0, 4.0, 1.0];
        assert_eq!(bucket(&[1.0, 1.0, 1.0], &coeff, 1.0, 0.0), Some(10));
    }

    #[test]
    fn scale_and_bias_applied() {
        // plane = 10.0; scale 2.0 -> 20.0; bias 5.0 -> 15.0.
        assert_eq!(bucket(&[10.0, 0.0, 0.0], &ID, 2.0, 5.0), Some(15));
    }

    #[test]
    fn monotonic_within_valid_range() {
        // Increasing x never decreases the resulting bucket index until rejected.
        let mut prev = 0usize;
        let mut x = 0.0_f32;
        while x <= 31.0 {
            if let Some(b) = bucket(&[x, 0.0, 0.0], &ID, 1.0, 0.0) {
                assert!(b >= prev, "bucket should be non-decreasing in x");
                assert!((0..32).contains(&b), "bucket must stay in 0..32");
                prev = b;
            }
            x += 0.25;
        }
    }

    #[test]
    fn result_always_in_range_or_none() {
        // Any output, regardless of inputs, is either None or a valid 0..32 index.
        for &x in &[-50.0_f32, -1.0, 0.0, 7.3, 31.9, 32.0, 500.0] {
            if let Some(b) = bucket(&[x, 0.0, 0.0], &ID, 1.0, 0.0) {
                assert!((0..32).contains(&b));
            }
        }
    }
}

/// Projects an object world position onto the bucket axis and rounds+clamps to a bucket index.
///
/// `Some(0..=31)` is the destination bucket; `None` means the node projects past
/// the far bucket and must be dropped (no relink). The shipped body runs the
/// 4-lane form below; this scalar transcription is the oracle that form is
/// pinned against, and compiles only into test builds.
// The ten scalars are the object position, the plane row, the base, the scale
// and the bias the reference passes individually.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub fn height_bucket_insert_node_from_object__6816f0(
    px: f32,
    py: f32,
    pz: f32,
    base: f32,
    cx: f32,
    cy: f32,
    cz: f32,
    cw: f32,
    scale: f32,
    bias: f32,
) -> Option<usize> {
    let plane = cx * px + cy * py + cz * pz + cw;
    let projected = scale * (plane - base) - bias;
    // `FISTP` rounds to nearest, ties-to-even (the FPU default rounding mode).
    let idx = projected.round_ties_even() as i32;
    if idx < 0 {
        return Some(0);
    }
    if idx > 0x1f {
        return None;
    }
    Some(idx as usize)
}

/// 4-lane SSE form of `height_bucket_insert_node_from_object__6816f0`.
///
/// `pos4` is the node's `+0x5c` world triple with the `+0x68` base height in
/// lane 3, `coeff` the plane row at `0x87cfb8`. One `mulps` replaces the three
/// `mulss`, lane for lane and pair for pair, and the lanes are folded in the
/// reference's serial order, `(x*c0 + y*c1) + z*c2`, rather than as a tree, so
/// every rounding stays where the scalar kernel put it. Lane 3 of the product
/// (`base*c3`) is computed and dropped: the plane constant is added and the
/// base subtracted as scalars exactly as the reference does, and folding them
/// into the reduction would put `base*c3` in their place and move the bucket.
/// The property test pins the two kernels together.
pub fn height_bucket_insert_node_from_object4__6816f0(
    pos4: [f32; 4],
    coeff: [f32; 4],
    scale: f32,
    bias: f32,
) -> Option<usize> {
    #[cfg(target_arch = "x86")]
    use core::arch::x86::{
        _mm_add_ss, _mm_cvtss_f32, _mm_loadu_ps, _mm_movehl_ps, _mm_mul_ps, _mm_shuffle_ps,
    };
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::{
        _mm_add_ss, _mm_cvtss_f32, _mm_loadu_ps, _mm_movehl_ps, _mm_mul_ps, _mm_shuffle_ps,
    };
    // Every intrinsic below is SSE1, available on every ISA baseline this
    // crate builds for.
    // SAFETY: the load covers the 16 in-bounds bytes of its `[f32; 4]` argument.
    let pos = unsafe { _mm_loadu_ps(pos4.as_ptr()) };
    // SAFETY: as above.
    let row = unsafe { _mm_loadu_ps(coeff.as_ptr()) };
    // SAFETY: lane-wise multiply of two initialized vectors.
    let prod = unsafe { _mm_mul_ps(pos, row) };
    // SAFETY: lane 1 of an initialized vector selected into lane 0.
    let lane1 = unsafe { _mm_shuffle_ps::<0b01_01_01_01>(prod, prod) };
    // SAFETY: scalar add of lane 0 of two initialized vectors.
    let xy = unsafe { _mm_add_ss(lane1, prod) };
    // SAFETY: the upper half of an initialized vector moved into the lower.
    let lane2 = unsafe { _mm_movehl_ps(prod, prod) };
    // SAFETY: scalar add of lane 0 of two initialized vectors.
    let xyz = unsafe { _mm_add_ss(lane2, xy) };
    // SAFETY: lane 0 of an initialized vector.
    let plane = unsafe { _mm_cvtss_f32(xyz) } + coeff[3];
    let projected = scale * (plane - pos4[3]) - bias;
    // `FISTP` rounds to nearest, ties-to-even (the FPU default rounding mode).
    let idx = projected.round_ties_even() as i32;
    if idx < 0 {
        return Some(0);
    }
    if idx > 0x1f {
        return None;
    }
    Some(idx as usize)
}

#[cfg(test)]
mod tests_height_bucket_insert_node_from_object__6816f0 {
    use super::{
        height_bucket_insert_node_from_object__6816f0 as bucket,
        height_bucket_insert_node_from_object4__6816f0 as bucket4,
    };

    /// Drives the projection so the kernel sees exactly `projected` along `px`.
    ///
    /// With an identity plane/scale/bias (cx=1, all other plane terms 0).
    fn at(projected: f32) -> Option<usize> {
        bucket(projected, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0)
    }

    #[test]
    fn known_values() {
        // Identity projection: projected == px, rounds straight to the bucket.
        assert_eq!(at(0.0), Some(0));
        assert_eq!(at(5.0), Some(5));
        assert_eq!(at(31.0), Some(31));
        // Past the far bucket: dropped.
        assert_eq!(at(40.0), None);
        // Below zero: clamped to bucket 0 (still relinked there).
        assert_eq!(at(-3.0), Some(0));
    }

    #[test]
    fn ties_round_to_even() {
        // round_ties_even: 2.5 -> 2, 3.5 -> 4, 30.5 -> 30, 31.5 -> 32 (dropped).
        assert_eq!(at(2.5), Some(2));
        assert_eq!(at(3.5), Some(4));
        assert_eq!(at(30.5), Some(30));
        assert_eq!(at(31.5), None);
    }

    #[test]
    fn boundary_clamp_and_drop() {
        // The far edge: 31 is the last valid bucket, 32 is the first drop.
        assert_eq!(at(31.4), Some(31));
        assert_eq!(at(31.6), None);
        // The near edge: -0.4 rounds to 0; -0.6 rounds to -1 then clamps to 0.
        assert_eq!(at(-0.4), Some(0));
        assert_eq!(at(-0.6), Some(0));
    }

    #[test]
    fn result_is_always_in_range() {
        // Sweep a range of projected values; every Some must land in 0..=31.
        let probes = [
            -1000.0_f32,
            -31.0,
            -1.0,
            0.0,
            0.5,
            7.3,
            15.0,
            23.9,
            31.0,
            31.5,
            100.0,
            1.0e9,
        ];
        for p in &probes {
            if let Some(b) = at(*p) {
                assert!(
                    (0..32).contains(&b),
                    "bucket {b} out of range for projected {p}"
                );
            }
        }
    }

    #[test]
    fn clamp_low_drop_high() {
        // Everything well below zero clamps to bucket 0.
        for p in &[-1.0_f32, -10.0, -1.0e6] {
            assert_eq!(at(*p), Some(0));
        }
        // Everything well above 31 is dropped.
        for p in &[40.0_f32, 1000.0, 1.0e6] {
            assert_eq!(at(*p), None);
        }
    }

    #[test]
    fn monotonic_across_valid_range() {
        // Walking px upward through the valid range never decreases the bucket.
        let mut prev = 0usize;
        let mut steps = 0u32;
        let mut p = 0.0_f32;
        while p <= 31.0 {
            let b = at(p).expect("in-range projected must produce a bucket");
            assert!(b >= prev, "bucket {b} < previous {prev} at projected {p}");
            prev = b;
            steps += 1;
            p += 0.25;
        }
        assert!(steps > 0);
    }

    #[test]
    fn plane_equation_combines_all_terms() {
        // Full plane: cx*px + cy*py + cz*pz + cw, then scale*(plane-base)-bias.
        // plane = 2*3 + 4*1 + (-1)*2 + 1 = 6 + 4 - 2 + 1 = 9;
        // projected = 0.5*(9 - 2) - 0.5 = 3.5 - 0.5 = 3.0 -> bucket 3.
        let got = bucket(3.0, 1.0, 2.0, 2.0, 2.0, 4.0, -1.0, 1.0, 0.5, 0.5);
        assert_eq!(got, Some(3));
    }

    /// The 4-lane kernel must agree with the scalar one on every input class.
    ///
    /// The sweep drives the x coefficient, the x coordinate and the base
    /// height across ordinary values, both zeroes, both infinities and a NaN,
    /// holding the rest of the plane row at values that keep all four lanes
    /// live, the lane-3 product the packed form computes and drops included.
    #[test]
    fn packed_matches_scalar_kernel() {
        let vals = [
            -1.0f32,
            0.0,
            -0.0,
            0.5,
            2.5,
            3.5,
            31.5,
            40.0,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
            1.0e9,
        ];
        for &px in &vals {
            for &cx in &vals {
                for &base in &vals {
                    let (py, pz) = (1.0f32, 2.0f32);
                    let (cy, cz, cw) = (0.5f32, -0.25f32, 1.5f32);
                    let (scale, bias) = (0.75f32, 0.25f32);
                    assert_eq!(
                        bucket(px, py, pz, base, cx, cy, cz, cw, scale, bias),
                        bucket4([px, py, pz, base], [cx, cy, cz, cw], scale, bias),
                        "px={px} cx={cx} base={base}"
                    );
                }
            }
        }
    }
}

/// Builds the two 9-entry edge-index lists for a cell occluder pass.
///
/// The first list (column edges) seeds at `0x88` when `col_far` (the frustum
/// bound exceeds its threshold) else `0`, and steps by `stride`. The second
/// (row edges) seeds at `8` when `row_far` else `0`, and steps by `stride*0x11`
/// while a parallel cursor advances by `stride` to bound the 9-entry loop.
pub fn height_bucket__rasterize_cell_occluder__6c15d0(
    stride: i32,
    col_far: bool,
    row_far: bool,
) -> ([i32; 9], [i32; 9]) {
    let col_offset = if col_far { 0x88 } else { 0 };
    let mut col = [0i32; 9];
    let mut acc = 0i32;
    for slot in &mut col {
        *slot = acc + col_offset;
        acc += stride;
    }
    let row_offset = if row_far { 8 } else { 0 };
    let row_step = stride * 0x11;
    let mut row = [0i32; 9];
    let mut val = row_offset;
    for slot in &mut row {
        *slot = val;
        val += row_step;
    }
    (col, row)
}

#[cfg(test)]
mod tests_height_bucket__rasterize_cell_occluder__6c15d0 {
    use super::height_bucket__rasterize_cell_occluder__6c15d0 as kernel;

    /// Column list: `col[i] = i*stride + (0x88 if col_far else 0)`.
    ///
    /// Row list: `row[i] = (8 if row_far else 0) + i*(stride*0x11)`.
    /// Known value for `stride=1`, both bounds far.
    #[test]
    fn known_value_stride_one_far() {
        let (col, row) = kernel(1, true, true);
        assert_eq!(col, [136, 137, 138, 139, 140, 141, 142, 143, 144]);
        assert_eq!(row, [8, 25, 42, 59, 76, 93, 110, 127, 144]);
    }

    /// Known value for `stride=2`, both bounds near (no seed offset).
    #[test]
    fn known_value_stride_two_near() {
        let (col, row) = kernel(2, false, false);
        assert_eq!(col, [0, 2, 4, 6, 8, 10, 12, 14, 16]);
        assert_eq!(row, [0, 34, 68, 102, 136, 170, 204, 238, 272]);
    }

    /// Both lists are arithmetic progressions.
    ///
    /// The difference between adjacent entries is constant (`stride` for
    /// columns, `stride*0x11` for rows).
    #[test]
    fn law_constant_first_difference() {
        for &stride in &[1i32, 2, 3, 7] {
            for &col_far in &[false, true] {
                for &row_far in &[false, true] {
                    let (col, row) = kernel(stride, col_far, row_far);
                    for pair in col.windows(2) {
                        assert_eq!(pair[1] - pair[0], stride);
                    }
                    for pair in row.windows(2) {
                        assert_eq!(pair[1] - pair[0], stride * 0x11);
                    }
                }
            }
        }
    }

    /// The far flags only shift the first entry by a fixed seed (`0x88` columns, `8` rows).
    ///
    /// Every entry shifts by exactly that seed relative to the near case.
    #[test]
    fn law_far_flag_is_pure_offset() {
        for &stride in &[1i32, 4, 9] {
            let (col_near, row_near) = kernel(stride, false, false);
            let (col_far, row_far) = kernel(stride, true, true);
            for (near, far) in col_near.iter().zip(col_far.iter()) {
                assert_eq!(far - near, 0x88);
            }
            for (near, far) in row_near.iter().zip(row_far.iter()) {
                assert_eq!(far - near, 8);
            }
        }
    }

    /// The seed determines the first entry exactly.
    ///
    /// The seed range is one of the two documented values per axis.
    #[test]
    fn law_seed_drives_first_entry() {
        for &col_far in &[false, true] {
            for &row_far in &[false, true] {
                let (col, row) = kernel(5, col_far, row_far);
                let col_seed = if col_far { 0x88 } else { 0 };
                let row_seed = if row_far { 8 } else { 0 };
                assert_eq!(col[0], col_seed);
                assert_eq!(row[0], row_seed);
                assert!((0..0x89).contains(&col[0]));
                assert!((0..9).contains(&row[0]));
            }
        }
    }
}

/// Number of horizon columns in the screen-space occlusion buffer.
pub const HB_COLUMN_COUNT: i32 = 0x140;
/// Half-screen column origin added to every mapped column index.
const HB_COLUMN_ORIGIN: i32 = 0xa0;

/// Transforms a point by a column-major 4x4 with implied `w = 1`.
///
/// The translation row is at indices 12/13/14, matching
/// `C44Matrix::TransformPoint` at `0x7bca80`.
fn hb_transform_point(p: [f32; 3], mat: &[f32; 16]) -> [f32; 3] {
    let x = p[0] * mat[0] + p[1] * mat[4] + p[2] * mat[8] + mat[12];
    let y = p[0] * mat[1] + p[1] * mat[5] + p[2] * mat[9] + mat[13];
    let z = p[0] * mat[2] + p[1] * mat[6] + p[2] * mat[10] + mat[14];
    [x, y, z]
}

/// Round-half-to-even to i32 matching x87 `FISTP` under the default rounding mode.
///
/// Including the integer-indefinite value (`0x80000000`) the FPU stores for NaN
/// and out-of-range magnitudes — unlike Rust's saturating float-to-int cast,
/// which would clamp to `i32::MAX`/`i32::MIN`. Mirrors the indefinite convention
/// in `ftol__40a2b0` (which truncates toward zero rather than rounding to
/// nearest).
fn hb_round_to_i32(v: f32) -> i32 {
    let r = f64::from(v).round_ties_even();
    if (-2_147_483_648.0..2_147_483_648.0).contains(&r) {
        r as i32
    } else {
        // NaN/Inf and every |r| >= 2^31: the x87 integer indefinite value.
        i32::MIN
    }
}

/// Maps a projected screen-X to a horizon column via `round(x*scale - bias)`.
///
/// Plus the half-screen origin and a caller-supplied per-end adjustment. x87
/// `ROUND` is round-half-to-even, which `f32::round_ties_even` matches; the
/// integer add wraps, matching x86 `add` (the downstream range test and the
/// adapter's `[0, HB_COLUMN_COUNT-1]` clamp absorb the wrapped column).
fn hb_map_column(screen_x: f32, scale: f32, bias: f32, extra: i32) -> i32 {
    hb_round_to_i32(screen_x * scale - bias)
        .wrapping_add(HB_COLUMN_ORIGIN)
        .wrapping_add(extra)
}

/// Result of an occluder projection.
///
/// The inclusive horizon-column span the geometry covers and the projected
/// screen height to compare against the horizon. The adapter clamps the span to
/// `[0, HB_COLUMN_COUNT-1]` and reads the buffer; `None` means the geometry was
/// rejected before reaching the buffer.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct OccluderSpan {
    pub col_min: i32,
    pub col_max: i32,
    pub test_height: f32,
}

/// One projected occluder vertex.
///
/// Its perspective-divided screen X, the perspective-divided screen height
/// stored as the horizon depth, and the raw transformed depth (`eye_z`) used for
/// the near-plane test.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ProjectedVert {
    pub screen_x: f32,
    pub depth: f32,
    pub eye_z: f32,
}

/// Projects one occluder vertex through the view matrix and divides by depth.
///
/// The vertex is an indexed source position plus an offset. Yields its screen X,
/// the perspective-divided height stored for horizon rasterization, and the raw
/// transformed `eye_z` kept for the near-plane test.
pub fn height_bucket__project_vert__681b50(
    src: [f32; 3],
    offset: &[f32; 3],
    view: &[f32; 16],
    focal: f32,
) -> ProjectedVert {
    let world = [src[0] + offset[0], src[1] + offset[1], src[2] + offset[2]];
    let v = hb_transform_point(world, view);
    let inv_z = focal / v[2];
    ProjectedVert {
        screen_x: v[0] * inv_z,
        depth: v[1] * inv_z,
        eye_z: v[2],
    }
}

/// Rasterizes one occluder edge into the horizon column span.
///
/// The inclusive, pre-clamp `[col_lo, col_hi]` buffer-column range (already
/// including the half-screen origin) and the depth to raise each covered column
/// to (the smaller of the two endpoint depths). Returns `None` when either
/// endpoint's raw `eye_z` fails the near-plane test (`near_z`), so the edge is
/// skipped.
// `!(near_z <= eye_z)` keeps an unordered endpoint on the reject side, where
// `eye_z < near_z` would let a NaN depth through.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn height_bucket__edge_span__681b50(
    a: ProjectedVert,
    b: ProjectedVert,
    near_z: f32,
    col_scale: f32,
    col_bias: f32,
) -> Option<(i32, i32, f32)> {
    if !(near_z <= a.eye_z) || !(near_z <= b.eye_z) {
        return None;
    }
    let raise = if b.depth <= a.depth { b.depth } else { a.depth };
    let c0 = hb_map_column(a.screen_x, col_scale, col_bias, 0);
    let c1 = hb_map_column(b.screen_x, col_scale, col_bias, 0);
    let (lo, hi) = if c1 < c0 { (c1, c0) } else { (c0, c1) };
    Some((lo, hi, raise))
}

#[cfg(test)]
mod tests_height_bucket_rasterize {
    use super::{
        ProjectedVert, hb_map_column, height_bucket__edge_span__681b50 as edge_span,
        height_bucket__project_vert__681b50 as project_vert,
    };

    const ID: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ];

    #[test]
    fn project_vert_perspective_divide() {
        // Identity view, focal 2.0: (3, 4, 10) -> screen_x = 0.6, depth = 0.8, eye_z = 10.
        let p = project_vert([3.0, 4.0, 10.0], &[0.0, 0.0, 0.0], &ID, 2.0);
        assert!((p.screen_x - 0.6).abs() < 1e-6);
        assert!((p.depth - 0.8).abs() < 1e-6);
        assert!((p.eye_z - 10.0).abs() < 1e-6);
    }

    #[test]
    fn project_vert_applies_offset() {
        let p = project_vert([1.0, 1.0, 1.0], &[2.0, 3.0, 9.0], &ID, 1.0);
        // world = (3, 4, 10); eye_z = 10.
        assert!((p.eye_z - 10.0).abs() < 1e-6);
        assert!((p.screen_x - 0.3).abs() < 1e-6);
    }

    #[test]
    fn edge_span_uses_min_depth_and_orders_columns() {
        let a = ProjectedVert {
            screen_x: 0.5,
            depth: 5.0,
            eye_z: 20.0,
        };
        let b = ProjectedVert {
            screen_x: -0.5,
            depth: 3.0,
            eye_z: 15.0,
        };
        let (lo, hi, raise) = edge_span(a, b, 1.0, 10.0, 0.0).expect("both pass near plane");
        assert!(lo <= hi);
        assert_eq!(raise, 3.0); // min of the two endpoint depths
    }

    #[test]
    fn edge_span_columns_include_origin() {
        // screen_x 0, scale 10, bias 0 -> column = 0xa0 (origin) for both ends.
        let a = ProjectedVert {
            screen_x: 0.0,
            depth: 5.0,
            eye_z: 20.0,
        };
        let b = ProjectedVert {
            screen_x: 0.0,
            depth: 3.0,
            eye_z: 15.0,
        };
        let (lo, hi, _) = edge_span(a, b, 1.0, 10.0, 0.0).unwrap();
        assert_eq!(lo, 0xa0);
        assert_eq!(hi, 0xa0);
    }

    #[test]
    fn edge_span_near_plane_rejects() {
        let a = ProjectedVert {
            screen_x: 0.5,
            depth: 5.0,
            eye_z: 0.5,
        };
        let b = ProjectedVert {
            screen_x: -0.5,
            depth: 3.0,
            eye_z: 15.0,
        };
        assert_eq!(edge_span(a, b, 1.0, 10.0, 0.0), None);
    }

    /// A vertex projecting toward the near plane drives `screen_x` past the i32 range.
    ///
    /// The mapping must not trap (stock x86 `add` wraps): it yields the x87
    /// integer-indefinite column (`i32::MIN`) plus the origin, wrapping — not a
    /// saturated positive value. The old `as i32` saturate + checked `+` panicked.
    #[test]
    fn map_column_extreme_x_wraps_instead_of_trapping() {
        let huge = hb_map_column(1.0e30, 1.0, 0.0, 0);
        assert_eq!(huge, i32::MIN.wrapping_add(super::HB_COLUMN_ORIGIN));
        let huge_neg = hb_map_column(-1.0e30, 1.0, 0.0, 1);
        assert_eq!(
            huge_neg,
            i32::MIN
                .wrapping_add(super::HB_COLUMN_ORIGIN)
                .wrapping_add(1)
        );
        // NaN folds to the indefinite value too (matches `fistp`), no panic.
        assert_eq!(
            hb_map_column(f32::NAN, 1.0, 0.0, 0),
            i32::MIN.wrapping_add(super::HB_COLUMN_ORIGIN)
        );
    }

    /// The full edge-projection path no longer panics.
    ///
    /// Both endpoints project to an extreme screen X (the near-plane perspective
    /// blow-up the crash log hit).
    #[test]
    fn edge_span_extreme_projection_does_not_panic() {
        let a = ProjectedVert {
            screen_x: 1.0e30,
            depth: 1.0,
            eye_z: 20.0,
        };
        let b = ProjectedVert {
            screen_x: -1.0e30,
            depth: 1.0,
            eye_z: 20.0,
        };
        let (lo, hi, _) = edge_span(a, b, 1.0, 1.0, 0.0).expect("both pass near plane");
        assert!(lo <= hi);
    }
}

/// Projects the 8 corners of an AABB through the view matrix.
///
/// `min` = `aabb[0..3]`, `max` = `aabb[3..6]`. Reduces to a horizon-column span
/// plus the topmost projected height. Returns `None` when near-clip rejects a
/// corner (unless `skip_near` is set) or the span never reaches the buffer.
///
/// The reference enumerates the 8 corners through fixed index tables; because the
/// result aggregates `min`/`max` of the projected X and `max` of the projected
/// height over all 8, the enumeration order is irrelevant and the Cartesian
/// product of `{min, max}` per axis is used directly.
pub fn height_bucket__test_box__686180(
    aabb: &[f32; 6],
    view: &[f32; 16],
    focal: f32,
    near_z: f32,
    skip_near: bool,
    col_scale: f32,
    col_bias: f32,
) -> Option<OccluderSpan> {
    let mut x_min = f32::MAX;
    let mut x_max = -f32::MAX;
    let mut y_max = -f32::MAX;

    for ci in 0..8usize {
        let corner = [
            aabb[(ci & 1) * 3],
            aabb[((ci >> 1) & 1) * 3 + 1],
            aabb[((ci >> 2) & 1) * 3 + 2],
        ];
        let v = hb_transform_point(corner, view);
        if !skip_near && v[2] < near_z {
            return None;
        }
        let inv_z = focal / v[2];
        let sx = v[0] * inv_z;
        let sy = v[1] * inv_z;
        if sx < x_min {
            x_min = sx;
        }
        if x_max < sx {
            x_max = sx;
        }
        if y_max < sy {
            y_max = sy;
        }
    }

    let col_min = hb_map_column(x_min, col_scale, col_bias, 0);
    let col_max = hb_map_column(x_max, col_scale, col_bias, 1);
    if col_min < HB_COLUMN_COUNT && col_max >= 0 {
        Some(OccluderSpan {
            col_min,
            col_max,
            test_height: y_max,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests_height_bucket_test_box {
    use super::height_bucket__test_box__686180 as test_box;

    const ID: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ];

    #[test]
    fn box_centered_axis_aligned_span() {
        let aabb = [-1.0f32, -1.0, 9.0, 1.0, 1.0, 11.0];
        let s = test_box(&aabb, &ID, 1.0, 0.5, false, 100.0, 0.0).expect("visible");
        assert!(s.col_min <= s.col_max);
        assert!(s.col_min < 0xa0 && s.col_max > 0xa0);
    }

    #[test]
    fn box_near_clip_rejects() {
        let aabb = [-1.0f32, -1.0, 0.1, 1.0, 1.0, 0.2];
        assert_eq!(test_box(&aabb, &ID, 1.0, 1.0, false, 100.0, 0.0), None);
        assert!(test_box(&aabb, &ID, 1.0, 1.0, true, 100.0, 0.0).is_some());
    }

    #[test]
    fn box_corner_order_independent() {
        let a = [-2.0f32, -1.0, 9.0, 3.0, 4.0, 11.0];
        let b = [3.0f32, 4.0, 11.0, -2.0, -1.0, 9.0];
        let sa = test_box(&a, &ID, 1.0, 0.5, true, 100.0, 0.0).unwrap();
        let sb = test_box(&b, &ID, 1.0, 0.5, true, 100.0, 0.0).unwrap();
        assert_eq!(sa, sb);
    }
}

/// Projects a bounding sphere to a horizon-column span and a test height.
///
/// The sphere is a center + a screen-space radius derived from a separate radius
/// matrix. `center` is projected by `view`; the screen-space radius comes from
/// transforming `(radius, radius, 0)` by `radius_mat`. Returns `None` on a
/// too-small radius, near-clip rejection, or an out-of-buffer span.
// The ten parameters are the reference's argument list. The radius and
// near-plane gates are negated ordered compares, so a NaN radius or depth
// rejects as the original's compares did.
#[allow(clippy::too_many_arguments, clippy::neg_cmp_op_on_partial_ord)]
pub fn height_bucket__test_sphere__686000(
    center: &[f32; 3],
    radius: f32,
    view: &[f32; 16],
    radius_mat: &[f32; 16],
    focal: f32,
    near_z: f32,
    min_radius: f32,
    skip_near: bool,
    col_scale: f32,
    col_bias: f32,
) -> Option<OccluderSpan> {
    if !(min_radius <= radius.abs()) {
        return None;
    }

    let c = hb_transform_point(*center, view);
    let r = hb_transform_point([radius, radius, 0.0], radius_mat);

    let cz = c[2];
    if !skip_near && !(near_z <= cz) {
        return None;
    }

    let inv_z = focal / cz;
    let screen_cx = c[0] * inv_z;
    let screen_cy = c[1] * inv_z;
    let screen_rx = r[0] * inv_z;
    let screen_ry = r[1] * inv_z;

    let col_min = hb_map_column(screen_cx - screen_ry, col_scale, col_bias, 0);
    let col_max = hb_map_column(screen_cx + screen_ry, col_scale, col_bias, 1);
    if col_min < HB_COLUMN_COUNT && col_max >= 0 {
        Some(OccluderSpan {
            col_min,
            col_max,
            // Topmost screen height of the sphere: center-Y plus the screen radius
            // taken from the radius vector's projected X component.
            test_height: screen_cy + screen_rx,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests_height_bucket_test_sphere {
    use super::height_bucket__test_sphere__686000 as test_sphere;

    const ID: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ];

    #[test]
    fn sphere_small_radius_rejected() {
        let c = [0.0f32, 0.0, 10.0];
        assert_eq!(
            test_sphere(&c, 0.001, &ID, &ID, 1.0, 0.5, 0.1, false, 100.0, 0.0),
            None
        );
    }

    #[test]
    fn sphere_visible_span_brackets_center() {
        let c = [0.0f32, 0.0, 10.0];
        let s = test_sphere(&c, 2.0, &ID, &ID, 1.0, 0.5, 0.1, false, 100.0, 0.0)
            .expect("visible sphere");
        assert!(s.col_min < 0xa0 && s.col_max > 0xa0);
    }

    #[test]
    fn sphere_near_clip_rejects_unless_skipped() {
        let c = [0.0f32, 0.0, 0.2];
        assert_eq!(
            test_sphere(&c, 2.0, &ID, &ID, 1.0, 1.0, 0.1, false, 100.0, 0.0),
            None
        );
        assert!(test_sphere(&c, 2.0, &ID, &ID, 1.0, 1.0, 0.1, true, 100.0, 0.0).is_some());
    }
}

/// Page/cell decomposition and reconstructed world XY for a terrain-height cache.
///
/// Keyed by an integer grid cell (column, row). The page grid is 5 columns wide;
/// each page holds a 32x32 block of height values.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TerrainCellLookup {
    /// Index into the page-pointer grid: `(col>>5) + ((row>>5)+2)*5`.
    pub page_index: usize,
    /// Value index within the page: `(row&0x1f)*0x20 + (col&0x1f)`.
    pub cell_index: usize,
    /// Reconstructed world XY of the cell centre, fed to a downward raycast on a cache miss.
    pub world_x: f32,
    pub world_y: f32,
}

/// Decomposes an integer terrain cell `(col, row)` into the cache page/cell indices.
///
/// Also returns the reconstructed world XY. The world position anchors at the
/// integer grid origin scaled by `origin_scale`, steps by `cell_size` per cell,
/// and offsets to the cell centre by `cell_size * sub_offset`.
pub fn terrain_height_cache_sample_at_cell__67ca90(
    col: i32,
    row: i32,
    origin_x: i32,
    origin_y: i32,
    origin_scale: f32,
    cell_size: f32,
    sub_offset: f32,
) -> TerrainCellLookup {
    let page_index = ((col >> 5) + ((row >> 5) + 2) * 5) as usize;
    let cell_index = (((row & 0x1f) * 0x20) + (col & 0x1f)) as usize;

    let base_x = (origin_x as f32) * origin_scale + (col as f32) * cell_size;
    let base_y = (origin_y as f32) * origin_scale + (row as f32) * cell_size;
    let world_x = base_x + cell_size * sub_offset;
    let world_y = base_y + cell_size * sub_offset;

    TerrainCellLookup {
        page_index,
        cell_index,
        world_x,
        world_y,
    }
}

#[cfg(test)]
mod tests_terrain_height_cache_cell {
    use super::{TerrainCellLookup, terrain_height_cache_sample_at_cell__67ca90 as at_cell};

    #[test]
    fn cell_page_and_index_decomposition() {
        let r = at_cell(0x25, 0x43, 0, 0, 1.0, 1.0, 0.0);
        assert_eq!(r.page_index, (1 + (2 + 2) * 5) as usize);
        assert_eq!(r.cell_index, ((3 * 0x20) + 5) as usize);
    }

    #[test]
    fn cell_world_reconstruction() {
        let r = at_cell(3, 2, 10, 5, 2.0, 4.0, 0.5);
        assert!((r.world_x - 34.0).abs() < 1e-4, "{}", r.world_x);
        assert!((r.world_y - 20.0).abs() < 1e-4, "{}", r.world_y);
    }

    #[test]
    fn cell_negative_col_uses_arithmetic_shift() {
        let r = at_cell(-1, 0, 0, 0, 1.0, 1.0, 0.0);
        assert_eq!(r.page_index, ((-1) + 2 * 5) as usize);
        assert_eq!(r.cell_index, 31usize);
    }

    #[test]
    fn cell_lookup_struct_eq() {
        let a = TerrainCellLookup {
            page_index: 1,
            cell_index: 2,
            world_x: 3.0,
            world_y: 4.0,
        };
        let b = a;
        assert_eq!(a, b);
    }
}

/// Quantizes a world XY into a 32x32 single-page terrain-height cache cell.
///
/// The world position is shifted to grid-local space (`origin` scaled by
/// `origin_scale`), rejected if below `lo_bound`, then mapped to a cell via the
/// reference's bit-reinterpret trick: a magic bias is added so the integer part
/// lands in the float mantissa, and the raw IEEE-754 bits are shifted. Returns
/// `None` when the position is below the grid origin or maps outside the 32x32
/// page; otherwise the flat `row*0x20 + col` value index.
// The eight parameters are the client's argument list. `!(lo_bound <= f)`
// rejects a NaN coordinate, where `f < lo_bound` would accept it.
#[allow(clippy::too_many_arguments, clippy::neg_cmp_op_on_partial_ord)]
pub fn terrain_height_cache_sample_at_pos__67c820(
    pos_x: f32,
    pos_y: f32,
    origin_x: i32,
    origin_y: i32,
    origin_scale: f32,
    lo_bound: f32,
    quant_scale: f32,
    quant_bias: f32,
) -> Option<usize> {
    let fx = pos_x - (origin_x as f32) * origin_scale;
    let fy = pos_y - (origin_y as f32) * origin_scale;
    if !(lo_bound <= fx) || !(lo_bound <= fy) {
        return None;
    }

    // Bit-reinterpret quantization: `(quant_scale*f + quant_bias)` is read back as
    // an integer; the cell index sits in bits 14..22.
    let col = ((quant_scale * fx + quant_bias).to_bits() >> 0xe) & 0xff;
    let row = ((quant_scale * fy + quant_bias).to_bits() >> 0xe) & 0xff;
    if col >= 0x20 || row >= 0x20 {
        return None;
    }
    Some((row * 0x20 + col) as usize)
}

#[cfg(test)]
mod tests_terrain_height_cache_pos {
    use super::terrain_height_cache_sample_at_pos__67c820 as at_pos;

    #[test]
    fn pos_below_origin_returns_none() {
        assert_eq!(at_pos(-100.0, 50.0, 0, 0, 1.0, 0.0, 1.0, 0.0), None);
        assert_eq!(at_pos(50.0, -100.0, 0, 0, 1.0, 0.0, 1.0, 0.0), None);
    }

    #[test]
    fn pos_quantization_bit_trick_known() {
        let bias = 8_388_608.0f32; // 2^23
        let fx = (5i32 << 14) as f32; // -> col 5
        let fy = (7i32 << 14) as f32; // -> row 7
        let r = at_pos(fx, fy, 0, 0, 1.0, -1e9, 1.0, bias).unwrap();
        assert_eq!(r, 7 * 0x20 + 5);
    }

    #[test]
    fn pos_out_of_page_returns_none() {
        let bias = 8_388_608.0f32;
        let fx = (0x20i32 << 14) as f32; // col = 0x20 -> out of page
        assert_eq!(at_pos(fx, 0.0, 0, 0, 1.0, -1e9, 1.0, bias), None);
    }
}

/// Projection of a world-space AABB into the coarse tile-block grid.
///
/// Used by the terrain spatial query.
///
/// Each of the four extent components is mapped through the world map's affine
/// tile transform `round(scale * -(c - bias_a) - bias_b)` (round to nearest,
/// matching the reference's `FISTP`), yielding four integer tile coordinates.
/// The arithmetic shift `>> 3` collapses tiles into 8x8 blocks, giving the
/// inclusive block-index ranges the caller iterates. The returned `coords` are
/// the four tile coordinates in the order the per-block query consumes them:
/// `[from_max_x, from_max_y, from_min_x, from_min_y]`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TileGridBox {
    pub coords: [i32; 4],
    pub block_x_lo: i32,
    pub block_x_hi: i32,
    pub block_y_lo: i32,
    pub block_y_hi: i32,
}

#[inline]
fn project_tile(c: f32, scale: f32, bias_a: f32, bias_b: f32) -> i32 {
    // `round` matches the x87 `FISTP` round-to-nearest used by the reference.
    (scale * -(c - bias_a) - bias_b).round() as i32
}

pub fn world_query_tile_grid_box_terrain_pass__6ad4e0(
    aabb_min_x: f32,
    aabb_min_y: f32,
    aabb_max_x: f32,
    aabb_max_y: f32,
    scale: f32,
    bias_a: f32,
    bias_b: f32,
) -> TileGridBox {
    let tile_max_x = project_tile(aabb_max_x, scale, bias_a, bias_b);
    let tile_max_y = project_tile(aabb_max_y, scale, bias_a, bias_b);
    let tile_min_x = project_tile(aabb_min_x, scale, bias_a, bias_b);
    let tile_min_y = project_tile(aabb_min_y, scale, bias_a, bias_b);

    TileGridBox {
        coords: [tile_max_x, tile_max_y, tile_min_x, tile_min_y],
        // Outer (block-X) range spans the X extents; inner (block-Y) the Y.
        block_x_lo: tile_max_x >> 3,
        block_x_hi: tile_min_x >> 3,
        block_y_lo: tile_max_y >> 3,
        block_y_hi: tile_min_y >> 3,
    }
}

#[cfg(test)]
mod world_query_tile_grid_box_terrain_pass_tests {
    use super::world_query_tile_grid_box_terrain_pass__6ad4e0 as project;

    #[test]
    fn round_to_nearest_matches_fistp() {
        let r = project(2.4, -2.4, 2.6, -2.6, 1.0, 0.0, 0.0);
        assert_eq!(r.coords, [-3, 3, -2, 2]);
    }

    #[test]
    fn known_affine_and_blocks() {
        let r = project(10.0, 26.0, 90.0, 42.0, 0.5, 10.0, 1.0);
        assert_eq!(r.coords, [-41, -17, -1, -9]);
        assert_eq!(r.block_x_lo, -41 >> 3);
        assert_eq!(r.block_x_hi, -1 >> 3);
        assert_eq!(r.block_y_lo, -17 >> 3);
        assert_eq!(r.block_y_hi, -9 >> 3);
    }

    #[test]
    fn arithmetic_shift_is_floor_div() {
        let r = project(0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0);
        assert_eq!(r.coords, [0, 0, 0, 0]);
        assert_eq!(r.block_x_lo, 0);
    }
}

/// 6-bit conservative AABB clip outcode of a point, epsilon-slackened.
///
/// Bit `n` is the IEEE sign bit of the slackened f32 face difference: bits
/// 0/1/2 are set when the point lies more than `eps` below the box minimum on
/// x/y/z, bits 3/4/5 when more than `eps` above the maximum. `aabb` is
/// `[min_x, min_y, min_z, max_x, max_y, max_z]`. Three vertices whose outcodes
/// share a set bit span a triangle wholly outside that face (trivially
/// rejectable); the sign-bit encoding makes each face test branch-free.
pub fn c_map_chunk__rasterize_chunk_triangles__6ad7e0(
    point: &[f32; 3],
    aabb: &[f32; 6],
    eps: f32,
) -> u32 {
    let mut code = (point[0] - aabb[0] + eps).to_bits() >> 31;
    code |= ((point[1] - aabb[1] + eps).to_bits() >> 30) & 0x2;
    code |= ((point[2] - aabb[2] + eps).to_bits() >> 29) & 0x4;
    code |= ((aabb[3] - point[0] + eps).to_bits() >> 28) & 0x8;
    code |= ((aabb[4] - point[1] + eps).to_bits() >> 27) & 0x10;
    code |= ((aabb[5] - point[2] + eps).to_bits() >> 26) & 0x20;
    code
}

#[cfg(test)]
mod tests_c_map_chunk__rasterize_chunk_triangles__6ad7e0 {
    use super::c_map_chunk__rasterize_chunk_triangles__6ad7e0 as outcode;

    /// The host image's clip epsilon (`0x3C9F49F4`).
    const EPS: f32 = f32::from_bits(0x3C9F_49F4);
    const BOX: [f32; 6] = [-10.0, -20.0, -30.0, 10.0, 20.0, 30.0];

    #[test]
    fn inside_and_faces_are_zero() {
        assert_eq!(outcode(&[0.0, 0.0, 0.0], &BOX, EPS), 0);
        // Points exactly on the min/max corners sit on the faces: inside.
        assert_eq!(outcode(&[-10.0, -20.0, -30.0], &BOX, EPS), 0);
        assert_eq!(outcode(&[10.0, 20.0, 30.0], &BOX, EPS), 0);
    }

    #[test]
    fn per_axis_low_bits() {
        assert_eq!(outcode(&[-11.0, 0.0, 0.0], &BOX, EPS), 0x1);
        assert_eq!(outcode(&[0.0, -21.0, 0.0], &BOX, EPS), 0x2);
        assert_eq!(outcode(&[0.0, 0.0, -31.0], &BOX, EPS), 0x4);
        assert_eq!(outcode(&[-11.0, -21.0, -31.0], &BOX, EPS), 0x7);
    }

    #[test]
    fn per_axis_high_bits() {
        assert_eq!(outcode(&[11.0, 0.0, 0.0], &BOX, EPS), 0x8);
        assert_eq!(outcode(&[0.0, 21.0, 0.0], &BOX, EPS), 0x10);
        assert_eq!(outcode(&[0.0, 0.0, 31.0], &BOX, EPS), 0x20);
        assert_eq!(outcode(&[11.0, 21.0, 31.0], &BOX, EPS), 0x38);
    }

    #[test]
    fn known_mixed_value() {
        // x below min (bit 0), y inside, z above max (bit 5).
        let aabb = [0.0, 0.0, 0.0, 4.0, 4.0, 4.0];
        assert_eq!(outcode(&[-1.0, 2.0, 5.0], &aabb, EPS), 0x21);
    }

    #[test]
    fn epsilon_slack_boundary() {
        // Exact-arithmetic box at the origin: x = -eps gives (x - 0) + eps
        // == +0.0 (sign clear, still inside); x = -2*eps gives -eps (outside).
        let aabb = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        assert_eq!(outcode(&[-EPS * 0.5, 0.5, 0.5], &aabb, EPS), 0);
        assert_eq!(outcode(&[-EPS, 0.5, 0.5], &aabb, EPS), 0);
        assert_eq!(outcode(&[-EPS * 2.0, 0.5, 0.5], &aabb, EPS), 0x1);
        // Same band on a max face (exact at 0.0: coarser grids re-round eps).
        let neg = [-1.0, -1.0, -1.0, 0.0, 0.0, 0.0];
        assert_eq!(outcode(&[-0.5, EPS, -0.5], &neg, EPS), 0);
        assert_eq!(outcode(&[-0.5, EPS * 2.0, -0.5], &neg, EPS), 0x10);
    }

    /// Mirroring point and box through the origin swaps min-face and max-face bits.
    ///
    /// The arithmetic is bit-identical.
    #[test]
    fn mirror_metamorphic() {
        let pts: [[f32; 3]; 4] = [
            [-11.0, 3.0, 31.5],
            [9.99, -20.5, 0.0],
            [-10.02, 20.02, -30.02],
            [55.5, -0.125, -41.0],
        ];
        let mirrored_box = [-BOX[3], -BOX[4], -BOX[5], -BOX[0], -BOX[1], -BOX[2]];
        for p in pts {
            let a = outcode(&p, &BOX, EPS);
            let b = outcode(&[-p[0], -p[1], -p[2]], &mirrored_box, EPS);
            assert_eq!(b, ((a & 0x7) << 3) | ((a >> 3) & 0x7));
        }
    }

    /// Cycling the axes x->y->z->x cycles both bit triples the same way.
    #[test]
    fn axis_cycle_metamorphic() {
        let pts: [[f32; 3]; 3] = [[-11.0, 25.0, 0.0], [10.5, -20.5, 30.5], [-9.0, 19.0, -29.0]];
        for p in pts {
            let a = outcode(&p, &BOX, EPS);
            let cycled_p = [p[2], p[0], p[1]];
            let cycled_box = [BOX[2], BOX[0], BOX[1], BOX[5], BOX[3], BOX[4]];
            let b = outcode(&cycled_p, &cycled_box, EPS);
            let cycle = |c: u32| {
                let lo = c & 0x7;
                let hi = (c >> 3) & 0x7;
                let rot = |t: u32| ((t << 1) | (t >> 2)) & 0x7;
                (rot(hi) << 3) | rot(lo)
            };
            assert_eq!(b, cycle(a));
        }
    }
}

/// Inclusive cell rectangle of a 16x16 chunk-grid radius query.
///
/// Plus the routine's leftover scalar return.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct QueryRadiusRect {
    pub ret: i32,
    pub row_lo: i32,
    pub row_hi: i32,
    pub col_lo: i32,
    pub col_hi: i32,
}

/// Quantizes a world XY and query radius into the inclusive `[0, 15]` cell rectangle.
///
/// The chunk grid is 16x16.
///
/// `row` comes from `px`, `col` from `py`, each as
/// `round((origin - coord) * scale - half) & 0xf`. The cell radius `n` is
/// `round(radius * radius_scale)`, recovered through the host's add-`bias` /
/// shift-`14` / mask-`0xff` reinterpret. `ret` mirrors the reference's leftover
/// return: `0` whenever the row span is non-empty, else the clamped low row.
// The point, radius, origin, scale, half-cell, radius scale and bias are the
// reference's argument list, not a design choice.
#[allow(clippy::too_many_arguments)]
pub fn c_map_chunk_grid__query_radius__68b0d0(
    px: f32,
    py: f32,
    radius: f32,
    origin: f32,
    scale: f32,
    half: f32,
    radius_scale: f32,
    bias: f32,
) -> QueryRadiusRect {
    let row = round_i32(scale * (origin - px) - half) & 0xf;
    let col = round_i32(scale * (origin - py) - half) & 0xf;
    let n = (((radius * radius_scale).round_ties_even() + bias).to_bits() >> 14 & 0xff) as i32;
    let row_lo = (row - n).max(0);
    let col_lo = (col - n).max(0);
    let row_hi = (row + n).min(15);
    let col_hi = (col + n).min(15);
    let ret = if row_lo <= row_hi { 0 } else { row_lo };
    QueryRadiusRect {
        ret,
        row_lo,
        row_hi,
        col_lo,
        col_hi,
    }
}

/// Offset vector and squared distance from a 16x16 sub-grid candidate to the query point.
///
/// `cell_x`/`cell_y` are the cell's world base; `sub_row`/`sub_col` index the 8x8
/// sub-grid at `sub_spacing`; `lo`/`hi` are the candidate's Z extent averaged by
/// `z_mid`. Returns `[dx, dy, dz, dist2]`.
// The nine values are what the enclosing routine passes positionally at the
// candidate site; the count is a fact about that call, not a design choice.
#[allow(clippy::too_many_arguments)]
pub fn query_radius_candidate(
    cell_x: f32,
    cell_y: f32,
    sub_row: i32,
    sub_col: i32,
    sub_spacing: f32,
    lo: f32,
    hi: f32,
    z_mid: f32,
    world: &[f32; 3],
) -> [f32; 4] {
    let dx = (cell_x - sub_row as f32 * sub_spacing) - world[0];
    let dy = (cell_y - sub_col as f32 * sub_spacing) - world[1];
    let dz = (hi + lo) * z_mid - world[2];
    let dist2 = (dx * dx + dy * dy) + dz * dz;
    [dx, dy, dz, dist2]
}

#[cfg(test)]
mod tests_c_map_chunk_grid__query_radius__68b0d0 {
    use super::{c_map_chunk_grid__query_radius__68b0d0 as rect, query_radius_candidate};

    const ORIGIN: f32 = 17066.666;
    // Written as the decimal expansion of the constant the client stores, so it
    // reads as that float rather than a rounder one; the bits are unchanged.
    #[allow(clippy::excessive_precision)]
    const SCALE: f32 = 0.029999999;
    const HALF: f32 = 0.5;
    const RADIUS_SCALE: f32 = 0.24000001;
    const BIAS: f32 = 512.0;
    const SUB_SPACING: f32 = 4.1666665;
    const Z_MID: f32 = 0.5;

    #[test]
    fn radius_zero_single_cell() {
        let r = rect(ORIGIN, ORIGIN, 0.0, ORIGIN, SCALE, HALF, RADIUS_SCALE, BIAS);
        assert_eq!((r.row_lo, r.row_hi, r.col_lo, r.col_hi), (0, 0, 0, 0));
        assert_eq!(r.ret, 0);
    }

    #[test]
    fn cell_radius_magic_bias_matches_round() {
        for (radius, want) in [(10.0_f32, 2), (100.0_f32, 24), (0.0_f32, 0), (50.0_f32, 12)] {
            let n =
                (((radius * RADIUS_SCALE).round_ties_even() + BIAS).to_bits() >> 14 & 0xff) as i32;
            assert_eq!(n, want, "radius {radius}");
        }
    }

    #[test]
    fn big_radius_saturates_grid() {
        let r = rect(
            ORIGIN,
            ORIGIN,
            10000.0,
            ORIGIN,
            SCALE,
            HALF,
            RADIUS_SCALE,
            BIAS,
        );
        assert_eq!((r.row_lo, r.col_lo, r.row_hi, r.col_hi), (0, 0, 15, 15));
    }

    #[test]
    fn candidate_at_origin_zero_offset() {
        let world = [3.0_f32, 4.0, 5.0];
        let v = query_radius_candidate(3.0, 4.0, 0, 0, SUB_SPACING, 5.0, 5.0, Z_MID, &world);
        assert_eq!(v, [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn candidate_distance_known() {
        let world = [0.0_f32, 0.0, 0.0];
        let v = query_radius_candidate(3.0, 4.0, 0, 0, SUB_SPACING, 0.0, 6.0, Z_MID, &world);
        assert_eq!(v, [3.0, 4.0, 3.0, 34.0]);
    }

    #[test]
    fn candidate_subindex_steps_by_spacing() {
        let world = [0.0_f32, 0.0, 0.0];
        let v = query_radius_candidate(0.0, 0.0, 1, 2, SUB_SPACING, 0.0, 0.0, Z_MID, &world);
        assert_eq!(v[0], -SUB_SPACING);
        assert_eq!(v[1], -2.0 * SUB_SPACING);
    }
}

/// Screen-space Z for a terrain map chunk, recomputed each frame.
///
/// Evaluates the active view plane at the chunk's three cached coordinates and
/// subtracts the chunk's base height: `a*x + b*y + c*z + d - base`. `plane` is
/// `[a, b, c, d]` and `coords` is `[x, y, z]`; the accumulation order matches
/// the reference (the `b*y` product is summed first).
pub fn c_map_chunk__update__6afad0(plane: [f32; 4], coords: [f32; 3], base: f32) -> f32 {
    plane[1] * coords[1] + plane[2] * coords[2] + plane[0] * coords[0] + plane[3] - base
}

/// Centre of an axis-aligned box, scaled by the host's `half` (`0.5`) constant.
///
/// Returns `[(min[i] + max[i]) * half ; i in 0..3]`.
pub fn map_chunk_box_center(min: [f32; 3], max: [f32; 3], half: f32) -> [f32; 3] {
    [
        (min[0] + max[0]) * half,
        (min[1] + max[1]) * half,
        (min[2] + max[2]) * half,
    ]
}

/// Translates a chunk-local vertex into world space by adding the chunk origin.
pub fn map_chunk_vertex_world(vert: [f32; 3], origin: [f32; 3]) -> [f32; 3] {
    [
        vert[0] + origin[0],
        vert[1] + origin[1],
        vert[2] + origin[2],
    ]
}

/// AABB-vs-view-box overlap test used by the per-frame chunk tick.
///
/// Returns true when the box `[lo_corner, hi_corner]` overlaps the view box
/// `[view_lo, view_hi]`: every `lo_corner[i] <= view_hi[i]` and every
/// `hi_corner[i] >= view_lo[i]`. NaN on any axis fails, matching the reference's
/// ordered x87 compares. The shipped body runs the 4-lane form below; this
/// scalar transcription stays as its test oracle.
// Kept as the oracle the 4-lane property test compares against, so outside
// test builds it has no caller by design.
#[cfg_attr(not(test), allow(dead_code))]
pub fn map_chunk_box_overlaps_view(
    lo_corner: [f32; 3],
    hi_corner: [f32; 3],
    view_lo: [f32; 3],
    view_hi: [f32; 3],
) -> bool {
    lo_corner[0] <= view_hi[0]
        && lo_corner[1] <= view_hi[1]
        && lo_corner[2] <= view_hi[2]
        && hi_corner[0] >= view_lo[0]
        && hi_corner[1] >= view_lo[1]
        && hi_corner[2] >= view_lo[2]
}

/// 4-lane SSE form of [`map_chunk_box_overlaps_view`]; the fourth lane is ignored.
///
/// Callers may fill lane 3 with whatever dword happens to follow the 3-float
/// value in memory. `cmpleps` is an ordered compare: a NaN lane yields false,
/// exactly like the scalar `<=`/`>=`, and `view_lo <= hi_corner` is the same
/// ordered predicate as `hi_corner >= view_lo`, so the boolean is identical to
/// the scalar kernel's on every input (the property test pins this).
pub fn map_chunk_box_overlaps_view4(
    lo_corner: [f32; 4],
    hi_corner: [f32; 4],
    view_lo: [f32; 4],
    view_hi: [f32; 4],
) -> bool {
    #[cfg(target_arch = "x86")]
    use core::arch::x86::{_mm_and_ps, _mm_cmple_ps, _mm_loadu_ps, _mm_movemask_ps};
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::{_mm_and_ps, _mm_cmple_ps, _mm_loadu_ps, _mm_movemask_ps};
    // Every intrinsic below is SSE1, available on every ISA baseline this
    // crate builds for.
    // SAFETY: the load covers the 16 in-bounds bytes of its `[f32; 4]` argument.
    let lo = unsafe { _mm_loadu_ps(lo_corner.as_ptr()) };
    // SAFETY: as above.
    let hi = unsafe { _mm_loadu_ps(hi_corner.as_ptr()) };
    // SAFETY: as above.
    let vlo = unsafe { _mm_loadu_ps(view_lo.as_ptr()) };
    // SAFETY: as above.
    let vhi = unsafe { _mm_loadu_ps(view_hi.as_ptr()) };
    // SAFETY: lane-wise ordered compare of two initialized vectors.
    let below = unsafe { _mm_cmple_ps(lo, vhi) };
    // SAFETY: lane-wise ordered compare of two initialized vectors.
    let above = unsafe { _mm_cmple_ps(vlo, hi) };
    // SAFETY: lane-wise AND of two compare masks.
    let both = unsafe { _mm_and_ps(below, above) };
    // SAFETY: sign-bit extraction from an initialized vector.
    let mask = unsafe { _mm_movemask_ps(both) };
    (mask & 0b111) == 0b111
}

#[cfg(test)]
mod tests_c_map_chunk__update__6afad0 {
    use super::{
        c_map_chunk__update__6afad0 as screen_z, map_chunk_box_center as center,
        map_chunk_box_overlaps_view as overlaps, map_chunk_box_overlaps_view4 as overlaps4,
        map_chunk_vertex_world as vworld,
    };

    #[test]
    fn screen_z_known_value() {
        let z = screen_z([2.0, 3.0, 4.0, 5.0], [1.0, 1.0, 1.0], 1.0);
        assert_eq!(z.to_bits(), 13.0f32.to_bits());
    }

    #[test]
    fn screen_z_matches_machine_order() {
        let p = [1.5f32, -2.25, 0.75, 4.0];
        let c = [3.0f32, -1.0, 8.0];
        let base = 2.5f32;
        let expect = p[1] * c[1] + p[2] * c[2] + p[0] * c[0] + p[3] - base;
        assert_eq!(screen_z(p, c, base).to_bits(), expect.to_bits());
    }

    #[test]
    fn center_known() {
        let m = center([0.0, 2.0, 4.0], [2.0, 4.0, 6.0], 0.5);
        assert_eq!(m[0].to_bits(), 1.0f32.to_bits());
        assert_eq!(m[1].to_bits(), 3.0f32.to_bits());
        assert_eq!(m[2].to_bits(), 5.0f32.to_bits());
    }

    #[test]
    fn vertex_world_adds_origin() {
        assert_eq!(
            vworld([1.0, 2.0, 3.0], [10.0, 20.0, 30.0]),
            [11.0, 22.0, 33.0]
        );
    }

    #[test]
    fn overlap_true_and_false() {
        assert!(overlaps(
            [0.0, 0.0, 0.0],
            [10.0, 10.0, 10.0],
            [5.0, 5.0, 5.0],
            [20.0, 20.0, 20.0]
        ));
        assert!(!overlaps(
            [0.0, 0.0, 0.0],
            [10.0, 10.0, 3.0],
            [5.0, 5.0, 5.0],
            [20.0, 20.0, 20.0]
        ));
        assert!(!overlaps(
            [25.0, 0.0, 0.0],
            [30.0, 10.0, 10.0],
            [5.0, 5.0, 5.0],
            [20.0, 20.0, 20.0]
        ));
    }

    #[test]
    fn overlap_touching_edge_counts() {
        assert!(overlaps(
            [20.0, 5.0, 5.0],
            [5.0, 20.0, 20.0],
            [5.0, 5.0, 5.0],
            [20.0, 20.0, 20.0]
        ));
    }

    #[test]
    fn overlap_nan_fails() {
        assert!(!overlaps(
            [f32::NAN, 0.0, 0.0],
            [10.0, 10.0, 10.0],
            [5.0, 5.0, 5.0],
            [20.0, 20.0, 20.0]
        ));
    }

    /// The 4-lane kernel must agree with the scalar one on every input class.
    ///
    /// Covers ordinary orderings, touching edges, negative zero, infinities,
    /// and NaN across the significant lanes, with the ignored fourth lane set
    /// to values chosen to produce a wrong answer if it ever leaked into the
    /// mask.
    #[test]
    fn overlap4_matches_scalar_kernel() {
        let vals = [
            -1.0f32,
            0.0,
            -0.0,
            1.0,
            5.0,
            20.0,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
        ];
        let pad = |a: [f32; 3], junk: f32| [a[0], a[1], a[2], junk];
        for &a in &vals {
            for &b in &vals {
                let lo = [a, 0.0, 0.0];
                let hi = [b, 10.0, 10.0];
                let vlo = [0.0f32, b, 5.0];
                let vhi = [20.0f32, 20.0, a];
                for &junk in &[f32::NAN, f32::NEG_INFINITY, 0.0] {
                    assert_eq!(
                        overlaps(lo, hi, vlo, vhi),
                        overlaps4(pad(lo, junk), pad(hi, junk), pad(vlo, junk), pad(vhi, junk)),
                        "lo={lo:?} hi={hi:?} vlo={vlo:?} vhi={vhi:?} junk={junk}"
                    );
                }
            }
        }
    }

    #[test]
    fn overlap_translation_invariant() {
        let lo = [1.0f32, 2.0, 3.0];
        let hi = [11.0f32, 12.0, 13.0];
        let vlo = [5.0f32, 5.0, 5.0];
        let vhi = [20.0f32, 20.0, 20.0];
        for d in [-100.0f32, 0.0, 7.5, 250.0] {
            let t3 = |a: [f32; 3]| [a[0] + d, a[1] + d, a[2] + d];
            assert_eq!(
                overlaps(lo, hi, vlo, vhi),
                overlaps(t3(lo), t3(hi), t3(vlo), t3(vhi))
            );
        }
    }
}

/// Per-group geometry-test flag word derived from the trace flag bits.
pub fn c_map_obj__group_test_flags__6a37b0(flags: u32) -> u32 {
    let mut f: u32 = if flags & 0x10 != 0 { 0xc6 } else { 0xee };
    if flags & 0x20 != 0 {
        f &= !0x24;
    }
    if flags & 0x40 == 0 {
        f &= !0x02;
    }
    if flags & 0x4000 == 0 {
        f &= !0x40;
    }
    f
}

/// Geometric normal of a hit triangle and its dot product with the segment direction.
///
/// Edges are `a = v2 - v0`, `b = v1 - v0`; the normal is `cross(b, a)` and the
/// dot accumulates `(nz*sz + ny*sy) + nx*sx` like the reference.
pub fn c_map_obj__hit_normal_dot__6a37b0(
    v0: &[f32; 3],
    v1: &[f32; 3],
    v2: &[f32; 3],
    seg: &[f32; 3],
) -> ([f32; 3], f32) {
    let a = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
    let b = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
    let n = [
        b[1] * a[2] - b[2] * a[1],
        b[2] * a[0] - a[2] * b[0],
        b[0] * a[1] - b[1] * a[0],
    ];
    let dot = (n[2] * seg[2] + n[1] * seg[1]) + n[0] * seg[0];
    (n, dot)
}

/// Final facet-dot normalisation.
///
/// `dot / |n|` with the reference's `(nz² + ny²) + nx²` accumulation (a zero
/// normal divides by zero).
pub fn c_map_obj__normalize_facet_dot__6a37b0(dot: f32, n: &[f32; 3]) -> f32 {
    dot / ((n[2] * n[2] + n[1] * n[1]) + n[0] * n[0]).sqrt()
}

#[cfg(test)]
mod tests_c_map_obj__trace_line_groups__6a37b0 {
    use super::{
        c_map_obj__group_test_flags__6a37b0 as test_flags,
        c_map_obj__hit_normal_dot__6a37b0 as normal_dot,
        c_map_obj__normalize_facet_dot__6a37b0 as norm_dot,
    };

    #[test]
    fn flag_word_known_values() {
        assert_eq!(test_flags(0), 0xac);
        assert_eq!(test_flags(0x10), 0x84);
        assert_eq!(test_flags(0x70), 0x82);
        assert_eq!(test_flags(0x4040), 0xee);
        assert_eq!(test_flags(0x4070), 0xc2);
    }

    #[test]
    fn unit_triangle_normal_and_dot() {
        let v0 = [0.0, 0.0, 0.0];
        let v1 = [1.0, 0.0, 0.0];
        let v2 = [0.0, 1.0, 0.0];
        let (n, d) = normal_dot(&v0, &v1, &v2, &[0.0, 0.0, -1.0]);
        assert_eq!(n[0].to_bits(), 0.0f32.to_bits());
        assert_eq!(n[1].to_bits(), 0.0f32.to_bits());
        assert_eq!(n[2].to_bits(), 1.0f32.to_bits());
        assert_eq!(d.to_bits(), (-1.0f32).to_bits());
    }

    #[test]
    fn normal_is_orthogonal_to_both_edges() {
        let v0 = [1.0, 2.0, 3.0];
        let v1 = [4.0, -1.0, 2.0];
        let v2 = [0.5, 7.0, -2.0];
        let (n, _) = normal_dot(&v0, &v1, &v2, &[0.0, 0.0, 0.0]);
        let a = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
        let b = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
        let da = n[0] * a[0] + n[1] * a[1] + n[2] * a[2];
        let db = n[0] * b[0] + n[1] * b[1] + n[2] * b[2];
        assert!(da.abs() < 1e-3, "n.a = {da}");
        assert!(db.abs() < 1e-3, "n.b = {db}");
    }

    #[test]
    fn normalize_divides_by_length() {
        let r = norm_dot(6.0, &[0.0, 3.0, 4.0]);
        assert_eq!(r.to_bits(), 1.2f32.to_bits());
    }

    #[test]
    fn zero_normal_divides_to_non_finite() {
        // Faithful to the reference: an untouched zero normal yields inf/NaN.
        assert!(norm_dot(1.0, &[0.0, 0.0, 0.0]).is_infinite());
        assert!(norm_dot(0.0, &[0.0, 0.0, 0.0]).is_nan());
    }
}

/// 6-bit AABB exclusion outcode of a BSP leaf vertex against a query box.
///
/// `box6` is `[min_x, min_y, min_z, max_x, max_y, max_z]`. A bit is set when the
/// vertex is on or past a face: `0x20`/`0x10` for `x <= min`/`x >= max`,
/// `0x08`/`0x04` for y, `0x02`/`0x01` for z (boundary contact counts as outside;
/// the inside region is the open box `min < v < max`). Three vertices sharing a
/// set bit (`& 0x3f`) span a triangle trivially outside that face.
pub fn c_world_bsp__collect_leaf_triangles_in_box__6b8c60(v: &[f32; 3], box6: &[f32; 6]) -> u8 {
    let mut c = 0u8;
    if v[0] <= box6[0] {
        c |= 0x20;
    }
    if v[0] >= box6[3] {
        c |= 0x10;
    }
    if v[1] <= box6[1] {
        c |= 0x08;
    }
    if v[1] >= box6[4] {
        c |= 0x04;
    }
    if v[2] <= box6[2] {
        c |= 0x02;
    }
    if v[2] >= box6[5] {
        c |= 0x01;
    }
    c
}

#[cfg(test)]
mod tests_c_world_bsp__collect_leaf_triangles_in_box__6b8c60 {
    use super::c_world_bsp__collect_leaf_triangles_in_box__6b8c60 as outcode;

    const BOX: [f32; 6] = [-10.0, -20.0, -30.0, 10.0, 20.0, 30.0];

    #[test]
    fn inside_is_zero() {
        assert_eq!(outcode(&[0.0, 0.0, 0.0], &BOX), 0);
    }

    #[test]
    fn faces_count_as_outside() {
        assert_eq!(outcode(&[-10.0, 0.0, 0.0], &BOX), 0x20);
        assert_eq!(outcode(&[0.0, 0.0, 30.0], &BOX), 0x01);
    }

    #[test]
    fn per_axis_bits() {
        assert_eq!(outcode(&[-11.0, 0.0, 0.0], &BOX), 0x20);
        assert_eq!(outcode(&[11.0, 0.0, 0.0], &BOX), 0x10);
        assert_eq!(outcode(&[0.0, -21.0, 0.0], &BOX), 0x08);
        assert_eq!(outcode(&[0.0, 21.0, 0.0], &BOX), 0x04);
        assert_eq!(outcode(&[0.0, 0.0, -31.0], &BOX), 0x02);
        assert_eq!(outcode(&[0.0, 0.0, 31.0], &BOX), 0x01);
    }

    #[test]
    fn corners_combine_bits() {
        assert_eq!(outcode(&[-11.0, -21.0, -31.0], &BOX), 0x2a);
        assert_eq!(outcode(&[11.0, 21.0, 31.0], &BOX), 0x15);
    }

    #[test]
    fn nan_clears_bits() {
        assert_eq!(outcode(&[f32::NAN, 0.0, 0.0], &BOX), 0);
    }

    #[test]
    fn trivial_reject_and() {
        let a = outcode(&[-11.0, 0.0, 0.0], &BOX);
        let b = outcode(&[-12.0, 5.0, 5.0], &BOX);
        let c = outcode(&[-13.0, -5.0, -5.0], &BOX);
        assert_ne!(a & b & c & 0x3f, 0);
        let d = outcode(&[0.0, 0.0, 0.0], &BOX);
        assert_eq!(a & b & d & 0x3f, 0);
    }
}

/// Inclusive AABB overlap test of a WMO-group box against a query box.
///
/// Each box is `[min_x, min_y, min_z, max_x, max_y, max_z]`. Overlap requires the
/// group minimum to be `<=` the query maximum and the group maximum to be `>=`
/// the query minimum on every axis (boundary contact counts as overlap). The
/// group walk at `0x6abc40` runs the predicate as [`map_chunk_box_overlaps_view4`]
/// over the lanes [`map_chunk_box_lanes`] builds; this scalar form stays the
/// oracle its property test compares against.
pub fn map_chunk__query_wmo_groups__6abc40(group: &[f32; 6], query: &[f32; 6]) -> bool {
    group[0] <= query[3]
        && group[1] <= query[4]
        && group[2] <= query[5]
        && group[3] >= query[0]
        && group[4] >= query[1]
        && group[5] >= query[2]
}

/// Splits a `[min_xyz, max_xyz]` box into the two 4-lane vectors the SSE overlap test takes.
///
/// [`map_chunk_box_overlaps_view4`] masks lane 3 away, so it is a don't-care in
/// both vectors: the minimum vector carries `bounds[3]` there because that is the
/// dword following the minimum triple, and the maximum vector pads with zero.
/// Splitting by copy rather than by a 16-byte load keeps the read inside the six
/// declared floats. A caller whose box is loop-invariant builds it once.
pub fn map_chunk_box_lanes(bounds: &[f32; 6]) -> ([f32; 4], [f32; 4]) {
    (
        [bounds[0], bounds[1], bounds[2], bounds[3]],
        [bounds[3], bounds[4], bounds[5], 0.0],
    )
}

#[cfg(test)]
mod tests_map_chunk__query_wmo_groups__6abc40 {
    use super::{
        map_chunk__query_wmo_groups__6abc40 as overlap, map_chunk_box_lanes as lanes,
        map_chunk_box_overlaps_view4 as overlaps4,
    };

    #[test]
    fn concentric_overlaps() {
        let g = [-1.0_f32, -1.0, -1.0, 1.0, 1.0, 1.0];
        let q = [-2.0_f32, -2.0, -2.0, 2.0, 2.0, 2.0];
        assert!(overlap(&g, &q));
    }

    #[test]
    fn touching_face_overlaps() {
        let g = [-2.0_f32, 0.0, 0.0, 0.0, 1.0, 1.0];
        let q = [0.0_f32, 0.0, 0.0, 2.0, 1.0, 1.0];
        assert!(overlap(&g, &q));
    }

    #[test]
    fn separated_x_misses() {
        let g = [-5.0_f32, 0.0, 0.0, -3.0, 1.0, 1.0];
        let q = [0.0_f32, 0.0, 0.0, 2.0, 1.0, 1.0];
        assert!(!overlap(&g, &q));
    }

    #[test]
    fn separated_y_misses() {
        let g = [0.0_f32, 10.0, 0.0, 1.0, 12.0, 1.0];
        let q = [0.0_f32, 0.0, 0.0, 1.0, 1.0, 1.0];
        assert!(!overlap(&g, &q));
    }

    /// The split boxes fed to the 4-lane kernel must answer as this kernel does.
    ///
    /// Sweeps ordinary orderings, touching faces, negative zero, both infinities
    /// and NaN through a minimum and a maximum lane of each box. The fourth lane
    /// is a don't-care, so a disagreement here is it leaking into the mask.
    #[test]
    fn lane_split_matches_scalar_kernel() {
        let vals = [
            -1.0f32,
            0.0,
            -0.0,
            1.0,
            5.0,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
        ];
        for &a in &vals {
            for &b in &vals {
                let g = [a, -1.0, -1.0, b, 1.0, 1.0];
                let q = [-2.0, b, -2.0, 2.0, a, 2.0];
                let (gmin4, gmax4) = lanes(&g);
                let (qmin4, qmax4) = lanes(&q);
                assert_eq!(
                    overlap(&g, &q),
                    overlaps4(gmin4, gmax4, qmin4, qmax4),
                    "g={g:?} q={q:?}"
                );
            }
        }
    }
}

/// Low/high cell indices (per axis, masked to a 64-wide grid) covered by a circular region.
///
/// `ftol(floor((center ± radius) * scale ∓ bias)) & 0x3f`.
///
/// The reference evaluates each bound in extended precision and floors the
/// double; the four results are `[x_lo, x_hi, y_lo, y_hi]`.
pub fn spatial_grid__prune_region__70a0f0(
    center: [f32; 2],
    radius: f32,
    scale: f32,
    bias: f32,
) -> [u32; 4] {
    let cell = |c: f32, r: f32, b: f32| -> u32 {
        let v = (f64::from(c) + f64::from(r)) * f64::from(scale) + f64::from(b);
        (crate::math::misc::ftol__40a2b0(v.floor()) as u32) & 0x3f
    };
    [
        cell(center[0], -radius, -bias),
        cell(center[0], radius, bias),
        cell(center[1], -radius, -bias),
        cell(center[1], radius, bias),
    ]
}

#[cfg(test)]
mod tests_spatial_grid__prune_region__70a0f0 {
    use super::spatial_grid__prune_region__70a0f0 as cells;

    #[test]
    fn unit_scale_wraps_negative_low_bound() {
        // floor(-1) = -1 -> masked to 0x3f; floor(1) = 1.
        assert_eq!(cells([0.0, 0.0], 1.0, 1.0, 0.0), [0x3f, 1, 0x3f, 1]);
    }

    #[test]
    fn fractional_center_floors_per_bound() {
        // x: floor(10.25)=10, floor(10.75)=10; y: floor(20.0)=20, floor(20.5)=20.
        assert_eq!(cells([10.5, 20.25], 0.25, 1.0, 0.0), [10, 10, 20, 20]);
    }

    #[test]
    fn bias_widens_both_bounds() {
        // x: floor(10.25-0.5)=9, floor(10.75+0.5)=11; y: floor(19.5)=19, floor(21.0)=21.
        let r = cells([10.5, 20.25], 0.25, 1.0, 0.5);
        assert_eq!(r, [9, 11, 19, 21]);
    }

    #[test]
    fn scale_compresses_world_units() {
        // 128 world units at 1/64 scale: floor(2.0)=2 on both x bounds (r=0).
        assert_eq!(cells([128.0, 256.0], 0.0, 1.0 / 64.0, 0.0), [2, 2, 4, 4]);
    }

    #[test]
    fn mask_keeps_low_six_bits() {
        // floor(100) = 100 -> 100 & 0x3f = 36.
        assert_eq!(cells([100.0, 100.0], 0.0, 1.0, 0.0), [36, 36, 36, 36]);
    }
}

/// Grid cell coordinate of one world-space component: `ftol(floor(scale * v))`.
pub fn terrain_height_cache_trace_los_cell__67cb60(v: f32, scale: f32) -> i32 {
    // The product of two f32 values is exact in f64; floor of the extended
    // intermediate then truncates to the low 32 bits exactly as the reference.
    crate::math::misc::ftol__40a2b0((f64::from(scale) * f64::from(v)).floor()) as i32
}

/// Precomputed stepping state for the cell-grid line walk.
pub struct TraceLosSetup {
    /// Number of major-axis steps (clamped to at least 1).
    pub major: i32,
    /// Parametric step per iteration (`one / major`).
    pub step_t: f32,
    /// Initial decision accumulator.
    pub err: i32,
    /// Accumulator increment when `err <= 0`.
    pub inc_le: i32,
    /// Accumulator increment when `err > 0`.
    pub inc_gt: i32,
    /// Cell advance `[x, y]` applied when `err <= 0`.
    pub step_le: [i32; 2],
    /// Cell advance `[x, y]` applied when `err > 0`.
    pub step_gt: [i32; 2],
}

/// Builds the discrete line-walk state between two grid cells.
///
/// The major axis is the one with the larger absolute cell delta (ties pick
/// the second axis); step signs come from strict float comparisons of the
/// world-space endpoints, so a NaN component leaves the positive step.
pub fn terrain_height_cache_trace_los_setup__67cb60(
    start_cell: [i32; 2],
    end_cell: [i32; 2],
    start: &[f32; 3],
    end: &[f32; 3],
    one: f32,
) -> TraceLosSetup {
    let adx = end_cell[0].wrapping_sub(start_cell[0]).wrapping_abs();
    let ady = end_cell[1].wrapping_sub(start_cell[1]).wrapping_abs();

    // Major-axis pick: second axis wins when |dx| <= |dy|.
    let y_major = adx <= ady;
    let (major_raw, minor) = if y_major { (ady, adx) } else { (adx, ady) };
    let err = minor.wrapping_mul(2).wrapping_sub(major_raw);
    let inc_le = minor.wrapping_mul(2);
    let inc_gt = minor.wrapping_sub(major_raw).wrapping_mul(2);

    let (mut sx_le, mut sy_le) = if y_major { (0, 1) } else { (1, 0) };
    let (mut sx_gt, mut sy_gt) = (1, 1);
    if start[0] > end[0] {
        sx_le = -sx_le;
        sx_gt = -1;
    }
    if start[1] > end[1] {
        sy_le = -sy_le;
        sy_gt = -1;
    }

    let major = if major_raw <= 0 { 1 } else { major_raw };
    TraceLosSetup {
        major,
        step_t: one / major as f32,
        err,
        inc_le,
        inc_gt,
        step_le: [sx_le, sy_le],
        step_gt: [sx_gt, sy_gt],
    }
}

/// Interpolated sample position at parameter `t` along the segment.
pub fn terrain_height_cache_trace_los_pos__67cb60(
    start: &[f32; 3],
    delta: &[f32; 3],
    t: f32,
) -> [f32; 3] {
    [
        delta[0] * t + start[0],
        delta[1] * t + start[1],
        delta[2] * t + start[2],
    ]
}

#[cfg(test)]
mod tests_terrain_height_cache_trace_los__67cb60 {
    use super::{
        terrain_height_cache_trace_los_cell__67cb60 as cell,
        terrain_height_cache_trace_los_pos__67cb60 as pos,
        terrain_height_cache_trace_los_setup__67cb60 as setup,
    };

    fn walk(s: &super::TraceLosSetup, from: [i32; 2]) -> [i32; 2] {
        let mut cur = from;
        let mut err = s.err;
        for _ in 0..s.major {
            if err > 0 {
                cur[0] += s.step_gt[0];
                cur[1] += s.step_gt[1];
                err += s.inc_gt;
            } else {
                cur[0] += s.step_le[0];
                cur[1] += s.step_le[1];
                err += s.inc_le;
            }
        }
        cur
    }

    #[test]
    fn cell_floors_scaled_coordinate() {
        assert_eq!(cell(3.25, 2.0), 6);
        assert_eq!(cell(-0.1, 1.0), -1);
        assert_eq!(cell(0.0, 1.0), 0);
    }

    #[test]
    fn x_major_walk_reaches_end_cell() {
        let s = setup([0, 0], [5, 2], &[0.0, 0.0, 0.0], &[5.0, 2.0, 0.0], 1.0);
        assert_eq!(s.major, 5);
        assert_eq!(s.err, -1);
        assert_eq!(s.step_le, [1, 0]);
        assert_eq!(s.step_gt, [1, 1]);
        assert_eq!(walk(&s, [0, 0]), [5, 2]);
    }

    #[test]
    fn y_major_walk_reaches_end_cell() {
        let s = setup([0, 0], [2, 5], &[0.0, 0.0, 0.0], &[2.0, 5.0, 0.0], 1.0);
        assert_eq!(s.major, 5);
        assert_eq!(s.step_le, [0, 1]);
        assert_eq!(walk(&s, [0, 0]), [2, 5]);
    }

    #[test]
    fn negative_direction_flips_step_signs() {
        let s = setup([0, 0], [-5, 2], &[5.0, 0.0, 0.0], &[0.0, 2.0, 0.0], 1.0);
        assert_eq!(s.step_le, [-1, 0]);
        assert_eq!(s.step_gt, [-1, 1]);
        assert_eq!(walk(&s, [0, 0]), [-5, 2]);
        let s = setup([0, 0], [2, -5], &[0.0, 5.0, 0.0], &[2.0, 0.0, 0.0], 1.0);
        assert_eq!(s.step_le, [0, -1]);
        assert_eq!(s.step_gt, [1, -1]);
        assert_eq!(walk(&s, [0, 0]), [2, -5]);
    }

    #[test]
    fn degenerate_trace_clamps_major_to_one() {
        // Equal cells: the reference still takes one (y-major) unit step.
        let s = setup([3, 3], [3, 3], &[1.0, 1.0, 0.0], &[1.0, 1.0, 0.0], 1.0);
        assert_eq!(s.major, 1);
        assert_eq!(s.step_t.to_bits(), 1.0f32.to_bits());
        assert_eq!(walk(&s, [3, 3]), [3, 4]);
    }

    #[test]
    fn pos_interpolates_endpoints() {
        let start = [1.0, 2.0, 3.0];
        let delta = [4.0, -2.0, 6.0];
        let p0 = pos(&start, &delta, 0.0);
        assert_eq!(p0[0].to_bits(), 1.0f32.to_bits());
        let p1 = pos(&start, &delta, 1.0);
        assert_eq!(p1[0].to_bits(), 5.0f32.to_bits());
        assert_eq!(p1[1].to_bits(), 0.0f32.to_bits());
        assert_eq!(p1[2].to_bits(), 9.0f32.to_bits());
        let ph = pos(&start, &delta, 0.5);
        assert_eq!(ph[2].to_bits(), 6.0f32.to_bits());
    }
}

/// Per-frame fade-band bases: `[a0 - d0, a1 - d1, min(limit, a2) - d2]`.
///
/// The third base clamps its range start to a runtime view limit when that
/// limit is strictly smaller (NaN keeps the static range).
pub fn world_scene__deferred_fade_bases__683f80(
    a0: f32,
    d0: f32,
    a1: f32,
    d1: f32,
    a2: f32,
    d2: f32,
    limit: f32,
) -> [f32; 3] {
    let far2 = if limit < a2 { limit } else { a2 };
    [a0 - d0, a1 - d1, far2 - d2]
}

/// Distance-fade alpha of one deferred scene node.
///
/// `None` when the node is below the visibility threshold and must not be
/// submitted.
///
/// The fade band is selected by the node radius against three cutoffs; the
/// fade input is the horizontal camera distance minus the radius. Alphas
/// above `one` clamp to 1.0; only `alpha > min_alpha` submits (a NaN alpha
/// skips, matching the reference's comparison chain).
// The ten parameters are the reference's argument list. The cutoff tests are
// written `!(radius <= cut)` so a NaN radius takes the fully-visible branch,
// matching the original's compare chain.
#[allow(clippy::too_many_arguments, clippy::neg_cmp_op_on_partial_ord)]
pub fn world_scene__deferred_fade_alpha__683f80(
    center_xy: [f32; 2],
    cam_xy: [f32; 2],
    radius: f32,
    cut0: f32,
    cut1: f32,
    cut2: f32,
    bases: [f32; 3],
    divs: [f32; 3],
    one: f32,
    min_alpha: f32,
) -> Option<f32> {
    // Radius above the outer cutoff (or NaN): always fully visible.
    if !(radius <= cut0) {
        return Some(1.0);
    }
    let dx = center_xy[0] - cam_xy[0];
    let dy = center_xy[1] - cam_xy[1];
    let dist = (dy * dy + dx * dx).sqrt() - radius;
    let raw = if radius <= cut1 {
        (dist - bases[0]) / divs[0]
    } else if !(radius <= cut2) {
        (dist - bases[2]) / divs[2]
    } else {
        (dist - bases[1]) / divs[1]
    };
    let alpha = one - raw;
    if alpha > one {
        return Some(1.0);
    }
    if alpha > min_alpha { Some(alpha) } else { None }
}

#[cfg(test)]
mod tests_world_scene__flush_deferred_models__683f80 {
    use super::{
        world_scene__deferred_fade_alpha__683f80 as fade,
        world_scene__deferred_fade_bases__683f80 as bases,
    };

    const CUTS: (f32, f32, f32) = (100.0, 10.0, 50.0);
    const BASES: [f32; 3] = [9.0, 19.0, 29.0];
    const DIVS: [f32; 3] = [20.0, 40.0, 60.0];

    fn run(center: [f32; 2], radius: f32) -> Option<f32> {
        fade(
            center,
            [0.0, 0.0],
            radius,
            CUTS.0,
            CUTS.1,
            CUTS.2,
            BASES,
            DIVS,
            1.0,
            0.01,
        )
    }

    #[test]
    fn bases_pick_smaller_limit() {
        let b = bases(10.0, 1.0, 20.0, 2.0, 30.0, 3.0, 25.0);
        assert_eq!(b[0].to_bits(), 9.0f32.to_bits());
        assert_eq!(b[1].to_bits(), 18.0f32.to_bits());
        assert_eq!(b[2].to_bits(), 22.0f32.to_bits());
        let b = bases(10.0, 1.0, 20.0, 2.0, 30.0, 3.0, 35.0);
        assert_eq!(b[2].to_bits(), 27.0f32.to_bits());
    }

    #[test]
    fn radius_above_outer_cutoff_is_fully_visible() {
        assert_eq!(run([1.0e6, 0.0], 101.0), Some(1.0));
    }

    #[test]
    fn near_node_clamps_to_one() {
        // dist - r = 5 - 1 = 4; raw = (4-9)/20 < 0 -> alpha > 1 -> clamp.
        assert_eq!(run([3.0, 4.0], 1.0), Some(1.0));
    }

    #[test]
    fn mid_band_returns_linear_alpha() {
        // dist - r = 20 - 1 = 19; raw = (19-9)/20 = 0.5 -> alpha 0.5.
        let a = run([12.0, 16.0], 1.0).unwrap();
        assert_eq!(a.to_bits(), 0.5f32.to_bits());
    }

    #[test]
    fn far_node_is_skipped() {
        // dist - r = 50 - 1 = 49; raw = 2 -> alpha = -1 -> below min.
        assert_eq!(run([30.0, 40.0], 1.0), None);
    }

    #[test]
    fn band_selection_by_radius() {
        // radius 20: middle band (cut1 < r <= cut2) -> divisor 40.
        // dist - r = 100 - 20 = 80; raw = (80-19)/40 = 1.525; alpha < 0 -> None.
        assert_eq!(run([60.0, 80.0], 20.0), None);
        // radius 60: outer band (r > cut2) -> divisor 60, base 29.
        // dist - r = 100 - 60 = 40; raw = (40-29)/60; alpha ~ 0.8166.
        let a = run([60.0, 80.0], 60.0).unwrap();
        let want = 1.0f32 - (40.0f32 - 29.0) / 60.0;
        assert_eq!(a.to_bits(), want.to_bits());
    }

    #[test]
    fn nan_radius_is_fully_visible() {
        assert_eq!(run([0.0, 0.0], f32::NAN), Some(1.0));
    }
}

/// View-space depth sort key of a scene node.
///
/// One view-matrix row applied to the node centre, minus the node radius.
///
/// Accumulation order matches the reference: `(r1*y + r2*z) + r0*x + r3 - radius`.
pub fn world_scene__node_depth_key__683700(center: &[f32; 3], row: &[f32; 4], radius: f32) -> f32 {
    (row[1] * center[1] + row[2] * center[2]) + row[0] * center[0] + row[3] - radius
}

/// Squared distance from a node centre to the camera position.
///
/// Accumulation order matches the reference: `(dz*dz + dx*dx) + dy*dy`. The
/// chain never reaches memory — the deltas stay on the x87 stack and the sole
/// consumer is a `FCOMP m32` against the threshold, which promotes the f32
/// threshold rather than narrowing the sum. So this returns `f64` and the
/// comparison belongs at that width too.
pub fn world_scene__node_cam_dist_sq__683700(center: &[f32; 3], cam: &[f32; 3]) -> f64 {
    let dx = f64::from(center[0]) - f64::from(cam[0]);
    let dy = f64::from(center[1]) - f64::from(cam[1]);
    let dz = f64::from(center[2]) - f64::from(cam[2]);
    (dz * dz + dx * dx) + dy * dy
}

#[cfg(test)]
mod tests_world_scene__update_and_cull_nodes__683700 {
    use super::{
        world_scene__node_cam_dist_sq__683700 as dist_sq,
        world_scene__node_depth_key__683700 as depth_key,
    };

    #[test]
    fn depth_key_identity_row_returns_axis_minus_radius() {
        // Row picks x only: key = x + 0 - radius.
        let k = depth_key(&[7.0, 100.0, -3.0], &[1.0, 0.0, 0.0, 0.0], 2.0);
        assert_eq!(k.to_bits(), 5.0f32.to_bits());
    }

    #[test]
    fn depth_key_translation_only() {
        let k = depth_key(&[0.0, 0.0, 0.0], &[0.5, 0.25, 0.125, 9.0], 1.0);
        assert_eq!(k.to_bits(), 8.0f32.to_bits());
    }

    #[test]
    fn depth_key_full_row() {
        // (2*3 + 3*4) + 1*2 + 10 - 5 = 25.
        let k = depth_key(&[2.0, 3.0, 4.0], &[1.0, 2.0, 3.0, 10.0], 5.0);
        assert_eq!(k.to_bits(), 25.0f32.to_bits());
    }

    #[test]
    fn dist_sq_matches_pythagoras() {
        let d = dist_sq(&[3.0, 4.0, 0.0], &[0.0, 0.0, 0.0]);
        assert_eq!(d.to_bits(), 25.0f64.to_bits());
    }

    #[test]
    fn dist_sq_translation_invariant() {
        let a = dist_sq(&[1.0, 2.0, 3.0], &[4.0, 6.0, 3.0]);
        let b = dist_sq(&[11.0, 12.0, 13.0], &[14.0, 16.0, 13.0]);
        assert_eq!(a.to_bits(), b.to_bits());
        assert_eq!(a.to_bits(), 25.0f64.to_bits());
    }
}

/// Grid lookup derived from a world position for the surface-height sampler.
pub struct SurfaceCellLookup {
    /// Scaled grid-space coordinates `[x, y]` (kept for the liquid sampler).
    pub coord: [f32; 2],
    /// Rounded integer cell coordinates `[x, y]`.
    pub cell: [i32; 2],
    /// Index into the 64x64 top-level tile-pointer grid (< 4096).
    pub tile_index: usize,
    /// Index into a tile's 16x16 chunk-pointer table (< 256).
    pub chunk_index: usize,
}

/// Resolves the tile/chunk grid indices under a world XY position.
///
/// Coordinates are mirrored about `origin`, scaled, then rounded to the
/// nearest integer (ties to even) after a half-cell bias; tile and chunk
/// indices take bits `[13:7]` and `[6:3]` of each axis.
pub fn world_sample_surface_height_cells__6b7070(
    x: f32,
    y: f32,
    origin: f32,
    scale: f32,
    half: f32,
) -> SurfaceCellLookup {
    let coord = |v: f32| -> f32 { ((f64::from(origin) - f64::from(v)) * f64::from(scale)) as f32 };
    let cell = |c: f32| -> i32 {
        crate::math::misc::ftol__40a2b0((f64::from(c) - f64::from(half)).round_ties_even()) as i32
    };
    let coord_x = coord(x);
    let coord_y = coord(y);
    let cell_x = cell(coord_x);
    let cell_y = cell(coord_y);
    let tile_index = ((((cell_x >> 7) & 0x3f) as usize) << 6) | ((cell_y >> 7) & 0x3f) as usize;
    let chunk_index = ((((cell_x >> 3) & 0xf) as usize) << 4) | ((cell_y >> 3) & 0xf) as usize;
    SurfaceCellLookup {
        coord: [coord_x, coord_y],
        cell: [cell_x, cell_y],
        tile_index,
        chunk_index,
    }
}

#[cfg(test)]
mod tests_world_sample_surface_height__6b7070 {
    use super::world_sample_surface_height_cells__6b7070 as cells;

    #[test]
    fn origin_mirror_and_scale() {
        let l = cells(-100.0, 50.0, 0.0, 1.0, 0.5);
        // coord = -(v - 0) * 1 = -v.
        assert_eq!(l.coord[0].to_bits(), 100.0f32.to_bits());
        assert_eq!(l.coord[1].to_bits(), (-50.0f32).to_bits());
        // round_ties_even(99.5) = 100; round_ties_even(-50.5) = -50.
        assert_eq!(l.cell, [100, -50]);
    }

    #[test]
    fn tile_and_chunk_bits() {
        let l = cells(-300.0, -200.0, 0.0, 1.0, 0.5);
        assert_eq!(l.cell, [300, 200]);
        // 300 >> 7 = 2; 200 >> 7 = 1 -> tile = 2*64 + 1.
        assert_eq!(l.tile_index, 2 * 64 + 1);
        // (300 >> 3) & 0xf = 37 & 0xf = 5; (200 >> 3) & 0xf = 25 & 0xf = 9.
        assert_eq!(l.chunk_index, 5 * 16 + 9);
    }

    #[test]
    fn negative_cells_use_arithmetic_shift() {
        let l = cells(10.0, 10.0, 0.0, 1.0, 0.5);
        // coord = -10; cell = round_ties_even(-10.5) = -10 (even tie).
        assert_eq!(l.cell, [-10, -10]);
        // -10 >> 7 = -1 -> & 0x3f = 0x3f.
        assert_eq!(l.tile_index, 0x3f * 64 + 0x3f);
        // -10 >> 3 = -2 -> & 0xf = 0xe.
        assert_eq!(l.chunk_index, 0xe * 16 + 0xe);
    }

    // The scale literal is the decimal expansion of the constant the client
    // stores; it sits in a `let`, so the allow goes on the enclosing test fn.
    #[allow(clippy::excessive_precision)]
    #[test]
    fn world_constants_shape() {
        // Realistic constants: origin 17066.666, scale 1/533.33333 * 16 ~= 0.03.
        let origin = 17066.666f32;
        let scale = 0.029_999_999f32;
        let l = cells(origin, origin, origin, scale, 0.5);
        assert_eq!(l.cell, [0, 0]);
        assert_eq!(l.tile_index, 0);
        assert_eq!(l.chunk_index, 0);
    }

    #[test]
    fn half_bias_shifts_rounding() {
        // coord = 5.25; cell = round_ties_even(4.75) = 5.
        let l = cells(-5.25, -5.25, 0.0, 1.0, 0.5);
        assert_eq!(l.cell, [5, 5]);
        // coord = 5.0; cell = round_ties_even(4.5) = 4 (tie to even).
        let l = cells(-5.0, -5.0, 0.0, 1.0, 0.5);
        assert_eq!(l.cell, [4, 4]);
    }
}

/// `CMapChunk::DecompressVertexNormals`.
///
/// Decode 145 packed signed-byte normals into f32 components.
///
/// Each packed normal is three contiguous `i8` components (range -128..127). The
/// stock body widens each byte (`movsx`/`FILD`) and multiplies by the `.rdata`
/// constant `1.0/127.0` (f32 bits `0x3c01_0204`) with a single `FMUL`, storing the
/// rounded f32 at a 12-byte stride; component order is straight 0,1,2. The
/// `i8 -> f32` widening is exact, so the lone multiply rounds once — bit-exact to
/// the x87 original.
pub fn c_map_chunk__decompress_vertex_normals__6aff10(packed: &[[i8; 3]; 145]) -> [[f32; 3]; 145] {
    // Stock `.rdata` reciprocal at 0x810c08 (closest f32 to 1.0/127.0); multiply,
    // never divide, to match the original's `FMUL dword`.
    const RECIP_127: f32 = f32::from_bits(0x3c01_0204);
    core::array::from_fn(|i| {
        let n = packed[i];
        [
            f32::from(n[0]) * RECIP_127,
            f32::from(n[1]) * RECIP_127,
            f32::from(n[2]) * RECIP_127,
        ]
    })
}

#[cfg(test)]
mod tests_c_map_chunk__decompress_vertex_normals__6aff10 {
    use super::c_map_chunk__decompress_vertex_normals__6aff10 as decode;

    const RECIP_127: f32 = f32::from_bits(0x3c01_0204);

    #[test]
    fn recip_constant_is_one_over_127() {
        // The stock .rdata dword at 0x810c08 is the closest f32 to 1/127.
        assert_eq!(RECIP_127.to_bits(), (1.0f32 / 127.0f32).to_bits());
    }

    #[test]
    fn boundary_samples_are_bit_exact() {
        let mut packed = [[0i8; 3]; 145];
        packed[0] = [-128, -127, -1];
        packed[1] = [0, 1, 63];
        packed[144] = [127, -64, 64];
        let out = decode(&packed);
        let cases = [
            (-128i8, out[0][0]),
            (-127, out[0][1]),
            (-1, out[0][2]),
            (0, out[1][0]),
            (1, out[1][1]),
            (63, out[1][2]),
            (127, out[144][0]),
            (-64, out[144][1]),
            (64, out[144][2]),
        ];
        for (b, f) in cases {
            assert_eq!(
                f.to_bits(),
                (f32::from(b) * RECIP_127).to_bits(),
                "byte {b}"
            );
        }
    }

    #[test]
    fn signed_widening_not_unsigned() {
        // -1 must decode negative (movsx), not as a 255/127 unsigned value.
        let mut packed = [[0i8; 3]; 145];
        packed[0] = [-1, 0, 0];
        let out = decode(&packed);
        assert!(out[0][0] < 0.0);
        assert_eq!(out[0][0].to_bits(), (-RECIP_127).to_bits());
    }

    #[test]
    fn component_order_straight_012() {
        let mut packed = [[0i8; 3]; 145];
        packed[5] = [10, 20, 30];
        let out = decode(&packed);
        assert_eq!(out[5][0].to_bits(), (10.0f32 * RECIP_127).to_bits());
        assert_eq!(out[5][1].to_bits(), (20.0f32 * RECIP_127).to_bits());
        assert_eq!(out[5][2].to_bits(), (30.0f32 * RECIP_127).to_bits());
    }
}

/// `CWorldBsp::TraverseNode` interior-node split-axis overlap.
///
/// Does the box being traversed (`arg2`) overlap the node's split/clip box
/// (`arg1`) on the split axis?
///
/// Decoded from the two interior compares (`0x6bc23d..0x6bc25e`):
/// * site1 `fld arg1[axis+3]; fcomp arg2[axis]; test ah,0x5; jnp REJECT` — reject
///   **only** when `arg1[axis+3] < arg2[axis]` strictly; `>`, `==`, unordered continue.
/// * site2 `fld arg1[axis]; fcomp arg2[axis+3]; test ah,0x41; je REJECT` — reject
///   **only** when `arg1[axis] > arg2[axis+3]` strictly; `<`, `==`, unordered continue.
///
/// So the node is entered iff `!(arg1[axis+3] < arg2[axis]) && !(arg1[axis] >
/// arg2[axis+3])` — STRICT separation with **unordered-CONTINUE**, deliberately
/// NOT the inclusive `<=`/unordered-reject idiom of `aabb__min_less_than_max__6acb40`
/// (reusing that would invert both boundary and NaN routing — the crash class).
// Both compares are negated so an unordered operand continues rather than
// rejects, matching the strict `jnp` / `je` routing the doc above decodes.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn c_world_bsp__node_overlaps_box__6bc1c0_leaf(
    arg1: &[f32; 6],
    arg2: &[f32; 6],
    axis: usize,
) -> bool {
    // `!(strict)` keeps NaN on the continue side, matching the JNP/JE routing.
    !(arg1[axis + 3] < arg2[axis]) && !(arg1[axis] > arg2[axis + 3])
}

/// Which child(ren) `CWorldBsp::TraverseNode` descends.
///
/// From the child-side compares (`0x6bc28c..0x6bc2d0`), both against
/// `arg1[axis]`/`arg1[axis+3]` vs the node split plane:
/// * site3 `fld arg1[axis]; fcomp split; test ah,0x41; jne` — greater-side-only
///   **only** when `arg1[axis] > split` strictly (`<`, `==`, unordered → onward).
/// * site4 `fld arg1[axis+3]; fcomp split; test ah,0x5; jp` — both-children when
///   `arg1[axis+3] > split`, `==`, **or unordered**; less-side-only only when
///   `arg1[axis+3] < split` strictly.
///
/// Child offsets are offset-mapped: greater-side child `[node+0x4]`, less-side
/// child `[node+0x2]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BspChildSide {
    /// `arg1[axis] > split` strictly — descend only the greater-side child.
    GreaterOnly,
    /// `arg1[axis+3] < split` strictly — descend only the less-side child.
    LessOnly,
    /// The box (or a NaN extent) spans the plane — descend both, greater first.
    Both,
}

/// Pure child-descent selection for `CWorldBsp::TraverseNode` (`0x6bc28c..0x6bc2d0`).
///
/// `min` = `arg1[axis]`, `max` = `arg1[axis+3]`, `split` = `[node+0xc]`. Two strict
/// compares with unordered routed into `Both` (site4's `jp` sends NaN to Both).
pub fn c_world_bsp__child_side__6bc1c0(min: f32, max: f32, split: f32) -> BspChildSide {
    if min > split {
        // site3 fall-through: strict greater, NaN does NOT land here.
        BspChildSide::GreaterOnly
    } else if max < split {
        // site4 fall-through: strict less only; `==`/unordered already excluded.
        BspChildSide::LessOnly
    } else {
        // site4 `jp`: `max >= split` or unordered.
        BspChildSide::Both
    }
}

#[cfg(test)]
mod tests_c_world_bsp__node_overlaps_box__6bc1c0_leaf {
    use super::c_world_bsp__node_overlaps_box__6bc1c0_leaf as overlaps;

    #[test]
    fn separated_below_rejects() {
        let arg1 = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        let arg2 = [2.0, 2.0, 2.0, 3.0, 3.0, 3.0];
        assert!(!overlaps(&arg1, &arg2, 0));
    }

    #[test]
    fn separated_above_rejects() {
        let arg1 = [5.0, 0.0, 0.0, 6.0, 1.0, 1.0];
        let arg2 = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        assert!(!overlaps(&arg1, &arg2, 0));
    }

    #[test]
    fn touching_faces_overlap_inclusive() {
        // arg1.max == arg2.min : site1 `<` is FALSE => continue.
        let arg1 = [0.0, 0.0, 0.0, 1.0, 9.0, 9.0];
        let arg2 = [1.0, 0.0, 0.0, 2.0, 9.0, 9.0];
        assert!(overlaps(&arg1, &arg2, 0));
        let arg3 = [2.0, 0.0, 0.0, 3.0, 9.0, 9.0];
        let arg4 = [1.0, 0.0, 0.0, 2.0, 9.0, 9.0];
        assert!(overlaps(&arg3, &arg4, 0));
    }

    #[test]
    fn nan_continues_not_rejects() {
        let nan = f32::NAN;
        let arg1 = [nan, 0.0, 0.0, nan, 1.0, 1.0];
        let arg2 = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        assert!(overlaps(&arg1, &arg2, 0));
        let arg3 = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        let arg4 = [nan, 0.0, 0.0, nan, 1.0, 1.0];
        assert!(overlaps(&arg3, &arg4, 0));
    }

    #[test]
    fn axis_selects_only_split_axis() {
        let arg1 = [0.0, 100.0, 0.0, 1.0, 101.0, 1.0];
        let arg2 = [0.0, -100.0, 0.0, 1.0, -99.0, 1.0];
        assert!(overlaps(&arg1, &arg2, 0));
        assert!(!overlaps(&arg1, &arg2, 1));
    }
}

#[cfg(test)]
mod tests_c_world_bsp__child_side__6bc1c0 {
    use super::{BspChildSide, c_world_bsp__child_side__6bc1c0 as side};

    #[test]
    fn strictly_above_split_is_greater_only() {
        assert_eq!(side(2.0, 5.0, 1.0), BspChildSide::GreaterOnly);
    }

    #[test]
    fn strictly_below_split_is_less_only() {
        assert_eq!(side(-5.0, -2.0, 0.0), BspChildSide::LessOnly);
    }

    #[test]
    fn spanning_split_is_both() {
        assert_eq!(side(-1.0, 1.0, 0.0), BspChildSide::Both);
    }

    #[test]
    fn min_equal_split_is_both_not_greater() {
        // site3 is STRICT `>`; equality on min falls through, max >= split => Both.
        assert_eq!(side(1.0, 5.0, 1.0), BspChildSide::Both);
    }

    #[test]
    fn max_equal_split_is_both_not_less() {
        // site4 `jp` includes equality => Both, not LessOnly.
        assert_eq!(side(-5.0, 1.0, 1.0), BspChildSide::Both);
    }

    #[test]
    fn nan_routes_to_both() {
        assert_eq!(side(f32::NAN, f32::NAN, 0.0), BspChildSide::Both);
        assert_eq!(side(-1.0, f32::NAN, 0.0), BspChildSide::Both);
    }
}

/// `CWorld::LinkEntityToTiles` index range.
///
/// Quantize an entity's world-space XY AABB into the inclusive tile-index loop
/// bounds the stock driver iterates.
///
/// Stock re-anchors each AABB float against the map origin (`d = -(coord -
/// 17066.666)`), scales by the cell size (`0.03`), biases by `0.5`, then x87
/// `FISTP` — i.e. `round_ties_even(0.03 * d - 0.5)` (the `-0.5` is an integer bias
/// baked into round-to-nearest-even, NOT a floor). Field map (0x6a8cbf-0x6a8d53):
/// `min_x@0x50 -> ix_lo`, `max_x@0x44 -> ix_hi` (row), `min_y@0x54 -> iy_lo`,
/// `max_y@0x48 -> iy_hi` (col). Returns `(ix_lo, ix_hi, iy_lo, iy_hi)`. Pure leaf.
pub fn link_entity_tile_index_range(
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
    origin: f32,
    scale: f32,
    bias: f32,
) -> (i32, i32, i32, i32) {
    // `round_ties_even(scale*-(coord-origin) - bias)`; reuse the module FISTP
    // round — naive `as i32` truncation diverges on .5 boundaries.
    let q = |coord: f32| round_i32(scale * -(coord - origin) - bias);
    (q(min_x), q(max_x), q(min_y), q(max_y))
}

/// `CWorld::LinkEntityToTiles` per-tile grid/sub indices.
///
/// Decompose `(ix, iy)` into the flat 64x64 row-table index and the 16x16
/// sub-cell index, matching the stock `[4*grid + 0xc96318]` row lookup and
/// `[row + 4*sub + 0x278]` tile lookup. Same idiom as
/// `c_world__point_in_liquid_grid__69b350` with `x = ix`, `y = iy`.
/// The `>> 4` is arithmetic (`ix`/`iy` may be negative) — `i32 >> 4` (SAR).
pub fn link_entity_tile_indices(ix: i32, iy: i32) -> (usize, usize) {
    let grid_index = (((ix >> 4) & 0x3f) * 0x40 + ((iy >> 4) & 0x3f)) as usize;
    let sub_index = ((ix & 0xf) * 0x10 + (iy & 0xf)) as usize;
    (grid_index, sub_index)
}

/// `CWorld::LinkEntityToTiles` z-clip predicate.
///
/// Link proceeds onto a tile when the entity top-Z is not strictly below the
/// tile's threshold.
///
/// Stock compare at 0x6a8dc1: `fld [ecx+0x58]` (ztop); `fcomp [edi+0x4c]`
/// (thresh); `fnstsw ax`; `test ah,0x5`; `jnp SKIP` — masked C0(0x1)+C2(0x4),
/// popcount parity: GT proceeds, LT skips, EQ proceeds, NaN/unordered proceeds.
/// Net `proceed = !(ztop < thresh)`, keeping NaN on the proceed side (inverting
/// this mask was the root of three prior in-world crashes).
// `!(ztop < thresh)` keeps NaN on the proceed side, which `ztop >= thresh`
// does not; that polarity is load-bearing here.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn link_entity_ztop_passes_tile(ztop: f32, thresh: f32) -> bool {
    !(ztop < thresh)
}

#[cfg(test)]
mod tests_link_entity_to_tiles__6a8ca0 {
    use super::{
        link_entity_tile_index_range, link_entity_tile_indices, link_entity_ztop_passes_tile,
        round_i32,
    };

    const ORIGIN: f32 = f32::from_bits(0x4685_5555);
    const SCALE: f32 = f32::from_bits(0x3cf5_c28f);
    const BIAS: f32 = f32::from_bits(0x3f00_0000);

    #[test]
    fn range_matches_round_ties_even_per_field() {
        let (min_x, max_x, min_y, max_y) = (16000.0f32, 16500.0f32, 15000.0f32, 15123.0f32);
        let (ix_lo, ix_hi, iy_lo, iy_hi) =
            link_entity_tile_index_range(min_x, max_x, min_y, max_y, ORIGIN, SCALE, BIAS);
        let q = |c: f32| round_i32(SCALE * -(c - ORIGIN) - BIAS);
        assert_eq!(ix_lo, q(min_x));
        assert_eq!(ix_hi, q(max_x));
        assert_eq!(iy_lo, q(min_y));
        assert_eq!(iy_hi, q(max_y));
    }

    #[test]
    fn range_is_round_to_even_not_truncation() {
        // scale=1, bias=0, origin=0 -> q(coord) = round_ties_even(-coord).
        let (a, _, _, _) = link_entity_tile_index_range(-2.5, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0);
        assert_eq!(a, 2); // round_ties_even(2.5) = 2
        let (b, _, _, _) = link_entity_tile_index_range(-3.5, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0);
        assert_eq!(b, 4); // round_ties_even(3.5) = 4; truncation would give 3
    }

    #[test]
    fn indices_decompose_like_liquid_grid() {
        let (grid, sub) = link_entity_tile_indices(0x123, 0x4ab);
        assert_eq!(
            grid,
            (((0x123i32 >> 4) & 0x3f) * 0x40 + ((0x4abi32 >> 4) & 0x3f)) as usize
        );
        assert_eq!(sub, ((0x123i32 & 0xf) * 0x10 + (0x4abi32 & 0xf)) as usize);
        assert!(grid < 0x40 * 0x40);
        assert!(sub < 0x10 * 0x10);
    }

    #[test]
    fn indices_use_arithmetic_shift_on_negative() {
        // ix=iy=-1: arithmetic -1>>4 = -1 (SAR); &0x3f = 0x3f, &0xf = 0xf.
        // A logical (unsigned) shift would inject zeros and give a different index.
        let (grid, sub) = link_entity_tile_indices(-1, -1);
        assert_eq!(grid, 0x3f * 0x40 + 0x3f); // 4095
        assert_eq!(sub, 0xf * 0x10 + 0xf); // 255
        // black_box keeps the shift out of const-folding so the SAR is real.
        let neg = core::hint::black_box(-1i32);
        assert_eq!((neg >> 4) & 0x3f, 0x3f);
    }

    #[test]
    fn ztop_predicate_polarity_and_nan() {
        assert!(link_entity_ztop_passes_tile(5.0, 4.0)); // GT proceeds
        assert!(link_entity_ztop_passes_tile(4.0, 4.0)); // EQ proceeds
        assert!(!link_entity_ztop_passes_tile(3.0, 4.0)); // LT skips
        assert!(link_entity_ztop_passes_tile(f32::NAN, 4.0)); // unordered proceeds
        assert!(link_entity_ztop_passes_tile(4.0, f32::NAN)); // unordered proceeds
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LiquidQueryGrid {
    pub grid_index: usize,
    pub sub_index: usize,
    /// In-chunk X vertex coordinate (`ix & 7`); strided face-vertex base.
    pub cx: u32,
    /// In-chunk Y vertex coordinate (`iy & 7`); strided face-vertex base.
    pub cy: u32,
    /// In-cell X weight `scaled_x - floor`, recovered via `FILD;FSUBR`.
    pub fx: f32,
    /// In-cell Y weight `scaled_y - floor`, recovered via `FILD;FSUBR`.
    pub fy: f32,
}

/// Maps a world-space XY into the `QueryLiquidAtPoint` (0x69b6d0) liquid grid.
///
/// `origin` (17066.666, `.data` 0x7ffab4), `scale` (0.24, `.data` 0x81073c) and
/// `bias` (0.5, `.data` 0x86a750) are the stock affine constants. For each axis:
/// `scaled = -(coord - origin) * scale`; `idx = round_ties_even(scaled - bias)`
/// (real `FISTP`, NOT a SHR magic-bias); the fractional weight is the original's
/// `FILD idx; FSUBR scaled` = `scaled - idx`. Returns the chunk index
/// (`idx>>7 & 0x3f` packed `x<<6 | y`), the sub index (`idx>>3 & 0xf` packed
/// `x<<4 | y`), the in-chunk vertex coords (`idx & 7`) and the two weights. The
/// caller performs the live grid/chunk dereferences and bounds via null checks
/// (this map has NO numeric lo/hi band — out-of-range falls out as a null grid
/// slot in the live image).
pub fn c_world__query_liquid_grid_map__69b6d0(
    px: f32,
    py: f32,
    origin: f32,
    scale: f32,
    bias: f32,
) -> LiquidQueryGrid {
    // y feeds `ecx`/`[ebp+0xc]` (the "Y" lane), x feeds `eax`/`[ebp-0x8]`.
    let scaled_y = -(py - origin) * scale;
    let scaled_x = -(px - origin) * scale;

    // round-to-nearest-even (FISTP under default rounding), then the integer.
    let iy = round_i32(scaled_y - bias);
    let ix = round_i32(scaled_x - bias);

    // Chunk index: (ix>>7 & 0x3f) << 6 | (iy>>7 & 0x3f). (0x69b74e..0x69b763)
    let grid_index = ((((ix >> 7) & 0x3f) << 6) + ((iy >> 7) & 0x3f)) as usize;
    // Sub index: (ix>>3 & 0xf) << 4 | (iy>>3 & 0xf). (0x69b772..0x69b787)
    let sub_index = ((((ix >> 3) & 0xf) << 4) + ((iy >> 3) & 0xf)) as usize;

    // In-chunk vertex coords: low 3 bits. (`and ecx,7` / `and eax,7` at 0x69b799)
    let cx = (ix & 7) as u32;
    let cy = (iy & 7) as u32;

    // In-cell weights: scaled - round(scaled-bias) via FILD;FSUBR. (0x69b796 / 0x69b7ba)
    let fy = scaled_y - iy as f32;
    let fx = scaled_x - ix as f32;

    LiquidQueryGrid {
        grid_index,
        sub_index,
        cx,
        cy,
        fx,
        fy,
    }
}

/// Squared Euclidean distance between two vec3, at `0x69be70`.
///
/// Stock body (0x69be70): `(dx^2 + dz^2) + dy^2` to ST0 with `ret 4`; ECX =
/// `&a`, the single stack arg = `&b` (fastcall-with-one-reg-arg, NOT a `this`
/// object). Inlining this kernel removes one of SP's indirect FKI call-outs.
///
/// Nothing here reaches memory: the three deltas live in `ST(2)`/`ST(1)`/`ST(0)`
/// and the accumulator is only ever touched by `FADDP`, so the result leaves in
/// `ST(0)` un-narrowed and every caller compares it against a squared radius
/// built the same way. Hence `f64` out and no rounding anywhere inside. Note the
/// summation pairs x with z before adding y; the deltas themselves are exact in
/// `f64`, so only the squares and adds carry the difference.
#[inline]
pub fn sq_dist__69be70(a: &[f32; 3], b: &[f32; 3]) -> f64 {
    let dx = f64::from(a[0]) - f64::from(b[0]);
    let dy = f64::from(a[1]) - f64::from(b[1]);
    let dz = f64::from(a[2]) - f64::from(b[2]);
    (dx * dx + dz * dz) + dy * dy
}

#[cfg(test)]
mod tests_c_world__query_liquid_grid_map__69b6d0 {
    use super::{c_world__query_liquid_grid_map__69b6d0 as map, sq_dist__69be70};

    // Stock constants (verified against WoW.exe .data dwords).
    const ORIGIN: f32 = f32::from_bits(0x4685_5555); // 0x7ffab4 = 17066.666
    const SCALE: f32 = f32::from_bits(0x3e75_c28f); //  0x81073c = 0.24
    const BIAS: f32 = f32::from_bits(0x3f00_0000); //   0x86a750 = 0.5

    #[test]
    fn at_origin_indices_are_zero_band() {
        // coord == origin -> scaled == 0 -> idx = round(-0.5) = 0 (ties-even).
        let g = map(ORIGIN, ORIGIN, ORIGIN, SCALE, BIAS);
        assert_eq!(g.grid_index, 0);
        assert_eq!(g.sub_index, 0);
        assert_eq!(g.cx, 0);
        assert_eq!(g.cy, 0);
    }

    #[test]
    fn decomposition_matches_shift_layout() {
        // Drive a known integer pair with origin=0, scale=-1, bias=0 so
        // scaled = -(coord-0)*(-1) = coord and idx = round_ties_even(coord).
        let px = 200.0f32;
        let py = 73.0f32;
        let g = map(px, py, 0.0, -1.0, 0.0);
        let ix = 200i32;
        let iy = 73i32;
        let want_grid = ((((ix >> 7) & 0x3f) << 6) + ((iy >> 7) & 0x3f)) as usize;
        let want_sub = ((((ix >> 3) & 0xf) << 4) + ((iy >> 3) & 0xf)) as usize;
        assert_eq!(g.grid_index, want_grid);
        assert_eq!(g.sub_index, want_sub);
        assert_eq!(g.cx, (ix & 7) as u32);
        assert_eq!(g.cy, (iy & 7) as u32);
    }

    #[test]
    fn fractional_weights_are_scaled_minus_floor() {
        // origin=0, scale=-1, bias=0 -> scaled=coord, idx=round(coord).
        // coord 10.25 -> idx 10 -> fx = 0.25 exactly.
        let g = map(10.25, 4.75, 0.0, -1.0, 0.0);
        assert_eq!(g.fx.to_bits(), 0.25f32.to_bits());
        // 4.75 -> round_ties_even(4.75)=5 -> fy = 4.75 - 5 = -0.25.
        assert_eq!(g.fy.to_bits(), (-0.25f32).to_bits());
    }

    #[test]
    fn weights_use_round_ties_even_not_trunc() {
        // 2.5 ties to 2 (even); fx = 2.5 - 2 = 0.5, never 0.5-3 (trunc-up).
        let g = map(2.5, 0.0, 0.0, -1.0, 0.0);
        assert_eq!(g.fx.to_bits(), 0.5f32.to_bits());
    }

    #[test]
    fn sqdist_is_sum_of_squared_diffs() {
        let a = [1.0f32, 2.0, 3.0];
        let b = [4.0f32, 6.0, 3.0];
        // (1-4)^2 + (2-6)^2 + 0 = 9 + 16 = 25.
        assert_eq!(sq_dist__69be70(&a, &b).to_bits(), 25.0f64.to_bits());
        // Identical points -> 0.
        assert_eq!(sq_dist__69be70(&a, &a).to_bits(), 0.0f64.to_bits());
    }

    /// Law: x pairs with z and y is added last, at register precision.
    ///
    /// The stock body squares and sums entirely on the x87 stack — the deltas
    /// never reach memory and the accumulator is only touched by `FADDP` — so
    /// both the grouping and the width are load-bearing. The `assert_ne` keeps
    /// the fixture honest: it fails if these inputs stop separating this from
    /// the left-to-right f32 form the kernel used to have.
    #[test]
    fn sqdist_grouping_matches_stock_fadd_order() {
        let a = [0.1f32, 0.2, 0.3];
        let b = [0.4f32, 0.5, 0.9];
        let dx = f64::from(a[0]) - f64::from(b[0]);
        let dy = f64::from(a[1]) - f64::from(b[1]);
        let dz = f64::from(a[2]) - f64::from(b[2]);
        let want = (dx * dx + dz * dz) + dy * dy;
        assert_eq!(sq_dist__69be70(&a, &b).to_bits(), want.to_bits());

        let (fx, fy, fz) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
        let per_step = fx * fx + fy * fy + fz * fz;
        assert_ne!(f64::from(per_step).to_bits(), want.to_bits());
    }
}

/// `World::QueryObjectBoxes` entry box construction.
///
/// Given the queried object's local AABB (`min = local[0..3]`,
/// `max = local[3..6]`), returns both the box center `(min + max) * 0.5` and a
/// copy of the local AABB recentered about the origin (`min - center`,
/// `max - center`).
///
/// Pure prologue of the stock driver at `0x6ad330`: `C3Vector::Add`(min + max)
/// then `C3Vector::Scale`(0.5) (the `push 0x3f000000` = `0.5f` at `0x6ad364`),
/// three `fchs` sign flips on the center, and two `C3Vector::AddInPlace` of the
/// negated center into a `rep movsd ecx=6` copy of the local box. The add/scale
/// group exactly as the stock x87 schedule, and `fchs` is a sign-bit flip
/// (bit-exact to Rust `-x` for non-NaN; preserves the `-0.0` of a zero
/// component), so the result is bit-exact. Returns `(center, recentered_box)`.
pub fn world__query_object_boxes_recenter__6ad330(local: &[f32; 6]) -> ([f32; 3], [f32; 6]) {
    let min = [local[0], local[1], local[2]];
    let max = [local[3], local[4], local[5]];
    // center = (min + max) * 0.5 — matches `Add` then `Scale(0.5f)`.
    let sum = crate::math::vector::c3_vector__add__621160(&min, &max);
    let center = crate::math::vector::c3_vector__scale__5f8cf0(&sum, f32::from_bits(0x3f00_0000));
    // Negate the center (3x `fchs`), then add into a copy of each box corner.
    let neg = [-center[0], -center[1], -center[2]];
    let new_min = crate::math::vector::c3_vector__add_in_place__4841c0(&min, &neg);
    let new_max = crate::math::vector::c3_vector__add_in_place__4841c0(&max, &neg);
    (
        center,
        [
            new_min[0], new_min[1], new_min[2], new_max[0], new_max[1], new_max[2],
        ],
    )
}

#[cfg(test)]
mod tests_world__query_object_boxes_recenter__6ad330 {
    use super::world__query_object_boxes_recenter__6ad330 as recenter;

    // Straight transcription of the stock prologue, to guard against lane swaps
    // or grouping drift in the kernel.
    fn reference(local: &[f32; 6]) -> ([f32; 3], [f32; 6]) {
        let half = f32::from_bits(0x3f00_0000); // 0.5f
        let cx = (local[0] + local[3]) * half;
        let cy = (local[1] + local[4]) * half;
        let cz = (local[2] + local[5]) * half;
        (
            [cx, cy, cz],
            [
                local[0] + -cx,
                local[1] + -cy,
                local[2] + -cz,
                local[3] + -cx,
                local[4] + -cy,
                local[5] + -cz,
            ],
        )
    }

    #[test]
    fn center_is_box_midpoint() {
        let local = [-2.0, -4.0, -6.0, 4.0, 8.0, 12.0];
        let (center, _) = recenter(&local);
        assert_eq!(center[0].to_bits(), 1.0f32.to_bits());
        assert_eq!(center[1].to_bits(), 2.0f32.to_bits());
        assert_eq!(center[2].to_bits(), 3.0f32.to_bits());
    }

    #[test]
    fn recentered_box_is_symmetric_about_origin() {
        let local = [-2.0, -4.0, -6.0, 4.0, 8.0, 12.0];
        let (_, b) = recenter(&local);
        // min - center = -3,-6,-9 ; max - center = 3,6,9
        let want = [-3.0f32, -6.0, -9.0, 3.0, 6.0, 9.0];
        for (g, w) in b.iter().zip(want.iter()) {
            assert_eq!(g.to_bits(), w.to_bits());
        }
    }

    #[test]
    fn bit_exact_against_reference() {
        let cases = [
            [0.0f32, 0.0, 0.0, 0.0, 0.0, 0.0],
            [1.5, -2.25, 3.125, 7.0, -0.5, 9.75],
            [-100.0, 50.0, 0.0, 100.0, -50.0, 1.0],
            [
                f32::from_bits(0x3f8c_cccd),
                -0.0,
                12345.678,
                0.001,
                0.002,
                -9999.9,
            ],
        ];
        for local in cases {
            let (gc, gb) = recenter(&local);
            let (rc, rb) = reference(&local);
            for k in 0..3 {
                assert_eq!(
                    gc[k].to_bits(),
                    rc[k].to_bits(),
                    "center lane {k} for {local:?}"
                );
            }
            for k in 0..6 {
                assert_eq!(
                    gb[k].to_bits(),
                    rb[k].to_bits(),
                    "box lane {k} for {local:?}"
                );
            }
        }
    }

    #[test]
    fn negate_preserves_zero_sign_of_center() {
        // A zero-center component negates to -0.0, exactly like `fchs`.
        let local = [-1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        let (center, b) = recenter(&local);
        assert_eq!(center[0].to_bits(), 0.0f32.to_bits());
        // min.x - center.x = -1 - 0 = -1
        assert_eq!(b[0].to_bits(), (-1.0f32).to_bits());
        assert_eq!(b[3].to_bits(), 1.0f32.to_bits());
    }
}

/// Translates a query AABB into chunk-local space.
///
/// Subtracts the chunk origin from both the min (`aabb[0..3]`) and max
/// (`aabb[3..6]`) corners.
///
/// Mirrors `CollectTileGeometry`'s entry block (0x6aae9d..0x6aaed7): the stock
/// builds `neg = -origin`, adds it into the min corner with three inline
/// `fld;fadd;fstp` (single rounding each), then calls `C3Vector::AddInPlace`
/// on the max corner. The two halves are therefore identical component adds.
/// `origin` is `[*(cell+0x6c), *(cell+0x70), *(cell+0x74)]`.
#[must_use]
pub fn tile_collect_translate_aabb__6aadc0(aabb: &[f32; 6], origin: &[f32; 3]) -> [f32; 6] {
    let neg = [-origin[0], -origin[1], -origin[2]];
    // Min corner: three inline fadds (0x6aaebc..0x6aaed4).
    let lo0 = aabb[0] + neg[0];
    let lo1 = aabb[1] + neg[1];
    let lo2 = aabb[2] + neg[2];
    // Max corner: the inlined C3Vector::AddInPlace (0x4841c0), called by name so
    // the host-tested kernel proves bit-identical grouping.
    let hi =
        crate::math::vector::c3_vector__add_in_place__4841c0(&[aabb[3], aabb[4], aabb[5]], &neg);
    [lo0, lo1, lo2, hi[0], hi[1], hi[2]]
}

/// The `.rdata` clip bias added before the bit-reinterpret floor in the tile outcode.
///
/// Static 0x4101b8 == `0.019444443`. The production adapter reads this value
/// LIVE from `.rdata` and passes it to `tile_collect_outcode6__6aadc0`; this
/// pinned copy exists only so the host tests can fix the bit-trick against the
/// exact source dword, so it is gated to test builds.
// The `__6aadc0` VA suffix carries lowercase hex (`a`/`d`/`c`); the snake-suffix
// convention applies to every helper/const in this shared module, so allow the
// otherwise all-uppercase requirement here.
#[cfg(test)]
#[allow(non_upper_case_globals)]
const TILE_OUTCODE_BIAS__6aadc0: f32 = f32::from_bits(0x3c9f_49f4);

/// Computes the 6-bit clip outcode for one grid vertex against the chunk-local query AABB.
///
/// The query AABB is `tile_collect_translate_aabb__6aadc0`'s output.
///
/// Mirrors the inner ladder at 0x6aaf60..0x6ab00e. Each of the six plane tests
/// forms a biased difference, reinterprets its IEEE-754 bits, and extracts the
/// sign bit via a right shift — the proven magic-bias floor trick, NOT an
/// `as u32` cast. The bit is set iff the biased difference is negative (sign
/// bit == 1), i.e. the vertex is outside that plane. Bit layout (matching the
/// stock `and`/`or` masks):
/// - `bit 0`: `(v.x - aabb.min_x) + bias < 0`  (shr 0x1f)
/// - `bit 1`: `(v.y - aabb.min_y) + bias < 0`  (shr 0x1e, mask 2)
/// - `bit 2`: `(v.z - aabb.min_z) + bias < 0`  (shr 0x1d, mask 4)
/// - `bit 3`: `(aabb.max_x - v.x) + bias < 0`  (shr 0x1c, mask 8)
/// - `bit 4`: `(aabb.max_y - v.y) + bias < 0`  (shr 0x1b, mask 0x10)
/// - `bit 5`: `(aabb.max_z - v.z) + bias < 0`  (shr 0x1a, mask 0x20)
///
/// `aabb` is the 6-float `[min_xyz, max_xyz]` head; `v` is the vertex.
#[must_use]
pub fn tile_collect_outcode6__6aadc0(v: &[f32; 3], aabb: &[f32; 6], bias: f32) -> u32 {
    // Each line: bit = to_bits((diff) + bias) >> shift, then masked. Because the
    // shift isolates the sign bit, the high bits land at the mask position
    // exactly (the original shifts each independently then ANDs the lone bit).
    let mut code = ((v[0] - aabb[0]) + bias).to_bits() >> 0x1f;
    code |= (((v[1] - aabb[1]) + bias).to_bits() >> 0x1e) & 2;
    code |= (((v[2] - aabb[2]) + bias).to_bits() >> 0x1d) & 4;
    code |= (((aabb[3] - v[0]) + bias).to_bits() >> 0x1c) & 8;
    code |= (((aabb[4] - v[1]) + bias).to_bits() >> 0x1b) & 0x10;
    code |= (((aabb[5] - v[2]) + bias).to_bits() >> 0x1a) & 0x20;
    code
}

/// Translates one grid vertex into world space by adding the chunk origin.
///
/// Mirrors the three `fld vert; fadd [cell+0x6c/0x70/0x74]; fstp` adds emitted
/// per triangle vertex (e.g. 0x6ab0a7, 0x6ab129, 0x6ab1da). Trivial component
/// add; `origin` is `[*(cell+0x6c), *(cell+0x70), *(cell+0x74)]`.
#[must_use]
pub fn tile_collect_vertex_to_world__6aadc0(v: &[f32; 3], origin: &[f32; 3]) -> [f32; 3] {
    [v[0] + origin[0], v[1] + origin[1], v[2] + origin[2]]
}

/// Builds the face plane `[nx, ny, nz, d]` for one emitted triangle.
///
/// Uses the raw SSE `rsqrtss` approximation (the fast path for a nonzero flag
/// at `0xc9e388`, 0x6ab2bb..0x6ab375), which has NO Newton-Raphson refinement.
///
/// `n = cross(v1 - v0, v2 - v0)` with the stock fadd/fsub grouping; the length
/// is `len2 = (nx*nx + ny*ny) + nz*nz`; `inv = rsqrtss(len2)`; the normal is
/// scaled in place by `inv`; `d = -((nz*v0.z + ny*v0.y) + nx*v0.x)` using the
/// normalized components and the stock right-to-left accumulation order.
///
/// This is kept SEPARATE from `c4_plane__from_triangle__637480` (which uses a
/// full `1.0/sqrt`) so neither precision regresses: the stock takes the
/// `0x637480` path only when that flag at `0xc9e388` is zero. `rsqrtss` is
/// replicated via `_mm_rsqrt_ss`; `libm`'s reciprocal sqrt would NOT bit-match.
///
/// The same arm is emitted again inside `0x671cc0`, instruction for instruction
/// across all sixty-six of its floating-point and SSE ops once the operand
/// addresses are masked, so `build_mesh_triangle_planes__671cc0` calls this rather
/// than carrying its own copy.
///
/// Nothing narrows except where the original stores. `ea` stays on the x87 stack
/// whole; `eb`'s `x` and `y` are spilled to `f32` slots at `0x6ab2d3` / `0x6ab2dc`
/// while its `z` stays wide; each cross lane is formed wide and narrowed at its
/// own store into the plane record; `len2` is summed wide from the three narrowed
/// lanes and narrowed only because `rsqrtss` takes a memory operand. The three
/// scaled lanes use **`FST`**, not `FSTP` (`0x6ab34f`, `0x6ab357`, `0x6ab360`), so
/// the normal is narrowed but `d` is built from the unnarrowed products — the same
/// asymmetry `c4_plane__from_triangle__637480` has.
#[must_use]
pub fn tile_collect_triangle_plane__6aadc0(
    v0: &[f32; 3],
    v1: &[f32; 3],
    v2: &[f32; 3],
) -> [f32; 4] {
    let sub = |a: f32, b: f32| f64::from(a) - f64::from(b);

    // ea = v2 - v0 (stays on the x87 stack); eb = v1 - v0 (x and y spilled to f32,
    // z left live).
    let ea = [sub(v2[0], v0[0]), sub(v2[1], v0[1]), sub(v2[2], v0[2])];
    let eb_x = f64::from(super::f64_to_f32(sub(v1[0], v0[0])));
    let eb_y = f64::from(super::f64_to_f32(sub(v1[1], v0[1])));
    let eb_z = sub(v1[2], v0[2]);

    // n = cross(eb, ea): normal.x = eb.y*ea.z - eb.z*ea.y, etc. (0x6ab2e5..0x6ab314).
    let nx = super::f64_to_f32(eb_y * ea[2] - eb_z * ea[1]);
    let ny = super::f64_to_f32(eb_z * ea[0] - ea[2] * eb_x);
    let nz = super::f64_to_f32(ea[1] * eb_x - eb_y * ea[0]);

    // len2 = (nx*nx + ny*ny) + nz*nz, rebuilt wide from the three stored lanes and
    // narrowed at 0x6ab337 because `rsqrtss` reads it back from memory.
    let sq = |v: f32| f64::from(v) * f64::from(v);
    let len2 = super::f64_to_f32((sq(nx) + sq(ny)) + sq(nz));
    let inv = rsqrt_ss__6aadc0(len2);

    // Normalize in place. `FST` keeps each product live, so `d` sees the wide ones;
    // both factors are f32 there, which makes the products exact (0x6ab34a..0x6ab360).
    let nnx = f64::from(inv) * f64::from(nx);
    let nny = f64::from(inv) * f64::from(ny);
    let nnz = f64::from(inv) * f64::from(nz);

    // d = -((nz*v0.z + ny*v0.y) + nx*v0.x)  (right-to-left; 0x6ab363..0x6ab375).
    let d = super::f64_to_f32(
        -((nnz * f64::from(v0[2]) + nny * f64::from(v0[1])) + nnx * f64::from(v0[0])),
    );

    [
        super::f64_to_f32(nnx),
        super::f64_to_f32(nny),
        super::f64_to_f32(nnz),
        d,
    ]
}

/// Single-pass SSE reciprocal square root (`rsqrtss`, 12-bit approximation, no refinement).
///
/// Replicates the stock `rsqrtss xmm1,[mem]` at 0x6ab340 exactly; `libm`
/// reciprocal sqrt would not bit-match. Available on both `x86` and `x86_64`
/// (host tests run `x86_64` under the same translation as the shipped i686 image).
#[inline]
#[must_use]
// The three nested SSE intrinsics form one logically atomic `rsqrtss`; keeping
// them in a single unsafe block mirrors the one stock instruction.
#[allow(clippy::multiple_unsafe_ops_per_block)]
pub fn rsqrt_ss__6aadc0(x: f32) -> f32 {
    #[cfg(target_arch = "x86")]
    use core::arch::x86::{_mm_cvtss_f32, _mm_rsqrt_ss, _mm_set_ss};
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::{_mm_cvtss_f32, _mm_rsqrt_ss, _mm_set_ss};

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        // SAFETY: `_mm_set_ss`/`_mm_rsqrt_ss`/`_mm_cvtss_f32` are SSE1, available on
        // every supported target; they have no memory or aliasing preconditions.
        unsafe { _mm_cvtss_f32(_mm_rsqrt_ss(_mm_set_ss(x))) }
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    {
        // Non-x86 fallback (never the parity target; keeps the crate buildable).
        1.0_f32 / x.sqrt()
    }
}

#[cfg(test)]
mod tests_c_world__collect_tile_geometry__6aadc0 {
    use super::{
        TILE_OUTCODE_BIAS__6aadc0, rsqrt_ss__6aadc0, tile_collect_outcode6__6aadc0,
        tile_collect_translate_aabb__6aadc0, tile_collect_triangle_plane__6aadc0,
        tile_collect_vertex_to_world__6aadc0,
    };

    #[test]
    fn translate_subtracts_origin_from_both_corners() {
        let aabb = [10.0f32, 20.0, 30.0, 40.0, 50.0, 60.0];
        let origin = [1.0f32, 2.0, 3.0];
        let out = tile_collect_translate_aabb__6aadc0(&aabb, &origin);
        // Each component: aabb + (-origin), single rounding (bit-exact). Both
        // corners subtract the SAME -origin, so the max-corner X (out[3]) uses
        // -origin[0] = -1.0, NOT -origin[2].
        assert_eq!(out[0].to_bits(), (10.0f32 + -1.0).to_bits());
        assert_eq!(out[3].to_bits(), (40.0f32 + -1.0).to_bits());
        assert_eq!(out, [9.0, 18.0, 27.0, 39.0, 48.0, 57.0]);
    }

    #[test]
    fn outcode_inside_aabb_is_zero() {
        // A vertex strictly inside the box: every biased diff is positive -> no bits.
        let aabb = [0.0f32, 0.0, 0.0, 10.0, 10.0, 10.0];
        let v = [5.0f32, 5.0, 5.0];
        assert_eq!(
            tile_collect_outcode6__6aadc0(&v, &aabb, TILE_OUTCODE_BIAS__6aadc0),
            0
        );
    }

    #[test]
    fn outcode_sets_low_bits_below_min() {
        // Far below min on each axis -> bits 0,1,2 set; above max stays clear.
        let aabb = [0.0f32, 0.0, 0.0, 10.0, 10.0, 10.0];
        let v = [-100.0f32, -100.0, -100.0];
        let code = tile_collect_outcode6__6aadc0(&v, &aabb, 0.0);
        assert_eq!(code & 0x7, 0x7);
        assert_eq!(code & 0x38, 0);
    }

    #[test]
    fn outcode_sets_high_bits_above_max() {
        let aabb = [0.0f32, 0.0, 0.0, 10.0, 10.0, 10.0];
        let v = [100.0f32, 100.0, 100.0];
        let code = tile_collect_outcode6__6aadc0(&v, &aabb, 0.0);
        assert_eq!(code & 0x38, 0x38);
        assert_eq!(code & 0x7, 0);
    }

    #[test]
    fn outcode_is_sign_bit_of_biased_diff_not_cast() {
        // bias=0; diff exactly 0 -> +0.0 has sign bit 0 (inside). A tiny negative
        // diff flips the sign bit. This is the to_bits()>>31 trick, not `as u32`.
        let aabb = [0.0f32, 0.0, 0.0, 10.0, 10.0, 10.0];
        // v.x = aabb.min_x exactly -> (0)+0 = +0.0 -> bit 0 clear.
        let on_edge = [0.0f32, 5.0, 5.0];
        assert_eq!(tile_collect_outcode6__6aadc0(&on_edge, &aabb, 0.0) & 1, 0);
        // v.x just below min -> negative -> sign bit set.
        let below = [f32::from_bits(0.0f32.to_bits()) - 1.0, 5.0, 5.0];
        assert_eq!(tile_collect_outcode6__6aadc0(&below, &aabb, 0.0) & 1, 1);
    }

    #[test]
    fn outcode_bias_widens_the_inside_band() {
        // With a positive bias, a vertex slightly outside min still reads inside
        // because (diff + bias) stays non-negative.
        let aabb = [0.0f32, 0.0, 0.0, 10.0, 10.0, 10.0];
        let v = [-0.01f32, 5.0, 5.0];
        // bias 0.019444443 > 0.01 -> biased diff positive -> bit clear.
        assert_eq!(
            tile_collect_outcode6__6aadc0(&v, &aabb, TILE_OUTCODE_BIAS__6aadc0) & 1,
            0
        );
        // bias 0 -> negative -> bit set.
        assert_eq!(tile_collect_outcode6__6aadc0(&v, &aabb, 0.0) & 1, 1);
    }

    #[test]
    fn vertex_to_world_adds_origin() {
        let v = [1.0f32, 2.0, 3.0];
        let o = [10.0f32, 20.0, 30.0];
        assert_eq!(
            tile_collect_vertex_to_world__6aadc0(&v, &o),
            [11.0, 22.0, 33.0]
        );
    }

    #[test]
    fn triangle_plane_xy_triangle_normal_is_unit_z() {
        // Right triangle in z=0 plane, CCW: cross(v1-v0, v2-v0) = +z.
        let v0 = [0.0f32, 0.0, 0.0];
        let v1 = [1.0f32, 0.0, 0.0];
        let v2 = [0.0f32, 1.0, 0.0];
        let pl = tile_collect_triangle_plane__6aadc0(&v0, &v1, &v2);
        // rsqrtss(1.0) is ~1.0 (12-bit approx); normal ~ (0,0,1), d ~ 0.
        assert!(pl[2] > 0.99 && pl[2] < 1.01, "nz {}", pl[2]);
        assert!(pl[0].abs() < 1e-3 && pl[1].abs() < 1e-3);
        assert!(pl[3].abs() < 1e-3, "d {}", pl[3]);
    }

    #[test]
    fn triangle_plane_grouping_is_bit_exact() {
        // Full-mantissa vertices given as bit patterns. The small integers this
        // used to use made every intermediate exact, so the narrow and wide forms
        // agreed and the pin held whichever one the kernel had.
        let v0 = [
            f32::from_bits(0x3fc7_1d09),
            f32::from_bits(0xc019_3b7f),
            f32::from_bits(0x4055_e2cd),
        ];
        let v1 = [
            f32::from_bits(0x4093_a51b),
            f32::from_bits(0x3f2d_77e3),
            f32::from_bits(0xbfb1_04a7),
        ];
        let v2 = [
            f32::from_bits(0xc00b_9d55),
            f32::from_bits(0x40a7_31ef),
            f32::from_bits(0x4013_c6b1),
        ];

        // The narrow-per-step form the kernel used to have, kept as the witness.
        let narrow_form = || {
            let ea = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
            let eb = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
            let nx = eb[1] * ea[2] - eb[2] * ea[1];
            let ny = eb[2] * ea[0] - eb[0] * ea[2];
            let nz = eb[0] * ea[1] - eb[1] * ea[0];
            let len2 = (nx * nx + ny * ny) + nz * nz;
            let inv = rsqrt_ss__6aadc0(len2);
            let (nnx, nny, nnz) = (inv * nx, inv * ny, inv * nz);
            let d = -((nnz * v0[2] + nny * v0[1]) + nnx * v0[0]);
            [nnx, nny, nnz, d]
        };

        // Same grouping, but ea wide, eb's x and y narrowed, each cross lane and
        // `len2` narrowed at its own store, and `d` off the unnarrowed products.
        let sub = |a: f32, b: f32| f64::from(a) - f64::from(b);
        let ea = [sub(v2[0], v0[0]), sub(v2[1], v0[1]), sub(v2[2], v0[2])];
        let eb_x = f64::from(super::super::f64_to_f32(sub(v1[0], v0[0])));
        let eb_y = f64::from(super::super::f64_to_f32(sub(v1[1], v0[1])));
        let eb_z = sub(v1[2], v0[2]);
        let nx = super::super::f64_to_f32(eb_y * ea[2] - eb_z * ea[1]);
        let ny = super::super::f64_to_f32(eb_z * ea[0] - ea[2] * eb_x);
        let nz = super::super::f64_to_f32(ea[1] * eb_x - eb_y * ea[0]);
        let sq = |v: f32| f64::from(v) * f64::from(v);
        let inv = f64::from(rsqrt_ss__6aadc0(super::super::f64_to_f32(
            (sq(nx) + sq(ny)) + sq(nz),
        )));
        let (nnx, nny, nnz) = (
            inv * f64::from(nx),
            inv * f64::from(ny),
            inv * f64::from(nz),
        );
        let want = [
            super::super::f64_to_f32(nnx),
            super::super::f64_to_f32(nny),
            super::super::f64_to_f32(nnz),
            super::super::f64_to_f32(
                -((nnz * f64::from(v0[2]) + nny * f64::from(v0[1])) + nnx * f64::from(v0[0])),
            ),
        ];

        let got = tile_collect_triangle_plane__6aadc0(&v0, &v1, &v2);
        for i in 0..4 {
            assert_eq!(got[i].to_bits(), want[i].to_bits(), "lane {i}");
        }
        let narrow = narrow_form();
        assert!(
            (0..4).any(|i| got[i].to_bits() != narrow[i].to_bits()),
            "fixture no longer separates narrow-once from narrow-per-step"
        );
    }

    #[test]
    fn rsqrt_is_raw_approx_not_exact() {
        // rsqrtss(4.0) ~ 0.5 within the 12-bit relative error (< 1.5*2^-12).
        let r = rsqrt_ss__6aadc0(4.0);
        assert!((r - 0.5).abs() < 0.5 * 0.0004, "rsqrt(4)={r}");
        // It is an approximation: exactly 0.5 would imply a refined result.
        let exact = 0.5f32;
        // Allow it to equal exact OR differ slightly; the point is no NR step.
        assert!((r - exact).abs() < 0.001);
    }
}

/// Negates the query-box midpoint for the recentering subtract in `CollectGeometryFromNodes`.
///
/// The three inline `fld;fchs;fstp` at 0x6aaaf5..0x6aab19 feed the
/// `C3Vector::Set` store pack. Pure IEEE-754 sign flips — exact for every input,
/// including NaN payloads and ±0.
#[must_use]
pub fn collect_nodes_negate_center__6aaab0(center: &[f32; 3]) -> [f32; 3] {
    [-center[0], -center[1], -center[2]]
}

#[cfg(test)]
mod tests_collect_nodes_negate_center__6aaab0 {
    use super::collect_nodes_negate_center__6aaab0 as neg;

    #[test]
    fn sign_flips_are_exact() {
        let r = neg(&[1.5, -2.25, 0.0]);
        assert_eq!(r[0].to_bits(), (-1.5f32).to_bits());
        assert_eq!(r[1].to_bits(), 2.25f32.to_bits());
        // FCHS on +0 yields -0 (distinct bit pattern), like Rust negation.
        assert_eq!(r[2].to_bits(), (-0.0f32).to_bits());
    }

    #[test]
    fn nan_payload_and_extremes_survive() {
        let quiet = f32::from_bits(0x7fc0_1234);
        let r = neg(&[quiet, f32::INFINITY, f32::MIN_POSITIVE]);
        // FCHS flips only the sign bit — the NaN payload is untouched.
        assert_eq!(r[0].to_bits(), 0xffc0_1234);
        assert_eq!(r[1].to_bits(), f32::NEG_INFINITY.to_bits());
        assert_eq!(r[2].to_bits(), (-f32::MIN_POSITIVE).to_bits());
    }
}

/// `CWorldView::BuildDrawList` (0x707680) motion-delta prologue kernel.
///
/// Recomputes the view-translation `this+0xcc..0xd4` from the camera 4x4
/// rotation rows and the incoming per-frame `delta` vector. Each output axis is
/// `old[axis] - dot(rotrow, delta)`, but the three axes use DIFFERENT internal
/// `faddp` groupings in the stock x87 body (0x7076cb..0x70774c), so the
/// parenthesisation here is load-bearing and bit-exact (one rounding per `mul`,
/// per `add`).
///
/// `m` packs the nine rotation entries in stock-offset order:
/// `[0x9c,0xa0,0xa4, 0xac,0xb0,0xb4, 0xbc,0xc0,0xc4]`. `old` is the prior
/// translation `[0xcc,0xd0,0xd4]`; `delta` is the `C3Vector` arg `[dx,dy,dz]`.
///
/// Axis 0 (`0xcc`): `old - ((m_ac*dy + m_bc*dz) + m_9c*dx)`.
/// Axis 1 (`0xd0`): `old - ((m_b0*dy + m_a0*dx) + m_c0*dz)`.
/// Axis 2 (`0xd4`): `old - ((m_b4*dy + m_a4*dx) + m_c4*dz)`.
#[inline]
pub fn c_world_view__build_motion_delta__707680(
    m: &[f32; 9],
    old: &[f32; 3],
    delta: &[f32; 3],
) -> [f32; 3] {
    let dx = delta[0];
    let dy = delta[1];
    let dz = delta[2];
    // m[0]=0x9c m[1]=0xa0 m[2]=0xa4 m[3]=0xac m[4]=0xb0 m[5]=0xb4 m[6]=0xbc m[7]=0xc0 m[8]=0xc4.
    let x = old[0] - ((m[3] * dy + m[6] * dz) + m[0] * dx);
    let y = old[1] - ((m[4] * dy + m[1] * dx) + m[7] * dz);
    let z = old[2] - ((m[5] * dy + m[2] * dx) + m[8] * dz);
    [x, y, z]
}

/// `CWorldView::BuildDrawList` (0x707680) per-batch float bucket predicates.
///
/// The stock body decides each batch's draw bucket with a small set of x87
/// `fcomp`/`fnstsw`/`test ah,imm`/`Jcc` idioms interleaved with integer flag
/// reads. This module factors ONLY the pure float predicates so the impure
/// driver stays free of x87 polarity logic; every predicate's NaN result is
/// pinned to the exact stock mask parity (a naive translation gets these wrong).
pub mod draw_bucket_classify__707680 {
    /// Near-plane / zero comparison constant `[0x7ffd74]` (`0.0f`).
    pub const NEAR_ZERO: f32 = f32::from_bits(0x0000_0000);
    /// Opaque-alpha threshold constant `[0x808120]` (`0x3f7fff58`, ~0.99999f).
    pub const OPAQUE_THRESHOLD: f32 = f32::from_bits(0x3f7f_ff58);

    /// Facing test.
    ///
    /// `fcomp` then `test ah,0x41; jp` is taken on greater-OR-unordered (C3|C0
    /// even parity), i.e. `!(a <= b)` keeping NaN. Used at the geometry site
    /// 0x7079df and the cloud mirror 0x7084d8 to set the front-facing flag from
    /// the transformed depth `a` versus the (possibly `fchs`-negated)
    /// screen-radius term `b`.
    // `!(a <= b)` is the point of the helper: unordered operands answer `true`,
    // which `a > b` would not.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    #[inline]
    #[must_use]
    pub fn greater_or_unordered(a: f32, b: f32) -> bool {
        !(a <= b)
    }

    /// `fcomp` then `test ah,0x1; je` is C0-only.
    ///
    /// Returns `true` iff `a < b` strictly and ordered (the C0==1 case). Used at
    /// 0x7079ce / 0x7079f5 (vs 0.0). NaN is not-less (C0==0).
    #[inline]
    #[must_use]
    pub fn strictly_less(a: f32, b: f32) -> bool {
        a < b
    }

    /// Combined open-lower-band cull.
    ///
    /// At 0x707b45 `test ah,0x5; jnp` culls on strictly-less (`alpha < 0`), and
    /// at 0x707b59 `test ah,0x44; jnp` culls on equal-OR-unordered
    /// (`alpha == 0` or NaN). The batch therefore proceeds iff `alpha > 0.0`
    /// strictly and ordered. The cloud mirror is at 0x70853b / 0x70854f. Returns
    /// `true` to PROCEED.
    #[inline]
    #[must_use]
    pub fn alpha_proceeds(alpha: f32) -> bool {
        alpha > NEAR_ZERO
    }

    /// Translucency decision at 0x707ba6 (`fcomp` vs `OPAQUE_THRESHOLD` then `test ah,0x1; je`).
    ///
    /// The translucent flag is set iff `alpha < threshold` strictly (C0==1).
    /// `alpha >= threshold` (or eq/unordered) stays opaque. Returns `true` for
    /// translucent.
    #[inline]
    #[must_use]
    pub fn is_translucent(alpha: f32) -> bool {
        alpha < OPAQUE_THRESHOLD
    }

    /// Cloud-block clamp at 0x7080f9 (`fcomp` vs `0.0` then `test ah,0x5; jp`).
    ///
    /// The value is forced to `0.0` ONLY when strictly less than zero (the `jp`
    /// keeps it for `>= 0` or NaN). Returns the clamped value.
    #[inline]
    #[must_use]
    pub fn clamp_below_zero(v: f32) -> f32 {
        if v < NEAR_ZERO { 0.0 } else { v }
    }

    /// Cloud-block opaque test at 0x7081a8 / 0x7085ed.
    ///
    /// `fcomp` vs threshold then `test ah,0x1; jne`: the non-opaque branch is
    /// taken iff `v < threshold` strictly (C0==1). Returns `true` when the OPAQUE
    /// path should be taken (`v >= threshold`, eq, or unordered).
    // `!(v < OPAQUE_THRESHOLD)` keeps an unordered value on the opaque side, as
    // the `test ah,0x1; jne` routing does.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    #[inline]
    #[must_use]
    pub fn cloud_is_opaque(v: f32) -> bool {
        !(v < OPAQUE_THRESHOLD)
    }
}

#[cfg(test)]
mod tests_c_world_view__build_draw_list__707680 {
    use super::{
        c_world_view__build_motion_delta__707680 as motion_delta,
        draw_bucket_classify__707680 as bucket,
    };

    #[test]
    fn motion_delta_groups_each_axis_per_stock() {
        let m = [
            1.0f32, 2.0, 3.0, // 0x9c,0xa0,0xa4
            0.5, 0.25, 0.125, // 0xac,0xb0,0xb4
            -1.0, -2.0, -3.0, // 0xbc,0xc0,0xc4
        ];
        let old = [10.0f32, 20.0, 30.0];
        let d = [0.3f32, 0.7, 1.1];
        let r = motion_delta(&m, &old, &d);
        let x = old[0] - ((m[3] * d[1] + m[6] * d[2]) + m[0] * d[0]);
        let y = old[1] - ((m[4] * d[1] + m[1] * d[0]) + m[7] * d[2]);
        let z = old[2] - ((m[5] * d[1] + m[2] * d[0]) + m[8] * d[2]);
        assert_eq!(r[0].to_bits(), x.to_bits());
        assert_eq!(r[1].to_bits(), y.to_bits());
        assert_eq!(r[2].to_bits(), z.to_bits());
    }

    #[test]
    fn motion_delta_axis1_grouping_is_b0dy_then_a0dx_then_c0dz() {
        // a0=b0=c0=1 so axis 1 sums dx+dy+dz; output = old - 3.
        let m = [
            0.0f32, 1.0, 0.0, // 9c, a0, a4
            0.0, 1.0, 0.0, // ac, b0, b4
            0.0, 1.0, 0.0, // bc, c0, c4
        ];
        let old = [0.0f32, 0.0, 0.0];
        let d = [1.0f32, 1.0, 1.0];
        let r = motion_delta(&m, &old, &d);
        assert_eq!(r[1].to_bits(), (-3.0f32).to_bits());
    }

    #[test]
    fn constants_match_rdata_dwords() {
        assert_eq!(bucket::NEAR_ZERO.to_bits(), 0x0000_0000);
        assert_eq!(bucket::OPAQUE_THRESHOLD.to_bits(), 0x3f7f_ff58);
    }

    #[test]
    fn greater_or_unordered_keeps_nan() {
        assert!(bucket::greater_or_unordered(2.0, 1.0)); // greater
        assert!(!bucket::greater_or_unordered(1.0, 2.0)); // less
        assert!(!bucket::greater_or_unordered(1.0, 1.0)); // equal
        assert!(bucket::greater_or_unordered(f32::NAN, 1.0)); // unordered -> taken
        assert!(bucket::greater_or_unordered(1.0, f32::NAN)); // unordered -> taken
    }

    #[test]
    fn strictly_less_is_ordered_only() {
        assert!(bucket::strictly_less(1.0, 2.0));
        assert!(!bucket::strictly_less(2.0, 1.0));
        assert!(!bucket::strictly_less(1.0, 1.0));
        assert!(!bucket::strictly_less(f32::NAN, 1.0)); // unordered -> not less
    }

    #[test]
    fn alpha_proceeds_only_strictly_positive() {
        assert!(bucket::alpha_proceeds(0.001));
        assert!(!bucket::alpha_proceeds(0.0)); // equal culled
        assert!(!bucket::alpha_proceeds(-0.001)); // less culled
        assert!(!bucket::alpha_proceeds(f32::NAN)); // unordered culled
    }

    #[test]
    fn translucency_threshold_polarity() {
        assert!(bucket::is_translucent(0.5));
        assert!(!bucket::is_translucent(1.0)); // >= threshold opaque
        assert!(!bucket::is_translucent(bucket::OPAQUE_THRESHOLD)); // eq opaque
        assert!(!bucket::is_translucent(f32::NAN)); // unordered opaque (C0==0)
    }

    #[test]
    fn cloud_clamp_and_opaque_polarity() {
        assert_eq!(bucket::clamp_below_zero(0.5).to_bits(), 0.5f32.to_bits());
        assert_eq!(bucket::clamp_below_zero(0.0).to_bits(), 0.0f32.to_bits());
        assert_eq!(bucket::clamp_below_zero(-0.5).to_bits(), 0.0f32.to_bits());
        assert!(bucket::clamp_below_zero(f32::NAN).is_nan()); // jp taken -> not clamped
        assert!(bucket::cloud_is_opaque(1.0));
        assert!(bucket::cloud_is_opaque(bucket::OPAQUE_THRESHOLD)); // eq opaque
        assert!(!bucket::cloud_is_opaque(0.5));
        assert!(bucket::cloud_is_opaque(f32::NAN)); // unordered -> opaque path
    }
}

/// `CWorldView::ComputeSortHash` (0x70a600) polynomial fold.
///
/// `h = h*0x13 + term` per term, all wrapping. The stock body seeds `h` with the
/// `node[0x30]` value then folds a fixed sequence of record/node/substate dwords;
/// the adapter gathers those term ranges in stock order and folds them through
/// this kernel so the hash inlines into the dedup loop instead of calling out.
/// Integer-exact, so the dedup buckets identically to the original.
#[inline]
#[must_use]
pub fn c_world_view__compute_sort_hash_fold__70a600(initial: u32, terms: &[u32]) -> u32 {
    let mut h = initial;
    for &t in terms {
        h = h.wrapping_mul(0x13).wrapping_add(t);
    }
    h
}

#[cfg(test)]
mod tests_c_world_view__compute_sort_hash_fold__70a600 {
    use super::c_world_view__compute_sort_hash_fold__70a600 as fold;

    #[test]
    fn fold_matches_horner_poly_wrapping() {
        let terms = [1u32, 2, 3];
        let mut e = 7u32;
        for &t in &terms {
            e = e.wrapping_mul(0x13).wrapping_add(t);
        }
        assert_eq!(fold(7, &terms), e);
    }

    #[test]
    fn empty_terms_returns_initial() {
        assert_eq!(fold(42, &[]), 42);
    }

    #[test]
    fn folds_wrap_on_overflow() {
        assert_eq!(
            fold(u32::MAX, &[1]),
            u32::MAX.wrapping_mul(0x13).wrapping_add(1)
        );
    }
}

/// x87 `FISTP m32` under the default control word.
///
/// Round-to-nearest-even to a 32-bit integer, storing the integer-indefinite
/// pattern (`0x8000_0000`) for NaN, ±Inf, and every value whose rounded result
/// falls outside `i32` — unlike Rust's saturating `as` cast (NaN → 0, clamp to
/// `MAX`/`MIN`). Built from the raw f64 bits with no float rounding intrinsic
/// (which may lower through x87 `FRNDINT` on i686) and no FPU arithmetic at all,
/// mirroring the pure-integer discipline of `misc::ftol__40a2b0` (that one
/// truncates toward zero; this one rounds to nearest, ties to even).
/// `pub(crate)`: shared by the sibling biased-FISTP idioms in other kernel
/// families (e.g. `light`).
pub fn fistp_round_ties_even(x: f64) -> i32 {
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32;
    if exp == 0x7ff {
        // NaN / ±Inf -> integer indefinite.
        return i32::MIN;
    }
    let e = exp - 1023;
    if e < -1 {
        // |x| < 0.5 (zeros and subnormals included) rounds to 0.
        return 0;
    }
    if e >= 31 {
        // |x| >= 2^31: only -2^31 itself is representable, and it shares the
        // indefinite bit pattern, so every case collapses to 0x8000_0000.
        return i32::MIN;
    }
    // value = ±m * 2^(e-52) with the implicit leading bit; shift in 22..=53.
    let m = (bits & ((1_u64 << 52) - 1)) | (1_u64 << 52);
    let shift = (52 - e) as u32;
    let floor = m >> shift;
    let rem = m & ((1_u64 << shift) - 1);
    let half = 1_u64 << (shift - 1);
    let mag = floor + u64::from(rem > half || (rem == half && floor & 1 == 1));
    // mag <= 2^31 (e <= 30 caps floor at 2^31-1), so the negate/narrow below is
    // exact; positive 2^31 is out of range -> indefinite (the same bit pattern
    // exactly -2^31 lands on).
    if bits >> 63 == 0 {
        if mag > i32::MAX as u64 {
            i32::MIN
        } else {
            mag as i32
        }
    } else {
        (-(mag as i64)) as i32
    }
}

/// `World_RasterizeTraceLineCells` (0x69c780) DDA core.
///
/// Rasterizes the 2D segment `p1 -> p2` onto the cell grid described by `desc`,
/// driving `emit` with every appended `(perp, major)` i32 pair in stock order.
/// `desc` is `[start major, start perp, end major, end perp]`.
///
/// Faithful transcription of the stock walk:
/// - `slope = (y1-y0)/(x1-x0)` and `inv = numer/slope` are RAW divides with no
///   degenerate-segment guard — a vertical segment (or slope 0) propagates
///   ±Inf/NaN into the per-step index, which the FISTP folds to the x87
///   integer-indefinite `0x8000_0000`, exactly as stock.
/// - The y-intercept `b`, the seeded world coordinate, the per-step advance,
///   and the pre-bias index product are each narrowed to f32 at the exact
///   FSTP points; the surrounding arithmetic runs wide (f64 tracking the x87
///   80-bit stack).
/// - The walk seeds `(desc[1], desc[0])` unconditionally, steps the major axis
///   from `desc[0]` toward `desc[2]` by ±1 (direction from the signed
///   `desc[2] > desc[0]` compare; equal endpoints walk -1), emitting an
///   interior crossing pair whenever the rounded perpendicular index moves and
///   a `(index, major)` pair every step — the do-while runs until the major
///   coordinate passes ONE CELL BEYOND `desc[2]` (`desc[2]+dir`, wrapping),
///   so the one-past-end pair is emitted like stock. A tail
///   `(desc[3], desc[2])` pair closes the list when the last index never
///   reached `desc[3]`. All index compares are integer — no NaN polarity.
// Seven of the eight parameters are the reference's argument list; the eighth
// is the emit callback standing in for its inline stores.
#[allow(clippy::too_many_arguments)]
pub fn world_rasterize_trace_line_cells__69c780(
    p1: [f32; 2],
    p2: [f32; 2],
    desc: [i32; 4],
    numer: f32,
    step: f32,
    scale: f32,
    bias: f32,
    mut emit: impl FnMut(i32, i32),
) {
    let x0 = f64::from(p1[0]);
    let y0 = f64::from(p1[1]);
    let x1 = f64::from(p2[0]);
    let y1 = f64::from(p2[1]);
    // RAW divide (FDIVP, no dx==0 guard): vertical -> ±Inf, point -> NaN.
    let slope = (y1 - y0) / (x1 - x0);
    // y-intercept b = -(slope*x0) + y0 (FCHS-then-FADD order), f32 via FSTP.
    let b = super::f64_to_f32(-(slope * x0) + y0);
    // Second RAW divide (slope 0 -> ±Inf), f32 via FSTP.
    let inv = super::f64_to_f32(f64::from(numer) / slope);
    let start = desc[0];
    let end = desc[2];
    // Signed JLE: end > start walks +1 seeding the NEXT cell boundary
    // (start+1); end <= start (equal included) walks -1 seeding start itself.
    let (dir, seed_cell) = if end > start {
        (1_i32, start.wrapping_add(1))
    } else {
        (-1_i32, start)
    };
    // worldX = (float)seedCell * step (FILD; FMUL; FSTP f32).
    let mut world_x = super::f64_to_f32(f64::from(seed_cell) * f64::from(step));
    // Seed pair, emitted unconditionally.
    emit(desc[1], desc[0]);
    let mut prev = desc[1];
    let mut major = start;
    let stop = end.wrapping_add(dir);
    if major != stop {
        // Per-step world advance = (float)dir * step (FILD; FMUL; FSTP), once.
        let world_step = super::f64_to_f32(f64::from(dir) * f64::from(step));
        loop {
            // The FSTP/FLD round-trip narrows the index product to f32 BEFORE
            // the bias subtract.
            let t = super::f64_to_f32(
                (f64::from(world_x) - f64::from(b)) * f64::from(inv) * f64::from(scale),
            );
            // FISTP: round-to-nearest-even; degenerate slopes reach here as
            // ±Inf/NaN and store the integer indefinite 0x8000_0000.
            let idx = fistp_round_ties_even(f64::from(t) - f64::from(bias));
            // Interior crossing pair only when the perpendicular index moved.
            if idx != prev {
                emit(idx, major);
            }
            world_x = super::f64_to_f32(f64::from(world_step) + f64::from(world_x));
            major = major.wrapping_add(dir);
            emit(idx, major);
            prev = idx;
            if major == stop {
                break;
            }
        }
    }
    // Tail pair when the final perpendicular index never reached desc[3].
    if prev != desc[3] {
        emit(desc[3], desc[2]);
    }
}

#[cfg(test)]
mod tests_world_rasterize_trace_line_cells__69c780 {
    use super::fistp_round_ties_even as fistp;

    fn run(p1: [f32; 2], p2: [f32; 2], desc: [i32; 4], consts: [f32; 4]) -> Vec<(i32, i32)> {
        let mut out = Vec::new();
        super::world_rasterize_trace_line_cells__69c780(
            p1,
            p2,
            desc,
            consts[0],
            consts[1],
            consts[2],
            consts[3],
            |a, b| out.push((a, b)),
        );
        out
    }

    /// Line y = x/2 from (0,0) to (4,2), quarter-cell step.
    ///
    /// Hand-traced against the stock walk. Exercises both FISTP ties in one run
    /// (0.5 -> 0 and 2.5 -> 2 round to EVEN; round-half-away would emit a
    /// different list) and pins the one-past-end pair (major 5 = desc[2]+1).
    #[test]
    fn forward_walk_matches_hand_trace() {
        let got = run([0.0, 0.0], [4.0, 2.0], [0, 0, 4, 2], [1.0, 0.25, 1.0, 0.0]);
        let want = [
            (0, 0), // seed (desc[1], desc[0])
            (0, 1),
            (1, 1), // interior crossing at index move 0 -> 1
            (1, 2),
            (2, 2), // interior crossing at index move 1 -> 2 (t=1.5 ties to 2)
            (2, 3),
            (2, 4), // t=2.0 stays; t=2.5 ties to EVEN 2 -> no crossing
            (2, 5), // one-past-end pair (desc[2]+dir)
        ];
        assert_eq!(got, want);
    }

    /// Same line walked end -> start.
    ///
    /// Direction flips to -1 and the seed cell is desc[0] itself (not
    /// desc[0]+1). Tail pair still suppressed because the last index reaches
    /// desc[3].
    #[test]
    fn reverse_walk_matches_hand_trace() {
        let got = run([4.0, 2.0], [0.0, 0.0], [4, 2, 0, 0], [1.0, 0.25, 1.0, 0.0]);
        let want = [
            (2, 4), // seed
            (2, 3),
            (2, 2),
            (1, 2), // interior crossing 2 -> 1
            (1, 1),
            (0, 1), // interior crossing 1 -> 0 (t=0.5 ties to EVEN 0)
            (0, 0),
            (0, -1), // one-past-end pair (desc[2]-1)
        ];
        assert_eq!(got, want);
    }

    /// Equal endpoints still walk exactly one step with direction -1.
    ///
    /// Stock's signed JLE picks the negative arm on equality, and a mismatched
    /// final index appends the (desc[3], desc[2]) tail pair.
    #[test]
    fn equal_endpoints_walk_one_negative_step_and_tail() {
        let got = run([0.0, 0.0], [1.0, 1.0], [3, 7, 3, 9], [1.0, 1.0, 1.0, 0.0]);
        assert_eq!(got, [(7, 3), (3, 3), (3, 2), (9, 3)]);
    }

    /// Vertical segment.
    ///
    /// Slope = dy/0 = +Inf flows RAW through both divides (b = -Inf, inv = 0)
    /// and the per-step product is Inf*0 = NaN, which the FISTP folds to the x87
    /// integer-indefinite cell index — a guarded or saturating port would emit a
    /// different (non-0x80000000) index.
    #[test]
    fn vertical_segment_emits_integer_indefinite_indices() {
        let got = run([1.0, 0.0], [1.0, 5.0], [0, 9, 2, 9], [1.0, 1.0, 1.0, 0.0]);
        let ind = i32::MIN;
        assert_eq!(
            got,
            [(9, 0), (ind, 0), (ind, 1), (ind, 2), (ind, 3), (9, 2)]
        );
    }

    /// Horizontal segment.
    ///
    /// Slope = 0, so the SECOND raw divide blows up (inv = numer/0 = +Inf) and
    /// the finite (worldX-b) scales to -Inf -> integer indefinite.
    #[test]
    fn horizontal_segment_emits_integer_indefinite_indices() {
        let got = run([0.0, 2.0], [4.0, 2.0], [0, 5, 1, 5], [1.0, 0.25, 1.0, 0.0]);
        let ind = i32::MIN;
        assert_eq!(got, [(5, 0), (ind, 0), (ind, 1), (ind, 2), (5, 1)]);
    }

    /// desc[2]+dir wraps (end = `i32::MAX`, dir = +1) to exactly the start major.
    ///
    /// The stock pre-check then skips the whole walk after the seed pair. A
    /// checked add would panic here; stock's ADD wraps.
    #[test]
    fn wraparound_stop_skips_loop_after_seed() {
        let got = run(
            [0.0, 0.0],
            [4.0, 2.0],
            [i32::MIN, 5, i32::MAX, 5],
            [1.0, 0.25, 1.0, 0.0],
        );
        assert_eq!(got, [(5, i32::MIN)]);
    }

    /// FISTP specials.
    ///
    /// NaN, ±Inf, and out-of-range magnitudes all store the integer-indefinite
    /// pattern; the i32 boundary cases round exactly like the FPU (ties to even,
    /// indefinite only when the ROUNDED value escapes).
    #[test]
    fn fistp_specials_and_boundaries() {
        assert_eq!(fistp(f64::NAN), i32::MIN);
        assert_eq!(fistp(f64::INFINITY), i32::MIN);
        assert_eq!(fistp(f64::NEG_INFINITY), i32::MIN);
        assert_eq!(fistp(f64::MAX), i32::MIN);
        assert_eq!(fistp(-f64::MAX), i32::MIN);
        // 2^31-1+0.5 ties to 2^31 -> out of range -> indefinite.
        assert_eq!(fistp(2_147_483_647.5), i32::MIN);
        assert_eq!(fistp(2_147_483_647.0), i32::MAX);
        // 2^31-2+0.5 ties to the EVEN 2^31-2 -> in range.
        assert_eq!(fistp(2_147_483_646.5), 2_147_483_646);
        assert_eq!(fistp(2_147_483_648.0), i32::MIN);
        // -2^31-0.5 ties to the EVEN -2^31 -> representable (same bits as the
        // indefinite); anything further out is indefinite proper.
        assert_eq!(fistp(-2_147_483_648.5), i32::MIN);
        assert_eq!(fistp(-2_147_483_648.0), i32::MIN);
        assert_eq!(fistp(-2_147_483_649.0), i32::MIN);
    }

    /// FISTP small-magnitude rounding.
    ///
    /// Ties to even in both directions, exact zeros/subnormals collapse to 0.
    #[test]
    fn fistp_ties_to_even_small_values() {
        assert_eq!(fistp(0.5), 0);
        assert_eq!(fistp(-0.5), 0);
        assert_eq!(fistp(1.5), 2);
        assert_eq!(fistp(2.5), 2);
        assert_eq!(fistp(3.5), 4);
        assert_eq!(fistp(-1.5), -2);
        assert_eq!(fistp(-2.5), -2);
        assert_eq!(fistp(0.499_999_999), 0);
        assert_eq!(fistp(0.0), 0);
        assert_eq!(fistp(-0.0), 0);
        assert_eq!(fistp(5e-324), 0);
        assert_eq!(fistp(-5e-324), 0);
        assert_eq!(fistp(0.75), 1);
        assert_eq!(fistp(-0.75), -1);
    }

    /// Sweep oracle.
    ///
    /// The pure-integer helper agrees with the host's `round_ties_even` (SSE2 on
    /// `x86_64`) across a dense in-range sweep including every x.5 tie and non-tie
    /// fractions at several magnitudes.
    #[test]
    fn fistp_matches_round_ties_even_oracle_sweep() {
        for i in -100_000_i64..=100_000 {
            let x = i as f64 / 16.0;
            assert_eq!(fistp(x), x.round_ties_even() as i32, "x={x}");
            let y = i as f64 * 12_345.678_9;
            assert_eq!(fistp(y), y.round_ties_even() as i32, "y={y}");
        }
    }
}

/// `Rasterizer_EmitSlopedLineDDA` (0x69c600) core.
///
/// Walks the major axis of the 2D segment `p0 -> p1` one cell at a time,
/// appending `(major, minor)` i32 pairs in stock order via `emit`. `seg` is
/// `[start minor, start major, end minor, end major]`.
///
/// Sibling of [`world_rasterize_trace_line_cells__69c780`] (same biased-FISTP
/// round, same global output list). Faithful transcription:
/// - `slope = (y1-y0)/(x1-x0)` is a RAW divide (FDIVP, no guard): a vertical
///   or point segment sends ±Inf/NaN through the chain into the FISTP
///   integer-indefinite `0x8000_0000`, exactly as stock.
/// - The intercept `-(slope*x0) + y0` folds from the FULL-precision quotient
///   (stock keeps it on the x87 stack through the FCHS/FADD), but every
///   per-step evaluation reads back the NARROWED f32 slope copy (`FST
///   [ebp-8]` @0x69c61f) — each narrowing at its exact FST/FSTP point.
/// - `t` seeds at `(float)(start major + (dir > 0 ? 1 : 0)) * step` and
///   accumulates `dir * step` in f32 (an FSTP round-trip every iteration).
/// - Each step: `minor = FISTP((t*slope + intercept)*scale - bias)` with one
///   f32 narrow after the scale multiply, BEFORE the bias subtract; an
///   interior `(major, minor)` pair is emitted when the rounded minor moved,
///   then the major advances and the step pair is emitted unconditionally.
///   The walk runs until one cell past `end major` (`seg[3]+dir`, wrapping);
///   a tail `(end major, end minor)` pair closes the list when the last
///   minor never reached `end minor`. All index compares are integer.
pub fn rasterizer_emit_sloped_line_dda__69c600(
    p0: [f32; 2],
    p1: [f32; 2],
    seg: [i32; 4],
    step: f32,
    scale: f32,
    bias: f32,
    mut emit: impl FnMut(i32, i32),
) {
    let x0 = f64::from(p0[0]);
    let y0 = f64::from(p0[1]);
    // RAW divide (FDIVP, no dx==0 guard): vertical -> ±Inf, point -> NaN.
    let slope_full = (f64::from(p1[1]) - y0) / (f64::from(p1[0]) - x0);
    // FST: the narrowed copy the per-step math reads back.
    let slope = super::f64_to_f32(slope_full);
    // Intercept folds from the FULL quotient still on the stack:
    // -(slope*x0) + y0 (FCHS-then-FADD order), f32 via FSTP.
    let intercept = super::f64_to_f32(-(slope_full * x0) + y0);
    let minor_start = seg[0];
    let major_start = seg[1];
    let major_end = seg[3];
    // Signed JLE: end > start walks +1 seeding the NEXT cell boundary
    // (start+1); end <= start (equal included) walks -1 seeding start itself.
    let (dir, seed_cell) = if major_end > major_start {
        (1_i32, major_start.wrapping_add(1))
    } else {
        (-1_i32, major_start)
    };
    // t = (float)seedCell * step (FILD; FMUL; FSTP f32).
    let mut t = super::f64_to_f32(f64::from(seed_cell) * f64::from(step));
    // Seed pair, emitted unconditionally.
    emit(major_start, minor_start);
    let mut minor_prev = minor_start;
    let mut major = major_start;
    let stop = major_end.wrapping_add(dir);
    if major != stop {
        // Per-step advance = (float)dir * step (FILD; FMUL; FSTP), once.
        let dt = super::f64_to_f32(f64::from(dir) * f64::from(step));
        loop {
            // One narrow after the scale multiply, BEFORE the bias subtract.
            let v = super::f64_to_f32(
                (f64::from(t) * f64::from(slope) + f64::from(intercept)) * f64::from(scale),
            );
            // FISTP: round-to-nearest-even; degenerate slopes reach here as
            // ±Inf/NaN and store the integer indefinite 0x8000_0000.
            let minor = fistp_round_ties_even(f64::from(v) - f64::from(bias));
            // Interior crossing pair only when the rounded minor moved.
            if minor != minor_prev {
                emit(major, minor);
            }
            t = super::f64_to_f32(f64::from(dt) + f64::from(t));
            major = major.wrapping_add(dir);
            emit(major, minor);
            minor_prev = minor;
            if major == stop {
                break;
            }
        }
    }
    // Tail pair when the final minor never reached `end minor`.
    if minor_prev != seg[2] {
        emit(seg[3], seg[2]);
    }
}

#[cfg(test)]
mod tests_rasterizer_emit_sloped_line_dda__69c600 {
    fn run(p0: [f32; 2], p1: [f32; 2], seg: [i32; 4], consts: [f32; 3]) -> Vec<(i32, i32)> {
        let mut out = Vec::new();
        super::rasterizer_emit_sloped_line_dda__69c600(
            p0,
            p1,
            seg,
            consts[0],
            consts[1],
            consts[2],
            |a, b| out.push((a, b)),
        );
        out
    }

    /// Line y = x/2 from (0,0) to (4,2), quarter step.
    ///
    /// Hand-traced against the stock walk. The v=0.5 evaluation pins the FISTP
    /// tie (0.5 -> 0, round to EVEN); the (5,1) pair pins the one-past-end step.
    #[test]
    fn forward_walk_matches_hand_trace() {
        let got = run([0.0, 0.0], [4.0, 2.0], [0, 0, 2, 4], [0.25, 1.0, 0.0]);
        let want = [
            (0, 0), // seed (seg[1], seg[0])
            (1, 0),
            (2, 0),
            (3, 0),
            (4, 0), // v = 0.5 ties to even 0
            (4, 1), // interior crossing at major 4
            (5, 1), // one past end major
            (4, 2), // tail (seg[3], seg[2]): minor never reached 2
        ];
        assert_eq!(got, want);
    }

    /// Same line walked backward (end major < start major).
    ///
    /// Dir −1 seeds the start cell itself, and the final minor reaches
    /// `end minor`, so no tail pair is emitted.
    #[test]
    fn backward_walk_matches_hand_trace() {
        let got = run([0.0, 0.0], [4.0, 2.0], [2, 4, 0, 0], [0.25, 1.0, 0.0]);
        let want = [
            (4, 2), // seed
            (4, 0), // v = 0.5 ties to even 0, crossing from minor 2
            (3, 0),
            (2, 0),
            (1, 0),
            (0, 0),
            (-1, 0), // one past end major
        ];
        assert_eq!(got, want);
    }

    /// A point segment divides 0/0.
    ///
    /// The NaN flows through every evaluation into the FISTP integer indefinite,
    /// exactly as the unguarded stock FDIV.
    #[test]
    fn point_segment_nan_flows_to_integer_indefinite() {
        let got = run([1.0, 1.0], [1.0, 1.0], [0, 0, 0, 0], [0.25, 1.0, 0.0]);
        let want = [(0, 0), (0, i32::MIN), (-1, i32::MIN), (0, 0)];
        assert_eq!(got, want);
    }
}

/// `CMap::LoadWdl` (0x6944a0) MARE height-grid conversion (0x694602-0x69463a).
///
/// Converts one WDL tile's 545 `i16` heights (17x17 outer + 16x16 inner
/// lattice) to `f32` while tracking the running min/max. Stock seeds min with
/// the immediate `0x7f7f_ffff` (`f32::MAX`) and max with the `.rdata` const at
/// `0x8105b4` (`-f32::MAX`), passed in as `max_init`. Compare shape: min via
/// `FCOM;FNSTSW;TEST AH,0x5;JP` — update iff strictly below (C0 alone gives
/// odd parity, so the skip-jump falls through); max via
/// `FCOMP;FNSTSW;TEST AH,0x41;JNZ` — update iff strictly above (C3=C0=0).
/// Both compares SKIP on unordered, but the heights arrive through `FILD` of
/// `i16`, so NaN is unreachable on the value side and Rust's ordered `<`/`>`
/// reproduce the polarity exactly (a NaN `max_init` would stick forever under
/// both stock and this port). Every `i16` is exactly representable in `f32`,
/// so no rounding is involved anywhere in this loop.
#[must_use]
pub fn c_map__load_wdl_mare_to_grid__6944a0(
    heights: &[i16; 545],
    max_init: f32,
) -> ([f32; 545], f32, f32) {
    let mut min = f32::from_bits(0x7f7f_ffff);
    let mut max = max_init;
    let mut grid = [0.0f32; 545];
    for (slot, &h) in grid.iter_mut().zip(heights.iter()) {
        let v = f32::from(h);
        *slot = v;
        if v < min {
            min = v;
        }
        if v > max {
            max = v;
        }
    }
    (grid, min, max)
}

/// Per-tile AABB corner / bounding-sphere values of `CMap::LoadWdl` (0x6944a0).
///
/// Exactly the f32s stock stores at 0x69463c-0x694718. The offset map
/// (base = the `CMapAreaLow`): `min_x`/`min_y` at `+0x00`/`+0x04`, `max_x` at
/// `+0x0c` (dup `+0x28`), `max_y` at `+0x10` (dup `+0x2c`), `center` at
/// `+0x18..+0x20`, `radius` at `+0x24`. The min/max HEIGHTS (`+0x08`/`+0x14`)
/// come straight from the MARE loop and are not duplicated here.
#[derive(Clone, Copy, Debug)]
pub struct WdlTileBounds {
    /// `f32(outer * 33.3333)`.
    ///
    /// Stock's dead first store to `+0x2c` (immediately overwritten with
    /// `max_y`); kept for store-order fidelity.
    pub fvar1: f32,
    /// AABB max X (`+0x0c`, dup `+0x28`): `-(outer*33.3333) + 17066.666`.
    pub max_x: f32,
    /// AABB max Y (`+0x10`, dup `+0x2c`): `inner*-33.3333 + 17066.666`.
    pub max_y: f32,
    /// AABB min X (`+0x00`): `f32(max_x) - 533.3333`.
    pub min_x: f32,
    /// AABB min Y (`+0x04`): `f32(max_y) - 533.3333`.
    pub min_y: f32,
    /// Sphere center (`+0x18..+0x20`): `(min + max) * 0.5` per axis.
    pub center: [f32; 3],
    /// Bounding radius (`+0x24`): `sqrt((dz*dz + dy*dy) + dx*dx)` of `max - center`.
    pub radius: f32,
}

/// `CMap::LoadWdl` (0x6944a0) per-tile AABB + bounding-sphere math (0x69463c-0x694718).
///
/// Transcribed in stock x87 op order with `f64` intermediates narrowed through
/// [`super::f64_to_f32`] at every stock f32 store. `outer`/`inner` are the raw
/// 0x10-step tile loop counters (0, 0x10, .., 0x3f0) `FILD`ed as zero-extended
/// qwords; `consts` are the five `.rdata` f32 constants in stock order:
/// `[0x80fee4 (+33.3333 outer scale), 0x8105b0 (-33.3333 inner scale),
/// 0x7ffab4 (17066.666 corner origin), 0x80654c (533.3333 tile span),
/// 0x7ffa24 (0.5)]`. Precision shape:
/// `min_x`/`min_y` subtract the span from the ALREADY f32-rounded max (stock
/// reloads the f32 locals `[EBP-0x2c]`/`[EBP-0x8]`), the X center sum
/// round-trips through an f32 local (`FSTP [EBP-0x4c]`) while the Y/Z sums
/// stay extended (both shapes then multiply by 0.5, which is exact, so the
/// distinction cannot change the stored f32), and the radius folds
/// `(dz*dz + dy*dy) + dx*dx` in that exact `FADDP` order before `FSQRT`.
#[must_use]
pub fn c_map__load_wdl_tile_bounds__6944a0(
    min_h: f32,
    max_h: f32,
    outer: u32,
    inner: u32,
    consts: [f32; 5],
) -> WdlTileBounds {
    let [c_outer_scale, c_inner_scale, c_origin, c_span, c_half] = consts;
    // FILD qword [EBP-0x34]; FMUL [0x80fee4] — outer scaled, kept extended.
    let fvar1_ext = f64::from(outer) * f64::from(c_outer_scale);
    // FST float [ESI+0x2c] — the dead first store (overwritten with max_y).
    let fvar1 = super::f64_to_f32(fvar1_ext);
    // FILD qword [EBP-0x3c]; FMUL [0x8105b0]; FADD [0x7ffab4];
    // FSTP [EBP-0x8] — max Y narrows to f32 here.
    let max_y =
        super::f64_to_f32(f64::from(inner) * f64::from(c_inner_scale) + f64::from(c_origin));
    // FCHS; FADD [0x7ffab4]; FST [EBP-0x2c]/[ESI+0x28]; FSTP [ESI+0xc].
    let max_x = super::f64_to_f32(-fvar1_ext + f64::from(c_origin));
    // FLD [EBP-0x2c] (the f32 max X!); FSUB [0x80654c]; FST [ESI] — the
    // extended difference feeds the X center sum below.
    let min_x_ext = f64::from(max_x) - f64::from(c_span);
    let min_x = super::f64_to_f32(min_x_ext);
    // FLD [EBP-0x8] (the f32 max Y!); FSUB [0x80654c]; FST [ESI+0x4].
    let min_y_ext = f64::from(max_y) - f64::from(c_span);
    let min_y = super::f64_to_f32(min_y_ext);
    // FXCH; FADD [ESI+0xc]; FSTP [EBP-0x4c] — the X sum round-trips f32.
    let sum_x = super::f64_to_f32(min_x_ext + f64::from(max_x));
    // FADD [ESI+0x10] — the Y sum stays extended.
    let sum_y_ext = min_y_ext + f64::from(max_y);
    // FLD [EBP-0xc]; FADD [ESI+0x14] — the Z (height) sum stays extended.
    let sum_z_ext = f64::from(min_h) + f64::from(max_h);
    let half = f64::from(c_half);
    // FLD [EBP-0x4c]; FMUL [0x7ffa24]; FSTP [EBP-0x28] → [ESI+0x18].
    let cx = super::f64_to_f32(f64::from(sum_x) * half);
    // FXCH; FMUL [0x7ffa24]; FSTP [EBP-0x24] → [ESI+0x1c].
    let cy = super::f64_to_f32(sum_y_ext * half);
    // FMUL [0x7ffa24]; FSTP [EBP-0x20] → [ESI+0x20].
    let cz = super::f64_to_f32(sum_z_ext * half);
    // FLD [ESI+0xc]; FSUB [EAX+0x0] (center X) — deltas from the stored f32s.
    let dx = f64::from(max_x) - f64::from(cx);
    // FLD [ESI+0x10]; FSUB [EAX+0x4].
    let dy = f64::from(max_y) - f64::from(cy);
    // FLD [ESI+0x14]; FSUB [EAX+0x8].
    let dz = f64::from(max_h) - f64::from(cz);
    // FMUL/FADDP fold in stock order (dz*dz + dy*dy) + dx*dx; FSQRT;
    // FSTP [ESI+0x24].
    let radius = super::f64_to_f32(((dz * dz + dy * dy) + dx * dx).sqrt());
    WdlTileBounds {
        fvar1,
        max_x,
        max_y,
        min_x,
        min_y,
        center: [cx, cy, cz],
        radius,
    }
}

#[cfg(test)]
mod tests_c_map__load_wdl__6944a0 {
    use super::{
        c_map__load_wdl_mare_to_grid__6944a0 as mare_to_grid,
        c_map__load_wdl_tile_bounds__6944a0 as tile_bounds,
    };

    // The `.rdata` constants the adapter reads (bit patterns verified in the
    // 1.12.1.5875 image at 0x80fee4/0x8105b0/0x7ffab4/0x80654c/0x7ffa24 and
    // the max seed at 0x8105b4).
    const C_OUTER_SCALE: f32 = f32::from_bits(0x4205_5555); // +33.333332
    const C_INNER_SCALE: f32 = f32::from_bits(0xC205_5555); // -33.333332
    const C_ORIGIN: f32 = f32::from_bits(0x4685_5555); // 17066.666
    const C_SPAN: f32 = f32::from_bits(0x4405_5555); // 533.33331
    const C_HALF: f32 = 0.5;
    const MAX_SEED: f32 = f32::from_bits(0xff7f_ffff); // -f32::MAX
    const CONSTS: [f32; 5] = [C_OUTER_SCALE, C_INNER_SCALE, C_ORIGIN, C_SPAN, C_HALF];

    #[test]
    fn grid_converts_every_i16_exactly() {
        let mut heights = [0i16; 545];
        for (i, v) in heights.iter_mut().enumerate() {
            *v = i16::try_from(i)
                .unwrap()
                .wrapping_mul(-89)
                .wrapping_add(1234);
        }
        let (grid, _, _) = mare_to_grid(&heights, MAX_SEED);
        for (g, &h) in grid.iter().zip(heights.iter()) {
            assert_eq!(g.to_bits(), f32::from(h).to_bits());
        }
    }

    #[test]
    fn grid_min_max_match_iterator_oracle() {
        let mut heights = [0i16; 545];
        for (i, v) in heights.iter_mut().enumerate() {
            *v = i16::try_from(i)
                .unwrap()
                .wrapping_mul(-89)
                .wrapping_add(1234);
        }
        let (_, min, max) = mare_to_grid(&heights, MAX_SEED);
        let imin = heights.iter().copied().min().unwrap();
        let imax = heights.iter().copied().max().unwrap();
        assert_eq!(min.to_bits(), f32::from(imin).to_bits());
        assert_eq!(max.to_bits(), f32::from(imax).to_bits());
    }

    #[test]
    fn max_update_is_strictly_greater_so_a_high_seed_survives() {
        // TEST AH,0x41 / JNZ skips on equal-or-below: no height beats the
        // seed, so the seed is returned unchanged.
        let heights = [-5i16; 545];
        let (_, min, max) = mare_to_grid(&heights, 100.0);
        assert_eq!(min.to_bits(), (-5.0f32).to_bits());
        assert_eq!(max.to_bits(), 100.0f32.to_bits());
    }

    #[test]
    fn min_seed_is_the_flt_max_immediate() {
        // Stock seeds min via `MOV dword [EBP-0xc],0x7f7fffff`; even i16::MAX
        // is strictly below it, so min always lands on a real height.
        let heights = [i16::MAX; 545];
        let (_, min, max) = mare_to_grid(&heights, MAX_SEED);
        assert_eq!(min.to_bits(), f32::from(i16::MAX).to_bits());
        assert_eq!(max.to_bits(), f32::from(i16::MAX).to_bits());
    }

    #[test]
    fn nan_max_seed_sticks_forever_matching_unordered_skip() {
        // Unreachable in-game (the seed const is -FLT_MAX), but pins the
        // unordered-compare direction: FCOMP vs a NaN ST1 sets C3/C2/C0, so
        // TEST AH,0x41 is nonzero and the update is skipped every iteration.
        let heights = [1000i16; 545];
        let (_, min, max) = mare_to_grid(&heights, f32::NAN);
        assert_eq!(min.to_bits(), 1000.0f32.to_bits());
        assert!(max.is_nan());
    }

    fn narrow(v: f64) -> f32 {
        v as f32
    }

    #[test]
    fn tile_bounds_matches_the_f64_oracle_bit_exactly() {
        let (outer, inner) = (0x30u32, 0x150u32);
        let (min_h, max_h) = (-123.0f32, 456.0f32);
        let b = tile_bounds(min_h, max_h, outer, inner, CONSTS);

        let fvar1_ext = f64::from(outer) * f64::from(C_OUTER_SCALE);
        assert_eq!(b.fvar1.to_bits(), narrow(fvar1_ext).to_bits());
        let max_y = narrow(f64::from(inner) * f64::from(C_INNER_SCALE) + f64::from(C_ORIGIN));
        assert_eq!(b.max_y.to_bits(), max_y.to_bits());
        let max_x = narrow(-fvar1_ext + f64::from(C_ORIGIN));
        assert_eq!(b.max_x.to_bits(), max_x.to_bits());
        // Min corners subtract the span from the f32-rounded max.
        let min_x_ext = f64::from(max_x) - f64::from(C_SPAN);
        let min_y_ext = f64::from(max_y) - f64::from(C_SPAN);
        assert_eq!(b.min_x.to_bits(), narrow(min_x_ext).to_bits());
        assert_eq!(b.min_y.to_bits(), narrow(min_y_ext).to_bits());
        // Centers: X through the f32 sum local, Y extended, Z extended.
        let sum_x = narrow(min_x_ext + f64::from(max_x));
        let cx = narrow(f64::from(sum_x) * 0.5);
        let cy = narrow((min_y_ext + f64::from(max_y)) * 0.5);
        let cz = narrow((f64::from(min_h) + f64::from(max_h)) * 0.5);
        assert_eq!(b.center[0].to_bits(), cx.to_bits());
        assert_eq!(b.center[1].to_bits(), cy.to_bits());
        assert_eq!(b.center[2].to_bits(), cz.to_bits());
        // Radius: (dz*dz + dy*dy) + dx*dx in that order, then sqrt.
        let dx = f64::from(max_x) - f64::from(cx);
        let dy = f64::from(max_y) - f64::from(cy);
        let dz = f64::from(max_h) - f64::from(cz);
        let radius = narrow(((dz * dz + dy * dy) + dx * dx).sqrt());
        assert_eq!(b.radius.to_bits(), radius.to_bits());
    }

    #[test]
    fn tile_bounds_zero_tile_corners_are_the_origin_consts() {
        let b = tile_bounds(0.0, 0.0, 0, 0, CONSTS);
        assert_eq!(b.fvar1.to_bits(), 0.0f32.to_bits());
        assert_eq!(b.max_x.to_bits(), C_ORIGIN.to_bits());
        assert_eq!(b.max_y.to_bits(), C_ORIGIN.to_bits());
        let min = narrow(f64::from(C_ORIGIN) - f64::from(C_SPAN));
        assert_eq!(b.min_x.to_bits(), min.to_bits());
        assert_eq!(b.min_y.to_bits(), min.to_bits());
        assert_eq!(b.center[2].to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn tile_bounds_x_roundtrip_and_y_extended_paths_agree_on_equal_counters() {
        // The inner scale is the exact negation of the outer scale
        // (0xC2055555 = -0x42055555), so outer == inner makes the X and Y
        // corner math identical; the X path's extra f32 round-trip before the
        // exact *0.5 must not diverge from the Y path's extended sum.
        for step in [0u32, 0x10, 0x150, 0x3f0] {
            let b = tile_bounds(-40.0, 10.0, step, step, CONSTS);
            assert_eq!(b.max_x.to_bits(), b.max_y.to_bits());
            assert_eq!(b.min_x.to_bits(), b.min_y.to_bits());
            assert_eq!(b.center[0].to_bits(), b.center[1].to_bits());
        }
    }

    #[test]
    fn tile_bounds_geometry_sanity() {
        // Independent happy-path oracle: the corner span is one WDL tile
        // (533.3333), the center is the midpoint, and the radius is the
        // center-to-max-corner distance.
        let b = tile_bounds(10.0, 50.0, 0x40, 0x80, CONSTS);
        assert!((f64::from(b.max_x) - f64::from(b.min_x) - f64::from(C_SPAN)).abs() < 1e-3);
        assert!((f64::from(b.max_y) - f64::from(b.min_y) - f64::from(C_SPAN)).abs() < 1e-3);
        let mid_x = f64::midpoint(f64::from(b.min_x), f64::from(b.max_x));
        assert!((f64::from(b.center[0]) - mid_x).abs() < 1e-3);
        assert!((f64::from(b.center[2]) - 30.0).abs() < 1e-6);
        let dx = f64::from(b.max_x) - f64::from(b.center[0]);
        let dy = f64::from(b.max_y) - f64::from(b.center[1]);
        let dz = f64::from(50.0f32) - f64::from(b.center[2]);
        let dist = ((dz * dz + dy * dy) + dx * dx).sqrt();
        assert!((f64::from(b.radius) - dist).abs() < 1e-3);
    }
}
/// Output block for `CMapChunk::BuildVerticesAndBounds` (`0x6b0e50`).
///
/// The 145 generated vertex XY pairs (Z is the raw MCVT height dword the adapter
/// bit-copies alongside), the origin-translated AABB, its center and bounding
/// radius, and the cached 6-float box (the reference's final 6-dword copy of
/// `min`/`max` into `chunk+0x94`).
pub struct ChunkVertsAndBounds {
    pub xy: [[f32; 2]; 145],
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub center: [f32; 3],
    pub radius: f32,
    pub cached: [f32; 6],
}

/// Builds a terrain chunk's 145 local-space vertices.
///
/// A 9x9 outer grid interleaved with 8x8 half-cell inner rows as 17-vertex row
/// pairs, plus its AABB/center/radius, from the map-grid indices, the raw MCVT
/// height dwords, the chunk world origin, and the four host globals
/// (`scale` = grid-index -> world scale, `base` = world base offset,
/// `cells` = 8.0 cells/chunk, `half` = 2.0 midpoint divisor).
///
/// Arithmetic follows the reference's x87 op order, computed in `f64` and
/// narrowed exactly where the original stores an f32 — bit-exact, because each
/// narrowing is then a single rounding of an f64-exact intermediate (i32->f32
/// conversions, small-int x f32 products, and single f32 +/-/x// steps all fit
/// f64's mantissa; the one fused chain — the inner-row Y, whose `col x step`
/// product stays on the x87 stack before the half-cell add — is a <=28-bit
/// product plus a half-cell, still f64-exact). Per-vertex min accumulation
/// reuses the already-reimplemented callee's kernel (the reference CALLs `0x699250`
/// per vertex): `frustum_compute_c3_vector__699250` (component min; the vertex
/// operand wins ties/unordered on every lane). The
/// per-vertex max CALL (`0x6b11f0`) is reimplemented inline with the stock
/// polarity — the vertex wins equal AND unordered lanes on all three axes.
/// (The `vector::c3_vector__max_in_place__6b11f0` kernel originally kept the
/// accumulator on ties/unordered — a NaN-direction subtlety this
/// review caught; that kernel has since been fixed to the same stock polarity,
/// and the local transcription is kept for locality with the notes
/// below.) Both accumulators seed with the same
/// `0x7f7fffff`/`0xff7fffff` bits the reference stores at `+0x44`/`+0x50`.
///
/// The radius is the accepted <=1-ULP x87/SSE class: the reference sums the
/// three squared half-extents at 80 bits and narrows once, then its sqrt
/// helper (`0x741760`, an ST0-in/ST0-out CRT thunk that narrows ST0 to double
/// and square-roots it) produces the value narrowed to f32 at `+0x68`; the
/// `f64` sum + `f64::sqrt` here matches except for astronomically-rare
/// double-rounding ties in the 3-term sum. Everything else is bit-exact.
// The cell indices, height block, origin, scale, base and cell divisors are the
// reference's argument list, not a design choice.
#[allow(clippy::too_many_arguments)]
pub fn c_map_chunk__build_vertices_and_bounds__6b0e50(
    col: i32,
    row: i32,
    heights: &[u32; 145],
    origin: &[f32; 3],
    scale: f32,
    base: f32,
    cells: f32,
    half: f32,
) -> ChunkVertsAndBounds {
    let scale64 = f64::from(scale);
    let base64 = f64::from(base);
    // Grid anchors: `-(index * scale) + base` for row, col, row+1, col+1 (the
    // +1 indices are 32-bit wrapping increments, as the reference's INC), then
    // the spans `lo - hi`, negated, /cells for the per-cell steps and /half for
    // the half-cell offsets — one f32 narrowing per step, like the reference.
    let row_scaled = super::f64_to_f32(f64::from(row as f32) * scale64);
    let col_scaled = super::f64_to_f32(f64::from(col as f32) * scale64);
    let row1_scaled = super::f64_to_f32(f64::from(row.wrapping_add(1) as f32) * scale64);
    let col1_scaled = super::f64_to_f32(f64::from(col.wrapping_add(1) as f32) * scale64);
    let row_lo = super::f64_to_f32(f64::from(-row_scaled) + base64);
    let col_lo = super::f64_to_f32(f64::from(-col_scaled) + base64);
    let row_hi = super::f64_to_f32(f64::from(-row1_scaled) + base64);
    let col_hi = super::f64_to_f32(f64::from(-col1_scaled) + base64);
    let col_span = super::f64_to_f32(f64::from(col_lo) - f64::from(col_hi));
    let row_span = super::f64_to_f32(f64::from(row_lo) - f64::from(row_hi));
    let step_col = super::f64_to_f32(f64::from(-col_span) / f64::from(cells));
    let step_row = super::f64_to_f32(f64::from(-row_span) / f64::from(cells));
    let half_col = super::f64_to_f32(f64::from(step_col) / f64::from(half));
    let half_row = super::f64_to_f32(f64::from(step_row) / f64::from(half));

    // Stock's per-vertex max CALL (`0x6b11f0`): the accumulator lane survives
    // ONLY a strictly-greater ORDERED compare; the vertex wins equal AND
    // unordered lanes on all three axes (z/y: `FCOMPP` + `TEST AH,0x5` + `JP`
    // — the equal status pattern AH&5=0 and the unordered C0|C2 pattern
    // AH&5=0x05 both have even parity, so both take the src load; x: `FCOMP`
    // + `TEST AH,0x41` + `JNZ` — the equal C3 and unordered C0|C3 patterns
    // both jump to the src load). Kept as a local transcription for locality
    // with these notes (the shared
    // `vector::c3_vector__max_in_place__6b11f0` kernel, whose original
    // opposite polarity this review caught, now matches). Stock's winner
    // round-trips through FLD/FSTP (quieting an SNaN vertex height before the
    // store); this raw-bit copy is the campaign's accepted SNaN-payload class.
    fn max_in_place_stock(dst: &mut [f32; 3], src: &[f32; 3]) {
        for i in 0..3 {
            dst[i] = if dst[i] > src[i] { dst[i] } else { src[i] };
        }
    }

    // Emit order: per outer row r (0..9), nine outer vertices (x = r*step_row,
    // y = c*step_col), then — for r < 8 — eight inner vertices (x = outer x +
    // half_row, y = c*step_col + half_col), heights streamed strictly
    // sequentially across both sub-rows. Min/max accumulate per vertex with
    // the callees' exact tie/unordered polarity (vertex wins on both).
    let mut xy = [[0.0f32; 2]; 145];
    let mut bmin = [f32::MAX; 3];
    let mut bmax = [f32::MIN; 3];
    let mut k = 0usize;
    for r in 0..9i32 {
        let outer_x = super::f64_to_f32(f64::from(r as f32) * f64::from(step_row));
        for c in 0..9i32 {
            let y = super::f64_to_f32(f64::from(c as f32) * f64::from(step_col));
            let v = [outer_x, y, f32::from_bits(heights[k])];
            xy[k] = [v[0], v[1]];
            bmin = super::frustum::frustum_compute_c3_vector__699250(&bmin, &v);
            max_in_place_stock(&mut bmax, &v);
            k += 1;
        }
        if r < 8 {
            let inner_x = super::f64_to_f32(f64::from(outer_x) + f64::from(half_row));
            for c in 0..8i32 {
                // The `c * step_col` product feeds the half-cell add unrounded
                // in the reference; a single narrowing after the add, as here.
                let y = super::f64_to_f32(
                    f64::from(c as f32) * f64::from(step_col) + f64::from(half_col),
                );
                let v = [inner_x, y, f32::from_bits(heights[k])];
                xy[k] = [v[0], v[1]];
                bmin = super::frustum::frustum_compute_c3_vector__699250(&bmin, &v);
                max_in_place_stock(&mut bmax, &v);
                k += 1;
            }
        }
    }

    // Tail: translate both corners by the chunk origin (min lanes add
    // `origin + min`, max.x adds `max + origin`, max.y/z `origin + max` — the
    // reference's operand orders; f32 addition rounds identically either way),
    // then center = (min+max)/half and radius = sqrt(sum((max - center)^2)).
    let min = [
        super::f64_to_f32(f64::from(origin[0]) + f64::from(bmin[0])),
        super::f64_to_f32(f64::from(origin[1]) + f64::from(bmin[1])),
        super::f64_to_f32(f64::from(origin[2]) + f64::from(bmin[2])),
    ];
    let max = [
        super::f64_to_f32(f64::from(bmax[0]) + f64::from(origin[0])),
        super::f64_to_f32(f64::from(origin[1]) + f64::from(bmax[1])),
        super::f64_to_f32(f64::from(origin[2]) + f64::from(bmax[2])),
    ];
    let tx = super::f64_to_f32(f64::from(min[0]) + f64::from(max[0]));
    let ty = super::f64_to_f32(f64::from(min[1]) + f64::from(max[1]));
    let tz = super::f64_to_f32(f64::from(min[2]) + f64::from(max[2]));
    let center = [
        super::f64_to_f32(f64::from(tx) / f64::from(half)),
        super::f64_to_f32(f64::from(ty) / f64::from(half)),
        super::f64_to_f32(f64::from(tz) / f64::from(half)),
    ];
    let dx = super::f64_to_f32(f64::from(max[0]) - f64::from(center[0]));
    let dy = super::f64_to_f32(f64::from(max[1]) - f64::from(center[1]));
    let dz = super::f64_to_f32(f64::from(max[2]) - f64::from(center[2]));
    // The reference sums dx^2 + dy^2 (FADDP), then + dz^2, at 80-bit, and
    // narrows once — the accepted <=1-ULP spot (feeds only the radius).
    let sum = super::f64_to_f32(
        f64::from(dx) * f64::from(dx)
            + f64::from(dy) * f64::from(dy)
            + f64::from(dz) * f64::from(dz),
    );
    let radius = super::f64_to_f32(f64::from(sum).sqrt());
    let cached = [min[0], min[1], min[2], max[0], max[1], max[2]];
    ChunkVertsAndBounds {
        xy,
        min,
        max,
        center,
        radius,
        cached,
    }
}

#[cfg(test)]
mod tests_c_map_chunk__build_vertices_and_bounds__6b0e50 {
    use super::c_map_chunk__build_vertices_and_bounds__6b0e50 as build;

    /// Height dwords all set to the bits of one f32.
    fn flat_heights(v: f32) -> [u32; 145] {
        [v.to_bits(); 145]
    }

    /// Power-of-two globals make every coordinate exact.
    ///
    /// scale=32, base=1024 give step = -4.0 and half-step = -2.0 on both axes,
    /// so the interleave, the half-cell placement, and the emit-order indexing
    /// are all assertable bit-for-bit (including the `0 * negative-step = -0.0`
    /// sign at c=0/r=0).
    #[test]
    fn emit_order_interleave_and_half_cell_exact() {
        let out = build(
            3,
            5,
            &flat_heights(5.25),
            &[100.0, 200.0, -50.0],
            32.0,
            1024.0,
            8.0,
            2.0,
        );
        for r in 0..9usize {
            for c in 0..9usize {
                let k = r * 17 + c;
                let ex = -4.0 * r as f32; // r=0 -> -0.0
                let ey = -4.0 * c as f32; // c=0 -> -0.0
                assert_eq!(out.xy[k][0].to_bits(), ex.to_bits(), "outer x r{r} c{c}");
                assert_eq!(out.xy[k][1].to_bits(), ey.to_bits(), "outer y r{r} c{c}");
            }
            if r < 8 {
                for c in 0..8usize {
                    let k = r * 17 + 9 + c;
                    let ex = -4.0 * r as f32 - 2.0;
                    let ey = -4.0 * c as f32 - 2.0;
                    assert_eq!(out.xy[k][0].to_bits(), ex.to_bits(), "inner x r{r} c{c}");
                    assert_eq!(out.xy[k][1].to_bits(), ey.to_bits(), "inner y r{r} c{c}");
                }
            }
        }
        // Vertex 0 is (-0.0, -0.0): FILD 0 times a negative step keeps the
        // product's sign, exactly like the x87 original.
        assert_eq!(out.xy[0][0].to_bits(), (-0.0f32).to_bits());
        assert_eq!(out.xy[0][1].to_bits(), (-0.0f32).to_bits());
    }

    /// Bounds/center/radius over the exact grid.
    ///
    /// Local box is [-32,-32,5.25]..[-0.0,-0.0,5.25], translated by the origin;
    /// the radius is sqrt(16^2 + 16^2 + 0) = sqrt(512), exact through the f64
    /// path.
    #[test]
    fn bounds_center_radius_and_cached_copy() {
        let out = build(
            3,
            5,
            &flat_heights(5.25),
            &[100.0, 200.0, -50.0],
            32.0,
            1024.0,
            8.0,
            2.0,
        );
        assert_eq!(out.min, [68.0, 168.0, -44.75]);
        assert_eq!(out.max, [100.0, 200.0, -44.75]);
        assert_eq!(out.center, [84.0, 184.0, -44.75]);
        assert_eq!(out.radius.to_bits(), (512.0f64.sqrt() as f32).to_bits());
        // The +0x94 cached box is a 6-dword copy of the final min/max.
        for i in 0..3 {
            assert_eq!(
                out.cached[i].to_bits(),
                out.min[i].to_bits(),
                "cached min {i}"
            );
            assert_eq!(
                out.cached[i + 3].to_bits(),
                out.max[i].to_bits(),
                "cached max {i}"
            );
        }
        // With a zero origin the translate resolves max.x's -0.0 to +0.0
        // ((+0.0) + (-0.0) rounds to +0.0), like the x87 FADD.
        let out0 = build(3, 5, &flat_heights(5.25), &[0.0; 3], 32.0, 1024.0, 8.0, 2.0);
        assert_eq!(out0.max[0].to_bits(), 0.0f32.to_bits());
        assert_eq!(out0.cached[3].to_bits(), 0.0f32.to_bits());
        assert_eq!(out0.min[0], -32.0);
    }

    /// Realistic `WoW` globals with far-apart grid indices.
    ///
    /// The two axes' f32 spans (and so steps and half-cells) differ in their low
    /// bits, which pins x-from-row/y-from-col against transposition,
    /// bit-for-bit, via an independently written oracle of the same op chain;
    /// min/max/center are cross-checked against a plain fold; the radius against
    /// a full-f64 oracle within 1 ULP.
    #[test]
    fn realistic_globals_match_f64_oracle() {
        let (col, row) = (989i32, 21i32);
        let scale = 100.0f32 / 3.0;
        let base = 51200.0f32 / 3.0;
        let origin = [-4266.6665f32, 1233.3333, 87.9375];
        let mut heights = [0u32; 145];
        let mut s = 0x1234_5678u32;
        for h in &mut heights {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *h = (((s >> 9) as f32) * 0.001 - 200.0).to_bits();
        }
        let out = build(col, row, &heights, &origin, scale, base, 8.0, 2.0);

        // Oracle steps (independent transcription of the same chain).
        let anchor = |i: i32| -> f32 {
            let scaled = (f64::from(i as f32) * f64::from(scale)) as f32;
            (f64::from(-scaled) + f64::from(base)) as f32
        };
        let span = |lo: f32, hi: f32| (f64::from(lo) - f64::from(hi)) as f32;
        let row_span = span(anchor(row), anchor(row + 1));
        let col_span = span(anchor(col), anchor(col + 1));
        let step_row = (f64::from(-row_span) / 8.0) as f32;
        let step_col = (f64::from(-col_span) / 8.0) as f32;
        let half_row = (f64::from(step_row) / 2.0) as f32;
        let half_col = (f64::from(step_col) / 2.0) as f32;
        assert_ne!(step_row.to_bits(), step_col.to_bits(), "axes must differ");

        let mut k = 0usize;
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for r in 0..9i32 {
            let ox = (f64::from(r as f32) * f64::from(step_row)) as f32;
            for c in 0..9i32 {
                let ey = (f64::from(c as f32) * f64::from(step_col)) as f32;
                assert_eq!(out.xy[k][0].to_bits(), ox.to_bits(), "outer x k{k}");
                assert_eq!(out.xy[k][1].to_bits(), ey.to_bits(), "outer y k{k}");
                let z = f32::from_bits(heights[k]);
                for (i, w) in [ox, ey, z].into_iter().enumerate() {
                    lo[i] = lo[i].min(w);
                    hi[i] = hi[i].max(w);
                }
                k += 1;
            }
            if r < 8 {
                let ix = (f64::from(ox) + f64::from(half_row)) as f32;
                for c in 0..8i32 {
                    let ey =
                        (f64::from(c as f32) * f64::from(step_col) + f64::from(half_col)) as f32;
                    assert_eq!(out.xy[k][0].to_bits(), ix.to_bits(), "inner x k{k}");
                    assert_eq!(out.xy[k][1].to_bits(), ey.to_bits(), "inner y k{k}");
                    let z = f32::from_bits(heights[k]);
                    for (i, w) in [ix, ey, z].into_iter().enumerate() {
                        lo[i] = lo[i].min(w);
                        hi[i] = hi[i].max(w);
                    }
                    k += 1;
                }
            }
        }
        assert_eq!(k, 145);

        // Translate + center per the same narrowing chain.
        for i in 0..3 {
            let emin = (f64::from(origin[i]) + f64::from(lo[i])) as f32;
            let emax = (f64::from(hi[i]) + f64::from(origin[i])) as f32;
            assert_eq!(out.min[i].to_bits(), emin.to_bits(), "min {i}");
            assert_eq!(out.max[i].to_bits(), emax.to_bits(), "max {i}");
            let t = (f64::from(emin) + f64::from(emax)) as f32;
            let ec = (f64::from(t) / 2.0) as f32;
            assert_eq!(out.center[i].to_bits(), ec.to_bits(), "center {i}");
        }
        // Radius: full-f64 oracle (no intermediate f32 sum), within 1 ULP.
        let d: Vec<f64> = (0..3)
            .map(|i| f64::from((f64::from(out.max[i]) - f64::from(out.center[i])) as f32))
            .collect();
        let oracle = ((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()) as f32;
        let dist = (out.radius.to_bits() as i64 - oracle.to_bits() as i64).abs();
        assert!(dist <= 1, "radius {} vs oracle {oracle}", out.radius);
    }

    /// NaN heights follow both callees' stock unordered polarity.
    ///
    /// The VERTEX operand wins unordered lanes on min (0x699250) AND max
    /// (0x6b11f0). A mid-stream NaN poisons both accumulators' z and the next
    /// vertex recovers both (src wins the next unordered compare); a NaN on the
    /// LAST vertex sticks in BOTH and flows through translate/center into the
    /// radius and the cached box.
    #[test]
    fn nan_height_unordered_polarity() {
        // NaN first: recovered by vertex 1 on both accumulators; all finite.
        let mut heights = flat_heights(7.5);
        heights[0] = f32::NAN.to_bits();
        let out = build(3, 5, &heights, &[0.0; 3], 32.0, 1024.0, 8.0, 2.0);
        assert_eq!(out.min[2], 7.5);
        assert_eq!(out.max[2], 7.5);
        assert!(!out.radius.is_nan());

        // NaN last: min.z AND max.z stay NaN (stock's max takes src on the
        // unordered final compare), and the NaN flows through translate and
        // center into the radius and both cached z lanes.
        let mut heights = flat_heights(7.5);
        heights[144] = f32::NAN.to_bits();
        let out = build(3, 5, &heights, &[0.0; 3], 32.0, 1024.0, 8.0, 2.0);
        assert!(out.min[2].is_nan());
        assert!(out.max[2].is_nan());
        assert!(out.center[2].is_nan());
        assert!(out.radius.is_nan());
        assert!(out.cached[2].is_nan());
        assert!(out.cached[5].is_nan());
    }

    /// ±0 bit-ties take the VERTEX's bits in both accumulators.
    ///
    /// Per the stock callees (max 0x6b11f0: equal parity pattern jumps to the
    /// src load; min 0x699250: b wins ties). heights[0] = +0.0 seeds both z
    /// accumulators with +0.0; every later height is −0.0, an equal compare that
    /// stock re-stores as the src's −0.0 bits. Observable through the translate
    /// when origin.z = −0.0: (−0.0) + (−0.0) keeps the sign, so min.z, max.z
    /// and both cached z lanes must all carry the −0.0 bit pattern.
    #[test]
    fn max_zero_tie_takes_vertex_bits() {
        let mut heights = flat_heights(-0.0);
        heights[0] = 0.0f32.to_bits();
        let origin = [0.0f32, 0.0, -0.0];
        let out = build(3, 5, &heights, &origin, 32.0, 1024.0, 8.0, 2.0);
        assert_eq!(out.max[2].to_bits(), (-0.0f32).to_bits());
        assert_eq!(out.min[2].to_bits(), (-0.0f32).to_bits());
        assert_eq!(out.cached[2].to_bits(), (-0.0f32).to_bits());
        assert_eq!(out.cached[5].to_bits(), (-0.0f32).to_bits());
    }

    /// The +1 grid increments wrap like the reference's 32-bit INC.
    ///
    /// A plain `+ 1` would overflow-panic in debug builds and change the anchor.
    #[test]
    fn grid_index_increment_wraps() {
        let out = build(
            0,
            i32::MAX,
            &flat_heights(1.0),
            &[0.0; 3],
            1.5,
            0.0,
            8.0,
            2.0,
        );
        // row anchor: -(f32(MAX) * 1.5); row+1 wraps to MIN: -(f32(MIN) * 1.5).
        let lo = (f64::from(-((f64::from(i32::MAX as f32) * 1.5) as f32))) as f32;
        let hi = (f64::from(-((f64::from(i32::MIN as f32) * 1.5) as f32))) as f32;
        let span = (f64::from(lo) - f64::from(hi)) as f32;
        let step_row = (f64::from(-span) / 8.0) as f32;
        // Outer row r=1 starts at k=17; its x is 1 * step_row.
        assert_eq!(out.xy[17][0].to_bits(), step_row.to_bits());
    }
}

/// View-gate predicates for `CMap__UpdateMapObjs` @0x698720.
///
/// Every stock compare is FCOMP + FNSTSW with a polarity that CULLS on NaN
/// (unordered takes the skip arm in all three boxes), so plain ordered `<=`
/// reproduces each one exactly.
///
/// Near-box half-gate: `viewMin[i] <= objMax[i]` per axis (the other half is
/// the already-native `C3Vector__GreaterEqualAll(viewMax, objMin)`).
pub fn map_obj_view_gate__698720(view_min: &[f32; 3], obj_max: &[f32; 3]) -> bool {
    view_min[0] <= obj_max[0] && view_min[1] <= obj_max[1] && view_min[2] <= obj_max[2]
}

/// Tight-box full test (6 compares in stock order: the three max-side, then the three min-side).
pub fn map_obj_tight_gate__698720(
    tight: &[f32; 6],
    node_min: &[f32; 3],
    node_max: &[f32; 3],
) -> bool {
    tight[0] <= node_max[0]
        && tight[1] <= node_max[1]
        && tight[2] <= node_max[2]
        && node_min[0] <= tight[3]
        && node_min[1] <= tight[4]
        && node_min[2] <= tight[5]
}

/// Flagged far-pass test: `defMin[i] <= farHi[i]` then `farLo[i] <= defMax[i]`.
pub fn map_obj_far_gate__698720(
    far_lo: &[f32; 3],
    far_hi: &[f32; 3],
    def_min: &[f32; 3],
    def_max: &[f32; 3],
) -> bool {
    def_min[0] <= far_hi[0]
        && def_min[1] <= far_hi[1]
        && def_min[2] <= far_hi[2]
        && far_lo[0] <= def_max[0]
        && far_lo[1] <= def_max[1]
        && far_lo[2] <= def_max[2]
}

#[cfg(test)]
mod tests_map_obj_gates__698720 {
    use super::{map_obj_far_gate__698720, map_obj_tight_gate__698720, map_obj_view_gate__698720};

    /// Overlap passes, separation fails, boundary equality PASSES.
    ///
    /// Ordered `==` continues in stock, and any NaN bound CULLS.
    #[test]
    fn view_gate_polarity() {
        assert!(map_obj_view_gate__698720(&[0.0; 3], &[1.0; 3]));
        assert!(map_obj_view_gate__698720(&[1.0; 3], &[1.0; 3]));
        assert!(!map_obj_view_gate__698720(&[2.0; 3], &[1.0; 3]));
        assert!(!map_obj_view_gate__698720(&[0.0; 3], &[1.0, f32::NAN, 1.0]));
        assert!(!map_obj_view_gate__698720(&[0.0, f32::NAN, 0.0], &[1.0; 3]));
    }

    #[test]
    fn tight_gate_polarity() {
        let tight = [0.0, 0.0, 0.0, 10.0, 10.0, 10.0];
        assert!(map_obj_tight_gate__698720(&tight, &[1.0; 3], &[2.0; 3]));
        // Min beyond the tight max on one axis fails.
        assert!(!map_obj_tight_gate__698720(
            &tight,
            &[1.0, 11.0, 1.0],
            &[2.0, 12.0, 2.0]
        ));
        // Max below the tight min fails.
        assert!(!map_obj_tight_gate__698720(&tight, &[-2.0; 3], &[-1.0; 3]));
        // NaN anywhere culls.
        assert!(!map_obj_tight_gate__698720(
            &tight,
            &[f32::NAN, 1.0, 1.0],
            &[2.0; 3]
        ));
    }

    #[test]
    fn far_gate_polarity() {
        let (lo, hi) = ([0.0f32; 3], [10.0f32; 3]);
        assert!(map_obj_far_gate__698720(&lo, &hi, &[1.0; 3], &[2.0; 3]));
        assert!(!map_obj_far_gate__698720(
            &lo,
            &hi,
            &[11.0, 1.0, 1.0],
            &[12.0, 2.0, 2.0]
        ));
        assert!(!map_obj_far_gate__698720(
            &lo,
            &hi,
            &[1.0; 3],
            &[2.0, -1.0, 2.0]
        ));
        assert!(!map_obj_far_gate__698720(
            &lo,
            &hi,
            &[1.0; 3],
            &[2.0, f32::NAN, 2.0]
        ));
    }
}

/// Depth sort key of `CWorldObjList__UpdateVisibility` 0x6838f0 (block 0x683972).
///
/// `((ny·y + nz·z) + nx·x + d) − radius`, folded in exactly that FADDP order at
/// x87 extended (f64 here), narrowed once by the final FSTP into the object's
/// `+0x78` slot. `plane` = `[nx,ny,nz,d]` (the camera depth plane globals at
/// 0xc7bcb0..bc).
pub fn visibility_sort_key__6838f0(plane: &[f32; 4], center: &[f32; 3], radius: f32) -> f32 {
    super::f64_to_f32(
        ((f64::from(plane[1]) * f64::from(center[1]) + f64::from(plane[2]) * f64::from(center[2]))
            + f64::from(plane[0]) * f64::from(center[0])
            + f64::from(plane[3]))
            - f64::from(radius),
    )
}

/// Near test of 0x6838f0 (block 0x683a48).
///
/// Squared distance from the object center to the camera reference, folded
/// `(dz² + dx²) + dy²` (that FADDP order), compared `< threshold` with
/// `TEST AH,0x5; JNP` — strictly less sets near; `>=` AND
/// **unordered (NaN) both give far/not-near**.
pub fn visibility_is_near__6838f0(center: &[f32; 3], reference: &[f32; 3], thresh: f32) -> bool {
    let dx = f64::from(center[0]) - f64::from(reference[0]);
    let dy = f64::from(center[1]) - f64::from(reference[1]);
    let dz = f64::from(center[2]) - f64::from(reference[2]);
    (dz * dz + dx * dx) + dy * dy < f64::from(thresh)
}

#[cfg(test)]
mod tests_update_visibility__6838f0 {
    use super::*;

    #[test]
    fn sort_key_is_plane_dot_minus_radius() {
        // Unit +z plane, d = -5: key = z - 5 - r.
        let plane = [0.0f32, 0.0, 1.0, -5.0];
        assert_eq!(
            visibility_sort_key__6838f0(&plane, &[9.0, 9.0, 12.0], 2.0),
            5.0
        );
    }

    #[test]
    fn sort_key_fold_order() {
        // Associativity-sensitive: (ny·y + nz·z) + nx·x + d, then − r.
        let plane = [1.0f32, 1e-8, -1.0, 0.0];
        let c = [3.0f32, 5.0, 3.0];
        let want = super::super::f64_to_f32(
            ((f64::from(plane[1]) * f64::from(c[1]) + f64::from(plane[2]) * f64::from(c[2]))
                + f64::from(plane[0]) * f64::from(c[0])
                + 0.0)
                - 0.25,
        );
        assert_eq!(visibility_sort_key__6838f0(&plane, &c, 0.25), want);
    }

    #[test]
    fn near_polarity() {
        let r = [0.0f32, 0.0, 0.0];
        assert!(visibility_is_near__6838f0(&[1.0, 0.0, 0.0], &r, 2.0));
        // Equality is NOT near (strict <).
        assert!(!visibility_is_near__6838f0(&[2.0, 0.0, 0.0], &r, 4.0));
        assert!(!visibility_is_near__6838f0(&[3.0, 0.0, 0.0], &r, 4.0));
        // NaN distance or threshold => far/not-near (unordered PF=1 path).
        assert!(!visibility_is_near__6838f0(&[f32::NAN, 0.0, 0.0], &r, 4.0));
        assert!(!visibility_is_near__6838f0(&[1.0, 0.0, 0.0], &r, f32::NAN));
    }
}

/// LOD near test of `WorldScene__CullAndBucketNodes` 0x6834e0 (block 0x683681).
///
/// The camera deltas are EACH narrowed to f32 (stock FSTPs every component to
/// the stack before `C3Vector__Set`), the squared magnitude is the hooked
/// 0x4549f0 kernel (f32 `x·x + y·y + z·z` — stock already calls our hook at
/// 0x6836b4), and the compare is `FCOMP; TEST AH,0x5; JP` against the LOD
/// threshold at `0x810178`: near = strictly less; `>=` AND unordered (NaN)
/// both give far.
pub fn cull_lod_is_near__6834e0(center: &[f32; 3], camera: &[f32; 3], thresh: f32) -> bool {
    let d = [
        super::f64_to_f32(f64::from(center[0]) - f64::from(camera[0])),
        super::f64_to_f32(f64::from(center[1]) - f64::from(camera[1])),
        super::f64_to_f32(f64::from(center[2]) - f64::from(camera[2])),
    ];
    super::vector::c3_vector__squared_magnitude__4549f0(&d) < thresh
}

#[cfg(test)]
mod tests_cull_and_bucket__6834e0 {
    use super::*;

    #[test]
    fn near_known_values() {
        let cam = [10.0f32, -4.0, 3.0];
        // delta = (3,4,0) => distSq 25.
        assert!(cull_lod_is_near__6834e0(&[13.0, 0.0, 3.0], &cam, 25.5));
        assert!(!cull_lod_is_near__6834e0(&[13.0, 0.0, 3.0], &cam, 24.5));
    }

    #[test]
    fn boundary_is_far() {
        // Equality is NOT near (strict <, JP falls through on ZF-only).
        let cam = [0.0f32, 0.0, 0.0];
        assert!(!cull_lod_is_near__6834e0(&[5.0, 0.0, 0.0], &cam, 25.0));
    }

    #[test]
    fn nan_is_far() {
        // Unordered (NaN) takes the JP => far, matching TEST AH,0x5.
        let cam = [0.0f32, 0.0, 0.0];
        assert!(!cull_lod_is_near__6834e0(
            &[f32::NAN, 0.0, 0.0],
            &cam,
            100.0
        ));
        assert!(!cull_lod_is_near__6834e0(&[1.0, 0.0, 0.0], &cam, f32::NAN));
    }

    #[test]
    fn deltas_narrowed_per_component() {
        // Each delta narrows to f32 before squaring (stock FSTPs each
        // component): the f64 subtract of two f32 is exact, so the
        // narrowed delta equals the plain f32 subtraction.
        let c = [16_777_217.0f32, 0.0, 0.0]; // 2^24 + 1 (rounds as f32)
        let cam = [1.0f32, 0.0, 0.0];
        let d = c[0] - cam[0];
        assert_eq!(
            cull_lod_is_near__6834e0(&c, &cam, d * d),
            d * d < d * d // false: strict
        );
        assert!(cull_lod_is_near__6834e0(&c, &cam, d * d * 1.001));
    }
}

/// Span-node timer tick for `SceneTransientMgr::TickAndReap` (@0x6b3890, 0x6b398a/0x6b39be).
///
/// `FLD t; FSUB delta; FSTP` — one narrow, and the expiry compare re-loads the
/// NARROWED store from memory (the caller does that re-read and the strict
/// `< 0` ordered compare itself).
pub fn transient_span_timer__6b3890(t: f32, delta: f32) -> f32 {
    super::f64_to_f32(f64::from(t) - f64::from(delta))
}

/// Lifetime timer tick for `SceneTransientMgr::TickAndReap`.
///
/// At the 0x6b39eb model arm and the 0x6b3a72 node arm, `FST` keeps the WIDE
/// difference on the stack, so the store narrows but the
/// `FCOMP 0.0; TEST AH,0x41; JP` expiry gate consumes the wide value — expired
/// iff `t − delta <= zero` ORDERED (both C0-set and C3-set fall through; NaN's
/// even parity skips). For a single f32−f32 subtract the wide and narrowed
/// compares agree (the exact difference is a multiple of 2^-149, never rounding
/// to zero from positive); the wide form is kept because it is what the
/// FST/FCOMP pair executes.
pub fn transient_life_timer__6b3890(t: f32, delta: f32, zero: f32) -> (f32, bool) {
    let wide = f64::from(t) - f64::from(delta);
    (super::f64_to_f32(wide), wide <= f64::from(zero))
}

#[cfg(test)]
mod tests_tick_and_reap_timers__6b3890 {
    use super::{transient_life_timer__6b3890 as life, transient_span_timer__6b3890 as span};

    #[test]
    fn span_narrows_once() {
        assert_eq!(span(1.0, 0.25).to_bits(), 0.75f32.to_bits());
        let want = (f64::from(0.1f32) - f64::from(0.3f32)) as f32;
        assert_eq!(span(0.1, 0.3).to_bits(), want.to_bits());
        assert!(span(f32::NAN, 0.1).is_nan());
    }

    #[test]
    fn life_expires_on_wide_zero_and_below() {
        let (stored, expired) = life(0.25, 0.25, 0.0);
        assert_eq!(stored.to_bits(), 0.0f32.to_bits());
        assert!(expired, "exact zero is expired (C3 falls through)");
        let (stored, expired) = life(0.25, 0.5, 0.0);
        assert_eq!(stored.to_bits(), (-0.25f32).to_bits());
        assert!(expired);
    }

    /// Any strictly positive remainder survives.
    ///
    /// Down to the smallest representable difference of two f32 inputs.
    #[test]
    fn life_positive_remainder_survives() {
        let (stored, expired) = life(1.0 + f32::EPSILON, 1.0, 0.0);
        assert!(stored > 0.0);
        assert!(!expired);
        // Minimal positive difference: one subnormal step.
        let t = f32::from_bits(2);
        let delta = f32::from_bits(1);
        let (stored, expired) = life(t, delta, 0.0);
        assert!(stored > 0.0);
        assert!(!expired);
    }

    #[test]
    fn life_nan_skips() {
        let (stored, expired) = life(f32::NAN, 0.1, 0.0);
        assert!(stored.is_nan());
        assert!(!expired);
    }
}

/// Bounds guard for `CWorld::CollectGeometryInBox` (@0x6aa8b0, 0x6aa92d–0x6aa9ae).
///
/// Negates the four offset-relative box coordinates (each `FSUB ofs; FCHS`
/// narrowed to the stack — except `b4`'s, whose WIDE value feeds the first
/// compare via `FST`) and applies the four NaN-PASSING guards: fail on
/// `−(b4−ofs) < lo` (wide) or `−(b3−ofs) < lo` (narrowed), both ordered; fail on
/// `−(b1−ofs) >= hi` or `−(b0−ofs) >= hi` (narrowed, ordered) — every unordered
/// compare passes, exactly the stock `TEST AH,0x5; JNP` / `TEST AH,0x1; JZ`
/// parities. Returns the narrowed negated coords `(v0, v1, v3, v4)` for the cell
/// converts.
pub fn collect_box_guard__6aa8b0(
    b0: f32,
    b1: f32,
    b3: f32,
    b4: f32,
    ofs: f32,
    lo: f32,
    hi: f32,
) -> Option<(f32, f32, f32, f32)> {
    let ofs = f64::from(ofs);
    let v3 = super::f64_to_f32(-(f64::from(b3) - ofs));
    let v4w = -(f64::from(b4) - ofs);
    let v4 = super::f64_to_f32(v4w);
    let v0 = super::f64_to_f32(-(f64::from(b0) - ofs));
    let v1 = super::f64_to_f32(-(f64::from(b1) - ofs));
    if v4w < f64::from(lo) {
        return None;
    }
    if f64::from(v3) < f64::from(lo) {
        return None;
    }
    if f64::from(v1) >= f64::from(hi) {
        return None;
    }
    if f64::from(v0) >= f64::from(hi) {
        return None;
    }
    Some((v0, v1, v3, v4))
}

/// Cell-grid convert (0x6aa9b4–0x6aaa11).
///
/// `scale · v` narrows once (FSTP/FLD round-trip), the bias subtracts WIDE into
/// the FISTP — the shared biased-FISTP idiom (round-to-nearest-even;
/// NaN/overflow store the integer indefinite).
pub fn collect_cell_index__6aa8b0(v: f32, scale: f32, bias: f32) -> i32 {
    let p = super::f64_to_f32(f64::from(scale) * f64::from(v));
    fistp_round_ties_even(f64::from(p) - f64::from(bias))
}

#[cfg(test)]
mod tests_collect_geometry_in_box__6aa8b0 {
    use super::{collect_box_guard__6aa8b0 as guard, collect_cell_index__6aa8b0 as cell};

    #[test]
    fn guard_polarity_ordered() {
        // ofs 100: box {b0,b1,b3,b4} -> v = 100 - b. lo=0, hi=64.
        // All four v in [0, 64): pass.
        let got = guard(90.0, 80.0, 70.0, 60.0, 100.0, 0.0, 64.0).unwrap();
        assert_eq!(got, (10.0, 20.0, 30.0, 40.0));
        // v4 < lo -> fail (b4 > ofs).
        assert!(guard(90.0, 80.0, 70.0, 101.0, 100.0, 0.0, 64.0).is_none());
        // v3 < lo -> fail.
        assert!(guard(90.0, 80.0, 101.0, 60.0, 100.0, 0.0, 64.0).is_none());
        // v1 >= hi -> fail (b1 far below ofs).
        assert!(guard(90.0, 30.0, 70.0, 60.0, 100.0, 0.0, 64.0).is_none());
        // v0 >= hi -> fail; boundary == hi also fails.
        assert!(guard(36.0, 80.0, 70.0, 60.0, 100.0, 0.0, 64.0).is_none());
    }

    /// Every guard PASSES on NaN (stock's unordered parities fall through).
    #[test]
    fn guard_nan_passes() {
        let got = guard(f32::NAN, f32::NAN, f32::NAN, f32::NAN, 100.0, 0.0, 64.0);
        let (v0, v1, v3, v4) = got.unwrap();
        assert!(v0.is_nan() && v1.is_nan() && v3.is_nan() && v4.is_nan());
    }

    #[test]
    fn cell_biased_round() {
        // scale 8, bias 0.5: v=1.0 -> 8-0.5=7.5 ties to even 8.
        assert_eq!(cell(1.0, 8.0, 0.5), 8);
        // v=0.8125 -> 6.5-0.5 = 6.0 -> 6.
        assert_eq!(cell(0.8125, 8.0, 0.5), 6);
        // NaN -> integer indefinite.
        assert_eq!(cell(f32::NAN, 8.0, 0.5), i32::MIN);
    }
}

/// WMO-hit world lerp (0x6a2652..0x6a26a1).
///
/// `hit[i] = (end[i] - start[i]) × frac + start[i]`, with each lane's x87
/// narrowing pinned exactly — X narrows the delta×frac product before adding
/// start, Y folds the whole lane wide, Z narrows the delta (`FSTP m32`) before
/// the wide product+add.
pub fn trace_group_lerp__6a2600(start: [f32; 3], end: [f32; 3], frac: f32) -> [f32; 3] {
    let f = f64::from(frac);
    // X: delta×frac narrowed (FSTP m32), then + start narrowed.
    let px = super::f64_to_f32((f64::from(end[0]) - f64::from(start[0])) * f);
    let x = super::f64_to_f32(f64::from(px) + f64::from(start[0]));
    // Y: fully wide fold, one narrow.
    let y = super::f64_to_f32((f64::from(end[1]) - f64::from(start[1])) * f + f64::from(start[1]));
    // Z: delta narrowed (FSTP m32) before the wide product + start.
    let dz = super::f64_to_f32(f64::from(end[2]) - f64::from(start[2]));
    let z = super::f64_to_f32(f64::from(dz) * f + f64::from(start[2]));
    [x, y, z]
}

#[cfg(test)]
mod tests_trace_group_lerp__6a2600 {
    use super::trace_group_lerp__6a2600 as lerp;

    #[test]
    fn lerps_each_lane() {
        let h = lerp([0.0, 1.0, 2.0], [10.0, 3.0, 6.0], 0.5);
        assert_eq!(h, [5.0, 2.0, 4.0]);
    }

    #[test]
    fn frac_endpoints() {
        let s = [1.5, -2.0, 3.25];
        let e = [4.0, 8.0, -1.0];
        assert_eq!(lerp(s, e, 0.0), s);
        // frac 1: each lane returns end within one f32 ulp of the fold.
        let h = lerp(s, e, 1.0);
        for i in 0..3 {
            assert!((h[i] - e[i]).abs() <= f32::EPSILON * 8.0);
        }
    }
}

/// Terrain-trace world→grid recentre (0x69c32b..0x69c360).
///
/// `-(coord − c)` with the single `FSTP m32` narrow, `c` = the world-origin
/// global.
pub fn terrain_recentre__69c320(coord: f32, center: f32) -> f32 {
    super::f64_to_f32(-(f64::from(coord) - f64::from(center)))
}

/// Terrain-trace grid delta (0x69c363..0x69c372): `a − b`, one narrow.
pub fn terrain_delta__69c320(a: f32, b: f32) -> f32 {
    super::f64_to_f32(f64::from(a) - f64::from(b))
}

/// Terrain-trace cell index (0x69c375..0x69c3d2).
///
/// `round(narrow(scale × val) − bias)`. The product narrows (`FSTP m32`) before
/// the wide bias subtract and the round-ties-even `FISTP`.
pub fn terrain_cell__69c320(scale: f32, val: f32, bias: f32) -> i32 {
    let prod = super::f64_to_f32(f64::from(scale) * f64::from(val));
    fistp_round_ties_even(f64::from(prod) - f64::from(bias))
}

#[cfg(test)]
mod tests_terrain_trace__69c320 {
    use super::{
        terrain_cell__69c320 as cell, terrain_delta__69c320 as delta,
        terrain_recentre__69c320 as recentre,
    };

    #[test]
    fn recentre_negates_offset() {
        assert_eq!(recentre(10.0, 4.0), -6.0);
        assert_eq!(recentre(4.0, 4.0).to_bits(), (-0.0f32).to_bits());
    }

    #[test]
    fn delta_subtracts() {
        assert_eq!(delta(3.0, 1.0), 2.0);
    }

    #[test]
    fn cell_rounds_ties_even_after_bias() {
        // scale 2, val 3, bias 0.5 -> round(6 - 0.5) = round(5.5) = 6 (even).
        assert_eq!(cell(2.0, 3.0, 0.5), 6);
        // round(4.5) -> 4 (ties to even).
        assert_eq!(cell(1.0, 5.0, 0.5), 4);
    }
}

/// Anim-record scrolled parameter (0x71204a..0x712058).
///
/// `ftol((counter − base) × scale) + offset`, where `counter − base` is an i32
/// subtraction `FILDed` into the x87 product (80-bit) and truncated by `__ftol`,
/// then the i32 offset is added. Returns the scrolled i32.
pub fn anim_scroll_param__711fe0(delta: i32, scale: f32, offset: i32) -> i32 {
    let t = super::misc::ftol__40a2b0(f64::from(delta) * f64::from(scale)) as i32;
    t.wrapping_add(offset)
}

#[cfg(test)]
mod tests_anim_scroll_param__711fe0 {
    use super::anim_scroll_param__711fe0 as scroll;

    #[test]
    fn truncates_then_adds_offset() {
        // (10) * 2.5 = 25.0 -> trunc 25, + 3 = 28.
        assert_eq!(scroll(10, 2.5, 3), 28);
        // Truncation toward zero: 2.9 -> 2.
        assert_eq!(scroll(1, 2.9, 0), 2);
        assert_eq!(scroll(-1, 2.9, 0), -2);
    }
}

/// Depth-bucket projection cache (0x681a46..0x681a6e).
///
/// The 4-term plane dot `m60·p60 + m64·p64 + m5c·p5c + c − w`, folded wide with
/// one narrow at the `FSTP` into the object's `+0x78` slot.
// The eight scalars are the matrix row and point components the original passes
// individually; the count is a fact about that call.
#[allow(clippy::too_many_arguments)]
pub fn depth_bucket_project__681a40(
    m60: f32,
    m64: f32,
    m5c: f32,
    c: f32,
    w: f32,
    p5c: f32,
    p60: f32,
    p64: f32,
) -> f32 {
    super::f64_to_f32(
        f64::from(m60) * f64::from(p60)
            + f64::from(m64) * f64::from(p64)
            + f64::from(m5c) * f64::from(p5c)
            + f64::from(c)
            - f64::from(w),
    )
}

/// Depth-bucket index (0x681a71..0x681ab2).
///
/// The second plane dot stays 80-bit (never stored), so `scale × proj2` narrows
/// only after the full wide fold, then `round(narrow − bias)` picks the
/// 0x20-slot bucket.
// The ten scalars are the matrix row, point components, scale and bias the
// original passes individually; the count is a fact about that call.
#[allow(clippy::too_many_arguments)]
pub fn depth_bucket_index__681a40(
    m60: f32,
    m64: f32,
    m5c: f32,
    c: f32,
    w: f32,
    p5c: f32,
    p60: f32,
    p64: f32,
    scale: f32,
    bias: f32,
) -> i32 {
    let proj2 = f64::from(m60) * f64::from(p60)
        + f64::from(m64) * f64::from(p64)
        + f64::from(m5c) * f64::from(p5c)
        + f64::from(c)
        - f64::from(w);
    let prod = super::f64_to_f32(f64::from(scale) * proj2);
    fistp_round_ties_even(f64::from(prod) - f64::from(bias))
}

#[cfg(test)]
mod tests_depth_bucket__681a40 {
    use super::{depth_bucket_index__681a40 as idx, depth_bucket_project__681a40 as proj};

    #[test]
    fn projects_plane_dot() {
        // 1*2 + 1*3 + 1*4 + 10 - 5 = 14.
        assert_eq!(proj(1.0, 1.0, 1.0, 10.0, 5.0, 4.0, 2.0, 3.0), 14.0);
    }

    #[test]
    fn index_rounds_after_scale_and_bias() {
        // proj2 = 2+3+4+0-0 = 9; *2 = 18; -0.5 = 17.5 -> round-ties-even 18.
        assert_eq!(idx(1.0, 1.0, 1.0, 0.0, 0.0, 4.0, 2.0, 3.0, 2.0, 0.5), 18);
    }
}
