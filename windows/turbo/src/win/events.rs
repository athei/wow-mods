//! Event-dispatch gauge behind the `wow::events` debug filter.
//!
//! Observation-only instrumentation over the client's script event dispatch:
//! the adapters in `hooks.rs` forward every call to the original and, when the
//! gauge is armed, time it and attribute the cost. Armed means the
//! `wow::events` target has debug logging enabled; otherwise every entry point
//! here is a plain delegate and the tables are never touched, so a shipped
//! build adds nothing at the default filter.
//!
//! Attribution model. Event dispatch has two initiators: `SignalEvent`
//! (paramless, a full wrapper here) and `SignalEventParam` (variadic, entry
//! tap only). A paramless signal's cost is its wrapper time, which includes
//! the listener walk. A parameterized signal's cost is the sum of its
//! per-listener `0x702710` calls — that function has exactly two callers, the
//! parameterized dispatch loop and the variadic UI wrapper `0x7026f0`, and
//! both are tapped, so every `0x702710` call is preceded by either a "current
//! event" store or a clear and the attribution cannot go stale. The `0x702710`
//! wrapper restores the current-event slot on exit, so a handler that fires
//! nested UI invokes (or nested dispatches) cannot bleed the outer loop's
//! remaining listeners into the wrong bucket. Handler rows are keyed by frame
//! name (fetched before the handler runs — a handler may destroy its own
//! frame) and are inclusive of nested work; the per-second totals line only
//! counts depth-zero entries, so nothing is double-counted there.
//!
//! Reading a line. The header splits the dispatch total into `signals` (time
//! under an event, whether the paramless wrapper or a parameterized
//! listener) and `invokes` (everything else at depth zero: `OnUpdate` and
//! the rest of the per-frame UI callbacks). Both lines close with a
//! `+N more` row folding every table entry past the printed top-N, so the
//! ranked rows never imply the tail is empty; the events line also names the
//! `busiest` row by signal count when the ticks ranking would have hidden it,
//! which is how a storm of cheap zero-listener signals stays visible.
//!
//! The same header also splits the total the other way, into the `bodies` that
//! ran and the `machinery` around them: argument binding, the dispatch frames,
//! the stock prologue. That second split is the one that says how much of a
//! session's script cost is the scripts themselves, and it bounds what any
//! faster invoke path could ever win back.
//!
//! Do not read the event rows as a partition of `signals`. Rows are billed at
//! every depth while the totals count depth zero only, so a dispatch nested
//! inside a handler bills its own row while its time is already inside the
//! parent's. Summing the rows can therefore land slightly above `signals`, and
//! does whenever nesting occurs.
//!
//! Event registry (fixed client globals): base `*(0xceef68)`, count
//! `*(0xceef64)`, stride 0x10 — entry+0x0 event name, entry+0xc intrusive
//! listener list (node+0x4 next, low-bit-tagged terminator; node+0x8 the
//! listening frame). A frame's name comes from its vtable slot 1, exactly as
//! the original dispatcher resolves it.

use std::{
    collections::HashMap,
    ffi::CStr,
    fmt::Write as _,
    sync::{
        LazyLock, Mutex, PoisonError,
        atomic::{AtomicU32, Ordering},
    },
};

/// Address holding the event-registry entry count.
const REGISTRY_COUNT: usize = 0x00ce_ef64;
/// Address holding the event-registry base pointer.
const REGISTRY_BASE: usize = 0x00ce_ef68;
/// Bytes per registry entry.
const REGISTRY_STRIDE: usize = 0x10;
/// Stored-name cap; longer names are truncated for table keys and log lines.
const NAME_CAP: usize = 40;
/// Rows shown in the per-second top list.
///
/// Eight rather than four so the per-frame drivers (`WorldFrame`, `UIParent`)
/// stay visible under a heavy addon load: each runs its script once per frame,
/// so their call counts are the frame rate, which is what turns a per-second
/// cost into a share of the frame budget.
const TOP_PER_SECOND: usize = 8;
/// Rows shown in the periodic cumulative top list.
const TOP_CUMULATIVE: usize = 8;
/// Handler-table size cap; past it new names fall into the overflow row.
const HANDLER_TABLE_CAP: usize = 512;
/// Chunk-memo cap; a client cannot load more script files than this.
const OWNER_TABLE_CAP: usize = 4_096;
/// Bound on the addon-directory walk, so a diagnostic cannot wander.
const FILE_WALK_CAP: usize = 4_096;
/// Milliseconds between per-second summary emissions.
const WINDOW_MS: u64 = 1_000;
/// Milliseconds between cumulative summary emissions.
const CUMULATIVE_MS: u64 = 60_000;
/// Upper bound on any registry/listener-list walk, against corrupt links.
const WALK_CAP: u32 = 4_096;

/// Upper edges, in microseconds, for the handler-body cost buckets.
///
/// A body cannot cost less than the interpreter's own call setup, so the
/// cheapest bucket is a standing estimate of that floor, and how much of a
/// session's time sits in it says whether the cost is per-call overhead or
/// what the scripts actually do. The edges bracket what an armed session
/// shows: a floor around two microseconds, a mean near eight, and a long tail
/// of the handlers that genuinely work.
const BODY_BUCKET_US: [u64; 5] = [2, 4, 8, 16, 64];
/// Bucket count: one per edge, plus everything above the last.
const BODY_BUCKETS: usize = BODY_BUCKET_US.len() + 1;

/// Fixed-size, nul-padded name key.
type NameBuf = [u8; NAME_CAP];

/// Whether the gauge is armed, resolved once on first dispatch.
///
/// The logger is initialized in `DllMain` before any hook can fire, and the
/// filter never changes mid-run, so one resolution is enough. The armed line
/// lets a tester confirm their config from the login screen alone.
static ARMED: LazyLock<bool> = LazyLock::new(|| {
    let armed = log::log_enabled!(target: "wow::events", log::Level::Debug);
    if armed {
        log::debug!(target: "wow::events", "event gauge armed");
    }
    armed
});

/// Current parameterized-dispatch event, as id+1 (0 = none).
///
/// Stored by the `SignalEventParam` tap, cleared by the `0x7026f0` UI-wrapper
/// tap, restored across every `0x702710` call.
static PARAM_EVENT: AtomicU32 = AtomicU32::new(0);

/// Current paramless-dispatch event, as id+1 (0 = none).
///
/// Saved/set/restored by the `SignalEvent` wrapper, so it is exact under
/// nested dispatch.
static WRAPPER_EVENT: AtomicU32 = AtomicU32::new(0);

/// Nesting depth of timed dispatch entries; only depth zero feeds the totals.
static DEPTH: AtomicU32 = AtomicU32::new(0);

/// Per-handler accumulator.
#[derive(Default)]
struct Stat {
    count: u64,
    ticks: u64,
    max_ticks: u64,
}

/// Per-event accumulator.
struct EventRow {
    name: NameBuf,
    signals: u64,
    handler_calls: u64,
    ticks: u64,
    max_ticks: u64,
}

/// One accumulation window (per-second or cumulative).
#[derive(Default)]
struct Tables {
    signals: u64,
    handler_calls: u64,
    /// Depth-zero inclusive dispatch ticks — the "time in dispatch" total.
    total_ticks: u64,
    /// The part of `total_ticks` attributable to an event signal.
    ///
    /// The rest is per-frame script work: `OnUpdate` and the other UI
    /// invokes, which no event row can explain.
    signal_ticks: u64,
    events: HashMap<i32, EventRow>,
    handlers: HashMap<NameBuf, Stat>,
    /// Ticks inside the handler bodies themselves, outermost only.
    ///
    /// The window total minus this is the dispatch and argument-binding
    /// machinery around them.
    body_ticks: u64,
    /// Bodies behind `body_ticks`, so the line can state a per-body cost.
    body_calls: u64,
    /// Body ticks by how long the body took, bucketed by `BODY_BUCKET_US`.
    body_hist_ticks: [u64; BODY_BUCKETS],
    /// Bodies per bucket, so a bucket states both its mass and its count.
    body_hist_calls: [u64; BODY_BUCKETS],
    /// Per-addon body cost, keyed by the folder that owns the script.
    owners: HashMap<NameBuf, Stat>,
    /// Handler names dropped on the table cap (reported, never silent).
    dropped: u64,
    /// Ticks behind `dropped`, so the overflow row carries a cost too.
    dropped_ticks: u64,
}

/// Gauge state: the rolling one-second window plus the cumulative tables.
struct State {
    window_start: u64,
    cumulative_emit: u64,
    window: Tables,
    cumulative: Tables,
    /// Frames declared in addon markup, mapped to their declaring addon.
    ///
    /// Read from disk on first use, never on the load path.
    frames: Option<HashMap<NameBuf, NameBuf>>,
    /// Chunk-to-addon memo, keyed by a hash of the chunk name.
    ///
    /// Keyed by content rather than by the string object, so a reload that
    /// re-interns every chunk at a fresh address reuses the entries instead of
    /// doubling them, and a recycled address cannot inherit an owner.
    owners: HashMap<u64, NameBuf>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| {
    let t = wow_shared::tsc::rdtsc();
    Mutex::new(State {
        window_start: t,
        cumulative_emit: t,
        window: Tables::default(),
        cumulative: Tables::default(),
        frames: None,
        owners: HashMap::new(),
    })
});

/// Whether the gauge is armed (cheap after the first call).
pub fn armed() -> bool {
    *ARMED
}

/// Nesting depth of timed handler bodies; only the outermost is measured.
static BODY_DEPTH: AtomicU32 = AtomicU32::new(0);

/// Hash a chunk's bytes, to tell one path from another by content.
///
/// FNV-1a over the whole string: a path is a few dozen bytes, so this is
/// cheaper than the parse it guards and vastly cheaper than being wrong.
fn chunk_hash(text: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in text {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Resolve a chunk to the addon that owns it, memoized on the chunk string.
///
/// Address and length find the entry; the content decides whether to trust it.
/// Chunk strings are collectable, so a UI reload can free one and hand the same
/// block back for another script's path — and paths cluster in length, sharing
/// the `Interface\AddOns\` prefix, so identical address and length is a real
/// coincidence rather than a fanciful one. Comparing a hash of the bytes makes
/// a stale hit re-resolve instead of quietly billing one addon's time to
/// another, which is the kind of error a reader of the log could never catch.
///
/// One entry per script file, so the table is bounded by the addon set and
/// every steady-state lookup is a hit plus one hash of a short string.
fn owner_of(st: &mut State, chunk: (usize, u32)) -> NameBuf {
    let (bytes, len) = chunk;
    if bytes == 0 {
        // A C function: engine code reached through a script handler slot,
        // which no addon owns.
        return name_from_bytes(b"(engine)");
    }
    // SAFETY: `bytes`/`len` came from a live string object, whose bytes are
    // inline and whose length excludes the terminator.
    let text = unsafe { core::slice::from_raw_parts(bytes as *const u8, len as usize) };
    let hash = chunk_hash(text);
    if let Some(owner) = st.owners.get(&hash) {
        return *owner;
    }
    let owner = match xml_frame(text) {
        // A markup-defined script: the frame is all its name carries, so ask
        // the index which addon declared that frame. Absent from the index
        // means the frame came with the client or was made at runtime, and the
        // row keeps the frame's own name.
        Some(frame) => {
            let index = st.frames.get_or_insert_with(build_frame_index);
            index
                .get(&frame)
                .copied()
                .unwrap_or_else(|| xml_row(&frame))
        }
        None => addon_from_chunk(text),
    };
    if st.owners.len() >= OWNER_TABLE_CAP {
        // Start over rather than stop caching: a table that stopped inserting
        // would re-parse every chunk on every invoke, for the rest of the
        // session, on the hot path.
        st.owners.clear();
    }
    // One line per script FILE, the first time it runs a handler: the only
    // place the raw chunk name is visible to check an attribution against.
    // Scripts written inline in markup are excluded — there is one chunk per
    // handler rather than per file, thousands of them in a session, and their
    // name is already the row they group under.
    if text.contains(&b'\\') {
        log::debug!(
            target: "wow::events",
            "chunk: {:?} -> {}",
            String::from_utf8_lossy(&text[..text.len().min(120)]),
            name_str(&owner),
        );
    }
    st.owners.insert(hash, owner);
    owner
}

/// Map every frame an addon declares in its markup to that addon's folder.
///
/// A script written inline in markup loses its file at compile time, so the
/// only thing left naming it is the frame. The declaring file is still on disk
/// though, and the addon directory sits beside the executable we are loaded
/// into, so reading it recovers exactly what the compiler discarded.
///
/// Read once, lazily, off the load path: the client is running by the time any
/// handler fires. A frame absent from every addon's markup came with the client
/// or was made at runtime, and stays attributed to itself.
fn build_frame_index() -> HashMap<NameBuf, NameBuf> {
    let mut index = HashMap::new();
    let Some(addons) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("Interface").join("AddOns")))
    else {
        return index;
    };
    let Ok(dir) = std::fs::read_dir(&addons) else {
        return index;
    };
    for addon in dir.flatten().take(FILE_WALK_CAP) {
        let owner = name_from_bytes(addon.file_name().as_encoded_bytes());
        let mut pending = vec![addon.path()];
        let mut budget = FILE_WALK_CAP;
        while let Some(path) = pending.pop() {
            if budget == 0 {
                break;
            }
            budget -= 1;
            if path.is_dir() {
                pending.extend(
                    std::fs::read_dir(&path)
                        .into_iter()
                        .flatten()
                        .flatten()
                        .map(|e| e.path()),
                );
                continue;
            }
            if path
                .extension()
                .is_none_or(|e| !e.eq_ignore_ascii_case("xml"))
            {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            for frame in declared_frames(&text) {
                index.entry(frame).or_insert(owner);
            }
        }
    }
    log::debug!(
        target: "wow::events",
        "addon frame index: {} frames declared in addon markup",
        index.len(),
    );
    index
}

/// The frame a markup-defined chunk names, if that is what this chunk is.
///
/// Those chunks are named `Frame:Handler` and carry no path, which is exactly
/// what distinguishes them from a chunk compiled out of a file.
fn xml_frame(text: &[u8]) -> Option<NameBuf> {
    if text.contains(&b'\\') {
        return None;
    }
    let colon = text.iter().position(|&b| b == b':')?;
    Some(name_from_bytes(&text[..colon]))
}

/// Label a frame the addon index could not claim, as a row of its own.
fn xml_row(frame: &NameBuf) -> NameBuf {
    let mut buf: NameBuf = [0; NAME_CAP];
    let name = name_str(frame).as_bytes();
    let n = name.len().min(NAME_CAP - 5);
    buf[..4].copy_from_slice(b"xml:");
    buf[4..4 + n].copy_from_slice(&name[..n]);
    buf
}

/// Pull every `name="Frame"` a markup file declares.
///
/// Deliberately a scan for the attribute rather than a parse: the frame names
/// are all that is wanted, and a malformed or unusual file should yield nothing
/// rather than derail a diagnostic.
fn declared_frames(text: &str) -> Vec<NameBuf> {
    let mut out = Vec::new();
    for (at, _) in text.match_indices("name=\"") {
        let rest = &text.as_bytes()[at + 6..];
        let Some(end) = rest.iter().position(|&b| b == b'"') else {
            continue;
        };
        // `$parent`-relative names resolve at runtime to something this scan
        // cannot predict, so they are skipped rather than recorded wrong.
        let name = &rest[..end];
        if !name.is_empty() && !name.contains(&b'$') {
            out.push(name_from_bytes(name));
        }
    }
    out
}

/// Extract the owning addon's folder from a script chunk name.
///
/// A chunk compiled from a file carries its path, so the folder under `AddOns`
/// names the addon and the stock interface folders fall out as their own
/// buckets: the split between what shipped with the client and what a user
/// installed. Lua prefixes file chunks with `@`.
///
/// A script written inline in a frame's markup carries no path at all — the
/// loader names it `Frame:Handler` — so the file, and with it the addon, is
/// gone by the time the function exists. Those group under the frame with an
/// `xml:` prefix, which keeps one row per frame instead of one per handler and
/// is honest that the row names a frame rather than an owner. Anything else is
/// kept verbatim rather than pooled into an anonymous bucket.
fn addon_from_chunk(text: &[u8]) -> NameBuf {
    const ADDONS: &[u8] = b"addons\\";
    let text = text.strip_prefix(b"@").unwrap_or(text);
    let lower: Vec<u8> = text.to_ascii_lowercase();
    if let Some(at) = lower
        .windows(ADDONS.len())
        .position(|w| w == ADDONS)
        .map(|p| p + ADDONS.len())
    {
        let rest = &text[at..];
        let end = rest.iter().position(|&b| b == b'\\').unwrap_or(rest.len());
        return name_from_bytes(&rest[..end]);
    }
    if lower.starts_with(b"interface\\") {
        let rest = &text[b"interface\\".len()..];
        let end = rest.iter().position(|&b| b == b'\\').unwrap_or(rest.len());
        return name_from_bytes(&rest[..end]);
    }
    name_from_bytes(text)
}

/// Build a name key from raw bytes, truncating at the cap.
fn name_from_bytes(bytes: &[u8]) -> NameBuf {
    let mut buf: NameBuf = [0; NAME_CAP];
    let n = bytes.len().min(NAME_CAP - 1);
    buf[..n].copy_from_slice(&bytes[..n]);
    buf
}

/// Time one handler body, the protected call that runs the script itself.
///
/// Splits an invoke into the script it ran and everything else: the argument
/// binding either side of it, the dispatch frames above it, and the stock
/// prologue between. Subtracting from the window total gives that remainder,
/// which is the part a faster invoke path could ever win back, so the split
/// bounds the whole avenue instead of hiding it under body noise.
///
/// Unarmed this is the call itself and nothing more. Armed it costs two
/// counter reads on the OUTERMOST body only: a handler that dispatches into
/// another handler nests, and timing both would count the inner one twice,
/// once on its own and once inside its parent.
pub fn time_body<T>(chunk: (usize, u32), body: impl FnOnce() -> T) -> T {
    if !armed() {
        return body();
    }
    let nested = BODY_DEPTH.fetch_add(1, Ordering::Relaxed) != 0;
    if nested {
        let out = body();
        BODY_DEPTH.fetch_sub(1, Ordering::Relaxed);
        return out;
    }
    let t0 = wow_shared::tsc::rdtsc();
    let out = body();
    let dt = wow_shared::tsc::rdtsc().wrapping_sub(t0);
    BODY_DEPTH.fetch_sub(1, Ordering::Relaxed);
    let mut st = state();
    st.window.body_ticks += dt;
    st.window.body_calls += 1;
    let us = ticks_to_us(dt);
    let bucket = BODY_BUCKET_US
        .iter()
        .position(|&edge| us < edge)
        .unwrap_or(BODY_BUCKETS - 1);
    st.window.body_hist_ticks[bucket] += dt;
    st.window.body_hist_calls[bucket] += 1;
    let owner = owner_of(&mut st, chunk);
    let stat = st.window.owners.entry(owner).or_default();
    stat.count += 1;
    stat.ticks += dt;
    stat.max_ticks = stat.max_ticks.max(dt);
    drop(st);
    out
}

/// Encode an event id into the context slots (id+1; 0 means none).
const fn id_tag(event_id: i32) -> u32 {
    event_id.cast_unsigned().wrapping_add(1)
}

/// Ticks-delta to microseconds via the published engine-clock scale.
fn ticks_to_us(ticks: u64) -> u64 {
    super::hooks::clock_ticks_to_ms(ticks.saturating_mul(1000))
}

/// Append `X.XXX` milliseconds (from a ticks delta) to a line.
fn push_ms(line: &mut String, ticks: u64) {
    let us = ticks_to_us(ticks);
    let _ = write!(line, "{}.{:03}", us / 1000, us % 1000);
}

/// Copy a nul-terminated name into a fixed key, truncating at the cap.
fn name_from_cstr(p: *const u8) -> NameBuf {
    let mut buf: NameBuf = [0; NAME_CAP];
    if p.is_null() {
        buf[..9].copy_from_slice(b"(unnamed)");
        return buf;
    }
    // SAFETY: the pointer is an event or frame name the client itself treats
    // as a nul-terminated C string in its own dispatch path.
    let bytes = unsafe { CStr::from_ptr(p.cast()) }.to_bytes();
    let n = bytes.len().min(NAME_CAP - 1);
    buf[..n].copy_from_slice(&bytes[..n]);
    buf
}

/// Render a name key for a log line.
fn name_str(name: &NameBuf) -> &str {
    let end = name.iter().position(|&b| b == 0).unwrap_or(NAME_CAP);
    std::str::from_utf8(&name[..end]).unwrap_or("(non-utf8)")
}

/// Read the event registry base pointer (0 when not yet initialized).
const fn registry_base() -> usize {
    // SAFETY: fixed client global at the load-checked image base; holds the
    // event-registry base pointer.
    unsafe { *(REGISTRY_BASE as *const usize) }
}

/// Read the event registry entry count.
const fn registry_count() -> u32 {
    // SAFETY: fixed client global at the load-checked image base; holds the
    // event-registry entry count.
    unsafe { *(REGISTRY_COUNT as *const u32) }
}

/// Resolve an event id to its registry name, or a placeholder.
fn event_name(event_id: i32) -> NameBuf {
    let base = registry_base();
    let count = registry_count();
    let id = event_id.cast_unsigned();
    if base == 0 || id >= count.min(WALK_CAP) {
        let mut buf: NameBuf = [0; NAME_CAP];
        buf[..8].copy_from_slice(b"(bad-id)");
        return buf;
    }
    let entry = base + id as usize * REGISTRY_STRIDE;
    // SAFETY: `entry` is a live registry row (id < count); +0x0 is the name.
    let p = unsafe { *(entry as *const *const u8) };
    name_from_cstr(p)
}

/// Count the listeners of one registry entry (bounded walk).
const fn listener_count(entry: usize) -> u32 {
    // SAFETY: `entry` is a live registry row; +0xc is the listener list head.
    let mut node = unsafe { *((entry + 0xc) as *const usize) };
    let mut n = 0;
    while node & 1 == 0 && node != 0 && n < WALK_CAP {
        n += 1;
        // SAFETY: `node` is an untagged, non-null listener node; +0x4 links
        // to the next node (low-bit-tagged terminator).
        node = unsafe { *((node + 4) as *const usize) };
    }
    n
}

/// Resolve a frame's name via its vtable slot 1, as the dispatcher does.
///
/// Called before the frame's handler runs, while the pointer is known live
/// (the original is about to make the same virtual call on it).
fn frame_name(frame: *mut core::ffi::c_void) -> NameBuf {
    let mut buf: NameBuf = [0; NAME_CAP];
    if frame.is_null() {
        buf[..6].copy_from_slice(b"(null)");
        return buf;
    }
    // SAFETY: `frame` is the live frame object the original dispatcher is
    // about to invoke; its first word is the vtable pointer.
    let vtbl = unsafe { *frame.cast::<usize>() };
    if vtbl == 0 {
        buf[..9].copy_from_slice(b"(unnamed)");
        return buf;
    }
    // SAFETY: vtable slot 1 is the name getter the original dispatch path
    // itself calls on this object.
    let slot = unsafe { *((vtbl + 4) as *const usize) };
    if slot == 0 {
        buf[..9].copy_from_slice(b"(unnamed)");
        return buf;
    }
    let get_name: extern "thiscall" fn(*mut core::ffi::c_void) -> *const u8 =
        // SAFETY: the slot holds the client's `thiscall` name getter, the same
        // pointer the original dispatcher invokes on this object.
        unsafe { core::mem::transmute(slot) };
    name_from_cstr(get_name(frame))
}

/// Lock the state, riding through a (never-expected) poisoned lock.
fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `SignalEvent` wrapper — paramless dispatch, timed whole.
pub fn signal_event(event_id: i32) {
    let original = super::symbols::originals::signal_event__703e50();
    if !armed() {
        original(event_id);
        return;
    }
    let dump = event_name(event_id);
    let prev = WRAPPER_EVENT.swap(id_tag(event_id), Ordering::Relaxed);
    let depth = DEPTH.fetch_add(1, Ordering::Relaxed);
    let t0 = wow_shared::tsc::rdtsc();
    original(event_id);
    let dt = wow_shared::tsc::rdtsc().wrapping_sub(t0);
    DEPTH.fetch_sub(1, Ordering::Relaxed);
    WRAPPER_EVENT.store(prev, Ordering::Relaxed);
    record_signal(event_id, dt, depth == 0);
    // Bracket the loading screens. A capture is a mix of loading and playing,
    // and the two have opposite shapes: loading runs a handful of enormous
    // one-time handlers, playing runs many small ones every frame. Reading a
    // session without knowing which is which turns a load hitch into an
    // apparent frame-dropper, so the log says where the boundaries are.
    // A reload brackets itself the same way a zone change does, so the pair
    // alone cannot tell them apart. Two more markers do: only a reload tears
    // the interface down and reads the saved variables back, and its cost is
    // an interface rebuild rather than a world load.
    match name_str(&dump) {
        "PLAYER_ENTERING_WORLD" => {
            log::debug!(target: "wow::events", "world: entered (loading screen ended)");
            dump_registry();
        }
        "PLAYER_LEAVING_WORLD" => {
            log::debug!(target: "wow::events", "world: leaving (loading screen started)");
        }
        "PLAYER_LOGOUT" => {
            log::debug!(target: "wow::events", "ui: unloading (reload or logout)");
        }
        "VARIABLES_LOADED" => {
            log::debug!(target: "wow::events", "ui: rebuilt (saved variables read back)");
        }
        _ => {}
    }
}

/// `SignalEventParam` entry tap — store the current event and count it.
pub fn signal_event_param_tap(args: *const u32) {
    if !armed() {
        return;
    }
    // SAFETY: `args` is the hooked call's argument area; argument 0 is the
    // event id.
    let event_id = unsafe { args.read() }.cast_signed();
    PARAM_EVENT.store(id_tag(event_id), Ordering::Relaxed);
    let mut st = state();
    let row = event_row(&mut st.window, event_id);
    row.signals += 1;
    st.window.signals += 1;
}

/// `0x7026f0` entry tap — a UI invoke follows, not an event dispatch.
pub fn invoke_formatted_tap(_args: *const u32) {
    if !armed() {
        return;
    }
    PARAM_EVENT.store(0, Ordering::Relaxed);
}

/// `0x702690` wrapper — one paramless handler call, timed.
pub fn invoke_handler(frame: *mut core::ffi::c_void, handler_slot: *mut u32) {
    let original = super::symbols::originals::frame_script_invoke_handler__702690();
    if !armed() {
        original(frame, handler_slot);
        return;
    }
    let name = frame_name(frame);
    let ctx = WRAPPER_EVENT.load(Ordering::Relaxed);
    let depth = DEPTH.fetch_add(1, Ordering::Relaxed);
    let t0 = wow_shared::tsc::rdtsc();
    original(frame, handler_slot);
    let dt = wow_shared::tsc::rdtsc().wrapping_sub(t0);
    DEPTH.fetch_sub(1, Ordering::Relaxed);
    record_handler(&name, dt, ctx, depth == 0, false);
}

/// `0x702710` wrapper — one formatted handler call, timed.
///
/// Restores the current-event slot on exit so nested invokes inside the
/// handler cannot re-attribute the outer dispatch's remaining listeners.
pub fn invoke_handler_formatted_v(
    frame: *mut core::ffi::c_void,
    handler_slot: *mut u32,
    format: *const u8,
    args: *const u32,
) {
    let original = super::symbols::originals::frame_script_invoke_handler_formatted_v__702710();
    if !armed() {
        original(frame, handler_slot, format, args);
        return;
    }
    let name = frame_name(frame);
    let ctx = PARAM_EVENT.load(Ordering::Relaxed);
    let depth = DEPTH.fetch_add(1, Ordering::Relaxed);
    let t0 = wow_shared::tsc::rdtsc();
    original(frame, handler_slot, format, args);
    let dt = wow_shared::tsc::rdtsc().wrapping_sub(t0);
    DEPTH.fetch_sub(1, Ordering::Relaxed);
    PARAM_EVENT.store(ctx, Ordering::Relaxed);
    record_handler(&name, dt, ctx, depth == 0, true);
}

/// Get-or-insert an event row, resolving its name once.
fn event_row(t: &mut Tables, event_id: i32) -> &mut EventRow {
    t.events.entry(event_id).or_insert_with(|| EventRow {
        name: event_name(event_id),
        signals: 0,
        handler_calls: 0,
        ticks: 0,
        max_ticks: 0,
    })
}

/// Record one whole paramless signal (wrapper time, listener walk included).
fn record_signal(event_id: i32, dt: u64, depth_zero: bool) {
    let mut st = state();
    let row = event_row(&mut st.window, event_id);
    row.signals += 1;
    row.ticks += dt;
    row.max_ticks = row.max_ticks.max(dt);
    st.window.signals += 1;
    if depth_zero {
        st.window.total_ticks += dt;
        st.window.signal_ticks += dt;
    }
    maybe_emit(&mut st);
    drop(st);
}

/// Record one timed handler call, attributed to the current event (if any).
fn record_handler(name: &NameBuf, dt: u64, ctx: u32, depth_zero: bool, param_path: bool) {
    let mut st = state();
    st.window.handler_calls += 1;
    if depth_zero {
        st.window.total_ticks += dt;
        if ctx != 0 {
            st.window.signal_ticks += dt;
        }
    }
    if ctx != 0 {
        let row = event_row(&mut st.window, ctx.wrapping_sub(1).cast_signed());
        row.handler_calls += 1;
        if param_path {
            // Parameterized dispatch has no wrapper; the per-listener calls
            // ARE the signal's cost.
            row.ticks += dt;
            row.max_ticks = row.max_ticks.max(dt);
        }
    }
    let over_cap =
        st.window.handlers.len() >= HANDLER_TABLE_CAP && !st.window.handlers.contains_key(name);
    if over_cap {
        st.window.dropped += 1;
        st.window.dropped_ticks += dt;
    } else {
        let stat = st.window.handlers.entry(*name).or_default();
        stat.count += 1;
        stat.ticks += dt;
        stat.max_ticks = stat.max_ticks.max(dt);
    }
    maybe_emit(&mut st);
    drop(st);
}

/// Emit the per-second window (and periodically the cumulative tables).
fn maybe_emit(st: &mut State) {
    let now = wow_shared::tsc::rdtsc();
    let window_ms = super::hooks::clock_ticks_to_ms(now.wrapping_sub(st.window_start));
    if window_ms < WINDOW_MS {
        return;
    }
    emit_tables(&st.window, window_ms, TOP_PER_SECOND, "");
    let window = std::mem::take(&mut st.window);
    merge(&mut st.cumulative, window);
    st.window_start = now;
    let cum_ms = super::hooks::clock_ticks_to_ms(now.wrapping_sub(st.cumulative_emit));
    if cum_ms >= CUMULATIVE_MS {
        emit_tables(&st.cumulative, cum_ms, TOP_CUMULATIVE, "total ");
        st.cumulative_emit = now;
    }
}

/// Fold one window into the cumulative tables (caps respected).
fn merge(cum: &mut Tables, w: Tables) {
    cum.signals += w.signals;
    cum.handler_calls += w.handler_calls;
    cum.total_ticks += w.total_ticks;
    cum.signal_ticks += w.signal_ticks;
    cum.body_ticks += w.body_ticks;
    cum.body_calls += w.body_calls;
    for i in 0..BODY_BUCKETS {
        cum.body_hist_ticks[i] += w.body_hist_ticks[i];
        cum.body_hist_calls[i] += w.body_hist_calls[i];
    }
    cum.dropped += w.dropped;
    cum.dropped_ticks += w.dropped_ticks;
    for (id, row) in w.events {
        let dst = cum.events.entry(id).or_insert_with(|| EventRow {
            name: row.name,
            signals: 0,
            handler_calls: 0,
            ticks: 0,
            max_ticks: 0,
        });
        dst.signals += row.signals;
        dst.handler_calls += row.handler_calls;
        dst.ticks += row.ticks;
        dst.max_ticks = dst.max_ticks.max(row.max_ticks);
    }
    for (name, stat) in w.owners {
        let dst = cum.owners.entry(name).or_default();
        dst.count += stat.count;
        dst.ticks += stat.ticks;
        dst.max_ticks = dst.max_ticks.max(stat.max_ticks);
    }
    for (name, stat) in w.handlers {
        if cum.handlers.len() >= HANDLER_TABLE_CAP && !cum.handlers.contains_key(&name) {
            cum.dropped += stat.count;
            cum.dropped_ticks += stat.ticks;
            continue;
        }
        let dst = cum.handlers.entry(name).or_default();
        dst.count += stat.count;
        dst.ticks += stat.ticks;
        dst.max_ticks = dst.max_ticks.max(stat.max_ticks);
    }
}

/// Append the unprinted event rows as one aggregate row.
///
/// `shown` rows were printed by ticks rank and `skip` (if in range) was
/// printed as the busiest-by-signals row; everything else lands here.
fn push_event_tail(line: &mut String, rows: &[&EventRow], shown: usize, skip: usize) {
    let tail = rows
        .iter()
        .skip(shown)
        .enumerate()
        .filter_map(|(i, r)| (shown + i != skip).then_some(*r));
    let (mut n, mut signals, mut handler_calls, mut ticks) = (0u64, 0u64, 0u64, 0u64);
    for row in tail {
        n += 1;
        signals += row.signals;
        handler_calls += row.handler_calls;
        ticks += row.ticks;
    }
    if n == 0 {
        return;
    }
    let _ = write!(line, " +{n} more s={signals} h={handler_calls} ");
    push_ms(line, ticks);
    line.push_str(" ms;");
}

/// Emit the per-addon script cost, ranked, with the tail folded in.
///
/// This is the line that names what to optimize: the folder under `AddOns` that
/// owns each script, so the cost lands on the addon rather than on whichever
/// frame it happened to attach a handler to. The stock interface files bucket
/// under their own names, which separates what shipped with the client from
/// what a user installed.
fn emit_owners(t: &Tables, top: usize, label: &str) {
    if t.owners.is_empty() {
        return;
    }
    let mut rows: Vec<(&NameBuf, &Stat)> = t.owners.iter().collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.1.ticks));
    let mut line = format!("{label}addons:");
    for (name, stat) in rows.iter().take(top) {
        let _ = write!(line, " {} x{} ", name_str(name), stat.count);
        push_ms(&mut line, stat.ticks);
        line.push_str(" ms (max ");
        push_ms(&mut line, stat.max_ticks);
        line.push_str(");");
    }
    if let Some(tail) = rows.get(top..).filter(|t| !t.is_empty()) {
        let calls: u64 = tail.iter().map(|r| r.1.count).sum();
        let ticks: u64 = tail.iter().map(|r| r.1.ticks).sum();
        let _ = write!(line, " +{} more x{} ", tail.len(), calls);
        push_ms(&mut line, ticks);
        line.push_str(" ms;");
    }
    log::debug!(target: "wow::events", "{line}");
}

/// Emit where the body time sat on the cost scale.
///
/// Mass in the cheapest buckets is per-call cost that no addon can be blamed
/// for and no faster dispatch can remove, since it is already inside the
/// protected call; mass in the tail is what the scripts are doing. Which end
/// carries the time decides what is worth attacking.
fn emit_body_histogram(t: &Tables, label: &str) {
    if t.body_calls == 0 {
        return;
    }
    let mut line = format!("{label}bodies:");
    for (i, ticks) in t.body_hist_ticks.iter().enumerate() {
        if t.body_hist_calls[i] == 0 {
            continue;
        }
        if let Some(edge) = BODY_BUCKET_US.get(i) {
            let _ = write!(line, " <{edge}us x{} ", t.body_hist_calls[i]);
        } else {
            let last = BODY_BUCKET_US[BODY_BUCKETS - 2];
            let _ = write!(line, " {last}us+ x{} ", t.body_hist_calls[i]);
        }
        push_ms(&mut line, *ticks);
        line.push_str(" ms;");
    }
    log::debug!(target: "wow::events", "{line}");
}

/// Emit one events line and one handlers line for a window.
fn emit_tables(t: &Tables, span_ms: u64, top: usize, label: &str) {
    if t.signals == 0 && t.handler_calls == 0 {
        return;
    }
    let mut line = format!(
        "{label}events: {} signals, {} handlers, ",
        t.signals, t.handler_calls
    );
    push_ms(&mut line, t.total_ticks);
    let _ = write!(line, " ms in {span_ms} ms (signals ");
    push_ms(&mut line, t.signal_ticks);
    line.push_str(", invokes ");
    push_ms(&mut line, t.total_ticks.saturating_sub(t.signal_ticks));
    let _ = write!(line, "; {} bodies ", t.body_calls);
    push_ms(&mut line, t.body_ticks);
    line.push_str(" ms, machinery ");
    push_ms(&mut line, t.total_ticks.saturating_sub(t.body_ticks));
    line.push_str(") |");
    let mut rows: Vec<&EventRow> = t.events.values().collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.ticks));
    for row in rows.iter().take(top) {
        let _ = write!(
            line,
            " {} s={} h={} ",
            name_str(&row.name),
            row.signals,
            row.handler_calls
        );
        push_ms(&mut line, row.ticks);
        line.push_str(" ms (max ");
        push_ms(&mut line, row.max_ticks);
        line.push_str(");");
    }
    // The tail is where a cheap-but-enormous signal storm hides: rank by
    // ticks and a million zero-listener signals never make the list. Name the
    // busiest row by signal count when the ticks ranking missed it, then sum
    // whatever is still unprinted so no window is silently truncated.
    let shown = rows.len().min(top);
    let top_signals = rows[..shown].iter().map(|r| r.signals).max().unwrap_or(0);
    let mut skip = usize::MAX;
    let busiest = rows[shown..]
        .iter()
        .enumerate()
        .max_by_key(|(_, r)| r.signals)
        .filter(|(_, r)| r.signals > top_signals);
    if let Some((i, row)) = busiest {
        let _ = write!(
            line,
            " busiest {} s={} h={} ",
            name_str(&row.name),
            row.signals,
            row.handler_calls
        );
        push_ms(&mut line, row.ticks);
        line.push_str(" ms;");
        skip = shown + i;
    }
    push_event_tail(&mut line, &rows, shown, skip);
    log::debug!(target: "wow::events", "{line}");
    emit_owners(t, top, label);
    emit_body_histogram(t, label);
    if t.handlers.is_empty() {
        return;
    }
    let mut line = format!("{label}handlers:");
    let mut rows: Vec<(&NameBuf, &Stat)> = t.handlers.iter().collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.1.ticks));
    for (name, stat) in rows.iter().take(top) {
        let _ = write!(line, " {} x{} ", name_str(name), stat.count);
        push_ms(&mut line, stat.ticks);
        line.push_str(" ms;");
    }
    if let Some(tail) = rows.get(top..).filter(|t| !t.is_empty()) {
        let calls: u64 = tail.iter().map(|r| r.1.count).sum();
        let ticks: u64 = tail.iter().map(|r| r.1.ticks).sum();
        let _ = write!(line, " +{} more x{} ", tail.len(), calls);
        push_ms(&mut line, ticks);
        line.push_str(" ms;");
    }
    if t.dropped > 0 {
        let _ = write!(line, " (+{} calls past the name cap, ", t.dropped);
        push_ms(&mut line, t.dropped_ticks);
        line.push_str(" ms)");
    }
    log::debug!(target: "wow::events", "{line}");
}

/// Dump every event with at least one listener (fires on each world enter).
fn dump_registry() {
    let base = registry_base();
    if base == 0 {
        return;
    }
    let count = registry_count().min(WALK_CAP);
    let mut listened = 0u32;
    for id in 0..count {
        let entry = base + id as usize * REGISTRY_STRIDE;
        let n = listener_count(entry);
        if n == 0 {
            continue;
        }
        listened += 1;
        // SAFETY: `entry` is a live registry row (id < count); +0x0 is the
        // name.
        let p = unsafe { *(entry as *const *const u8) };
        let name = name_from_cstr(p);
        log::debug!(target: "wow::events", "reg: {} listeners={n}", name_str(&name));
    }
    log::debug!(
        target: "wow::events",
        "reg: {listened} of {count} events have listeners",
    );
}
