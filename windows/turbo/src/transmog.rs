//! Visible-item write-coalescing kernel backing the descriptor-write hooks.
//!
//! The client applies every equipment-appearance change eagerly: each raw
//! descriptor write to a visible-item slot can trigger a full model refresh,
//! and the refresh is what loads models and textures. Server-driven appearance
//! updates arrive as a zero-then-restore flicker on the same field, so the
//! eager path reloads a model for a change that is gone one packet later, and
//! a burst of them (browsing an appearance collection, a crowd updating) is a
//! visible stall.
//!
//! This kernel suppresses the flicker. A write of zero to a visible-item
//! field is swallowed and remembered; the restoring write inside a 100 ms
//! window is swallowed too (nothing changed on screen), while a genuinely new
//! value passes through. For other units a swallowed zero that nothing
//! restored is applied for real at the next scene-end flush, at most 16 units
//! per frame, each followed by exactly one display refresh. For the local
//! player a pending zero simply expires (5 s): the slot only really empties
//! when the equipped-item mirror says the item left the slot, and that path
//! applies the zero immediately. A companion table of recently-seen units
//! lets the enter-world hook downgrade a full re-enter to a lightweight
//! refresh when every difference it sees is such an in-window flicker.
//!
//! Pure and host-testable: no FFI, no statics, no game-memory access, and the
//! clock is a `now_ms` argument. The FFI adapter owns descriptor reads and
//! writes, object-manager lookups and the refresh calls; this module answers
//! only "what should happen to this write".

/// Number of visible-item slots on a player.
pub const VISIBLE_SLOTS: usize = 19;

/// First descriptor dword index of the first visible-item block.
// Read only by the 32-bit adapter, so a host test build has no reader.
#[cfg_attr(not(target_arch = "x86"), allow(dead_code))]
pub const VISIBLE_FIRST: u32 = 0xf8;

/// Descriptor dwords per visible-item block.
// Read only by the 32-bit adapter, so a host test build has no reader.
#[cfg_attr(not(target_arch = "x86"), allow(dead_code))]
pub const VISIBLE_STRIDE: u32 = 0xc;

/// First descriptor dword index of the inventory-slot GUID region.
// Read only by the 32-bit adapter, so a host test build has no reader.
#[cfg_attr(not(target_arch = "x86"), allow(dead_code))]
pub const INV_GUID_FIRST: u32 = 0x1da;

/// Dwords in the inventory-slot GUID region (24 GUID pairs).
// Read only by the 32-bit adapter, so a host test build has no reader.
#[cfg_attr(not(target_arch = "x86"), allow(dead_code))]
pub const INV_GUID_DWORDS: u32 = 0x30;

/// First GUID pair of the region that holds an equipped (visible) item.
///
/// Pairs below this cover non-equipment inventory; pair `INV_EQUIP_PAIR + k`
/// is the item shown in visible-item slot `k`.
pub const INV_EQUIP_PAIR: u32 = 5;

/// The item-descriptor dword the deferred equipped-item write targets.
// Read only by the 32-bit adapter, so a host test build has no reader.
#[cfg_attr(not(target_arch = "x86"), allow(dead_code))]
pub const ITEM_DEFER_FIELD: u32 = 0x2e;

/// Window in which a restore cancels a swallowed zero, in milliseconds.
const RESTORE_WINDOW_MS: u32 = 0x64;

/// Lifetime of a pending player-slot zero before it is dropped, in ms.
const PLAYER_PENDING_TTL_MS: u32 = 0x1388;

/// Entries in the unit table (open addressing, linear probe).
const UNIT_TABLE_LEN: usize = 0x407;

/// Probes before a lookup or an insert gives up.
const UNIT_PROBE_LIMIT: usize = 32;

/// Units flushed (and refreshed) per scene-end at most.
pub const FLUSH_UNITS_CAP: usize = 16;

/// Recently-seen units tracked for the enter-world downgrade.
const SEEN_UNITS: usize = 64;

/// One swallowed unit-slot zero, keyed by (GUID, slot).
struct UnitEntry {
    guid: u64,
    slot: u32,
    stamp_ms: u32,
    unit_ref: u32,
    used: bool,
}

const EMPTY_UNIT: UnitEntry = UnitEntry {
    guid: 0,
    slot: 0,
    stamp_ms: 0,
    unit_ref: 0,
    used: false,
};

/// One player visible-item slot's pending state.
struct PlayerSlot {
    /// The last non-zero value written to the slot's visible field.
    value: u32,
    stamp_ms: u32,
    /// Deferred equipped-item field value, applied when the restore arrives.
    deferred: u32,
    valid: bool,
    /// Whether a deferred item write is parked in `deferred`.
    has_deferred: bool,
}

const EMPTY_PLAYER_SLOT: PlayerSlot = PlayerSlot {
    value: 0,
    stamp_ms: 0,
    deferred: 0,
    valid: false,
    has_deferred: false,
};

/// Snapshot of one recently-seen unit's visible fields.
struct SeenUnit {
    guid: u64,
    stamp_ms: u32,
    snapshot: [u32; VISIBLE_SLOTS],
    /// Per-slot stamp of a value that went to zero, awaiting its restore.
    zero_stamp: [u32; VISIBLE_SLOTS],
    /// Whether some slot's zero is still inside its restore window.
    pending: bool,
    used: bool,
}

const EMPTY_SEEN: SeenUnit = SeenUnit {
    guid: 0,
    stamp_ms: 0,
    snapshot: [0; VISIBLE_SLOTS],
    zero_stamp: [0; VISIBLE_SLOTS],
    pending: false,
    used: false,
};

/// Verdict for a write to one of the local player's visible-item fields.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlayerWrite {
    /// Run the stock write.
    Passthrough,
    /// Drop the write; the descriptor already shows the right value.
    Swallow,
    /// Drop the write and re-scan equipment status.
    SwallowAndScan,
    /// Drop the write and park `value` for the slot's equipped item.
    ///
    /// The parked value lands on that item's [`ITEM_DEFER_FIELD`] dword, and
    /// equipment status is re-scanned after.
    SwallowDeferredItem {
        /// GUID of the equipped item to apply the deferred value to.
        guid: u64,
        /// The parked value for the item's [`ITEM_DEFER_FIELD`] dword.
        value: u32,
    },
    /// Run the stock write with value zero: the slot really emptied.
    ApplyZero,
}

/// Verdict for a write to one of the inventory-slot GUID fields.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InvWrite {
    /// Run the stock write.
    Passthrough,
    /// Apply the pending zero first, then run the stock write.
    ///
    /// The zero goes to `visible_slot`'s visible field, which the stock write
    /// would otherwise overtake.
    ApplyZeroThenPassthrough {
        /// The visible-item slot whose parked zero must be applied first.
        visible_slot: usize,
    },
}

/// The coalescing state machine.
///
/// One instance serves the whole process; the adapter wraps it in its lock.
pub struct Coalescer {
    units: Box<[UnitEntry]>,
    unit_count: u32,
    player: [PlayerSlot; VISIBLE_SLOTS],
    player_count: u32,
    /// Mirror of the last non-zero value seen per player visible slot.
    last_value: [u32; VISIBLE_SLOTS],
    /// Mirror of the equipped-item GUID per player visible slot.
    equip_guids: [u64; VISIBLE_SLOTS],
    /// Whether the player mirrors below are populated.
    bound: bool,
    seen: Box<[SeenUnit]>,
}

impl Coalescer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            units: core::iter::repeat_with(|| EMPTY_UNIT)
                .take(UNIT_TABLE_LEN)
                .collect(),
            unit_count: 0,
            player: [EMPTY_PLAYER_SLOT; VISIBLE_SLOTS],
            player_count: 0,
            last_value: [0; VISIBLE_SLOTS],
            equip_guids: [0; VISIBLE_SLOTS],
            bound: false,
            seen: core::iter::repeat_with(|| EMPTY_SEEN)
                .take(SEEN_UNITS)
                .collect(),
        }
    }

    /// Whether both pending pools are empty (the scene-end early-out).
    #[must_use]
    pub fn idle(&self) -> bool {
        self.player_count == 0 && self.unit_count == 0
    }

    /// Drop the player binding (the active player could not be resolved).
    pub fn unbind(&mut self) {
        self.bound = false;
    }

    /// Populate the player mirrors from live descriptor state.
    pub fn bind(&mut self, equip_guids: [u64; VISIBLE_SLOTS], values: [u32; VISIBLE_SLOTS]) {
        self.equip_guids = equip_guids;
        self.last_value = values;
        self.bound = true;
    }

    /// Equipped-item GUID mirrored for a visible slot.
    #[must_use]
    pub fn equip_guid(&self, slot: usize) -> u64 {
        self.equip_guids[slot]
    }

    /// Verdict for a write to the player's visible-item field of `slot`.
    pub fn player_visible_write(&mut self, slot: usize, value: u32, now_ms: u32) -> PlayerWrite {
        if value != 0 {
            let entry = &mut self.player[slot];
            if !entry.valid {
                self.last_value[slot] = value;
                return PlayerWrite::Passthrough;
            }
            if value != entry.value {
                entry.valid = false;
                entry.has_deferred = false;
                self.player_count -= 1;
                self.last_value[slot] = value;
                return PlayerWrite::Passthrough;
            }
            // The restore of a swallowed zero: the descriptor still shows
            // `value`, so the write itself is a no-op either way.
            if !entry.has_deferred {
                entry.valid = false;
                self.player_count -= 1;
                return PlayerWrite::SwallowAndScan;
            }
            let deferred = entry.deferred;
            entry.valid = false;
            entry.has_deferred = false;
            self.player_count -= 1;
            if deferred == 0 {
                return PlayerWrite::Passthrough;
            }
            let guid = self.equip_guids[slot];
            if guid == 0 {
                return PlayerWrite::SwallowAndScan;
            }
            return PlayerWrite::SwallowDeferredItem {
                guid,
                value: deferred,
            };
        }
        let previous = self.last_value[slot];
        if previous == 0 {
            return PlayerWrite::Passthrough;
        }
        if self.bound && self.equip_guids[slot] == 0 {
            self.last_value[slot] = 0;
            return PlayerWrite::ApplyZero;
        }
        let entry = &mut self.player[slot];
        if !entry.valid {
            self.player_count += 1;
        }
        entry.value = previous;
        entry.stamp_ms = now_ms;
        entry.valid = true;
        entry.has_deferred = false;
        PlayerWrite::Swallow
    }

    /// Verdict for a write to an inventory-slot GUID dword.
    ///
    /// `dword` is the offset inside the GUID region (`index - INV_GUID_FIRST`,
    /// already checked against [`INV_GUID_DWORDS`]). Requires the player
    /// mirrors to be bound; unbound callers pass the write through untouched.
    pub fn player_inv_guid_write(&mut self, dword: u32, value: u32) -> InvWrite {
        let pair = dword / 2;
        if dword < INV_EQUIP_PAIR * 2 {
            return InvWrite::Passthrough;
        }
        let slot = (pair - INV_EQUIP_PAIR) as usize;
        let mirrored = self.equip_guids[slot];
        self.equip_guids[slot] = if dword & 1 == 0 {
            (mirrored & 0xffff_ffff_0000_0000) | u64::from(value)
        } else {
            (mirrored & 0xffff_ffff) | (u64::from(value) << 32)
        };
        if dword & 1 == 0 && value == 0 && self.player[slot].valid {
            self.last_value[slot] = 0;
            self.player[slot].valid = false;
            self.player[slot].has_deferred = false;
            self.player_count -= 1;
            return InvWrite::ApplyZeroThenPassthrough { visible_slot: slot };
        }
        InvWrite::Passthrough
    }

    /// Park an equipped-item field write behind `slot`'s pending zero.
    ///
    /// Returns whether the write was parked (caller swallows it); a slot with
    /// no pending zero leaves the write to stock.
    pub fn defer_item_write(&mut self, slot: usize, value: u32, now_ms: u32) -> bool {
        let entry = &mut self.player[slot];
        if !entry.valid {
            return false;
        }
        entry.deferred = value;
        entry.has_deferred = true;
        entry.stamp_ms = now_ms;
        true
    }

    /// Verdict for a visible-field write on a unit that is not the player.
    ///
    /// `current` is the field's live descriptor value. Returns whether the
    /// write is swallowed; `false` runs the stock write.
    pub fn unit_write(
        &mut self,
        guid: u64,
        slot: u32,
        value: u32,
        current: u32,
        now_ms: u32,
    ) -> bool {
        let found = self.unit_find(guid, slot);
        if value != 0 {
            let Some(idx) = found else {
                return false;
            };
            let age = now_ms.wrapping_sub(self.units[idx].stamp_ms);
            self.units[idx].used = false;
            self.unit_count -= 1;
            // The restore inside the window, of the exact value still on
            // screen: the pair of writes cancels out.
            return age < RESTORE_WINDOW_MS && value == current;
        }
        if current == 0 {
            return false;
        }
        match found {
            Some(idx) => {
                self.units[idx].stamp_ms = now_ms;
                true
            }
            None => self.unit_insert(guid, slot, now_ms, 0),
        }
    }

    /// [`Coalescer::unit_write`] with the unit reference recorded on insert.
    pub fn unit_write_ref(
        &mut self,
        guid: u64,
        slot: u32,
        value: u32,
        current: u32,
        now_ms: u32,
        unit_ref: u32,
    ) -> bool {
        let swallowed = self.unit_write(guid, slot, value, current, now_ms);
        if swallowed
            && value == 0
            && let Some(idx) = self.unit_find(guid, slot)
        {
            self.units[idx].unit_ref = unit_ref;
        }
        swallowed
    }

    /// Next unit entry at or after `from` whose zero is due for applying.
    ///
    /// Returns the entry index with its key; the caller looks the unit up,
    /// applies the zero (or not) and always [`Coalescer::unit_remove`]s it.
    #[must_use]
    pub fn unit_due(&self, from: usize, now_ms: u32) -> Option<(usize, u64, u32)> {
        if self.unit_count == 0 {
            return None;
        }
        self.units[from..].iter().enumerate().find_map(|(off, e)| {
            (e.used && now_ms.wrapping_sub(e.stamp_ms) >= RESTORE_WINDOW_MS).then_some((
                from + off,
                e.guid,
                e.slot,
            ))
        })
    }

    /// Drop a unit entry found via [`Coalescer::unit_due`].
    pub fn unit_remove(&mut self, idx: usize) {
        if self.units[idx].used {
            self.units[idx].used = false;
            self.unit_count -= 1;
        }
    }

    /// Drop every pending player entry whose zero aged past its lifetime.
    ///
    /// A pending zero the server never restored is stale after 5 s: applying
    /// it then would blank a slot the player still sees filled, so it is
    /// dropped instead.
    pub fn player_expire(&mut self, now_ms: u32) {
        if self.player_count == 0 {
            return;
        }
        for entry in &mut self.player {
            if entry.valid && now_ms.wrapping_sub(entry.stamp_ms) >= PLAYER_PENDING_TTL_MS {
                entry.valid = false;
                entry.has_deferred = false;
                self.player_count -= 1;
            }
        }
    }

    /// Whether the player has any pending entry (the enter-world downgrade).
    #[must_use]
    pub fn player_has_pending(&self) -> bool {
        self.player_count > 0
    }

    /// Record a unit entering the world; returns whether to downgrade.
    ///
    /// `fresh` is the unit's live visible-field snapshot. Downgrading is
    /// correct only when every difference from the stored snapshot is a
    /// zero-then-restore flicker inside the restore window: then the full
    /// enter-world would rebuild a model that never visibly changed.
    pub fn seen_enter_world(
        &mut self,
        guid: u64,
        fresh: &[u32; VISIBLE_SLOTS],
        now_ms: u32,
    ) -> bool {
        let idx = self.seen_slot(guid, now_ms);
        let entry = &mut self.seen[idx];
        entry.stamp_ms = now_ms;
        let mut restored_in_window = 0u32;
        let mut went_zero = 0u32;
        let mut all_restored = true;
        for ((stored, live), zero_stamp) in entry
            .snapshot
            .iter()
            .zip(fresh.iter())
            .zip(entry.zero_stamp.iter_mut())
        {
            if stored == live {
                continue;
            }
            if *stored != 0 && *live == 0 {
                went_zero += 1;
                *zero_stamp = now_ms;
                entry.pending = true;
            } else if *stored == 0 && *live != 0 {
                if *zero_stamp != 0 && now_ms.wrapping_sub(*zero_stamp) < RESTORE_WINDOW_MS {
                    restored_in_window += 1;
                    *zero_stamp = 0;
                } else {
                    all_restored = false;
                }
            } else {
                *zero_stamp = 0;
                all_restored = false;
            }
        }
        entry.snapshot = *fresh;
        entry.pending = false;
        for zero_stamp in &mut entry.zero_stamp {
            if *zero_stamp == 0 {
                continue;
            }
            if now_ms.wrapping_sub(*zero_stamp) >= RESTORE_WINDOW_MS {
                *zero_stamp = 0;
            } else {
                entry.pending = true;
            }
        }
        restored_in_window > 0 && went_zero == 0 && all_restored
    }

    /// Index of `guid`'s seen-unit entry, claiming the oldest on a miss.
    fn seen_slot(&mut self, guid: u64, now_ms: u32) -> usize {
        let mut oldest = 0usize;
        let mut oldest_stamp = u32::MAX;
        let mut free = None;
        for (i, entry) in self.seen.iter().enumerate() {
            if entry.used && entry.guid == guid {
                return i;
            }
            if !entry.used && free.is_none() {
                free = Some(i);
            }
            if entry.stamp_ms < oldest_stamp {
                oldest_stamp = entry.stamp_ms;
                oldest = i;
            }
        }
        let idx = free.unwrap_or(oldest);
        self.seen[idx] = SeenUnit {
            guid,
            stamp_ms: now_ms,
            used: true,
            ..EMPTY_SEEN
        };
        idx
    }

    fn unit_find(&self, guid: u64, slot: u32) -> Option<usize> {
        let start = unit_bucket(guid, slot);
        (0..UNIT_PROBE_LIMIT).find_map(|step| {
            let idx = (start + step) % UNIT_TABLE_LEN;
            let e = &self.units[idx];
            (e.used && e.guid == guid && e.slot == slot).then_some(idx)
        })
    }

    fn unit_insert(&mut self, guid: u64, slot: u32, now_ms: u32, unit_ref: u32) -> bool {
        let start = unit_bucket(guid, slot);
        for step in 0..UNIT_PROBE_LIMIT {
            let idx = (start + step) % UNIT_TABLE_LEN;
            if self.units[idx].used {
                continue;
            }
            self.units[idx] = UnitEntry {
                guid,
                slot,
                stamp_ms: now_ms,
                unit_ref,
                used: true,
            };
            self.unit_count += 1;
            return true;
        }
        false
    }
}

impl Default for Coalescer {
    fn default() -> Self {
        Self::new()
    }
}

/// Table slot a (GUID, visible slot) pair probes from.
fn unit_bucket(guid: u64, slot: u32) -> usize {
    // The truncation is the operation: the mix wants the low half of a wider
    // value, and a checked conversion would reject exactly the inputs it is
    // fed.
    #[expect(clippy::cast_possible_truncation)]
    let guid_lo = guid as u32;
    let guid_hi = (guid >> 32) as u32;
    let p = 0x9e37_79b1u64 * u64::from(slot);
    // The truncation is the operation: the mix wants the low half of a wider
    // value, and a checked conversion would reject exactly the inputs it is
    // fed.
    #[expect(clippy::cast_possible_truncation)]
    let a = (p as u32) ^ guid_lo;
    let d = ((p >> 32) as u32) ^ guid_hi;
    let c = d.wrapping_mul(0xed55_8ccd);
    let folded = (d >> 1) ^ a;
    let q = u64::from(folded) * 0xed55_8ccd;
    let hi = folded
        .wrapping_mul(0xff51_afd7)
        .wrapping_add((q >> 32) as u32)
        .wrapping_add(c);
    // The truncation is the operation: the mix wants the low half of a wider
    // value, and a checked conversion would reject exactly the inputs it is
    // fed.
    #[expect(clippy::cast_possible_truncation)]
    let lo = (hi >> 1) ^ (q as u32);
    let mixed = (u64::from(hi) << 32) | u64::from(lo);
    (mixed % UNIT_TABLE_LEN as u64) as usize
}

#[cfg(test)]
mod tests_transmog {
    use super::{Coalescer, FLUSH_UNITS_CAP, InvWrite, PlayerWrite, VISIBLE_SLOTS};

    fn bound() -> Coalescer {
        let mut c = Coalescer::new();
        let mut guids = [0u64; VISIBLE_SLOTS];
        let mut values = [0u32; VISIBLE_SLOTS];
        for (i, (g, v)) in guids.iter_mut().zip(values.iter_mut()).enumerate() {
            *g = 0x4000_0000_0000 + i as u64;
            *v = 100 + i as u32;
        }
        c.bind(guids, values);
        c
    }

    #[test]
    fn unit_flicker_is_swallowed_entirely() {
        let mut c = Coalescer::new();
        // Zero write while the field still shows 500: swallowed, recorded.
        assert!(c.unit_write(0x1234, 3, 0, 500, 1000));
        assert!(!c.idle());
        // Restore of the same value inside the window: swallowed, forgotten.
        assert!(c.unit_write(0x1234, 3, 500, 500, 1050));
        assert!(c.idle());
        assert_eq!(c.unit_due(0, 2000), None);
    }

    #[test]
    fn unit_restore_with_new_value_passes_through() {
        let mut c = Coalescer::new();
        assert!(c.unit_write(0x1234, 3, 0, 500, 1000));
        // A different value is a real change: pass through, entry dropped.
        assert!(!c.unit_write(0x1234, 3, 777, 500, 1050));
        assert!(c.idle());
    }

    #[test]
    fn unit_restore_after_window_passes_through() {
        let mut c = Coalescer::new();
        assert!(c.unit_write(0x1234, 3, 0, 500, 1000));
        assert!(!c.unit_write(0x1234, 3, 500, 500, 1100));
        assert!(c.idle());
    }

    #[test]
    fn unit_zero_on_zero_field_passes_through() {
        let mut c = Coalescer::new();
        assert!(!c.unit_write(0x1234, 3, 0, 0, 1000));
        assert!(c.idle());
    }

    #[test]
    fn unit_due_after_window_and_removal() {
        let mut c = Coalescer::new();
        assert!(c.unit_write_ref(0x1234, 3, 0, 500, 1000, 0xdead));
        assert_eq!(c.unit_due(0, 1050), None);
        let (idx, guid, slot) = c.unit_due(0, 1100).expect("due after the window");
        assert_eq!((guid, slot), (0x1234, 3));
        c.unit_remove(idx);
        assert!(c.idle());
        assert_eq!(c.unit_due(0, 1100), None);
    }

    #[test]
    fn unit_same_key_reinsert_refreshes_stamp() {
        let mut c = Coalescer::new();
        assert!(c.unit_write(0x1234, 3, 0, 500, 1000));
        assert!(c.unit_write(0x1234, 3, 0, 500, 1090));
        // Refreshed at 1090: not due at 1150, due at 1190.
        assert_eq!(c.unit_due(0, 1150), None);
        assert!(c.unit_due(0, 1190).is_some());
    }

    #[test]
    fn unit_table_survives_many_distinct_keys() {
        let mut c = Coalescer::new();
        let mut inserted = 0u32;
        for i in 0..FLUSH_UNITS_CAP as u64 * 8 {
            if c.unit_write(0x5000 + i, (i % 19) as u32, 0, 42, 1000) {
                inserted += 1;
            }
        }
        assert!(inserted > 100);
        let mut drained = 0u32;
        let mut from = 0usize;
        while let Some((idx, _, _)) = c.unit_due(from, 2000) {
            c.unit_remove(idx);
            from = idx + 1;
            drained += 1;
        }
        assert_eq!(drained, inserted);
        assert!(c.idle());
    }

    #[test]
    fn player_flicker_swallow_and_rescan() {
        let mut c = bound();
        // Zero on a filled slot with the item still equipped: swallowed.
        assert_eq!(c.player_visible_write(2, 0, 1000), PlayerWrite::Swallow);
        assert!(c.player_has_pending());
        // Restore of the same value: swallowed, equipment rescan requested.
        assert_eq!(
            c.player_visible_write(2, 102, 1010),
            PlayerWrite::SwallowAndScan
        );
        assert!(!c.player_has_pending());
    }

    #[test]
    fn player_zero_with_empty_slot_applies() {
        let mut c = bound();
        // Clear both GUID halves: the slot is genuinely empty afterwards.
        assert_eq!(
            c.player_inv_guid_write(2 * (5 + 4), 0),
            InvWrite::Passthrough
        );
        assert_eq!(
            c.player_inv_guid_write(2 * (5 + 4) + 1, 0),
            InvWrite::Passthrough
        );
        assert_eq!(c.player_visible_write(4, 0, 1000), PlayerWrite::ApplyZero);
        // The mirror now holds zero: a second zero write passes through.
        assert_eq!(c.player_visible_write(4, 0, 1010), PlayerWrite::Passthrough);
    }

    #[test]
    fn player_new_value_passes_through_and_drops_pending() {
        let mut c = bound();
        assert_eq!(c.player_visible_write(2, 0, 1000), PlayerWrite::Swallow);
        assert_eq!(
            c.player_visible_write(2, 999, 1010),
            PlayerWrite::Passthrough
        );
        assert!(!c.player_has_pending());
        // The mirror learned the new value: its flicker coalesces next.
        assert_eq!(c.player_visible_write(2, 0, 2000), PlayerWrite::Swallow);
        assert_eq!(
            c.player_visible_write(2, 999, 2010),
            PlayerWrite::SwallowAndScan
        );
    }

    #[test]
    fn player_deferred_item_write_rides_the_restore() {
        let mut c = bound();
        assert_eq!(c.player_visible_write(2, 0, 1000), PlayerWrite::Swallow);
        assert!(c.defer_item_write(2, 0x5555, 1005));
        assert_eq!(
            c.player_visible_write(2, 102, 1010),
            PlayerWrite::SwallowDeferredItem {
                guid: 0x4000_0000_0002,
                value: 0x5555
            },
        );
    }

    #[test]
    fn player_defer_without_pending_is_refused() {
        let mut c = bound();
        assert!(!c.defer_item_write(2, 0x5555, 1005));
    }

    #[test]
    fn player_pending_expires_without_applying() {
        let mut c = bound();
        assert_eq!(c.player_visible_write(2, 0, 1000), PlayerWrite::Swallow);
        c.player_expire(5999);
        assert!(c.player_has_pending());
        c.player_expire(6000);
        assert!(!c.player_has_pending());
    }

    #[test]
    fn inv_guid_clear_applies_pending_zero() {
        let mut c = bound();
        assert_eq!(c.player_visible_write(6, 0, 1000), PlayerWrite::Swallow);
        // The equipped item leaves the slot. The apply triggers on the low
        // GUID half going to zero; the high half follows separately.
        assert_eq!(
            c.player_inv_guid_write(2 * (5 + 6), 0),
            InvWrite::ApplyZeroThenPassthrough { visible_slot: 6 },
        );
        assert!(!c.player_has_pending());
        assert_eq!(
            c.player_inv_guid_write(2 * (5 + 6) + 1, 0),
            InvWrite::Passthrough
        );
        assert_eq!(c.equip_guid(6), 0);
    }

    #[test]
    fn inv_guid_below_equipment_region_is_ignored() {
        let mut c = bound();
        let before = c.equip_guid(0);
        assert_eq!(c.player_inv_guid_write(3, 0x77), InvWrite::Passthrough);
        assert_eq!(c.equip_guid(0), before);
    }

    #[test]
    fn inv_guid_halves_merge() {
        let mut c = bound();
        assert_eq!(
            c.player_inv_guid_write(2 * 5, 0x1111),
            InvWrite::Passthrough
        );
        assert_eq!(
            c.player_inv_guid_write(2 * 5 + 1, 0x2222),
            InvWrite::Passthrough
        );
        assert_eq!(c.equip_guid(0), 0x2222_0000_1111);
    }

    #[test]
    fn seen_downgrades_only_pure_flicker() {
        let mut c = Coalescer::new();
        let filled: [u32; VISIBLE_SLOTS] = core::array::from_fn(|i| 10 + i as u32);
        let mut blanked = filled;
        blanked[7] = 0;
        // First sight records the snapshot, no downgrade.
        assert!(!c.seen_enter_world(0x9999, &filled, 1000));
        // Slot goes to zero: a full enter-world (something changed).
        assert!(!c.seen_enter_world(0x9999, &blanked, 1010));
        // Restore inside the window: pure flicker, downgrade.
        assert!(c.seen_enter_world(0x9999, &filled, 1050));
    }

    #[test]
    fn seen_restore_after_window_is_full_enter() {
        let mut c = Coalescer::new();
        let filled: [u32; VISIBLE_SLOTS] = core::array::from_fn(|i| 10 + i as u32);
        let mut blanked = filled;
        blanked[7] = 0;
        assert!(!c.seen_enter_world(0x9999, &filled, 1000));
        assert!(!c.seen_enter_world(0x9999, &blanked, 1010));
        assert!(!c.seen_enter_world(0x9999, &filled, 1200));
    }

    #[test]
    fn seen_any_nonzero_restore_counts_as_flicker() {
        let mut c = Coalescer::new();
        let filled: [u32; VISIBLE_SLOTS] = core::array::from_fn(|i| 10 + i as u32);
        let mut blanked = filled;
        blanked[7] = 0;
        let mut changed = filled;
        changed[7] = 555;
        assert!(!c.seen_enter_world(0x9999, &filled, 1000));
        assert!(!c.seen_enter_world(0x9999, &blanked, 1010));
        // The stored snapshot holds the zero, so any in-window nonzero
        // restore reads as flicker; the pre-zero value is not compared.
        assert!(c.seen_enter_world(0x9999, &changed, 1050));
    }

    #[test]
    fn seen_direct_value_change_is_full_enter() {
        let mut c = Coalescer::new();
        let filled: [u32; VISIBLE_SLOTS] = core::array::from_fn(|i| 10 + i as u32);
        let mut changed = filled;
        changed[7] = 555;
        assert!(!c.seen_enter_world(0x9999, &filled, 1000));
        assert!(!c.seen_enter_world(0x9999, &changed, 1010));
    }

    #[test]
    fn seen_table_recycles_oldest() {
        let mut c = Coalescer::new();
        let snap = [1u32; VISIBLE_SLOTS];
        for i in 0..70u64 {
            assert!(!c.seen_enter_world(0xa000 + i, &snap, 1000 + i as u32));
        }
        // Entry 0xa000 was recycled; re-seeing it starts from scratch and
        // the flicker dance works like a first sighting.
        let mut blanked = snap;
        blanked[0] = 0;
        assert!(!c.seen_enter_world(0xa000, &snap, 2000));
        assert!(!c.seen_enter_world(0xa000, &blanked, 2010));
        assert!(c.seen_enter_world(0xa000, &snap, 2040));
    }

    #[test]
    fn unbound_mirrors_swallow_zero_without_apply() {
        let mut c = Coalescer::new();
        let mut values = [0u32; VISIBLE_SLOTS];
        values[3] = 300;
        c.bind([0u64; VISIBLE_SLOTS], values);
        c.unbind();
        // Unbound: the empty-slot fast path is unavailable, zeroes park.
        assert_eq!(c.player_visible_write(3, 0, 1000), PlayerWrite::Swallow);
    }
}
