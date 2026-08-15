//! The frame metatable's `__index` (`0x7020b0`), reimplemented with a memo.
//!
//! Every frame object shares one metatable, so every `frame:Method()` in every
//! addon resolves its method through this metamethod, which makes it the
//! hottest entry in the script API. Stock resolves from scratch on every call:
//! the method is never written back into the frame's table, so each call
//! re-fetches the C++ object out of the frame table and walks the object's
//! class resolver chain, hashing the whole method name once per class until a
//! table answers. The answer is a pure function of (class vtable, interned
//! name): every class's method table is built once at startup, and each
//! resolver consults a static table, forwarding the object only to its
//! parent's resolver, which ignores it too.
//!
//! The memo maps that pair to the class table's own registry ref, recorded by
//! the second reimplementation here, the per-class resolver (`0x702000`),
//! whenever a walk resolves. Memoizing the ref rather than the value bytes
//! (or a ref of our own) is what keeps a hit exact: `lua_rawgeti` on the
//! recorded ref is precisely what stock ends on, so the pushed value and its
//! taint word cannot diverge. A hit reproduces that push with zero client
//! calls. The elided `lua_rawgeti(L, 1, 0)` has a persistent taint effect,
//! which is transcribed; nothing else in the elided chain touches the GC
//! clock for a string-typed key (`lua_tostring` only checks it when it has to
//! convert), so a hit shifts no GC pacing. A name found in no class table is
//! memoized as ref 0 and answered with the nil push stock ends on, but only
//! when the walk proved it: the reimplemented resolver ran, no level was an
//! unbuilt table (that miss would outlive its reason), and the delegate
//! really pushed nil (a resolver level owned by another mod could answer
//! without touching the recorder).
//!
//! No diff table is possible: both sides push onto the live Lua stack, so
//! running them together would push the result twice. The armed hit/miss
//! counters and in-game observation are the verification.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use super::tally::Counter;

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

/// `SStrHashPath` — `stdcall(name)`, the hash the class tables were built with.
const SSTR_HASH_PATH_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0024_b3f0;
/// `Storm_SStrCmpN` — `stdcall(a, b, max)`, zero when equal.
const SSTR_CMP_N_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0024_a480;
/// `lua_rawgeti` — `fastcall(ecx = L, edx = idx) + stack(n)`, `RET 4`.
const LUA_RAWGETI_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x002f_3bc0;
/// `lua_pushnil` — `fastcall(ecx = L)`.
const LUA_PUSHNIL_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x002f_37f0;
/// `luaL_ref` — `fastcall(ecx = L, edx = t)`, pops the value at the top.
const LUA_L_REF_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x002f_5310;
/// `luaL_unref` — `fastcall(ecx = L, edx = t) + stack(ref)`, `RET 4`.
const LUA_L_UNREF_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x002f_5400;
/// The registry pseudo-index both ref calls use.
const LUA_REGISTRYINDEX: i32 = -10_000;

const SETS: usize = 256;
const WAYS: usize = 4;

/// One memo way: a key and the resolution it maps to.
///
/// Written only from the game thread (the one thread that runs script
/// methods); the atomics provide shared mutability for a `static`, not
/// cross-thread ordering. `key == 0` marks an empty way, and `key` is written
/// last on insert so a partially-written way is never live.
struct Way {
    /// Class vtable in the low half, the interned name above it.
    key: AtomicU64,
    /// Method registry ref in the low half (0 = no method), the name's anchor above it.
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

/// A set-associative store of recorded method resolutions.
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
    ///
    /// The whole key matters: one `frame:Method()` resolves the same name
    /// against every class in the parent chain, so an index blind to the
    /// vtable half would land a chain's every level in one set, and past
    /// `WAYS` levels a chain would then evict itself on each call.
    const fn set_of(key: u64) -> usize {
        let mixed = (key ^ (key >> 29)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        ((mixed >> 33) as usize) & (SETS - 1)
    }

    /// The value recorded for `key`, if any.
    fn get(&self, key: u64) -> Option<u64> {
        self.ways[Self::set_of(key)].iter().find_map(|way| {
            (way.key.load(Ordering::Relaxed) == key).then(|| way.value.load(Ordering::Relaxed))
        })
    }

    /// Record `key -> (method_ref, anchor)`, releasing whatever it displaces.
    fn put(&self, key: u64, method_ref: u32, anchor: u32) {
        let set = &self.ways[Self::set_of(key)];
        let chosen = set
            .iter()
            .find(|way| matches!(way.key.load(Ordering::Relaxed), k if k == key || k == 0));
        let way = chosen.unwrap_or_else(|| {
            let cursor = self.cursor.load(Ordering::Relaxed);
            self.cursor.store(cursor.wrapping_add(1), Ordering::Relaxed);
            super::tally::bump(&EVICTIONS);
            &set[cursor as usize % WAYS]
        });
        let prior = way.key.swap(0, Ordering::Relaxed);
        if prior != 0 {
            // Every name anchor this way stops pointing at has to go back, or
            // it holds a registry slot for the life of the state. That covers
            // re-recording the same key: the replacement anchor is a
            // different slot even when the key is unchanged. The method ref
            // is never released: the class table owns it.
            let victim = way.value.load(Ordering::Relaxed);
            release_ref((victim >> 32) as u32);
        }
        way.value.store(
            u64::from(method_ref) | u64::from(anchor) << 32,
            Ordering::Relaxed,
        );
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

/// Method resolutions, keyed by (class vtable, interned name).
static MEMO: Memo = Memo::new();

/// Registry ref the resolver recorded for the walk in flight, zero for none.
///
/// One slot is enough: the resolver's callees are leaf calls (the hash, the
/// compare, `lua_rawgeti`), so a second walk cannot start while one runs, and
/// the `__index` miss path clears the slot immediately before delegating and
/// consumes it immediately after.
static RECORDED_REF: AtomicU32 = AtomicU32::new(0);

/// Whether the walk in flight saw a class table that was not built yet.
static SAW_UNBUILT: AtomicU32 = AtomicU32::new(0);

/// Whether the reimplemented resolver ran at all during the walk in flight.
///
/// A resolver chain can answer without ever reaching the hook: a level owned
/// by another mod, or the `0x702000` entry itself refusing to patch. A "no
/// method" result may only be memoized when the chain demonstrably ran
/// through this code, or a name a foreign level resolves would be frozen to
/// nil.
static WALKED: AtomicU32 = AtomicU32::new(0);

static HITS: Counter = Counter::zero();
static NIL_HITS: Counter = Counter::zero();
static MISSES: Counter = Counter::zero();
static DELEGATED: Counter = Counter::zero();
static EVICTIONS: Counter = Counter::zero();

/// The `__index` reimplementation; `fastcall(ecx = L)`, returns result count.
pub fn index(l: i32) -> i32 {
    let Some(key) = method_key(l) else {
        // A shape not modelled here: run the displaced code, which owns it.
        super::tally::bump(&DELEGATED);
        return (super::symbols::originals::frame_script_meta_index__7020b0())(l);
    };
    if let Some(value) = MEMO.get(key) {
        let method_ref = u32::try_from(value & 0xffff_ffff).expect("masked to 32 bits");
        if method_ref == 0 {
            push_nil(l);
            super::tally::bump(&NIL_HITS);
        } else {
            push_registry(l, method_ref);
            super::tally::bump(&HITS);
        }
        return 1;
    }
    super::tally::bump(&MISSES);
    RECORDED_REF.store(0, Ordering::Relaxed);
    SAW_UNBUILT.store(0, Ordering::Relaxed);
    WALKED.store(0, Ordering::Relaxed);
    let pushed = (super::symbols::originals::frame_script_meta_index__7020b0())(l);
    let method_ref = RECORDED_REF.load(Ordering::Relaxed);
    if (method_ref != 0 || negative_provable(l))
        && let Some(anchor) = anchor_name(l)
    {
        MEMO.put(key, method_ref, anchor);
    }
    pushed
}

/// Whether "this name resolves to no method" was proved by the walk just run.
///
/// Three conditions, all required. The reimplemented resolver ran, so the
/// chain was not answered wholesale by code this module cannot see; no level
/// was an unbuilt table, whose miss would outlive its reason; and the
/// delegate really pushed nil, since a level between two reimplemented ones
/// (or below them) could have answered without touching the recorder.
fn negative_provable(l: i32) -> bool {
    if WALKED.load(Ordering::Relaxed) == 0 || SAW_UNBUILT.load(Ordering::Relaxed) != 0 {
        return false;
    }
    let ls = l.cast_unsigned() as usize;
    // SAFETY: `+0x8` is the state's stack top pointer, just advanced past the
    // delegate's result.
    let top = unsafe { *((ls + 0x8) as *const usize) };
    // SAFETY: `+0x0` of the result slot is its tag, `0` being nil.
    unsafe { *((top - 0x10) as *const i32) == 0 }
}

/// The memo key for this call, or `None` for any shape the memo cannot serve.
///
/// Reproduces the persistent half of the `lua_rawgeti(L, 1, 0)` a hit elides:
/// reading a slot whose taint word is set publishes that word to the current
/// owner while tainting is enabled. Its pushed copy died with the `settop`
/// that followed, so nothing else of it survives. Delegated shapes: fewer
/// than two arguments, argument 1 not a table, argument 2 not a string,
/// `t[0]` not light userdata (stock leaves the fetched value on the stack on
/// that path, which delegating reproduces), or a null object pointer.
fn method_key(l: i32) -> Option<u64> {
    let ls = l.cast_unsigned() as usize;
    // SAFETY: `l` is the live `lua_State` the host dispatched with; `+0x8` is
    // its stack top pointer.
    let top = unsafe { *((ls + 0x8) as *const usize) };
    // SAFETY: `+0xc` is the state's stack base pointer (argument 1).
    let base = unsafe { *((ls + 0xc) as *const usize) };
    if top.wrapping_sub(base) >> 4 < 2 {
        return None;
    }
    // SAFETY: `base` addresses argument 1's 16-byte slot; `+0x0` is its tag.
    if unsafe { *(base as *const i32) } != 5 {
        return None;
    }
    let arg2 = base + 0x10;
    // SAFETY: argument 2 exists (checked above); `+0x0` is its tag.
    if unsafe { *(arg2 as *const i32) } != 4 {
        return None;
    }
    // SAFETY: a tag-5 slot's payload at `+0x8` is the `Table*`.
    let table = unsafe { *((base + 0x8) as *const usize) };
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
    if obj == 0 {
        return None;
    }
    // SAFETY: `obj` is a live object; its first word is its vtable pointer.
    let vtbl = unsafe { *(obj as *const usize) };
    // SAFETY: a tag-4 slot's payload at `+0x8` is the interned `TString*`.
    let name = unsafe { *((arg2 + 0x8) as *const usize) };
    Some(vtbl as u64 | (name as u64) << 32)
}

/// Push `registry[method_ref]` the way `lua_rawgeti` (`0x6f3bc0`) does.
///
/// The registry `TObject` sits at `+0x30` of the `global_State`, its payload
/// being the registry table, and `luaH_getnum` is a reimplemented hook, so
/// the whole push runs without a client call. The 16-byte slot copy carries
/// the host's value-copy taint rule: a set taint word travels with the copy
/// and is published to the current owner while tainting is enabled; an unset
/// one takes the current owner instead.
fn push_registry(l: i32, method_ref: u32) {
    let ls = l.cast_unsigned() as usize;
    // SAFETY: `+0x10` of the state is its `global_State*`.
    let g = unsafe { *((ls + 0x10) as *const usize) };
    // SAFETY: `G + 0x30` is the registry `TObject`; its payload at `+0x8` is
    // the registry `Table*`.
    let registry = unsafe { *((g + 0x38) as *const usize) };
    let src =
        super::hooks::lua_h_getnum__6fa700(registry as *mut u8, method_ref.cast_signed()) as usize;
    // SAFETY: `+0x8` is the state's stack top pointer, the next free slot.
    let top = unsafe { *((ls + 0x8) as *const usize) };
    // SAFETY: `src` is a live `TObject` (the shared nil object at worst);
    // `+0x0` is its tag.
    let tag = unsafe { *(src as *const u32) };
    // SAFETY: `+0x4` of the slot is its taint word.
    let taint = unsafe { *((src + 0x4) as *const u32) };
    // SAFETY: `+0x8` is the payload; the original copies all four words of
    // the slot, `+0xc` included.
    let payload = unsafe { *((src + 0x8) as *const u32) };
    // SAFETY: as above.
    let spare = unsafe { *((src + 0xc) as *const u32) };
    // SAFETY: `top` addresses the next free 16-byte slot (a script method has
    // headroom above its arguments).
    unsafe { *(top as *mut u32) = tag };
    // SAFETY: `+0x8` and `+0xc` take the payload words.
    unsafe { *((top + 0x8) as *mut u32) = payload };
    // SAFETY: as above.
    unsafe { *((top + 0xc) as *mut u32) = spare };
    if taint == 0 {
        // SAFETY: `TAINT_CUR` is the current-owner word an untainted copy
        // takes.
        let owner = unsafe { *(TAINT_CUR as *const u32) };
        // SAFETY: `+0x4` of the slot being pushed is its taint word.
        unsafe { *((top + 0x4) as *mut u32) = owner };
    } else {
        // SAFETY: `TAINT_ON` gates publishing a set taint word.
        if unsafe { *(TAINT_ON as *const u32) } != 0 {
            // SAFETY: publishing the copied value's taint word, as every
            // host-side value copy does.
            unsafe { *(TAINT_CUR as *mut u32) = taint };
        }
        // SAFETY: `+0x4` of the slot being pushed carries the copied word.
        unsafe { *((top + 0x4) as *mut u32) = taint };
    }
    // SAFETY: publishing the slot by advancing the top, as the original does.
    unsafe { *((ls + 0x8) as *mut usize) = top + 0x10 };
}

/// Push nil through the host, as stock's not-found path does.
fn push_nil(l: i32) {
    // SAFETY: a fixed `.text` entry in the live host image (base verified at
    // load); the transmuted signature matches the declared prototype
    // (`__fastcall(ecx = L)`, no return).
    let push: extern "fastcall" fn(i32) = unsafe { core::mem::transmute(LUA_PUSHNIL_VA) };
    push(l);
}

/// Anchor the interned method name, so the memo key's pointer stays valid.
///
/// Duplicates argument 2 (re-read off the state, after the delegate ran) onto
/// the top so `luaL_ref`'s pop consumes the copy; the ref pins the `TString`
/// for the life of the state, which is what lets a hit trust pointer equality
/// with no content check. The copy carries the host's value-copy taint rule:
/// a set taint word is published to the current owner (when enabled) and
/// travels with the copy, an unset one takes the current owner instead.
fn anchor_name(l: i32) -> Option<u32> {
    let ls = l.cast_unsigned() as usize;
    // SAFETY: `+0xc` is the state's stack base pointer (argument 1).
    let base = unsafe { *((ls + 0xc) as *const usize) };
    let arg2 = base + 0x10;
    // SAFETY: argument 2's 16-byte slot; `+0x0` is its tag, `4` a string.
    if unsafe { *(arg2 as *const i32) } != 4 {
        return None;
    }
    // SAFETY: a tag-4 slot's payload at `+0x8` is the `TString*`.
    let ts = unsafe { *((arg2 + 0x8) as *const usize) };
    // SAFETY: `+0x4` of the slot is its taint word.
    let taint = unsafe { *((arg2 + 0x4) as *const u32) };
    // SAFETY: `+0x8` is the state's stack top pointer, the next free slot.
    let top = unsafe { *((ls + 0x8) as *const usize) };
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
    (anchored > 0).then(|| anchored.cast_unsigned())
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

/// The per-class resolver reimplementation, recording the ref a match uses.
///
/// A transcription of `0x702000` plus one store: the registry ref a match
/// resolves to is left in [`RECORDED_REF`] for the `__index` miss path to
/// memoize. Everything else is stock, and has to be, because resolver chains
/// also reach this without going through `__index` at all: hash the name,
/// walk the class table's bucket, and push a match through the host's own
/// `lua_rawgeti` (return 1), or return 0 so the caller moves on to the
/// parent class's resolver.
pub fn lookup(l: i32, name: *const u8, table: *mut u8) -> i32 {
    WALKED.store(1, Ordering::Relaxed);
    let t = table.addr();
    // SAFETY: `+0x24` of a class method table is its bucket mask, -1 until
    // the table is built.
    let mask = unsafe { *((t + 0x24) as *const u32) };
    if mask == 0xffff_ffff {
        SAW_UNBUILT.store(1, Ordering::Relaxed);
        return 0;
    }
    let hash = hash_path(name);
    // SAFETY: `+0x1c` of the table is its bucket array (12-byte stride).
    let buckets = unsafe { *((t + 0x1c) as *const usize) };
    let bucket = buckets + (hash & mask) as usize * 12;
    // SAFETY: `+0x0` of a bucket is the intrusive-link offset of its nodes.
    let link = unsafe { *(bucket as *const usize) };
    // SAFETY: `+0x8` of a bucket is its first node.
    let mut node = unsafe { *((bucket + 0x8) as *const usize) };
    loop {
        if node == 0 || node & 1 != 0 {
            // End of chain: null, or a low-bit-tagged list terminator.
            return 0;
        }
        // SAFETY: `+0x0` of a node is the hash its name was inserted under.
        if unsafe { *(node as *const u32) } == hash {
            // SAFETY: `+0x14` of a node is its NUL-terminated method name.
            let node_name = unsafe { *((node + 0x14) as *const *const u8) };
            if cmp_n(node_name, name, 0x7fff_ffff) == 0 {
                break;
            }
        }
        // SAFETY: a node's next pointer lives at its link offset plus 4.
        node = unsafe { *((node + link + 4) as *const usize) };
    }
    // SAFETY: `+0x18` of a node is the registry ref of its method.
    let method_ref = unsafe { *((node + 0x18) as *const u32) };
    RECORDED_REF.store(method_ref, Ordering::Relaxed);
    rawgeti(l, LUA_REGISTRYINDEX, method_ref.cast_signed());
    1
}

/// Hash a method name the way the class tables were built.
fn hash_path(name: *const u8) -> u32 {
    // SAFETY: a fixed `.text` entry in the live host image (base verified at
    // load); the transmuted signature matches the declared prototype
    // (`stdcall(name)`, register `ret`).
    let hash: extern "stdcall" fn(*const u8) -> u32 =
        unsafe { core::mem::transmute(SSTR_HASH_PATH_VA) };
    hash(name)
}

/// Compare two names the way the resolver does, zero meaning equal.
fn cmp_n(a: *const u8, b: *const u8, max: u32) -> i32 {
    // SAFETY: a fixed `.text` entry in the live host image (base verified at
    // load); the transmuted signature matches the declared prototype
    // (`stdcall(a, b, max)`, register `ret`).
    let cmp: extern "stdcall" fn(*const u8, *const u8, u32) -> i32 =
        unsafe { core::mem::transmute(SSTR_CMP_N_VA) };
    cmp(a, b, max)
}

/// Push `registry[n]` through the host's `lua_rawgeti`, as stock does.
fn rawgeti(l: i32, idx: i32, n: i32) {
    // SAFETY: a fixed `.text` entry in the live host image (base verified at
    // load); the transmuted signature matches the declared prototype
    // (`__fastcall(ecx = L, edx = idx)` plus `n` on the stack, `RET 4`).
    let raw: extern "fastcall" fn(i32, i32, i32) = unsafe { core::mem::transmute(LUA_RAWGETI_VA) };
    raw(l, idx, n);
}

/// Drop the memo and the recorder wholesale, without unref-ing.
///
/// Called from the `FrameScript_Shutdown` adapter before the stock teardown
/// runs `lua_close`: the recorded method refs and the name anchors index the
/// state being closed, so they die with it, and a recycled allocation for the
/// next state cannot inherit a stale entry.
pub fn forget() {
    MEMO.clear();
    RECORDED_REF.store(0, Ordering::Relaxed);
    SAW_UNBUILT.store(0, Ordering::Relaxed);
    WALKED.store(0, Ordering::Relaxed);
}

/// One cumulative counters line on the events gauge's 60-second cadence.
pub fn emit_cumulative() {
    let hits = HITS.get();
    let nil_hits = NIL_HITS.get();
    let misses = MISSES.get();
    let delegated = DELEGATED.get();
    let evictions = EVICTIONS.get();
    if hits | nil_hits | misses | delegated == 0 {
        return;
    }
    log::info!(
        target: super::tally::TARGET,
        "index: {hits} hits + {nil_hits} nil hits / {misses} misses, \
         delegated {delegated}, evict {evictions}",
    );
}
