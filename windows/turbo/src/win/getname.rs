//! The `GetName` script method (`0x7a1390`), reimplemented with a memo.
//!
//! Every frame class shares this method, which makes it one of the hottest
//! entries in the script API. It resolves the C++ object out of the frame
//! table, checks its type, and pushes the object's name as a Lua string. The
//! expensive part is not the lookup but the push: interning a string walks the
//! host's string table on every call, and the name of a given frame never
//! changes, so the result is a pure function of state a cheap guard re-checks.
//!
//! A mod that replaces this method wholesale adds a second signature to it:
//! with a truthy second argument it returns the owning unit's GUID, read from
//! the object at `+0x4e8` and formatted as `0x` followed by sixteen uppercase
//! hex digits. Nameplate addons call that once per plate per frame to resolve
//! a unit, so it is the shape that dominates in a raid — and the formatting
//! plus interning is most of what it costs. Both behaviours are reproduced
//! here, so this adapter stands in for either prologue rather than deferring
//! to one, and the GUID string is memoized by its VALUE: `guid -> text` is a
//! pure function, so a plate recycled onto a unit seen before hits with no
//! guard at all.
//!
//! What it will not do is guess. The entry's byte signature admits any
//! five-byte jump, which does not say whose jump it is, so the module owning
//! the displaced code is identified before install and only a prologue that is
//! stock, or a replacement whose body has been read, enables the fast path.
//! The GUID half is served whenever a replacement that answers it is
//! installed, whichever of the two it is; a machine with neither keeps the
//! stock reading, so a script probing for the extension cannot detect a
//! server that is not there. Anything else runs the displaced code unchanged,
//! as do the argument shapes this does not model (the error paths, where the
//! host owns the message).
//!
//! Pushes reproduce the host's own: the taint propagation of the elided
//! `lua_rawgeti(L, 1, 0)`, and — on a memo hit, which calls nothing — the
//! GC-threshold check `lua_pushlstring` runs before touching the stack, so GC
//! pacing does not shift. No diff table is possible: both sides push onto the
//! live Lua stack, so running them together would push the result twice. The
//! armed hit/miss counters and in-game observation are the verification.

use core::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};

/// RVA of the `GetName` script method inside the host image.
const GET_NAME_RVA: usize = 0x003a_1390;

/// In-module offsets of the replacement handlers whose bodies have been read.
///
/// Verified against this reimplementation: the GUID source at object `+0x4e8`,
/// its uppercase `0x%016X` rendering, and the optional-argument classification.
const VERIFIED_HANDLER_OFFSETS: [usize; 2] = [0x3da0, 0x36a0];

/// Module those offsets belong to.
const VERIFIED_HANDLER_MODULE: &str = "SuperWoWhook.dll";

/// A module that installs its own handler after this one is already live.
///
/// Its handler is a full replacement, read end to end: the name half is the
/// stock sequence, and the optional second argument selects the same GUID at
/// object `+0x4e8` rendered by the same uppercase `0x%016X`. It writes no
/// globals and keeps no per-call state, so nothing is lost by not running it.
///
/// It classifies that argument with a numeric conversion where the other
/// replacement takes the broader optional-boolean reading, so the two disagree
/// for a boolean argument. The broader reading is the documented one, and it
/// is what this reimplementation serves wherever either replacement is
/// installed; the numeric shape scripts actually pass classifies identically
/// under both.
const LATE_HANDLER_MODULE: &str = "nampower.dll";

/// In-module offset of [`LATE_HANDLER_MODULE`]'s handler.
const LATE_HANDLER_OFFSET: usize = 0x7_2990;

/// Opening bytes of that handler: `sub esp,8` then the four register saves.
///
/// Checked before the entry is reclaimed, because the offset alone would name
/// whatever a different build of that module happens to put there, and what
/// this reimplementation stands in for is the body that was read, not an
/// address.
const LATE_HANDLER_PROLOGUE: [u8; 6] = [0x83, 0xec, 0x08, 0x53, 0x55, 0x56];

/// Type token the frame classes' `IsA` predicate is queried with.
///
/// Allocated lazily out of a shared counter by whichever script method runs
/// first, which is why every one of them opens by materializing it.
const TYPE_TOKEN: usize = crate::win::EXPECTED_IMAGE_BASE + 0x008f_0c3c;
/// Counter [`TYPE_TOKEN`] takes its value from.
const TYPE_TOKEN_SEQ: usize = crate::win::EXPECTED_IMAGE_BASE + 0x008e_ef6c;

/// Current taint owner (`0xceeac0`) and taint-enabled flag (`0xceeac4`).
///
/// The client widens every `TObject` with a taint word at `+0x4`: copying a
/// value with a non-zero taint word publishes it to the current owner (when
/// enabled), and a fresh push stamps the current owner into the new slot.
const TAINT_CUR: usize = crate::win::EXPECTED_IMAGE_BASE + 0x008e_eac0;
/// See [`TAINT_CUR`].
const TAINT_ON: usize = crate::win::EXPECTED_IMAGE_BASE + 0x008e_eac4;

/// The client's global `lua_State*`, zero between states.
const GLOBAL_STATE: usize = crate::win::EXPECTED_IMAGE_BASE + 0x008e_ef74;

/// `lua_pushlstring` — `fastcall(ecx = L, edx = s) + stack(len)`, `RET 4`.
const LUA_PUSHLSTRING_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x002f_3840;
/// `lua_pushnil` — `fastcall(ecx = L)`.
const LUA_PUSHNIL_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x002f_37f0;
/// `luaL_ref` — `fastcall(ecx = L, edx = t)`, pops the value at the top.
const LUA_L_REF_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x002f_5310;
/// `luaL_unref` — `fastcall(ecx = L, edx = t) + stack(ref)`, `RET 4`.
const LUA_L_UNREF_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x002f_5400;
/// The registry pseudo-index both ref calls use.
const LUA_REGISTRYINDEX: i32 = -10_000;

/// Run the displaced code for every call: the prologue's owner is unknown.
const MODE_DELEGATE: u8 = 0;
/// No GUID server installed: the name behaviour only, extra arguments ignored.
const MODE_STOCK: u8 = 1;
/// A GUID server whose body has been read is installed: name and GUID.
const MODE_EXTENDED: u8 = 2;

/// Which behaviour [`detect_underlying`] established, decided before install.
static MODE: AtomicU8 = AtomicU8::new(MODE_DELEGATE);

const SETS: usize = 256;
const WAYS: usize = 4;

/// One memo way: a key and the anchored string it resolves to.
///
/// Written only from the game thread (the one thread that runs script
/// methods); the atomics provide shared mutability for a `static`, not
/// cross-thread ordering. `key == 0` marks an empty way, and `key` is written
/// last on insert so a partially-written way is never live.
struct Way {
    /// Object pointer, or the GUID itself, depending on the store.
    key: AtomicU64,
    /// Interned `TString*` in the low half, its registry ref above it.
    value: AtomicU64,
}

impl Way {
    const fn empty() -> Self {
        Self {
            key: AtomicU64::new(0),
            value: AtomicU64::new(0),
        }
    }
}

/// A set-associative store of anchored strings.
struct Memo {
    ways: [[Way; WAYS]; SETS],
    /// Round-robin eviction cursor; a full set evicts `cursor % WAYS`.
    cursor: AtomicU32,
}

impl Memo {
    const fn new() -> Self {
        Self {
            ways: [const { [const { Way::empty() }; WAYS] }; SETS],
            cursor: AtomicU32::new(0),
        }
    }

    /// The set a key belongs to, mixed from the whole key.
    const fn set_of(key: u64) -> usize {
        let mixed = (key ^ (key >> 29)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        ((mixed >> 33) as usize) & (SETS - 1)
    }

    /// The `TString*` recorded for `key`, if any.
    fn get(&self, key: u64) -> Option<usize> {
        self.ways[Self::set_of(key)].iter().find_map(|way| {
            (way.key.load(Ordering::Relaxed) == key).then(|| {
                let value = way.value.load(Ordering::Relaxed);
                usize::try_from(value & 0xffff_ffff).expect("masked to 32 bits")
            })
        })
    }

    /// Record `key -> (ts, anchored)`, releasing whatever it displaces.
    fn put(&self, key: u64, ts: usize, anchored: u32) {
        let set = &self.ways[Self::set_of(key)];
        let chosen = set
            .iter()
            .find(|way| matches!(way.key.load(Ordering::Relaxed), k if k == key || k == 0));
        let way = chosen.unwrap_or_else(|| {
            let cursor = self.cursor.load(Ordering::Relaxed);
            self.cursor.store(cursor.wrapping_add(1), Ordering::Relaxed);
            bump(&EVICTIONS);
            &set[cursor as usize % WAYS]
        });
        let prior = way.key.swap(0, Ordering::Relaxed);
        if prior != 0 {
            // Every anchor this way stops pointing at has to go back, or it
            // holds a registry slot for the life of the state. That covers
            // re-recording the same key: the replacement anchor is a
            // different slot even when the key is unchanged.
            let victim = way.value.load(Ordering::Relaxed);
            release_ref((victim >> 32) as u32);
        }
        way.value
            .store(ts as u64 | u64::from(anchored) << 32, Ordering::Relaxed);
        way.key.store(key, Ordering::Relaxed);
    }

    /// Forget everything, without unref-ing: the state itself is going away.
    fn clear(&self) {
        for set in &self.ways {
            for way in set {
                way.key.store(0, Ordering::Relaxed);
            }
        }
    }
}

/// Frame names, keyed by object and guarded by the live name bytes.
static NAMES: Memo = Memo::new();

/// GUID renderings, keyed by the GUID itself — a pure function, no guard.
static GUIDS: Memo = Memo::new();

/// Class vtables whose `IsA` predicate has already answered yes.
///
/// A vtable is a fixed address in the host image, so an entry can never go
/// stale within a run; the array is a bound, not a cache policy.
static ISA_OK: [AtomicU32; 64] = [const { AtomicU32::new(0) }; 64];

static NAME_HITS: AtomicU32 = AtomicU32::new(0);
static NAME_MISSES: AtomicU32 = AtomicU32::new(0);
static GUID_HITS: AtomicU32 = AtomicU32::new(0);
static GUID_MISSES: AtomicU32 = AtomicU32::new(0);
static DELEGATED: AtomicU32 = AtomicU32::new(0);
static EVICTIONS: AtomicU32 = AtomicU32::new(0);

/// Single-writer counter bump (armed runs only), load-add-store on purpose.
///
/// The game thread is the only writer, so a read-modify-write is exact, and a
/// `fetch_add` would be a `lock`-prefixed RMW on i686.
fn bump(counter: &AtomicU32) {
    if super::events::armed() {
        counter.store(
            counter.load(Ordering::Relaxed).wrapping_add(1),
            Ordering::Relaxed,
        );
    }
}

/// Establish which behaviour this entry's prologue has, before install.
///
/// Must run before `install_all`: afterwards the bytes at the entry are this
/// mod's own detour and the displaced owner can no longer be read. The entry's
/// signature accepts any five-byte jump, which is deliberately not enough to
/// identify a replacement — this is what identifies it.
pub fn detect_underlying(image_base: usize) {
    let va = image_base + GET_NAME_RVA;
    let Some(target) = wow_hook::detour_target(va) else {
        // The late module leaves this prologue stock until in-world init, so
        // at install time its presence, not the entry's bytes, is what says
        // this machine has the GUID reading.
        if late_handler_installed() {
            log::info!(
                target: super::LOG_TARGET,
                "getname: stock prologue with {LATE_HANDLER_MODULE} installed, name and GUID",
            );
            MODE.store(MODE_EXTENDED, Ordering::Relaxed);
        } else {
            log::info!(target: super::LOG_TARGET, "getname: stock handler, name only");
            MODE.store(MODE_STOCK, Ordering::Relaxed);
        }
        return;
    };
    let owner = wow_hook::module_of(target);
    let verified = owner.as_ref().is_some_and(|&(ref name, base)| {
        name.eq_ignore_ascii_case(VERIFIED_HANDLER_MODULE)
            && VERIFIED_HANDLER_OFFSETS.contains(&target.wrapping_sub(base))
    });
    let owner = owner.map_or_else(
        || String::from("an unnamed module"),
        |(name, base)| format!("{name}+{:#x}", target.wrapping_sub(base)),
    );
    if verified {
        log::info!(
            target: super::LOG_TARGET,
            "getname: standing in for {owner}, name and GUID",
        );
    } else {
        log::info!(
            target: super::LOG_TARGET,
            "getname: unrecognized handler {owner}, running it unchanged",
        );
    }
    MODE.store(
        if verified {
            MODE_EXTENDED
        } else {
            MODE_DELEGATE
        },
        Ordering::Relaxed,
    );
}

/// Whether [`LATE_HANDLER_MODULE`] is loaded with the handler body that was read.
fn late_handler_installed() -> bool {
    let Some(base) = wow_hook::module_base(LATE_HANDLER_MODULE) else {
        return false;
    };
    let handler = base + LATE_HANDLER_OFFSET;
    // SAFETY: `handler` is an offset into a loaded module's image, and the
    // read stays inside the body whose first instructions these are.
    let seen: [u8; 6] = unsafe { *(handler as *const [u8; 6]) };
    seen == LATE_HANDLER_PROLOGUE
}

/// Whether the handler whose body was read is what the entry now runs.
///
/// The prologue does not point at that handler: it points at a thunk the
/// other module generates in private memory, which belongs to no module, sits
/// somewhere different every run, and opens with padding and a register save
/// rather than a jump — so there is nothing to follow, and the handler's
/// address is nowhere in it. What is checked is therefore what can be
/// checked: the module is loaded, and the bytes at the offset are the body
/// that was read rather than whatever a different build puts there.
///
/// That leaves one step unproven — that this thunk belongs to that install
/// rather than to some third party. It is accepted rather than proven,
/// because the module's own image carries the entry's address beside the
/// handler's, so what it does with this function is not in doubt; because it
/// is the only loaded module besides the one displaced at attach whose image
/// mentions the entry at all; and because reclaiming loses nothing either
/// way: both behaviours it served are served here, and the shapes this
/// reimplementation does not model go to the displaced code.
fn late_handler_owns(thunk: usize) -> bool {
    if wow_hook::module_base(LATE_HANDLER_MODULE).is_none() {
        log::info!(
            target: super::LOG_TARGET,
            "getname: the entry went to unnamed memory at {thunk:#010x} \
             [{}] and {LATE_HANDLER_MODULE} is not loaded — leaving it",
            wow_hook::thunk_bytes(thunk),
        );
        return false;
    }
    if !late_handler_installed() {
        log::info!(
            target: super::LOG_TARGET,
            "getname: {LATE_HANDLER_MODULE}+{LATE_HANDLER_OFFSET:#x} is not the body \
             that was read — leaving the entry alone",
        );
        return false;
    }
    log::info!(
        target: super::LOG_TARGET,
        "getname: the entry went to a thunk at {thunk:#010x} [{}] and \
         {LATE_HANDLER_MODULE}+{LATE_HANDLER_OFFSET:#x} is loaded and unchanged",
        wow_hook::thunk_bytes(thunk),
    );
    true
}

/// Decide whether to reclaim the entry from whoever overwrote the prologue.
///
/// Called once, from the periodic prologue check, with the address the
/// rewritten entry now jumps to. The new owner must be the handler whose body
/// has been read, so that what stops running is understood rather than merely
/// displaced; both of its behaviours are then served here, faster. The one
/// mode that refuses is [`MODE_DELEGATE`]: there the displaced code is a
/// handler this reimplementation does not model, and re-asserting the hook
/// would keep that handler reachable only for the shapes that delegate while
/// swallowing the rest.
fn reclaim_entry(owner_va: usize) -> bool {
    // The prologue points at a thunk in private memory, not at the handler,
    // and that allocation sits somewhere different every run — so ask where
    // the jumps end up rather than where the first one lands.
    if !late_handler_owns(owner_va) {
        return false;
    }
    if MODE.load(Ordering::Relaxed) == MODE_DELEGATE {
        log::info!(
            target: super::LOG_TARGET,
            "getname: an unmodelled handler is underneath, \
             leaving the entry to {LATE_HANDLER_MODULE}",
        );
        return false;
    }
    // The new owner's GUID reading is served here from now on. Its numeric
    // argument dialect differs from the optional-boolean one only for a
    // boolean argument, which is now answered under the documented reading.
    MODE.store(MODE_EXTENDED, Ordering::Relaxed);
    log::info!(
        target: super::LOG_TARGET,
        "getname: reclaiming the entry from {LATE_HANDLER_MODULE}, name and GUID",
    );
    true
}

/// Register the reclaim policy for the periodic prologue check.
pub fn arm_reclaim(image_base: usize) {
    wow_hook::on_overwrite(image_base + GET_NAME_RVA, reclaim_entry);
}

/// The `GetName` reimplementation; `fastcall(ecx = L)`, returns result count.
pub fn get_name(l: i32) -> i32 {
    let mode = MODE.load(Ordering::Relaxed);
    if let Some(pushed) = (mode != MODE_DELEGATE).then(|| resolve(l, mode)).flatten() {
        return pushed;
    }
    // Every shape not modelled above, and every unrecognized prologue: run the
    // code this hook displaced, which owns the error messages.
    bump(&DELEGATED);
    super::symbols::originals::script_get_name__7a1390()(l)
}

/// Push the name or GUID for this call, or `None` to run the displaced code.
fn resolve(l: i32, mode: u8) -> Option<i32> {
    let ls = l.cast_unsigned() as usize;
    // SAFETY: `l` is the live `lua_State` the host dispatched with; `+0x8` is
    // its stack top pointer.
    let top = unsafe { *((ls + 0x8) as *const usize) };
    // SAFETY: `+0xc` is the state's stack base pointer (argument 1).
    let base = unsafe { *((ls + 0xc) as *const usize) };
    let argc = top.wrapping_sub(base) >> 4;
    if argc < 1 {
        return None;
    }
    // SAFETY: `base` addresses argument 1's 16-byte slot; `+0x0` is its tag.
    if unsafe { *(base as *const i32) } != 5 {
        // Not a table: the displaced code raises the usage error.
        return None;
    }
    let want_guid = match mode {
        // Stock reads nothing past the frame argument.
        MODE_STOCK => false,
        _ => classify(base, argc)?,
    };
    let token = type_token();
    // SAFETY: a tag-5 slot's payload at `+0x8` is the `Table*`.
    let table = unsafe { *((base + 0x8) as *const usize) };
    let obj = object_of(table)?;
    // SAFETY: `obj` is a live object; its first word is its vtable pointer.
    let vtbl = unsafe { *(obj as *const usize) };
    if !is_a(obj, vtbl, token) {
        // Wrong object type: the displaced code raises that error too.
        return None;
    }
    if want_guid {
        push_guid(l, obj);
    } else {
        push_name(l, obj, vtbl);
    }
    Some(1)
}

/// Materialize the shared type token, allocating it on first use.
fn type_token() -> u32 {
    // SAFETY: `TYPE_TOKEN` is a fixed host global at the verified image base.
    let token = unsafe { *(TYPE_TOKEN as *const u32) };
    if token != 0 {
        return token;
    }
    // SAFETY: `TYPE_TOKEN_SEQ` is the counter tokens are drawn from.
    let next = unsafe { *(TYPE_TOKEN_SEQ as *const u32) }.wrapping_add(1);
    // SAFETY: publishing the drawn value, as every script method's prologue
    // does when it finds the token unset.
    unsafe { *(TYPE_TOKEN_SEQ as *mut u32) = next };
    // SAFETY: the token global, now claimed for the frame classes.
    unsafe { *(TYPE_TOKEN as *mut u32) = next };
    next
}

/// Resolve `t[0]`, the C++ object behind a frame table.
///
/// Reproduces the persistent half of the `lua_rawgeti(L, 1, 0)` this elides:
/// reading a slot whose taint word is set publishes that word to the current
/// owner while tainting is enabled. Its pushed copy dies with the `settop`
/// that followed, so nothing else of it survives.
fn object_of(table: usize) -> Option<usize> {
    let slot = super::hooks::lua_h_getnum__6fa700(table as *mut u8, 0) as usize;
    // SAFETY: `luaH_getnum` returns a live `TObject*` (the shared nil object
    // on a miss); `+0x0` is its tag, `2` being light userdata.
    if unsafe { *(slot as *const i32) } != 2 {
        return None;
    }
    // SAFETY: `+0x4` of a live `TObject` is its taint word.
    let taint = unsafe { *((slot + 0x4) as *const u32) };
    if taint != 0 {
        // SAFETY: `TAINT_ON` is the host's taint-enabled flag.
        if unsafe { *(TAINT_ON as *const u32) } != 0 {
            // SAFETY: `TAINT_CUR` is the current-owner word; writing it is
            // exactly what the elided read would have done.
            unsafe { *(TAINT_CUR as *mut u32) = taint };
        }
    }
    // SAFETY: a tag-2 slot's payload at `+0x8` is the object pointer.
    let obj = unsafe { *((slot + 0x8) as *const usize) };
    (obj != 0).then_some(obj)
}

/// Whether this object's class answers the frame-object type predicate.
///
/// The answer is a property of the class, so a vtable that has answered yes is
/// remembered and the virtual call skipped. A class the table has no room for
/// simply pays the call every time.
fn is_a(obj: usize, vtbl: usize, token: u32) -> bool {
    let slot_value = u32::try_from(vtbl).expect("32-bit image pointer");
    let mut free: Option<&AtomicU32> = None;
    for entry in &ISA_OK {
        match entry.load(Ordering::Relaxed) {
            0 => {
                free = Some(entry);
                break;
            }
            known if known == slot_value => return true,
            _ => {}
        }
    }
    // SAFETY: a frame class's vtable holds its type predicate at `+0x10`,
    // taking the object in `ecx` and the token on the stack.
    let predicate = unsafe { *((vtbl + 0x10) as *const usize) };
    // SAFETY: a fixed `.rdata` vtable entry of the live host image; the
    // transmuted signature matches the declared prototype
    // (`__thiscall(ecx = this)` plus the token, byte `ret`).
    let is_a: extern "thiscall" fn(usize, u32) -> u8 = unsafe { core::mem::transmute(predicate) };
    if is_a(obj, token) == 0 {
        return false;
    }
    if let Some(entry) = free {
        entry.store(slot_value, Ordering::Relaxed);
    }
    true
}

/// Map the optional second argument to "GUID wanted", or `None` to delegate.
///
/// Mirrors the host's optional-boolean helper (`0x6f1c10`) for the argument
/// shapes it can prove — absent, `nil`, boolean, number (which that helper
/// truncates through `__ftol`) — where the default the replacement passes is
/// `0`. The rest delegate: a string is matched against `on`/`off` wording,
/// and the remaining tags fall through to that default by a path not worth
/// reproducing for how rarely a script takes it.
fn classify(base: usize, argc: usize) -> Option<bool> {
    if argc < 2 {
        return Some(false);
    }
    let opt = base + 0x10;
    // SAFETY: argument 2 exists (`argc >= 2`); `+0x0` is its tag.
    let opt_tag = unsafe { *(opt as *const i32) };
    match opt_tag {
        0 => Some(false),
        // SAFETY: a boolean's payload is the word at `+0x8`.
        1 => Some(unsafe { *((opt + 0x8) as *const u32) } != 0),
        3 => {
            // SAFETY: a number's payload is the f64 at `+0x8`; a stack slot is
            // not guaranteed 8-aligned, so this reads unaligned.
            let v = unsafe { ((opt + 0x8) as *const f64).read_unaligned() };
            Some(crate::math::misc::ftol__40a2b0(v) != 0)
        }
        _ => None,
    }
}

/// Push the frame's name, memoized per object.
fn push_name(l: i32, obj: usize, vtbl: usize) {
    // SAFETY: a frame class's vtable holds its name getter at `+0x4`, taking
    // the object in `ecx`.
    let getter = unsafe { *((vtbl + 0x4) as *const usize) };
    // SAFETY: a fixed `.rdata` vtable entry of the live host image; the
    // transmuted signature matches the declared prototype
    // (`__thiscall(ecx = this)`, register `ret`).
    let name_of: extern "thiscall" fn(usize) -> *const u8 = unsafe { core::mem::transmute(getter) };
    let name = name_of(obj);
    // SAFETY: the getter returns this object's name buffer or null.
    if name.is_null() || unsafe { *name } == 0 {
        // An unnamed frame yields nil, exactly as the host does.
        // SAFETY: a fixed `.text` entry in the live host image (base verified
        // at load); the transmuted signature matches the declared prototype
        // (`__fastcall(ecx = L)`, no return).
        let push_nil: extern "fastcall" fn(i32) = unsafe { core::mem::transmute(LUA_PUSHNIL_VA) };
        push_nil(l);
        return;
    }
    let key = name.addr() as u64;
    if let Some(ts) = NAMES.get(key).filter(|&ts| name_matches(name, ts)) {
        push_cached(l, ts);
        bump(&NAME_HITS);
        return;
    }
    bump(&NAME_MISSES);
    let len = strlen(name);
    push_lstring(l, name, len);
    if let Some((ts, anchored)) = anchor(l) {
        NAMES.put(key, ts, anchored);
    }
}

/// Push the object's unit GUID as text, memoized by the GUID itself.
fn push_guid(l: i32, obj: usize) {
    // SAFETY: `obj` passed the frame-object type predicate, and the handler
    // this stands in for reads its GUID from `+0x4e8..+0x4f0`.
    let guid = unsafe { ((obj + 0x4e8) as *const u64).read_unaligned() };
    if let Some(ts) = GUIDS.get(guid) {
        push_cached(l, ts);
        bump(&GUID_HITS);
        return;
    }
    bump(&GUID_MISSES);
    let text = render_guid(guid);
    push_lstring(l, text.as_ptr(), text.len());
    // A zero GUID — an object with no unit attached — is the one value that
    // cannot be stored, since zero is what marks a way empty. It renders and
    // pushes normally; only the memo skips it, and anchoring it anyway would
    // hold a registry slot that nothing could ever release.
    if guid != 0
        && let Some((ts, anchored)) = anchor(l)
    {
        GUIDS.put(guid, ts, anchored);
    }
}

/// Render a GUID the way the handler this stands in for does.
///
/// `0x` and then all sixteen hex digits, most significant first, in UPPERCASE
/// — scripts use these strings as table keys and compare them to each other,
/// so the alphabet and the fixed width are part of the behaviour, not a
/// presentation choice.
fn render_guid(guid: u64) -> [u8; 18] {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = [0u8; 18];
    out[0] = b'0';
    out[1] = b'x';
    for (i, slot) in out[2..].iter_mut().enumerate() {
        let shift = 60 - i * 4;
        *slot = DIGITS[((guid >> shift) & 0xf) as usize];
    }
    out
}

/// Whether the live name bytes still equal the memoized string.
///
/// Compared one byte at a time, stopping at the first difference, because the
/// live buffer's length is not known up front: a recycled object can hold a
/// shorter name, and a wide read of the cached length could run off the end of
/// its allocation. A mismatch is found at or before that end, since the live
/// name's terminator differs from any cached byte there.
fn name_matches(name: *const u8, ts: usize) -> bool {
    // SAFETY: a `TString`'s length lives at `+0xc`.
    let len = unsafe { *((ts + 0xc) as *const usize) };
    for i in 0..=len {
        // SAFETY: `ts + 0x10` is the interned string's byte array, `len + 1`
        // long including its terminator (the registry ref keeps it alive).
        let want = unsafe { *((ts + 0x10 + i) as *const u8) };
        // SAFETY: every byte before `i` matched, so byte `i` is still within
        // the object's name buffer (its terminator at worst).
        if unsafe { name.wrapping_byte_add(i).read() } != want {
            return false;
        }
    }
    true
}

/// Length of a host NUL-terminated string.
const fn strlen(s: *const u8) -> usize {
    let mut n = 0;
    // SAFETY: `s` is a NUL-terminated buffer owned by the host object, so a
    // terminator is reached before the end of its allocation.
    while unsafe { s.wrapping_byte_add(n).read() } != 0 {
        n += 1;
    }
    n
}

/// Intern and push a string through the host, exactly as the original does.
fn push_lstring(l: i32, s: *const u8, len: usize) {
    // SAFETY: a fixed `.text` entry in the live host image (base verified at
    // load); the transmuted signature matches the declared prototype
    // (`__fastcall(ecx = L, edx = s)` plus the length on the stack, `RET 4`).
    let push: extern "fastcall" fn(i32, *const u8, u32) =
        unsafe { core::mem::transmute(LUA_PUSHLSTRING_VA) };
    push(
        l,
        s,
        u32::try_from(len).expect("a host name is far below 4 GiB"),
    );
}

/// Anchor the string just pushed, leaving it in place for the caller.
///
/// The value is duplicated first so `luaL_ref`'s pop consumes the copy, and
/// the copy carries the host's value-copy taint rule: a set taint word is
/// published to the current owner (when enabled) and travels with the copy, an
/// unset one takes the current owner instead.
fn anchor(l: i32) -> Option<(usize, u32)> {
    let ls = l.cast_unsigned() as usize;
    // SAFETY: `+0x8` is the state's stack top pointer, just advanced past the
    // value that was pushed.
    let top = unsafe { *((ls + 0x8) as *const usize) };
    let result = top - 0x10;
    // SAFETY: `+0x0` of the result slot is its tag; `4` is a string.
    if unsafe { *(result as *const i32) } != 4 {
        return None;
    }
    // SAFETY: a tag-4 slot's payload at `+0x8` is the `TString*`.
    let ts = unsafe { *((result + 0x8) as *const usize) };
    // SAFETY: `+0x4` of the result slot is its taint word.
    let taint = unsafe { *((result + 0x4) as *const u32) };
    // SAFETY: `top` is the next free slot; tag first.
    unsafe { *(top as *mut u32) = 4 };
    if taint == 0 {
        // SAFETY: `TAINT_CUR` is the current-owner word a fresh value takes.
        let owner = unsafe { *(TAINT_CUR as *const u32) };
        // SAFETY: `+0x4` of the duplicate slot.
        unsafe { *((top + 0x4) as *mut u32) = owner };
    } else {
        // SAFETY: `TAINT_ON` gates publishing a set taint word.
        if unsafe { *(TAINT_ON as *const u32) } != 0 {
            // SAFETY: publishing the copied value's taint word, as every
            // host-side value copy does.
            unsafe { *(TAINT_CUR as *mut u32) = taint };
        }
        // SAFETY: `+0x4` of the duplicate slot carries the copied word.
        unsafe { *((top + 0x4) as *mut u32) = taint };
    }
    // SAFETY: `+0x8` of the duplicate slot takes the payload.
    unsafe { *((top + 0x8) as *mut usize) = ts };
    // SAFETY: publish the duplicate by advancing the top.
    unsafe { *((ls + 0x8) as *mut usize) = top + 0x10 };
    // SAFETY: a fixed `.text` entry in the live host image (base verified at
    // load); the transmuted signature matches the declared prototype
    // (`__fastcall(ecx = L, edx = t)`, register `ret`).
    let lua_l_ref: extern "fastcall" fn(i32, i32) -> i32 =
        unsafe { core::mem::transmute(LUA_L_REF_VA) };
    let anchored = lua_l_ref(l, LUA_REGISTRYINDEX);
    (anchored > 0).then(|| (ts, anchored.cast_unsigned()))
}

/// Push an interned string the way `lua_pushlstring` (`0x6f3840`) does.
///
/// The memoized path calls nothing, so the GC-threshold check that function
/// opens with is transcribed: eliding it would move the collector's pacing by
/// however many pushes the memo absorbs.
fn push_cached(l: i32, ts: usize) {
    let ls = l.cast_unsigned() as usize;
    // SAFETY: `+0x10` of the state is its `global_State*`.
    let g = unsafe { *((ls + 0x10) as *const usize) };
    // SAFETY: `G + 0x28` is the live-byte count, `G + 0x24` the GC trigger.
    let nblocks = unsafe { *((g + 0x28) as *const u32) };
    // SAFETY: as above, the trigger this is compared against.
    let threshold = unsafe { *((g + 0x24) as *const u32) };
    if nblocks >= threshold {
        super::hooks::lua_c_collectgarbage__6f7340(l);
    }
    // Read the top only after a possible collect, which can move the stack —
    // the order the original reads them in.
    // SAFETY: `+0x8` is the state's stack top pointer.
    let top = unsafe { *((ls + 0x8) as *const usize) };
    // SAFETY: `top` addresses the next free 16-byte slot (a script method has
    // headroom above its arguments); tag 4 marks a string.
    unsafe { *(top as *mut u32) = 4 };
    // SAFETY: `TAINT_CUR` is the current-owner word a fresh push stamps into
    // the new slot's taint word at `+0x4`.
    let taint = unsafe { *(TAINT_CUR as *const u32) };
    // SAFETY: `+0x4` of the slot being pushed.
    unsafe { *((top + 0x4) as *mut u32) = taint };
    // SAFETY: `+0x8` takes the payload (the original writes these same three
    // words and leaves `+0xc` untouched).
    unsafe { *((top + 0x8) as *mut usize) = ts };
    // SAFETY: publishing the slot by advancing the top, as the original does.
    unsafe { *((ls + 0x8) as *mut usize) = top + 0x10 };
}

/// Drop a registry anchor this memo holds.
fn release_ref(anchor: u32) {
    if anchor == 0 {
        return;
    }
    // SAFETY: `GLOBAL_STATE` holds the host's global `lua_State*`, the state
    // every script method runs on; zero between states.
    let ls = unsafe { *(GLOBAL_STATE as *const i32) };
    if ls == 0 {
        return;
    }
    // SAFETY: a fixed `.text` entry in the live host image (base verified at
    // load); the transmuted signature matches the declared prototype
    // (`__fastcall(ecx = L, edx = t)` plus the ref on the stack, `RET 4`).
    let lua_l_unref: extern "fastcall" fn(i32, i32, i32) =
        unsafe { core::mem::transmute(LUA_L_UNREF_VA) };
    lua_l_unref(ls, LUA_REGISTRYINDEX, anchor.cast_signed());
}

/// Drop both memos wholesale, without unref-ing.
///
/// Called from the `FrameScript_Shutdown` adapter before the stock teardown
/// runs `lua_close`: the registry refs index the state being closed, so they
/// die with it (never unref-ed one by one into a state mid-teardown), and a
/// recycled allocation for the next state cannot inherit a stale entry.
pub fn forget() {
    NAMES.clear();
    GUIDS.clear();
}

/// One cumulative counters line on the events gauge's 60-second cadence.
pub fn emit_cumulative() {
    let name_hits = NAME_HITS.load(Ordering::Relaxed);
    let name_misses = NAME_MISSES.load(Ordering::Relaxed);
    let guid_hits = GUID_HITS.load(Ordering::Relaxed);
    let guid_misses = GUID_MISSES.load(Ordering::Relaxed);
    let delegated = DELEGATED.load(Ordering::Relaxed);
    let evictions = EVICTIONS.load(Ordering::Relaxed);
    log::debug!(
        target: "wow::events",
        "getname: name {name_hits} hits / {name_misses} misses, \
         guid {guid_hits} / {guid_misses}, delegated {delegated}, evict {evictions}",
    );
}
