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
//! after it; how many dynamic-buffer lock cycles the particle and
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
/// `worker_ticks` is the post-join sum of the per-root brackets across every
/// lane (the work that left or overlapped the game thread), `wait_ticks` the
/// coordinator's pure join wait after its own participation, and
/// `fork_ticks` the publish-to-quiescence wall time. The fork pays off while
/// `fork_ticks` stays well under the same roots' serial cost (the `seam
/// anim` line's `root us sum` baseline).
pub fn anim_par_pass(
    armed: &Armed,
    roots: u32,
    wait_ticks: u64,
    fork_ticks: u64,
    worker_ticks: u64,
) {
    ANIM_PAR_PASSES.bump(armed);
    ANIM_PAR_ROOTS.add(armed, u64::from(roots));
    ANIM_PAR_WAIT_TICKS_SUM.add(armed, wait_ticks);
    ANIM_PAR_WAIT_TICKS_MAX.max(armed, wait_ticks);
    ANIM_PAR_FORK_TICKS_SUM.add(armed, fork_ticks);
    ANIM_PAR_WORKER_TICKS.add(armed, worker_ticks);
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

/// One multi-root pass below the fork gate.
///
/// The gate-tuning signal: a large count next to a small forked-pass count
/// means the threshold is starving the fork.
#[inline]
pub fn anim_par_gated() {
    super::tally::bump(&ANIM_PAR_GATED);
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
             worker us sum {}, wait us sum {} max {}, fork us sum {}",
            ANIM_PAR_ROOTS.get(),
            ticks_to_us(ANIM_PAR_WORKER_TICKS.get()),
            ticks_to_us(ANIM_PAR_WAIT_TICKS_SUM.get()),
            ticks_to_us(ANIM_PAR_WAIT_TICKS_MAX.get()),
            ticks_to_us(ANIM_PAR_FORK_TICKS_SUM.get()),
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
    let p = PARTICLE_LOCKS.get();
    let r = RAIN_LOCKS.get();
    if p != 0 || r != 0 {
        log::debug!(target: super::tally::TARGET, "seam locks: particle {p}, rain {r}");
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
