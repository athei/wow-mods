//! Armed counters that price the candidate fork seams before any is taken.
//!
//! Each family answers one question a static read cannot: is the client's
//! own two-thread bone-animation arm (`[view+4] & 4`) live on this stack;
//! how many animate-eligible roots does a draw-list pass walk and how is
//! their cost distributed (the measured bound on what a worker split could
//! recover); does any model instance get visited twice in one frame through
//! the `+0x40` stamp (the aliasing hazard a fork must exclude); what a
//! forked animate pass pays at the join against the per-root cost its lanes
//! absorb; where a draw-list build's game-thread time goes phase by phase,
//! which is what bounds any attempt to overlap that join with the phases
//! after it; which of the five steps inside the finalize phase holds that
//! phase's time, over how many records and how many comparisons, which is
//! what ranks the levers against the largest phase of the build; how many
//! dynamic-buffer lock cycles the particle and
//! precipitation paths pay per session; what grain the per-emitter particle
//! draws offer a fork (draws per second, live particles per draw with a
//! histogram whose buckets end at 4/16/64/256, the quad build's cost spread,
//! and how much of an up-front output reservation the emitted counts would
//! waste) and what the forked build then hits, misses and waits for; and which
//! entry family feeds the collision/trace cluster and at what call rate
//! (per-unit ground movement, swept-volume queries, the terrain and M2-scene
//! arms of trace lines, spatial-node geometry collection, the chunk WMO and
//! doodad queries, and grid-vertex rebuilds), which is what apportions a
//! sampler's collision share when the callers above these entries carry no
//! frame pointers.
//!
//! Counters ride the counter arm (`wow::perf` at debug), which is NOT the
//! event gauge's: these cost nothing to keep, that one does not, so asking
//! for these numbers must not buy it. Unarmed, every entry point here is a
//! load and a branch. All writers are the
//! game thread except the two bone-animation entry counters, which the
//! client's own worker thread can also reach and which are therefore declared
//! as the locked-add shape.

use super::tally::{Accum, Armed, Counter, SharedCounter};

static ANIM_PASSES: Counter = Counter::zero();
static ANIM_MULTI: Counter = Counter::zero();
static ANIM_WORKER_ARM: Counter = Counter::zero();
static ROOTS_SUM: Accum = Accum::zero();
static ROOTS_MAX: Counter = Counter::zero();
static LIST_SUM: Accum = Accum::zero();
static LIST_MAX: Counter = Counter::zero();
static ROOT_TICKS_SUM: Accum = Accum::zero();
static ROOT_TICKS_MAX: Accum = Accum::zero();

static ANIM_CALLS: SharedCounter = SharedCounter::zero();
static REPEAT_VISITS: SharedCounter = SharedCounter::zero();

static ANIM_PAR_PASSES: Counter = Counter::zero();
static ANIM_PAR_GATED: Counter = Counter::zero();
static ANIM_PAR_ROOTS: Accum = Accum::zero();
static ANIM_PAR_WAIT_TICKS_SUM: Accum = Accum::zero();
static ANIM_PAR_WAIT_TICKS_MAX: Accum = Accum::zero();
static ANIM_PAR_WORKER_TICKS: Accum = Accum::zero();
static ANIM_PAR_FORK_TICKS_SUM: Accum = Accum::zero();
static ANIM_PAR_STALLS: Accum = Accum::zero();
static ANIM_PAR_HELPED: Accum = Accum::zero();

static BDL_PASSES: Counter = Counter::zero();
static BDL_TOTAL_TICKS: Accum = Accum::zero();
static BDL_TOTAL_TICKS_MAX: Accum = Accum::zero();
static BDL_PROLOGUE_TICKS: Accum = Accum::zero();
static BDL_COLLECT_TICKS: Accum = Accum::zero();
static BDL_ANIMATE_TICKS: Accum = Accum::zero();
static BDL_PARTICLES_TICKS: Accum = Accum::zero();
static BDL_REFRESH_TICKS: Accum = Accum::zero();
static BDL_SPATIAL_TICKS: Accum = Accum::zero();
static BDL_CHILD_TICKS: Accum = Accum::zero();
static BDL_FINALIZE_TICKS: Accum = Accum::zero();

static FIN_PASSES: Counter = Counter::zero();
static FIN_B0_IN_SUM: Accum = Accum::zero();
static FIN_B0_IN_MAX: Counter = Counter::zero();
static FIN_B0_OUT_SUM: Accum = Accum::zero();
static FIN_RUNS_SUM: Accum = Accum::zero();
static FIN_SPILL_SUM: Accum = Accum::zero();
static FIN_PROBE_STEPS: Accum = Accum::zero();
static FIN_CMP_CALLS: Accum = Accum::zero();
static FIN_TRANS_SUM: Accum = Accum::zero();
static FIN_TRANS_MAX: Counter = Counter::zero();
static FIN_OPAQUE_SUM: Accum = Accum::zero();
static FIN_OPAQUE_MAX: Counter = Counter::zero();
static FIN_TRANS_CMPS: Accum = Accum::zero();
static FIN_TRANS_TAG: Accum = Accum::zero();
static FIN_OPAQUE_CMPS: Accum = Accum::zero();
static FIN_DEDUP_TICKS: Accum = Accum::zero();
static FIN_TEXSORT_TICKS: Accum = Accum::zero();
static FIN_MERGE_TICKS: Accum = Accum::zero();
static FIN_TRANS_TICKS: Accum = Accum::zero();
static FIN_OPAQUE_TICKS: Accum = Accum::zero();

static CLIP_CALLS: Accum = Accum::zero();
static CLIP_VERTS: Accum = Accum::zero();
static CLIP_HIST: [Counter; CLIP_HIST_EDGES.len() + 1] = [
    Counter::zero(),
    Counter::zero(),
    Counter::zero(),
    Counter::zero(),
];

/// Upper edges for the clip's vertex-count histogram; one bucket past.
///
/// The polygon is seeded at 3 and grows by at most one vertex per plane, so
/// the interesting question is how much of the distribution sits at or below
/// four — which is what says whether a four-wide vector pass over the distance
/// loop would have lanes to fill.
const CLIP_HIST_EDGES: [u32; 3] = [3, 4, 8];

static CLIP_KEPT: Accum = Accum::zero();
static CLIP_DROPPED: Accum = Accum::zero();
static CLIP_CUT: Accum = Accum::zero();

static FACE_PASSES: Accum = Accum::zero();
static FACE_VISITED: Accum = Accum::zero();
static FACE_BACK: Accum = Accum::zero();
static FACE_WIPED: Accum = Accum::zero();
static FACE_SWEPT: Accum = Accum::zero();

static PARTICLE_LOCKS: Counter = Counter::zero();
static RAIN_LOCKS: Counter = Counter::zero();

static TRACE_GROUND: Counter = Counter::zero();
static TRACE_SWEEP: Counter = Counter::zero();
static TRACE_TERRAIN: Counter = Counter::zero();
static TRACE_SCENE: Counter = Counter::zero();
static TRACE_GEOMETRY: Counter = Counter::zero();
static TRACE_WMO_QUERY: Counter = Counter::zero();
static TRACE_DOODAD_QUERY: Counter = Counter::zero();
static TRACE_GRID_BUILD: Counter = Counter::zero();

static PART_DRAWS: Counter = Counter::zero();
static PART_CLAMPED: Counter = Counter::zero();
static PART_COUNT_SUM: Accum = Accum::zero();
static PART_COUNT_MAX: Counter = Counter::zero();
static PART_CAP_SUM: Accum = Accum::zero();
static PART_EMITTED_SUM: Accum = Accum::zero();
static PART_HIST: [Counter; PART_HIST_EDGES.len() + 1] = [
    Counter::zero(),
    Counter::zero(),
    Counter::zero(),
    Counter::zero(),
    Counter::zero(),
];
static PART_BUILD_TICKS_SUM: Accum = Accum::zero();
static PART_BUILD_TICKS_MAX: Accum = Accum::zero();

static PART_PRE_PASSES: Counter = Counter::zero();
static PART_PRE_GATED: Counter = Counter::zero();
static PART_PRE_EMITTERS: Accum = Accum::zero();
static PART_PRE_PRESCAN_TICKS: Accum = Accum::zero();
static PART_PRE_HITS: Counter = Counter::zero();
static PART_PRE_MISS_ABSENT: Counter = Counter::zero();
static PART_PRE_MISS_VALIDATE: [Counter; 7] = [
    Counter::zero(),
    Counter::zero(),
    Counter::zero(),
    Counter::zero(),
    Counter::zero(),
    Counter::zero(),
    Counter::zero(),
];
static PART_PRE_MISS_TIMEOUT: Counter = Counter::zero();
static PART_PRE_MISS_DECLINED: Counter = Counter::zero();
static PART_PRE_UNCONSUMED: Accum = Accum::zero();
static PART_PRE_WAIT_TICKS_SUM: Accum = Accum::zero();
static PART_PRE_WAIT_TICKS_MAX: Accum = Accum::zero();
static PART_PRE_WORKER_TICKS: Accum = Accum::zero();

/// Upper edges for the live-particles-per-draw histogram; one bucket past.
const PART_HIST_EDGES: [u32; 4] = [4, 16, 64, 256];

/// One draw-list animation pass: which arm ran and what it walked.
///
/// `roots` counts the animate-eligible nodes our walk dispatched (the even
/// half when the client's worker arm is on, the whole list on a forked
/// pass), `root_ticks_*` their measured cost, `list_len` the full
/// visible-list length from the particle pass.
pub fn anim_phase(
    armed: &Armed,
    worker_arm: bool,
    roots: u32,
    ticks_sum: u64,
    ticks_max: u64,
    list_len: u32,
) {
    ANIM_PASSES.bump(armed);
    if worker_arm {
        ANIM_WORKER_ARM.bump(armed);
    }
    if roots > 1 {
        ANIM_MULTI.bump(armed);
    }
    ROOTS_SUM.add(armed, u64::from(roots));
    ROOTS_MAX.max(armed, roots);
    LIST_SUM.add(armed, u64::from(list_len));
    LIST_MAX.max(armed, list_len);
    ROOT_TICKS_SUM.add(armed, ticks_sum);
    ROOT_TICKS_MAX.max(armed, ticks_max);
}

/// One bone-animation entry; `repeat` means the `+0x40` stamp matched.
///
/// A repeat is the dedup the serial walk relies on firing twice in one
/// frame, i.e. an instance reachable from two roots. Locked adds: with the
/// client's worker arm live this entry runs on two threads.
#[inline]
pub fn anim_entry(repeat: bool) {
    let Some(armed) = super::tally::arm() else {
        return;
    };
    ANIM_CALLS.bump(&armed);
    if repeat {
        REPEAT_VISITS.bump(&armed);
    }
}

/// One forked animate pass: its size and where the time went.
///
/// `worker us sum` is the sum of the per-root brackets across every lane (the
/// work that left or overlapped the game thread) and `fork us` the
/// publish-to-quiescence wall, which is the length of the *window* rather
/// than a cost: the game thread spends it running the phases that read the
/// results.
///
/// **`wait us` is the number that says what the fork still costs**: the game
/// thread's per-node stalls plus whatever the final drain had to wait for,
/// i.e. the part of the window it could not fill with useful work. `stalls`
/// counts the nodes that were not ready, against the visible-list length, and
/// `helped` the chunks the game thread animated from inside those stalls.
pub fn anim_par_pass(armed: &Armed, s: &super::hooks::BdlAnimForkStats) {
    ANIM_PAR_PASSES.bump(armed);
    ANIM_PAR_ROOTS.add(armed, u64::from(s.roots));
    ANIM_PAR_WAIT_TICKS_SUM.add(armed, s.wait_ticks);
    ANIM_PAR_WAIT_TICKS_MAX.max(armed, s.wait_ticks);
    ANIM_PAR_FORK_TICKS_SUM.add(armed, s.fork_ticks);
    ANIM_PAR_WORKER_TICKS.add(armed, s.root_ticks_sum);
    ANIM_PAR_STALLS.add(armed, u64::from(s.stalls));
    ANIM_PAR_HELPED.add(armed, u64::from(s.helped));
}

/// Game-thread ticks of one draw-list build, phase by phase.
///
/// The phases are the work, not a partition of `total`: the gaps between
/// them (the arm reads, the root buffer's take and put-back, the bucket count
/// resets) are what the remainder measures, and a remainder that stops being
/// small is itself the finding.
///
/// `animate` is the dispatch alone — the fork's publish-to-quiescence wall on
/// a forked pass, the walk on a serial one — so it is the game-thread cost of
/// animation, while `seam anim par`'s `worker us sum` is the work behind it.
/// Everything from `particles` on reads what that dispatch wrote, which is
/// why their sum bounds what overlapping the join could recover.
#[derive(Default)]
pub struct BdlPhases {
    /// The whole build, entry to return.
    pub total: u64,
    /// Entry to the animation phase: counter, cache reset, camera matrices.
    pub prologue: u64,
    /// The animate-eligible root walk.
    pub collect: u64,
    /// The fork's wall, or the serial animate walk.
    ///
    /// A forked pass closes the bracket before its own bookkeeping; a serial
    /// one cannot, so its walk carries the armed per-root brackets inside it
    /// and reads slightly high.
    pub animate: u64,
    /// `UpdateParticlesAndChildren` over the visible list.
    pub particles: u64,
    /// The node-bounds refresh that empties that list.
    pub refresh: u64,
    /// The bucket resets and the spatial-node walk behind the draw records.
    pub spatial: u64,
    /// The child-view walk.
    pub child: u64,
    /// Texture dedup and the bucket sorts.
    pub finalize: u64,
}

/// One draw-list build's phase costs.
pub fn bdl_pass(armed: &Armed, t: &BdlPhases) {
    BDL_PASSES.bump(armed);
    BDL_TOTAL_TICKS.add(armed, t.total);
    BDL_TOTAL_TICKS_MAX.max(armed, t.total);
    BDL_PROLOGUE_TICKS.add(armed, t.prologue);
    BDL_COLLECT_TICKS.add(armed, t.collect);
    BDL_ANIMATE_TICKS.add(armed, t.animate);
    BDL_PARTICLES_TICKS.add(armed, t.particles);
    BDL_REFRESH_TICKS.add(armed, t.refresh);
    BDL_SPATIAL_TICKS.add(armed, t.spatial);
    BDL_CHILD_TICKS.add(armed, t.child);
    BDL_FINALIZE_TICKS.add(armed, t.finalize);
}

/// One finalize phase: what it walked, and which of its five steps holds it.
///
/// The phase is the largest of the build, and `seam bdl` prices it as one
/// number. These split it the way the code is written — dedup (the scratch
/// clear and the probe loop), the bucket-0 texture sort, the run-merge, the
/// transparent sort, the two opaque sorts — and give each step the population
/// it walked, so a step's cost per record is readable rather than inferred.
///
/// The two opaque sorts share one bracket and one population: they run the
/// same comparator over two arrays, and it is their sum that ranks against
/// the transparent sort.
///
/// `probe_steps` and `cmp_calls` answer whether the 251-slot scratch
/// saturates: at a low load factor the probe is one step and one comparator
/// call per record, and the whole dedup is a hash pass; saturated, every
/// record walks all 251 slots.
#[derive(Default)]
pub struct FinalizeStats {
    /// Bucket-0 records the dedup walked, before the run-merge compacts them.
    pub bucket0_in: u32,
    /// Bucket-0 records left after the run-merge.
    pub bucket0_out: u32,
    /// Equal-texture runs the merge kept in bucket 0.
    pub runs: u32,
    /// Singletons the merge spilled into the transparent bucket.
    pub spilled: u32,
    /// Slots the dedup probe examined, summed over the records.
    pub probe_steps: u32,
    /// Equality comparator calls the probe made.
    pub cmp_calls: u32,
    /// Transparent-bucket elements sorted, spill included.
    pub transparent: u32,
    /// Elements sorted across both opaque buckets.
    pub opaque: u32,
    /// Comparisons the transparent sort made.
    pub transparent_cmps: u64,
    /// Comparisons the transparent sort decided on the element type tag alone.
    ///
    /// The chain's first key. A high share says the comparator already exits
    /// before it reads anything a precomputed key could hold, which is the
    /// measurement the rejected ten-lane key was missing.
    pub transparent_tag: u64,
    /// Comparisons the two opaque sorts made.
    pub opaque_cmps: u64,
    /// Ticks in the scratch clear and the dedup probe loop.
    pub dedup_ticks: u64,
    /// Ticks in the bucket-0 texture sort.
    pub tex_sort_ticks: u64,
    /// Ticks in the run-merge and its singleton spill.
    pub merge_ticks: u64,
    /// Ticks in the transparent-bucket sort.
    pub transparent_ticks: u64,
    /// Ticks in the two opaque-bucket sorts.
    pub opaque_ticks: u64,
}

/// One finalize phase's populations, comparisons and step costs.
pub fn finalize_pass(armed: &Armed, f: &FinalizeStats) {
    FIN_PASSES.bump(armed);
    FIN_B0_IN_SUM.add(armed, u64::from(f.bucket0_in));
    FIN_B0_IN_MAX.max(armed, f.bucket0_in);
    FIN_B0_OUT_SUM.add(armed, u64::from(f.bucket0_out));
    FIN_RUNS_SUM.add(armed, u64::from(f.runs));
    FIN_SPILL_SUM.add(armed, u64::from(f.spilled));
    FIN_PROBE_STEPS.add(armed, u64::from(f.probe_steps));
    FIN_CMP_CALLS.add(armed, u64::from(f.cmp_calls));
    FIN_TRANS_SUM.add(armed, u64::from(f.transparent));
    FIN_TRANS_MAX.max(armed, f.transparent);
    FIN_OPAQUE_SUM.add(armed, u64::from(f.opaque));
    FIN_OPAQUE_MAX.max(armed, f.opaque);
    FIN_TRANS_CMPS.add(armed, f.transparent_cmps);
    FIN_TRANS_TAG.add(armed, f.transparent_tag);
    FIN_OPAQUE_CMPS.add(armed, f.opaque_cmps);
    FIN_DEDUP_TICKS.add(armed, f.dedup_ticks);
    FIN_TEXSORT_TICKS.add(armed, f.tex_sort_ticks);
    FIN_MERGE_TICKS.add(armed, f.merge_ticks);
    FIN_TRANS_TICKS.add(armed, f.transparent_ticks);
    FIN_OPAQUE_TICKS.add(armed, f.opaque_ticks);
}

/// One multi-root pass below the fork gate.
///
/// The gate-tuning signal: a large count next to a small forked-pass count
/// means the threshold is starving the fork.
#[inline]
pub fn anim_par_gated() {
    super::tally::bump(&ANIM_PAR_GATED);
}

/// One clip call's counters, with the arm resolved once for the whole call.
///
/// The clip counts twice per call — the call itself, then whichever of its
/// three exits it takes — and it is the hottest function in the process, so
/// reading the arm again at the exit would double the unarmed cost of the site
/// for nothing. [`clip_call`] reads it once and this carries it. Zero-sized,
/// so the `Option` a caller holds across the body is a byte on the stack.
pub struct ClipCall(Armed);

/// Count one polygon clip and hold the arm for its exit.
///
/// The profiler put `collision_clip_polygon_by_plane` at 6.14% of the thread,
/// the hottest function in the process, and a sampler cannot say whether that
/// is a call rate or a cost per call. This says which: the rate against the
/// movement counters above it, and the vertex distribution against the choice
/// between making the loop cheaper and running it less.
#[inline]
pub fn clip_call(count: usize) -> Option<ClipCall> {
    let armed = super::tally::arm()?;
    let n = u32::try_from(count).unwrap_or(u32::MAX);
    CLIP_CALLS.add(&armed, 1);
    CLIP_VERTS.add(&armed, u64::from(n));
    let bucket = CLIP_HIST_EDGES
        .iter()
        .position(|&edge| n <= edge)
        .unwrap_or(CLIP_HIST_EDGES.len());
    CLIP_HIST[bucket].bump(&armed);
    Some(ClipCall(armed))
}

impl ClipCall {
    /// The polygon was wholly inside the plane and came back untouched.
    ///
    /// The two early exits leave the polygon alone; only [`ClipCall::cut`] pays
    /// the snapshot. The clip's cost per call is a blend of three very
    /// different bodies, and this split is what says how much of it a cheaper
    /// snapshot reaches.
    #[inline]
    pub fn kept(&self) {
        CLIP_KEPT.add(&self.0, 1);
    }

    /// The polygon was wholly outside the plane and was dropped.
    #[inline]
    pub fn dropped(&self) {
        CLIP_DROPPED.add(&self.0, 1);
    }

    /// The plane cut the polygon, so the snapshot and the rebuild were paid.
    #[inline]
    pub fn cut(&self) {
        CLIP_CUT.add(&self.0, 1);
    }
}

/// One event on a 64-bit counter: the `Accum` twin of [`super::tally::bump`].
#[inline]
fn count_one(counter: &Accum) {
    let Some(armed) = super::tally::arm() else {
        return;
    };
    counter.add(&armed, 1);
}

/// One visited candidate face, on the counter for how it ended.
#[inline]
fn sweep_face(outcome: &Accum) {
    let Some(armed) = super::tally::arm() else {
        return;
    };
    FACE_VISITED.add(&armed, 1);
    outcome.add(&armed, 1);
}

/// A candidate face rejected by the front-face dot, before any clip.
///
/// `seam clip` prices one clip; this family prices the loop that asks for them.
/// The gathered face list is walked linearly with that dot as its only filter,
/// so `visited` against `sweep` is the multiplier behind the clip rate, and
/// [`sweep_face_wiped`] is the ceiling on what a broad-phase cull could remove.
#[inline]
pub fn sweep_face_back_facing() {
    sweep_face(&FACE_BACK);
}

/// A candidate face the clip chain emptied: every plane's work bought nothing.
#[inline]
pub fn sweep_face_wiped() {
    sweep_face(&FACE_WIPED);
}

/// A candidate face that survived the clip chain and reached the swept-distance pass.
#[inline]
pub fn sweep_face_swept() {
    sweep_face(&FACE_SWEPT);
}

/// One pass of the face loop, i.e. one prism face tested against the gathered list.
#[inline]
pub fn sweep_face_pass() {
    count_one(&FACE_PASSES);
}

/// One dynamic-buffer resize/lock cycle in the particle emitter render.
#[inline]
pub fn particle_lock() {
    super::tally::bump(&PARTICLE_LOCKS);
}

/// One dynamic-buffer resize/lock cycle in the precipitation draw.
#[inline]
pub fn rain_lock() {
    super::tally::bump(&RAIN_LOCKS);
}

/// One ground-move step (`CMovement`), the per-unit movement entry of the collision cluster.
#[inline]
pub fn trace_ground() {
    super::tally::bump(&TRACE_GROUND);
}

/// One swept-volume collision query against the world planes.
#[inline]
pub fn trace_sweep() {
    super::tally::bump(&TRACE_SWEEP);
}

/// One terrain arm of a world trace line.
#[inline]
pub fn trace_terrain() {
    super::tally::bump(&TRACE_TERRAIN);
}

/// One M2-scene arm of a trace line.
#[inline]
pub fn trace_scene() {
    super::tally::bump(&TRACE_SCENE);
}

/// One geometry collection over the world's spatial nodes.
#[inline]
pub fn trace_geometry() {
    super::tally::bump(&TRACE_GEOMETRY);
}

/// One WMO-group query on a map chunk.
#[inline]
pub fn trace_wmo_query() {
    super::tally::bump(&TRACE_WMO_QUERY);
}

/// One doodad-set query on a map chunk.
#[inline]
pub fn trace_doodad_query() {
    super::tally::bump(&TRACE_DOODAD_QUERY);
}

/// One grid-vertex rebuild on a map chunk (terrain streaming).
#[inline]
pub fn trace_grid_build() {
    super::tally::bump(&TRACE_GRID_BUILD);
}

/// One per-emitter particle draw: its grain and what the quad build cost.
///
/// `count` is the live-particle count the draw walked, `cap` the capacity the
/// vertex buffer was clamped to, `emitted` how far the build advanced the
/// stream cursor, and the `t0..t1` bracket is the whole per-emitter quad
/// build. Together they size the per-emitter fork grain: draws per second
/// against the frame rate gives the width a frame offers, the histogram and
/// tick max give the skew a work split has to survive, and `cap` (known
/// before the lock) against `emitted` says what a build that reserved its
/// output range up front would waste.
pub fn particle_draw(armed: &Armed, count: u32, cap: u32, emitted: u32, t0: u64, t1: u64) {
    PART_DRAWS.bump(armed);
    if cap < count {
        PART_CLAMPED.bump(armed);
    }
    PART_COUNT_SUM.add(armed, u64::from(count));
    PART_COUNT_MAX.max(armed, count);
    PART_CAP_SUM.add(armed, u64::from(cap));
    PART_EMITTED_SUM.add(armed, u64::from(emitted));
    let bucket = PART_HIST_EDGES
        .iter()
        .position(|&edge| count <= edge)
        .unwrap_or(PART_HIST_EDGES.len());
    PART_HIST[bucket].bump(armed);
    let dt = t1.saturating_sub(t0);
    PART_BUILD_TICKS_SUM.add(armed, dt);
    PART_BUILD_TICKS_MAX.max(armed, dt);
}

/// One published particle pass job: its size and what the pre-scan cost.
pub fn pq_pass(armed: &Armed, emitters: u32, prescan_ticks: u64) {
    PART_PRE_PASSES.bump(armed);
    PART_PRE_EMITTERS.add(armed, u64::from(emitters));
    PART_PRE_PRESCAN_TICKS.add(armed, prescan_ticks);
}

/// One pass below the publish gate (portraits, single-model views).
#[inline]
pub fn pq_pass_gated() {
    super::tally::bump(&PART_PRE_GATED);
}

/// One prebuilt draw consumed, with how long the consume waited.
pub fn pq_hit(armed: &Armed, wait_ticks: u64) {
    PART_PRE_HITS.bump(armed);
    PART_PRE_WAIT_TICKS_SUM.add(armed, wait_ticks);
    PART_PRE_WAIT_TICKS_MAX.max(armed, wait_ticks);
}

/// A draw with no live pass entry (unknown caller or duplicate emitter).
#[inline]
pub fn pq_miss_absent() {
    super::tally::bump(&PART_PRE_MISS_ABSENT);
}

/// A draw whose snapshot no longer matched the just-written state.
///
/// `field` names the drifting prediction: 0 composed, 1 normal, 2 emitter
/// scalars, 3 cap, 4 fixup, 5 axis, 6 heap base.
#[inline]
pub fn pq_miss_validate(field: u32) {
    let idx = (field as usize).min(PART_PRE_MISS_VALIDATE.len() - 1);
    super::tally::bump(&PART_PRE_MISS_VALIDATE[idx]);
}

/// A draw that gave up waiting on its worker.
#[inline]
pub fn pq_miss_timeout() {
    super::tally::bump(&PART_PRE_MISS_TIMEOUT);
}

/// A draw whose worker declined it (guard tripped or bound exceeded).
#[inline]
pub fn pq_miss_declined() {
    super::tally::bump(&PART_PRE_MISS_DECLINED);
}

/// One pass retired: entries never consumed, and the workers' build time.
pub fn pq_retire(armed: &Armed, unconsumed: u32, worker_ticks: u64) {
    PART_PRE_UNCONSUMED.add(armed, u64::from(unconsumed));
    PART_PRE_WORKER_TICKS.add(armed, worker_ticks);
}

/// Ticks to microseconds through the calibrated engine clock.
fn ticks_to_us(ticks: u64) -> u64 {
    super::hooks::clock_ticks_to_ms(ticks.saturating_mul(1000))
}

/// Emit the cumulative seam lines on the gauge's heartbeat cadence.
///
/// Counters are never reset: like the sibling memo counters these are
/// session-cumulative, and the log's last line carries the totals.
pub fn emit_cumulative() {
    let passes = ANIM_PASSES.get();
    if passes != 0 {
        let calls = ANIM_CALLS.get();
        log::debug!(
            target: super::tally::TARGET,
            "seam anim: {passes} passes ({} multi-root, {} worker-arm), roots sum {} max {}, \
             list sum {} max {}, root us sum {} max {}, repeat {}/{calls}",
            ANIM_MULTI.get(),
            ANIM_WORKER_ARM.get(),
            ROOTS_SUM.get(),
            ROOTS_MAX.get(),
            LIST_SUM.get(),
            LIST_MAX.get(),
            ticks_to_us(ROOT_TICKS_SUM.get()),
            ticks_to_us(ROOT_TICKS_MAX.get()),
            REPEAT_VISITS.get(),
        );
    }
    let par_passes = ANIM_PAR_PASSES.get();
    let par_gated = ANIM_PAR_GATED.get();
    if par_passes != 0 || par_gated != 0 {
        log::debug!(
            target: super::tally::TARGET,
            "seam anim par: {par_passes} passes ({par_gated} gated multi-root), roots {}, \
             worker us sum {}, wait us sum {} max {}, fork us sum {}, stalls {}, helped {}",
            ANIM_PAR_ROOTS.get(),
            ticks_to_us(ANIM_PAR_WORKER_TICKS.get()),
            ticks_to_us(ANIM_PAR_WAIT_TICKS_SUM.get()),
            ticks_to_us(ANIM_PAR_WAIT_TICKS_MAX.get()),
            ticks_to_us(ANIM_PAR_FORK_TICKS_SUM.get()),
            ANIM_PAR_STALLS.get(),
            ANIM_PAR_HELPED.get(),
        );
    }
    let bdl_passes = BDL_PASSES.get();
    if bdl_passes != 0 {
        log::debug!(
            target: super::tally::TARGET,
            "seam bdl: {bdl_passes} passes, total us {} max {}, us prologue {} collect {} \
             animate {} particles {} refresh {} spatial {} child {} finalize {}",
            ticks_to_us(BDL_TOTAL_TICKS.get()),
            ticks_to_us(BDL_TOTAL_TICKS_MAX.get()),
            ticks_to_us(BDL_PROLOGUE_TICKS.get()),
            ticks_to_us(BDL_COLLECT_TICKS.get()),
            ticks_to_us(BDL_ANIMATE_TICKS.get()),
            ticks_to_us(BDL_PARTICLES_TICKS.get()),
            ticks_to_us(BDL_REFRESH_TICKS.get()),
            ticks_to_us(BDL_SPATIAL_TICKS.get()),
            ticks_to_us(BDL_CHILD_TICKS.get()),
            ticks_to_us(BDL_FINALIZE_TICKS.get()),
        );
    }
    let fin_passes = FIN_PASSES.get();
    if fin_passes != 0 {
        log::debug!(
            target: super::tally::TARGET,
            "seam finalize: {fin_passes} passes, b0 in sum {} max {} out {}, runs {}, spill {}, \
             probe steps {}, cmp calls {}, us dedup {} texsort {} merge {}",
            FIN_B0_IN_SUM.get(),
            FIN_B0_IN_MAX.get(),
            FIN_B0_OUT_SUM.get(),
            FIN_RUNS_SUM.get(),
            FIN_SPILL_SUM.get(),
            FIN_PROBE_STEPS.get(),
            FIN_CMP_CALLS.get(),
            ticks_to_us(FIN_DEDUP_TICKS.get()),
            ticks_to_us(FIN_TEXSORT_TICKS.get()),
            ticks_to_us(FIN_MERGE_TICKS.get()),
        );
        log::debug!(
            target: super::tally::TARGET,
            "seam finalize sorts: trans sum {} max {}, cmps {} (tag {}), us {}, \
             opaque sum {} max {}, cmps {}, us {}",
            FIN_TRANS_SUM.get(),
            FIN_TRANS_MAX.get(),
            FIN_TRANS_CMPS.get(),
            FIN_TRANS_TAG.get(),
            ticks_to_us(FIN_TRANS_TICKS.get()),
            FIN_OPAQUE_SUM.get(),
            FIN_OPAQUE_MAX.get(),
            FIN_OPAQUE_CMPS.get(),
            ticks_to_us(FIN_OPAQUE_TICKS.get()),
        );
    }
    let p = PARTICLE_LOCKS.get();
    let r = RAIN_LOCKS.get();
    if p != 0 || r != 0 {
        log::debug!(target: super::tally::TARGET, "seam locks: particle {p}, rain {r}");
    }
    let clip_calls = CLIP_CALLS.get();
    if clip_calls != 0 {
        log::debug!(
            target: super::tally::TARGET,
            "seam clip: {clip_calls} calls, verts {}, hist {}/{}/{}/{}, \
             kept {}, dropped {}, cut {}",
            CLIP_VERTS.get(),
            CLIP_HIST[0].get(),
            CLIP_HIST[1].get(),
            CLIP_HIST[2].get(),
            CLIP_HIST[3].get(),
            CLIP_KEPT.get(),
            CLIP_DROPPED.get(),
            CLIP_CUT.get(),
        );
    }
    let face_passes = FACE_PASSES.get();
    if face_passes != 0 {
        log::debug!(
            target: super::tally::TARGET,
            "seam faces: {face_passes} passes, visited {}, back {}, wiped {}, swept {}",
            FACE_VISITED.get(),
            FACE_BACK.get(),
            FACE_WIPED.get(),
            FACE_SWEPT.get(),
        );
    }
    let ground = TRACE_GROUND.get();
    let sweep = TRACE_SWEEP.get();
    let terrain = TRACE_TERRAIN.get();
    let scene = TRACE_SCENE.get();
    let geometry = TRACE_GEOMETRY.get();
    let wmo = TRACE_WMO_QUERY.get();
    let doodad = TRACE_DOODAD_QUERY.get();
    let grid = TRACE_GRID_BUILD.get();
    if ground | sweep | terrain | scene | geometry | wmo | doodad | grid != 0 {
        log::debug!(
            target: super::tally::TARGET,
            "seam traces: ground {ground}, sweep {sweep}, terrain {terrain}, scene {scene}, \
             geom {geometry}, wmo {wmo}, doodad {doodad}, grid {grid}",
        );
    }
    let draws = PART_DRAWS.get();
    if draws != 0 {
        log::debug!(
            target: super::tally::TARGET,
            "seam particles: {draws} draws ({} clamped), count sum {} max {}, \
             hist {}/{}/{}/{}/{}, cap sum {}, emitted sum {}, build us sum {} max {}",
            PART_CLAMPED.get(),
            PART_COUNT_SUM.get(),
            PART_COUNT_MAX.get(),
            PART_HIST[0].get(),
            PART_HIST[1].get(),
            PART_HIST[2].get(),
            PART_HIST[3].get(),
            PART_HIST[4].get(),
            PART_CAP_SUM.get(),
            PART_EMITTED_SUM.get(),
            ticks_to_us(PART_BUILD_TICKS_SUM.get()),
            ticks_to_us(PART_BUILD_TICKS_MAX.get()),
        );
    }
    let pre_passes = PART_PRE_PASSES.get();
    let pre_gated = PART_PRE_GATED.get();
    if pre_passes != 0 || pre_gated != 0 {
        log::debug!(
            target: super::tally::TARGET,
            "seam particles par: {pre_passes} passes ({pre_gated} gated), emitters {}, hits {}, \
             miss absent {} validate {}/{}/{}/{}/{}/{}/{} timeout {} declined {}, \
             unconsumed {}, wait us sum {} max {}, prescan us sum {}, worker us sum {}",
            PART_PRE_EMITTERS.get(),
            PART_PRE_HITS.get(),
            PART_PRE_MISS_ABSENT.get(),
            PART_PRE_MISS_VALIDATE[0].get(),
            PART_PRE_MISS_VALIDATE[1].get(),
            PART_PRE_MISS_VALIDATE[2].get(),
            PART_PRE_MISS_VALIDATE[3].get(),
            PART_PRE_MISS_VALIDATE[4].get(),
            PART_PRE_MISS_VALIDATE[5].get(),
            PART_PRE_MISS_VALIDATE[6].get(),
            PART_PRE_MISS_TIMEOUT.get(),
            PART_PRE_MISS_DECLINED.get(),
            PART_PRE_UNCONSUMED.get(),
            ticks_to_us(PART_PRE_WAIT_TICKS_SUM.get()),
            ticks_to_us(PART_PRE_WAIT_TICKS_MAX.get()),
            ticks_to_us(PART_PRE_PRESCAN_TICKS.get()),
            ticks_to_us(PART_PRE_WORKER_TICKS.get()),
        );
    }
}
