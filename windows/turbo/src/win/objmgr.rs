//! Typed access to the client's object manager and unit records.
//!
//! Consolidates the object-manager facts scattered across adapters into one
//! place: the manager global, the GUID hash lookup, the per-object offsets
//! (GUID `+0x30`, type `+0x14`, descriptor `*(obj+0x110)`, movement
//! `*(obj+0x118)`), and the behavioral entry points shared by more than one
//! feature. Every raw pointer read is guarded by the client's own liveness
//! heuristic — non-null and even — before it is dereferenced, and a reader
//! whose record is missing answers `None` rather than a sentinel.
//!
//! Positions go through the object's own virtual `GetPosition` (vtable slot
//! `+0x14`) rather than the movement record at `+0x10`, because the virtual
//! reading resolves transport-relative coordinates and the raw record does
//! not.

/// The object-manager global; the live manager pointer, zero out-of-world.
const OBJECT_MANAGER: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0074_1414;
/// The engine's millisecond tick global, advanced once per frame.
const GAME_TICK: usize = crate::win::EXPECTED_IMAGE_BASE + 0x008f_0bc8;
/// GUID hash lookup — `fastcall(guid on the stack)`, object pointer or zero.
const OBJECT_BY_GUID_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0006_4870;
/// Unit-token resolver — `fastcall(ecx = token text)`, GUID in `edx:eax`.
const GUID_OF_TOKEN_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0011_5970;
/// Active player GUID — no arguments, GUID in `edx:eax`.
const ACTIVE_PLAYER_GUID_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0006_8550;
/// Set the player's target — `fastcall(ecx = *guid)`.
const TARGET_BY_GUID_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0008_9a40;
/// Whether `self` may attack `target` — `thiscall(ecx = self) + stack(target)`.
const CAN_ATTACK_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0020_6980;
/// A unit's creature type — `fastcall(ecx = unit)`.
const CREATURE_TYPE_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0020_5570;

/// Object type codes at object `+0x14`.
pub const TYPE_UNIT: i32 = 3;
/// See [`TYPE_UNIT`].
pub const TYPE_PLAYER: i32 = 4;

/// Creature type code for critters.
pub const CREATURE_TYPE_CRITTER: i32 = 8;

/// Classification code for world bosses (descriptor block at `+0xb30`).
pub const CLASSIFICATION_WORLD_BOSS: i32 = 3;

/// Movement flags meaning the unit is translating, turning, or airborne.
///
/// Forward/backward/strafe, pitch, jumping, falling far, spline elevation,
/// plus the two turn bits.
const MOVING_OR_TURNING: u32 = 0x0400_60ff;

/// A validated pointer to a live object-manager object.
///
/// The wrapped address passed the client's own liveness heuristic (non-null,
/// even) at construction; record readers re-apply it to every indirection
/// they follow, so a missing descriptor or movement record answers `None` or
/// a zero default rather than faulting.
#[derive(Clone, Copy)]
pub struct UnitRef(usize);

impl UnitRef {
    /// Wrap a raw object pointer, refusing null and odd (freed-slot) values.
    pub fn try_from_raw(raw: u32) -> Option<Self> {
        let addr = raw as usize;
        (addr != 0 && addr & 1 == 0).then_some(Self(addr))
    }

    /// The raw object address, for client calls that take the object itself.
    pub const fn raw(self) -> usize {
        self.0
    }

    /// The object's GUID, at `+0x30`.
    pub fn guid(self) -> u64 {
        // SAFETY: the wrapped address passed the liveness heuristic; `+0x30`
        // is the object's GUID field (not 8-aligned by contract, so read
        // unaligned).
        unsafe { ((self.0 + 0x30) as *const u64).read_unaligned() }
    }

    /// The object's type code, at `+0x14`.
    pub fn object_type(self) -> i32 {
        // SAFETY: the wrapped address passed the liveness heuristic; `+0x14`
        // is the object's type code.
        unsafe { *((self.0 + 0x14) as *const i32) }
    }

    /// Whether the object is a unit or a player.
    pub fn is_unit_or_player(self) -> bool {
        matches!(self.object_type(), TYPE_UNIT | TYPE_PLAYER)
    }

    /// The object's world position, through its virtual `GetPosition`.
    ///
    /// Zero when any pointer on the way fails the liveness heuristic — the
    /// shape the features this serves were written against.
    pub fn position(self) -> [f32; 3] {
        let mut out = [0.0f32; 3];
        // SAFETY: the wrapped address passed the liveness heuristic; the
        // object's first word is its vtable pointer.
        let vtbl = unsafe { *(self.0 as *const usize) };
        if vtbl == 0 || vtbl & 1 != 0 {
            return out;
        }
        // SAFETY: `vtbl` passed the liveness heuristic; `+0x14` is the
        // virtual `GetPosition` slot.
        let slot = unsafe { *((vtbl + 0x14) as *const usize) };
        if slot == 0 || slot & 1 != 0 {
            return out;
        }
        // SAFETY: a live vtable entry of a live object; the transmuted
        // signature matches the declared prototype
        // (`__thiscall(ecx = this)` plus the out vector, returned back).
        let get_position: extern "thiscall" fn(usize, *mut [f32; 3]) -> *mut f32 =
            unsafe { core::mem::transmute(slot) };
        get_position(self.0, &raw mut out);
        out
    }

    /// The unit descriptor block, at `*(obj + 0x110)`.
    fn descriptor(self) -> Option<usize> {
        // SAFETY: the wrapped address passed the liveness heuristic; `+0x110`
        // holds the descriptor pointer.
        let attr = unsafe { *((self.0 + 0x110) as *const usize) };
        (attr != 0 && attr & 1 == 0).then_some(attr)
    }

    /// Whether the unit's combat flag (descriptor `+0xa0`, bit `0x80000`) is set.
    pub fn in_combat(self) -> bool {
        self.descriptor().is_some_and(|attr| {
            // SAFETY: `attr` passed the liveness heuristic; `+0xa0` is the
            // unit-flags field.
            let flags = unsafe { *((attr + 0xa0) as *const u32) };
            flags & 0x8_0000 != 0
        })
    }

    /// Whether the unit is player-controlled (descriptor `+0xa0`, bit `0x8`).
    pub fn is_player_controlled(self) -> Option<bool> {
        self.descriptor().map(|attr| {
            // SAFETY: `attr` passed the liveness heuristic; `+0xa0` is the
            // unit-flags field.
            let flags = unsafe { *((attr + 0xa0) as *const u32) };
            flags & 0x8 != 0
        })
    }

    /// The unit's current health (descriptor `+0x40`).
    pub fn current_hp(self) -> Option<u32> {
        self.descriptor().map(|attr| {
            // SAFETY: `attr` passed the liveness heuristic; `+0x40` is the
            // current-health field.
            unsafe { *((attr + 0x40) as *const u32) }
        })
    }

    /// The unit's maximum health (descriptor `+0x58`).
    pub fn max_hp(self) -> Option<u32> {
        self.descriptor().map(|attr| {
            // SAFETY: `attr` passed the liveness heuristic; `+0x58` is the
            // maximum-health field.
            unsafe { *((attr + 0x58) as *const u32) }
        })
    }

    /// Whether the unit is dead: no health, or the feign/dead flag at `+0x224`.
    pub fn is_dead(self) -> Option<bool> {
        self.descriptor().map(|attr| {
            // SAFETY: `attr` passed the liveness heuristic; `+0x40` is the
            // current-health field.
            let hp = unsafe { *((attr + 0x40) as *const u32) };
            // SAFETY: `+0x224` is the dead-state flags field.
            let dead_flags = unsafe { *((attr + 0x224) as *const u32) };
            hp < 1 || dead_flags & 0x20 != 0
        })
    }

    /// The GUID the unit is targeting (descriptor `+0x28`), zero for none.
    pub fn target_guid(self) -> u64 {
        self.descriptor().map_or(0, |attr| {
            // SAFETY: `attr` passed the liveness heuristic; `+0x28` is the
            // target-GUID field (unaligned by the same contract as `guid`).
            unsafe { ((attr + 0x28) as *const u64).read_unaligned() }
        })
    }

    /// The unit's server bounding radius (descriptor `+0x1ec`).
    pub fn bounding_radius(self) -> Option<f32> {
        self.descriptor().map(|attr| {
            // SAFETY: `attr` passed the liveness heuristic; `+0x1ec` is the
            // bounding-radius field.
            unsafe { *((attr + 0x1ec) as *const f32) }
        })
    }

    /// The unit's server combat reach (descriptor `+0x1f0`).
    pub fn combat_reach(self) -> Option<f32> {
        self.descriptor().map(|attr| {
            // SAFETY: `attr` passed the liveness heuristic; `+0x1f0` is the
            // combat-reach field.
            unsafe { *((attr + 0x1f0) as *const f32) }
        })
    }

    /// The unit's mount display id (descriptor `+0x1fc`), zero when unmounted.
    pub fn mount_display_id(self) -> u32 {
        self.descriptor().map_or(0, |attr| {
            // SAFETY: `attr` passed the liveness heuristic; `+0x1fc` is the
            // mount-display field.
            unsafe { *((attr + 0x1fc) as *const u32) }
        })
    }

    /// The unit's classification, gated exactly as the reference reader is.
    ///
    /// Requires the second descriptor block at `+0xb30` AND a zero word at
    /// unit descriptor `+0x214`; the code then comes from the second block's
    /// `+0x20`.
    pub fn classification(self) -> Option<i32> {
        // SAFETY: the wrapped address passed the liveness heuristic; `+0xb30`
        // holds the second descriptor block pointer.
        let attr0 = unsafe { *((self.0 + 0xb30) as *const usize) };
        if attr0 == 0 || attr0 & 1 != 0 {
            return None;
        }
        let attr1 = self.descriptor()?;
        // SAFETY: `attr1` passed the liveness heuristic; `+0x214` gates
        // whether the classification block is meaningful.
        if unsafe { *((attr1 + 0x214) as *const u32) } != 0 {
            return None;
        }
        // SAFETY: `attr0` passed the liveness heuristic; `+0x20` is the
        // classification code.
        Some(unsafe { *((attr0 + 0x20) as *const i32) })
    }

    /// The movement record, at `*(obj + 0x118)`.
    fn movement(self) -> Option<usize> {
        // SAFETY: the wrapped address passed the liveness heuristic; `+0x118`
        // holds the movement-record pointer.
        let mv = unsafe { *((self.0 + 0x118) as *const usize) };
        (mv != 0 && mv & 1 == 0).then_some(mv)
    }

    /// The unit's collision box height (movement `+0xb4`), zero if missing.
    pub fn collision_box_height(self) -> f32 {
        self.movement().map_or(0.0, |mv| {
            // SAFETY: `mv` passed the liveness heuristic; `+0xb4` is the
            // collision-box height.
            unsafe { *((mv + 0xb4) as *const f32) }
        })
    }

    /// The unit's facing in radians (movement `+0x1c`), zero if missing.
    pub fn facing(self) -> f32 {
        self.movement().map_or(0.0, |mv| {
            // SAFETY: `mv` passed the liveness heuristic; `+0x1c` is the
            // facing angle.
            unsafe { *((mv + 0x1c) as *const f32) }
        })
    }

    /// Whether the unit is moving or turning (movement flags at `+0x40`).
    pub fn is_moving(self) -> bool {
        self.movement().is_some_and(|mv| {
            // SAFETY: `mv` passed the liveness heuristic; `+0x40` is the
            // movement-flags field.
            let flags = unsafe { *((mv + 0x40) as *const u32) };
            flags & MOVING_OR_TURNING != 0
        })
    }

    /// The unit's creature type, through the client's own reader.
    pub fn creature_type(self) -> i32 {
        // SAFETY: a fixed `.text` entry in the live host image (base
        // verified at load); the transmuted signature matches the declared
        // prototype (`__fastcall(ecx = unit)`, register ret).
        let creature_type: extern "fastcall" fn(usize) -> i32 =
            unsafe { core::mem::transmute(CREATURE_TYPE_VA) };
        creature_type(self.0)
    }

    /// Whether `self` may attack `target`, through the client's own test.
    pub fn can_attack(self, target: Self) -> bool {
        // SAFETY: a fixed `.text` entry in the live host image; the
        // transmuted signature matches the declared prototype
        // (`__thiscall(ecx = self)` plus the target, callee-cleaned).
        let can_attack: extern "thiscall" fn(usize, usize) -> u8 =
            unsafe { core::mem::transmute(CAN_ATTACK_VA) };
        can_attack(self.0, target.0) != 0
    }
}

/// Resolve a GUID through the client's hash lookup.
pub fn object_by_guid(guid: u64) -> Option<UnitRef> {
    if guid == 0 {
        return None;
    }
    // SAFETY: a fixed `.text` entry in the live host image (base verified
    // at load); the transmuted signature matches the declared prototype
    // (`__fastcall`, the 8-byte GUID on the stack, callee-cleaned).
    let lookup: extern "fastcall" fn(u64) -> u32 =
        unsafe { core::mem::transmute(OBJECT_BY_GUID_VA) };
    UnitRef::try_from_raw(lookup(guid))
}

/// Resolve a unit token (`"player"`, `"target"`, …) to a GUID, zero for none.
pub fn guid_of_token(token: &core::ffi::CStr) -> u64 {
    // SAFETY: a fixed `.text` entry in the live host image; the transmuted
    // signature matches the declared prototype (`__fastcall(ecx = text)`,
    // GUID in `edx:eax`).
    let resolve: extern "fastcall" fn(*const u8) -> u64 =
        unsafe { core::mem::transmute(GUID_OF_TOKEN_VA) };
    resolve(token.as_ptr().cast())
}

/// The active player's GUID, straight from the client global, zero when none.
pub fn active_player_guid() -> u64 {
    // SAFETY: a fixed `.text` entry in the live host image; the transmuted
    // signature matches the declared prototype (no arguments, GUID in
    // `edx:eax`).
    let read: extern "fastcall" fn() -> u64 =
        unsafe { core::mem::transmute(ACTIVE_PLAYER_GUID_VA) };
    read()
}

/// The active player's object, when in world.
pub fn player() -> Option<UnitRef> {
    object_by_guid(active_player_guid())
}

/// Set the player's target to `guid`, through the client's own path.
pub fn target_by_guid(guid: u64) {
    let mut boxed = guid;
    // SAFETY: a fixed `.text` entry in the live host image; the transmuted
    // signature matches the declared prototype (`__fastcall(ecx = *guid)`,
    // no return).
    let target: extern "fastcall" fn(*mut u64) = unsafe { core::mem::transmute(TARGET_BY_GUID_VA) };
    target(&raw mut boxed);
}

/// The raid target-mark table: eight GUID slots.
const RAID_MARKS: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0077_1368;

/// The raid mark on `guid` (1-8), or -1 for none.
pub fn target_mark_of(guid: u64) -> i32 {
    if guid == 0 {
        return -1;
    }
    for slot in 0..8usize {
        // SAFETY: `RAID_MARKS` is a fixed host global table of eight GUIDs at
        // the verified image base (unaligned by the same contract as `guid`).
        let marked = unsafe { ((RAID_MARKS + slot * 8) as *const u64).read_unaligned() };
        if marked == guid {
            return i32::try_from(slot).expect("slot is 0..8") + 1;
        }
    }
    -1
}

/// The engine's millisecond tick, advanced once per frame.
pub fn game_tick_ms() -> u32 {
    // SAFETY: `GAME_TICK` is a fixed host global at the verified image base.
    unsafe { *(GAME_TICK as *const u32) }
}

/// Iterator over every object in the manager's intrusive list.
pub struct Objects {
    link_base: usize,
    current: usize,
}

impl Iterator for Objects {
    type Item = UnitRef;

    fn next(&mut self) -> Option<Self::Item> {
        let object = UnitRef::try_from_raw(u32::try_from(self.current).ok()?)?;
        // SAFETY: the link base and the live object address were both read
        // from the live manager; their sum plus 4 is this object's next link.
        let next = unsafe { *((self.link_base + object.raw() + 4) as *const u32) };
        self.current = next as usize;
        Some(object)
    }
}

/// Walk the object manager's list, empty when out of world.
pub fn objects() -> Objects {
    // SAFETY: `OBJECT_MANAGER` is a fixed host global at the verified image
    // base, zero out-of-world.
    let manager = unsafe { *(OBJECT_MANAGER as *const usize) };
    if manager == 0 || manager & 1 != 0 {
        return Objects {
            link_base: 0,
            current: 0,
        };
    }
    // SAFETY: `manager` passed the liveness heuristic; `+0xac` is the first
    // object of the intrusive list.
    let first = unsafe { *((manager + 0xac) as *const u32) };
    // SAFETY: `+0xa4` is the base the per-object next links are relative to.
    let link_base = unsafe { *((manager + 0xa4) as *const usize) };
    Objects {
        link_base,
        current: first as usize,
    }
}
