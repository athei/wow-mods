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
//! Inside the bodies the cost splits once more, into `api` and `vm`. Every
//! Lua-level call funnels through `luaD_precall`, which either prepares a Lua
//! closure for the interpreter or calls a C closure outright, so timing the
//! second case separates the client's own script API from the interpreter and
//! the Lua runtime under it. `api` covers only calls made inside a timed body,
//! which is what keeps it comparable with the `bodies` number beside it; the
//! `api:` line then ranks those C functions by their entry address, left
//! unresolved on purpose so no table of names has to ship in the mod. An
//! address outside the client's own image carries a trailing `*`: other loaded
//! modules register script API too, and one of theirs can rank high enough to
//! read as a target when nothing in this tree could ever reimplement it.
//!
//! When the armed line carries `api spans sampled 1-in-N`, this machine's
//! counter reads are slow enough to dominate the spans they bracket, so only
//! one C call in N is timed and its span stands for its cohort at that
//! weight. Every call is still counted exactly; per-entry totals and averages
//! stay unbiased, `max` describes the sampled subset, and a hitch long enough
//! for the quarantine is caught one time in N.
//!
//! A `rejects N` in the header means the counter came back below where a span
//! started N times, and those spans were dropped rather than accumulated. The
//! clause is absent when nothing was dropped, so its presence is the signal:
//! a window carrying one is under-reporting by whatever those spans held.
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
    ffi::CStr,
    fmt::Write as _,
    sync::{
        LazyLock, Mutex, PoisonError,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
};

use rustc_hash::FxHashMap;

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
/// Rows shown in the script-API top list, in both the window and the total pass.
///
/// Wider than the name tables because the api table is what a reimplementation
/// picks its targets from, and a row here is an eight-character address rather
/// than a frame name. Eight rows leave around a quarter of measured API time
/// folded into the tail, which is enough to hide a target from the ranking
/// entirely; this deep the tail holds a few per cent.
const TOP_API: usize = 48;
/// Handler-table size cap; past it new names fall into the overflow row.
const HANDLER_TABLE_CAP: usize = 512;
/// Script-API table size cap; past it new addresses fall into the overflow row.
///
/// Larger than the handler cap because the key is an address rather than a
/// stored name, so an entry costs a fraction of a handler row, and because a
/// bound that binds makes the tail unrankable however many rows are printed.
const API_TABLE_CAP: usize = 2_048;
/// Microseconds past which a C-API span is quarantined rather than ranked.
///
/// A span this size inside a handler body is one of two things: a real hitch a
/// player felt, or a counter artifact — spans of whole seconds have been
/// recorded inside bodies whose own bracket measured milliseconds, with the
/// frame counts running uninterrupted, which no monotonic counter can produce.
/// The two cannot be told apart here, but both poison the ranked table and the
/// api/vm split if folded in (`api` exceeding `bodies` is the visible symptom),
/// so past this bound the span goes to its own table and the body-level
/// numbers arbitrate offline: a real hitch shows up in the body histogram and
/// the handler max, an artifact does not.
const API_SUSPECT_US: u64 = 100_000;
/// Suspect-table size cap; past it new addresses fall into the overflow row.
///
/// Quarantined spans are rare by definition — the cap exists so a machine that
/// produces them wholesale cannot grow a table on the hot path.
const API_SUSPECT_CAP: usize = 64;
/// Counter-read cost, in nanoseconds, past which C-API spans are sampled.
///
/// A timed span is three counter reads and the median C call is under a
/// microsecond, so on a counter whose reads cost tens of nanoseconds each,
/// timing every call makes the clock the largest single cost on the thread
/// being measured: an armed session on such a machine spends more of its
/// frame inside the counter's emulation than inside any function it ranks.
/// Below this bound the reads are noise and every call is timed.
const API_SAMPLE_READ_COST_NS: u64 = 25;
/// Span-sampling stride on a slow counter: one C-API call in this many.
///
/// Cuts the read rate by the same factor while every call is still counted
/// exactly. Eight keeps a per-second window statistically dense (the hot
/// entries run tens of thousands of calls a second, so each still gets
/// thousands of timed spans) while removing seven-eighths of the observer
/// effect on the machines that need it.
const API_SAMPLE_STRIDE: u32 = 8;
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
        // Force the counter's read-cost calibration here rather than leaving it
        // to the first C call, which would run it inside a handler body and
        // bill a millisecond of it to whatever script was unlucky. The armed
        // line prints the result: a counter read is emulated on this platform
        // and its cost varies by orders of magnitude between machines, which
        // decides how much of every span below is the clock rather than the
        // client. A reader needs that number to judge the rest of the log.
        // Milli-ticks through the linear microsecond scale is nanoseconds.
        let read_cost_ns = ticks_to_us(wow_shared::tsc::read_cost_milli_ticks());
        let stride = *API_SAMPLE;
        if stride > 1 {
            log::debug!(
                target: "wow::events",
                "event gauge armed, counter read cost {read_cost_ns} ns, \
                 api spans sampled 1-in-{stride}",
            );
        } else {
            log::debug!(
                target: "wow::events",
                "event gauge armed, counter read cost {read_cost_ns} ns",
            );
        }
    }
    armed
});

/// Ticks a window's C-API spans carry purely as counter-read latency.
///
/// One read per span, priced by the counter's own calibration — the same place
/// the tick-to-millisecond scale comes from, since both are properties of the
/// counter rather than of this gauge. Per call even under sampling: a timed
/// span carries one read's latency and is scaled by the stride, one timed span
/// stands for stride calls, so the inflation per counted call is unchanged.
fn clock_overhead(calls: u64) -> u64 {
    calls.saturating_mul(wow_shared::tsc::read_cost_milli_ticks()) / 1_000
}

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
    /// The part of `ticks` spent inside the client's C script API.
    ///
    /// Only the per-addon table fills this: it is what says whether an addon is
    /// expensive because of what it asks the client to do or because of the Lua
    /// it runs, and those want different work.
    api_ticks: u64,
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
///
/// Every map here is fed under the state mutex from the dispatch and API hot
/// paths, keyed by an address, an id or a short fixed-size name (nothing an
/// adversary chooses), so they all use `FxHashMap`: the standard hasher's
/// collision resistance buys nothing here and its cost lands inside the very
/// spans this gauge exists to measure.
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
    events: FxHashMap<i32, EventRow>,
    handlers: FxHashMap<NameBuf, Stat>,
    /// Ticks inside the handler bodies themselves, outermost only.
    ///
    /// The window total minus this is the dispatch and argument-binding
    /// machinery around them.
    body_ticks: u64,
    /// Bodies behind `body_ticks`, so the line can state a per-body cost.
    body_calls: u64,
    /// The part of `body_ticks` spent inside the client's own C script API.
    ///
    /// `body_ticks` minus this and minus `body_gauge_ticks` is the interpreter
    /// and the Lua runtime under it — the other half of the split.
    api_ticks: u64,
    /// C-API calls behind `api_ticks`, so the line can state a per-call cost.
    api_calls: u64,
    /// Calls that only prepared a Lua closure, so the VM side has a call count.
    ///
    /// The bound on what this gauge can be overstating the VM by: every one of
    /// these paid a detour that no measurement from inside can see.
    vm_calls: u64,
    /// Ticks the API measurement itself spent, inside the bodies it measured.
    ///
    /// Kept apart from `gauge_ticks` because that one sits outside the bodies
    /// and is already subtracted from `machinery`; subtracting this there too
    /// would take the same time off twice.
    body_gauge_ticks: u64,
    /// Per-C-function cost, keyed by the function's entry address.
    ///
    /// Addresses rather than names: the mapping is a build-side artifact, and
    /// resolving it offline keeps a few hundred strings out of the mod.
    api: FxHashMap<usize, Stat>,
    /// C functions dropped on the API-table cap (reported, never silent).
    api_dropped: u64,
    /// Ticks behind `api_dropped`, so the overflow row carries a cost too.
    api_dropped_ticks: u64,
    /// Quarantined C-API spans, keyed like `api` (see [`API_SUSPECT_US`]).
    ///
    /// Kept out of `api_ticks` and `api_calls` entirely: the split and the
    /// ranked table stay meaningful, and this table says what was set aside.
    api_suspect: FxHashMap<usize, Stat>,
    /// Suspect spans dropped on the suspect-table cap (reported, never silent).
    api_suspect_dropped: u64,
    /// Ticks behind `api_suspect_dropped`, so the overflow carries a cost too.
    api_suspect_dropped_ticks: u64,
    /// Ticks this gauge itself spent inside the span it is measuring.
    ///
    /// Resolving a handler's owner and folding it into the tables happens
    /// between the dispatch wrapper's two clock reads, so without measuring it
    /// the cost would land in `machinery` and inflate the very number that
    /// says whether the invoke path is worth optimizing.
    gauge_ticks: u64,
    /// Body ticks by how long the body took, bucketed by `BODY_BUCKET_US`.
    body_hist_ticks: [u64; BODY_BUCKETS],
    /// Bodies per bucket, so a bucket states both its mass and its count.
    body_hist_calls: [u64; BODY_BUCKETS],
    /// Per-addon body cost, keyed by the folder that owns the script.
    owners: FxHashMap<NameBuf, Stat>,
    /// Handler names dropped on the table cap (reported, never silent).
    dropped: u64,
    /// Ticks behind `dropped`, so the overflow row carries a cost too.
    dropped_ticks: u64,
    /// Spans this window discarded because the counter went backwards.
    ///
    /// Printed only when non-zero, so the ordinary line is unchanged and a
    /// window that lost a span says so instead of quietly under-reporting.
    span_rejects: u64,
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
    frames: Option<FxHashMap<NameBuf, NameBuf>>,
    /// Chunk-to-addon memo, keyed by a hash of the chunk name.
    ///
    /// A per-invoke lookup whose key is already a hash, so hashing it again
    /// with anything heavier would be pure cost.
    ///
    /// Keyed by content rather than by the string object, so a reload that
    /// re-interns every chunk at a fresh address reuses the entries instead of
    /// doubling them, and a recycled address cannot inherit an owner.
    owners: FxHashMap<u64, NameBuf>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| {
    let t = wow_shared::tsc::rdtsc();
    Mutex::new(State {
        window_start: t,
        cumulative_emit: t,
        window: Tables::default(),
        cumulative: Tables::default(),
        frames: None,
        owners: FxHashMap::default(),
    })
});

/// Whether the gauge is armed (cheap after the first call).
pub fn armed() -> bool {
    *ARMED
}

/// Nesting depth of timed handler bodies; only the outermost is measured.
static BODY_DEPTH: AtomicU32 = AtomicU32::new(0);

/// Running total of C-API ticks, read as a delta across a handler body.
///
/// A free-running counter rather than per-body state: `luaD_precall` knows
/// nothing about which body it is under, and the difference across the body's
/// own bracket answers that without either side having to look the other up.
static API_TICKS: AtomicU64 = AtomicU64::new(0);

/// Running total of what the API measurement cost, on the same delta scheme.
static API_SELF_TICKS: AtomicU64 = AtomicU64::new(0);

/// Running count of calls that only prepared a Lua closure, same delta scheme.
///
/// These are the calls the split has nothing to measure, and counting them is
/// what bounds the one cost that cannot be measured from inside this module:
/// the detour into it. Multiply by the per-call detour cost and that is the
/// most the VM side can be overstated by.
static VM_CALLS: AtomicU64 = AtomicU64::new(0);

/// Add to one of the running counters, without a read-modify-write.
///
/// Both counters are written from `luaD_precall` and read from the body bracket
/// around it, which is the same thread in both cases: the client runs one Lua
/// state on the game thread. A load, an add and a store are three cheap
/// instructions where a 64-bit atomic add on this target is a compare-exchange
/// loop, and this sits on the hottest path the gauge touches. The atomic type
/// is kept for the shared-mutability it grants, not for the arithmetic.
fn add_ticks(counter: &AtomicU64, ticks: u64) {
    counter.store(
        counter.load(Ordering::Relaxed).wrapping_add(ticks),
        Ordering::Relaxed,
    );
}

/// Spans rejected because the counter came back below where it started.
static SPAN_REJECTS: AtomicU32 = AtomicU32::new(0);

/// The address range the client's own image occupies.
///
/// Other loaded modules register script API of their own, and those entries
/// rank in the same table as the client's. Without the range there is nothing
/// in a row to say which is which — an address outside the image reads like any
/// other hex number, and a reimplementation cannot target one.
static HOST_IMAGE: LazyLock<std::ops::Range<usize>> = LazyLock::new(|| {
    let base = wow_hook::host_image_base();
    base..base + wow_hook::host_image_size()
});

/// Ticks between two counter reads, or zero if the counter did not advance.
///
/// A read that returns below its predecessor turns the subtraction into a value
/// near `u64::MAX`, and every accumulator it reaches then carries that for the
/// rest of the window — which surfaces as the one constant `ticks_to_us`
/// saturates to, in a line where nothing else looks wrong. Rejecting the span
/// where it is formed keeps one bad read inside one call, and the counter is
/// what stops that being silent: a window that rejected nothing is a window
/// whose totals are known to be sound, not merely assumed to be.
fn span(t0: u64, t1: u64) -> u64 {
    if t1 >= t0 {
        return t1 - t0;
    }
    SPAN_REJECTS.fetch_add(1, Ordering::Relaxed);
    0
}

/// Nesting depth of timed C-API calls; only the outermost is measured.
///
/// A C function that calls back into Lua reaches `luaD_precall` again, and the
/// time is already inside the outer call's span.
static API_DEPTH: AtomicU32 = AtomicU32::new(0);

/// C-API span-sampling stride, resolved once beside the arming decision.
///
/// 1 on a fast counter, so every C call is timed. Past
/// [`API_SAMPLE_READ_COST_NS`] the reads themselves dominate what they
/// bracket, and one call in [`API_SAMPLE_STRIDE`] carries the clock for all
/// of them.
static API_SAMPLE: LazyLock<u32> = LazyLock::new(|| {
    let read_cost_ns = ticks_to_us(wow_shared::tsc::read_cost_milli_ticks());
    if read_cost_ns >= API_SAMPLE_READ_COST_NS {
        API_SAMPLE_STRIDE
    } else {
        1
    }
});

/// Calls seen by the sampling pick, on the same single-thread store scheme.
static API_SAMPLE_SEEN: AtomicU32 = AtomicU32::new(0);

/// Whether this C-API call's span is timed, and at what weight.
///
/// Zero for a call that is only counted; otherwise the factor its measured
/// span stands in for. The pick multiplies a running counter by a large odd
/// constant and keeps the low fraction of the hashed range, so the timed
/// subset spreads pseudo-randomly through the call stream instead of striding
/// in lockstep with an addon loop that calls a fixed cycle of functions;
/// a plain modulo would time the same function forever in such a loop.
fn api_sample_weight() -> u64 {
    let stride = *API_SAMPLE;
    if stride == 1 {
        return 1;
    }
    let seen = API_SAMPLE_SEEN.load(Ordering::Relaxed);
    API_SAMPLE_SEEN.store(seen.wrapping_add(1), Ordering::Relaxed);
    if seen.wrapping_mul(0x9E37_79B9) < u32::MAX / stride {
        u64::from(stride)
    } else {
        0
    }
}

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
fn build_frame_index() -> FxHashMap<NameBuf, NameBuf> {
    let mut index = FxHashMap::default();
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
pub fn time_body<T>(chunk: impl FnOnce() -> (usize, u32), body: impl FnOnce() -> T) -> T {
    if !armed() {
        return body();
    }
    let nested = BODY_DEPTH.fetch_add(1, Ordering::Relaxed) != 0;
    if nested {
        let out = body();
        BODY_DEPTH.fetch_sub(1, Ordering::Relaxed);
        return out;
    }
    // Everything between here and the body, and between the body and the
    // return, is this gauge's own work sitting inside the span the dispatch
    // wrapper is timing. Bracket it so it can be reported and subtracted
    // rather than silently inflating `machinery`.
    let gauge_in = wow_shared::tsc::rdtsc();
    let chunk = chunk();
    let api_in = API_TICKS.load(Ordering::Relaxed);
    let api_self_in = API_SELF_TICKS.load(Ordering::Relaxed);
    let vm_calls_in = VM_CALLS.load(Ordering::Relaxed);
    let t0 = wow_shared::tsc::rdtsc();
    let out = body();
    let t1 = wow_shared::tsc::rdtsc();
    let dt = span(t0, t1);
    BODY_DEPTH.fetch_sub(1, Ordering::Relaxed);
    // A script error inside a C function does not return through `precall` — it
    // longjmps to the protected call that just returned here, so that call's
    // exit bookkeeping never ran and its depth counter is still raised. This is
    // the point where no C call can be outstanding, so it is where the counter
    // is known to be zero rather than assumed to be. Without this, one addon
    // error would silently end API accounting for the rest of the session.
    API_DEPTH.store(0, Ordering::Relaxed);
    // What the C API cost inside this body, and what measuring it cost. Both
    // counters free-run, so the difference across the body is this body's share.
    let api_dt = API_TICKS.load(Ordering::Relaxed).saturating_sub(api_in);
    let api_self_dt = API_SELF_TICKS
        .load(Ordering::Relaxed)
        .saturating_sub(api_self_in);
    let mut st = state();
    st.window.body_ticks += dt;
    st.window.body_calls += 1;
    st.window.api_ticks += api_dt;
    st.window.body_gauge_ticks += api_self_dt;
    st.window.vm_calls += VM_CALLS.load(Ordering::Relaxed).saturating_sub(vm_calls_in);
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
    stat.api_ticks += api_dt;
    let gauge_out = wow_shared::tsc::rdtsc();
    st.window.gauge_ticks += span(gauge_in, t0) + span(t1, gauge_out);
    drop(st);
    out
}

/// `luaD_precall` wrapper — time the calls that land in the client's C API.
///
/// Every Lua-level call arrives here, and the two branches are what the split
/// needs: a Lua closure is only prepared (the interpreter runs its body
/// afterwards, on the VM side of the ledger), while a C closure is called
/// outright from inside this function. Reading the closure up front tells the
/// two apart before any clock is touched, so a pure-Lua call pays two relaxed
/// loads and the classification, and nothing else.
///
/// The hook is installed only when the gauge is armed, so there is no unarmed
/// path to keep cheap here — the unarmed client never reaches this code at all.
/// Calls made outside a timed handler body are skipped: `api` is quoted against
/// `bodies`, and counting the client's own start-up scripts in it would put the
/// two on different scales.
///
/// The clock is read only around a timed C span, never on the Lua branch: an
/// emulated read costs orders of magnitude more than the classification it
/// would bracket, so measuring the cheap branch was itself the largest cost
/// on it. What the wrapper leaves unmeasured there (the detour, the
/// classification, one counted store) is bounded by `vm_calls` times a
/// per-call constant, which is how the ledger already quotes the one cost no
/// hook can see from inside.
///
/// A C span carries its own bookkeeping between the second read and a third:
/// that is this gauge's work sitting inside the body the dispatch wrapper is
/// timing, and bracketing it keeps it out of `machinery`. When the sampling
/// stride is active the bracket is scaled like the span it follows, so the
/// untimed cohort's bookkeeping (strictly less of it, one counted store per
/// call) is estimated slightly high rather than dropped.
pub fn precall(l: i32, func: i32) -> i32 {
    let original = super::symbols::originals::lua_d_precall__6f6050();
    if BODY_DEPTH.load(Ordering::Relaxed) == 0 || API_DEPTH.load(Ordering::Relaxed) != 0 {
        return original(l, func);
    }
    let Some(entry) = c_entry(func) else {
        add_ticks(&VM_CALLS, 1);
        return original(l, func);
    };
    let weight = api_sample_weight();
    if weight == 0 {
        API_DEPTH.store(1, Ordering::Relaxed);
        let out = original(l, func);
        API_DEPTH.store(0, Ordering::Relaxed);
        record_api_counted(entry);
        return out;
    }
    API_DEPTH.store(1, Ordering::Relaxed);
    let t0 = wow_shared::tsc::rdtsc();
    let out = original(l, func);
    let t1 = wow_shared::tsc::rdtsc();
    API_DEPTH.store(0, Ordering::Relaxed);
    // Raw here, counter-read latency and all: the correction is a fraction of a
    // tick, so it is applied once against the call count when the window is
    // reported rather than truncated to zero on every call.
    let dt = span(t0, t1);
    if ticks_to_us(dt) >= API_SUSPECT_US {
        record_api_suspect(entry, dt);
    } else {
        let weighted = dt.saturating_mul(weight);
        add_ticks(&API_TICKS, weighted);
        record_api(entry, dt, weighted);
    }
    let gauge_out = wow_shared::tsc::rdtsc();
    add_ticks(&API_SELF_TICKS, span(t1, gauge_out).saturating_mul(weight));
    out
}

/// The C function a call is about to run, or `None` for anything else.
///
/// `func` points at the value being called. The offsets are the ones the
/// collector and the chunk walk already use: the value's tag at `+0x0` (6 is a
/// function) and its object at `+0x8`, then a closure's `isC` byte at `+0x6`
/// and, at `+0xc`, the C entry point for a C closure (where a Lua closure keeps
/// its proto instead).
///
/// A value that is not a function can still end up called, through the call
/// metamethod the original resolves internally. That case reads as VM time,
/// which is the safer way round: it under-reports the API rather than billing it
/// for a function this walk never saw.
fn c_entry(func: i32) -> Option<usize> {
    const LUA_TFUNCTION: i32 = 6;
    if func == 0 {
        return None;
    }
    let value = func.cast_unsigned() as usize;
    // SAFETY: `func` is the live stack slot the original is about to call; a
    // value's tag is the word at `+0x0`.
    if unsafe { *(value as *const i32) } != LUA_TFUNCTION {
        return None;
    }
    // SAFETY: a function value's object pointer sits at `+0x8`.
    let closure = unsafe { *((value + 0x8) as *const usize) };
    if closure == 0 {
        return None;
    }
    // SAFETY: `isC` is the byte at `+0x6` of any closure.
    if unsafe { *((closure + 0x6) as *const u8) } == 0 {
        return None;
    }
    // SAFETY: a C closure's function pointer is at `+0xc`, after the header.
    let entry = unsafe { *((closure + 0xc) as *const usize) };
    (entry != 0).then_some(entry)
}

/// Record one timed C-API call against its entry address.
///
/// `dt` is the span as measured and `weighted` is what it stands for: the same
/// value at stride 1, the whole cohort's estimate under sampling. Totals carry
/// the weight; `max` keeps the span a clock actually saw, because a maximum of
/// estimates would report a hitch nobody measured.
///
/// Deliberately does not emit: this runs inside a handler body, and closing the
/// window here would print a body that has not finished. The invoke wrappers
/// emit often enough that a window can never run long.
fn record_api(entry: usize, dt: u64, weighted: u64) {
    let mut st = state();
    st.window.api_calls += 1;
    let over_cap = st.window.api.len() >= API_TABLE_CAP && !st.window.api.contains_key(&entry);
    if over_cap {
        st.window.api_dropped += 1;
        st.window.api_dropped_ticks += weighted;
    } else {
        let stat = st.window.api.entry(entry).or_default();
        stat.count += 1;
        stat.ticks += weighted;
        stat.max_ticks = stat.max_ticks.max(dt);
    }
    drop(st);
}

/// Count one untimed C-API call against its entry address.
///
/// The sampling stride's other branch: the call is real and the counts must
/// stay exact, but no clock was read, so nothing else moves. Runs the same
/// cap rule as the timed path, so a capped table drops timed and untimed
/// calls alike instead of skewing the overflow row toward one kind.
fn record_api_counted(entry: usize) {
    let mut st = state();
    st.window.api_calls += 1;
    let over_cap = st.window.api.len() >= API_TABLE_CAP && !st.window.api.contains_key(&entry);
    if over_cap {
        st.window.api_dropped += 1;
    } else {
        st.window.api.entry(entry).or_default().count += 1;
    }
    drop(st);
}

/// Quarantine one over-threshold C-API span (see [`API_SUSPECT_US`]).
///
/// Deliberately touches none of the ordinary api accumulators: a span set
/// aside here must not move `api_calls` or the free-running total either, or
/// the per-call averages and the body-level split inherit exactly the
/// distortion the quarantine exists to remove.
fn record_api_suspect(entry: usize, dt: u64) {
    let mut st = state();
    let over_cap = st.window.api_suspect.len() >= API_SUSPECT_CAP
        && !st.window.api_suspect.contains_key(&entry);
    if over_cap {
        st.window.api_suspect_dropped += 1;
        st.window.api_suspect_dropped_ticks += dt;
    } else {
        let stat = st.window.api_suspect.entry(entry).or_default();
        stat.count += 1;
        stat.ticks += dt;
        stat.max_ticks = stat.max_ticks.max(dt);
    }
    drop(st);
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
    let dt = span(t0, wow_shared::tsc::rdtsc());
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
    let dt = span(t0, wow_shared::tsc::rdtsc());
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
    let dt = span(t0, wow_shared::tsc::rdtsc());
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
///
/// Emission is the most expensive thing the gauge does — formatting several
/// ranked tables and writing them out — and it happens wherever the window
/// happened to expire. A nested invoke can trigger it, which puts a stdout write
/// inside a handler body that is still being timed, so what it cost is charged
/// to the gauge's own buckets rather than to the script or the dispatch it
/// interrupted. The charge lands on the window that follows, since the one it
/// reported has already been printed.
fn maybe_emit(st: &mut State) {
    let now = wow_shared::tsc::rdtsc();
    let window_ms = super::hooks::clock_ticks_to_ms(span(st.window_start, now));
    if window_ms < WINDOW_MS {
        return;
    }
    // Rejections are counted on the hot path, which cannot take this lock, so
    // the running counter is drained here — once per window, into the window it
    // belongs to, and from there into the cumulative tables by the ordinary
    // merge.
    st.window.span_rejects += u64::from(SPAN_REJECTS.swap(0, Ordering::Relaxed));
    emit_tables(&st.window, window_ms, TOP_PER_SECOND, "");
    let window = std::mem::take(&mut st.window);
    merge(&mut st.cumulative, window);
    st.window_start = now;
    let cum_ms = super::hooks::clock_ticks_to_ms(span(st.cumulative_emit, now));
    if cum_ms >= CUMULATIVE_MS {
        emit_tables(&st.cumulative, cum_ms, TOP_CUMULATIVE, "total ");
        // A memo that never hits looks exactly like one that works: its target
        // is reached from a script, not through the dispatch this gauge
        // measures, so no table above can show it.
        super::getname::emit_cumulative();
        super::script_method::emit_cumulative();
        super::seam_probe::emit_cumulative();
        super::unitxp::emit_cumulative();
        st.cumulative_emit = now;
    }
    let spent = span(now, wow_shared::tsc::rdtsc());
    if BODY_DEPTH.load(Ordering::Relaxed) == 0 {
        if DEPTH.load(Ordering::Relaxed) != 0 {
            st.window.gauge_ticks += spent;
        }
    } else {
        st.window.body_gauge_ticks += spent;
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
    cum.gauge_ticks += w.gauge_ticks;
    cum.api_ticks += w.api_ticks;
    cum.api_calls += w.api_calls;
    cum.vm_calls += w.vm_calls;
    cum.body_gauge_ticks += w.body_gauge_ticks;
    cum.api_dropped += w.api_dropped;
    cum.api_dropped_ticks += w.api_dropped_ticks;
    cum.api_suspect_dropped += w.api_suspect_dropped;
    cum.api_suspect_dropped_ticks += w.api_suspect_dropped_ticks;
    for i in 0..BODY_BUCKETS {
        cum.body_hist_ticks[i] += w.body_hist_ticks[i];
        cum.body_hist_calls[i] += w.body_hist_calls[i];
    }
    cum.dropped += w.dropped;
    cum.dropped_ticks += w.dropped_ticks;
    cum.span_rejects += w.span_rejects;
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
        dst.api_ticks += stat.api_ticks;
    }
    for (entry, stat) in w.api {
        if cum.api.len() >= API_TABLE_CAP && !cum.api.contains_key(&entry) {
            cum.api_dropped += stat.count;
            cum.api_dropped_ticks += stat.ticks;
            continue;
        }
        let dst = cum.api.entry(entry).or_default();
        dst.count += stat.count;
        dst.ticks += stat.ticks;
        dst.max_ticks = dst.max_ticks.max(stat.max_ticks);
    }
    for (entry, stat) in w.api_suspect {
        if cum.api_suspect.len() >= API_SUSPECT_CAP && !cum.api_suspect.contains_key(&entry) {
            cum.api_suspect_dropped += stat.count;
            cum.api_suspect_dropped_ticks += stat.ticks;
            continue;
        }
        let dst = cum.api_suspect.entry(entry).or_default();
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
        // A row with no time has no API share either, so the guard against
        // dividing by it can just as well be part of the divisor.
        let _ = write!(line, ", api {}%", stat.api_ticks * 100 / stat.ticks.max(1));
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

/// Emit the client's C script API, ranked by cost, with the tail folded in.
///
/// Keyed by the function's entry address, because the names live in a build-side
/// map rather than in the mod: an address is what a reader can resolve offline,
/// and a table of a few hundred strings would have to ship to say no more.
/// Ranked [`TOP_API`] deep rather than to the shared cap, for the reason given
/// there.
fn emit_api(t: &Tables, label: &str) {
    if t.api.is_empty() {
        return;
    }
    let mut rows: Vec<(&usize, &Stat)> = t.api.iter().collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.1.ticks));
    let mut line = format!("{label}api:");
    for (entry, stat) in rows.iter().take(TOP_API) {
        let mark = if HOST_IMAGE.contains(entry) { "" } else { "*" };
        let _ = write!(line, " {entry:#010x}{mark} x{} ", stat.count);
        push_ms(&mut line, stat.ticks);
        line.push_str(" ms;");
    }
    if let Some(tail) = rows.get(TOP_API..).filter(|t| !t.is_empty()) {
        let calls: u64 = tail.iter().map(|r| r.1.count).sum();
        let ticks: u64 = tail.iter().map(|r| r.1.ticks).sum();
        let _ = write!(line, " +{} more x{calls} ", tail.len());
        push_ms(&mut line, ticks);
        line.push_str(" ms;");
    }
    if t.api_dropped > 0 {
        let _ = write!(line, " (+{} calls past the address cap, ", t.api_dropped);
        push_ms(&mut line, t.api_dropped_ticks);
        line.push_str(" ms)");
    }
    log::debug!(target: "wow::events", "{line}");
}

/// Emit the quarantined C-API spans, when any exist (see [`API_SUSPECT_US`]).
///
/// One row per entry address with count, total and worst span, so the reader
/// can decide per address which of the two meanings applies. Printed after the
/// ranked table it was kept out of.
fn emit_api_suspect(t: &Tables, label: &str) {
    if t.api_suspect.is_empty() && t.api_suspect_dropped == 0 {
        return;
    }
    let mut rows: Vec<(&usize, &Stat)> = t.api_suspect.iter().collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.1.ticks));
    let mut line = format!("{label}api suspect:");
    for (entry, stat) in rows {
        let mark = if HOST_IMAGE.contains(entry) { "" } else { "*" };
        let _ = write!(line, " {entry:#010x}{mark} x{} ", stat.count);
        push_ms(&mut line, stat.ticks);
        line.push_str(" ms (max ");
        push_ms(&mut line, stat.max_ticks);
        line.push_str(");");
    }
    if t.api_suspect_dropped > 0 {
        let _ = write!(line, " (+{} spans past the cap, ", t.api_suspect_dropped);
        push_ms(&mut line, t.api_suspect_dropped_ticks);
        line.push_str(" ms)");
    }
    log::debug!(target: "wow::events", "{line}");
}

/// Build the header: the window total, split both ways, then the api/vm split.
fn header_line(t: &Tables, span_ms: u64, label: &str) -> String {
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
    // The C spans were accumulated raw, so lift one counter read per call off
    // `api` and onto the gauge. Only the split between the two moves: the time
    // was really spent inside the bodies, it just was not spent by the client.
    let read_ticks = clock_overhead(t.api_calls);
    line.push_str(" ms (api ");
    push_ms(&mut line, t.api_ticks.saturating_sub(read_ticks));
    let _ = write!(line, " in {} calls, vm ", t.api_calls);
    push_ms(
        &mut line,
        t.body_ticks
            .saturating_sub(t.api_ticks)
            .saturating_sub(t.body_gauge_ticks),
    );
    let _ = write!(line, " in {} calls), machinery ", t.vm_calls);
    push_ms(
        &mut line,
        t.total_ticks
            .saturating_sub(t.body_ticks)
            .saturating_sub(t.gauge_ticks),
    );
    line.push_str(" + gauge ");
    push_ms(&mut line, t.gauge_ticks + t.body_gauge_ticks + read_ticks);
    if t.span_rejects > 0 {
        let _ = write!(line, ", rejects {}", t.span_rejects);
    }
    line.push_str(") |");
    line
}

/// Emit one events line and one handlers line for a window.
fn emit_tables(t: &Tables, span_ms: u64, top: usize, label: &str) {
    if t.signals == 0 && t.handler_calls == 0 {
        return;
    }
    let mut line = header_line(t, span_ms, label);
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
    emit_api(t, label);
    emit_api_suspect(t, label);
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
