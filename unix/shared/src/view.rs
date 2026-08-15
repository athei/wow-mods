//! Read-at-use views over a float block the caller already holds.
//!
//! Another boundary newtype, and the one the set in [`crate::ffi_boundary`]
//! cannot hold, because every type there hands out a reference and this one
//! must not. A seam adapter has a pointer and a kernel that wants
//! `&[f32; N]`, and the idiom that bridges them copies:
//! `&unsafe { p.cast::<[f32; N]>().read_unaligned() }` materialises the whole
//! block in the frame before the kernel's first branch, so a kernel that
//! rejects on its first compare has still paid for every lane. [`F32s`] hands
//! over the address instead and reads each lane where it is used.
//!
//! Two properties are load-bearing, and neither is incidental.
//!
//! **The pointer stays raw.** `unsafe { &*p.cast::<[f32; N]>() }` is the
//! shorter spelling and the wrong one here: a shared reference carries
//! `noalias`, `dereferenceable` and an alignment obligation, and seam inputs
//! legitimately overlap: an in-place adapter passes the same pointer as input
//! and output, and callers pass `a == b`. A raw pointer asserts none of that,
//! so a read is never reordered past a store the optimizer cannot
//! disambiguate, which is the conservative direction.
//!
//! **Every lane is read unaligned.** Float records in the client are not all
//! 4-aligned; a packed vertex stream or a file-blob offset can put one at 2
//! mod 4. That is the same precondition `read_unaligned` already carried at
//! the sites this replaces, and it is why the view cannot be a reference even
//! where aliasing is not a question.
//!
//! What the type does **not** decide is whether a given copy may be elided at
//! all. A snapshot freezes the bytes at seam entry; a view reads them at the
//! use site, so the copy is load-bearing wherever anything can write that
//! region in between: the kernel writing through a second pointer into the
//! same record, a plane that points into the polygon being clipped, a stock
//! callee reached from inside the region. That judgement is per site and
//! belongs in the `// SAFETY:` comment there, not in this file.
//!
//! Read-only by construction. A record a kernel mutates in place is a
//! different question and goes through [`crate::InPtrMut`], which asserts the
//! exclusivity this one deliberately does not.
//!
//! It lives beside the other boundary types rather than in the mod that
//! consumes it because nothing in it is target-specific. The seam it is named
//! for is 32-bit, the type is not, so it is linted on both targets and its
//! tests run on the host.

use core::marker::PhantomData;

/// A borrowed run of `N` `f32`, read one lane at a time.
///
/// One pointer wide and `Copy`, so passing it to a kernel costs a register and
/// holding it across a call costs what holding the pointer costs. The lifetime
/// is carried in a [`PhantomData`] rather than by the pointer's type, which is
/// what keeps the borrow checked at compile time while leaving the pointer raw
/// in the generated code.
#[derive(Clone, Copy)]
pub struct F32s<'a, const N: usize>(*const f32, PhantomData<&'a f32>);

impl<const N: usize> F32s<'_, N> {
    /// Wrap a run of `N` `f32` the caller owns.
    ///
    /// # Safety
    /// Caller asserts `N` `f32` are readable at `p` for as long as the view is
    /// used, and that nothing writes that region while it is. The second half
    /// is the one that is not mechanical: a lane is read at its use rather
    /// than at construction, so a write in between changes what the kernel
    /// sees, where a copy would have frozen it. Alignment is not asserted:
    /// every read below is unaligned.
    #[must_use]
    pub const unsafe fn new(p: *const f32) -> Self {
        Self(p, PhantomData)
    }

    /// Lane `I`, with the index checked at compile time.
    ///
    /// The form for a kernel that indexes with literals, which is most of
    /// them: there is no bound to test at run time and no panic edge, so the
    /// read is the whole body.
    #[must_use]
    pub const fn at<const I: usize>(self) -> f32 {
        const { assert!(I < N, "F32s lane index out of range") }
        // SAFETY: `I < N` by the assertion above, and construction asserted N
        // lanes are readable from the base.
        let lane = unsafe { self.0.add(I) };
        // SAFETY: `lane` is in bounds by the assertion above, and an unaligned
        // read carries no alignment requirement of its own.
        unsafe { lane.read_unaligned() }
    }

    /// Lane `i`, bounds-checked as indexing the array would have been.
    ///
    /// # Panics
    /// Panics if `i >= N`, which is the panic `[f32; N]` indexing already
    /// carried at the same site. A constant index folds the test away.
    #[must_use]
    pub const fn get(self, i: usize) -> f32 {
        assert!(i < N, "F32s lane index out of range");
        // SAFETY: `i < N` by the assertion above, and construction asserted N
        // lanes are readable from the base.
        let lane = unsafe { self.0.add(i) };
        // SAFETY: `lane` is in bounds by the assertion above, and an unaligned
        // read carries no alignment requirement of its own.
        unsafe { lane.read_unaligned() }
    }
}

impl<'a, const N: usize> From<&'a [f32; N]> for F32s<'a, N> {
    fn from(block: &'a [f32; N]) -> Self {
        // SAFETY: a live borrow of exactly N `f32` outliving the view it
        // yields, and the borrow forbids a write through any other path for
        // as long as it is held.
        unsafe { Self::new(block.as_ptr()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run of `N` floats at byte offset `off` in a fresh buffer.
    ///
    /// Offsets of 2 put the run at 2 mod 4, which is the alignment the client
    /// records are allowed to have and the reason the reads are unaligned.
    fn packed(lanes: &[f32], off: usize) -> Vec<u8> {
        let mut buf = vec![0_u8; off + lanes.len() * 4 + 8];
        for (i, v) in lanes.iter().enumerate() {
            buf[off + i * 4..off + i * 4 + 4].copy_from_slice(&v.to_bits().to_le_bytes());
        }
        buf
    }

    /// A view over an array reads back what the array holds.
    #[test]
    fn borrows_an_array_lane_for_lane() {
        let block: [f32; 6] = [1.5, -2.25, 0.0, f32::INFINITY, 3.75, -0.0];
        let view: F32s<'_, 6> = (&block).into();
        for (i, want) in block.iter().enumerate() {
            assert_eq!(view.get(i).to_bits(), want.to_bits(), "lane {i}");
        }
        assert_eq!(view.at::<0>().to_bits(), block[0].to_bits());
        assert_eq!(view.at::<5>().to_bits(), block[5].to_bits());
    }

    /// A NaN payload survives the read, because the read moves bits.
    #[test]
    fn carries_a_nan_payload_through_unchanged() {
        let block = [f32::from_bits(0x7fc0_dead)];
        let view: F32s<'_, 1> = (&block).into();
        assert_eq!(view.at::<0>().to_bits(), 0x7fc0_dead);
    }

    /// The base does not have to be 4-aligned.
    #[test]
    fn reads_a_run_at_two_mod_four() {
        let lanes = [9.5_f32, -1.25, 7.0, 0.125];
        for off in 0..4 {
            let buf = packed(&lanes, off);
            // SAFETY: four lanes were packed at `off`, so the offset is inside
            // the buffer.
            let base = unsafe { buf.as_ptr().add(off) };
            // SAFETY: four `f32` are readable from `base` and the buffer
            // outlives the view; nothing writes it while the view is live.
            let view: F32s<'_, 4> = unsafe { F32s::new(base.cast()) };
            for (i, want) in lanes.iter().enumerate() {
                assert_eq!(view.get(i).to_bits(), want.to_bits(), "{off}/{i}");
            }
        }
    }

    /// A lane is read at its use, not at construction.
    ///
    /// This is the whole semantic difference from the snapshot the view
    /// replaces, and the reason a copy is load-bearing wherever the region can
    /// be written while a kernel is reading it.
    #[test]
    fn reads_the_storage_at_the_use_site() {
        let mut buf = [0_u8; 8];
        let base: *mut u8 = (&raw mut buf).cast();
        // SAFETY: `buf` is eight live bytes, so one `f32` fits at offset 0.
        unsafe { base.cast::<f32>().write_unaligned(1.5) };
        // SAFETY: one `f32` is readable at `base` and `buf` outlives the view.
        let view: F32s<'_, 1> = unsafe { F32s::new(base.cast()) };
        assert_eq!(view.at::<0>().to_bits(), 1.5_f32.to_bits());
        // SAFETY: as the first write; the view holds no reference to elide it.
        unsafe { base.cast::<f32>().write_unaligned(-2.5) };
        assert_eq!(view.at::<0>().to_bits(), (-2.5_f32).to_bits());
    }

    /// A runtime index past the end panics where `[f32; N]` would have.
    #[test]
    #[should_panic(expected = "F32s lane index out of range")]
    fn runtime_index_past_the_end_panics() {
        let block = [1.0_f32, 2.0];
        let view: F32s<'_, 2> = (&block).into();
        let _ = view.get(2);
    }

    /// The view is one pointer wide, so it costs a register and no slot.
    #[test]
    fn is_one_pointer_wide() {
        assert_eq!(
            core::mem::size_of::<F32s<'_, 16>>(),
            core::mem::size_of::<*const f32>(),
        );
    }
}
