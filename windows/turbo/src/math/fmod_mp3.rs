//! FMOD3 MPEG Layer-III decode DSP kernels.
//!
//! The decode chain feeding the synthesis dewindow ([`super::fmod_mixer`])
//! runs on the same FMOD mixer thread and carries the same x87 cost profile,
//! so the hot stages get the same treatment: portable `f64` kernels that
//! lower to SSE/NEON, hooked at fmod's per-launch base in `win::fmod`.
//!
//! Every coefficient table these stages read lives in fmod's `.data`/BSS and
//! is computed at init, so kernels take table pointers as arguments; nothing
//! is baked in.
// Kernel names carry the `__<rva>` suffix like every reimpl (the `__` is the
// convention marker), so the module is non-snake-case by construction.
#![allow(non_snake_case)]

/// Reimplementation of the alias-reduction stage (fmod RVA `0x1d890`).
///
/// `__cdecl(hybrid_buf: *mut f32, gr_info: *const _)` in the original; the
/// adapter decodes `gr_info` into `boundaries` (see `win::fmod`) so the
/// kernel is pure butterfly arithmetic. For each subband boundary
/// `b in 1..=boundaries` (18-float subband lines), 8 butterflies fold the
/// samples straddling the boundary, in place:
///
/// * `down[i] = bu*cs[i] - bd*ca[i]` into `buf[18*b - 1 - i]`
/// * `up[i]   = bd*cs[i] + bu*ca[i]` into `buf[18*b + i]`
///
/// where `bu = buf[18*b - 1 - i]`, `bd = buf[18*b + i]` are the pre-update
/// values (the original loads both before either store). `cs`/`ca` are the
/// 8-entry tables at fmod RVA `0x59740`/`0x59720`, runtime-computed. Loads
/// are `f32`, products and sums are carried in `f64` (the [`super`]
/// convention for tracking an x87 original), stores narrow to `f32`.
///
/// `boundaries` is `u32` on purpose: the original computes its count with a
/// plain `dec` and only skips the loop on exactly zero, so the wrapped value
/// from a (never observed) zero subband count walks the same addresses here
/// as there.
///
/// # Safety
///
/// `buf` must be valid for reads and writes over
/// `[18 - 8, 18 * boundaries + 8)` (float indices), and `ca`/`cs` for reads
/// of 8 floats each; the kernel performs the identical address arithmetic to
/// the original.
pub unsafe fn fmod_iii_antialias__1d890(
    buf: *mut f32,
    boundaries: u32,
    ca: *const f32,
    cs: *const f32,
) {
    let rd = |p: *mut f32, i: isize| -> f64 {
        // SAFETY: per the contract `buf` covers every offset the original
        // touches for this `boundaries`; this reproduces one of those reads.
        let value = unsafe { p.wrapping_offset(i).read() };
        f64::from(value)
    };
    let wr = |p: *mut f32, i: isize, value: f64| {
        // SAFETY: per the contract `buf` covers every offset the original
        // touches; the original stores through the identical address.
        unsafe { p.wrapping_offset(i).write(super::f64_to_f32(value)) };
    };
    let tab = |p: *const f32, i: isize| -> f64 {
        // SAFETY: per the contract the table holds 8 entries.
        let value = unsafe { p.wrapping_offset(i).read() };
        f64::from(value)
    };

    // Walk the boundary pointer 18 floats per line, as the original does.
    let mut base = buf.wrapping_add(18);
    for _ in 0..boundaries {
        for i in 0..8isize {
            let bu = rd(base, -1 - i);
            let bd = rd(base, i);
            let cs_i = tab(cs, i);
            let ca_i = tab(ca, i);
            wr(base, -1 - i, bu * cs_i - bd * ca_i);
            wr(base, i, bd * cs_i + bu * ca_i);
        }
        base = base.wrapping_add(18);
    }
}

/// Per-quad fold of the DCT's last two butterfly stages.
///
/// Each stage-3 quad `y` collapses to the six combinations the output stage
/// consumes; see [`fmod_dct64__17f60`] for the naming.
struct QuadFold {
    /// `q0 + q1`: the plain sum chain.
    hs: f64,
    /// `(q0 - q1) * c4`: the sum-side twiddled difference.
    h: f64,
    /// `q2 + q3 + l`: the difference-side running sum.
    ls: f64,
    /// `(q2 - q3) * c4`: the difference-side twiddled difference.
    l: f64,
    /// `q0 + q1 + ls`: the full accumulation.
    full: f64,
}

/// Reimplementation of the synthesis-filterbank DCT (fmod RVA `0x17f60`).
///
/// `__cdecl(out0: *mut f32, out1: *mut f32, samples: *const f32)`, void. The
/// 32-point fast DCT feeding the dewindow ([`super::fmod_mixer`]): five
/// butterfly stages over the 32 input samples, then an output-combination
/// tail that writes 17 values into `out0` and 16 into `out1`, both at a
/// stride of 16 floats (the two synthesis V-buffer halves).
///
/// The five twiddle tables (16/8/4/2/1 entries, runtime-computed) sit behind
/// a five-slot pointer array in fmod's image; the adapter dereferences the
/// slots per call and passes the resolved pointers as `pnts`, mirroring the
/// original's per-stage slot loads.
///
/// Stage naming: stage 1 folds `samples` around the centre into 16 sums `s`
/// and 16 twiddled differences `d`; stage 2 folds each around its centre
/// (sums `a`/`ad`, twiddles `b`/`bd`); stage 3 folds the resulting octets
/// into quads (`aa`/`ab`, `ba`/`bb`, `ada`/`adb`, `bda`/`bdb`); stages 4 and
/// 5 collapse each quad into the [`QuadFold`] combinations; the tail sums
/// those into the 33 outputs. Loads are `f32`, arithmetic is `f64`, stores
/// narrow to `f32` (the [`super`] convention for tracking an x87 original;
/// association inside multi-term sums follows the algorithm's natural order,
/// which tracks the original within one ULP after narrowing).
///
/// # Safety
///
/// `samples` must be valid for 32 float reads; `out0` for stores at float
/// indices `0, 16, .., 256`; `out1` at `0, 16, .., 240`; each `pnts[k]` for
/// `16 >> k` float reads. The original performs the identical accesses.
pub unsafe fn fmod_dct64__17f60(
    out0: *mut f32,
    out1: *mut f32,
    samples: *const f32,
    pnts: &[*const f32; 5],
) {
    let rd = |p: *const f32, i: usize| -> f64 {
        // SAFETY: per the contract every pointer is valid for the indices
        // the stages read; this reproduces one of those reads.
        let value = unsafe { p.wrapping_add(i).read() };
        f64::from(value)
    };
    let wr = |p: *mut f32, i: usize, value: f64| {
        // SAFETY: per the contract the out pointers are valid at every
        // stride-16 index the tail stores; the original stores through the
        // identical address.
        unsafe { p.wrapping_add(i).write(super::f64_to_f32(value)) };
    };

    // Stage 1: fold the 32 samples around the centre.
    let mut s = [0.0f64; 16];
    let mut d = [0.0f64; 16];
    for i in 0..16 {
        let lo = rd(samples, i);
        let hi = rd(samples, 31 - i);
        s[i] = lo + hi;
        d[i] = (lo - hi) * rd(pnts[0], i);
    }

    // Stage 2: fold each 16-vector around its centre.
    let mut a = [0.0f64; 8];
    let mut b = [0.0f64; 8];
    let mut ad = [0.0f64; 8];
    let mut bd = [0.0f64; 8];
    for i in 0..8 {
        a[i] = s[i] + s[15 - i];
        b[i] = (s[i] - s[15 - i]) * rd(pnts[1], i);
        ad[i] = d[i] + d[15 - i];
        bd[i] = (d[i] - d[15 - i]) * rd(pnts[1], i);
    }

    // Stage 3: fold each octet into a sum quad and a twiddled quad.
    let fold8 = |x: &[f64; 8]| -> ([f64; 4], [f64; 4]) {
        let mut sums = [0.0f64; 4];
        let mut tw = [0.0f64; 4];
        for i in 0..4 {
            sums[i] = x[i] + x[7 - i];
            tw[i] = (x[i] - x[7 - i]) * rd(pnts[2], i);
        }
        (sums, tw)
    };
    let (aa, ab) = fold8(&a);
    let (ba, bb) = fold8(&b);
    let (ada, adb) = fold8(&ad);
    let (bda, bdb) = fold8(&bd);

    // Stages 4 + 5: collapse each quad into the tail's six combinations.
    let c4 = rd(pnts[4], 0);
    let fold4 = |y: &[f64; 4]| -> QuadFold {
        let q0 = y[0] + y[3];
        let q1 = y[1] + y[2];
        let q2 = (y[0] - y[3]) * rd(pnts[3], 0);
        let q3 = (y[1] - y[2]) * rd(pnts[3], 1);
        let h = (q0 - q1) * c4;
        let l = (q2 - q3) * c4;
        QuadFold {
            hs: q0 + q1,
            h,
            ls: q2 + q3 + l,
            l,
            full: q0 + q1 + (q2 + q3 + l),
        }
    };
    let aa = fold4(&aa);
    let ab = fold4(&ab);
    let ba = fold4(&ba);
    let bb = fold4(&bb);
    let ada = fold4(&ada);
    let adb = fold4(&adb);
    let bda = fold4(&bda);
    let bdb = fold4(&bdb);

    // Output combination, at a 16-float stride. `out0` consumes the running
    // sums, `out1` the twiddled-difference chains; the odd rows chain the
    // `ad`/`bd` groups exactly as the original accumulates them.
    wr(out0, 0x00, aa.h);
    wr(out0, 0x10, ada.h + (bda.h + (bdb.ls + bdb.h)));
    wr(out0, 0x20, (bb.ls + bb.h) + ba.h);
    wr(out0, 0x30, (adb.ls + adb.h) + (bda.h + (bdb.ls + bdb.h)));
    wr(out0, 0x40, ab.ls + ab.h);
    wr(out0, 0x50, (adb.ls + adb.h) + (bda.ls + (bdb.ls + bdb.h)));
    wr(out0, 0x60, (bb.ls + bb.h) + ba.ls);
    wr(out0, 0x70, ada.ls + (bda.ls + (bdb.ls + bdb.h)));
    wr(out0, 0x80, aa.ls);
    wr(out0, 0x90, ada.ls + (bda.ls + bdb.full));
    wr(out0, 0xa0, bb.full + ba.ls);
    wr(out0, 0xb0, adb.full + (bda.ls + bdb.full));
    wr(out0, 0xc0, ab.full);
    wr(out0, 0xd0, adb.full + (bda.hs + bdb.full));
    wr(out0, 0xe0, bb.full + ba.hs);
    wr(out0, 0xf0, ada.hs + (bda.hs + bdb.full));
    wr(out0, 0x100, aa.hs);

    wr(out1, 0x00, aa.h);
    wr(out1, 0x10, ada.h + (bda.h + (bdb.h + bdb.l)));
    wr(out1, 0x20, (bb.h + bb.l) + ba.h);
    wr(out1, 0x30, (adb.h + adb.l) + (bda.h + (bdb.h + bdb.l)));
    wr(out1, 0x40, ab.l + ab.h);
    wr(out1, 0x50, (adb.h + adb.l) + (bda.l + (bdb.h + bdb.l)));
    wr(out1, 0x60, (bb.h + bb.l) + ba.l);
    wr(out1, 0x70, ada.l + (bda.l + (bdb.h + bdb.l)));
    wr(out1, 0x80, aa.l);
    wr(out1, 0x90, ada.l + bda.l + bdb.l);
    wr(out1, 0xa0, bb.l + ba.l);
    wr(out1, 0xb0, adb.l + bda.l + bdb.l);
    wr(out1, 0xc0, ab.l);
    wr(out1, 0xd0, adb.l + bdb.l);
    wr(out1, 0xe0, bb.l);
    wr(out1, 0xf0, bdb.l);
}

#[cfg(test)]
mod tests_fmod_dct64__17f60 {
    use std::f64::consts::PI;

    use super::fmod_dct64__17f60 as dct64;

    // The standard twiddle sets the original's init computes: level k holds
    // 16 >> k entries of 0.5 / cos(pi * (2i + 1) / (64 >> k)).
    fn twiddles() -> [Vec<f32>; 5] {
        core::array::from_fn(|k| {
            let div = f64::from(64 >> k);
            (0..16 >> k)
                .map(|i| (0.5 / (PI * f64::from(2 * i + 1) / div).cos()) as f32)
                .collect()
        })
    }

    // Deterministic LCG so the test needs no rng dependency.
    fn lcg(state: &mut u64) -> f32 {
        *state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let bits = (*state >> 40) as u32;
        (f32::from_bits(0x3f80_0000 | (bits & 0x007f_ffff)) - 1.5) * 2.0
    }

    // Independent reference: the synthesis matrixing the fast DCT computes.
    // V[j] = sum_k in[k] * cos(pi * (16 + j) * (2k + 1) / 64); the two output
    // halves fold V as out0[16i] = -V[32 + i] (i = 1..=16, V[0] at i = 0) and
    // out1[16i] = -V[32 - i] (i = 1..=15, V[0] at i = 0).
    fn v_row(input: &[f32; 32], j: i32) -> f64 {
        (0..32)
            .map(|k| {
                f64::from(input[k])
                    * (PI * f64::from(16 + j) * f64::from(2 * k as i32 + 1) / 64.0).cos()
            })
            .sum()
    }

    #[test]
    fn matches_the_synthesis_matrixing() {
        let tw = twiddles();
        let pnts = [
            tw[0].as_ptr(),
            tw[1].as_ptr(),
            tw[2].as_ptr(),
            tw[3].as_ptr(),
            tw[4].as_ptr(),
        ];
        let mut state = 0x0bad_cafe_1234_5678u64;
        for round in 0..8 {
            let mut input = [0.0f32; 32];
            for x in &mut input {
                *x = lcg(&mut state);
            }
            let mut out0 = vec![0.0f32; 257];
            let mut out1 = vec![0.0f32; 241];
            // SAFETY: the buffers cover every stride-16 index the kernel
            // stores and the twiddle vectors their documented lengths.
            unsafe { dct64(out0.as_mut_ptr(), out1.as_mut_ptr(), input.as_ptr(), &pnts) };

            // The fast DCT reassociates, so compare against the direct
            // matrix within a tolerance far above f32 rounding noise
            // (~2e-7 on unit-scale inputs) and far below any structural
            // error (a wrong index, sign, or store lands O(0.1..1)).
            let tol = 1e-4;
            for i in 0..=16i32 {
                let want = if i == 0 {
                    v_row(&input, 0)
                } else {
                    -v_row(&input, 32 + i)
                };
                let got = f64::from(out0[16 * usize::try_from(i).unwrap()]);
                assert!(
                    (got - want).abs() < tol,
                    "round {round} out0[16*{i}]: got {got}, want {want}"
                );
            }
            for i in 0..=15i32 {
                let want = if i == 0 {
                    v_row(&input, 0)
                } else {
                    -v_row(&input, 32 - i)
                };
                let got = f64::from(out1[16 * usize::try_from(i).unwrap()]);
                assert!(
                    (got - want).abs() < tol,
                    "round {round} out1[16*{i}]: got {got}, want {want}"
                );
            }
        }
    }

    #[test]
    fn writes_only_the_stride_16_lanes() {
        let tw = twiddles();
        let pnts = [
            tw[0].as_ptr(),
            tw[1].as_ptr(),
            tw[2].as_ptr(),
            tw[3].as_ptr(),
            tw[4].as_ptr(),
        ];
        let mut state = 0x1111_2222_3333_4444u64;
        let mut input = [0.0f32; 32];
        for x in &mut input {
            *x = lcg(&mut state);
        }
        let sentinel = f32::from_bits(0x7fc0_1234);
        let mut out0 = vec![sentinel; 257];
        let mut out1 = vec![sentinel; 241];
        // SAFETY: the buffers cover every stride-16 index the kernel stores.
        unsafe { dct64(out0.as_mut_ptr(), out1.as_mut_ptr(), input.as_ptr(), &pnts) };
        for (i, &x) in out0.iter().enumerate() {
            if i % 16 == 0 {
                assert_ne!(x.to_bits(), sentinel.to_bits(), "out0[{i}] must be written");
            } else {
                assert_eq!(
                    x.to_bits(),
                    sentinel.to_bits(),
                    "out0[{i}] must be untouched"
                );
            }
        }
        for (i, &x) in out1.iter().enumerate() {
            if i % 16 == 0 {
                assert_ne!(x.to_bits(), sentinel.to_bits(), "out1[{i}] must be written");
            } else {
                assert_eq!(
                    x.to_bits(),
                    sentinel.to_bits(),
                    "out1[{i}] must be untouched"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests_fmod_iii_antialias__1d890 {
    use super::fmod_iii_antialias__1d890 as antialias;

    // Independent reference from the butterfly formula (not the kernel's
    // pointer walk): fold 8 sample pairs around each 18-float boundary.
    fn reference(buf: &mut [f32], boundaries: usize, ca: &[f32; 8], cs: &[f32; 8]) {
        for b in 1..=boundaries {
            for i in 0..8 {
                let down = 18 * b - 1 - i;
                let up = 18 * b + i;
                let bu = f64::from(buf[down]);
                let bd = f64::from(buf[up]);
                buf[down] = (bu * f64::from(cs[i]) - bd * f64::from(ca[i])) as f32;
                buf[up] = (bd * f64::from(cs[i]) + bu * f64::from(ca[i])) as f32;
            }
        }
    }

    // Deterministic LCG so the test needs no rng dependency.
    fn lcg(state: &mut u64) -> f32 {
        *state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let bits = (*state >> 40) as u32;
        (f32::from_bits(0x3f80_0000 | (bits & 0x007f_ffff)) - 1.5) * 2.0
    }

    fn tables(state: &mut u64) -> ([f32; 8], [f32; 8]) {
        let mut ca = [0.0f32; 8];
        let mut cs = [0.0f32; 8];
        for i in 0..8 {
            ca[i] = lcg(state);
            cs[i] = lcg(state);
        }
        (ca, cs)
    }

    #[test]
    fn matches_reference_across_boundary_counts() {
        let mut state = 0x0123_4567_89ab_cdefu64;
        let (ca, cs) = tables(&mut state);
        for boundaries in 1..=31usize {
            let src: Vec<f32> = (0..18 * (boundaries + 1))
                .map(|_| lcg(&mut state) * 8.0)
                .collect();
            let mut got = src.clone();
            let mut want = src;
            // SAFETY: `got` covers indices [10, 18*boundaries + 8), the
            // kernel's documented extent for this count.
            unsafe {
                antialias(
                    got.as_mut_ptr(),
                    u32::try_from(boundaries).unwrap(),
                    ca.as_ptr(),
                    cs.as_ptr(),
                );
            }
            reference(&mut want, boundaries, &ca, &cs);
            let got_bits: Vec<u32> = got.iter().map(|x| x.to_bits()).collect();
            let want_bits: Vec<u32> = want.iter().map(|x| x.to_bits()).collect();
            assert_eq!(got_bits, want_bits, "boundaries {boundaries}");
        }
    }

    #[test]
    fn untouched_lanes_stay_untouched() {
        let mut state = 0xdead_beef_cafe_f00du64;
        let (ca, cs) = tables(&mut state);
        let boundaries = 3usize;
        let src: Vec<f32> = (0..18 * (boundaries + 1))
            .map(|_| lcg(&mut state))
            .collect();
        let mut got = src.clone();
        // SAFETY: `got` covers the kernel's documented extent for this count.
        unsafe {
            antialias(got.as_mut_ptr(), 3, ca.as_ptr(), cs.as_ptr());
        }
        for (i, (&g, &s)) in got.iter().zip(&src).enumerate() {
            let line = i / 18;
            let pos = i % 18;
            // Butterflies touch the last 8 of line b and the first 8 of
            // line b+1, for b in 1..=boundaries.
            let touched = (pos >= 10 && line < boundaries) || (pos < 8 && line >= 1);
            if !touched {
                assert_eq!(g.to_bits(), s.to_bits(), "lane {i} must be untouched");
            }
        }
    }

    #[test]
    fn identity_tables_swap_nothing_but_scale() {
        // cs = 1, ca = 0: every butterfly is the identity; the buffer must
        // come back bit-identical.
        let mut state = 0x5555_aaaa_5555_aaaau64;
        let ca = [0.0f32; 8];
        let cs = [1.0f32; 8];
        let src: Vec<f32> = (0..18 * 4).map(|_| lcg(&mut state)).collect();
        let mut got = src.clone();
        // SAFETY: `got` covers the kernel's documented extent for this count.
        unsafe {
            antialias(got.as_mut_ptr(), 3, ca.as_ptr(), cs.as_ptr());
        }
        let got_bits: Vec<u32> = got.iter().map(|x| x.to_bits()).collect();
        let src_bits: Vec<u32> = src.iter().map(|x| x.to_bits()).collect();
        assert_eq!(got_bits, src_bits);
    }
}
