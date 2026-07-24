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

/// The four twiddle-table pointers the long-block IMDCT reads.
///
/// All four live in fmod's BSS, computed at init as plain cosines (arguments
/// noted per field), so the adapter publishes their resolved addresses and
/// the kernel reads them per call.
pub struct Dct36Tables {
    /// 2 entries: `cos(pi/3)`, `cos(pi/6)`.
    pub cos6: *const f32,
    /// 3 entries: `cos(pi/9)`, `cos(5 pi/9)`, `cos(7 pi/9)`.
    pub ca: *const f32,
    /// 3 entries: `cos(pi/18)`, `cos(11 pi/18)`, `cos(13 pi/18)`.
    pub cb: *const f32,
    /// 9 entries: `0.5 / cos((2i + 1) pi/36)`.
    pub tf: *const f32,
}

/// Reimplementation of the long-block IMDCT + window stage (fmod RVA `0x1db00`).
///
/// `__cdecl(inbuf, o1, o2, wintab, tsbuf)`, void. The 36-point inverse MDCT
/// of one long-block granule channel, fused with windowing and the
/// overlap-add against the previous block:
///
/// * `inbuf` (18 floats) is **mutated in place** by two running-sum
///   prepasses (`in[i] += in[i-1]` for `i = 17..1`, then
///   `in[i] += in[i-2]` for odd `i = 17..3`) — callers rely on the mutated
///   contents, so the kernel reproduces the passes exactly, including the
///   original's register carries: the `in[1]` and `in[3]` sums stay live
///   unrounded (they feed each other and the odd core), only their stores
///   narrow to `f32`.
/// * two 9-point cores (even/odd prepassed lanes; the odd core's results
///   carry the `tf` twiddles) produce 9 sum/difference pairs.
/// * the tail writes `o2[j] = sum * wintab[18 + j]` (18 floats) and
///   `tsbuf[32 * n] = diff * wintab[n] + o1[n]` (18 lanes at a 32-float
///   stride).
///
/// Loads are `f32`, arithmetic is `f64`, stores narrow to `f32` (the
/// [`super`] convention for tracking an x87 original; each store rounds
/// once, as the original's `fstp` does).
///
/// # Safety
///
/// `inbuf` must be valid for 18 float reads and writes, `o1` for 18 reads,
/// `o2` for 18 writes, `wintab` for 36 reads, `tsbuf` for writes at float
/// indices `0, 32, .., 544`, and the [`Dct36Tables`] pointers for their
/// documented lengths. The original performs the identical accesses.
// The 9-point cores' temporaries deliberately pair even/odd names
// (`es0`/`oy0`, `ep_a`/`op_a`): the two cores are the same dataflow and the
// pairing is what makes that reviewable.
#[allow(clippy::similar_names)]
pub unsafe fn fmod_dct36__1db00(
    inbuf: *mut f32,
    o1: *const f32,
    o2: *mut f32,
    wintab: *const f32,
    tsbuf: *mut f32,
    tables: &Dct36Tables,
) {
    let rd = |p: *const f32, i: usize| -> f64 {
        // SAFETY: per the contract every pointer is valid for the indices
        // the stage reads; this reproduces one of those reads.
        let value = unsafe { p.wrapping_add(i).read() };
        f64::from(value)
    };
    let wr = |p: *mut f32, i: usize, value: f64| {
        // SAFETY: per the contract the out pointers are valid at every index
        // the tail stores; the original stores through the identical address.
        unsafe { p.wrapping_add(i).write(super::f64_to_f32(value)) };
    };

    // Prepass 1: pairwise sums, descending, each partial narrowed back to
    // f32 through memory — except the `in[1]` sum, which the original keeps
    // live in a register: the second pass and the odd core consume the
    // UNROUNDED value, only the store narrows.
    for i in (2..18).rev() {
        wr(inbuf, i, rd(inbuf, i) + rd(inbuf, i - 1));
    }
    let in1_carry = rd(inbuf, 0) + rd(inbuf, 1);
    wr(inbuf, 1, in1_carry);
    // Prepass 2: the odd lanes again, stride 2; the `in[3]` sum is the
    // second register carry (built from the first, feeding the odd core).
    for i in (5..18).rev().step_by(2) {
        wr(inbuf, i, rd(inbuf, i) + rd(inbuf, i - 2));
    }
    let in3_carry = in1_carry + rd(inbuf, 3);
    wr(inbuf, 3, in3_carry);

    let inb = |i: usize| rd(inbuf, i);
    let cos6_2 = rd(tables.cos6, 0);
    let cos6_1 = rd(tables.cos6, 1);
    let ca = [rd(tables.ca, 0), rd(tables.ca, 1), rd(tables.ca, 2)];
    let cb = [rd(tables.cb, 0), rd(tables.cb, 1), rd(tables.cb, 2)];
    let tf = |i: usize| rd(tables.tf, i);

    // Even 9-point core, over the even prepassed lanes.
    let e_mid = cos6_2 * inb(12);
    let e_fold = ((inb(8) + inb(16)) - inb(4)) * cos6_2;
    let e0 = e_mid + inb(0);
    let e_base = (inb(0) - e_mid) - e_mid;
    let e1 = e_base - e_fold;
    let ep_a = (inb(8) + inb(4)) * ca[0];
    let ep_b = (inb(8) - inb(16)) * ca[1];
    let e2 = e_fold + e_fold + e_base;
    let ep_c = (inb(4) + inb(16)) * ca[2];
    let em0 = (e0 - ep_a) - ep_c;
    let em1 = ep_a + e0 + ep_b;
    let em2 = (ep_c - ep_b) + e0;
    let eq_a = (inb(10) + inb(2)) * cb[0];
    let eq_b = (inb(10) - inb(14)) * cb[1];
    let e_six = cos6_1 * inb(6);
    let er0 = e_six + eq_b + eq_a;
    let es0 = em1 + er0;
    let es8 = em1 - er0;
    let eq_c = (inb(14) + inb(2)) * cb[2];
    let er1 = (eq_c - e_six) + eq_a;
    let es3 = er1 + em2;
    let e_fold2 = ((inb(14) + inb(10)) - inb(2)) * cos6_1;
    let es5 = em2 - er1;
    let er2 = eq_b - (e_six + eq_c);
    let es1 = e1 - e_fold2;
    let es7 = e_fold2 + e1;
    let es2 = em0 + er2;
    let es6 = em0 - er2;

    // Odd 9-point core, over the odd prepassed lanes; results carry the
    // `tf` twiddles. `in[1]` and `in[3]` enter as the unrounded register
    // carries, not the narrowed stores.
    let o_mid = cos6_2 * inb(13);
    let o_fold = ((inb(17) + inb(9)) - inb(5)) * cos6_2;
    let o0 = in1_carry + o_mid;
    let o_base = (in1_carry - o_mid) - o_mid;
    let o1v = o_base - o_fold;
    let op_a = (inb(9) + inb(5)) * ca[0];
    let op_b = (inb(9) - inb(17)) * ca[1];
    let o2v = (o_fold + o_fold + o_base) * tf(4);
    let op_c = (inb(17) + inb(5)) * ca[2];
    let om0 = (o0 - op_a) - op_c;
    let om1 = op_a + o0 + op_b;
    let om2 = (op_c - op_b) + o0;
    let oq_a = (in3_carry + inb(11)) * cb[0];
    let oq_b = (inb(11) - inb(15)) * cb[1];
    let o_six = cos6_1 * inb(7);
    let or0 = o_six + oq_b + oq_a;
    let oy0 = (om1 + or0) * tf(0);
    let oy8 = (om1 - or0) * tf(8);
    let oq_c = (in3_carry + inb(15)) * cb[2];
    let or1 = (oq_c - o_six) + oq_a;
    let oy3 = (om2 + or1) * tf(3);
    let o_fold2 = ((inb(15) + inb(11)) - in3_carry) * cos6_1;
    let oy5 = (om2 - or1) * tf(5);
    let or2 = oq_b - (o_six + oq_c);
    let oy1 = (o1v - o_fold2) * tf(1);
    let oy7 = (o_fold2 + o1v) * tf(7);
    let oy2 = (om0 + or2) * tf(2);
    let oy6 = (om0 - or2) * tf(6);

    // Tail: 9 sum/difference pairs, in the original's store order. Pair `k`
    // writes `o2[9 + k]`/`o2[8 - k]` from the sum and overlap-adds the
    // difference into `tsbuf[32 * (8 - k)]`/`tsbuf[32 * (9 + k)]`.
    let evens = [es0, es1, es2, es3, e2, es5, es6, es7, es8];
    let odds = [oy0, oy1, oy2, oy3, o2v, oy5, oy6, oy7, oy8];
    for k in 0..9 {
        let sum = odds[k] + evens[k];
        wr(o2, 9 + k, sum * rd(wintab, 27 + k));
        wr(o2, 8 - k, sum * rd(wintab, 26 - k));
        let diff = evens[k] - odds[k];
        wr(
            tsbuf,
            32 * (8 - k),
            diff * rd(wintab, 8 - k) + rd(o1, 8 - k),
        );
        wr(
            tsbuf,
            32 * (9 + k),
            diff * rd(wintab, 9 + k) + rd(o1, 9 + k),
        );
    }
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
mod tests_fmod_dct36__1db00 {
    use std::f64::consts::PI;

    use super::{Dct36Tables, fmod_dct36__1db00 as dct36};

    // The tables the original's init computes (verified against its cosine
    // arguments): cos(pi/3), cos(pi/6); cos(pi{1,5,7}/9); cos(pi{1,11,13}/18);
    // 0.5/cos((2i+1)pi/36).
    struct Tables {
        cos6: Vec<f32>,
        ca: Vec<f32>,
        cb: Vec<f32>,
        tf: Vec<f32>,
    }

    fn tables() -> Tables {
        Tables {
            cos6: vec![(PI / 3.0).cos() as f32, (PI / 6.0).cos() as f32],
            ca: [1.0f64, 5.0, 7.0]
                .iter()
                .map(|k| ((k * PI / 9.0).cos()) as f32)
                .collect(),
            cb: [1.0f64, 11.0, 13.0]
                .iter()
                .map(|k| ((k * PI / 18.0).cos()) as f32)
                .collect(),
            tf: (0..9)
                .map(|i| (0.5 / ((f64::from(2 * i + 1)) * PI / 36.0).cos()) as f32)
                .collect(),
        }
    }

    fn ptrs(t: &Tables) -> Dct36Tables {
        Dct36Tables {
            cos6: t.cos6.as_ptr(),
            ca: t.ca.as_ptr(),
            cb: t.cb.as_ptr(),
            tf: t.tf.as_ptr(),
        }
    }

    // The decoder's long-block window: the plain sine window with the
    // odd-core twiddle folded in (which is why the kernel's raw sum/diff
    // values are not themselves IMDCT rows).
    fn window() -> Vec<f32> {
        (0..36)
            .map(|i| {
                let i = f64::from(i);
                (0.5 * (PI * (2.0 * i + 1.0) / 72.0).sin() / (PI * (2.0 * i + 19.0) / 72.0).cos())
                    as f32
            })
            .collect()
    }

    // Deterministic LCG so the test needs no rng dependency.
    fn lcg(state: &mut u64) -> f32 {
        *state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let bits = (*state >> 40) as u32;
        (f32::from_bits(0x3f80_0000 | (bits & 0x007f_ffff)) - 1.5) * 2.0
    }

    #[test]
    fn matches_the_windowed_imdct() {
        let t = tables();
        let tp = ptrs(&t);
        let win = window();
        let mut state = 0xfeed_f00d_0123_4567u64;
        for round in 0..8 {
            let input: Vec<f32> = (0..18).map(|_| lcg(&mut state)).collect();
            let overlap: Vec<f32> = (0..18).map(|_| lcg(&mut state)).collect();
            let mut inbuf = input.clone();
            let mut o2 = vec![0.0f32; 18];
            let mut ts = vec![0.0f32; 545];
            // SAFETY: every buffer covers the kernel's documented extent.
            unsafe {
                dct36(
                    inbuf.as_mut_ptr(),
                    overlap.as_ptr(),
                    o2.as_mut_ptr(),
                    win.as_ptr(),
                    ts.as_mut_ptr(),
                    &tp,
                );
            }
            // With the true window the composite is the windowed 36-point
            // IMDCT: lane n gets sin(pi(2n+1)/72) * V_n, where
            // V_n = sum_k in[k] cos(pi/72 (2n+19)(2k+1)); the first half
            // overlap-adds o1, the second half lands in o2. Tolerance well
            // above f32 rounding noise (~2e-7), well below structural error.
            let tol = 1e-4;
            for n in 0..36 {
                let nf = f64::from(n);
                let w = (PI * (2.0 * nf + 1.0) / 72.0).sin();
                let v: f64 = (0..18)
                    .map(|k| {
                        f64::from(input[k])
                            * (PI / 72.0 * (2.0 * nf + 19.0) * f64::from(2 * k as i32 + 1)).cos()
                    })
                    .sum();
                let n = usize::try_from(n).unwrap();
                let (got, want) = if n < 18 {
                    (f64::from(ts[32 * n]), w * v + f64::from(overlap[n]))
                } else {
                    (f64::from(o2[n - 18]), w * v)
                };
                assert!(
                    (got - want).abs() < tol,
                    "round {round} lane {n}: got {got}, want {want}"
                );
            }
        }
    }

    #[test]
    fn mutates_inbuf_with_the_two_prepasses() {
        let t = tables();
        let tp = ptrs(&t);
        let win = window();
        let mut state = 0x5a5a_a5a5_5a5a_a5a5u64;
        let input: Vec<f32> = (0..18).map(|_| lcg(&mut state)).collect();
        let overlap = [0.0f32; 18];

        // Independent reference for the in-place passes: pairwise sums
        // descending, then the odd lanes again at stride 2. The `in[1]` and
        // `in[3]` sums carry unrounded f64 into their consumers (the
        // original keeps them in registers), so only their stores narrow.
        let mut want = input.clone();
        for i in (2..18).rev() {
            want[i] += want[i - 1];
        }
        let carry1 = f64::from(want[0]) + f64::from(want[1]);
        want[1] = carry1 as f32;
        for i in (5..18).rev().step_by(2) {
            want[i] += want[i - 2];
        }
        want[3] = (carry1 + f64::from(want[3])) as f32;

        let mut inbuf = input;
        let mut o2 = vec![0.0f32; 18];
        let mut ts = vec![0.0f32; 545];
        // SAFETY: every buffer covers the kernel's documented extent.
        unsafe {
            dct36(
                inbuf.as_mut_ptr(),
                overlap.as_ptr(),
                o2.as_mut_ptr(),
                win.as_ptr(),
                ts.as_mut_ptr(),
                &tp,
            );
        }
        let got: Vec<u32> = inbuf.iter().map(|x| x.to_bits()).collect();
        let want: Vec<u32> = want.iter().map(|x| x.to_bits()).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn writes_only_the_stride_32_lanes() {
        let t = tables();
        let tp = ptrs(&t);
        let win = window();
        let mut state = 0x0f0f_f0f0_0f0f_f0f0u64;
        let mut inbuf: Vec<f32> = (0..18).map(|_| lcg(&mut state)).collect();
        let overlap = [0.0f32; 18];
        let sentinel = f32::from_bits(0x7fc0_4321);
        let mut o2 = vec![sentinel; 18];
        let mut ts = vec![sentinel; 545];
        // SAFETY: every buffer covers the kernel's documented extent.
        unsafe {
            dct36(
                inbuf.as_mut_ptr(),
                overlap.as_ptr(),
                o2.as_mut_ptr(),
                win.as_ptr(),
                ts.as_mut_ptr(),
                &tp,
            );
        }
        for (i, &x) in ts.iter().enumerate() {
            if i % 32 == 0 {
                assert_ne!(x.to_bits(), sentinel.to_bits(), "ts[{i}] must be written");
            } else {
                assert_eq!(x.to_bits(), sentinel.to_bits(), "ts[{i}] must be untouched");
            }
        }
        for (j, &x) in o2.iter().enumerate() {
            assert_ne!(x.to_bits(), sentinel.to_bits(), "o2[{j}] must be written");
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
