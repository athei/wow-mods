//! Portable sweep kernel for the Lua garbage-collector fast path.
//!
//! Lua 5.0's sweep phase walks every GC object list (userdata, every
//! string-table bucket, the main `rootgc` list) one linked node at a time and
//! pays a four-frame call chain per dead object (`sweeplist` → `freeobj` →
//! `luaM_irealloc` → `SmallBlockPool__Realloc`). The walk is latency-bound
//! pointer chasing over the whole heap, and the live gauge shows it dominating
//! collect pauses as the heap grows. This kernel reproduces the stock
//! `sweeplist` walk exactly — same dead test, same unlink order, same
//! survivor mark-clearing — with the next node prefetched while the current
//! one is processed, and leaves the per-object free action to a caller
//! callback so the 32-bit adapter can dispatch cheap frees directly to the
//! client's pool allocator. No FFI here; the adapter lives in `win::hooks`.

/// The common header every Lua 5.0 GC object starts with.
///
/// On the i686 target this matches the client layout exactly: list link at
/// `+0`, type tag at `+4`, mark byte at `+5`. Host-side tests instantiate the
/// same structure with native pointer width; the algorithm is identical.
#[repr(C)]
pub struct GcHeader {
    /// Next object in whichever GC list this object is linked on.
    pub next: *mut Self,
    /// Lua type tag (`LUA_T*`).
    pub tt: u8,
    /// Mark byte: bit 0 = reachable, bit 1 = finalizer flag, bit 4 = fixed.
    pub marked: u8,
}

/// The weak-key/weak-value mode bits `traversetable` parks in the mark byte.
///
/// Excluded from the dead test per the lua-5.0.3 `sweeplist` bugfix (see
/// [`sweep_list`]); kept intact on survivors, exactly like upstream.
pub const WEAK_MODE_BITS: u8 = 0x06;

/// Sweep one GC object list, returning the number of objects freed.
///
/// Reproduces stock `sweeplist` (`0x6f7210`) semantics — an object whose mark
/// byte tests `<= deadmask` is unlinked from the list and handed to `free`; a
/// survivor gets bit 0 of its mark byte cleared for the next cycle — with one
/// deliberate upstream bugfix: the client's Lua is 5.0.2, whose dead test
/// compares the WHOLE mark byte, so a table that was ever traversed while
/// weak keeps its stale [`WEAK_MODE_BITS`] and can never be swept — every
/// once-weak table leaks as an unsweepable zombie until client exit. Lua
/// 5.0.3 fixed this by masking the weak bits out of the dead test
/// (`(marked & ~(KEYWEAK | VALUEWEAK)) > limit`), and this kernel applies the
/// same mask. The stock deadmask is `0` for a normal collect (only
/// fully-unmarked objects die; the `0x10` fixed bit keeps an object alive)
/// and `0x100` for the free-everything close path. The next node's header is
/// prefetched before the current object is processed so the free work
/// overlaps the chase.
///
/// # Safety
///
/// `anchor` must point at the head link of a well-formed, null-terminated
/// GC list whose nodes all start with [`GcHeader`]. `free` must release the
/// object without touching the list links the kernel still holds (the node is
/// unlinked before `free` runs, matching stock order).
pub unsafe fn sweep_list(
    anchor: *mut *mut GcHeader,
    deadmask: u32,
    free: &mut impl FnMut(*mut GcHeader),
) -> u32 {
    let mut slot = anchor;
    let mut freed = 0u32;
    // SAFETY: caller guarantees `anchor` is a valid list-head link.
    let mut cur = unsafe { *slot };
    while !cur.is_null() {
        // SAFETY: `cur` is a live node of the caller's list.
        let next = unsafe { (*cur).next };
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        {
            #[cfg(target_arch = "x86")]
            use core::arch::x86::{_MM_HINT_T0, _mm_prefetch};
            #[cfg(target_arch = "x86_64")]
            use core::arch::x86_64::{_MM_HINT_T0, _mm_prefetch};
            // SAFETY: a prefetch performs no architectural access, and an
            // operand that is null or untranslatable is a no-op that cannot
            // fault or set flags, so the last node's null `next` needs no
            // guard; the compare and branch that guarded it leave the walk.
            unsafe { _mm_prefetch(next.cast::<i8>(), _MM_HINT_T0) };
        }
        // SAFETY: `cur` is a live node; the mark byte is at header offset 5.
        let marked = unsafe { (*cur).marked };
        if u32::from(marked & !WEAK_MODE_BITS) <= deadmask {
            // SAFETY: `slot` is the link that points at `cur`; unlink first,
            // free second — stock order.
            unsafe { *slot = next };
            free(cur);
            freed += 1;
        } else {
            // SAFETY: survivor: clear the reachable bit for the next cycle.
            unsafe { (*cur).marked = marked & 0xfe };
            // SAFETY: the node's `next` field is the new predecessor link.
            slot = unsafe { &raw mut (*cur).next };
        }
        cur = next;
    }
    freed
}

/// One `SmallBlockPool` chunk span: `[base, end)` plus the chunk descriptor.
///
/// Built by the sweep adapter from the client's six pool classes; sorted by
/// `base`. Frees resolve their owning chunk by binary search instead of the
/// stock allocator's linear scan over every chunk of every class — the scan
/// that makes stock frees O(total chunks) and dominates sweep pauses on
/// large heaps.
#[derive(Clone, Copy)]
pub struct ChunkSpan {
    /// First byte of the chunk's payload.
    pub base: usize,
    /// One past the last payload byte.
    pub end: usize,
    /// The chunk descriptor (freelist head at `+0x4`, free count at `+0x10`).
    pub chunk: usize,
    /// The size class (0..6) this chunk belongs to.
    ///
    /// Read only by the 32-bit realloc path, so a host test build sees no
    /// reader.
    // Read only on the 32-bit realloc path, so a host test build has no reader.
    #[cfg_attr(not(target_arch = "x86"), allow(dead_code))]
    pub class: u8,
}

/// Find the chunk span owning `ptr`, if any.
///
/// `spans` must be sorted by `base` and non-overlapping (they are distinct
/// allocations). Returns the descriptor address, or `None` for pointers no
/// pool chunk owns (oversize blocks the stock path routes to `SMemFree`).
#[must_use]
pub fn chunk_owning(spans: &[ChunkSpan], ptr: usize) -> Option<usize> {
    chunk_owning_span(spans, ptr).map(|s| s.chunk)
}

/// Find the full span owning `ptr` (the realloc path also needs the class).
#[must_use]
pub fn chunk_owning_span(spans: &[ChunkSpan], ptr: usize) -> Option<&ChunkSpan> {
    let i = spans.partition_point(|s| s.base <= ptr);
    if i == 0 {
        return None;
    }
    let s = &spans[i - 1];
    (ptr < s.end).then_some(s)
}

/// Push a freed block onto its chunk's freelist, atomically.
///
/// The same state transition as the stock free (`*block = freehead;
/// freehead = block; freecount += 1`), implemented with a CAS loop on the
/// freelist head so concurrent sweep workers can free into the same chunk.
/// No allocation runs during a sweep, so the head is only contended by
/// other frees; uncontended, the CAS costs what the plain store did.
///
/// # Safety
///
/// `chunk` must be a live chunk descriptor (freelist head at `+0x4`, free
/// count at `+0x10`) owning `block`, and `block` must be dead — its first
/// word becomes the freelist link.
// Guest addresses are 32-bit, so the block pointer narrows to the link word.
//
// Reached only from the 32-bit hook path and from tests gated on a 32-bit
// pointer width (the freelist links are guest-width words), so a 64-bit host
// test build compiles it with no caller.
#[cfg_attr(not(target_pointer_width = "32"), allow(dead_code))]
pub unsafe fn chunk_free_push(chunk: usize, block: usize) {
    use core::sync::atomic::{AtomicU32, Ordering};
    // SAFETY: caller guarantees a live chunk descriptor; u32 fields aligned.
    let head = unsafe { AtomicU32::from_ptr((chunk + 0x4) as *mut u32) };
    // SAFETY: see above.
    let count = unsafe { AtomicU32::from_ptr((chunk + 0x10) as *mut u32) };
    let mut h = head.load(Ordering::Relaxed);
    loop {
        // SAFETY: `block` is dead; its first word is ours to use as a link.
        unsafe { *(block as *mut u32) = h };
        match head.compare_exchange_weak(h, block as u32, Ordering::Release, Ordering::Relaxed) {
            Ok(_) => break,
            Err(cur) => h = cur,
        }
    }
    count.fetch_add(1, Ordering::Relaxed);
}

#[cfg(test)]
mod tests_lua_gc {
    use super::{GcHeader, sweep_list};

    /// A fake GC object: the header plus a payload marker the tests check.
    #[repr(C)]
    struct FakeObj {
        hdr: GcHeader,
        id: u32,
    }

    // `Box` per node, not a flat `Vec<FakeObj>`: the list threads raw `next`
    // pointers between nodes, so each needs an address independent of the
    // vector's buffer.
    #[allow(clippy::vec_box)]
    fn build_list(marks: &[u8]) -> Vec<Box<FakeObj>> {
        let mut objs: Vec<Box<FakeObj>> = marks
            .iter()
            .enumerate()
            .map(|(i, &m)| {
                Box::new(FakeObj {
                    hdr: GcHeader {
                        next: core::ptr::null_mut(),
                        tt: 4,
                        marked: m,
                    },
                    id: u32::try_from(i).unwrap(),
                })
            })
            .collect();
        for i in (0..objs.len().saturating_sub(1)).rev() {
            let next: *mut FakeObj = &raw mut *objs[i + 1];
            objs[i].hdr.next = next.cast::<GcHeader>();
        }
        objs
    }

    fn collect_list(head: *mut GcHeader) -> Vec<u32> {
        let mut ids = Vec::new();
        let mut cur = head;
        while !cur.is_null() {
            let obj = cur.cast::<FakeObj>();
            // SAFETY: `cur` is non-null and `build_list` only links `FakeObj`s.
            ids.push(unsafe { (*obj).id });
            // SAFETY: `GcHeader` is `FakeObj`'s first field, so `cur` points at one.
            cur = unsafe { (*cur).next };
        }
        ids
    }

    #[test]
    fn frees_unmarked_keeps_marked_and_clears_bit0() {
        let mut objs = build_list(&[0x01, 0x00, 0x11, 0x00, 0x01]);
        let mut head: *mut GcHeader = (&raw mut *objs[0]).cast();
        let mut freed_ids = Vec::new();
        let mut record = |o: *mut GcHeader| {
            // SAFETY: the sweep hands back a live `FakeObj` before freeing it.
            freed_ids.push(unsafe { (*o.cast::<FakeObj>()).id });
        };
        // SAFETY: `head` anchors the list `build_list` just built, and `objs`
        // keeps every node alive for the duration of the call.
        let freed = unsafe { sweep_list(&raw mut head, 0, &mut record) };
        assert_eq!(freed, 2);
        assert_eq!(freed_ids, vec![1, 3]);
        assert_eq!(collect_list(head), vec![0, 2, 4]);
        // Survivors: reachable bit cleared, fixed bit kept.
        assert_eq!(objs[0].hdr.marked, 0x00);
        assert_eq!(objs[2].hdr.marked, 0x10);
        assert_eq!(objs[4].hdr.marked, 0x00);
    }

    #[test]
    fn frees_head_run_and_relinks_anchor() {
        let mut objs = build_list(&[0x00, 0x00, 0x01]);
        let mut head: *mut GcHeader = (&raw mut *objs[0]).cast();
        // SAFETY: `head` anchors the list `build_list` just built, and `objs`
        // keeps every node alive for the duration of the call.
        let freed = unsafe { sweep_list(&raw mut head, 0, &mut |_| {}) };
        assert_eq!(freed, 2);
        assert_eq!(collect_list(head), vec![2]);
    }

    #[test]
    fn deadmask_all_frees_everything_including_fixed() {
        let mut objs = build_list(&[0x01, 0x10, 0x11]);
        let mut head: *mut GcHeader = (&raw mut *objs[0]).cast();
        // SAFETY: `head` anchors the list `build_list` just built, and `objs`
        // keeps every node alive for the duration of the call.
        let freed = unsafe { sweep_list(&raw mut head, 0x100, &mut |_| {}) };
        assert_eq!(freed, 3);
        assert!(head.is_null());
    }

    #[test]
    fn zombie_weak_table_is_freed_but_live_weak_survives() {
        // marked 0x04 = dead table with a stale weak-value bit (the 5.0.2
        // zombie); 0x05 = the same table while reachable. Upstream 5.0.3
        // frees the former, keeps the latter with its weak bit intact.
        let mut objs = build_list(&[0x04, 0x05, 0x06]);
        let mut head: *mut GcHeader = (&raw mut *objs[0]).cast();
        let mut freed_ids = Vec::new();
        let mut record = |o: *mut GcHeader| {
            // SAFETY: the sweep hands back a live `FakeObj` before freeing it.
            freed_ids.push(unsafe { (*o.cast::<FakeObj>()).id });
        };
        // SAFETY: `head` anchors the list `build_list` just built, and `objs`
        // keeps every node alive for the duration of the call.
        let freed = unsafe { sweep_list(&raw mut head, 0, &mut record) };
        assert_eq!(freed, 2);
        assert_eq!(freed_ids, vec![0, 2]);
        assert_eq!(collect_list(head), vec![1]);
        assert_eq!(objs[1].hdr.marked, 0x04);
    }

    // The freelist push writes GUEST-WIDTH (32-bit) link words, so these two
    // tests only make sense where host pointers fit them; on a 64-bit test
    // host the links would truncate real addresses.
    #[cfg(target_pointer_width = "32")]
    #[test]
    fn chunk_free_push_builds_a_freelist() {
        use super::chunk_free_push;
        // Fake chunk descriptor: [base, freehead, size, pad, freecount].
        let mut chunk = [0u32; 5];
        let mut blocks = [[0u32; 4]; 3];
        let chunk_addr = chunk.as_mut_ptr() as usize;
        for b in &mut blocks {
            let block = b.as_mut_ptr() as usize;
            // SAFETY: `chunk_addr` is the five-word descriptor above and `block`
            // a four-word slot, both live stack arrays for the whole loop; the
            // push writes only the descriptor's head/count and the block's first
            // word, all of which are in bounds.
            unsafe { chunk_free_push(chunk_addr, block) };
        }
        assert_eq!(chunk[4], 3);
        // Head is the last-pushed block; links walk back in push order.
        assert_eq!(chunk[1] as usize, blocks[2].as_ptr() as usize);
        assert_eq!(blocks[2][0] as usize, blocks[1].as_ptr() as usize);
        assert_eq!(blocks[1][0] as usize, blocks[0].as_ptr() as usize);
        assert_eq!(blocks[0][0], 0);
    }

    #[cfg(target_pointer_width = "32")]
    #[test]
    fn chunk_free_push_survives_concurrent_pushers() {
        use super::chunk_free_push;
        let mut chunk = [0u32; 5];
        let chunk_addr = chunk.as_mut_ptr() as usize;
        let mut blocks = vec![[0u32; 4]; 4000];
        let addrs: Vec<usize> = blocks.iter_mut().map(|b| b.as_mut_ptr() as usize).collect();
        std::thread::scope(|s| {
            for part in addrs.chunks(1000) {
                s.spawn(move || {
                    for &b in part {
                        // SAFETY: as above, and the descriptor outlives the
                        // scope every thread is joined within. Concurrent pushes
                        // are what this test exercises: the head word is updated
                        // by compare-exchange, so overlapping callers are the
                        // supported case rather than a violated precondition.
                        unsafe { chunk_free_push(chunk_addr, b) };
                    }
                });
            }
        });
        assert_eq!(chunk[4], 4000);
        // Every pushed block appears exactly once on the list.
        let mut seen = 0u32;
        let mut cur = chunk[1] as usize;
        while cur != 0 {
            seen += 1;
            // SAFETY: `cur` is a link word the pushes above wrote, so it is
            // either zero (loop ends) or the address of one of the `blocks`
            // slots, which are still alive here; its first word is the next link.
            cur = unsafe { *(cur as *const u32) } as usize;
        }
        assert_eq!(seen, 4000);
    }

    #[test]
    fn chunk_owning_finds_only_in_bounds() {
        use super::{ChunkSpan, chunk_owning};
        let spans = [
            ChunkSpan {
                base: 0x1000,
                end: 0x2000,
                chunk: 1,
                class: 0,
            },
            ChunkSpan {
                base: 0x3000,
                end: 0x3800,
                chunk: 2,
                class: 1,
            },
            ChunkSpan {
                base: 0x8000,
                end: 0x9000,
                chunk: 3,
                class: 5,
            },
        ];
        assert_eq!(chunk_owning(&spans, 0x1000), Some(1));
        assert_eq!(chunk_owning(&spans, 0x1fff), Some(1));
        assert_eq!(chunk_owning(&spans, 0x2000), None);
        assert_eq!(chunk_owning(&spans, 0x2fff), None);
        assert_eq!(chunk_owning(&spans, 0x3400), Some(2));
        assert_eq!(chunk_owning(&spans, 0x8fff), Some(3));
        assert_eq!(chunk_owning(&spans, 0x9000), None);
        assert_eq!(chunk_owning(&spans, 0xfff), None);
        assert_eq!(chunk_owning(&[], 0x1000), None);
    }

    #[test]
    fn empty_list_is_a_noop() {
        let mut head: *mut GcHeader = core::ptr::null_mut();
        // SAFETY: `head` is a valid anchor holding the empty (null) list.
        let freed = unsafe { sweep_list(&raw mut head, 0, &mut |_| {}) };
        assert_eq!(freed, 0);
        assert!(head.is_null());
    }
}
