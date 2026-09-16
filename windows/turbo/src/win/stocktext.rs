//! Overflow ownership and stable placement for stock floating combat text.
//!
//! The client's owner stores four text pointers at `+0x20`. Additional banks
//! temporarily occupy that same window during native calls, then the original
//! four pointers are restored. Animation, fading, and lifetime use the native
//! routines without extending the client's allocation. Collision displacement
//! persists until the line is released, so dense text keeps its placement.

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::{cell::RefCell, collections::BTreeMap, sync::Mutex};

use crate::stocktext::{Banks, Placement};

/// Native text updates and their nested placement calls share thread-local state.
#[derive(Default)]
struct Placements {
    lines: BTreeMap<usize, Placement>,
    active: Option<Placement>,
}

thread_local! {
    /// Line displacements are removed by the same native release used for expiry.
    static PLACEMENTS: RefCell<Placements> = RefCell::new(Placements::default());
}

/// Each extension is enabled only while its required hooks are installed.
static ENABLED: AtomicU8 = AtomicU8::new(0);
/// Every overflow ownership hook is active.
const OVERFLOW: u8 = 1;
/// Line update, placement, and release hooks are active.
const PLACEMENT: u8 = 2;
/// Game-thread registry; no lock is held while calling the client.
static OWNERS: Mutex<BTreeMap<usize, Banks>> = Mutex::new(BTreeMap::new());
/// Avoid the registry lock on frames with no overflow anywhere.
static HAS_OVERFLOW: AtomicBool = AtomicBool::new(false);

/// Enable each extension after its required hooks were installed here.
pub fn initialize() {
    let Some((_, own_base)) = wow_hook::module_of(initialize as *const () as usize) else {
        return;
    };
    let installed_here = |va| {
        wow_hook::detour_target(va)
            .and_then(wow_hook::detour_endpoint)
            .and_then(wow_hook::module_of)
            .is_some_and(|(_, base)| base == own_base)
    };
    let placement_installed = [0x006c_7cc0, 0x0050_9520, 0x006c_86a0]
        .into_iter()
        .all(installed_here);
    let installed = [0x006c_6d40, 0x006c_6e00, 0x006c_6cd0]
        .into_iter()
        .all(installed_here);
    // SAFETY: this fixed entry is in the mapped client image; it releases one
    // text through its destructor and returns its storage to the native pool.
    let release_matches = installed_here(0x006c_86a0)
        || unsafe {
            wow_hook::signature_matches(0x006c_86a0, "56 8B F1 8B 06 6A 00 FF 10 6A 00 6A 00 56")
        };
    ENABLED.store(
        (u8::from(installed && release_matches) * OVERFLOW)
            | (u8::from(placement_installed) * PLACEMENT),
        Ordering::Relaxed,
    );
    if !installed || !release_matches {
        crate::defer_log!(target: super::LOG_TARGET, log::Level::Warn, "stock text overflow disabled: lifecycle hooks unavailable");
    }
}

/// Carry one line's placement through its native animation and projection.
pub fn update_line(this: *mut u8, now: u32) -> u8 {
    let original = super::symbols::originals::world_text__update__6c7cc0();
    let enabled = ENABLED.load(Ordering::Relaxed) & PLACEMENT != 0;
    if !enabled {
        return original(this, now);
    }
    let saved = PLACEMENTS.with_borrow_mut(|state| {
        // Keep the map entry allocated during the native call. No borrow or
        // lock crosses it, and a nested line update saves the outer context.
        let placement = core::mem::take(state.lines.entry(this.addr()).or_default());
        state.active.replace(placement)
    });
    let alive = original(this, now);
    PLACEMENTS.with_borrow_mut(|state| {
        let placement = core::mem::replace(&mut state.active, saved);
        if alive == 0 {
            state.lines.remove(&this.addr());
        } else if let Some(placement) = placement {
            state.lines.insert(this.addr(), placement);
        }
    });
    alive
}

/// Seed stock text collision searches with the previous frame's displacement.
pub fn place(list: i32, rect: *mut f32) {
    let original = super::symbols::originals::floating_text_place_rect__509520();
    if list != 1 {
        original(list, rect);
        return;
    }
    let placement = PLACEMENTS.with_borrow_mut(|state| state.active.take());
    let Some(mut placement) = placement else {
        original(list, rect);
        return;
    };
    // SAFETY: the native placement entry receives one writable four-float rect.
    let projected = unsafe { rect.cast::<[f32; 4]>().read_unaligned() };
    let mut seeded = projected;
    placement.seed(&mut seeded);
    let fits = placement_fits(&placement, &mut seeded);
    // SAFETY: the same caller-owned rect remains writable throughout this call.
    unsafe { rect.cast::<[f32; 4]>().write_unaligned(seeded) };
    if fits {
        let append: extern "fastcall" fn(i32, *const f32) =
            // SAFETY: the native placement path appends its four-float result
            // through this fixed fastcall entry with the same list index.
            unsafe { core::mem::transmute(0x0050_9660usize) };
        append(list, rect);
    } else {
        original(list, rect);
    }
    // SAFETY: the native search has written the four-float result in place.
    let placed = unsafe { rect.cast::<[f32; 4]>().read_unaligned() };
    placement.remember(projected, placed);
    PLACEMENTS.with_borrow_mut(|state| state.active = Some(placement));
}

/// Read the native text obstacle list without keeping a borrow across a call.
fn placement_fits(placement: &Placement, rect: &mut [f32; 4]) -> bool {
    let slide: extern "thiscall" fn(*mut f32) =
        // SAFETY: this native entry slides one four-float rectangle onto the
        // screen. The stock search calls it before testing any obstacles.
        unsafe { core::mem::transmute(0x0050_9e20usize) };
    slide(rect.as_mut_ptr());
    // SAFETY: the client stores its two initialized UI screen extents here.
    let screen = unsafe { (0x0083_2a44 as *const [f32; 2]).read_unaligned() };
    // SAFETY: list 1's count is a live dword in the native placement table.
    let count = unsafe { (0x00be_0ba0 as *const u32).read() };
    // SAFETY: the adjacent dword points to that list's contiguous rectangle array.
    let data = unsafe { (0x00be_0ba4 as *const *const [f32; 4]).read() };
    let obstacles = if count == 0 {
        &[]
    } else {
        // SAFETY: the native list owns `count` initialized four-float rectangles;
        // only the game thread mutates it, outside this call.
        unsafe { core::slice::from_raw_parts(data, count as usize) }
    };
    placement.fits(*rect, screen, obstacles)
}

/// Forget a released line before its pool address can be reused.
pub fn release_line(this: *mut u8) {
    PLACEMENTS.with_borrow_mut(|state| state.lines.remove(&this.addr()));
}

/// Read the owner's four pointers without borrowing them across a native call.
const fn slots(this: *mut u8) -> [usize; 4] {
    // SAFETY: every caller receives the live owner from a verified native
    // entry. Its four pointer slots occupy bytes 0x20 through 0x2f on i686.
    unsafe {
        this.wrapping_add(0x20)
            .cast::<[usize; 4]>()
            .read_unaligned()
    }
}

/// Run one native operation with a bank installed, restoring the real slots.
fn with_slots(this: *mut u8, bank: [usize; 4], call: impl FnOnce()) -> [usize; 4] {
    let saved = slots(this);
    // SAFETY: the live owner's complete four-pointer window is writable.
    unsafe {
        this.wrapping_add(0x20)
            .cast::<[usize; 4]>()
            .write_unaligned(bank);
    }
    call();
    let result = slots(this);
    // SAFETY: the native create/update/draw calls do not free the owner.
    unsafe {
        this.wrapping_add(0x20)
            .cast::<[usize; 4]>()
            .write_unaligned(saved);
    }
    result
}

/// Move an owner's banks out of the registry while native code visits them.
fn with_banks(this: *mut u8, call: impl FnOnce(&mut Banks)) {
    let mut banks = {
        let mut owners = OWNERS.lock().expect("text owners are game-thread only");
        let banks = owners.remove(&this.addr()).unwrap_or_default();
        HAS_OVERFLOW.store(!owners.is_empty(), Ordering::Relaxed);
        banks
    };
    call(&mut banks);
    if !banks.is_empty() {
        let mut owners = OWNERS.lock().expect("text owners are game-thread only");
        match owners.entry(this.addr()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                // Return the vector itself so steady visits reuse its storage.
                entry.insert(banks);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                // A nested creation may have returned banks for this owner.
                entry.get_mut().append(&mut banks);
            }
        }
        HAS_OVERFLOW.store(true, Ordering::Relaxed);
        drop(owners);
    }
}

/// Create through an extra bank when the native window is full.
pub fn create(this: *mut u8, kind: i32, text: *const u8, color: u32, flag: u32) -> bool {
    if ENABLED.load(Ordering::Relaxed) & OVERFLOW == 0 || slots(this).contains(&0) {
        return false;
    }
    with_banks(this, |banks| {
        banks.create(|bank| {
            with_slots(this, bank, || {
                (super::symbols::originals::create_world_text__6c73f0())(
                    this, kind, text, color, flag,
                );
            })
        });
    });
    true
}

/// Tick overflow through the native update, including expiry and font setup.
pub fn update(this: *mut u8, now: u32) {
    if !HAS_OVERFLOW.load(Ordering::Relaxed) {
        return;
    }
    with_banks(this, |banks| {
        banks.visit(|bank| {
            with_slots(this, bank, || {
                (super::symbols::originals::world_text_owner__update__6c6d40())(this, now);
            })
        });
    });
}

/// Submit the native auxiliary draws for every overflow bank.
pub fn draw(this: *mut u8) {
    if !HAS_OVERFLOW.load(Ordering::Relaxed) {
        return;
    }
    with_banks(this, |banks| {
        banks.visit(|bank| {
            with_slots(this, bank, || {
                (super::symbols::originals::world_text_owner__draw__6c6e00())(this);
            })
        });
    });
}

/// Release overflow before the client destroys the owner and its four slots.
pub fn destroy(this: *mut u8) {
    if !HAS_OVERFLOW.load(Ordering::Relaxed) {
        return;
    }
    let banks = {
        let mut owners = OWNERS.lock().expect("text owners are game-thread only");
        let banks = owners.remove(&this.addr());
        HAS_OVERFLOW.store(!owners.is_empty(), Ordering::Relaxed);
        banks
    };
    if let Some(banks) = banks {
        let release: extern "thiscall" fn(usize) =
            // SAFETY: initialize verified the fixed release entry before any
            // overflow was created. It takes one live native text pointer.
            unsafe { core::mem::transmute(0x006c_86a0usize) };
        for line in banks.into_lines() {
            release(line);
        }
    }
}
