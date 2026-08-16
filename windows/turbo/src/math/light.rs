// Adapter and kernel names mirror the host's C++ symbols verbatim, with `__`
// standing in for the `::`, so the whole module is non-snake-case by construction.
#![allow(non_snake_case)]

/// Inverse of 255, the host's constant at `0x8026c8`.
///
/// Normalises a 0..255 colour byte to 0.0..1.0.
const INV_255: f32 = 1.0 / 255.0;

/// Unpacks a packed `BGRA` colour word into `[R, G, B, A]`, each byte scaled by `scale / 255`.
///
/// The low byte is blue, then green, red, with the high byte alpha. Every
/// channel is multiplied by `scale` (the per-light intensity) and the fixed
/// `1/255` normaliser, and returned in destination order `[R, G, B, A]` so the
/// caller writes four contiguous floats.
///
/// The four channels run as one vector rather than the reference's four
/// `movzx` / `cvtsi2ss` / `mulss` triples. `pmovzxbd` zero-extends the word's
/// bytes into lane order `(B, G, R, A)`; the `0xc6` dword shuffle reorders them
/// to `(R, G, B, A)`; one `mulps` against a broadcast of `k` presents every
/// lane the same operand pair the scalar multiplies did, `k` being the single
/// `scale * 1/255` product the reference also folds once before its four
/// multiplies. The byte operands are 0..255 and so exactly representable, which
/// is what makes `cvtdq2ps` and the per-channel `cvtsi2ss` agree bit for bit.
/// The extend and the shuffle are one permutation of a dword, so the backend is
/// free to emit them as a single `pshufb` against a constant mask, and does.
pub fn unpack_bgra_scaled(packed: u32, scale: f32) -> [f32; 4] {
    #[cfg(target_arch = "x86")]
    use core::arch::x86::{
        _mm_cvtepi32_ps, _mm_cvtepu8_epi32, _mm_cvtsi32_si128, _mm_mul_ps, _mm_set1_ps,
        _mm_shuffle_epi32, _mm_storeu_ps,
    };
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::{
        _mm_cvtepi32_ps, _mm_cvtepu8_epi32, _mm_cvtsi32_si128, _mm_mul_ps, _mm_set1_ps,
        _mm_shuffle_epi32, _mm_storeu_ps,
    };

    let k = scale * INV_255;
    // SAFETY: moves a general register into the low dword of a vector; the
    // intrinsic reads no memory and has no operand precondition.
    let word = unsafe { _mm_cvtsi32_si128(packed.cast_signed()) };
    // SAFETY: byte-to-dword zero-extend of an initialized vector. SSE4.1 is in
    // both baselines this crate builds for: nehalem on the client target,
    // penryn on the host target the kernels are tested under.
    let bgra = unsafe { _mm_cvtepu8_epi32(word) };
    // SAFETY: dword shuffle of an initialized vector. `0xc6` is `11 00 01 10`,
    // which selects source lanes 2, 1, 0, 3, taking `(B, G, R, A)` to
    // `(R, G, B, A)`.
    let rgba = unsafe { _mm_shuffle_epi32::<0xc6>(bgra) };
    // SAFETY: lane-wise signed-int-to-float convert of an initialized vector.
    let floats = unsafe { _mm_cvtepi32_ps(rgba) };
    // SAFETY: broadcasts a scalar across four lanes; reads no memory.
    let kv = unsafe { _mm_set1_ps(k) };
    // SAFETY: lane-wise multiply of two initialized vectors.
    let scaled = unsafe { _mm_mul_ps(floats, kv) };
    let mut out = [0.0_f32; 4];
    // SAFETY: the store covers the 16 in-bounds bytes of the local `[f32; 4]`,
    // and is the unaligned form, so `out`'s own alignment is not a precondition.
    unsafe { _mm_storeu_ps(out.as_mut_ptr(), scaled) };
    out
}

#[cfg(test)]
mod tests_unpack_bgra_scaled {
    use super::{INV_255, unpack_bgra_scaled};

    /// Per-channel oracle for the packed kernel, one lane at a time.
    ///
    /// The shape the kernel replaced: one `k`, four byte extracts, four
    /// multiplies, assembled in destination order.
    fn ref_unpack(packed: u32, scale: f32) -> [f32; 4] {
        let k = scale * INV_255;
        let r = ((packed >> 0x10) & 0xff) as f32;
        let g = ((packed >> 0x08) & 0xff) as f32;
        let b = (packed & 0xff) as f32;
        let a = ((packed >> 0x18) & 0xff) as f32;
        [r * k, g * k, b * k, a * k]
    }

    #[test]
    fn bit_exact_against_scalar_oracle() {
        let words = [
            0x0000_0000u32,
            0xffff_ffff,
            0x0012_3456,
            0x00ab_cdef,
            0x8040_2010,
            0x7f3f_1f0f,
            0xdead_beef,
            0x0000_00ff,
            0x00ff_0000,
            0xff00_0000,
        ];
        // Deliberately all finite: `0 * inf` is an invalid operation whose
        // default `QNaN` the host hardware and a constant-folding compiler
        // spell with opposite sign bits, so it is not a bit-comparable input.
        let scales = [0.0f32, 1.0, -1.0, 0.5, 255.0, 3.7, 1e-30, 1e30];
        for &w in &words {
            for &s in &scales {
                let got = unpack_bgra_scaled(w, s);
                let want = ref_unpack(w, s);
                for i in 0..4 {
                    assert_eq!(
                        got[i].to_bits(),
                        want[i].to_bits(),
                        "w={w:#010x} s={s} lane {i}"
                    );
                }
            }
        }
    }

    /// A non-finite scale reaches every lane the same way in both forms.
    ///
    /// Compared as classes rather than bits: the invalid `0 * inf` lanes carry
    /// a `QNaN` whose sign a constant-folded oracle need not agree on.
    #[test]
    fn non_finite_scale_matches_scalar_oracle() {
        for scale in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let got = unpack_bgra_scaled(0x00ff_0000, scale);
            let want = ref_unpack(0x00ff_0000, scale);
            for i in 0..4 {
                assert_eq!(got[i].is_nan(), want[i].is_nan(), "scale {scale} lane {i}");
                if !got[i].is_nan() {
                    assert_eq!(
                        got[i].to_bits(),
                        want[i].to_bits(),
                        "scale {scale} lane {i}"
                    );
                }
            }
        }
    }

    #[test]
    fn known_white_unit_scale() {
        // Every byte 0xff with unit scale -> 1.0 in all four channels.
        let out = unpack_bgra_scaled(0xffff_ffff, 1.0);
        for c in out {
            assert!((c - 1.0).abs() < 1e-6, "{out:?}");
        }
    }

    #[test]
    fn channel_routing() {
        // packed = A<<24 | R<<16 | G<<8 | B; scale*1/255 == 1 -> raw byte values.
        let packed = (0x80u32 << 24) | (0x40 << 16) | (0x20 << 8) | 0x10;
        let out = unpack_bgra_scaled(packed, 255.0);
        assert!((out[0] - 64.0).abs() < 1e-4, "R {}", out[0]);
        assert!((out[1] - 32.0).abs() < 1e-4, "G {}", out[1]);
        assert!((out[2] - 16.0).abs() < 1e-4, "B {}", out[2]);
        assert!((out[3] - 128.0).abs() < 1e-4, "A {}", out[3]);
    }

    #[test]
    fn black_is_zero() {
        assert_eq!(unpack_bgra_scaled(0x0000_0000, 3.7), [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn scale_zero_is_zero() {
        assert_eq!(unpack_bgra_scaled(0xdead_beef, 0.0), [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn linear_in_scale() {
        // Doubling the scale doubles every channel.
        let one = unpack_bgra_scaled(0x1234_5678, 1.5);
        let two = unpack_bgra_scaled(0x1234_5678, 3.0);
        for i in 0..4 {
            assert!(
                (two[i] - 2.0 * one[i]).abs() < 1e-5,
                "i={i} {one:?} {two:?}"
            );
        }
    }

    #[test]
    fn bounds_unit_scale() {
        // Unit scale lands every channel in [0, 1].
        for packed in [0x0000_0000u32, 0x8040_2010, 0xffff_ffff, 0x7f3f_1f0f] {
            let out = unpack_bgra_scaled(packed, 1.0);
            for c in out {
                assert!((0.0..=1.0).contains(&c), "{packed:#x} -> {out:?}");
            }
        }
    }

    #[test]
    fn channels_isolated() {
        // Only the masked byte of each channel contributes; no cross-bleed.
        let only_red = unpack_bgra_scaled(0x00ff_0000, 255.0);
        assert!((only_red[0] - 255.0).abs() < 1e-3);
        assert_eq!([only_red[1], only_red[2], only_red[3]], [0.0, 0.0, 0.0]);
    }
}

/// Packs the 28-float fog/light gradient block.
///
/// From 27 source params and the eight per-column scale constants.
///
/// `p` holds the seven rows of source parameters (27 contiguous floats). The
/// scale arguments name their reference-DLL global by suffix: `c4`, `c0`, `bc`,
/// `b8`, `b4`, `b0`, `ac`, `a8`. The last output lane is the homogeneous 1.0.
// The 27-float source block and the eight per-column scale globals are the
// stock routine's own argument list, fixed by the convention it is called
// through. Folding them into a params struct would misdescribe the ABI.
#[allow(clippy::too_many_arguments)]
pub fn gx_light_pack_fog_gradient_block__71c4e0(
    p: &[f32; 27],
    c4: f32,
    c0: f32,
    bc: f32,
    b8: f32,
    b4: f32,
    b0: f32,
    ac: f32,
    a8: f32,
) -> [f32; 28] {
    let mut out = [0.0_f32; 28];
    // Lanes 0-11 are three packed rows; see `fog_gradient_row__71c4e0`.
    fog_gradient_row__71c4e0::<0, 0>(p, &mut out, c4, c0, bc, b8);
    fog_gradient_row__71c4e0::<9, 4>(p, &mut out, c4, c0, bc, b8);
    fog_gradient_row__71c4e0::<18, 8>(p, &mut out, c4, c0, bc, b8);
    // Lanes 12-27 stay lane-wise: their source floats are strided, not a
    // contiguous quad, and each column takes its own scale.
    out[12] = p[4] * b4;
    out[13] = p[5] * b0;
    out[14] = p[6] * ac;
    out[15] = p[7] * b0;
    out[16] = p[13] * b4;
    out[17] = p[14] * b0;
    out[18] = p[15] * ac;
    out[19] = p[16] * b0;
    out[20] = p[22] * b4;
    out[21] = p[23] * b0;
    out[22] = p[24] * ac;
    out[23] = p[25] * b0;
    out[24] = p[8] * a8;
    out[25] = p[17] * a8;
    out[26] = p[26] * a8;
    out[27] = 1.0;
    out
}

/// Builds one packed four-lane row of the fog/light gradient block's lanes 0-11.
///
/// `SRC` is the row's source index (0, 9, 18) and `DST` its output lane (0, 4,
/// 8). `p[SRC..SRC + 4]` is contiguous, so the row is one 16-byte load, one
/// permutation to `(3, 1, 2, 0)`, one multiply by the shared column vector
/// `(c4, c4, c0, bc)`, and `p[SRC + 6] * b8` subtracted in lane 3 only.
///
/// Lanes 0-2 subtract `+0.0`, which is the identity for every value a `mulps`
/// can produce: finite, either zero (`-0.0 - +0.0` stays `-0.0`), infinite, and
/// quiet NaN. The constant must stay `+0.0`, because a `-0.0` would turn a
/// stored `-0.0` lane into `+0.0`.
#[inline]
fn fog_gradient_row__71c4e0<const SRC: usize, const DST: usize>(
    p: &[f32; 27],
    out: &mut [f32; 28],
    c4: f32,
    c0: f32,
    bc: f32,
    b8: f32,
) {
    #[cfg(target_arch = "x86")]
    use core::arch::x86::{
        _mm_loadu_ps, _mm_mul_ps, _mm_set_ps, _mm_shuffle_ps, _mm_storeu_ps, _mm_sub_ps,
    };
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::{
        _mm_loadu_ps, _mm_mul_ps, _mm_set_ps, _mm_shuffle_ps, _mm_storeu_ps, _mm_sub_ps,
    };
    // Every intrinsic below is SSE1, available on every ISA baseline this crate
    // builds for. The two const parameters are checked here so the loads and the
    // store below need no runtime test.
    const { assert!(SRC + 6 < 27 && DST + 4 <= 28) };

    // SAFETY: `SRC + 6 < 27` is const-checked above, so `SRC` and the three
    // floats after it are inside `p`.
    let quad_p = unsafe { p.as_ptr().add(SRC) };
    // SAFETY: as above; the 16 bytes read end well before `p`'s last float, and
    // `_mm_loadu_ps` has no alignment requirement.
    let quad = unsafe { _mm_loadu_ps(quad_p) };
    // SAFETY: `_mm_shuffle_ps` is a register lane permute with no memory or
    // aliasing precondition; the mask selects lanes 3, 1, 2, 0 of `quad`.
    let perm = unsafe { _mm_shuffle_ps::<0x27>(quad, quad) };
    // SAFETY: `_mm_set_ps` builds a register from four values; no precondition.
    let cols = unsafe { _mm_set_ps(bc, c0, c4, c4) };
    // SAFETY: lane-wise multiply of two initialized vectors.
    let scaled = unsafe { _mm_mul_ps(perm, cols) };
    // SAFETY: as for `cols`; lanes 0-2 are `+0.0` per this function's contract.
    let minus = unsafe { _mm_set_ps(p[SRC + 6] * b8, 0.0, 0.0, 0.0) };
    // SAFETY: lane-wise subtract of two initialized vectors.
    let row = unsafe { _mm_sub_ps(scaled, minus) };
    // SAFETY: `DST + 4 <= 28` is const-checked above, so `DST` and the three
    // lanes after it are inside `out`.
    let row_p = unsafe { out.as_mut_ptr().add(DST) };
    // SAFETY: as above; the 16 bytes written are in bounds of `out`, which this
    // call borrows exclusively, and `_mm_storeu_ps` has no alignment requirement.
    unsafe { _mm_storeu_ps(row_p, row) };
}

#[cfg(test)]
mod tests_gx_light_pack_fog_gradient_block__71c4e0 {
    use super::gx_light_pack_fog_gradient_block__71c4e0 as f;

    #[test]
    fn homogeneous_lane_is_one() {
        let p = [0.0_f32; 27];
        let out = f(&p, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0);
        assert_eq!(out[27], 1.0);
    }

    #[test]
    fn unit_scales_pass_params_through() {
        let p = [
            1.0_f32, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0, 5.5, 6.0, 6.5, 7.0, 7.5, 8.0, 8.5,
            9.0, 9.5, 10.0, 10.5, 11.0, 11.5, 12.0, 12.5, 13.0, 13.5, 14.0,
        ];
        // zero the b8 scale so the two-term lanes reduce to col-bc only
        let out = f(&p, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0);
        assert_eq!(out[0], p[3]);
        assert_eq!(out[1], p[1]);
        assert_eq!(out[2], p[2]);
        assert_eq!(out[3], p[0]);
        assert_eq!(out[7], p[9]);
        assert_eq!(out[11], p[18]);
        assert_eq!(out[24], p[8]);
        assert_eq!(out[26], p[26]);
        assert_eq!(out[27], 1.0);
    }

    #[test]
    fn subtract_columns_apply() {
        let mut p = [0.0_f32; 27];
        p[0] = 2.0;
        p[6] = 3.0;
        p[9] = 5.0;
        p[15] = 7.0;
        p[18] = 11.0;
        p[24] = 13.0;
        let out = f(&p, 1.0, 1.0, 2.0, 4.0, 1.0, 1.0, 1.0, 1.0);
        assert_eq!(out[3], 2.0 * 2.0 - 3.0 * 4.0);
        assert_eq!(out[7], 5.0 * 2.0 - 7.0 * 4.0);
        assert_eq!(out[11], 11.0 * 2.0 - 13.0 * 4.0);
    }

    #[test]
    fn packed_rows_match_the_scalar_lanes_bit_for_bit() {
        // Lanes 0-11 run as three packed rows; the lane-wise expressions they
        // replaced are the oracle, compared on bits so a signed zero or a NaN
        // payload cannot pass as equal.
        let odd = [
            0.0f32,
            -0.0,
            1.0,
            -1.0,
            f32::MIN_POSITIVE,
            -f32::MIN_POSITIVE,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
            -7.25,
            1e30,
            1e-30,
        ];
        for rot in 0..odd.len() {
            let mut p = [0.0f32; 27];
            for (i, slot) in p.iter_mut().enumerate() {
                *slot = odd[(i + rot) % odd.len()];
            }
            let c4 = odd[rot % odd.len()];
            let c0 = odd[(rot + 3) % odd.len()];
            let bc = odd[(rot + 5) % odd.len()];
            let b8 = odd[(rot + 7) % odd.len()];
            let out = f(&p, c4, c0, bc, b8, 1.0, 1.0, 1.0, 1.0);
            for k in 0..3usize {
                let s = k * 9;
                let d = k * 4;
                assert_eq!(
                    out[d].to_bits(),
                    (p[s + 3] * c4).to_bits(),
                    "rot {rot} row {k} lane 0"
                );
                assert_eq!(
                    out[d + 1].to_bits(),
                    (p[s + 1] * c4).to_bits(),
                    "rot {rot} row {k} lane 1"
                );
                assert_eq!(
                    out[d + 2].to_bits(),
                    (p[s + 2] * c0).to_bits(),
                    "rot {rot} row {k} lane 2"
                );
                assert_eq!(
                    out[d + 3].to_bits(),
                    (p[s] * bc - p[s + 6] * b8).to_bits(),
                    "rot {rot} row {k} lane 3"
                );
            }
        }
    }

    #[test]
    fn scaling_is_linear() {
        let p = [
            -13.0_f32, -12.0, -11.0, -10.0, -9.0, -8.0, -7.0, -6.0, -5.0, -4.0, -3.0, -2.0, -1.0,
            0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0,
        ];
        let a = f(&p, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0);
        let b = f(&p, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0);
        for i in 0..27 {
            assert!(
                (b[i] - 2.0 * a[i]).abs() < 1e-4,
                "lane {i}: {} vs {}",
                b[i],
                a[i]
            );
        }
        assert_eq!(a[27], 1.0);
        assert_eq!(b[27], 1.0);
    }
}

/// Builds the cached ambient light direction.
///
/// Plus the transformed primary light direction and the updated
/// light-direction accumulator.
///
/// `dir` is the raw ambient direction, `m` the row-major 3x3 orientation,
/// `light` the primary light vector, `accum_in` the prior accumulator. `eps_sq`
/// is the squared-length floor below which the direction collapses to
/// `(0, 0, -1)`, `eps` the length floor under which the normalize step is
/// skipped, `k_inv` the normalize numerator (1.0 in the reference), `scale_a`
/// and `scale_b` the two per-frame light scales. Returns
/// `(ambient_dir, light_dir0, accum_out)`.
// Three vectors, a row-major 3x3 orientation and five loose float scales: the
// parameter list is the stock routine's, dictated by its calling convention
// rather than chosen, so there is no grouping to add without leaving the ABI.
#[allow(clippy::too_many_arguments)]
pub fn gx_light_prepare_ambient_and_light_dirs__71c2f0(
    dir: &[f32; 3],
    m: &[f32; 9],
    light: &[f32; 3],
    accum_in: &[f32; 3],
    eps_sq: f32,
    eps: f32,
    k_inv: f32,
    scale_a: f32,
    scale_b: f32,
) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let mut a = *dir;
    let sq = a[0] * a[0] + a[1] * a[1] + a[2] * a[2];
    if sq <= eps_sq {
        a = [0.0, 0.0, -1.0];
    } else {
        let len = sq.sqrt();
        if len.abs() >= eps {
            let inv = k_inv / len;
            a[0] *= inv;
            a[1] *= inv;
            a[2] *= inv;
        }
    }

    // Row-major 3x3 times the ambient direction.
    let r0 = m[0] * a[0] + m[1] * a[1] + m[2] * a[2];
    let r1 = m[3] * a[0] + m[4] * a[1] + m[5] * a[2];
    let r2 = m[6] * a[0] + m[7] * a[1] + m[8] * a[2];

    let s0 = light[0] * scale_a;
    let s1 = light[1] * scale_a;
    let s2 = light[2] * scale_a;

    let t0 = r0 * scale_b;
    let t1 = r1 * scale_b;
    let t2 = r2 * scale_b;

    let light_dir0 = [t0 - s0, t1 - s1, t2 - s2];

    let accum_out = [
        accum_in[0] + (light[0] - r0) * scale_a,
        accum_in[1] + (light[1] - r1) * scale_a,
        accum_in[2] + (light[2] - r2) * scale_a,
    ];

    (a, light_dir0, accum_out)
}

#[cfg(test)]
mod tests_gx_light_prepare_ambient_and_light_dirs__71c2f0 {
    use super::gx_light_prepare_ambient_and_light_dirs__71c2f0 as f;

    fn mag(v: &[f32; 3]) -> f32 {
        (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
    }

    #[test]
    fn normalizes_to_unit_with_identity() {
        let dir = [3.0_f32, 0.0, 4.0]; // length 5
        let m = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        let light = [0.0_f32, 0.0, 0.0];
        let accum = [0.0_f32, 0.0, 0.0];
        let (a, ld0, acc) = f(&dir, &m, &light, &accum, 1e-6, 1e-6, 1.0, 1.0, 1.0);
        assert!(
            (mag(&a) - 1.0).abs() < 1e-5,
            "ambient not unit: {}",
            mag(&a)
        );
        assert!((ld0[0] - 0.6).abs() < 1e-5);
        assert!((ld0[2] - 0.8).abs() < 1e-5);
        assert!((acc[0] + 0.6).abs() < 1e-5);
        assert!((acc[2] + 0.8).abs() < 1e-5);
    }

    #[test]
    fn collapses_to_down_when_below_eps() {
        let dir = [0.0_f32, 0.0, 0.0];
        let m = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        let light = [1.0_f32, 2.0, 3.0];
        let accum = [10.0_f32, 20.0, 30.0];
        let (a, _ld0, _acc) = f(&dir, &m, &light, &accum, 1e-6, 1e-6, 1.0, 2.0, 0.5);
        assert_eq!(a, [0.0, 0.0, -1.0]);
    }

    #[test]
    fn accumulator_adds_onto_prior() {
        let dir = [0.0_f32, 0.0, -1.0]; // already unit
        let m = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        let light = [4.0_f32, 4.0, 4.0];
        let accum = [100.0_f32, 200.0, 300.0];
        let scale_a = 0.25_f32;
        let (a, _ld0, acc) = f(&dir, &m, &light, &accum, 1e-6, 1e-6, 1.0, scale_a, 1.0);
        assert!((acc[0] - (100.0 + (4.0 - 0.0) * 0.25)).abs() < 1e-4);
        assert!((acc[1] - (200.0 + (4.0 - 0.0) * 0.25)).abs() < 1e-4);
        assert!((acc[2] - (300.0 + (4.0 - (-1.0)) * 0.25)).abs() < 1e-4);
        assert_eq!(a, [0.0, 0.0, -1.0]);
    }

    #[test]
    fn rotation_preserves_unit_length_of_ambient() {
        let dir = [1.0_f32, 0.0, 0.0];
        // row-major rotation about Z by 90deg
        let m = [0.0, -1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        let light = [0.0_f32, 0.0, 0.0];
        let accum = [0.0_f32, 0.0, 0.0];
        let (a, ld0, _acc) = f(&dir, &m, &light, &accum, 1e-6, 1e-6, 1.0, 1.0, 1.0);
        assert!((mag(&a) - 1.0).abs() < 1e-5);
        assert!((ld0[0] - 0.0).abs() < 1e-5);
        assert!((ld0[1] - 1.0).abs() < 1e-5);
        assert!((ld0[2] - 0.0).abs() < 1e-5);
    }
}

/// Per-frame color/intensity interpolation step for an animated light object.
///
/// Marches each of the three current color bytes toward its target by an
/// integer increment derived from the frame delta time, clamping at the target.
/// If no channel moved and the object is not in manual mode, the color snaps to
/// the host default. Then steps the intensity float toward its target by a
/// separate delta-time-scaled increment, clamping at the target.
///
/// Returns `(stepped_color, snap, new_intensity)`. `stepped_color` packs the
/// three marched bytes into the low 24 bits (byte 0 = the `+0x9c` channel,
/// byte 1 = `+0x9d`, byte 2 = `+0x9e`), top byte zero. `snap` is true when no
/// channel moved and the object is not in manual mode — the caller then writes
/// the host default dword to both the live and target color slots.
// The ten parameters are the fields the stock body reads off the light object
// and the frame state, passed positionally so the kernel stays free of the
// object layout. The count is a fact about the call, not a design choice.
#[allow(clippy::too_many_arguments)]
pub fn light_animate_color_and_intensity__69e770(
    color: [u8; 3],
    target_color: [u8; 3],
    color_scale_a: f32,
    color_scale_b: f32,
    dt: f32,
    manual: bool,
    intensity: f32,
    target_intensity: f32,
    intensity_scale: f32,
    threshold: f32,
) -> (u32, bool, f32) {
    let mut cur = [
        i32::from(color[0]),
        i32::from(color[1]),
        i32::from(color[2]),
    ];
    let tgt = [
        i32::from(target_color[0]),
        i32::from(target_color[1]),
        i32::from(target_color[2]),
    ];

    // Some per-channel delta is non-zero exactly when the two byte triples
    // differ, so this is the same flag the march used to raise from inside the
    // loop, and it is also the only condition under which the increment below
    // is read, since every use of it sits in a `delta != 0` arm. Deciding it up
    // front keeps the two scale loads and the saturating conversion off the
    // settled path, which is the one a light that has reached its target takes
    // every frame.
    let any_changed = color != target_color;
    if any_changed {
        // Color increment: truncate-toward-zero of the scaled dt, floored at 1.
        let raw = (dt * color_scale_a * color_scale_b) as i32;
        let inc = if raw < 1 { 1 } else { raw };

        for (c, t) in cur.iter_mut().zip(tgt.iter()) {
            let delta = *t - *c;
            if delta != 0 {
                if delta > 0 {
                    let v = *c + inc;
                    *c = if v > *t { *t } else { v };
                } else {
                    let v = *c - inc;
                    *c = if v < *t { *t } else { v };
                }
            }
        }
    }

    let snap = !manual && !any_changed;
    let stepped_color =
        (cur[0] as u32 & 0xff) | ((cur[1] as u32 & 0xff) << 8) | ((cur[2] as u32 & 0xff) << 16);

    // Intensity step: delta-time-scaled increment toward the target, clamped.
    let iinc = intensity_scale * dt;
    let diff = intensity - target_intensity;
    let new_intensity = if diff == threshold {
        intensity
    } else if diff > threshold {
        // current above target: step down, clamp at target.
        let v = intensity - iinc;
        if v < target_intensity {
            target_intensity
        } else {
            v
        }
    } else {
        // current below target: step up, clamp at target.
        let v = intensity + iinc;
        if v > target_intensity {
            target_intensity
        } else {
            v
        }
    };

    (stepped_color, snap, new_intensity)
}

#[cfg(test)]
mod tests_light_animate_color_and_intensity__69e770 {
    use super::light_animate_color_and_intensity__69e770 as f;

    fn bytes(packed: u32) -> [u8; 3] {
        [
            (packed & 0xff) as u8,
            ((packed >> 8) & 0xff) as u8,
            ((packed >> 16) & 0xff) as u8,
        ]
    }

    #[test]
    fn color_marches_up_and_clamps() {
        let (c, snap, _) = f(
            [10, 20, 30],
            [100, 100, 100],
            1.0,
            1.0,
            1000.0,
            true,
            0.0,
            0.0,
            0.0,
            0.0,
        );
        assert_eq!(bytes(c), [100, 100, 100]);
        assert!(!snap);
    }

    #[test]
    fn color_marches_down_by_one() {
        let (c, snap, _) = f(
            [50, 50, 50],
            [40, 40, 40],
            0.0,
            0.0,
            0.0,
            true,
            0.0,
            0.0,
            0.0,
            0.0,
        );
        assert_eq!(bytes(c), [49, 49, 49]);
        assert!(!snap);
    }

    #[test]
    fn no_change_non_manual_requests_snap() {
        let (_, snap, _) = f(
            [7, 8, 9],
            [7, 8, 9],
            1.0,
            1.0,
            10.0,
            false,
            0.0,
            0.0,
            0.0,
            0.0,
        );
        assert!(snap);
    }

    #[test]
    fn no_change_manual_keeps_color() {
        let (c, snap, _) = f(
            [7, 8, 9],
            [7, 8, 9],
            1.0,
            1.0,
            10.0,
            true,
            0.0,
            0.0,
            0.0,
            0.0,
        );
        assert_eq!(bytes(c), [7, 8, 9]);
        assert!(!snap);
    }

    #[test]
    fn changed_non_manual_does_not_snap() {
        let (_, snap, _) = f(
            [0, 8, 9],
            [10, 8, 9],
            1.0,
            1.0,
            10.0,
            false,
            0.0,
            0.0,
            0.0,
            0.0,
        );
        assert!(!snap);
    }

    /// A settled channel stays put while its neighbours march.
    ///
    /// The march is entered on the whole triple differing, so the per-channel
    /// `delta != 0` test still has to hold each settled channel in place, and
    /// the increment still has to reach the two that move.
    #[test]
    fn settled_channel_holds_while_others_march() {
        let (c, snap, _) = f(
            [10, 50, 30],
            [40, 50, 20],
            1.0,
            1.0,
            5.0,
            false,
            0.0,
            0.0,
            0.0,
            0.0,
        );
        assert_eq!(bytes(c), [15, 50, 25]);
        assert!(!snap);
    }

    #[test]
    fn intensity_equal_is_noop() {
        let (_, _, i) = f(
            [0, 0, 0],
            [0, 0, 0],
            0.0,
            0.0,
            0.0,
            true,
            0.5,
            0.5,
            1.0,
            0.0,
        );
        assert_eq!(i.to_bits(), 0.5_f32.to_bits());
    }

    #[test]
    fn intensity_steps_up_and_clamps() {
        let (_, _, i) = f(
            [0, 0, 0],
            [0, 0, 0],
            0.0,
            0.0,
            1.0,
            true,
            0.0,
            1.0,
            5.0,
            0.0,
        );
        assert_eq!(i.to_bits(), 1.0_f32.to_bits());
    }

    #[test]
    fn intensity_steps_down_partial() {
        let (_, _, i) = f(
            [0, 0, 0],
            [0, 0, 0],
            0.0,
            0.0,
            0.25,
            true,
            1.0,
            0.0,
            1.0,
            0.0,
        );
        assert_eq!(i.to_bits(), 0.75_f32.to_bits());
    }

    #[test]
    fn intensity_steps_down_and_clamps() {
        let (_, _, i) = f(
            [0, 0, 0],
            [0, 0, 0],
            0.0,
            0.0,
            1.0,
            true,
            1.0,
            0.0,
            5.0,
            0.0,
        );
        assert_eq!(i.to_bits(), 0.0_f32.to_bits());
    }

    #[test]
    fn metamorphic_increment_floor_is_one() {
        let (c, _, _) = f(
            [0, 0, 0],
            [3, 0, 0],
            -100.0,
            1.0,
            1.0,
            true,
            0.0,
            0.0,
            0.0,
            0.0,
        );
        assert_eq!(bytes(c), [1, 0, 0]);
    }

    #[test]
    fn metamorphic_monotone_convergence() {
        let mut col = [0u8, 0, 0];
        for _ in 0..300 {
            let (c, _, _) = f(col, [200, 130, 60], 1.0, 1.0, 1.0, true, 0.0, 0.0, 0.0, 0.0);
            col = bytes(c);
        }
        assert_eq!(col, [200, 130, 60]);
    }
}

/// `GxLight::ApplySceneLighting` fog/sky-tail colour encode (0x71c902..0x71ca28).
///
/// Encodes three scene-fog colour channels into one packed `D3DCOLOR`. The stock
/// body reads the three floats at `this+0x190/+0x194/+0x198`, runs each
/// independently through the clamp-to-`[0,1]` + remap `v*255.0 + 0.5` +
/// truncate-toward-zero encode (the same per-lane math as the `byte` closure of
/// [`crate::math::gx::gx_light_clamp_color_component_to_byte__71ca80`], but with
/// NO reciprocal-max normalisation here), then packs them as `ARGB` with the
/// alpha lane forced to `0xff` (`or esi,0xffffff00` before the first `shl 8`).
/// The pack walks the channels high-to-low so `channels[2]` (`this+0x198`) lands
/// in the R byte (bits 16..23), `channels[1]` (`this+0x194`) in G (8..15), and
/// `channels[0]` (`this+0x190`) in B (0..7).
///
/// The caller decides *whether* to encode (the `|fog| >= eps` gate at 0x71c910);
/// this leaf only performs the encode and is what makes the tail host-testable.
// The low clamp is written `!(0.0 < v)` so an unordered compare takes the
// zero branch, matching the stock ordered test; a `v <= 0.0` rewrite would
// route NaN into the remap instead. NaN polarity is load-bearing here.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn encode_fog_color__71c730(channels: &[f32; 3]) -> u32 {
    // Per-channel clamp+remap+truncate matching the stock x87 path: the low
    // clamp `!(0.0 < v)` (TEST AH,0x41/JP at 0x71c930) snaps to `0.0` on
    // not-greater-or-unordered, the high clamp `v < 1.0` (TEST AH,0x1/JNE at
    // 0x71c948) saturates to `255.0`, and the `__ftol` truncation is the
    // `mapped as i32 as u32` round-toward-zero cast (matching `ftol__40a2b0`).
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
    // Stock packs ascending source offsets into descending byte lanes (x87 FLD
    // order +0x198,+0x194,+0x190; the three `__ftol` pops reverse it): channels[0]
    // =+0x190 -> bits 16..23, [1]=+0x194 -> bits 8..15, [2]=+0x198 -> bits 0..7.
    0xff00_0000 | (byte(channels[0]) << 16) | (byte(channels[1]) << 8) | byte(channels[2])
}

#[cfg(test)]
mod tests_encode_fog_color__71c730 {
    use super::encode_fog_color__71c730 as encode;

    /// Reference per-lane encode lifted from the stock per-channel math.
    // Transcribes the kernel's `!(0.0 < v)` low clamp verbatim, NaN polarity
    // included; an ordered rewrite would leave the reference disagreeing with
    // the code it exists to check on exactly the input that distinguishes them.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    fn ref_byte(v: f32) -> u32 {
        let mapped = if !(0.0 < v) {
            0.0_f32
        } else if v < 1.0 {
            v * 255.0 + 0.5
        } else {
            255.0_f32
        };
        (mapped as i32 as u32) & 0xff
    }

    #[test]
    fn alpha_lane_is_always_ff() {
        assert_eq!((encode(&[0.0, 0.0, 0.0]) >> 24) & 0xff, 0xff);
        assert_eq!((encode(&[1.0, 1.0, 1.0]) >> 24) & 0xff, 0xff);
        assert_eq!((encode(&[-3.0, 5.0, 0.5]) >> 24) & 0xff, 0xff);
    }

    #[test]
    fn channel_to_byte_lane_mapping_matches_stock() {
        // Stock routes channels[0]=+0x190 -> bits 16..23, [1]=+0x194 -> bits 8..15,
        // [2]=+0x198 -> bits 0..7 (ascending source offset, descending byte lane).
        let p = encode(&[0.125, 0.25, 0.5]);
        assert_eq!((p >> 16) & 0xff, ref_byte(0.125)); // channels[0]=0x190 -> high lane
        assert_eq!((p >> 8) & 0xff, ref_byte(0.25)); // channels[1]=0x194
        assert_eq!(p & 0xff, ref_byte(0.5)); // channels[2]=0x198 -> low lane
    }

    #[test]
    fn negative_clamps_low_to_zero() {
        let p = encode(&[-1.0, -0.0, -1e9]);
        assert_eq!(p & 0xff, 0);
        assert_eq!((p >> 8) & 0xff, 0);
        assert_eq!((p >> 16) & 0xff, 0);
        assert_eq!((p >> 24) & 0xff, 0xff);
    }

    #[test]
    fn at_or_above_one_saturates_to_255() {
        let p = encode(&[1.0, 2.5, 1e30]);
        assert_eq!(p & 0xff, 255);
        assert_eq!((p >> 8) & 0xff, 255);
        assert_eq!((p >> 16) & 0xff, 255);
    }

    #[test]
    fn nan_takes_low_clamp_branch_to_zero() {
        // The stock low clamp drives a not-positive-or-NaN channel to `0.0`
        // (`fcomp` against `0.0`; a NaN is unordered, C0=C2=C3). `!(0.0 < v)`
        // mirrors it: `0.0 < NaN` is false, so the kernel loads `0.0`. NaN fog
        // is a measure-zero, smoke-accepted divergence either way.
        let p = encode(&[f32::NAN, f32::NAN, f32::NAN]);
        assert_eq!(p & 0xff, 0);
        assert_eq!((p >> 8) & 0xff, 0);
        assert_eq!((p >> 16) & 0xff, 0);
        assert_eq!((p >> 24) & 0xff, 0xff);
    }

    #[test]
    fn half_rounds_via_plus_half_bias() {
        // 0.5 * 255 + 0.5 = 128.0 -> truncates to 128.
        assert_eq!(encode(&[0.5, 0.5, 0.5]) & 0xff, 128);
    }

    #[test]
    fn midrange_is_bit_exact_to_reference() {
        // channels[0] -> bits 16..23, [1] -> bits 8..15, [2] -> bits 0..7 (stock pack).
        for &c0 in &[0.0f32, 0.1, 0.3, 0.49, 0.5, 0.75, 0.999] {
            for &c1 in &[0.0f32, 0.2, 0.6, 1.0] {
                for &c2 in &[0.0f32, 0.4, 0.8, 2.0] {
                    let p = encode(&[c0, c1, c2]);
                    let want =
                        0xff00_0000 | (ref_byte(c0) << 16) | (ref_byte(c1) << 8) | ref_byte(c2);
                    assert_eq!(p, want, "c0={c0} c1={c1} c2={c2}");
                }
            }
        }
    }
}

/// `ColorTrack::SampleARGB` per-channel colour lerp — the x87 hotspot at 0x6d631e–0x6d63c7.
///
/// Given the two bracketing keyframe colours (packed `0x??RRGGBB`, the alpha
/// byte ignored) and the interpolation fraction `frac` the keyframe search
/// produced, it linearly blends each of R/G/B and re-packs an opaque ARGB8888
/// (`0xff << 24`). The empty-track (`count == 0`) early-out that yields opaque
/// black lives in the adapter.
///
/// Each channel reproduces the stock magic-bias fast-round **bit-exactly**:
/// `fild(end-start) · fmul frac · fiadd start · fadd 512.0`, then the raw f32 bits
/// are `>> 14 & 0xff` (the 512.0f constant at `0x8029cc`, read at 0x6d6372). The
/// `+512.0` biases the value into `[512, 1024)`, whose f32 mantissa is exactly
/// `channel << 14`; the shift extracts the **truncated** integer (a floor, not a
/// rounded `as`-cast). Intermediates run in `f64` narrowed once to f32, which is
/// bit-identical to the x87 80-bit chain here: `(end-start)` is an integer in
/// `[-255,255]`, `frac` is an f32, so every term is exactly representable and only
/// the final store rounds — and any bit below `2^-14` is masked away by the shift,
/// so f64-vs-80-bit intermediate differences cannot reach the extracted byte.
pub fn color_track__sample_argb__6d62e0(start_color: u32, end_color: u32, frac: f32) -> u32 {
    // One channel at `shift` (16=R, 8=G, 0=B), mirroring the stock op order.
    let channel = |shift: u32| -> u32 {
        let start = ((start_color >> shift) & 0xff) as i32;
        let end = ((end_color >> shift) & 0xff) as i32;
        let v = (end - start) as f64 * frac as f64 + start as f64 + 512.0;
        ((v as f32).to_bits() >> 14) & 0xff
    };
    let r = channel(16);
    let g = channel(8);
    let b = channel(0);
    0xff00_0000 | (r << 16) | (g << 8) | b
}

#[cfg(test)]
mod tests_color_track__sample_argb__6d62e0 {
    use super::color_track__sample_argb__6d62e0 as sample;

    /// Independent reference for one channel byte via the magic-bias round.
    fn ref_byte(start: u32, end: u32, shift: u32, frac: f32) -> u32 {
        let s = ((start >> shift) & 0xff) as i32;
        let e = ((end >> shift) & 0xff) as i32;
        let v = (e - s) as f64 * frac as f64 + s as f64 + 512.0;
        ((v as f32).to_bits() >> 14) & 0xff
    }

    #[test]
    fn alpha_lane_is_always_ff() {
        assert_eq!(sample(0x0000_0000, 0x00ff_ffff, 0.5) >> 24, 0xff);
        assert_eq!(sample(0x0012_3456, 0x00ab_cdef, 0.0) >> 24, 0xff);
    }

    #[test]
    fn frac_zero_returns_start_color_opaque() {
        // frac=0: (end-start)*0 + start = start, per channel.
        assert_eq!(sample(0x0011_2233, 0x00aa_bbcc, 0.0), 0xff11_2233);
        assert_eq!(sample(0x0000_0000, 0x00ff_ffff, 0.0), 0xff00_0000);
    }

    #[test]
    fn frac_one_returns_end_color_opaque() {
        // frac=1: (end-start)*1 + start = end, per channel.
        assert_eq!(sample(0x0011_2233, 0x00aa_bbcc, 1.0), 0xffaa_bbcc);
        assert_eq!(sample(0x00ff_ffff, 0x0000_0000, 1.0), 0xff00_0000);
    }

    #[test]
    fn alpha_byte_of_inputs_is_ignored() {
        // The R byte is read from bits 16..23; alpha (24..31) never participates.
        // Inputs share bits 0..23 and differ only in the alpha byte.
        let with_alpha = sample(0xde12_3456, 0xbe78_9abc, 0.5);
        let no_alpha = sample(0x0012_3456, 0x0078_9abc, 0.5);
        assert_eq!(with_alpha, no_alpha);
    }

    #[test]
    fn midpoint_truncates_not_rounds() {
        // start=0, end=255, frac=0.5 -> 127.5 -> bits>>14 truncates to 127.
        assert_eq!(sample(0x0000_0000, 0x0000_00ff, 0.5) & 0xff, 127);
        // Symmetric endpoint at frac 0.5 mirrors the same truncation.
        assert_eq!(sample(0x0000_00ff, 0x0000_0000, 0.5) & 0xff, 127);
    }

    #[test]
    fn channels_are_independent_and_ordered() {
        // Distinct per-channel endpoints stay in their R<<16 / G<<8 / B lanes.
        let p = sample(0x0010_2030, 0x0040_5060, 1.0);
        assert_eq!((p >> 16) & 0xff, 0x40);
        assert_eq!((p >> 8) & 0xff, 0x50);
        assert_eq!(p & 0xff, 0x60);
    }

    #[test]
    fn bit_exact_against_reference_sweep() {
        let colors = [
            0x0000_0000u32,
            0x00ff_ffff,
            0x0012_3456,
            0x00ab_cdef,
            0x0080_4020,
            0x0001_00ff,
        ];
        let fracs = [0.0f32, 0.1, 0.25, 0.333_333_34, 0.5, 0.6667, 0.75, 0.9, 1.0];
        for &s in &colors {
            for &e in &colors {
                for &f in &fracs {
                    let got = sample(s, e, f);
                    let want = 0xff00_0000
                        | (ref_byte(s, e, 16, f) << 16)
                        | (ref_byte(s, e, 8, f) << 8)
                        | ref_byte(s, e, 0, f);
                    assert_eq!(got, want, "s={s:#010x} e={e:#010x} f={f}");
                }
            }
        }
    }
}

/// Wrapped-segment membership test + interpolation fraction.
///
/// (`AngleTable__FindWrappedIntervalFraction` @0x6d63e0, per-segment body).
///
/// The keyframe-time table is cyclic with period 0xb40 (2880 day-time ticks).
/// An ascending segment (`hi > lo`) contains `query` iff `lo <= query <= hi`;
/// a wrapping segment (`hi <= lo`) iff `query <= hi || query >= lo`, and both
/// `hi` and (when `query < lo`) `query` are lifted by one period before the
/// divide — the stock `ADD 0xb40` pair at 0x6d642e/0x6d6437. The fraction is
/// `FILD (query−lo) ÷ FIDIV (hi−lo)` stored once to f32; an f64 divide
/// narrowed once is bit-identical (53 ≥ 2·24+2, so double rounding through
/// f64 — like the stock's through 80-bit — cannot move the f32). Degenerate
/// 0/0 (a wrapping segment whose raw span is exactly −0xb40, only possible
/// with out-of-period table values) yields the x87 real-indefinite `QNaN`
/// `0xffc0_0000`, not Rust's positive NaN; nonzero÷0 is the masked
/// zero-divide ±∞, which the f64 path already matches.
pub fn wrapped_interval_fraction__6d63e0(query: i32, lo: i32, hi: i32) -> Option<f32> {
    const PERIOD: i32 = 0xb40;
    let (dq, ds) = if hi > lo {
        if lo <= query && query <= hi {
            (query.wrapping_sub(lo), hi.wrapping_sub(lo))
        } else {
            return None;
        }
    } else if query <= hi || lo <= query {
        let hi = hi.wrapping_add(PERIOD);
        let query = if query < lo {
            query.wrapping_add(PERIOD)
        } else {
            query
        };
        (query.wrapping_sub(lo), hi.wrapping_sub(lo))
    } else {
        return None;
    };
    if ds == 0 && dq == 0 {
        return Some(f32::from_bits(0xffc0_0000));
    }
    Some(super::f64_to_f32(f64::from(dq) / f64::from(ds)))
}

#[cfg(test)]
mod tests_wrapped_interval_fraction__6d63e0 {
    use super::wrapped_interval_fraction__6d63e0 as frac;

    #[test]
    fn ascending_segment() {
        assert_eq!(frac(50, 0, 100).unwrap().to_bits(), 0.5f32.to_bits());
        assert_eq!(frac(0, 0, 100).unwrap().to_bits(), 0.0f32.to_bits());
        assert_eq!(frac(100, 0, 100).unwrap().to_bits(), 1.0f32.to_bits());
        // Narrowing pin: 1/3 divides in f64 and rounds once into f32.
        assert_eq!(
            frac(1, 0, 3).unwrap().to_bits(),
            ((1.0f64 / 3.0) as f32).to_bits()
        );
        assert!(frac(101, 0, 100).is_none());
        assert!(frac(-1, 0, 100).is_none());
    }

    #[test]
    fn wrapping_segment() {
        // lo=2800, hi=80 wraps through the seam; a query below it is lifted:
        // (40+2880−2800)/(80+2880−2800) = 120/160.
        assert_eq!(frac(40, 2800, 80).unwrap().to_bits(), 0.75f32.to_bits());
        assert_eq!(frac(2840, 2800, 80).unwrap().to_bits(), 0.25f32.to_bits());
        assert_eq!(frac(80, 2800, 80).unwrap().to_bits(), 1.0f32.to_bits());
        assert_eq!(frac(2800, 2800, 80).unwrap().to_bits(), 0.0f32.to_bits());
        assert!(frac(1000, 2800, 80).is_none());
    }

    /// Equal endpoints classify as a wrapping segment.
    ///
    /// `hi` is lifted a full period, so the span is 0xb40, never zero.
    #[test]
    fn equal_endpoints_span_full_period() {
        assert_eq!(frac(100, 100, 100).unwrap().to_bits(), 0.0f32.to_bits());
        assert_eq!(
            frac(100 + 720, 100, 100).unwrap().to_bits(),
            0.25f32.to_bits()
        );
    }

    /// Out-of-period tables can fold the span to zero.
    ///
    /// 0/0 must be the x87 real-indefinite `QNaN`, n/0 the masked-zero-divide
    /// infinity.
    #[test]
    fn degenerate_zero_span() {
        assert_eq!(frac(0xb40, 0xb40, 0).unwrap().to_bits(), 0xffc0_0000);
        assert_eq!(
            frac(0xb41, 0xb40, 0).unwrap().to_bits(),
            f32::INFINITY.to_bits()
        );
        assert_eq!(
            frac(-2, 0xb40, 0).unwrap().to_bits(),
            f32::NEG_INFINITY.to_bits()
        );
    }
}

/// Sky-dome channel rounding (`SkyDome__ComputeVertexColors` @0x6d0f50).
///
/// The stock chain is `FSTP f32` (one rounding of the lerp result), `FSUB bias`
/// at extended precision, `FISTP` (round-to-nearest-even), `(char)` truncate.
/// `round_ties_even` + the x87-free ftol keeps the i686 build free of the
/// f64→i64 cast's fld/fisttp leak.
pub fn sky_round_channel__6d0f50(v: f32, bias: f32) -> u8 {
    let r = (f64::from(v) - f64::from(bias)).round_ties_even();
    (crate::math::misc::ftol__40a2b0(r) & 0xff) as u8
}

/// Byte-lerp one color channel.
///
/// `u8 → FILD → Math_LerpFloat → round`. The int→f32 conversions are exact for
/// 0..255.
pub fn sky_lerp_channel__6d0f50(a: u8, b: u8, t: f32, bias: f32) -> u8 {
    let v = crate::math::misc::math_lerp_float__6d15f0(f32::from(a), f32::from(b), t);
    sky_round_channel__6d0f50(v, bias)
}

/// Endpoint half of the byte-lerp, for a loop whose endpoints do not move.
///
/// `Math_LerpFloat` @0x6d15f0 picks its arm from `a <= b`, and on the dome's ring
/// loop both endpoints are per-band constants while only the weight varies. This
/// runs the compare and the taken arm's own subtraction once, returning
/// `(a, |b - a|, a <= b)`; the difference keeps that arm's operand order so
/// `sky_lerp_apply__6d0f50` can reproduce the arm rather than reconstruct it.
pub fn sky_lerp_prep__6d0f50(a: u8, b: u8) -> (f32, f32, bool) {
    let (a, b) = (f32::from(a), f32::from(b));
    if a <= b {
        (a, b - a, true)
    } else {
        (a, a - b, false)
    }
}

/// Weight half of the byte-lerp, evaluating only the arm the prep selected.
///
/// `(b - a) * t + a` when the prep took `a <= b`, `a - (a - b) * t` otherwise:
/// the two operand orderings the original branches to keep apart, so the rounding
/// at the endpoints matches in both directions. The arms stay separate even where
/// the two differences are exact negations of each other, because the multiply
/// carries `t` through unchanged in one and negated in the other, and an
/// unordered `t` can see that.
pub fn sky_lerp_apply__6d0f50(prep: (f32, f32, bool), t: f32, bias: f32) -> u8 {
    let (a, diff, ascending) = prep;
    let v = if ascending {
        diff * t + a
    } else {
        a - diff * t
    };
    sky_round_channel__6d0f50(v, bias)
}

/// Pack three channel bytes with a forced 0xFF alpha.
///
/// Memory order `[c0, c1, c2, a]` (`CImVector__SetPackedBGRA(0xff, c2, c1, c0)`
/// inlined — both packer twins produce exactly this dword).
pub fn sky_pack_channels__6d0f50(c0: u8, c1: u8, c2: u8) -> u32 {
    u32::from(c0) | (u32::from(c1) << 8) | (u32::from(c2) << 16) | 0xff00_0000
}

/// Lightning-flash blend weight.
///
/// `((1.0 − flash) · k + magic)` single-rounded to f32, then the magic-bias
/// fixed-point extraction `(bits >> 14) & 0xff` (the stock float→fixed trick;
/// k @0x7ffe58, magic @0x8029cc).
pub fn sky_flash_alpha__6d0f50(flash: f32, k: f32, magic: f32) -> u8 {
    let v = ((1.0 - f64::from(flash)) * f64::from(k) + f64::from(magic)) as f32;
    ((v.to_bits() >> 14) & 0xff) as u8
}

#[cfg(test)]
mod tests_sky_dome__6d0f50 {
    use super::{
        sky_flash_alpha__6d0f50, sky_lerp_apply__6d0f50, sky_lerp_channel__6d0f50,
        sky_lerp_prep__6d0f50, sky_pack_channels__6d0f50, sky_round_channel__6d0f50,
    };

    /// Round-to-nearest-even after the bias subtract, truncated to a byte.
    #[test]
    fn round_channel_ties_even() {
        // bias 0: plain nearest-even.
        assert_eq!(sky_round_channel__6d0f50(2.5, 0.0), 2);
        assert_eq!(sky_round_channel__6d0f50(3.5, 0.0), 4);
        assert_eq!(sky_round_channel__6d0f50(200.4, 0.0), 200);
        // A -0.5 bias shifts the rounding point (round-half-up shape).
        assert_eq!(sky_round_channel__6d0f50(2.5, -0.5), 3);
        // Low-byte truncation like the stock (char) cast.
        assert_eq!(sky_round_channel__6d0f50(256.0, 0.0), 0);
    }

    /// Channel lerp endpoints and midpoint through the native lerp kernel.
    #[test]
    fn lerp_channel_endpoints() {
        assert_eq!(sky_lerp_channel__6d0f50(10, 200, 0.0, 0.0), 10);
        assert_eq!(sky_lerp_channel__6d0f50(10, 200, 1.0, 0.0), 200);
        assert_eq!(sky_lerp_channel__6d0f50(10, 20, 0.5, 0.0), 15);
    }

    /// The hoisted split answers exactly what the one-shot channel lerp answers.
    ///
    /// `sky_lerp_channel__6d0f50` is the oracle: it still runs the whole chain
    /// through `Math_LerpFloat`, so any drift in the prep/apply pair shows up as a
    /// byte difference here. The weights include both infinities and a NaN, since
    /// the arm the prep froze has to carry an unordered `t` the same way the
    /// unsplit form does.
    #[test]
    fn split_lerp_matches_the_unsplit_channel() {
        let weights = [
            -1.0f32,
            0.0,
            0.25,
            0.5,
            1.0,
            2.0,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
        ];
        for a in [0u8, 1, 17, 128, 200, 254, 255] {
            for b in [0u8, 1, 17, 128, 200, 254, 255] {
                let prep = sky_lerp_prep__6d0f50(a, b);
                for &t in &weights {
                    for &bias in &[0.0f32, -0.5, 0.5] {
                        assert_eq!(
                            sky_lerp_apply__6d0f50(prep, t, bias),
                            sky_lerp_channel__6d0f50(a, b, t, bias),
                            "a={a} b={b} t={t} bias={bias}"
                        );
                    }
                }
            }
        }
    }

    /// Pack layout: bytes [c0, c1, c2, 0xFF] in memory order.
    #[test]
    fn pack_layout() {
        assert_eq!(sky_pack_channels__6d0f50(0x11, 0x22, 0x33), 0xff33_2211);
        assert_eq!(
            sky_pack_channels__6d0f50(0x11, 0x22, 0x33).to_le_bytes(),
            [0x11, 0x22, 0x33, 0xff]
        );
    }

    /// The magic-bias extraction.
    ///
    /// Anchored at 1.5·2^23 (ulp = 1.0), a scale in units of 2^14 places the
    /// weight exactly in bits 14..21 — the stock float→fixed trick this
    /// reproduces bit-for-bit.
    #[test]
    fn flash_alpha_extraction() {
        let magic = 12_582_912.0f32; // 1.5 * 2^23, mantissa 0x400000
        let k = 255.0 * 16_384.0; // full-scale weight in bit-14 ulps
        assert_eq!(sky_flash_alpha__6d0f50(1.0, k, magic), 0);
        assert_eq!(sky_flash_alpha__6d0f50(0.0, k, magic), 255);
        // The fixed-point extract TRUNCATES the sub-ulp fraction (127.5 -> 127).
        assert_eq!(sky_flash_alpha__6d0f50(0.5, k, magic), 127);
    }
}

/// Camera distance to a light zone for `DayNight_BlendNearbyLights`.
///
/// (@0x6d2d00, hot block 0x6d2d69): the three camera−pos deltas stay WIDE on
/// the x87 stack (no FSTP between the subtracts and the squares, unlike the
/// particle sibling), fold `(dz·dz + dy·dy) + dx·dx`, FSQRT, one narrow.
pub fn light_camera_distance__6d2d00(cam: [f32; 3], pos: [f32; 3]) -> f32 {
    let dx = f64::from(cam[0]) - f64::from(pos[0]);
    let dy = f64::from(cam[1]) - f64::from(pos[1]);
    let dz = f64::from(cam[2]) - f64::from(pos[2]);
    super::f64_to_f32((dz * dz + dy * dy + dx * dx).sqrt())
}

/// Half-minute time index (0x6d2f40/0x6d2f98).
///
/// `t · scale` narrows once (FSTP/FLD round-trip), then the bias subtracts
/// WIDE into the FISTP — the shared biased-FISTP idiom (round-to-nearest-even,
/// indefinite on NaN/overflow).
pub fn light_time_index__6d2d00(t: f32, scale: f32, bias: f32) -> i32 {
    let v = super::f64_to_f32(f64::from(t) * f64::from(scale));
    crate::math::world::fistp_round_ties_even(f64::from(v) - f64::from(bias))
}

/// Falloff blend weight (0x6d2ff8–0x6d3022).
///
/// 1.0 inside `falloffStart`, else `1 − (dist−start)/(end−start)` folded wide
/// with one narrow. The gate is `dist > start` ORDERED (`TEST AH,0x41; JNZ`
/// keeps the 1.0 immediate on `<=`, `==`, and NaN); the raw FDIV has no
/// degenerate guard — `end == start` propagates ±Inf/NaN into the weight, as
/// stock.
pub fn light_blend_weight__6d2d00(dist: f32, start: f32, end: f32, one: f32) -> f32 {
    if dist > start {
        super::f64_to_f32(
            f64::from(one)
                - (f64::from(dist) - f64::from(start)) / (f64::from(end) - f64::from(start)),
        )
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests_blend_nearby_lights__6d2d00 {
    use super::{
        light_blend_weight__6d2d00 as weight, light_camera_distance__6d2d00 as dist,
        light_time_index__6d2d00 as tidx,
    };

    #[test]
    fn camera_distance_known() {
        assert_eq!(
            dist([4.0, 6.0, 3.0], [1.0, 2.0, 3.0]).to_bits(),
            5.0f32.to_bits()
        );
        assert!(dist([f32::NAN, 0.0, 0.0], [0.0; 3]).is_nan());
    }

    #[test]
    fn time_index_biased_round() {
        // t*scale = 5.5, bias 0 -> nearest-even 6? No: 5.5 ties to even 6.
        assert_eq!(tidx(5.5, 1.0, 0.0), 6);
        // 4.5 ties to even 4.
        assert_eq!(tidx(4.5, 1.0, 0.0), 4);
        // The bias subtracts wide: (f32(2879.6) - 0.5) rounds to 2879.
        assert_eq!(tidx(2879.6, 1.0, 0.5), 2879);
        // NaN -> integer indefinite.
        assert_eq!(tidx(f32::NAN, 1.0, 0.0), i32::MIN);
    }

    #[test]
    fn weight_gate_and_fold() {
        // Inside falloffStart (and exactly on it) -> 1.0; NaN dist -> 1.0.
        assert_eq!(weight(5.0, 10.0, 20.0, 1.0).to_bits(), 1.0f32.to_bits());
        assert_eq!(weight(10.0, 10.0, 20.0, 1.0).to_bits(), 1.0f32.to_bits());
        assert_eq!(
            weight(f32::NAN, 10.0, 20.0, 1.0).to_bits(),
            1.0f32.to_bits()
        );
        // Midway: 1 - 5/10 = 0.5.
        assert_eq!(weight(15.0, 10.0, 20.0, 1.0).to_bits(), 0.5f32.to_bits());
        // Degenerate span: (dist-start)/0 -> inf -> weight -inf, unguarded.
        assert_eq!(
            weight(15.0, 10.0, 10.0, 1.0).to_bits(),
            f32::NEG_INFINITY.to_bits()
        );
    }
}
