//! `gx` family kernels.
#![allow(
    non_snake_case,
    // intentional NaN-reject via `!(a >= b)`
    // (differs from `a < b` on NaN), bit-exact source constants kept verbatim,
    // and ABI-dictated parameter counts.
    clippy::neg_cmp_op_on_partial_ord,
    clippy::excessive_precision,
    clippy::too_many_arguments
)]

/// Returns the selected 16-float (4x4) matrix unchanged.
///
/// The original copies the current top-of-stack matrix into the destination
/// verbatim.
pub fn c_gx_device__copy_current_matrix__592730(src: &[f32; 16]) -> [f32; 16] {
    *src
}

#[cfg(test)]
mod tests_c_gx_device__copy_current_matrix__592730 {
    use super::c_gx_device__copy_current_matrix__592730 as copy;

    #[test]
    fn identity_roundtrips() {
        let id: [f32; 16] = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        assert_eq!(copy(&id), id);
    }

    #[test]
    fn arbitrary_is_verbatim() {
        let m: [f32; 16] = [
            1.5, -2.0, 3.25, 4.0, 5.0, 6.5, 7.0, 8.0, 9.0, 10.0, 11.5, 12.0, 13.0, 14.0, 15.0, 16.0,
        ];
        assert_eq!(copy(&m), m);
        for i in 0..16 {
            assert_eq!(copy(&m)[i].to_bits(), m[i].to_bits());
        }
    }

    #[test]
    fn preserves_negative_zero_and_specials() {
        let m: [f32; 16] = [
            -0.0,
            0.0,
            f32::INFINITY,
            f32::NEG_INFINITY,
            1.0,
            2.0,
            3.0,
            4.0,
            5.0,
            6.0,
            7.0,
            8.0,
            9.0,
            10.0,
            11.0,
            12.0,
        ];
        let out = copy(&m);
        assert_eq!(out[0].to_bits(), (-0.0f32).to_bits());
        assert!(out[2].is_infinite() && out[2] > 0.0);
        assert!(out[3].is_infinite() && out[3] < 0.0);
    }
}

/// Computes `m = max(baseline, rgb[0], rgb[1], rgb[2])`.
///
/// When `m >= threshold`, normalizes each of `rgb` and `baseline` by `1/m`,
/// clamps each normalized value to `[0, 1]`, maps it to `[0, 255]` (with a
/// `+0.5` round bias on the in-range path, then truncating toward zero), and
/// packs the four bytes into one dword as
/// `baseline<<24 | rgb0<<16 | rgb1<<8 | rgb2`. Returns `(packed, m)`;
/// `packed` is `0` when `m < threshold`.
///
/// The clamp sentinels (`0.0`/`255.0`) and the in-range remap `v*255.0 + 0.5`
/// are the reference `.rdata` constants; `threshold` is injected by the caller.
pub fn gx_light_clamp_color_component_to_byte__71ca80(
    rgb: &[f32; 3],
    baseline: f32,
    threshold: f32,
) -> (u32, f32) {
    let mut m = baseline;
    if m < rgb[0] {
        m = rgb[0];
    }
    if m < rgb[1] {
        m = rgb[1];
    }
    if m < rgb[2] {
        m = rgb[2];
    }

    if !(threshold <= m) {
        return (0, m);
    }

    let inv = 1.0_f32 / m;
    let a = inv * baseline;
    let r = inv * rgb[0];
    let g = inv * rgb[1];
    let b = inv * rgb[2];

    let byte = |v: f32| -> u32 {
        let mapped = if !(0.0 < v) {
            0.0_f32
        } else if v < 1.0 {
            v * 255.0 + 0.5
        } else {
            255.0_f32
        };
        (mapped as i32 as u32) & 0xff
    };

    let packed = (byte(a) << 24) | (byte(r) << 16) | (byte(g) << 8) | byte(b);
    (packed, m)
}

#[cfg(test)]
mod tests_gx_light_clamp_color_component_to_byte__71ca80 {
    use super::gx_light_clamp_color_component_to_byte__71ca80 as clamp;

    const THRESH: f32 = 1.0e-5;

    #[test]
    fn below_threshold_packs_zero_but_reports_max() {
        let (p, m) = clamp(&[1.0e-7, 2.0e-7, 3.0e-7], 5.0e-7, THRESH);
        assert_eq!(p, 0);
        assert!((m - 5.0e-7).abs() < 1e-12);
    }

    #[test]
    fn max_is_the_componentwise_maximum() {
        let (_p, m) = clamp(&[0.2, 0.9, 0.4], 0.1, THRESH);
        assert_eq!(m.to_bits(), 0.9f32.to_bits());
        let (_p, m2) = clamp(&[0.2, 0.3, 0.4], 0.95, THRESH);
        assert_eq!(m2.to_bits(), 0.95f32.to_bits());
    }

    #[test]
    fn brightest_channel_saturates_to_255() {
        let (p, m) = clamp(&[1.0, 0.5, 0.25], 0.0, THRESH);
        assert!((m - 1.0).abs() < 1e-7);
        let a = (p >> 24) & 0xff;
        let r = (p >> 16) & 0xff;
        let g = (p >> 8) & 0xff;
        let b = p & 0xff;
        assert_eq!(a, 0);
        assert_eq!(r, 255);
        assert_eq!(g, (0.5f32 * 255.0 + 0.5) as i32 as u32);
        assert_eq!(b, (0.25f32 * 255.0 + 0.5) as i32 as u32);
    }

    #[test]
    fn pack_layout_matches_argb_order() {
        let (p, _m) = clamp(&[0.5, 0.25, 0.125], 1.0, THRESH);
        let a = (p >> 24) & 0xff;
        let r = (p >> 16) & 0xff;
        let g = (p >> 8) & 0xff;
        let b = p & 0xff;
        assert_eq!(a, 255);
        assert_eq!(r, (0.5f32 * 255.0 + 0.5) as i32 as u32);
        assert_eq!(g, (0.25f32 * 255.0 + 0.5) as i32 as u32);
        assert_eq!(b, (0.125f32 * 255.0 + 0.5) as i32 as u32);
    }

    #[test]
    fn negative_lane_clamps_to_zero() {
        let (p, _m) = clamp(&[-1.0, 0.5, 1.0], 0.0, THRESH);
        let r = (p >> 16) & 0xff;
        assert_eq!(r, 0);
    }

    #[test]
    fn all_bytes_in_range() {
        for &base in &[0.0f32, 0.3, 1.0, 2.5] {
            for &x in &[0.0f32, 0.1, 0.6, 1.0, 3.0] {
                let (p, _m) = clamp(&[x, x * 0.5, x * 0.25], base, THRESH);
                // Deterministic: identical inputs always pack to the same dword.
                let (p2, _m2) = clamp(&[x, x * 0.5, x * 0.25], base, THRESH);
                assert_eq!(p, p2);
            }
        }
    }
}

/// Returns whether a plane-array slot must be rewritten.
///
/// The host stores the new value only when it differs (strict ordered
/// inequality) from the cached one. The original mirrors the x87 `FCOMP`/`JNP`
/// skip-when-equal test, so an unordered (NaN) comparison takes the skip path
/// and is reported as no-change.
#[allow(clippy::double_comparisons)]
pub fn c_gx_device__set_plane_float__593770(current: f32, value: f32) -> bool {
    current < value || current > value
}

#[cfg(test)]
mod c_gx_device__set_plane_float__593770_tests {
    use super::c_gx_device__set_plane_float__593770 as should_store;

    #[test]
    fn equal_is_no_change() {
        assert!(!should_store(1.5_f32, 1.5_f32));
        assert!(!should_store(0.0_f32, 0.0_f32));
        // +0.0 == -0.0 in IEEE ordered compare -> no change.
        assert!(!should_store(0.0_f32, -0.0_f32));
    }

    #[test]
    fn distinct_values_change() {
        assert!(should_store(1.0_f32, 2.0_f32));
        assert!(should_store(2.0_f32, 1.0_f32));
        assert!(should_store(-3.25_f32, 3.25_f32));
    }

    #[test]
    fn unordered_takes_skip_path() {
        let nan = f32::NAN;
        assert!(!should_store(nan, 1.0_f32));
        assert!(!should_store(1.0_f32, nan));
        assert!(!should_store(nan, nan));
    }

    #[test]
    fn antisymmetry() {
        for &(a, b) in &[(0.0_f32, 1.0_f32), (5.0, -5.0), (1e9, 1e-9)] {
            assert_eq!(should_store(a, b), should_store(b, a));
        }
    }
}

/// Packs the device fog gradient color (mode-1 path) into one `0xAARRGGBB` dword.
///
/// With the alpha forced to `0xff`. `c0`/`c1`/`c2` are the raw `r`/`g`/`b`
/// channels read from `device+0x190`/`+0x194`/`+0x198`. Each channel is clamped
/// to `[0, 1]` (the `!(0.0 < v)` lower test snaps `<= 0` and `NaN` to `0`; the
/// `v < 1.0` upper test maps in-range values by `v * 255.0 + 0.5` and snaps
/// `>= 1.0` to `255.0`), then truncated toward zero through the reference
/// `__ftol` shim (NOT an `as` cast) and masked to a byte.
///
/// Layout: `r` occupies bits 16..24, `g` bits 8..16, `b` bits 0..8 — the
/// byte-lane order matches the original's `or edi,0xffffff00; shl 8` byte-shift
/// chain.
pub fn fog_pack_color_argb__70baf0(c0: f32, c1: f32, c2: f32) -> u32 {
    0xff00_0000
        | (fog_clamp_byte__70baf0(c0) << 16)
        | (fog_clamp_byte__70baf0(c1) << 8)
        | fog_clamp_byte__70baf0(c2)
}

/// Clamps `v` to `[0, 1]`.
///
/// Maps the in-range arm by `v * 255.0 + 0.5`, and truncates toward zero via
/// the reference `__ftol` (`crate::math::misc`), returning the low byte.
/// Mirrors the proven clamp used by
/// `gx_light_clamp_color_component_to_byte__71ca80`; the compare polarities are
/// the x87 `fcomp`/`test ah,0x41`/`jp` (lower) and `test ah,0x1`/`jne` (upper)
/// tests, so a `NaN` lane snaps to `0`. The in-range remap runs in `f32` whereas
/// the original keeps an 80-bit x87 intermediate before truncation, so the
/// result can differ by at most one byte LSB — the project's beat-x87 design.
pub fn fog_clamp_byte__70baf0(v: f32) -> u32 {
    let mapped = if !(0.0_f32 < v) {
        0.0_f32
    } else if v < 1.0_f32 {
        v * 255.0_f32 + 0.5_f32
    } else {
        255.0_f32
    };
    (crate::math::misc::ftol__40a2b0(f64::from(mapped)) as u32) & 0xff
}

#[cfg(test)]
mod tests_fog_pack_color_argb__70baf0 {
    use super::{fog_clamp_byte__70baf0 as byte, fog_pack_color_argb__70baf0 as pack};

    #[test]
    fn alpha_is_forced_to_ff() {
        let p = pack(0.0, 0.0, 0.0);
        assert_eq!((p >> 24) & 0xff, 0xff);
    }

    #[test]
    fn channel_byte_lanes_are_rgb_order() {
        // r=1.0 (saturates 255), g=0.0 (snaps 0), b in-range.
        let p = pack(1.0, 0.0, 0.5);
        assert_eq!((p >> 16) & 0xff, 255);
        assert_eq!((p >> 8) & 0xff, 0);
        assert_eq!(p & 0xff, byte(0.5));
    }

    #[test]
    fn lower_clamp_snaps_nonpositive_and_nan() {
        assert_eq!(byte(0.0), 0);
        assert_eq!(byte(-1.0), 0);
        assert_eq!(byte(f32::NAN), 0);
    }

    #[test]
    fn upper_clamp_saturates_at_or_above_one() {
        assert_eq!(byte(1.0), 255);
        assert_eq!(byte(2.0), 255);
        assert_eq!(byte(f32::INFINITY), 255);
    }

    #[test]
    fn in_range_uses_round_bias_then_truncate() {
        // v*255+0.5 then truncate toward zero (ftol), masked to a byte.
        assert_eq!(byte(0.5), ((0.5f32 * 255.0 + 0.5) as i32 as u32) & 0xff);
        assert_eq!(byte(0.25), (0.25f32 * 255.0 + 0.5) as i32 as u32 & 0xff);
        assert_eq!(byte(0.999), (0.999f32 * 255.0 + 0.5) as i32 as u32 & 0xff);
    }

    #[test]
    fn pack_is_deterministic() {
        for r in [0.0f32, 0.3, 1.0, 2.0] {
            for g in [-1.0f32, 0.1, 0.7] {
                for b in [0.0f32, 0.5, 1.5] {
                    assert_eq!(pack(r, g, b), pack(r, g, b));
                }
            }
        }
    }
}

/// `CGxDevice::SetViewport` (0x592530) dedup predicate.
///
/// Returns `true` when the six incoming viewport floats differ from the
/// currently-cached rect, so the caller should store them and raise the
/// viewport-dirty flag. The cached rect lives at `this+0xf38..0xf4c` and the
/// dirty flag at `this+0xf34`.
///
/// Bit-faithful to the stock x87 dedup chain (six `FLD [this+0xf..]; FCOMP
/// [stack]; FNSTSW; TEST AH,0x44; JP write`). `TEST AH,0x44; JP` fires the write
/// on *greater*, *less*, OR *unordered* (a NaN operand) — anything that is not
/// exact equality — so a NaN counts as a change. Rust `!=` reproduces this
/// exactly (`NaN != x` is always true, including `NaN != NaN`); do NOT special-
/// case NaN. Short-circuits on the first differing lane, matching the stock jump.
pub fn c_gx_device__set_viewport__592530(cur: &[f32; 6], incoming: &[f32; 6]) -> bool {
    cur.iter().zip(incoming).any(|(c, n)| *c != *n)
}

#[cfg(test)]
mod tests_c_gx_device__set_viewport__592530 {
    use super::c_gx_device__set_viewport__592530 as changed;

    #[test]
    fn identical_rect_is_no_change() {
        let v = [0.0, 0.0, 1.0, 1.0, 0.0, 1.0];
        assert!(!changed(&v, &v));
    }

    #[test]
    fn any_lane_difference_is_a_change() {
        let cur = [0.0, 0.0, 1.0, 1.0, 0.0, 1.0];
        for i in 0..6 {
            let mut inc = cur;
            inc[i] += 0.25;
            assert!(changed(&cur, &inc), "lane {i}");
        }
    }

    #[test]
    fn nan_incoming_is_a_change() {
        let cur = [0.0, 0.0, 1.0, 1.0, 0.0, 1.0];
        for i in 0..6 {
            let mut inc = cur;
            inc[i] = f32::NAN;
            assert!(changed(&cur, &inc), "nan in lane {i}");
        }
    }

    #[test]
    fn nan_cached_reads_as_change_even_against_same_bits() {
        // Stock FCOMP is unordered on either operand, so a cached NaN always
        // reads as changed — even when the incoming value is also NaN.
        let cur = [f32::NAN, 0.0, 1.0, 1.0, 0.0, 1.0];
        assert!(changed(&cur, &cur));
    }
}

/// `GxSetViewport` (0x58af60) validate+clamp predicate.
///
/// Validates a normalized viewport box and snaps its near-edges to 0/1,
/// returning the sanitized box or `None` (⇒ caller raises
/// STORM_ERROR_INVALID_PARAMETER). `near`/`far` are the device depth-range
/// globals (`DAT_007ffd74`=0.0, `_DAT_007ff9d8`=1.0), read live.
///
/// Every predicate is ordered — the stock x87 `FCOMP` chain routes greater/less/
/// unordered to the error path — so plain IEEE `<`/`<=` reproduce the NaN→reject
/// polarity exactly (an ordered compare is false on NaN). Pass requires
/// `x0<x1 && y0<y1 && minZ<=maxZ && near<=minZ && maxZ<=far`; then snaps
/// `x0/y0<=near → 0.0` and `x1/y1>=far → 1.0` (matching the stock
/// `if (x0 <= near) x0 = 0; if (far <= x1) x1 = 1; …`).
#[allow(clippy::too_many_arguments)]
pub fn gx_set_viewport__58af60(
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
    min_z: f32,
    max_z: f32,
    near: f32,
    far: f32,
) -> Option<[f32; 6]> {
    if x0 < x1 && y0 < y1 && min_z <= max_z && near <= min_z && max_z <= far {
        Some([
            if x0 <= near { 0.0 } else { x0 },
            if far <= x1 { 1.0 } else { x1 },
            if y0 <= near { 0.0 } else { y0 },
            if far <= y1 { 1.0 } else { y1 },
            min_z,
            max_z,
        ])
    } else {
        None
    }
}

#[cfg(test)]
mod tests_gx_set_viewport__58af60 {
    use super::gx_set_viewport__58af60 as vp;
    const NEAR: f32 = 0.0;
    const FAR: f32 = 1.0;

    #[test]
    fn valid_interior_box_passes_unchanged() {
        // Interior box (no edges to snap) passes as-is.
        assert_eq!(
            vp(0.25, 0.75, 0.25, 0.75, 0.1, 0.9, NEAR, FAR),
            Some([0.25, 0.75, 0.25, 0.75, 0.1, 0.9])
        );
    }

    #[test]
    fn near_edges_snap_to_0_and_1() {
        // x0<=near → 0, x1>=far → 1 (same for y); depth left intact.
        assert_eq!(
            vp(-0.5, 1.5, 0.0, 1.0, 0.0, 1.0, NEAR, FAR),
            Some([0.0, 1.0, 0.0, 1.0, 0.0, 1.0])
        );
    }

    #[test]
    fn degenerate_rect_rejected() {
        assert_eq!(vp(0.5, 0.5, 0.0, 1.0, 0.0, 1.0, NEAR, FAR), None); // x0==x1
        assert_eq!(vp(0.75, 0.25, 0.0, 1.0, 0.0, 1.0, NEAR, FAR), None); // x0>x1
        assert_eq!(vp(0.0, 1.0, 0.6, 0.4, 0.0, 1.0, NEAR, FAR), None); // y0>y1
    }

    #[test]
    fn depth_out_of_range_rejected() {
        assert_eq!(vp(0.0, 1.0, 0.0, 1.0, 0.5, 0.4, NEAR, FAR), None); // minZ>maxZ
        assert_eq!(vp(0.0, 1.0, 0.0, 1.0, -0.1, 0.9, NEAR, FAR), None); // minZ<near
        assert_eq!(vp(0.0, 1.0, 0.0, 1.0, 0.1, 1.1, NEAR, FAR), None); // maxZ>far
    }

    #[test]
    fn minz_equals_maxz_passes() {
        // The stock predicate is `minZ<=maxZ` (equal allowed).
        assert_eq!(
            vp(0.25, 0.75, 0.25, 0.75, 0.5, 0.5, NEAR, FAR),
            Some([0.25, 0.75, 0.25, 0.75, 0.5, 0.5])
        );
    }

    #[test]
    fn any_nan_rejects() {
        let nan = f32::NAN;
        for i in 0..6 {
            let mut a = [0.25f32, 0.75, 0.25, 0.75, 0.4, 0.6];
            a[i] = nan;
            assert_eq!(
                vp(a[0], a[1], a[2], a[3], a[4], a[5], NEAR, FAR),
                None,
                "nan arg {i}"
            );
        }
    }
}

/// Unsigned lexicographic byte compare of two `len`-byte blocks.
///
/// The exact `REPE CMPSB; SBB EAX,EAX; SBB EAX,-1` idiom the 1.12 client
/// inlines for its render-sort key blocks (e.g. `RenderBatch::CompareSortKey`
/// @ 0x70a710): the first differing byte decides (`-1` when `a`'s byte is
/// smaller unsigned, `+1` when larger), all bytes equal returns `0`. Classic
/// `memcmp` sign semantics.
///
/// Raw-pointer loop with unaligned reads — never a `&[u8]` over game memory
/// (see the repo rule: `slice::from_raw_parts` debug-asserts alignment and
/// non-null even for len 0 on lazily-allocated buffers).
///
/// # Safety
///
/// `a` and `b` must each point to at least `len` readable bytes.
pub unsafe fn lex_cmp(a: *const u8, b: *const u8, len: usize) -> i32 {
    for i in 0..len {
        // SAFETY: `a + i` is within the `len` readable bytes the caller guarantees.
        let pa = unsafe { a.add(i) };
        // SAFETY: as above.
        let ba = unsafe { pa.read_unaligned() };
        // SAFETY: `b + i` is within the `len` readable bytes the caller guarantees.
        let pb = unsafe { b.add(i) };
        // SAFETY: as above.
        let bb = unsafe { pb.read_unaligned() };
        if ba != bb {
            return if ba < bb { -1 } else { 1 };
        }
    }
    0
}

#[cfg(test)]
mod tests_lex_cmp {
    use super::lex_cmp;

    /// Independent oracle.
    ///
    /// Rust's slice `Ord` is unsigned byte lexicographic; on equal-length
    /// inputs it matches `memcmp` sign semantics exactly.
    fn oracle(a: &[u8], b: &[u8]) -> i32 {
        match a.cmp(b) {
            core::cmp::Ordering::Less => -1,
            core::cmp::Ordering::Equal => 0,
            core::cmp::Ordering::Greater => 1,
        }
    }

    fn cmp(a: &[u8], b: &[u8]) -> i32 {
        assert_eq!(a.len(), b.len());
        // SAFETY: both pointers address `len` bytes of the live slices.
        unsafe { lex_cmp(a.as_ptr(), b.as_ptr(), a.len()) }
    }

    #[test]
    fn equal_blocks_return_zero() {
        let a = [0u8, 1, 2, 3, 0x7f, 0x80, 0xff, 42, 0, 0, 0, 9];
        assert_eq!(cmp(&a, &a), 0);
        assert_eq!(cmp(&[], &[]), 0); // len 0: REPE with ECX=0 never compares
    }

    #[test]
    fn first_differing_byte_decides() {
        // Difference in the FIRST byte wins even when later bytes reverse it.
        let a = [1u8, 0xff, 0xff, 0xff];
        let b = [2u8, 0x00, 0x00, 0x00];
        assert_eq!(cmp(&a, &b), -1);
        assert_eq!(cmp(&b, &a), 1);
        // Difference only in the LAST byte still decides.
        let c = [7u8, 7, 7, 7];
        let d = [7u8, 7, 7, 8];
        assert_eq!(cmp(&c, &d), -1);
        assert_eq!(cmp(&d, &c), 1);
    }

    #[test]
    fn compare_is_unsigned() {
        // 0x80 (128) > 0x7f (127) unsigned; a signed byte compare would flip this.
        assert_eq!(cmp(&[0x80], &[0x7f]), 1);
        assert_eq!(cmp(&[0x7f], &[0x80]), -1);
        assert_eq!(cmp(&[0x00], &[0xff]), -1);
        assert_eq!(cmp(&[0xff], &[0x00]), 1);
    }

    #[test]
    fn matches_memcmp_oracle_exhaustively() {
        // Every pair of 3-byte blocks over a boundary-heavy alphabet.
        let alphabet = [0x00u8, 0x01, 0x7f, 0x80, 0xff];
        let mut blocks = Vec::new();
        for &x in &alphabet {
            for &y in &alphabet {
                for &z in &alphabet {
                    blocks.push([x, y, z]);
                }
            }
        }
        for a in &blocks {
            for b in &blocks {
                assert_eq!(cmp(a, b), oracle(a, b), "a={a:?} b={b:?}");
            }
        }
    }

    #[test]
    fn unaligned_pointers_are_fine() {
        // Read from deliberately odd addresses inside a larger buffer.
        let buf: [u8; 16] = [9, 1, 2, 3, 4, 9, 1, 2, 3, 5, 0, 0, 0, 0, 0, 0];
        // SAFETY: offset 1 starts an in-bounds 4-byte window of `buf`.
        let pa = unsafe { buf.as_ptr().add(1) };
        // SAFETY: offset 6 starts an in-bounds 4-byte window of `buf`.
        let pb = unsafe { buf.as_ptr().add(6) };
        // SAFETY: both windows address 4 readable bytes.
        let r = unsafe { lex_cmp(pa, pb, 4) };
        assert_eq!(r, -1); // [1,2,3,4] < [1,2,3,5]
    }
}

/// `CGxDeviceD3d::IXformSetProjection` (0x5a11d0) conversion core.
///
/// Turns the engine's GL-convention row-major projection into D3D convention.
/// Three stages, transcribed from the stock x87 body (computed in `f64`
/// tracking the 80-bit intermediates, narrowed at the exact points stock
/// stores `f32`):
///
/// 1. **eps-guarded w-normalize** — unless `|m23 − 1| < eps` or `|m23| < eps`
///    (`m23 = m[11]`), every element is scaled by `1/m23`. Both stock guards
///    are `FABS; FCOMP eps; FNSTSW; TEST AH,5; JNP skip`: the skip fires only
///    on an ORDERED `|x| < eps` (C0 alone ⇒ odd parity); a NaN `m23` compares
///    unordered (C3=C2=C0 ⇒ even parity) and PROCEEDS to the normalize. Rust's
///    ordered `<` is false on NaN, so `!(|x| < eps)` reproduces that polarity.
/// 2. **depth-range remap** `[-1,1] → [0,1]` on `m22 = m[10]` / `m32 = m[14]`.
///    The select is `FCOMP m33, 0.0; TEST AH,0x44; JP ortho` (`m33 = m[15]`):
///    only ORDERED equality (C3 alone ⇒ odd parity) falls through to the
///    perspective arm; greater, less AND unordered (NaN ⇒ even parity) all take
///    the ortho arm — Rust `== 0.0` (true for ±0.0, false for NaN) matches.
///    * perspective: `zn = −(m32/(m22+1))`, `zf = −(m32/(m22−1))`,
///      `m22' = zf/(zf−zn)`, `m32' = (zf·zn)/(zn−zf)`;
///    * ortho: `inv = 1/m22` (stock folds the two divides into one `FDIV` +
///      two `FMUL`s), `zn = (−1−m32)·inv`, `zf = (1−m32)·inv`,
///      `m22' = 1/(zf−zn)`, `m32' = zn/(zn−zf)`.
///    * every divide is unguarded exactly like stock (`m22±1`, `zf−zn`,
///      `zn−zf` may be 0 ⇒ ±inf/NaN propagate raw).
/// 3. **conditional `diag(0.2,0.2,0.2,1)` right-multiply** — skipped when the
///    caller's caps bit is set (`caps_no_scale`) or `m33 == 1.0`. The gate is
///    `FCOMP m33, 1.0; TEST AH,0x44; JNP skip`: only ORDERED equality skips;
///    greater, less and NaN all multiply — Rust `!= 1.0` (true on NaN) matches.
///    The multiply keeps the stock 4-term row·column accumulation (k=3 term
///    first, then k=0,1,2 — the `FADDP` chain order) and does NOT elide the
///    zero off-diagonal terms, so `0.0 × NaN/inf` propagates exactly as stock.
pub fn c_gx_device_d3d__i_xform_set_projection__5a11d0(
    proj: &[f32; 16],
    eps: f32,
    caps_no_scale: bool,
) -> [f32; 16] {
    let mut m = *proj;

    // (1) eps-guarded w-normalize by 1/m23.
    let m23 = f64::from(m[11]);
    let eps = f64::from(eps);
    let skip_near_one = (m23 - 1.0).abs() < eps; // ordered: NaN ⇒ false ⇒ proceed
    let skip_near_zero = m23.abs() < eps; // ordered: NaN ⇒ false ⇒ proceed
    if !skip_near_one && !skip_near_zero {
        let w = 1.0 / m23;
        for v in &mut m {
            *v = super::f64_to_f32(f64::from(*v) * w);
        }
    }

    // (2) perspective/ortho depth-range remap of m22/m32 (post-normalize).
    let m22 = f64::from(m[10]);
    let m32 = f64::from(m[14]);
    if m[15] == 0.0 {
        // perspective (ordered equality with ±0.0 only)
        let zn = -(m32 / (m22 + 1.0));
        let zf = -(m32 / (m22 - 1.0));
        m[10] = super::f64_to_f32(zf / (zf - zn));
        m[14] = super::f64_to_f32((zf * zn) / (zn - zf));
    } else {
        // ortho (greater, less, or NaN m33)
        let inv = 1.0 / m22;
        let zn = (-1.0 - m32) * inv;
        let zf = (1.0 - m32) * inv;
        m[10] = super::f64_to_f32(1.0 / (zf - zn));
        m[14] = super::f64_to_f32(zn / (zn - zf));
    }

    // (3) conditional diag(0.2,0.2,0.2,1) right-multiply (full 4-term rows).
    if !caps_no_scale && m[15] != 1.0 {
        let mc = m;
        // 0.2 is the stock imm 0x3e4ccccd; 1.0 is 0x3f800000.
        const S: [f32; 16] = [
            0.2, 0.0, 0.0, 0.0, 0.0, 0.2, 0.0, 0.0, 0.0, 0.0, 0.2, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        for r in 0..4 {
            for c in 0..4 {
                let mut acc = f64::from(S[12 + c]) * f64::from(mc[r * 4 + 3]);
                acc += f64::from(S[c]) * f64::from(mc[r * 4]);
                acc += f64::from(S[4 + c]) * f64::from(mc[r * 4 + 1]);
                acc += f64::from(S[8 + c]) * f64::from(mc[r * 4 + 2]);
                m[r * 4 + c] = super::f64_to_f32(acc);
            }
        }
    }

    m
}

#[cfg(test)]
mod tests_c_gx_device_d3d__i_xform_set_projection__5a11d0 {
    use super::c_gx_device_d3d__i_xform_set_projection__5a11d0 as xform;

    /// `_DAT_008029d4` in the shipped image: bits 0x34800000 = 2^-22.
    const EPS: f32 = f32::from_bits(0x3480_0000);

    /// GL-convention row-vector perspective (v' = v·M).
    ///
    /// `m[10] = (f+n)/(n−f)`, `m[14] = 2fn/(n−f)`, `m[11] = −1`, `m[15] = 0`.
    fn gl_perspective(n: f32, f: f32) -> [f32; 16] {
        let mut m = [0.0f32; 16];
        m[0] = 1.5;
        m[5] = 2.0;
        m[10] = (f + n) / (n - f);
        m[11] = -1.0;
        m[14] = 2.0 * f * n / (n - f);
        m
    }

    /// GL-convention row-vector ortho.
    ///
    /// `m[10] = −2/(f−n)`, `m[14] = −(f+n)/(f−n)`, `m[11] = 0`, `m[15] = 1`.
    fn gl_ortho(n: f32, f: f32) -> [f32; 16] {
        let mut m = [0.0f32; 16];
        m[0] = 0.5;
        m[5] = 0.25;
        m[10] = -2.0 / (f - n);
        m[12] = 0.125;
        m[14] = -(f + n) / (f - n);
        m[15] = 1.0;
        m
    }

    /// Independent oracle.
    ///
    /// Post-conversion NDC depth of an eye-space point at `z` (row-vector
    /// convention, GL looks down −z).
    fn depth(m: &[f32; 16], z: f32) -> f32 {
        (z * m[10] + m[14]) / (z * m[11] + m[15])
    }

    #[test]
    fn perspective_remaps_depth_to_0_1() {
        let (n, f) = (1.0f32, 100.0f32);
        let out = xform(&gl_perspective(n, f), EPS, true);
        // w-normalize by 1/−1 ran: m23 became 1.
        assert_eq!(out[11], 1.0);
        // Independent functional oracle: near plane → 0, far plane → 1.
        assert!(depth(&out, -n).abs() < 1e-5);
        assert!((depth(&out, -f) - 1.0).abs() < 1e-5);
        // Closed form: m22' = f/(f−n), m32' = fn/(f−n) (for this GL input).
        assert!((out[10] - f / (f - n)).abs() < 1e-5);
        assert!((out[14] - f * n / (f - n)).abs() < 1e-4);
    }

    #[test]
    fn ortho_remaps_depth_and_m33_1_skips_scale() {
        let (n, f) = (1.0f32, 100.0f32);
        let out = xform(&gl_ortho(n, f), EPS, false);
        // m23 = 0 ⇒ |m23| < eps ⇒ w-normalize skipped; m33 = 1 ⇒ ortho arm.
        assert!(depth(&out, -n).abs() < 1e-6);
        assert!((depth(&out, -f) - 1.0).abs() < 1e-6);
        // Closed form: m22' = −1/(f−n), m32' = −n/(f−n).
        assert!((out[10] - (-1.0 / (f - n))).abs() < 1e-7);
        assert!((out[14] - (-n / (f - n))).abs() < 1e-7);
        // m33 == 1.0 skips the 0.2 right-multiply even with the caps bit clear.
        assert_eq!(out[0].to_bits(), 0.5f32.to_bits());
        assert_eq!(out[5].to_bits(), 0.25f32.to_bits());
        assert_eq!(out[12].to_bits(), 0.125f32.to_bits());
        assert_eq!(out[15].to_bits(), 1.0f32.to_bits());
    }

    #[test]
    fn w_normalize_skips_inside_eps_windows() {
        // |m23| < eps ⇒ skip (the scale by 1/m23 = 2^23 would be unmissable).
        let mut m = gl_ortho(1.0, 100.0);
        m[11] = EPS / 2.0;
        let out = xform(&m, EPS, false);
        assert_eq!(out[0].to_bits(), 0.5f32.to_bits());
        // |m23 − 1| < eps ⇒ skip: column-3 elements pass through bit-exactly.
        let mut m = gl_ortho(1.0, 100.0);
        m[7] = 3.7;
        m[11] = 1.0 - f32::from_bits(0x3380_0000); // 1 − 2^-24, inside the window
        let out = xform(&m, EPS, false);
        assert_eq!(out[7].to_bits(), 3.7f32.to_bits());
        assert_eq!(out[11].to_bits(), m[11].to_bits());
    }

    #[test]
    fn w_normalize_boundary_is_strictly_less() {
        // |m23 − 1| == eps exactly ⇒ NOT skipped (stock C0 fires only on
        // strict less-than): 1 + 2^-22 is representable and normalizes.
        let mut m = gl_ortho(1.0, 100.0);
        m[7] = 4.0;
        m[11] = 1.0 + EPS;
        let out = xform(&m, EPS, false);
        // 4.0 / (1 + 2^-22) rounds below 4.0 ⇒ the normalize observably ran.
        assert_ne!(out[7].to_bits(), 4.0f32.to_bits());
        assert!((out[7] - 4.0).abs() < 1e-5);
    }

    #[test]
    fn nan_m23_proceeds_to_normalize() {
        // NaN compares unordered in both guards (even parity ⇒ JNP not taken)
        // and PROCEEDS: 1/NaN poisons all 16 elements.
        let mut m = gl_perspective(1.0, 100.0);
        m[11] = f32::NAN;
        let out = xform(&m, EPS, true);
        assert!(out.iter().all(|v| v.is_nan()));
    }

    #[test]
    fn nan_m33_selects_ortho_and_runs_scale() {
        let (n, f) = (1.0f32, 100.0f32);
        let mut m = gl_ortho(n, f);
        m[15] = f32::NAN;
        let out = xform(&m, EPS, false);
        // Ortho arm despite the NaN selector (JP taken on unordered): the
        // perspective arm would give m22' ≈ 0.49 here, ortho gives −1/99·0.2.
        let ortho_m22 = -1.0 / (f - n);
        assert!((out[10] - 0.2 * ortho_m22).abs() < 1e-7);
        // NaN != 1.0 ⇒ the 0.2 right-multiply DID run on columns 0..2.
        let exp0 = super::super::f64_to_f32(f64::from(0.2f32) * f64::from(0.5f32));
        assert_eq!(out[0].to_bits(), exp0.to_bits());
        // Column 3 accumulates 1.0·NaN ⇒ stays NaN.
        assert!(out[15].is_nan());
    }

    #[test]
    fn caps_bit_gates_the_scale() {
        let base = gl_perspective(1.0, 100.0);
        let no_scale = xform(&base, EPS, true);
        let scaled = xform(&base, EPS, false);
        // Columns 0..2 scaled by 0.2, column 3 value-identical. Zero lanes are
        // compared by VALUE, not bits: the full 4-term row·column chain adds
        // the zero off-diagonal products, which can flip a −0.0 to +0.0
        // (−0 + +0 = +0) exactly as the stock FADDP chain does.
        for r in 0..4 {
            for c in 0..3 {
                let ns = no_scale[r * 4 + c];
                let sc = scaled[r * 4 + c];
                if ns == 0.0 {
                    assert_eq!(sc, 0.0, "r{r} c{c}");
                } else {
                    let exp = super::super::f64_to_f32(f64::from(0.2f32) * f64::from(ns));
                    assert_eq!(sc.to_bits(), exp.to_bits(), "r{r} c{c}");
                }
            }
            assert_eq!(scaled[r * 4 + 3], no_scale[r * 4 + 3], "r{r} c3");
        }
    }

    #[test]
    fn negative_m33_takes_ortho() {
        // FCOMP parity: less-than (C0) is even parity for TEST AH,0x44 ⇒ JP
        // taken ⇒ ortho, same as greater/NaN. Only ±0.0 is perspective.
        let (n, f) = (1.0f32, 100.0f32);
        let mut m = gl_ortho(n, f);
        m[15] = -1.0;
        let out = xform(&m, EPS, false);
        let ortho_m22 = -1.0 / (f - n);
        // m33 = −1 ≠ 1 ⇒ scale ran ⇒ compare against the scaled ortho value.
        assert!((out[10] - 0.2 * ortho_m22).abs() < 1e-7);
    }
}

/// `CGxString::EmitLineQuads` (0x5ccbe0) `unitsPerPixel`.
///
/// The per-run scale `snappedLineHeight / fontPixelHeight`. Stock zero-extends
/// the u32 pixel height to a qword (`FILD` of `{height, 0}`, exact) and divides
/// with `FDIVR`, rounding once at the f32 store; `f64` division reproduces that
/// (a pixel height of 0 yields ±inf/NaN under masked x87 exceptions, same as
/// IEEE f64).
pub fn text_units_per_pixel__5ccbe0(glyph_height_world: f32, font_px_height: u32) -> f32 {
    super::f64_to_f32(f64::from(glyph_height_world) / f64::from(font_px_height))
}

/// One `FLD m32; FADD m32; FSTP m32`.
///
/// The emitter's ubiquitous two-term f32 add (pen advance accumulation, quad
/// corner width/height adds, hyperlink pen stores). Computed in `f64` (exact
/// for two f32 addends within 2^29 exponent spread) and narrowed once, matching
/// the single x87 store rounding.
pub fn text_add_f32__5ccbe0(a: f32, b: f32) -> f32 {
    super::f64_to_f32(f64::from(a) + f64::from(b))
}

/// Sub-pixel glyph advance, screen-space arm (`worldFlag` low byte == 0).
///
/// The raw glyph advance run through the CRT ceil helper (0x73fdf5 forces
/// control word 0x1b3f = round toward +inf around `FRNDINT`) — native `ceil`,
/// with the f64 argument widening the stock `FLD m32; FSTP m64` spill exactly.
pub fn glyph_ceil_advance__5cb080(advance: f32) -> f32 {
    super::f64_to_f32(f64::from(advance).ceil())
}

/// Sub-pixel glyph advance, world-space arm.
///
/// `lineHeight / pixelHeight × advance`. The pixel height divides as a SIGNED
/// 32-bit integer (stock `FIDIV m32int` — the adjacent zeroed dword is a dead
/// qword-slot artifact), and the divide/multiply chain folds on the x87 stack
/// (modeled in f64) before the single f32 narrow at the return boundary.
pub fn glyph_scaled_advance__5cb080(line_height: f32, px_height: i32, advance: f32) -> f32 {
    super::f64_to_f32(f64::from(line_height) / f64::from(px_height) * f64::from(advance))
}

/// The four scaled glyph texture coordinates stored at `this+0x60..0x6c`.
///
/// `[prod*sy, u30*sx, sum*sy, (u48+u30)*sx]` where `prod = a*b` is a SIGNED
/// 32-bit wrap product (`FILD m32int`), `sum = prod + b` reloads as an
/// UNSIGNED dword (`FILD` of a zero-extended qword slot), and `u30`/`u48`
/// are u32 fields FILDed the same way. Each product folds `int × f32` wide
/// (exact through f64's 53 bits for the 2·24+2 double-rounding margin) and
/// narrows once per store, in stock's 0x60→0x64→0x68→0x6c order.
///
/// `a` is the glyph's shelf-row index and `b` the cell height, so slots 0/2
/// are the V (vertical) pair; `u30`/`u48` are the cell's start column and
/// width, so slots 1/3 are the U (horizontal) pair. Stock scales all four by
/// the single 1/256 `.rdata` constant (square 256×256 pages); the enlarged
/// 256-wide × taller pages split the scales — `sx` stays stock's 1/256,
/// `sy` becomes 1/pageHeight.
pub fn glyph_tex_coords__5c7f80(a: i32, b: i32, u30: u32, u48: u32, sx: f32, sy: f32) -> [f32; 4] {
    let prod = a.wrapping_mul(b);
    let sum = prod.wrapping_add(b) as u32;
    let sx = f64::from(sx);
    let sy = f64::from(sy);
    [
        super::f64_to_f32(f64::from(prod) * sy),
        super::f64_to_f32(f64::from(u30) * sx),
        super::f64_to_f32(f64::from(sum) * sy),
        super::f64_to_f32((f64::from(u48) + f64::from(u30)) * sx),
    ]
}

/// Kerned-advance fold on the miss path of the pair cache.
///
/// `baseAdvance + kern × scale`, where `kern` is the clamped (≤ 0) signed
/// kerning FILDed exactly, `scale` is the f32 at `font+0x188`, and the
/// FILD/FMUL/FADDP chain folds wide before the single `FSTP m32` narrow.
pub fn kerned_advance__5ca2d0(kern: i32, scale: f32, base: f32) -> f32 {
    super::f64_to_f32(f64::from(kern) * f64::from(scale) + f64::from(base))
}

/// Screen-space line-spacing seed (0x5cdcd1..0x5cdd10).
///
/// `spacing × deviceXScale + bias` truncates via `__ftol`, reloads as a
/// zero-extended u32 qword (a negative sum wraps huge-positive like stock),
/// `FST`s an f32 copy, and divides the WIDE reload back by the scale for the
/// reduced spacing. Returns `(t_f32, spacing_scaled)`.
pub fn build_line_spacing__5cdc20(spacing: f32, x_scale: u32, bias: f32) -> (f32, f32) {
    let t = super::misc::ftol__40a2b0(f64::from(spacing) * f64::from(x_scale) + f64::from(bias));
    let t_wide = f64::from(t as u32);
    (
        super::f64_to_f32(t_wide),
        super::f64_to_f32(t_wide / f64::from(x_scale)),
    )
}

/// Per-line block-height step.
///
/// `used + lineHeight + spacing` folded wide, one narrow.
pub fn build_block_step__5cdc20(used: f32, line_h: f32, spacing: f32) -> f32 {
    super::f64_to_f32(f64::from(used) + f64::from(line_h) + f64::from(spacing))
}

/// Pen / line-top decrement per emitted line: `a − lineAdvance`, one narrow.
pub fn build_pen_step__5cdc20(a: f32, advance: f32) -> f32 {
    super::f64_to_f32(f64::from(a) - f64::from(advance))
}

/// Justification snap argument.
///
/// Right-justify negates the width exactly (`FCHS`); center-justify negates
/// the WIDE `width × half` product and narrows at the argument push.
pub fn build_justify_arg__5cdc20(width: f32, half: f32, center: bool) -> f32 {
    if center {
        super::f64_to_f32(-(f64::from(width) * f64::from(half)))
    } else {
        -width
    }
}

#[cfg(test)]
mod tests_build_geometry__5cdc20 {
    use super::{
        build_block_step__5cdc20 as step, build_justify_arg__5cdc20 as justify,
        build_line_spacing__5cdc20 as spacing, build_pen_step__5cdc20 as pen,
    };

    #[test]
    fn spacing_truncates_biased_and_divides_wide() {
        // 2.75 * 2 + 0.5 = 6.0 -> t 6; spacing' = 6 / 2 = 3.
        let (t, s) = spacing(2.75, 2, 0.5);
        assert_eq!(t, 6.0);
        assert_eq!(s, 3.0);
        // Negative sum wraps through the u32 reload to a huge positive.
        let (t2, _) = spacing(-4.0, 1, 0.5);
        assert!(t2 > 4.0e9);
    }

    #[test]
    fn block_step_folds_wide_once() {
        assert_eq!(step(1.0, 2.0, 3.0), 6.0);
        let want =
            super::super::f64_to_f32(f64::from(0.1f32) + f64::from(0.2f32) + f64::from(0.3f32));
        assert_eq!(step(0.1, 0.2, 0.3).to_bits(), want.to_bits());
    }

    #[test]
    fn pen_step_subtracts() {
        assert_eq!(pen(10.0, 2.5), 7.5);
        assert!(pen(f32::NAN, 1.0).is_nan());
    }

    #[test]
    fn justify_center_narrows_the_product() {
        assert_eq!(justify(8.0, 0.5, true), -4.0);
        assert_eq!(justify(8.0, 0.5, false), -8.0);
        assert_eq!(justify(-0.0, 0.5, false).to_bits(), 0.0f32.to_bits());
    }
}

/// Line-fit pixel budget.
///
/// `ceil(pxHeight / snappedHeight × maxWidth × yScale)` with the pixel height
/// FILDed as a zero-extended u32, the device Y scale FIMULed as a SIGNED dword,
/// the whole chain wide, and the CRT-ceil (0x73fdf5) applied before the single
/// f32 narrow.
pub fn fit_budget__5c7470(px_height: u32, snapped: f32, max_width: f32, y_scale: i32) -> f32 {
    super::f64_to_f32(
        (f64::from(px_height) / f64::from(snapped) * f64::from(max_width) * f64::from(y_scale))
            .ceil(),
    )
}

/// Line-fit overflow gate.
///
/// The 80-bit `kern + advance + accum` fold compared against the pixel budget;
/// only a STRICTLY greater sum overflows (the `TEST AH,0x41; JZ` arm) — an
/// unordered/NaN sum keeps fitting.
pub fn fit_overflows__5c7470(kern: f32, adv: f32, accum: f32, budget: f32) -> bool {
    f64::from(kern) + f64::from(adv) + f64::from(accum) > f64::from(budget)
}

/// Line-fit tail rescale.
///
/// One wide quotient `one / yScale` multiplies both the `lastAdvance + accum`
/// fold and the recorded line width, each narrowed once (stock computes the
/// quotient once and keeps it on the stack for both FMULs). Returns
/// `(accum_scaled, width_scaled)`.
pub fn fit_tail_scale__5c7470(
    one: f32,
    y_scale: u32,
    last_adv: f32,
    accum: f32,
    width_px: f32,
) -> (f32, f32) {
    let q = f64::from(one) / f64::from(y_scale);
    (
        super::f64_to_f32((f64::from(last_adv) + f64::from(accum)) * q),
        super::f64_to_f32(f64::from(width_px) * q),
    )
}

/// Line-fit units-per-pixel split.
///
/// The quotient `snapped / pxHeight` is FST-narrowed for the break-rewind
/// width but stays WIDE for the full-line `q × accum` product. Returns
/// `(upp_narrowed, full_line_width)`.
pub fn fit_upp_width__5c7470(snapped: f32, px_height: u32, accum: f32) -> (f32, f32) {
    let q = f64::from(snapped) / f64::from(px_height);
    (
        super::f64_to_f32(q),
        super::f64_to_f32(q * f64::from(accum)),
    )
}

#[cfg(test)]
mod tests_fit_text_to_width__5c7470 {
    use super::{
        fit_budget__5c7470 as budget, fit_overflows__5c7470 as over,
        fit_tail_scale__5c7470 as tail, fit_upp_width__5c7470 as upp,
    };

    #[test]
    fn budget_ceils_the_wide_chain() {
        // 16 / 8 * 10.1 * 2 = 40.4 -> ceil 41.
        assert_eq!(budget(16, 8.0, 10.1, 2), 41.0);
        // Negative Y scale is a SIGNED multiply.
        assert_eq!(budget(16, 8.0, 10.0, -2), -40.0);
    }

    #[test]
    fn overflow_is_strictly_greater_and_nan_fits() {
        assert!(over(1.0, 1.0, 1.0, 2.9));
        assert!(!over(1.0, 1.0, 1.0, 3.0));
        assert!(!over(f32::NAN, 1.0, 1.0, 3.0));
    }

    #[test]
    fn tail_uses_one_wide_quotient_for_both_lanes() {
        let (a, w) = tail(1.0, 4, 2.0, 6.0, 10.0);
        assert_eq!(a, 2.0);
        assert_eq!(w, 2.5);
    }

    #[test]
    fn upp_narrows_only_the_stored_lane() {
        let (u, w) = upp(1.0, 3, 9.0);
        // Stored lane rounds to f32; the product folds from the wide quotient.
        assert_eq!(u, super::super::f64_to_f32(1.0 / 3.0));
        assert_eq!(
            w.to_bits(),
            super::super::f64_to_f32(1.0 / 3.0 * 9.0).to_bits()
        );
        assert_ne!(f64::from(u) * 9.0, 1.0 / 3.0 * 9.0);
    }
}

/// Final measured-run width.
///
/// `(kernAccum + lastAdvance) × (reqHeight / pixelHeight)`, with the pixel
/// height FILDed as a zero-extended u32 and the whole sum/quotient/product
/// chain folded wide before the single `FSTP m32` into the caller's out slot.
pub fn measured_width__5c7300(
    kern_accum: f32,
    last_advance: f32,
    req_height: f32,
    px_height: u32,
) -> f32 {
    super::f64_to_f32(
        (f64::from(kern_accum) + f64::from(last_advance))
            * (f64::from(req_height) / f64::from(px_height)),
    )
}

#[cfg(test)]
mod tests_measured_width__5c7300 {
    use super::measured_width__5c7300 as width;

    #[test]
    fn folds_sum_times_quotient_wide() {
        // (1.5 + 2.5) * (8 / 16) = 2.0 exactly.
        assert_eq!(width(1.5, 2.5, 8.0, 16), 2.0);
        let want = super::super::f64_to_f32(
            (f64::from(0.1f32) + f64::from(0.2f32)) * (f64::from(14.0f32) / f64::from(12u32)),
        );
        assert_eq!(width(0.1, 0.2, 14.0, 12).to_bits(), want.to_bits());
    }

    #[test]
    fn zero_px_height_divides_to_inf() {
        assert_eq!(width(1.0, 1.0, 2.0, 0), f32::INFINITY);
    }
}

/// Glyph advance fallback fold.
///
/// Sub-pixel advance plus the SIGNED 32-bit glyph width at `glyph+0x48`
/// (`FIADD m32int` on the x87 result), one f32 narrow at the return boundary.
pub fn glyph_advance_plus_width__5c6b70(subadv: f32, width: i32) -> f32 {
    super::f64_to_f32(f64::from(subadv) + f64::from(width))
}

#[cfg(test)]
mod tests_glyph_advance_plus_width__5c6b70 {
    use super::glyph_advance_plus_width__5c6b70 as fold;

    #[test]
    fn adds_signed_width_wide() {
        assert_eq!(fold(1.5, 7), 8.5);
        assert_eq!(fold(1.5, -2), -0.5);
        let want = super::super::f64_to_f32(f64::from(0.1f32) + f64::from(16_777_217_i32));
        assert_eq!(fold(0.1, 16_777_217).to_bits(), want.to_bits());
    }
}

#[cfg(test)]
mod tests_kerned_advance__5ca2d0 {
    use super::kerned_advance__5ca2d0 as adv;

    #[test]
    fn folds_wide_then_narrows_once() {
        assert_eq!(adv(-2, 0.5, 10.0), 9.0);
        assert_eq!(adv(0, 123.0, 7.25), 7.25);
        let want = super::super::f64_to_f32(f64::from(-3) * f64::from(0.1f32) + f64::from(8.7f32));
        assert_eq!(adv(-3, 0.1, 8.7).to_bits(), want.to_bits());
    }

    #[test]
    fn nan_base_propagates() {
        assert!(adv(-1, 1.0, f32::NAN).is_nan());
    }
}

#[cfg(test)]
mod tests_glyph_tex_coords__5c7f80 {
    use super::glyph_tex_coords__5c7f80 as coords;

    #[test]
    fn scales_each_lane() {
        // Equal scales 1/256, cell (3, 16): prod 48, sum 64, dims 128/256.
        let s = 1.0f32 / 256.0;
        let c = coords(3, 16, 128, 256, s, s);
        assert_eq!(c, [0.1875, 0.5, 0.25, 1.5]);
    }

    #[test]
    fn split_scales_route_v_and_u_lanes() {
        // sy applies to the row-product lanes 0/2, sx to the column lanes
        // 1/3 (the enlarged-page shape: 1/256 wide, 1/1024 tall).
        let sx = 1.0f32 / 256.0;
        let sy = 1.0f32 / 1024.0;
        let c = coords(3, 16, 128, 256, sx, sy);
        assert_eq!(c, [0.046875, 0.5, 0.0625, 1.5]);
    }

    #[test]
    fn product_is_signed_but_sum_reloads_unsigned() {
        // a*b wraps negative: prod = -2 -> lane0 negative; sum = prod + b =
        // -1 reloads as u32::MAX -> lane2 is the LARGE positive value.
        let c = coords(-2, 1, 0, 0, 1.0, 1.0);
        assert_eq!(c[0], -2.0);
        assert_eq!(c[2], u32::MAX as f32);
    }

    #[test]
    fn u48_u30_sum_is_exact_before_scaling() {
        // Two u32 near the top: the sum exceeds u32 but is exact in f64.
        let c = coords(0, 0, u32::MAX, u32::MAX, 0.5, 0.5);
        let want = super::super::f64_to_f32((f64::from(u32::MAX) + f64::from(u32::MAX)) * 0.5);
        assert_eq!(c[3].to_bits(), want.to_bits());
    }
}

#[cfg(test)]
mod tests_sub_pixel_advance__5cb080 {
    use super::{glyph_ceil_advance__5cb080 as ceil_adv, glyph_scaled_advance__5cb080 as scaled};

    #[test]
    fn ceil_rounds_toward_positive_infinity() {
        assert_eq!(ceil_adv(2.25), 3.0);
        assert_eq!(ceil_adv(-2.25), -2.0);
        assert_eq!(ceil_adv(4.0), 4.0);
        assert!(ceil_adv(f32::NAN).is_nan());
    }

    #[test]
    fn scaled_folds_wide_then_narrows_once() {
        // 0.75 / 3 * 8 = 2.0 exactly.
        assert_eq!(scaled(0.75, 3, 8.0), 2.0);
        // Wide fold: quotient stays f64 until the final narrow.
        let lh = 0.1f32;
        let want = super::super::f64_to_f32(f64::from(lh) / 12.0 * f64::from(7.0f32));
        assert_eq!(scaled(lh, 12, 7.0).to_bits(), want.to_bits());
    }

    #[test]
    fn pixel_height_is_signed_and_zero_divides_to_inf() {
        // FIDIV m32int is a signed divide; a negative height flips the sign.
        assert_eq!(scaled(1.0, -2, 4.0), -2.0);
        // Height 0 under masked x87 exceptions -> +inf, same as IEEE f64.
        assert_eq!(scaled(1.0, 0, 4.0), f32::INFINITY);
    }
}

/// Per-token pen advance `kerningPx * unitsPerPixel` (`FLD; FMUL; FSTP` at 0x5ccdb2..0x5ccdb8).
///
/// An exact f32×f32 product in `f64`, one f32 rounding.
pub fn text_advance__5ccbe0(kerning_px: f32, units_per_pixel: f32) -> f32 {
    super::f64_to_f32(f64::from(kerning_px) * f64::from(units_per_pixel))
}

/// Screen-space pen-advance rounding (0x5cd040..0x5cd05b).
///
/// `FADD` the bias (`DAT_007ffa24`, 0.5 in the shipped image), truncate via
/// `__ftol`, keep EAX only and zero the high dword, `FILD` the qword back and
/// store f32 — i.e. `f32(u64(u32(trunc(advance + bias))))`. A negative sum
/// wraps through the u32 reinterpretation to a huge positive float, exactly
/// like stock.
pub fn text_round_advance__5ccbe0(advance: f32, bias: f32) -> f32 {
    let t = super::misc::ftol__40a2b0(f64::from(advance) + f64::from(bias));
    super::f64_to_f32(f64::from(t as u32))
}

/// Glyph width in world units (0x5cd0b6..0x5cd0d2).
///
/// `FILD` of the zero-extended u32 advance-px (`{px, 0}` qword, exact) ×
/// `unitsPerPixel`, stored f32; then, screen-space only, the CRT rounder at
/// 0x73ff3f. That rounder is **floor**, not rint: it forces the FPU control
/// word to `[0x875f00]` = 0x173f (RC=01, round toward −∞; the sibling ceil at
/// 0x73fdf5 forces 0x1b3f/RC=10) around a double-precision `FRNDINT`, so
/// half-integer widths always round DOWN.
pub fn text_glyph_width_world__5ccbe0(
    advance_px: u32,
    units_per_pixel: f32,
    screen_floor: bool,
) -> f32 {
    let w = super::f64_to_f32(f64::from(advance_px) * f64::from(units_per_pixel));
    if screen_floor {
        super::f64_to_f32(f64::from(w).floor())
    } else {
        w
    }
}

/// Per-glyph quad top Y (0x5cd0d8..0x5cd113).
///
/// `penY − ySub` (the font outline/shadow offset, 0.0 when neither font flag is
/// set — `x − 0.0 ≡ x` bit-exactly, so the no-offset arm needs no special
/// case), plus `yBearing (i32, FILD dword) × unitsPerPixel`, all on the x87
/// stack with one f32 rounding at the store.
pub fn text_glyph_quad_y__5ccbe0(
    pen_y: f32,
    y_sub: f32,
    y_bearing: i32,
    units_per_pixel: f32,
) -> f32 {
    super::f64_to_f32(
        f64::from(y_bearing) * f64::from(units_per_pixel) + (f64::from(pen_y) - f64::from(y_sub)),
    )
}

/// Pre-run glyph-height adjust (0x5cccb1..0x5ccccd).
///
/// Font flag 0x8 (outline) adds `DAT_0080306c`, else flag 0x1 (shadow) adds
/// `DAT_00801628`; neither leaves the snapped height untouched (bit-exact
/// passthrough, no `+ 0.0`).
pub fn text_glyph_height_adjust__5ccbe0(
    snapped: f32,
    font_flags: u32,
    outline_add: f32,
    shadow_add: f32,
) -> f32 {
    if font_flags & 8 != 0 {
        super::f64_to_f32(f64::from(snapped) + f64::from(outline_add))
    } else if font_flags & 1 != 0 {
        super::f64_to_f32(f64::from(snapped) + f64::from(shadow_add))
    } else {
        snapped
    }
}

/// Full glyph quad (0x5cd162..0x5cd277).
///
/// Four 5-f32 vertices `[x,y,z,u,v]` in slot order v0..v3. All four start as
/// `(x, y, z, 0, 0)`; the corner fixup then adds `width` to v2/v3 x and
/// `height` to v1/v3 y. The stock world-space (0x5cd1aa) and screen-space
/// (0x5cd1cf) arms interleave their copies/adds differently but read every
/// input lane before any aliasing write, and the four freshly-written vertices
/// are identical — so both arms produce these same lanes (the arms differ only
/// in WHICH height local the caller selects, and in the world-only half-texel
/// UV inset applied here). UV lane mapping (from the stores at
/// 0x5cd250..0x5cd274): v0=(uv1,uv2), v1=(uv1,uv0), v2=(uv3,uv2), v3=(uv3,uv0),
/// with `uv[i] += uv_inset[i]` first when world-space (insets
/// `+1/512, +1/512, −1/512, −1/512` in the live image).
pub fn emit_glyph_quad__5ccbe0(
    pen: [f32; 3],
    width_world: f32,
    height_world: f32,
    uv: [f32; 4],
    world_space: bool,
    uv_inset: [f32; 4],
) -> [f32; 20] {
    let [x, y, z] = pen;
    let x2 = text_add_f32__5ccbe0(width_world, x);
    let y2 = text_add_f32__5ccbe0(height_world, y);
    let q = if world_space {
        [
            text_add_f32__5ccbe0(uv[0], uv_inset[0]),
            text_add_f32__5ccbe0(uv[1], uv_inset[1]),
            text_add_f32__5ccbe0(uv[2], uv_inset[2]),
            text_add_f32__5ccbe0(uv[3], uv_inset[3]),
        ]
    } else {
        uv
    };
    #[rustfmt::skip]
    let out = [
        x,  y,  z, q[1], q[2], // v0
        x,  y2, z, q[1], q[0], // v1
        x2, y,  z, q[3], q[2], // v2
        x2, y2, z, q[3], q[0], // v3
    ];
    out
}

#[cfg(test)]
mod tests_emit_line_quads_5ccbe0 {
    use super::{
        emit_glyph_quad__5ccbe0 as quad, text_add_f32__5ccbe0 as add,
        text_advance__5ccbe0 as advance, text_glyph_height_adjust__5ccbe0 as hadjust,
        text_glyph_quad_y__5ccbe0 as quad_y, text_glyph_width_world__5ccbe0 as width,
        text_round_advance__5ccbe0 as round_adv, text_units_per_pixel__5ccbe0 as upp,
    };

    #[test]
    fn units_per_pixel_exact_and_div_by_zero() {
        assert_eq!(upp(16.0, 32), 0.5);
        assert_eq!(upp(14.0, 14), 1.0);
        // Pixel height 0: masked x87 divide-by-zero yields +inf (NaN for 0/0).
        assert_eq!(upp(16.0, 0), f32::INFINITY);
        assert!(upp(0.0, 0).is_nan());
    }

    #[test]
    fn advance_is_single_rounded_product() {
        assert_eq!(advance(1.5, 0.5), 0.75);
        assert_eq!(advance(0.0, 123.0), 0.0);
        // f32(f64(a)*f64(b)) differs from f32 a*b only by double rounding;
        // exact dyadic operands must match plain f32 math.
        assert_eq!(advance(3.0, 0.25), 3.0f32 * 0.25);
    }

    #[test]
    fn round_advance_is_bias_plus_truncate() {
        // +0.5 then truncate toward zero (stock __ftol) — half-up for positives.
        assert_eq!(round_adv(2.25, 0.5), 2.0);
        assert_eq!(round_adv(2.5, 0.5), 3.0);
        assert_eq!(round_adv(2.49, 0.5), 2.0);
        assert_eq!(round_adv(0.0, 0.5), 0.0);
        // Small negatives truncate to 0 through the bias.
        assert_eq!(round_adv(-0.25, 0.5), 0.0);
        // A negative sum keeps only EAX and zero-extends: -2.5+0.5 = -2 →
        // 0xfffffffe → 4294967294.0 (the stock u32 wrap, faithfully weird).
        assert_eq!(round_adv(-2.5, 0.5), 4_294_967_294u32 as f32);
    }

    #[test]
    fn width_screen_floor_is_floor_not_round_ties_even() {
        // upp=0.5: 5 px → 2.5 → floor 2.0 (rint/ties-even would give 2.0 too,
        // but 7 px → 3.5 → floor 3.0, where ties-even gives 4.0).
        assert_eq!(width(5, 0.5, true), 2.0);
        assert_eq!(width(7, 0.5, true), 3.0);
        // Negative product (negative upp): floor goes DOWN, not toward zero.
        assert_eq!(width(5, -0.5, true), -3.0);
        // World-space skips the floor entirely.
        assert_eq!(width(5, 0.5, false), 2.5);
        assert_eq!(width(7, 0.5, false), 3.5);
        // The px dword is UNSIGNED (FILD of a zero-extended qword).
        assert_eq!(width(0xffff_ffff, 1.0, false), 4_294_967_295u32 as f32);
    }

    #[test]
    fn quad_y_combines_bearing_and_offset() {
        // y' = bearing*upp + (penY - sub) = -6*0.5 + (100-2) = 95.
        assert_eq!(quad_y(100.0, 2.0, -6, 0.5), 95.0);
        // No font flag: sub = 0.0, but the bearing product is STILL added
        // (stock runs FILD/FMUL/FADD unconditionally), so a -0.0 penY
        // canonicalizes to +0.0 through `0.0 + (-0.0)`.
        assert_eq!(quad_y(-0.0, 0.0, 0, 0.5).to_bits(), 0.0f32.to_bits());
        assert_eq!(quad_y(-2.5, 0.0, 0, 0.5), -2.5);
        assert_eq!(quad_y(7.25, 0.0, 4, 0.25), 8.25);
    }

    #[test]
    fn height_adjust_flag_priority() {
        // Flag 0x8 (outline) wins over 0x1 (shadow); neither = untouched bits.
        assert_eq!(hadjust(10.0, 0x8, 4.0, 2.0), 14.0);
        assert_eq!(hadjust(10.0, 0x9, 4.0, 2.0), 14.0);
        assert_eq!(hadjust(10.0, 0x1, 4.0, 2.0), 12.0);
        let untouched = hadjust(f32::from_bits(0x8000_0001), 0, 4.0, 2.0);
        assert_eq!(untouched.to_bits(), 0x8000_0001);
    }

    #[test]
    fn quad_screen_space_lanes() {
        // pen (10,20,3), w=5, h=7, uv=(0.25,0.5,0.75,1.0) — all dyadic/exact.
        let q = quad(
            [10.0, 20.0, 3.0],
            5.0,
            7.0,
            [0.25, 0.5, 0.75, 1.0],
            false,
            [9.0; 4], // inset must be ignored when screen-space
        );
        #[rustfmt::skip]
        let expect = [
            10.0, 20.0, 3.0, 0.5,  0.75, // v0 = (x, y),  uv (uv1, uv2)
            10.0, 27.0, 3.0, 0.5,  0.25, // v1 = (x, y+h), uv (uv1, uv0)
            15.0, 20.0, 3.0, 1.0,  0.75, // v2 = (x+w, y), uv (uv3, uv2)
            15.0, 27.0, 3.0, 1.0,  0.25, // v3 = (x+w, y+h), uv (uv3, uv0)
        ];
        assert_eq!(q, expect);
    }

    #[test]
    fn quad_world_space_applies_uv_inset() {
        let e = 0.001953125f32; // 1/512, the live-image inset
        let q = quad(
            [1.0, 2.0, 3.0],
            4.0,
            8.0,
            [0.25, 0.5, 0.75, 1.0],
            true,
            [e, e, -e, -e],
        );
        // Corners identical to the screen arm (both stock arms produce the
        // same lanes); UVs are inset before the lane mapping.
        let (u0, u1, u2, u3) = (0.25 + e, 0.5 + e, 0.75 - e, 1.0 - e);
        #[rustfmt::skip]
        let expect = [
            1.0, 2.0,  3.0, u1, u2,
            1.0, 10.0, 3.0, u1, u0,
            5.0, 2.0,  3.0, u3, u2,
            5.0, 10.0, 3.0, u3, u0,
        ];
        assert_eq!(q, expect);
    }

    #[test]
    fn quad_corner_adds_match_f32_oracle() {
        // The corner math is a plain f32 add (single rounding): cross-check the
        // kernel against direct f32 arithmetic on non-dyadic values.
        let (x, w) = (0.1f32, 0.2f32);
        let (y, h) = (1000.3f32, 0.7f32);
        let q = quad([x, y, 0.0], w, h, [0.0; 4], false, [0.0; 4]);
        assert_eq!(q[10], add(w, x));
        assert_eq!(q[16], add(h, y));
        assert_eq!(add(w, x), (f64::from(w) + f64::from(x)) as f32);
        assert_eq!(add(h, y), (f64::from(h) + f64::from(y)) as f32);
    }

    #[test]
    fn nan_flows_through_without_branching() {
        // No float compare drives control flow: NaN pen/uv lanes propagate.
        let q = quad(
            [f32::NAN, 0.0, 0.0],
            1.0,
            1.0,
            [f32::NAN; 4],
            true,
            [0.5; 4],
        );
        assert!(q[0].is_nan() && q[5].is_nan() && q[10].is_nan());
        assert!(q[3].is_nan() && q[19].is_nan());
        assert!(quad_y(f32::NAN, 0.0, 1, 1.0).is_nan());
        assert!(width(1, f32::NAN, false).is_nan());
        // __ftol of a NaN sum: the fistp indefinite → low u32 0 → 0.0.
        assert_eq!(round_adv(f32::NAN, 0.5), 0.0);
    }
}

/// UI textured-quad UV corner (0x6cd7a3..0x6cd7c6).
///
/// `u = baseU × scaleX + offX`, `v = baseV × scaleY + offY`, each folded wide
/// with one narrow at the `FSTP`.
pub fn quad_uv__6cd750(
    base_u: f32,
    base_v: f32,
    scale_x: f32,
    scale_y: f32,
    off_x: f32,
    off_y: f32,
) -> (f32, f32) {
    (
        super::f64_to_f32(f64::from(base_u) * f64::from(scale_x) + f64::from(off_x)),
        super::f64_to_f32(f64::from(base_v) * f64::from(scale_y) + f64::from(off_y)),
    )
}

#[cfg(test)]
mod tests_quad_uv__6cd750 {
    use super::quad_uv__6cd750 as uv;

    #[test]
    fn scales_and_offsets() {
        assert_eq!(uv(2.0, 3.0, 0.5, 0.25, 1.0, -0.5), (2.0, 0.25));
    }

    #[test]
    fn folds_wide_once() {
        let (u, _) = uv(0.1, 0.0, 0.2, 0.0, 0.3, 0.0);
        assert_eq!(
            u,
            super::super::f64_to_f32(f64::from(0.1f32) * f64::from(0.2f32) + f64::from(0.3f32))
        );
    }
}
