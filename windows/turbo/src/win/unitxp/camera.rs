//! The camera-editing feature and the camera state the sight tests share.
//!
//! After the client's own camera update runs each frame, the wrapper applies
//! the configured offsets: a shoulder displacement perpendicular to the
//! camera-to-subject line, a height offset (optionally pinning the eye at
//! the subject's collision-box height so shapeshifts do not jolt the view),
//! and a pitch tilt. Translation uses at most three current-pose ray probes,
//! without collecting triangles or iterating over candidate corrections. A
//! hit shortens the translation along its original path. Rotation and spaces
//! between the rays are not collision-validated.
//!
//! The final edited position and forward vector are published for the sight
//! features, so camera traces originate where the eye actually is; readers
//! fall back to the live camera record before the first publish. Follow
//! mode rebuilds the view basis toward the current target when it is
//! friendly-shaped, close and in sight.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::{
    math::editcamera::CameraPose,
    win::tally::{self, Accum, Counter},
};

/// The active camera record: no arguments, record pointer in `eax`.
const GET_ACTIVE_CAMERA_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0008_18f0;

/// The video-options record whose water-collision field picks the trace flag.
const VIDEO_OPTIONS: usize = crate::win::EXPECTED_IMAGE_BASE + 0x007e_1088;

/// The float tolerance shared with the offset arithmetic.
const TOLERANCE: f32 = 1e-5;

/// Whether the camera hook has published state this session.
static PUBLISHED: AtomicBool = AtomicBool::new(false);

/// The edited camera position, f32 bits per axis.
static TRANSLATED_POS: [AtomicU32; 3] = [const { AtomicU32::new(0) }; 3];
/// The edited camera forward vector, f32 bits per axis.
static ROTATED_FWD: [AtomicU32; 3] = [const { AtomicU32::new(0) }; 3];

/// Camera updates checked against world geometry (armed only).
static PROBE_BATCHES: Counter = Counter::zero();
static QUERY_FAILURES: Counter = Counter::zero();
static PROBES: Counter = Counter::zero();
static GUARD_TICKS: Accum = Accum::zero();
static GUARD_MAX_TICKS: Accum = Accum::zero();

fn store3(slots: &[AtomicU32; 3], value: [f32; 3]) {
    for (slot, v) in slots.iter().zip(value) {
        slot.store(v.to_bits(), Ordering::Relaxed);
    }
}

fn load3(slots: &[AtomicU32; 3]) -> [f32; 3] {
    let mut out = [0.0f32; 3];
    for (v, slot) in out.iter_mut().zip(slots) {
        *v = f32::from_bits(slot.load(Ordering::Relaxed));
    }
    out
}

/// Publish the per-frame edited camera state (the camera hook's last step).
fn publish(translated_pos: [f32; 3], rotated_fwd: [f32; 3]) {
    store3(&TRANSLATED_POS, translated_pos);
    store3(&ROTATED_FWD, rotated_fwd);
    PUBLISHED.store(true, Ordering::Relaxed);
}

/// The live camera record, or `None` out of world.
fn live_camera() -> Option<usize> {
    // SAFETY: a fixed `.text` entry in the live host image (base verified
    // at load); the transmuted signature matches the declared prototype
    // (no arguments, record pointer in `eax`).
    let get: extern "fastcall" fn() -> u32 = unsafe { core::mem::transmute(GET_ACTIVE_CAMERA_VA) };
    let camera = get() as usize;
    (camera != 0 && camera & 1 == 0).then_some(camera)
}

/// Read one float of the live camera record.
fn cam_f32(camera: usize, offset: usize) -> f32 {
    // SAFETY: `camera` passed the liveness heuristic; `offset` addresses a
    // float field of the camera record.
    unsafe { *((camera + offset) as *const f32) }
}

/// Read three consecutive floats of the live camera record.
fn camera_vec3(camera: usize, offset: usize) -> [f32; 3] {
    let mut out = [0.0f32; 3];
    for (i, v) in out.iter_mut().enumerate() {
        // SAFETY: `camera` passed the liveness heuristic; `offset` addresses
        // a three-float field of the camera record (position at `+0x8`, the
        // basis rows from `+0x14`).
        *v = unsafe { *((camera + offset + i * 4) as *const f32) };
    }
    out
}

/// Publish stock camera direction in world space, including on transports.
fn unmodified_forward(camera: usize) -> [f32; 3] {
    let forward = camera_vec3(camera, 0x14);
    // SAFETY: the live camera's transport GUID occupies +0x98, with only the
    // record's four-byte alignment guaranteed.
    let transport = unsafe { ((camera + 0x98) as *const u64).read_unaligned() };
    if transport == 0 {
        return forward;
    }
    super::camera_projection::CameraSpace::snapshot(camera)
        .and_then(|space| {
            space.renderer_basis([
                forward,
                camera_vec3(camera, 0x20),
                camera_vec3(camera, 0x2c),
            ])
        })
        .map_or(forward, |basis| basis[0])
}

/// Write the camera basis rows: forward, right, up from `+0x14`.
fn set_basis(camera: usize, basis: [[f32; 3]; 3]) {
    for (row, vec) in basis.iter().enumerate() {
        for (i, &v) in vec.iter().enumerate() {
            // SAFETY: `camera` passed the liveness heuristic; `+0x14` is the
            // 3x3 view basis this feature exists to rewrite.
            unsafe { *((camera + 0x14 + (row * 3 + i) * 4) as *mut f32) = v };
        }
    }
}

/// The GUID of the unit the camera is looking at (`+0x88`).
fn looking_at_guid(camera: usize) -> u64 {
    // SAFETY: `camera` passed the liveness heuristic; `+0x88` is the subject
    // GUID (unaligned by the record's packing).
    unsafe { ((camera + 0x88) as *const u64).read_unaligned() }
}

/// The game's own camera-collision flag, switched by the water option.
fn camera_query_flag() -> Option<u32> {
    // SAFETY: `VIDEO_OPTIONS` is a fixed host global at the verified image
    // base, holding the live options record pointer.
    let options = unsafe { *(VIDEO_OPTIONS as *const usize) };
    if options == 0 {
        return None;
    }
    // SAFETY: `+0x28` of the options record is the water-collision field the
    // client's camera collision switches its flag on.
    let water = unsafe { *((options + 0x28) as *const u32) };
    Some(if water != 0 { 0x001f_0171 } else { 0x0010_0171 })
}

/// Where sight traces originate: the edited camera position.
pub fn translated_position() -> [f32; 3] {
    if PUBLISHED.load(Ordering::Relaxed) {
        return load3(&TRANSLATED_POS);
    }
    live_camera().map_or([0.0; 3], |camera| camera_vec3(camera, 0x8))
}

/// The camera forward vector matching [`translated_position`].
pub fn rotated_forward() -> [f32; 3] {
    if PUBLISHED.load(Ordering::Relaxed) {
        return load3(&ROTATED_FWD);
    }
    live_camera().map_or([0.0; 3], unmodified_forward)
}

/// The follow-target basis rebuild, when the target qualifies.
fn follow_target(eye: [f32; 3]) -> Option<[[f32; 3]; 3]> {
    let target_guid = super::super::objmgr::guid_of_token(c"target");
    if target_guid == 0 {
        return None;
    }
    let target = super::super::objmgr::object_by_guid(target_guid)?;
    let player = super::super::objmgr::player()?;
    let qualifies = match target.object_type() {
        super::super::objmgr::TYPE_PLAYER => !player.can_attack(target),
        super::super::objmgr::TYPE_UNIT => target.is_player_controlled() == Some(false),
        _ => false,
    };
    if !qualifies {
        return None;
    }
    let close = super::distance::between_units(player, target, crate::math::reach::Meter::Ranged);
    // The sight test follows the original's truthiness: any non-zero verdict
    // (including the error shape) passes.
    if !(0.0..50.0).contains(&close) || super::insight::unit_in_sight(player, target) == 0 {
        return None;
    }
    let mut target_position = target.position();
    target_position[2] += target.collision_box_height();
    crate::math::editcamera::look_at_basis(eye, target_position)
}

/// The per-frame edit, run after the client's own camera update.
pub fn after_update(camera_raw: u32) {
    let camera = camera_raw as usize;
    if camera == 0 || camera & 1 != 0 {
        return;
    }
    let original_pos = camera_vec3(camera, 0x8);
    // All five settings are read before the subject lookup because the gate
    // below needs them: with none of them configured every step that follows is
    // a no-op on the camera record, down to a write-back of the bits just read,
    // so the default configuration takes the same exit the no-subject arm does.
    // The tolerance tests are spelled `<= TOLERANCE` rather than negated so a
    // NaN setting falls through into the full body instead of into this exit.
    let horizontal = super::settings::camera_horizontal();
    let vertical = super::settings::camera_vertical();
    let pitch = super::settings::camera_pitch();
    let pin_on = super::settings::camera_pin_height();
    let follow = super::settings::camera_follow_target();
    if !pin_on
        && !follow
        && horizontal.abs() <= TOLERANCE
        && vertical.abs() <= TOLERANCE
        && pitch.abs() <= TOLERANCE
    {
        publish(original_pos, unmodified_forward(camera));
        return;
    }
    let Some(unit) = super::super::objmgr::object_by_guid(looking_at_guid(camera))
        .filter(|u| u.is_unit_or_player())
    else {
        // No subject (login, cinematics): the unedited camera IS the state.
        publish(original_pos, unmodified_forward(camera));
        return;
    };
    let pin = if pin_on && unit.mount_display_id() == 0 {
        Some(crate::math::editcamera::PinHeight {
            // The eye height is the camera target's height above the unit
            // origin, replaced by the collision-box height while pinned.
            eye_height: cam_f32(camera, 0x17c) - unit.position()[2],
            box_height: unit.collision_box_height(),
        })
    } else {
        None
    };
    let raw_basis = [
        camera_vec3(camera, 0x14),
        camera_vec3(camera, 0x20),
        camera_vec3(camera, 0x2c),
    ];
    let Some(space) = super::camera_projection::CameraSpace::snapshot(camera) else {
        tally::bump(&QUERY_FAILURES);
        publish(original_pos, raw_basis[0]);
        return;
    };
    let Some(world_basis) = space.renderer_basis(raw_basis) else {
        tally::bump(&QUERY_FAILURES);
        publish(original_pos, raw_basis[0]);
        return;
    };
    let original = CameraPose {
        position: original_pos,
        basis: world_basis,
    };
    let mut edited = CameraPose {
        position: crate::math::editcamera::translate_camera(
            original_pos,
            unit.position(),
            horizontal,
            vertical,
            pin.as_ref(),
        ),
        basis: original.basis,
    };
    if pitch.abs() > TOLERANCE
        && let Some(basis) = crate::math::editcamera::pitch_basis(original.basis[0], pitch)
    {
        edited.basis = basis;
    }
    let mut accepted = if edited.basis == original.basis {
        AcceptedPose {
            pose: edited,
            raw_basis,
        }
    } else {
        let Some(pose) = rounded_pose(&edited, &space) else {
            tally::bump(&QUERY_FAILURES);
            publish(original_pos, world_basis[0]);
            return;
        };
        pose
    };
    if crate::math::editcamera::position_changed(&original.position, &accepted.pose.position) {
        let started = tally::arm().map(|_| wow_shared::tsc::rdtsc());
        let valid = validate_translation(camera, &original, &mut accepted.pose);
        if let Some(armed) = tally::arm()
            && let Some(started) = started
        {
            let ticks = wow_shared::tsc::rdtsc().wrapping_sub(started);
            GUARD_TICKS.add(&armed, ticks);
            GUARD_MAX_TICKS.max(&armed, ticks);
        }
        if !valid {
            tally::bump(&QUERY_FAILURES);
            publish(original_pos, world_basis[0]);
            return;
        }
    }
    // Follow from the corrected eye. Rotation adds no validation rays;
    // its existing target-eligibility sight query remains unchanged.
    if follow && let Some(basis) = follow_target(accepted.pose.position) {
        let candidate = CameraPose {
            position: accepted.pose.position,
            basis,
        };
        if let Some(pose) = rounded_pose(&candidate, &space) {
            accepted = pose;
        }
    }
    for (i, &v) in accepted.pose.position.iter().enumerate() {
        // SAFETY: `camera` passed the liveness heuristic; `+0x8` is the
        // camera position this feature exists to rewrite.
        unsafe { *((camera + 0x8 + i * 4) as *mut f32) = v };
    }
    set_basis(camera, accepted.raw_basis);
    publish(accepted.pose.position, accepted.pose.basis[0]);
}

struct AcceptedPose {
    pose: CameraPose,
    raw_basis: [[f32; 3]; 3],
}

fn rounded_pose(
    candidate: &CameraPose,
    space: &super::camera_projection::CameraSpace,
) -> Option<AcceptedPose> {
    let raw_basis = space.world_to_local(candidate.basis)?;
    Some(AcceptedPose {
        pose: CameraPose {
            position: candidate.position,
            basis: space.renderer_basis(raw_basis)?,
        },
        raw_basis,
    })
}

fn validate_translation(camera: usize, original: &CameraPose, edited: &mut CameraPose) -> bool {
    let Some(plane) = super::camera_projection::current_projection(camera) else {
        return false;
    };
    let Some(flag) = camera_query_flag() else {
        return false;
    };
    tally::bump(&PROBE_BATCHES);
    crate::math::editcamera::correct_translation(original, edited, &plane, |from, to| {
        tally::bump(&PROBES);
        super::trace::world_intersect_flagged(from, to, flag)
    })
}

/// One cumulative line for the translation probes, when any has run.
pub fn emit_cumulative() {
    let batches = PROBE_BATCHES.get();
    if batches != 0 || QUERY_FAILURES.get() != 0 {
        crate::defer_log!(target: tally::TARGET, log::Level::Info,
            "unitxp camera: {batches} translation batches, {} rays, {} unavailable, {} guard ticks, {} max guard ticks",
            PROBES.get(), QUERY_FAILURES.get(), GUARD_TICKS.get(), GUARD_MAX_TICKS.get(),
        );
    }
}
