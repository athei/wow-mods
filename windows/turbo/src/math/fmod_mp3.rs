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
