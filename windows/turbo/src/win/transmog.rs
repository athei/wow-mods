//! Host side of the visible-item write coalescer.
//!
//! Owns the shared [`crate::transmog::Coalescer`], the descriptor reads the
//! kernel's decisions are made against, and the refresh calls its flush
//! issues. The kernel answers "what should happen to this write"; everything
//! that touches the client's memory or calls into its code is here.
//!
//! Descriptor access goes through the raw field array at `+0x8` of the object
//! — the same indirection the descriptor-write entry itself takes — not the
//! parsed record `crate::win::objmgr` reads at `+0x110`. The two are different
//! structures, and only the raw array is indexed by the field numbers the
//! write entry receives.

use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};

use crate::transmog::{
    Coalescer, FLUSH_UNITS_CAP, INV_GUID_DWORDS, INV_GUID_FIRST, ITEM_DEFER_FIELD, InvWrite,
    PlayerWrite, VISIBLE_FIRST, VISIBLE_SLOTS, VISIBLE_STRIDE,
};

unsafe extern "system" {
    fn GetTickCount() -> u32;
}

/// Byte offset of the raw descriptor-field array pointer inside an object.
const DESCRIPTOR_PTR: usize = 0x8;

/// Byte offset of the first visible-item block in the descriptor array.
const VISIBLE_BYTES: usize = VISIBLE_FIRST as usize * 4;

/// Byte stride between visible-item blocks in the descriptor array.
const VISIBLE_STRIDE_BYTES: usize = VISIBLE_STRIDE as usize * 4;

/// Byte offset of the first equipped-item GUID in the descriptor array.
const EQUIP_GUID_BYTES: usize = (INV_GUID_FIRST as usize + 2 * 5) * 4;

/// Byte offset of the model handle a display refresh consumes.
const MODEL_HANDLE_FIELD: usize = 0xb34;

/// Fields the enter-world downgrade sets so the unit re-reads its own state.
const REFRESH_FLAG_FIELDS: [usize; 2] = [0xccc, 0xcd0];

/// The global whose `+0x10` holds the model handle a display refresh wants.
const MODEL_CONTEXT: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0080_de90;

/// `CGUnit_C::RefreshDisplayModel` — `thiscall(ecx = this)`, `RET 0`.
const REFRESH_DISPLAY_MODEL_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0020_abe0;

/// `CGUnit_C::RefreshDataPointers` — `thiscall(ecx = this)`, `RET 0`.
const REFRESH_DATA_POINTERS_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0020_afb0;

/// `CGInventoryAlert::ScanEquipmentStatus` — `cdecl`, no arguments.
const SCAN_EQUIPMENT_STATUS_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x000c_7ee0;

/// The scene-end entry, whose overwrite policy this module owns.
const SCENE_END_RVA: usize = 0x001a_17a0;

/// [`VISIBLE_SLOTS`] in the width the field indices are computed in.
const VISIBLE_SLOT_COUNT: u32 = VISIBLE_SLOTS as u32;

/// The descriptor field index of a visible-item block.
const fn visible_index(slot: usize) -> u32 {
    VISIBLE_FIRST + slot as u32 * VISIBLE_STRIDE
}

/// The coalescer, shared by the write entries and the scene-end flush.
///
/// The lock is held only across the kernel's pure decisions, never across a
/// call into the client — the flush drops it between each entry it drains and
/// the refresh that entry causes, so no thread ever waits on this while
/// holding something the client owns.
static STATE: LazyLock<Mutex<Coalescer>> = LazyLock::new(|| Mutex::new(Coalescer::new()));

/// The player object the mirrors were bound against, zero when unbound.
static BOUND_PLAYER: AtomicUsize = AtomicUsize::new(0);

static WRITES_SEEN: AtomicU32 = AtomicU32::new(0);
static SWALLOWED: AtomicU32 = AtomicU32::new(0);
static FLUSHED: AtomicU32 = AtomicU32::new(0);
static REFRESHED: AtomicU32 = AtomicU32::new(0);
static DEFERRED_ITEMS: AtomicU32 = AtomicU32::new(0);

/// The engine tick the reference implementation stamps its entries with.
fn now_ms() -> u32 {
    // SAFETY: the published signature of a `kernel32` export that takes no
    // arguments and touches no caller memory.
    unsafe { GetTickCount() }
}

/// The raw descriptor-field array of an object, if it has a live one.
fn descriptor(this: *mut u8) -> Option<usize> {
    if this.is_null() {
        return None;
    }
    // SAFETY: `this` is the descriptor-write entry's own receiver, whose
    // `+0x8` the entry itself dereferences on every call.
    let fields = unsafe { *(this.addr().wrapping_add(DESCRIPTOR_PTR) as *const usize) };
    (fields != 0 && fields & 1 == 0).then_some(fields)
}

/// The object's GUID, the first two dwords of its descriptor array.
fn guid_of(this: *mut u8) -> u64 {
    descriptor(this).map_or(0, |fields| {
        // SAFETY: `fields` passed the liveness heuristic; the first two dwords
        // of every descriptor array are the object GUID (not 8-aligned by
        // contract, so read unaligned).
        unsafe { (fields as *const u64).read_unaligned() }
    })
}

/// The live value of one visible-item field.
fn visible_field(fields: usize, slot: usize) -> u32 {
    // SAFETY: `fields` passed the liveness heuristic; the visible-item blocks
    // are `VISIBLE_SLOTS` entries at `VISIBLE_BYTES`, one every stride.
    unsafe { *((fields + VISIBLE_BYTES + slot * VISIBLE_STRIDE_BYTES) as *const u32) }
}

/// Every visible-item field of an object, zeroed when it has no descriptor.
fn visible_snapshot(this: *mut u8) -> [u32; VISIBLE_SLOTS] {
    descriptor(this).map_or([0; VISIBLE_SLOTS], |fields| {
        core::array::from_fn(|slot| visible_field(fields, slot))
    })
}

/// Every equipped-item GUID of an object, zeroed when it has no descriptor.
fn equip_snapshot(fields: usize) -> [u64; VISIBLE_SLOTS] {
    core::array::from_fn(|slot| {
        // SAFETY: `fields` passed the liveness heuristic; the equipped-item
        // GUID pairs run `VISIBLE_SLOTS` deep from `EQUIP_GUID_BYTES`
        // (unaligned by the same contract as the object GUID).
        unsafe { ((fields + EQUIP_GUID_BYTES + slot * 8) as *const u64).read_unaligned() }
    })
}

/// Run the unhooked descriptor write.
fn write_field(this: usize, index: u32, value: u32) {
    (super::symbols::originals::cg_object_c__set_descriptor_field_raw__6142e0())(
        this as *mut u8,
        index,
        value,
    );
}

/// Re-read the local player's mirrors, dropping the binding if it is gone.
///
/// The mirrors describe one specific player object, so a binding is only
/// reusable while that object is still the active player.
fn rebind(player_guid: u64) {
    let Some(player) = super::objmgr::object_by_guid(player_guid) else {
        BOUND_PLAYER.store(0, Ordering::Relaxed);
        if let Ok(mut state) = STATE.lock() {
            state.unbind();
        }
        return;
    };
    let Some(fields) = descriptor(player.raw() as *mut u8) else {
        BOUND_PLAYER.store(0, Ordering::Relaxed);
        if let Ok(mut state) = STATE.lock() {
            state.unbind();
        }
        return;
    };
    let equip = equip_snapshot(fields);
    let values = core::array::from_fn(|slot| visible_field(fields, slot));
    BOUND_PLAYER.store(player.raw(), Ordering::Relaxed);
    if let Ok(mut state) = STATE.lock() {
        state.bind(equip, values);
    }
}

/// Bind the mirrors to `this` when it is the active player, if not already.
///
/// Returns whether `this` is the local player.
fn ensure_bound(this: *mut u8) -> bool {
    let player_guid = super::objmgr::guid_of_token(c"player");
    if player_guid == 0 || guid_of(this) != player_guid {
        return false;
    }
    if BOUND_PLAYER.load(Ordering::Relaxed) != this.addr() {
        rebind(player_guid);
    }
    true
}

/// Decide a descriptor-field write; `true` means the write is swallowed.
///
/// The entry is one of the hottest in the client, so the index tests come
/// first and every write outside the three watched families leaves without
/// reading anything or taking the lock.
pub fn intercept_write(this: *mut u8, index: u32, value: u32) -> bool {
    let visible_slot = index
        .checked_sub(VISIBLE_FIRST)
        .filter(|off| *off < VISIBLE_STRIDE * VISIBLE_SLOT_COUNT && off % VISIBLE_STRIDE == 0)
        .map(|off| (off / VISIBLE_STRIDE) as usize);
    if let Some(slot) = visible_slot {
        return visible_write(this, slot, value);
    }
    if index == ITEM_DEFER_FIELD {
        return item_write(this, value);
    }
    let inv_dword = index
        .checked_sub(INV_GUID_FIRST)
        .filter(|off| *off < INV_GUID_DWORDS);
    if let Some(dword) = inv_dword {
        return inv_guid_write(this, dword, value);
    }
    false
}

/// A write to a visible-item field, on the player or on any other unit.
fn visible_write(this: *mut u8, slot: usize, value: u32) -> bool {
    WRITES_SEEN.fetch_add(1, Ordering::Relaxed);
    let now = now_ms();
    if ensure_bound(this) {
        return player_visible_write(this, slot, value, now);
    }
    let guid = guid_of(this);
    // A GUID whose high half is out of range is a record still being filled
    // in, which the table cannot key on.
    if guid == 0 || guid >> 32 > 0xffff {
        return false;
    }
    let Some(fields) = descriptor(this) else {
        return false;
    };
    let current = visible_field(fields, slot);
    let Ok(mut state) = STATE.lock() else {
        return false;
    };
    let swallowed = state.unit_write_ref(
        guid,
        u32::try_from(slot).expect("slot is below VISIBLE_SLOTS"),
        value,
        current,
        now,
        u32::try_from(this.addr()).unwrap_or(0),
    );
    drop(state);
    if swallowed {
        SWALLOWED.fetch_add(1, Ordering::Relaxed);
    }
    swallowed
}

/// A write to one of the local player's visible-item fields.
fn player_visible_write(this: *mut u8, slot: usize, value: u32, now: u32) -> bool {
    let Ok(mut state) = STATE.lock() else {
        return false;
    };
    let verdict = state.player_visible_write(slot, value, now);
    drop(state);
    match verdict {
        PlayerWrite::Passthrough => false,
        PlayerWrite::Swallow => {
            SWALLOWED.fetch_add(1, Ordering::Relaxed);
            true
        }
        PlayerWrite::SwallowAndScan => {
            SWALLOWED.fetch_add(1, Ordering::Relaxed);
            scan_equipment_status();
            true
        }
        PlayerWrite::SwallowDeferredItem {
            guid,
            value: parked,
        } => {
            SWALLOWED.fetch_add(1, Ordering::Relaxed);
            if let Some(item) = super::objmgr::object_by_guid(guid)
                && descriptor(item.raw() as *mut u8).is_some()
            {
                write_field(item.raw(), ITEM_DEFER_FIELD, parked);
            }
            scan_equipment_status();
            true
        }
        PlayerWrite::ApplyZero => {
            write_field(this.addr(), visible_index(slot), 0);
            true
        }
    }
}

/// A write to an equipped item's own deferred field.
///
/// Parks the value behind the pending zero of the visible slot the item sits
/// in, so the appearance and the item state come back in one step.
fn item_write(this: *mut u8, value: u32) -> bool {
    let guid = guid_of(this);
    if guid == 0 || BOUND_PLAYER.load(Ordering::Relaxed) == 0 {
        return false;
    }
    let now = now_ms();
    let Ok(mut state) = STATE.lock() else {
        return false;
    };
    let slot = (0..VISIBLE_SLOTS).find(|&slot| state.equip_guid(slot) == guid);
    let parked = slot.is_some_and(|slot| state.defer_item_write(slot, value, now));
    drop(state);
    if parked {
        DEFERRED_ITEMS.fetch_add(1, Ordering::Relaxed);
    }
    parked
}

/// A write to one of the player's inventory-slot GUID dwords.
///
/// Never swallowed: the GUID itself always lands. What this adds is applying
/// a slot's parked zero first, in the one case where the zero was real — the
/// item left the slot.
fn inv_guid_write(this: *mut u8, dword: u32, value: u32) -> bool {
    if !ensure_bound(this) {
        return false;
    }
    let Ok(mut state) = STATE.lock() else {
        return false;
    };
    let verdict = state.player_inv_guid_write(dword, value);
    drop(state);
    if let InvWrite::ApplyZeroThenPassthrough { visible_slot } = verdict {
        write_field(this.addr(), visible_index(visible_slot), 0);
    }
    false
}

/// Run the client's equipment-status rescan.
fn scan_equipment_status() {
    // SAFETY: a fixed `.text` entry in the live host image (base verified at
    // load); the transmuted signature matches the declared prototype (`cdecl`,
    // no arguments, no return).
    let scan: extern "cdecl" fn() = unsafe { core::mem::transmute(SCAN_EQUIPMENT_STATUS_VA) };
    scan();
}

/// Re-point a unit at its model and refresh it, once.
///
/// The handle the refresh consumes lives behind a client global; without it
/// there is nothing to refresh against, and re-entering the unit into the
/// world is the client's own path to rebuilding the same state.
fn refresh_unit(obj: usize) {
    REFRESHED.fetch_add(1, Ordering::Relaxed);
    // SAFETY: `MODEL_CONTEXT` is a fixed host global at the verified image
    // base, null before the world is up.
    let context = unsafe { *(MODEL_CONTEXT as *const usize) };
    let handle = if context == 0 || context & 1 != 0 {
        0
    } else {
        // SAFETY: `context` passed the liveness heuristic; `+0x10` is the
        // model handle a display refresh consumes.
        unsafe { *((context + 0x10) as *const usize) }
    };
    if handle == 0 {
        (super::symbols::originals::cg_unit_c__on_enter_world__5fb880())(obj as *mut u8, 0, 0, 1);
        return;
    }
    // SAFETY: `obj` is a live object this flush just wrote a descriptor field
    // through; `MODEL_HANDLE_FIELD` is the model-handle slot the refresh reads.
    unsafe { *((obj + MODEL_HANDLE_FIELD) as *mut usize) = handle };
    // SAFETY: a fixed `.text` entry in the live host image; the transmuted
    // signature matches the declared prototype (`thiscall(ecx = this)`,
    // `RET 0`, no return value).
    let refresh: extern "thiscall" fn(usize) =
        unsafe { core::mem::transmute(REFRESH_DISPLAY_MODEL_VA) };
    refresh(obj);
}

/// Apply every due deferred write, then refresh each unit affected once.
///
/// Runs from scene end. Zeroes apply for every entry that came due; the
/// refresh list is capped, so a frame that drains more units than that leaves
/// the rest to their next natural refresh rather than spending the frame on
/// model rebuilds.
pub fn flush() {
    let now = now_ms();
    {
        let Ok(mut state) = STATE.lock() else {
            return;
        };
        if state.idle() {
            return;
        }
        state.player_expire(now);
    }
    let mut units = [0usize; FLUSH_UNITS_CAP];
    let mut collected = 0usize;
    let mut from = 0usize;
    loop {
        let due = {
            let Ok(state) = STATE.lock() else {
                break;
            };
            state.unit_due(from, now)
        };
        let Some((idx, guid, slot)) = due else {
            break;
        };
        from = idx + 1;
        {
            let Ok(mut state) = STATE.lock() else {
                break;
            };
            state.unit_remove(idx);
        }
        let Some(unit) = super::objmgr::object_by_guid(guid) else {
            continue;
        };
        write_field(unit.raw(), VISIBLE_FIRST + slot * VISIBLE_STRIDE, 0);
        FLUSHED.fetch_add(1, Ordering::Relaxed);
        if !units[..collected].contains(&unit.raw()) && collected < FLUSH_UNITS_CAP {
            units[collected] = unit.raw();
            collected += 1;
        }
    }
    for &obj in &units[..collected] {
        refresh_unit(obj);
    }
}

/// Whether an enter-world call can be answered with a light refresh.
///
/// A unit whose only differences from the last sighting are appearance
/// flickers that already came back does not need the world-entry path: it
/// would rebuild a model that never visibly changed.
pub fn downgrade_enter_world(this: *mut u8) -> bool {
    let guid = guid_of(this);
    if guid == 0 || guid >> 32 > 0xffff {
        return false;
    }
    let now = now_ms();
    if BOUND_PLAYER.load(Ordering::Relaxed) == this.addr() {
        let pending = STATE.lock().is_ok_and(|state| state.player_has_pending());
        if pending {
            light_refresh(this.addr());
            return true;
        }
    }
    let fresh = visible_snapshot(this);
    let downgrade = {
        let Ok(mut state) = STATE.lock() else {
            return false;
        };
        state.seen_enter_world(guid, &fresh, now)
    };
    if !downgrade {
        return false;
    }
    light_refresh(this.addr());
    if BOUND_PLAYER.load(Ordering::Relaxed) == this.addr() {
        scan_equipment_status();
    }
    true
}

/// Re-read a unit's own record and mark it for its next update pass.
fn light_refresh(obj: usize) {
    // SAFETY: a fixed `.text` entry in the live host image (base verified at
    // load); the transmuted signature matches the declared prototype
    // (`thiscall(ecx = this)`, `RET 0`, no return value).
    let refresh: extern "thiscall" fn(usize) =
        unsafe { core::mem::transmute(REFRESH_DATA_POINTERS_VA) };
    refresh(obj);
    for field in REFRESH_FLAG_FIELDS {
        // SAFETY: `obj` is the live unit whose record was just refreshed; both
        // fields are the update flags its next pass reads.
        unsafe { *((obj + field) as *mut u32) = 1 };
    }
}

/// Make the on-disk file check unconditional, so loose files always resolve.
///
/// The resolver only consults the file system when the search scope asks for
/// it and the archive does not suppress the check, which leaves a loose file
/// unreachable on every other lookup path. Both guards are two-byte
/// conditional jumps; replacing each with two no-ops lets the check run every
/// time. Verified before it is written, and reported either way.
pub fn open_disk_lookup(image_base: usize) {
    /// The two guarded branches, with the bytes each must still hold.
    const GUARDS: [(usize, [u8; 2], &str); 2] = [
        (0x0025_4b5c, [0x74, 0x25], "file lookup: scope guard"),
        (0x0025_4b6a, [0x75, 0x17], "file lookup: archive guard"),
    ];
    const NOPS: [u8; 2] = [0x90, 0x90];

    for (rva, expected, label) in GUARDS {
        let va = image_base + rva;
        // SAFETY: `va` is a fixed `.text` address inside the resolver in the
        // live host image (base verified at load), and this runs at attach,
        // before the client's own threads reach that code.
        if !unsafe { wow_hook::patch_bytes(va, &expected, &NOPS, label) } {
            log::warn!(
                target: super::LOG_TARGET,
                "{label}: left unpatched; loose files resolve only on the scoped paths",
            );
        }
    }
}

/// Leave a later same-entry detour on scene end in place.
///
/// Every wrapper of this entry, ours included, delegates unconditionally, so
/// a newcomer's trampoline still reaches whatever was underneath it.
/// Re-asserting would restore the bytes displaced at our install time and
/// orphan the chain the newcomer built.
fn scene_end_chain(owner_va: usize) -> bool {
    let owner = wow_hook::module_of(owner_va).map_or_else(
        || String::from("an unnamed module"),
        |(name, base)| format!("{name}+{:#x}", owner_va.wrapping_sub(base)),
    );
    log::info!(
        target: super::LOG_TARGET,
        "scene end: the entry was re-hooked by {owner}; chaining underneath it",
    );
    false
}

/// Register the never-reassert overwrite policy for the scene-end entry.
pub fn arm_scene_end_policy(image_base: usize) {
    wow_hook::on_overwrite(image_base + SCENE_END_RVA, scene_end_chain);
}

/// One cumulative counters line on the events gauge's 60-second cadence.
pub fn emit_cumulative() {
    let seen = WRITES_SEEN.load(Ordering::Relaxed);
    let swallowed = SWALLOWED.load(Ordering::Relaxed);
    let flushed = FLUSHED.load(Ordering::Relaxed);
    let refreshed = REFRESHED.load(Ordering::Relaxed);
    let deferred = DEFERRED_ITEMS.load(Ordering::Relaxed);
    log::debug!(
        target: "wow::events",
        "transmog: {seen} visible writes, {swallowed} coalesced, \
         {flushed} applied late, {refreshed} refreshes, {deferred} item writes parked",
    );
}
