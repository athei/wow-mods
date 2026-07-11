//! `fmod__mixer_fpu` — FMOD3's MPEG-1 audio synthesis-filterbank dewindow.
//!
//! This is the single hottest x87 routine in a busy session: the profiler pins
//! both top hot blocks to its two unrolled inner loops (the FMOD mixer thread
//! decoding streamed MP3 music/ambience). FMOD ships only the FPU (x87) mixer in
//! this build, and this windowing stage is not part of the FPU/MMX mixer-quality
//! switch, so the only way off x87 is to replace the function — which is what
//! this kernel does, in portable f64 that lowers to SSE/NEON.
//!
//! The original (fmod RVA `0x348e0`, `__cdecl`) produces 32 PCM samples from a
//! window coefficient table and a decoded V buffer: 16 samples from an
//! alternating-sign 16-tap window·V dot product, one centre sample from the even
//! taps only, then 15 samples from the reverse window, negated. Each running sum
//! is converted by `FISTP`→clamp to the int16 range and stored at `out_stride`
//! spacing. The original loads f32 operands but accumulates in the x87 80-bit
//! stack; the product of two f32 values is exact in f64 and the reductions are
//! short, so accumulating in f64 (the [`super`] convention for tracking an x87
//! original) reproduces the result to within ≤1 int16 LSB.
#![allow(non_snake_case, clippy::pedantic, clippy::nursery)]

/// Round a synthesized sample to clamped 16-bit PCM, matching the original's
/// `FISTP`→`CMP 0x7fff`/`CMP 0xffff8000` sequence: round to nearest (ties to
/// even, the x87 default mode) then clamp into the int16 range.
#[inline]
fn pack_i16(sample: f64) -> i16 {
    let rounded = sample.round_ties_even();
    rounded.clamp(-32768.0, 32767.0) as i16
}

/// Reimplementation of `fmod__mixer_fpu` (fmod RVA `0x348e0`): the MPEG synthesis
/// dewindow. Computes the 32 output samples exactly as the original walks its
/// pointers — `window` advances `+0x80` bytes (32 f32) per sample in segments 1/2
/// and `-0x80` per sample in segment 3; `src` advances `±0x40` bytes (16 f32).
///
/// * `window` — the coefficient table base (`*(fmod_base + 0x55144)` at runtime).
/// * `src` — the decoded V buffer.
/// * `phase` — the window sub-index the original folds into the table offset.
/// * `out_stride` — output spacing in int16 elements (1 mono, 2 interleaved).
/// * `out` — int16 PCM destination (32 samples written, at `out_stride` spacing).
///
/// # Safety
///
/// `window`, `src` and `out` must be valid for the exact offsets the original
/// accesses for the given `phase`: `window` over float indices `[1, 543]`, `src`
/// over `[0, 271]`, and `out` over `0..32` at `out_stride` spacing — the kernel
/// performs the identical address arithmetic.
pub unsafe fn fmod_mixer_fpu__348e0(
    window: *const f32,
    src: *const f32,
    phase: i32,
    out_stride: i32,
    out: *mut i16,
) {
    let p = phase as isize;
    let stride = out_stride as isize;

    // `wrapping_offset` keeps the pointer arithmetic out of `unsafe` (the read is
    // the single unsafe op), mirroring the host's float-indexed loads.
    let w = |i: isize| -> f64 {
        // SAFETY: per the contract `window` is valid at every offset the original
        // reads for this `phase`; this reproduces one of those reads.
        let value = unsafe { window.wrapping_offset(i).read() };
        f64::from(value)
    };
    let v = |i: isize| -> f64 {
        // SAFETY: per the contract `src` is valid at every offset the original
        // reads; this reproduces one of those reads.
        let value = unsafe { src.wrapping_offset(i).read() };
        f64::from(value)
    };
    let store = |sample: isize, value: i16| {
        // SAFETY: per the contract `out` is valid for samples `0..32` at
        // `out_stride` spacing.
        unsafe { out.wrapping_offset(sample * stride).write(value) };
    };

    // Segment 1 — output samples 0..16: alternating-sign 16-tap window·V.
    for s in 0..16isize {
        let wbase = 16 - p + 32 * s;
        let vbase = 16 * s;
        let mut acc = 0.0f64;
        for j in 0..16isize {
            let prod = w(wbase + j) * v(vbase + j);
            acc += if (j & 1) == 0 { prod } else { -prod };
        }
        store(s, pack_i16(acc));
    }

    // Segment 2 — centre sample 16: even taps only, all positive.
    {
        let wbase = 16 - p + 512;
        let vbase = 256;
        let mut acc = 0.0f64;
        let mut j = 0isize;
        while j < 16 {
            acc += w(wbase + j) * v(vbase + j);
            j += 2;
        }
        store(16, pack_i16(acc));
    }

    // Segment 3 — output samples 17..32 (15 samples): reverse window, negated.
    // The window is read descending (`-(j+1)` for j<15, then `+0` for j=15).
    for t in 0..15isize {
        let wbase = 496 + p - 32 * t;
        let vbase = 240 - 16 * t;
        let mut acc = 0.0f64;
        for j in 0..15isize {
            acc += w(wbase - (j + 1)) * v(vbase + j);
        }
        acc += w(wbase) * v(vbase + 15);
        store(17 + t, pack_i16(-acc));
    }
}

#[cfg(test)]
mod tests_fmod_mixer_fpu__348e0 {
    use super::fmod_mixer_fpu__348e0 as mixer;

    // Independent reference for the 32 output samples, written from the
    // documented index/sign formula (not the kernel's loop structure), in f64.
    fn reference(window: &[f32], src: &[f32], phase: isize, stride: usize) -> Vec<i16> {
        let w = |i: isize| f64::from(window[usize::try_from(i).unwrap()]);
        let v = |i: isize| f64::from(src[usize::try_from(i).unwrap()]);
        let pack = |x: f64| -> i16 { x.round_ties_even().clamp(-32768.0, 32767.0) as i16 };
        let mut out = vec![0i16; 32 * stride];

        // Segment 1.
        for s in 0..16isize {
            let mut acc = 0.0f64;
            for j in 0..16isize {
                let prod = w(16 - phase + 32 * s + j) * v(16 * s + j);
                acc += if j % 2 == 0 { prod } else { -prod };
            }
            out[usize::try_from(s).unwrap() * stride] = pack(acc);
        }
        // Segment 2 (centre).
        let mut acc = 0.0f64;
        for k in (0..16isize).step_by(2) {
            acc += w(16 - phase + 512 + k) * v(256 + k);
        }
        out[16 * stride] = pack(acc);
        // Segment 3.
        for t in 0..15isize {
            let mut acc = 0.0f64;
            for j in 0..15isize {
                acc += w(496 + phase - 32 * t - (j + 1)) * v(240 - 16 * t + j);
            }
            acc += w(496 + phase - 32 * t) * v(240 - 16 * t + 15);
            out[usize::try_from(17 + t).unwrap() * stride] = pack(-acc);
        }
        out
    }

    // Deterministic LCG so the test needs no rng dependency.
    fn lcg(state: &mut u64) -> f32 {
        *state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        // Map the top bits to roughly [-1, 1).
        let bits = (*state >> 40) as u32;
        (f32::from_bits(0x3f80_0000 | (bits & 0x007f_ffff)) - 1.5) * 2.0
    }

    fn run(window: &[f32], src: &[f32], phase: i32, stride: usize) -> Vec<i16> {
        let mut out = vec![0i16; 32 * stride];
        // SAFETY: window/src/out are sized to cover every offset the kernel reads
        // for phases 0..16 (see the kernel's safety contract).
        unsafe {
            mixer(
                window.as_ptr(),
                src.as_ptr(),
                phase,
                i32::try_from(stride).unwrap(),
                out.as_mut_ptr(),
            );
        }
        out
    }

    #[test]
    fn all_ones_window_and_v() {
        // window=1, V=1: seg1 sums alternate to 0; centre = 8 even taps; seg3 =
        // 16 taps negated = -16.
        let window = vec![1.0f32; 560];
        let src = vec![1.0f32; 288];
        let got = run(&window, &src, 0, 1);
        for (i, &s) in got.iter().enumerate() {
            let expect = match i {
                0..=15 => 0,
                16 => 8,
                _ => -16,
            };
            assert_eq!(s, expect, "sample {i}");
        }
    }

    #[test]
    fn matches_reference_across_phases_and_strides() {
        let mut state = 0x1234_5678_9abc_def0u64;
        let window: Vec<f32> = (0..560).map(|_| lcg(&mut state) * 0.5).collect();
        let src: Vec<f32> = (0..288).map(|_| lcg(&mut state) * 4000.0).collect();
        for phase in 0..16i32 {
            for &stride in &[1usize, 2] {
                let got = run(&window, &src, phase, stride);
                let want = reference(&window, &src, isize::try_from(phase).unwrap(), stride);
                assert_eq!(got, want, "phase {phase} stride {stride}");
            }
        }
    }

    #[test]
    fn clamps_to_int16_range() {
        // Large positive V with an all-positive even window drives the centre
        // sample past +32767; a large negative V drives segment 3 past -32768
        // (segment 3 negates its sum).
        let window = vec![1.0f32; 560];
        let mut src = vec![5000.0f32; 288];
        let hi = run(&window, &src, 0, 1);
        assert_eq!(hi[16], 32767, "centre clamps high");
        assert_eq!(
            hi[17], -32768,
            "seg3 negates its sum, so positive V clamps low"
        );

        src.iter_mut().for_each(|x| *x = -5000.0);
        let lo = run(&window, &src, 0, 1);
        assert_eq!(lo[16], -32768, "centre clamps low");
        assert_eq!(lo[17], 32767, "seg3 negated → clamps high");
    }

    #[test]
    fn stereo_stride_leaves_other_channel_untouched() {
        let window = vec![0.25f32; 560];
        let src = vec![100.0f32; 288];
        let out = run(&window, &src, 3, 2);
        // Only even indices (this channel) are written; odd stay zero.
        for (i, &s) in out.iter().enumerate() {
            if i % 2 == 1 {
                assert_eq!(s, 0, "interleaved slot {i} must be untouched");
            }
        }
    }
}
