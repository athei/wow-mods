//! x87-free float→integer truncation.
//!
//! On 32-bit x86 a compiler-lowered `f64 as i64` emits an x87 `fld`/`fisttp`
//! (or the fnstcw/fistp control-word dance) plus a saturation clamp — under
//! Rosetta each x87 op expands ~20×, so every such cast in a hot path shows up
//! as a profiler row. This kernel truncates with one SSE convert where the
//! answer fits `i32` and from the raw IEEE bits in integer code where it does
//! not. Shared here because both the `wow_turbo` reimpls (where
//! it doubles as the faithful CRT `__ftol` replacement) and any i686
//! code needing a float→int cast without the x87 tax use it.
// The entry point keeps the CRT's `__ftol` spelling so the shim reads as the
// symbol it replaces.
#![allow(non_snake_case)]

/// CRT float-to-i64 truncation (`__ftol`).
///
/// Round toward zero, with the x87 indefinite value (`i64::MIN`) for NaN,
/// infinities, and out-of-range inputs.
///
/// No x87 anywhere — the original swaps the FPU control word twice around an
/// `fistp` for every float-to-int cast in the binary, and a float-to-i64 cast
/// lowered by the compiler on 32-bit x86 would re-emit that same control-word
/// dance. A truncating SSE convert answers everything that fits `i32`; the rest
/// is built from the raw bits in integer code. `-2^63` itself shares the
/// indefinite bit pattern, so every `|x| >= 2^63` case collapses to one return.
///
/// The `__40a2b0` suffix is the WoW.exe VA of the CRT `__ftol` this kernel
/// faithfully replaces (`wow_turbo` hooks that address and re-exports this fn).
pub fn ftol__40a2b0(x: f64) -> i64 {
    // Everything whose truncation fits `i32` — which is nearly every conversion
    // the client asks for — is one truncating convert. The indefinite value is
    // the sentinel for "the wide path decides": it covers NaN, the infinities
    // and every magnitude the narrow convert cannot represent. `-2^31` is the
    // one input that reaches the sentinel legitimately, and the wide path
    // answers exactly `-2^31` for it, so no input is decided twice.
    let narrow = trunc_to_i32(x);
    if narrow != i32::MIN {
        return i64::from(narrow);
    }

    let (lo, hi) = raw_halves(x);
    let exp = ((hi >> 20) & 0x7ff) as i32;
    if exp < 1023 {
        // |x| < 1 (zero and subnormals included) truncates to 0.
        return 0;
    }
    let e = exp - 1075;
    if e >= 11 {
        // NaN/Inf (exp = 0x7ff) and every |x| >= 2^63: the fistp indefinite
        // value. -2^63 lands on the same pattern, so it needs no carve-out.
        return i64::MIN;
    }
    // Masking the high half to 20 bits leaves bit 52 of the pair clear, so the
    // implicit bit is a disjoint set on that half alone and the low half is
    // untouched. Spelled on the halves because the 64-bit
    // `(bits & (2^52 - 1)) | 2^52` lowers to a two-add carry chain across the
    // register pair on a 32-bit target, where one `or` says the same thing.
    let m = (u64::from((hi & 0x000f_ffff) | 0x0010_0000) << 32) | u64::from(lo);
    let mag = if e >= 0 { m << e } else { m >> -e };
    if hi >> 31 == 0 {
        mag as i64
    } else {
        -(mag as i64)
    }
}

/// Truncate toward zero into `i32`, answering `i32::MIN` where that cannot be done.
///
/// This is `cvttsd2si`'s contract: round toward zero regardless of the rounding
/// mode, and the integer indefinite value for NaN, the infinities and anything
/// outside `i32`. The portable arm reproduces it exactly, so the host tests
/// exercise the same two-path control flow the shipped image takes.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[inline]
fn trunc_to_i32(x: f64) -> i32 {
    #[cfg(target_arch = "x86")]
    use core::arch::x86::{_mm_cvttsd_si32, _mm_set_sd};
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::{_mm_cvttsd_si32, _mm_set_sd};

    // SAFETY: `_mm_set_sd` is SSE2, present on every ISA baseline this crate
    // builds for; it moves the 64 bits into lane 0 without rounding them.
    let v = unsafe { _mm_set_sd(x) };
    // SAFETY: `_mm_cvttsd_si32` is SSE2 as above, and reads lane 0 of an
    // initialized vector.
    unsafe { _mm_cvttsd_si32(v) }
}

/// Truncate toward zero into `i32`, answering `i32::MIN` where that cannot be done.
///
/// Portable model of the convert the x86 arm above issues; the bound is written
/// as an open interval so NaN takes the indefinite answer through the same
/// unordered-compare polarity the instruction has.
#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
#[inline]
fn trunc_to_i32(x: f64) -> i32 {
    if x > -2_147_483_649.0 && x < 2_147_483_648.0 {
        x as i32
    } else {
        i32::MIN
    }
}

/// The low and high halves of the argument's IEEE bit pattern.
///
/// A 32-bit target has no register pair wide enough to hold `to_bits`, so the
/// backend homes the double in the frame and reads the two dwords back out of
/// it; taking the halves out of the vector register the value already occupies
/// keeps the whole kernel off the stack. Needs SSE4.1 for the lane extract,
/// which the i686 baseline this crate ships on (`nehalem`) provides.
#[cfg(target_arch = "x86")]
#[inline]
fn raw_halves(x: f64) -> (u32, u32) {
    use core::arch::x86::{_mm_castpd_si128, _mm_cvtsi128_si32, _mm_extract_epi32, _mm_set_sd};

    // SAFETY: `_mm_set_sd` is SSE2; it moves the 64 bits into lane 0 verbatim,
    // with no rounding and no NaN canonicalization.
    let d = unsafe { _mm_set_sd(x) };
    // SAFETY: a reinterpretation of the same register, no operation on it.
    let v = unsafe { _mm_castpd_si128(d) };
    // SAFETY: `_mm_cvtsi128_si32` is SSE2 and reads lane 0 of an initialized
    // vector.
    let lo = unsafe { _mm_cvtsi128_si32(v) };
    // SAFETY: `_mm_extract_epi32` is SSE4.1, inside the `nehalem` baseline this
    // target builds with, and reads lane 1 of the same initialized vector.
    let hi = unsafe { _mm_extract_epi32::<1>(v) };
    (lo.cast_unsigned(), hi.cast_unsigned())
}

/// The low and high halves of the argument's IEEE bit pattern.
///
/// Every target with a 64-bit register bitcasts in place, so the halves come
/// straight out of `to_bits` with no stack traffic to avoid.
#[cfg(not(target_arch = "x86"))]
#[inline]
const fn raw_halves(x: f64) -> (u32, u32) {
    let bits = x.to_bits();
    (bits as u32, (bits >> 32) as u32)
}

#[cfg(test)]
mod tests_ftol__40a2b0 {
    use super::ftol__40a2b0 as ftol;

    /// In-range values match the hardware truncating cast exactly.
    #[test]
    fn matches_truncating_cast_in_range() {
        for &x in &[
            0.0_f64,
            -0.0,
            0.5,
            -0.5,
            0.999_999_9,
            1.0,
            1.5,
            -1.5,
            2.75,
            -2.75,
            12345.678,
            -12345.678,
            4_294_967_295.9,
            (1_u64 << 52) as f64 + 0.5,
            -((1_u64 << 52) as f64) - 0.5,
            9.223_372_036_854_774e18,  // largest f64 below 2^63
            -9.223_372_036_854_776e18, // exactly -2^63
            f64::MIN_POSITIVE,         // subnormal-adjacent -> 0
            5e-324,                    // smallest subnormal -> 0
        ] {
            assert_eq!(ftol(x), x as i64, "x={x:e}");
        }
    }

    /// NaN, infinities, and out-of-range magnitudes store the indefinite value.
    #[test]
    fn specials_store_indefinite() {
        for &x in &[
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            9.223_372_036_854_776e18, // exactly 2^63 (not representable)
            1e19,
            -1e19,
            f64::MAX,
            f64::MIN,
        ] {
            assert_eq!(ftol(x), i64::MIN, "x={x:e}");
        }
    }

    /// The narrow/wide handover reproduces the reference cast on both sides.
    ///
    /// `-2^31` is the input that reaches the indefinite sentinel legitimately,
    /// so the values bracketing it are what pin the two paths agreeing.
    #[test]
    fn narrow_path_handover_is_seamless() {
        for &x in &[
            -2_147_483_647.5_f64,
            -2_147_483_648.0,
            -2_147_483_648.5,
            -2_147_483_649.0,
            -2_147_483_650.0,
            2_147_483_646.5,
            2_147_483_647.0,
            2_147_483_647.5,
            2_147_483_648.0,
            2_147_483_649.0,
            -4_294_967_296.0,
            4_294_967_296.0,
        ] {
            assert_eq!(ftol(x), x as i64, "x={x:e}");
        }
    }

    /// Sweep the exponent range against the reference cast.
    #[test]
    fn exponent_sweep() {
        // Stepping the exponent rather than doubling an accumulator walks the
        // same powers of two, and keeps the loop bound an exact integer
        // comparison instead of a float one.
        for e in 0..63 {
            let x = 2.0_f64.powi(e);
            for &v in &[x, -x, x * 1.5, -x * 1.5, x + 0.25] {
                assert_eq!(ftol(v), v as i64, "v={v:e}");
            }
        }
    }
}
