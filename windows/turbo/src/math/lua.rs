//! `lua` family kernels.
// Adapter and kernel names mirror the host's C++ symbols verbatim, with `__`
// standing in for the `::`, so the whole module is non-snake-case by construction.
#![allow(non_snake_case)]

/// Horner-scheme evaluation of a polynomial.
///
/// The `degree + 1` coefficients are `coeffs[0..=degree]`, ordered high-to-low:
/// `acc = coeffs[0]` then for each `i` in `1..=degree`,
/// `acc = acc * x + coeffs[i]`. Returns the value at `x`.
///
/// A strictly serial multiply-add dependency chain (no `mul_add`: the baseline
/// has no hardware FMA, so it would lower to a slow libm call and change
/// rounding versus the reference's x87 `FMUL`/`FADD` loop).
///
/// The accumulator is `f64` because the original's never leaves the x87 stack:
/// `0x453623` loads the leading coefficient and the loop at `0x453630` is a bare
/// `FMUL`/`FADD` pair with no store in it or after it, so the whole chain runs
/// at register width however long the polynomial is. Rounding to `f32` after
/// every step, as this did, compounds that error once per degree.
///
/// **Returns `f64`, and that is the point.** The original stores nothing on the
/// way out either: the result leaves in `ST(0)` at register width and each of the
/// three call sites decides for itself whether to narrow. All three
/// (`0x4535c4`, `0x453679`, `0x453709`) go straight on to multiply it wide, so a
/// narrowing here is a rounding none of them perform. Returning `f32` put one in.
pub fn eval_polynomial_horner__453620(degree: u32, coeffs: &[f32], x: f32) -> f64 {
    let x = f64::from(x);
    let mut acc = f64::from(coeffs[0]);
    let mut i = 1u32;
    while i <= degree {
        acc = acc * x + f64::from(coeffs[i as usize]);
        i += 1;
    }
    acc
}

#[cfg(test)]
mod tests_eval_polynomial_horner__453620 {
    use super::eval_polynomial_horner__453620 as horner;

    #[test]
    fn degree_zero_is_constant() {
        // No iterations: returns coeffs[0] regardless of x.
        assert_eq!(
            horner(0, &[7.5, 1.0, 2.0], 99.0).to_bits(),
            7.5f64.to_bits()
        );
    }

    #[test]
    fn linear_known_value() {
        // coeffs = [a, b] => a*x + b. a=2,b=3,x=5 => 13.
        assert_eq!(horner(1, &[2.0, 3.0], 5.0).to_bits(), 13.0f64.to_bits());
    }

    #[test]
    fn quadratic_known_value() {
        // coeffs = [1,-3,2] => x^2 - 3x + 2; at x=4 => 16-12+2 = 6.
        assert_eq!(
            horner(2, &[1.0, -3.0, 2.0], 4.0).to_bits(),
            6.0f64.to_bits()
        );
    }

    #[test]
    fn evaluates_at_one_is_coefficient_sum() {
        // At x=1 a polynomial equals the sum of its coefficients.
        let c = [1.0f32, 2.0, 3.0, 4.0];
        let sum = c.iter().map(|v| f64::from(*v)).sum::<f64>();
        assert_eq!(horner(3, &c, 1.0).to_bits(), sum.to_bits());
    }

    #[test]
    fn matches_explicit_horner_reference() {
        // Independent explicit expansion for a cubic at an arbitrary x, at the
        // width the original evaluates it: the accumulator never leaves the x87
        // stack, so the chain is carried wide and narrowed once at the end.
        let c = [0.5f32, -1.25, 2.0, -0.75];
        let x = 1.7f32;
        let xw = f64::from(x);
        let want = ((f64::from(c[0]) * xw + f64::from(c[1])) * xw + f64::from(c[2])) * xw
            + f64::from(c[3]);
        assert_eq!(horner(3, &c, x).to_bits(), want.to_bits());

        // This fixture separates the two shapes, so the assertion above is not
        // vacuous. Before the width fix the reference was written per-step in
        // f32 and this test pinned the defect rather than the original.
        let per_step = ((c[0] * x + c[1]) * x + c[2]) * x + c[3];
        assert_ne!(
            want.to_bits(),
            f64::from(per_step).to_bits(),
            "fixture no longer separates narrow-once from narrow-per-step"
        );
    }
}

/// Lua number-key main-position index helper.
///
/// Biases the key `n` by `bias` (a fixed disambiguation constant, classically
/// `1.0`), reinterprets the resulting `f64` as raw bits, folds its two 32-bit
/// halves with a wrapping sum, reduces that modulo
/// `((1 << (lsizenode & 0x1f)) - 1) | 1`, and returns the byte offset
/// `node_base + idx * 0x28` of the chosen 40-byte node.
///
/// Pure integer hashing over the bit pattern; no floating-point arithmetic other
/// than the single bias add. The wrapping sum mirrors the 32-bit `ADD` in the
/// original (the two halves can carry out of 32 bits).
///
/// The halves are taken out of the vector register rather than through
/// `f64::to_bits`, which on i686 lowers to an 8-byte spill read back as two
/// dwords — the function's only stack object, and the whole reason its frame
/// gets realigned. `castpd_si128` is a bitcast, the two extracts are SSE2 data
/// moves, and the bias add itself is untouched, so every bit of the result is
/// the same on every input.
pub fn lua_h_hashnum__6fa260(n: f64, bias: f64, lsizenode: u8, node_base: u32) -> u32 {
    #[cfg(target_arch = "x86")]
    use core::arch::x86::{_mm_castpd_si128, _mm_cvtsi128_si32, _mm_set_sd, _mm_srli_si128};
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::{_mm_castpd_si128, _mm_cvtsi128_si32, _mm_set_sd, _mm_srli_si128};

    let biased = n + bias;

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    let (lo, hi) = {
        // SAFETY: `_mm_set_sd` moves a scalar into lane 0 of a fresh register;
        // SSE2, no memory or alignment precondition.
        let scalar = unsafe { _mm_set_sd(biased) };
        // SAFETY: a reinterpretation between vector types, touching no memory.
        let bits = unsafe { _mm_castpd_si128(scalar) };
        // SAFETY: reads dword lane 0 of an initialized vector.
        let lo = unsafe { _mm_cvtsi128_si32(bits) };
        // SAFETY: byte-shifts an initialized vector; SSE2, no precondition.
        let shifted = unsafe { _mm_srli_si128::<4>(bits) };
        // SAFETY: reads dword lane 0 of an initialized vector.
        let hi = unsafe { _mm_cvtsi128_si32(shifted) };
        (lo as u32, hi as u32)
    };
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    let (lo, hi) = {
        // Non-x86 fallback (never the parity target; keeps the crate buildable).
        let bits = biased.to_bits();
        (bits as u32, (bits >> 32) as u32)
    };

    let sum = hi.wrapping_add(lo);
    let divisor = ((1u32 << (lsizenode & 0x1f)) - 1) | 1;
    let idx = sum % divisor;
    node_base.wrapping_add(idx.wrapping_mul(0x28))
}

#[cfg(test)]
mod tests_lua_h_hashnum__6fa260 {
    use super::lua_h_hashnum__6fa260 as hashnum;

    // Independent oracle mirroring the documented reduction.
    fn oracle(n: f64, bias: f64, lsizenode: u8, node_base: u32) -> u32 {
        let bits = (n + bias).to_bits();
        let lo = (bits & 0xffff_ffff) as u32;
        let hi = (bits >> 32) as u32;
        let sum = hi.wrapping_add(lo);
        let divisor = ((1u32 << (u32::from(lsizenode) & 0x1f)) - 1) | 1;
        node_base + (sum % divisor) * 0x28
    }

    #[test]
    fn index_within_bucket_count() {
        // idx must be < divisor for every key; the returned offset is node-aligned.
        let bias = 1.0f64;
        for &ls in &[1u8, 2, 3, 5, 8] {
            let divisor = ((1u32 << (u32::from(ls) & 0x1f)) - 1) | 1;
            for &n in &[0.0f64, 1.0, -1.0, 42.0, 1e9, -7.25, 1234.5] {
                let off = hashnum(n, bias, ls, 0);
                assert_eq!(off % 0x28, 0, "offset not node-aligned");
                let idx = off / 0x28;
                assert!(idx < divisor, "idx {idx} >= divisor {divisor}");
            }
        }
    }

    #[test]
    fn matches_independent_oracle() {
        // The oracle takes the two halves through `f64::to_bits`, so this also
        // pins the register extraction the kernel uses in its place. `ls` covers
        // the two shifts that collapse the divisor to 1, and stops at 20 because
        // the oracle spells `idx * 0x28` without wrapping; the keys include the
        // values whose bit patterns have a distinctive half: the infinities
        // (zero low half), both zeros, a NaN, and a subnormal (zero high half).
        let bias = 1.0f64;
        for &ls in &[0u8, 1, 3, 4, 7, 20] {
            for &n in &[
                0.0f64,
                2.0,
                -3.5,
                100.0,
                -0.0,
                65536.0,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NAN,
                f64::MIN_POSITIVE / 4.0,
                f64::MAX,
            ] {
                assert_eq!(hashnum(n, bias, ls, 0x1000), oracle(n, bias, ls, 0x1000));
            }
        }
    }

    #[test]
    fn node_base_is_added() {
        // The chosen node lives at node_base + idx*0x28; idx for these is 0.
        let bias = 1.0f64;
        // lsizenode=0 -> divisor = ((1<<0)-1)|1 = 1 -> every key maps to idx 0.
        let off = hashnum(12345.0, bias, 0, 0x4000);
        assert_eq!(off, 0x4000);
    }

    #[test]
    fn positive_and_negative_zero_collapse_with_bias() {
        // bias of 1.0 maps both +0.0 and -0.0 to exactly 1.0, same bucket.
        let bias = 1.0f64;
        assert_eq!(hashnum(0.0, bias, 5, 0), hashnum(-0.0, bias, 5, 0));
    }
}

/// Primitive Lua value equality for two tagged values.
///
/// Equal requires matching type tags (`tag_a == tag_b`); a nil tag (`0`) is
/// then always equal, a number tag (`3`) compares the two `f64` payloads (so
/// `NaN != NaN`, matching the reference's ordered `FCOMPP`), and any other tag
/// compares the 32-bit value/pointer payloads.
pub fn lua_o_rawequal_obj__6f58b0(
    tag_a: i32,
    tag_b: i32,
    num_a: f64,
    num_b: f64,
    val_a: i32,
    val_b: i32,
) -> bool {
    if tag_a != tag_b {
        return false;
    }
    match tag_a {
        0 => true,
        3 => num_a == num_b,
        _ => val_a == val_b,
    }
}

#[cfg(test)]
mod tests_lua_o_rawequal_obj__6f58b0 {
    use super::lua_o_rawequal_obj__6f58b0 as raweq;

    #[test]
    fn mismatched_tags_never_equal() {
        // Same payloads, different tags -> not equal.
        assert!(!raweq(3, 4, 1.0, 1.0, 5, 5));
        assert!(!raweq(0, 1, 0.0, 0.0, 0, 0));
    }

    #[test]
    fn nil_is_always_equal() {
        // tag 0: equal regardless of any payload.
        assert!(raweq(0, 0, 9.0, -9.0, 1, 2));
    }

    #[test]
    fn number_compares_payload() {
        assert!(raweq(3, 3, 2.5, 2.5, 0, 0));
        assert!(!raweq(3, 3, 2.5, 2.6, 0, 0));
    }

    #[test]
    fn nan_number_is_not_equal() {
        // Ordered compare: NaN != NaN.
        assert!(!raweq(3, 3, f64::NAN, f64::NAN, 0, 0));
    }

    #[test]
    fn other_tag_compares_value_dword() {
        // Non-nil, non-number (e.g. boolean/string pointer): compare the dword.
        assert!(raweq(1, 1, 7.0, 8.0, 42, 42));
        assert!(!raweq(1, 1, 7.0, 8.0, 42, 43));
        assert!(raweq(4, 4, 0.0, 0.0, 0xdead_u32 as i32, 0xdead_u32 as i32));
    }

    #[test]
    fn reflexive_for_matching_values() {
        // A value equals itself in each tag class.
        assert!(raweq(0, 0, 1.0, 1.0, 9, 9));
        assert!(raweq(3, 3, -3.25, -3.25, 9, 9));
        assert!(raweq(2, 2, 0.0, 0.0, 77, 77));
    }
}

/// How `lua_h_get__6fa7a0` routes a table lookup.
///
/// Based on the key's tag and (for numbers) its integer-exactness.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum LuaGetRoute {
    /// Number key whose `f64` value is an exact `i32`.
    ///
    /// Use the integer fast path (`luaH_getnum`) with the carried truncated key.
    NumberExactInt(i32),
    /// String key: use the interned-string lookup (`luaH_getstr`).
    String,
    /// Anything else (non-exact number, boolean, nil, ...).
    ///
    /// Generic node walk (`luaH_getany`).
    Any,
}

/// Classify a key for `lua_h_get__6fa7a0`.
///
/// Tag `3` is a number — if its `f64` value truncates losslessly to `i32`
/// (`(int)n as f64 == n`, matching the reference's `__ftol` + `FILD` + `FCOMPP`
/// equal test) it routes to the integer fast path carrying that `i32`;
/// otherwise to the generic walk. Tag `4` is a string. Every other tag uses the
/// generic walk.
pub fn lua_h_get_route__6fa7a0(key_tag: i32, key_num: f64) -> LuaGetRoute {
    if key_tag == 3 {
        let truncated = key_num as i32;
        if truncated as f64 == key_num {
            return LuaGetRoute::NumberExactInt(truncated);
        }
    } else if key_tag == 4 {
        return LuaGetRoute::String;
    }
    LuaGetRoute::Any
}

#[cfg(test)]
mod tests_lua_h_get__6fa7a0 {
    use super::{LuaGetRoute, lua_h_get_route__6fa7a0 as route};

    #[test]
    fn route_exact_int() {
        assert_eq!(route(3, 5.0), LuaGetRoute::NumberExactInt(5));
        assert_eq!(route(3, -123.0), LuaGetRoute::NumberExactInt(-123));
        assert_eq!(route(3, 0.0), LuaGetRoute::NumberExactInt(0));
    }
    #[test]
    fn route_non_exact_number_is_any() {
        assert_eq!(route(3, 2.5), LuaGetRoute::Any);
        assert_eq!(route(3, f64::NAN), LuaGetRoute::Any);
        // too large for i32: the `as i32` saturates, so the round-trip fails -> Any.
        assert_eq!(route(3, 1e18), LuaGetRoute::Any);
    }
    #[test]
    fn route_string() {
        assert_eq!(route(4, 0.0), LuaGetRoute::String);
    }
    #[test]
    fn route_other_tags_any() {
        for tag in &[0, 1, 2, 5, 6] {
            assert_eq!(route(*tag, 1.0), LuaGetRoute::Any);
        }
    }
    #[test]
    fn route_int_roundtrip_property() {
        for &v in &[i32::MIN, -1000, -1, 0, 1, 1000, 2_000_000, i32::MAX] {
            assert_eq!(route(3, v as f64), LuaGetRoute::NumberExactInt(v));
        }
    }
}

/// Array-part fast path for an integer-keyed table fetch.
///
/// A key in `1..=sizearray` indexes the contiguous array directly: the slot
/// lives at `array_base + (key - 1) * 0x10` (16-byte tagged values). Returns
/// the byte offset of the slot, or `None` when the key falls outside the array
/// part and the caller must fall back to the hash chain.
pub fn lua_h_getnum_array_slot__6fa700(key: i32, sizearray: i32, array_base: u32) -> Option<u32> {
    if key > 0 && key <= sizearray {
        // array_base + key*0x10 - 0x10, matching the LEA [base + key*16 - 16].
        Some(
            array_base
                .wrapping_add((key as u32).wrapping_mul(0x10))
                .wrapping_sub(0x10),
        )
    } else {
        None
    }
}

/// Does a hash node match the wanted integer key?
///
/// A node matches when its type tag is `3` (number) and its stored `f64`
/// payload equals the key promoted to `f64`. The equality is ordered (`==`), so
/// a `NaN` payload never matches — mirroring the reference's
/// `FCOMP`/`TEST AH,0x44`/`JNP` equal test.
pub fn lua_h_getnum_node_matches__6fa700(node_tag: i32, node_num: f64, key: i32) -> bool {
    node_tag == 3 && node_num == key as f64
}

#[cfg(test)]
mod tests_lua_h_getnum__6fa700 {
    use super::{
        lua_h_getnum_array_slot__6fa700 as array_slot,
        lua_h_getnum_node_matches__6fa700 as node_matches,
    };

    #[test]
    fn array_slot_in_range() {
        // key 1 -> base + 0 ; key 2 -> base + 0x10 ; key n -> base + (n-1)*0x10.
        assert_eq!(array_slot(1, 4, 0x1000), Some(0x1000));
        assert_eq!(array_slot(2, 4, 0x1000), Some(0x1010));
        assert_eq!(array_slot(4, 4, 0x1000), Some(0x1030));
    }
    #[test]
    fn array_slot_out_of_range_is_none() {
        assert_eq!(array_slot(0, 4, 0x1000), None);
        assert_eq!(array_slot(5, 4, 0x1000), None);
        assert_eq!(array_slot(-3, 4, 0x1000), None);
    }
    #[test]
    fn array_slot_stride_is_node_aligned() {
        for k in 1..=8 {
            let off = array_slot(k, 8, 0).unwrap();
            assert_eq!(off % 0x10, 0);
            assert_eq!(off, (k as u32 - 1) * 0x10);
        }
    }
    #[test]
    fn empty_array_rejects_all() {
        for k in -2..=3 {
            assert_eq!(array_slot(k, 0, 0x500), None);
        }
    }
    #[test]
    fn node_matches_exact() {
        assert!(node_matches(3, 42.0, 42));
        assert!(!node_matches(3, 41.0, 42));
    }
    #[test]
    fn node_matches_requires_number_tag() {
        assert!(!node_matches(4, 42.0, 42));
        assert!(!node_matches(0, 42.0, 42));
    }
    #[test]
    fn node_matches_nan_never() {
        assert!(!node_matches(3, f64::NAN, 0));
    }
    #[test]
    fn node_matches_zero_and_negatives() {
        assert!(node_matches(3, 0.0, 0));
        assert!(node_matches(3, -7.0, -7));
    }
}

/// Lua main-position table-node selector.
///
/// Given a key's type tag `tt` and the table's node array (`node_base`
/// absolute address, `lsizenode` size exponent), returns the byte offset of the
/// chosen 40-byte (`0x28`) node: `node_base + idx * 0x28`.
///
/// The index is derived per tag, mirroring the original dispatch: a boolean/`nil`
/// key (`tt = 1`) and a string key (`tt = 4`) mask the supplied 32-bit value by
/// `(1 << (lsizenode & 0x1f)) - 1`; an integer-ish key (`tt = 2`) and any other
/// tag reduce it modulo `((1 << (lsizenode & 0x1f)) - 1) | 1`; a number key
/// (`tt = 3`) routes through [`lua_h_hashnum__6fa260`] over the biased `f64`.
/// `key_u32` carries the pre-resolved 32-bit key word (for strings this is the
/// string's cached hash, resolved by the caller); `key_num`/`bias` feed the
/// number path. Pure integer indexing with a single bias add on the number path.
pub fn lua_h_mainposition__6fa1a0(
    tt: u32,
    node_base: u32,
    lsizenode: u8,
    key_u32: u32,
    key_num: f64,
    bias: f64,
) -> u32 {
    let shift = u32::from(lsizenode) & 0x1f;
    let size_mask = (1u32 << shift).wrapping_sub(1);
    let idx = match tt {
        1 | 4 => size_mask & key_u32,
        3 => return lua_h_hashnum__6fa260(key_num, bias, lsizenode, node_base),
        // tt == 2 and any other tag share the modulo path.
        _ => key_u32 % (size_mask | 1),
    };
    node_base.wrapping_add(idx.wrapping_mul(0x28))
}

#[cfg(test)]
mod tests_lua_h_mainposition__6fa1a0 {
    use super::{lua_h_hashnum__6fa260 as hashnum, lua_h_mainposition__6fa1a0 as mainpos};

    fn size_mask(lsizenode: u8) -> u32 {
        (1u32 << (u32::from(lsizenode) & 0x1f)).wrapping_sub(1)
    }

    #[test]
    fn mask_path_for_bool_and_string() {
        // tt=1 and tt=4 both AND-mask the key word by (2^lsizenode - 1).
        let base = 0x1000u32;
        for &ls in &[1u8, 3, 5, 8] {
            for &k in &[0u32, 1, 7, 0xdead_beef, 0xffff_ffff] {
                let want = base + (size_mask(ls) & k) * 0x28;
                assert_eq!(mainpos(1, base, ls, k, 0.0, 1.0), want);
                assert_eq!(mainpos(4, base, ls, k, 0.0, 1.0), want);
            }
        }
    }

    #[test]
    fn modulo_path_for_int_and_default() {
        // tt=2 and any unrecognised tag reduce modulo ((2^lsizenode - 1) | 1).
        let base = 0x2000u32;
        for &ls in &[1u8, 2, 4, 7] {
            let divisor = size_mask(ls) | 1;
            for &k in &[0u32, 5, 100, 65537, 0xffff_ffff] {
                let want = base + (k % divisor) * 0x28;
                assert_eq!(mainpos(2, base, ls, k, 0.0, 1.0), want);
                // Tags 0, 5, 99 (not 1/3/4) take the same default path.
                assert_eq!(mainpos(0, base, ls, k, 0.0, 1.0), want);
                assert_eq!(mainpos(99, base, ls, k, 0.0, 1.0), want);
            }
        }
    }

    #[test]
    fn number_path_delegates_to_hashnum() {
        // tt=3 must equal the folded hashnum kernel exactly.
        let base = 0x4000u32;
        let bias = 1.0f64;
        for &ls in &[1u8, 3, 5] {
            for &n in &[0.0f64, -0.0, 1.0, -7.25, 1e9, 1234.5] {
                assert_eq!(mainpos(3, base, ls, 0, n, bias), hashnum(n, bias, ls, base));
            }
        }
    }

    #[test]
    fn offsets_are_node_aligned() {
        // Every returned offset is a node-aligned multiple of 0x28.
        let base = 0u32;
        let bias = 1.0f64;
        for &ls in &[1u8, 3, 5, 8] {
            for tt in [0u32, 1, 2, 3, 4, 7] {
                let off = mainpos(tt, base, ls, 0x1234_5678, 42.5, bias);
                assert_eq!(off % 0x28, 0, "tt={tt} not aligned");
            }
        }
    }

    #[test]
    fn known_small_table() {
        // lsizenode=2 -> mask=3, divisor=3. Bool key value 5 (tt=1): 3 & 5 = 1 -> off 0x28.
        assert_eq!(mainpos(1, 0, 2, 5, 0.0, 1.0), 0x28);
        // Int key value 5 (tt=2): 5 % 3 = 2 -> off 0x50.
        assert_eq!(mainpos(2, 0, 2, 5, 0.0, 1.0), 0x50);
        // lsizenode=0 -> mask=0, divisor=1: every key maps to idx 0.
        assert_eq!(mainpos(2, 0x9000, 0, 0xffff_ffff, 0.0, 1.0), 0x9000);
        assert_eq!(mainpos(1, 0x9000, 0, 0xffff_ffff, 0.0, 1.0), 0x9000);
    }
}

/// Validity of a key being inserted into a table by `lua_h_set__6fa840`.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum LuaSetKey {
    /// Nil key — illegal; the reference raises "table index is nil".
    Nil,
    /// Number key whose value is `NaN` — illegal; "table index is NaN".
    Nan,
    /// A usable key: insert it.
    Ok,
}

/// Classify a not-yet-present key for `lua_h_set__6fa840`.
///
/// A nil tag (`0`) is rejected as `Nil`; a number tag (`3`) whose payload is
/// `NaN` (the self-unequal `n != n` test) is rejected as `Nan`; every other key
/// is `Ok` to insert.
pub fn lua_h_set_classify_key__6fa840(key_tag: i32, key_num: f64) -> LuaSetKey {
    if key_tag == 0 {
        LuaSetKey::Nil
    } else if key_tag == 3 && key_num.is_nan() {
        LuaSetKey::Nan
    } else {
        LuaSetKey::Ok
    }
}

#[cfg(test)]
mod tests_lua_h_set__6fa840 {
    use super::{LuaSetKey, lua_h_set_classify_key__6fa840 as classify};

    #[test]
    fn classify_nil() {
        assert_eq!(classify(0, 1.0), LuaSetKey::Nil);
    }
    #[test]
    fn classify_nan() {
        assert_eq!(classify(3, f64::NAN), LuaSetKey::Nan);
    }
    #[test]
    fn classify_ok_number() {
        assert_eq!(classify(3, 3.5), LuaSetKey::Ok);
        assert_eq!(classify(3, 0.0), LuaSetKey::Ok);
    }
    #[test]
    fn classify_ok_other_tags() {
        for tag in &[1, 2, 4, 5] {
            assert_eq!(classify(*tag, f64::NAN), LuaSetKey::Ok);
        }
    }
    #[test]
    fn classify_non_number_nan_payload_is_ok() {
        // NaN payload only matters for the number tag; tag 4 ignores it.
        assert_eq!(classify(4, f64::NAN), LuaSetKey::Ok);
    }
}

/// Maps the extended-precision scanner's flag word and the narrowing helper's retcode.
///
/// Onto the scan-result flags: `0x80` when either step reported range-high,
/// `0x100` (combinable) when either reported range-low. The no-conversion case
/// (`scan_flags & 4`) never reaches the narrowing step and is handled by the
/// caller.
pub fn scan_double_token__747560(scan_flags: u32, narrow_rc: i32) -> u32 {
    let mut flags = 0;
    if scan_flags & 2 != 0 || narrow_rc == 1 {
        flags = 0x80;
    }
    if scan_flags & 1 != 0 || narrow_rc == 2 {
        flags |= 0x100;
    }
    flags
}

#[cfg(test)]
mod tests_scan_double_token__747560 {
    use super::scan_double_token__747560 as map_flags;

    #[test]
    fn clean_conversion_has_no_flags() {
        assert_eq!(map_flags(0, 0), 0);
    }

    #[test]
    fn range_high_from_either_step() {
        assert_eq!(map_flags(2, 0), 0x80);
        assert_eq!(map_flags(0, 1), 0x80);
        assert_eq!(map_flags(2, 1), 0x80);
    }

    #[test]
    fn range_low_from_either_step() {
        assert_eq!(map_flags(1, 0), 0x100);
        assert_eq!(map_flags(0, 2), 0x100);
        assert_eq!(map_flags(1, 2), 0x100);
    }

    #[test]
    fn both_ranges_combine() {
        assert_eq!(map_flags(3, 0), 0x180);
        assert_eq!(map_flags(2, 2), 0x180);
        assert_eq!(map_flags(1, 1), 0x180);
    }

    #[test]
    fn unrelated_bits_and_codes_are_ignored() {
        assert_eq!(map_flags(8, 0), 0);
        assert_eq!(map_flags(0, 3), 0);
        assert_eq!(map_flags(0, -1), 0);
    }
}

/// Decides whether a constant array of `size` slots must grow.
///
/// Before appending entry number `count` (zero-based): grow when `count + 1`
/// would exceed the capacity. Counts are host-capped at `0x3ffff`, far below
/// overflow.
pub fn lua_k_addk__700ad0(count: i32, size: i32) -> bool {
    count + 1 > size
}

#[cfg(test)]
mod tests_lua_k_addk__700ad0 {
    use super::lua_k_addk__700ad0 as needs_grow;

    #[test]
    fn empty_array_grows_for_the_first_entry() {
        assert!(needs_grow(0, 0));
    }

    #[test]
    fn spare_capacity_does_not_grow() {
        assert!(!needs_grow(0, 4));
        assert!(!needs_grow(3, 4));
        assert!(!needs_grow(0x3fffe, 0x3ffff));
    }

    #[test]
    fn full_array_grows() {
        assert!(needs_grow(4, 4));
        assert!(needs_grow(0x3ffff, 0x3ffff));
    }
}

/// Accepts a string-to-number conversion.
///
/// The scan must have consumed at least one character and the remaining tail —
/// already whitespace-skipped by the caller — must terminate immediately.
/// Returns the host's `1`/`0`.
pub fn lua_o_str2d__6f5900(consumed_any: bool, tail_first_byte: u8) -> i32 {
    i32::from(consumed_any && tail_first_byte == 0)
}

#[cfg(test)]
mod tests_lua_o_str2d__6f5900 {
    use super::lua_o_str2d__6f5900 as accept;

    #[test]
    fn nothing_consumed_is_rejected() {
        assert_eq!(accept(false, 0), 0);
        assert_eq!(accept(false, b'9'), 0);
    }

    #[test]
    fn consumed_with_clean_tail_is_accepted() {
        assert_eq!(accept(true, 0), 1);
    }

    #[test]
    fn trailing_garbage_is_rejected() {
        assert_eq!(accept(true, b'x'), 0);
        assert_eq!(accept(true, b' '), 0);
    }
}

/// Classifies the scan-result flag word into the conversion outcome.
///
/// `0` use the parsed value, `1` no conversion (zero result, end pointer
/// rewound), `2` range high (signed huge result + range errno), `3` range low
/// (zero + range errno). The no-conversion mask (`0x240`) wins over both range
/// bits, and range high (`0x81` mask) wins over range low when both are
/// reported.
pub fn strtod__741e39(flags: u32) -> u32 {
    if flags & 0x240 != 0 {
        1
    } else if flags & 0x81 != 0 {
        2
    } else if flags & 0x100 != 0 {
        3
    } else {
        0
    }
}

#[cfg(test)]
mod tests_strtod__741e39 {
    use super::strtod__741e39 as outcome;

    #[test]
    fn clean_flags_use_the_parsed_value() {
        assert_eq!(outcome(0), 0);
    }

    #[test]
    fn no_conversion_mask_matches_either_bit() {
        assert_eq!(outcome(0x200), 1);
        assert_eq!(outcome(0x40), 1);
        assert_eq!(outcome(0x240), 1);
    }

    #[test]
    fn no_conversion_wins_over_range_bits() {
        assert_eq!(outcome(0x280), 1);
        assert_eq!(outcome(0x300), 1);
    }

    #[test]
    fn range_high_paths() {
        assert_eq!(outcome(0x80), 2);
        assert_eq!(outcome(0x01), 2);
        // High wins when the scanner reported both ranges.
        assert_eq!(outcome(0x180), 2);
    }

    #[test]
    fn range_low_path() {
        assert_eq!(outcome(0x100), 3);
    }
}

/// Stock Lua 5.0 string hash — the sampling recurrence inlined in `luaS_newlstr@0x6f9d00`.
///
/// Seeds with the length, then folds one byte every `step = (len >> 5) + 1`
/// positions walking from the tail, so it inspects at most ~32 bytes regardless
/// of length: `h ^= (h << 5) + (h >> 2) + byte`, all 32-bit wrapping.
///
/// Reproduced bit-for-bit because the interner reimpl's dual-probe fallback
/// re-hashes a miss with this function to find strings that were interned (and
/// hashed with it) before the hook was installed; they must land in the bucket
/// the stock code originally chose for them.
pub fn lua_s_newlstr_hash_stock__6f9d00(bytes: &[u8]) -> u32 {
    let len = bytes.len() as u32;
    let step = (len >> 5) + 1;
    let mut hash = len;
    // Driven by an iterator rather than the recurrence's own `while step <= l1`
    // walk indexing `bytes[(l1 - 1) as usize]`, which costs a bounds check and an
    // out-of-line panic block because nothing proves `l1 - 1 < len`. The sequence
    // is unchanged: `l1` takes `len - k * step` and the guard admits exactly
    // `floor(len / step)` of them, so the sampled indices are `len - 1 - k * step`
    // walking back from the tail, and at `len == 0` the count is 0 and neither
    // form folds a byte. `take` is what makes the count exact: a bare `step_by`
    // yields the `ceil` and would fold one byte too many whenever `step` does
    // not divide `len`. Its quotient is the one division in this family.
    for &b in bytes
        .iter()
        .rev()
        .step_by(step as usize)
        .take((len / step) as usize)
    {
        let term = u32::from(b).wrapping_add(hash << 5).wrapping_add(hash >> 2);
        hash ^= term;
    }
    hash
}

/// FNV-1a (32-bit) over the whole byte string.
///
/// The interner reimpl's hash, replacing the stock sampling recurrence. Every
/// byte is mixed (`xor` then a wrapping multiply by the FNV prime), so
/// structurally similar long strings (combat-log lines, chat, item links) that
/// the stock hash buckets together by its ~32-byte tail sample are spread
/// across buckets. That shortens the collision chains the interner walks — and
/// the full-length `memcmp` each chain node otherwise costs. Portable and
/// SIMD-free (the shipped i686 build stays 0-ymm), deterministic, returns the
/// value stored at `TString+0x08`.
pub fn lua_s_newlstr_hash__6f9d00(bytes: &[u8]) -> u32 {
    const FNV_OFFSET_BASIS: u32 = 0x811c_9dc5;
    const FNV_PRIME: u32 = 0x0100_0193;
    let mut hash = FNV_OFFSET_BASIS;
    for &b in bytes {
        hash ^= u32::from(b);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests_lua_s_newlstr_hash__6f9d00 {
    use super::{lua_s_newlstr_hash__6f9d00 as fnv, lua_s_newlstr_hash_stock__6f9d00 as stock};

    // Independent oracle for the stock recurrence (operates on `usize` indices,
    // unlike the kernel's tail-walk arithmetic, so a transcription slip diverges).
    fn stock_oracle(bytes: &[u8]) -> u32 {
        let len = bytes.len() as u32;
        let step = (len >> 5) + 1;
        let mut hash = len;
        let mut l1 = len as i64;
        while l1 >= step as i64 {
            let b = u32::from(bytes[(l1 - 1) as usize]);
            hash ^= b
                .wrapping_add(hash.wrapping_mul(32))
                .wrapping_add(hash >> 2);
            l1 -= step as i64;
        }
        hash
    }

    #[test]
    fn stock_empty_is_zero() {
        // len seed 0, no iterations.
        assert_eq!(stock(b""), 0);
    }

    #[test]
    fn stock_matches_oracle_over_lengths() {
        // Cover step == 1 (len <= 31) and step > 1 (len >= 32) sampling regimes.
        let mut state: u32 = 0x9e37_79b9;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        for len in 0usize..200 {
            let s: Vec<u8> = (0..len).map(|_| (next() & 0xff) as u8).collect();
            assert_eq!(stock(&s), stock_oracle(&s), "len={len} s={s:?}");
        }
    }

    #[test]
    fn both_are_deterministic() {
        let s = b"Interface\\AddOns\\Foo\\bar.lua";
        assert_eq!(stock(s), stock(s));
        assert_eq!(fnv(s), fnv(s));
    }

    #[test]
    fn fnv_known_vectors() {
        // Canonical FNV-1a 32-bit reference vectors.
        assert_eq!(fnv(b""), 0x811c_9dc5);
        assert_eq!(fnv(b"a"), 0xe40c_292c);
        assert_eq!(fnv(b"foobar"), 0xbf9c_f968);
    }

    // The exact win the reimpl targets: a block of length-64 strings identical at
    // every byte the stock sampler inspects (len 64 -> step 3 -> it folds only
    // indices that are multiples of 3, plus the length seed) and differing only
    // at index 1, which the sampler skips. Stock hashes them all identically (one
    // giant bucket + a full-length memcmp per probe); FNV-1a inspects index 1 and
    // spreads them out.
    fn stock_adversarial_block() -> Vec<Vec<u8>> {
        (0u16..256)
            .map(|i| {
                let mut s = vec![b'A'; 64];
                s[1] = i as u8;
                s
            })
            .collect()
    }

    #[test]
    fn stock_collides_adversarial_block_into_one_value() {
        let block = stock_adversarial_block();
        let h0 = stock(&block[0]);
        assert!(
            block.iter().all(|s| stock(s) == h0),
            "stock hash should be identical across the block",
        );
    }

    #[test]
    fn fnv_spreads_adversarial_block() {
        let block = stock_adversarial_block();
        let mut hashes: Vec<u32> = block.iter().map(|s| fnv(s)).collect();
        hashes.sort_unstable();
        hashes.dedup();
        assert_eq!(hashes.len(), 256, "FNV must distinguish all 256 strings");
    }

    #[test]
    fn fnv_beats_stock_total_probe_cost() {
        // Build a WoW-flavoured corpus: addon paths, item links, and long
        // combat-log-shaped lines (all > 32 bytes, so the stock sampler skips
        // most of each), plus the adversarial block. Bucket each hash into a
        // power-of-two table the size Lua would pick (load factor ~1) and score
        // the interner's worst case: sum of k*(k+1)/2 over buckets (the probes a
        // full re-intern of every key would walk) and the single longest chain.
        let mut corpus: Vec<Vec<u8>> = Vec::new();
        let mut state: u32 = 0x1234_5678;
        let mut next = |m: u32| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state % m
        };
        for i in 0..1500u32 {
            let kind = next(3);
            let line = match kind {
                0 => format!("Interface\\AddOns\\Mod{}\\frame_{}.lua", next(40), i),
                1 => format!(
                    "|cffffffff|Hitem:{}:0:0:0|h[Epic Sword of Testing {}]|h|r",
                    next(20000),
                    next(99)
                ),
                _ => format!(
                    "[{:02}:{:02}:{:02}] You hit Training Dummy for {} Physical damage. ({} blocked) [seq {}]",
                    next(24),
                    next(60),
                    next(60),
                    next(9999),
                    next(500),
                    i
                ),
            };
            corpus.push(line.into_bytes());
        }
        corpus.extend(stock_adversarial_block());

        let size: usize = corpus.len().next_power_of_two();
        let mask = (size - 1) as u32;
        let score = |hashes: &[u32]| -> (u64, u32) {
            let mut buckets = vec![0u32; size];
            for &h in hashes {
                buckets[(h & mask) as usize] += 1;
            }
            let cost: u64 = buckets
                .iter()
                .map(|&k| u64::from(k) * (u64::from(k) + 1) / 2)
                .sum();
            let max = buckets.iter().copied().max().unwrap_or(0);
            (cost, max)
        };

        let stock_hashes: Vec<u32> = corpus.iter().map(|s| stock(s)).collect();
        let fnv_hashes: Vec<u32> = corpus.iter().map(|s| fnv(s)).collect();
        let (stock_cost, stock_max) = score(&stock_hashes);
        let (fnv_cost, fnv_max) = score(&fnv_hashes);

        assert!(
            fnv_cost < stock_cost,
            "FNV total probe cost {fnv_cost} should beat stock {stock_cost}",
        );
        assert!(
            fnv_max < stock_max,
            "FNV longest chain {fnv_max} should beat stock {stock_max}",
        );
        // FNV at load factor ~1 should stay near-ideal: no pathological chain.
        assert!(
            fnv_max <= 12,
            "FNV longest chain {fnv_max} unexpectedly long"
        );
    }
}
