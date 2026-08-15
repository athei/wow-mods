//! Bounded fills and compares that never reach a CRT helper.
//!
//! Third sibling to [`crate::ftol`] and [`crate::trig`], and here for the same
//! reason those exist: the compiler's lowering of an operation is a libcall,
//! and the libcall is the defect. Every entry below stands in for a *general*
//! CRT helper (arbitrary length, arbitrary alignment) at a site where the
//! length is fixed at compile time, so the callee re-derives at run time a
//! fact the caller already had.
//!
//! Two lowerings feed that, and one rule cures both. `[u32; N] == [u32; N]`
//! and `&[u8] == &[u8]` go through the standard library's `PartialEq`
//! specialization, which reaches the `bcmp` intrinsic however small `N` is,
//! and `bcmp` needs an *address* per side, so operands that lived in registers
//! are spilled to give it one. Separately, a fill loop whose trip count is a
//! runtime value becomes a `memset` under loop-idiom recognition even where
//! the count is provably at most four. The rule: the trip count is a constant,
//! and a runtime value is a predicate, never a bound.
//!
//! Each entry takes raw pointers rather than references or slices, because
//! nothing here needs an address to exist and taking one is exactly what puts
//! the operands back in memory. Nothing here copies, so no entry has an
//! overlap precondition: the compares may be handed the same run twice, and
//! the fill writes a region it never reads.
//!
//! The bodies are an implementation detail. What they have to deliver is the
//! absence of the call in the linked image, which is a property of the
//! machine code and not of the source shape.

/// Whether `N` `u32` lanes at `a` and `b` hold the same bits.
///
/// The XOR-into-OR fold is the point: it has no early exit, so it is not the
/// `bcmp` idiom and it reads both runs in full. Comparing lanes as `u32`
/// rather than as floats is likewise deliberate where the caller's data is
/// float: a NaN equals itself here and `-0.0` does not equal `0.0`, which is
/// the meaning a bit compare already had.
///
/// # Safety
/// Caller asserts `N` `u32` lanes are readable at each of `lhs` and `rhs`.
/// Neither has to be aligned: both sides are read unaligned. Passing the same
/// run twice is allowed.
#[must_use]
#[inline]
pub const unsafe fn u32_lanes_eq<const N: usize>(lhs: *const u32, rhs: *const u32) -> bool {
    let mut diff = 0_u32;
    let mut lane = 0;
    while lane < N {
        // SAFETY: `lane < N` and the caller asserted N lanes are readable at
        // `lhs`, so the offset stays inside that run.
        let from_lhs = unsafe { lhs.add(lane) };
        // SAFETY: the same assertion, made for `rhs` at the same index.
        let from_rhs = unsafe { rhs.add(lane) };
        // SAFETY: `from_lhs` is in bounds by the assertion above, and an
        // unaligned read carries no alignment requirement of its own.
        let left = unsafe { from_lhs.read_unaligned() };
        // SAFETY: as `from_lhs`, for the lane read out of `rhs`.
        let right = unsafe { from_rhs.read_unaligned() };
        diff |= left ^ right;
        lane += 1;
    }
    diff == 0
}

/// Whether the `len` bytes at `p` are the literal packed into `head` and `tail`.
///
/// The form for a short key compared against a compile-time name once the
/// stored length has already been matched against the literal's: `head` is the
/// first four bytes in little-endian order, `tail` is the fifth, so the whole
/// compare is one unaligned word plus a byte and no length parameter reaches a
/// callee. `len` selects between the two shapes rather than bounding a loop.
///
/// Four and five are the widths this packing spans; a longer literal needs a
/// wider head, and the callers that have one are all four or five bytes.
///
/// # Safety
/// Caller asserts `len` is the run's true length and is 4 or 5, and that the
/// run is readable at `p`. The head word is read unconditionally, so a shorter
/// run is out of bounds; the fifth byte is read only when `len` says it is
/// there. No alignment is required.
#[must_use]
#[inline]
pub const unsafe fn short_bytes_eq(p: *const u8, len: usize, head: u32, tail: u8) -> bool {
    // SAFETY: the caller asserted at least four readable bytes at `p`, and an
    // unaligned read carries no alignment requirement of its own.
    let word = unsafe { p.cast::<u32>().read_unaligned() };
    if word != head {
        return false;
    }
    if len == 4 {
        return true;
    }
    // SAFETY: `len` is 5 here, so a fifth byte exists in the run the caller
    // asserted readable.
    let last = unsafe { p.add(4) };
    // SAFETY: `last` is in bounds by the assertion above.
    let last = unsafe { last.read_unaligned() };
    last == tail
}

/// Zero a `ROWS`-by-`LANES` grid of `u32`, skipping the first `skip` rows.
///
/// The rule in one signature: the trip count is `ROWS * LANES` and `skip` is a
/// predicate tested per row, never a loop bound, so no runtime extent reaches
/// a fill helper. A `skip` at or past `ROWS` writes nothing, which is what a
/// `for i in skip..ROWS` fill already did.
///
/// # Safety
/// Caller asserts `ROWS * LANES` `u32` are writable contiguously at `dst`. No
/// alignment is required: the stores are unaligned.
#[inline]
pub const unsafe fn zero_rows_from<const ROWS: usize, const LANES: usize>(
    dst: *mut u32,
    skip: usize,
) {
    let mut row = 0;
    while row < ROWS {
        if row >= skip {
            let mut lane = 0;
            while lane < LANES {
                // SAFETY: `row < ROWS` and `lane < LANES`, so the index is
                // inside the run the caller asserted writable.
                let slot = unsafe { dst.add(row * LANES + lane) };
                // SAFETY: `slot` is in bounds by the assertion above, and an
                // unaligned store carries no alignment requirement.
                unsafe { slot.write_unaligned(0) };
                lane += 1;
            }
        }
        row += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lanes packed into a byte buffer at `off`, so the run starts unaligned.
    ///
    /// Returns the buffer; the caller reads the run at `off` through a raw
    /// pointer, which is the shape every entry in this module takes.
    fn packed(lanes: &[u32], off: usize) -> Vec<u8> {
        let mut buf = vec![0_u8; off + lanes.len() * 4 + 8];
        for (i, v) in lanes.iter().enumerate() {
            buf[off + i * 4..off + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        buf
    }

    /// A distinct, well-spread value for lane `i`.
    fn lane_value(i: usize) -> u32 {
        u32::try_from(i)
            .expect("lane index fits in u32")
            .wrapping_mul(0x9e37_79b9)
    }

    /// Four lanes at `off_lhs` in `lhs` against four at `off_rhs` in `rhs`.
    ///
    /// The offsets are byte offsets, so every combination of them puts at
    /// least one side at an address the hardware would fault a `movaps` on.
    fn eq4(lhs: &[u8], off_lhs: usize, rhs: &[u8], off_rhs: usize) -> bool {
        // SAFETY: the caller packs four lanes at `off_lhs` before calling, so
        // the offset is inside the buffer.
        let from_lhs = unsafe { lhs.as_ptr().add(off_lhs) };
        // SAFETY: as above, for the right-hand buffer.
        let from_rhs = unsafe { rhs.as_ptr().add(off_rhs) };
        // SAFETY: four readable lanes sit at each offset, and neither pointer
        // is asserted aligned.
        unsafe { u32_lanes_eq::<4>(from_lhs.cast(), from_rhs.cast()) }
    }

    /// The fold answers what `==` on the two arrays answers, at every width.
    #[test]
    fn lanes_eq_matches_array_compare() {
        let a: [u32; 16] = core::array::from_fn(lane_value);
        let mut b = a;
        // SAFETY: both runs are 16 live lanes of a local array.
        assert!(unsafe { u32_lanes_eq::<16>(a.as_ptr(), b.as_ptr()) });
        // SAFETY: as above, at the narrower widths the call sites use.
        assert!(unsafe { u32_lanes_eq::<3>(a.as_ptr(), b.as_ptr()) });
        // SAFETY: as above.
        assert!(unsafe { u32_lanes_eq::<12>(a.as_ptr(), b.as_ptr()) });
        b[15] ^= 1;
        // SAFETY: as above; the differing lane is inside the 16-lane run.
        assert!(!unsafe { u32_lanes_eq::<16>(a.as_ptr(), b.as_ptr()) });
        // SAFETY: as above; the differing lane sits outside the 12-lane run.
        assert!(unsafe { u32_lanes_eq::<12>(a.as_ptr(), b.as_ptr()) });
    }

    /// A single differing lane is caught wherever in the run it sits.
    #[test]
    fn lanes_eq_catches_every_lane_position() {
        const N: usize = 16;
        let a: [u32; N] = core::array::from_fn(lane_value);
        for pos in 0..N {
            let mut b = a;
            b[pos] = b[pos].wrapping_add(1);
            // SAFETY: both runs are N live lanes of a local array.
            let got = unsafe { u32_lanes_eq::<N>(a.as_ptr(), b.as_ptr()) };
            assert_eq!(got, a == b, "lane {pos}");
            assert!(!got);
        }
    }

    /// Neither side has to be aligned, and the two may be offset differently.
    #[test]
    fn lanes_eq_reads_unaligned() {
        let lanes: [u32; 4] = [0xdead_beef, 0, 0x7f80_0000, 0xffff_ffff];
        for off_a in 0..4 {
            for off_b in 0..4 {
                let left = packed(&lanes, off_a);
                let mut right = packed(&lanes, off_b);
                assert!(eq4(&left, off_a, &right, off_b), "{off_a}/{off_b}");
                right[off_b + 9] ^= 0x20;
                assert!(!eq4(&left, off_a, &right, off_b), "{off_a}/{off_b}");
            }
        }
    }

    /// The same run on both sides compares equal; nothing here forbids overlap.
    #[test]
    fn lanes_eq_accepts_the_same_run_twice() {
        let a: [u32; 9] = core::array::from_fn(lane_value);
        // SAFETY: one live run of nine lanes, handed over as both operands.
        assert!(unsafe { u32_lanes_eq::<9>(a.as_ptr(), a.as_ptr()) });
    }

    /// A zero-lane compare is vacuously equal and reads nothing.
    #[test]
    fn lanes_eq_reads_nothing_at_zero_width() {
        // SAFETY: N is 0, so neither pointer is dereferenced.
        assert!(unsafe { u32_lanes_eq::<0>(core::ptr::dangling(), core::ptr::dangling()) });
    }

    /// Head-and-tail compare answers what a slice compare answers, both widths.
    #[test]
    fn short_bytes_eq_matches_slice_compare() {
        for want in [b"unit".as_slice(), b"event", b"OnKey", b"self"] {
            let head = u32::from_le_bytes([want[0], want[1], want[2], want[3]]);
            let tail = if want.len() == 5 { want[4] } else { 0 };
            for off in 0..4 {
                let mut buf = vec![0_u8; off + want.len() + 8];
                buf[off..off + want.len()].copy_from_slice(want);
                let run = &buf[off..off + want.len()];
                // SAFETY: `want.len()` bytes were written at `off`, so the
                // offset is inside the buffer.
                let at = unsafe { buf.as_ptr().add(off) };
                // SAFETY: the run at `at` holds `want.len()` bytes, which is
                // its true length and is 4 or 5.
                let got = unsafe { short_bytes_eq(at, want.len(), head, tail) };
                assert_eq!(got, run == want, "{off}");
                assert!(got);
            }
        }
    }

    /// A single differing byte is caught wherever in the run it sits.
    #[test]
    fn short_bytes_eq_catches_every_byte_position() {
        for want in [b"unit".as_slice(), b"event"] {
            let head = u32::from_le_bytes([want[0], want[1], want[2], want[3]]);
            let tail = if want.len() == 5 { want[4] } else { 0 };
            for pos in 0..want.len() {
                let mut have = want.to_vec();
                have[pos] ^= 0x20;
                // SAFETY: `have` holds exactly `want.len()` bytes, which is
                // the run's true length and is 4 or 5.
                let got = unsafe { short_bytes_eq(have.as_ptr(), have.len(), head, tail) };
                assert_eq!(got, have.as_slice() == want, "{pos}");
                assert!(!got);
            }
        }
    }

    /// A four-byte run never reads the fifth byte the literal would have had.
    #[test]
    fn short_bytes_eq_leaves_the_fifth_byte_alone_at_len_four() {
        let run = *b"unit";
        let head = u32::from_le_bytes(run);
        // SAFETY: `run` is exactly the four bytes the length names, so no
        // fifth byte is read whatever `tail` says.
        assert!(unsafe { short_bytes_eq(run.as_ptr(), 4, head, 0xff) });
    }

    /// The predicated fill writes exactly what a `skip..ROWS` loop wrote.
    #[test]
    fn zero_rows_from_matches_the_range_loop() {
        const ROWS: usize = 4;
        const LANES: usize = 4;
        for skip in 0..=ROWS + 1 {
            let mut got = [0xffff_ffff_u32; ROWS * LANES];
            let mut want = got;
            for row in skip..ROWS {
                for lane in 0..LANES {
                    want[row * LANES + lane] = 0;
                }
            }
            // SAFETY: `got` is ROWS * LANES live, writable lanes.
            unsafe { zero_rows_from::<ROWS, LANES>(got.as_mut_ptr(), skip) };
            assert_eq!(got, want, "skip {skip}");
        }
    }

    /// The fill stays inside its grid: neither guard lane is touched.
    #[test]
    fn zero_rows_from_writes_no_further_than_the_grid() {
        const ROWS: usize = 3;
        const LANES: usize = 4;
        let mut buf = [0xa5a5_a5a5_u32; ROWS * LANES + 2];
        // SAFETY: the grid occupies the first ROWS * LANES lanes of `buf`.
        unsafe { zero_rows_from::<ROWS, LANES>(buf.as_mut_ptr(), 0) };
        assert!(buf[..ROWS * LANES].iter().all(|&v| v == 0));
        assert_eq!(&buf[ROWS * LANES..], &[0xa5a5_a5a5, 0xa5a5_a5a5]);
    }

    /// The destination does not have to be aligned.
    #[test]
    fn zero_rows_from_writes_unaligned() {
        const ROWS: usize = 2;
        const LANES: usize = 4;
        for off in 0..4 {
            let mut buf = vec![0xff_u8; off + ROWS * LANES * 4];
            // SAFETY: the buffer is sized to hold the grid at `off`, so the
            // offset is inside it.
            let grid = unsafe { buf.as_mut_ptr().add(off) };
            // SAFETY: ROWS * LANES lanes are writable from `grid`, unaligned.
            unsafe { zero_rows_from::<ROWS, LANES>(grid.cast(), 1) };
            assert!(buf[..off + LANES * 4].iter().all(|&v| v == 0xff), "{off}");
            assert!(buf[off + LANES * 4..].iter().all(|&v| v == 0), "{off}");
        }
    }
}
