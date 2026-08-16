//! The camera-editing feature and the camera state the sight tests share.
//!
//! After the client's own camera update runs each frame, the wrapper applies
//! the configured offsets: a shoulder displacement perpendicular to the
//! camera-to-subject line, a height offset (optionally pinning the eye at
//! the subject's collision-box height so shapeshifts do not jolt the view),
//! and a pitch tilt. Offsets are validated against the world with up to
//! three collision probes — the near-clip plane corner in each translated
//! axis plus a camera-body sweep — pulling the camera back out of any
//! surface it would clip; probes re-run at most sixty times a second and
//! only when the camera actually moved, cached corrections serving the
//! frames between.
//!
//! The final edited position and forward vector are published for the sight
//! features, so camera traces originate where the eye actually is; readers
//! fall back to the live camera record before the first publish. Follow
//! mode rebuilds the view basis toward the current target when it is
//! friendly-shaped, close and in sight.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::win::tally::{self, Counter};

/// The active camera record — no arguments, record pointer in `eax`.
const GET_ACTIVE_CAMERA_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0008_18f0;

/// The video-options record whose water-collision field picks the trace flag.
const VIDEO_OPTIONS: usize = crate::win::EXPECTED_IMAGE_BASE + 0x007e_1088;

/// Camera-collision probes re-run at most this often, in engine milliseconds.
const REFRESH_INTERVAL_MS: u32 = 16;

/// The float tolerance shared with the offset arithmetic.
const TOLERANCE: f32 = 1e-5;

/// Whether the camera hook has published state this session.
static PUBLISHED: AtomicBool = AtomicBool::new(false);

/// The edited camera position, f32 bits per axis.
static TRANSLATED_POS: [AtomicU32; 3] = [const { AtomicU32::new(0) }; 3];
/// The edited camera forward vector, f32 bits per axis.
static ROTATED_FWD: [AtomicU32; 3] = [const { AtomicU32::new(0) }; 3];

/// Where the last collision probes ran from, f32 bits per axis.
static LAST_PROBED_POS: [AtomicU32; 3] = [const { AtomicU32::new(0) }; 3];
/// The pitch the last probes ran with, f32 bits.
static LAST_PROBED_PITCH: AtomicU32 = AtomicU32::new(0);
/// Engine time of the last probe batch.
static LAST_PROBE_MS: AtomicU32 = AtomicU32::new(0);
/// Cached corrections between probe batches, f32 bits per axis.
static LAST_V_CLIP: [AtomicU32; 3] = [const { AtomicU32::new(0) }; 3];
/// See [`LAST_V_CLIP`].
static LAST_H_CLIP: [AtomicU32; 3] = [const { AtomicU32::new(0) }; 3];
/// See [`LAST_V_CLIP`].
static LAST_V_BODY: [AtomicU32; 3] = [const { AtomicU32::new(0) }; 3];
/// See [`LAST_V_CLIP`].
static LAST_H_BODY: [AtomicU32; 3] = [const { AtomicU32::new(0) }; 3];

/// Probe batches run and frames served from cached corrections (armed only).
static PROBE_BATCHES: Counter = Counter::zero();
/// See [`PROBE_BATCHES`].
static PROBE_REUSES: Counter = Counter::zero();

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
fn camera_query_flag() -> u32 {
    // SAFETY: `VIDEO_OPTIONS` is a fixed host global at the verified image
    // base, holding the live options record pointer.
    let options = unsafe { *(VIDEO_OPTIONS as *const usize) };
    // SAFETY: `+0x28` of the options record is the water-collision field the
    // client's camera collision switches its flag on.
    let water = unsafe { *((options + 0x28) as *const u32) };
    if water != 0 { 0x001f_0171 } else { 0x0010_0171 }
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
    live_camera().map_or([0.0; 3], |camera| camera_vec3(camera, 0x14))
}

/// The near-clip corner probe for the vertical translation.
fn vertical_clip_probe(camera: usize, original: [f32; 3], translated: [f32; 3]) -> [f32; 3] {
    let near_clip = cam_f32(camera, 0x38);
    let fov = cam_f32(camera, 0x40) / cam_f32(camera, 0x44);
    let half = (fov / 2.0).tan() * near_clip;
    let signed = if translated[2] > original[2] {
        half
    } else {
        -half
    };
    let (from, to) = crate::math::editcamera::probe_endpoints(
        original,
        translated,
        camera_vec3(camera, 0x14),
        camera_vec3(camera, 0x2c),
        signed,
        near_clip,
    );
    match super::trace::world_intersect_flagged(&from, &to, camera_query_flag()) {
        Some(hit) if (0.0..=1.0).contains(&hit) => {
            crate::math::editcamera::clip_correction(from, to, hit, original, translated, false)
        }
        _ => [0.0; 3],
    }
}

/// The near-clip corner probe for the shoulder translation.
fn horizontal_clip_probe(
    camera: usize,
    horizontal: f32,
    original: [f32; 3],
    translated: [f32; 3],
) -> [f32; 3] {
    let near_clip = cam_f32(camera, 0x38);
    let fov = cam_f32(camera, 0x40);
    let half = (fov / 2.0).tan() * near_clip;
    let signed = if horizontal > 0.0 { half } else { -half };
    let (from, to) = crate::math::editcamera::probe_endpoints(
        original,
        translated,
        camera_vec3(camera, 0x14),
        camera_vec3(camera, 0x20),
        signed,
        near_clip,
    );
    match super::trace::world_intersect_flagged(&from, &to, camera_query_flag()) {
        Some(hit) if (0.0..=1.0).contains(&hit) => {
            crate::math::editcamera::clip_correction(from, to, hit, original, translated, true)
        }
        _ => [0.0; 3],
    }
}

/// The camera-body sweep from the unedited to the edited position.
fn body_probe(original: [f32; 3], translated: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    match super::trace::world_intersect_flagged(&original, &translated, camera_query_flag()) {
        Some(hit) if (0.0..=1.0).contains(&hit) => {
            crate::math::editcamera::body_corrections(original, translated, hit)
        }
        _ => ([0.0; 3], [0.0; 3]),
    }
}

fn positions_near(a: [f32; 3], b: [f32; 3]) -> bool {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    dx * dx + dy * dy + dz * dz <= TOLERANCE * TOLERANCE
}

/// The collision-validation block: probe or reuse, then correct `translated`.
fn validate_against_world(
    camera: usize,
    original: [f32; 3],
    translated: &mut [f32; 3],
    horizontal: f32,
    vertical: f32,
    pitch: f32,
    pin_on: bool,
) {
    let need_vertical = pin_on || vertical.abs() > TOLERANCE || pitch.abs() > TOLERANCE;
    let need_horizontal = horizontal.abs() > TOLERANCE;
    let now = super::super::objmgr::game_tick_ms();
    let due = now.wrapping_sub(LAST_PROBE_MS.load(Ordering::Relaxed)) > REFRESH_INTERVAL_MS;
    let last_pitch = f32::from_bits(LAST_PROBED_PITCH.load(Ordering::Relaxed));
    let moved = !positions_near(*translated, load3(&LAST_PROBED_POS))
        || (last_pitch.abs() - pitch.abs()).abs() > TOLERANCE;
    let (v_clip, h_clip, h_body, v_body);
    if due && moved {
        tally::bump(&PROBE_BATCHES);
        LAST_PROBE_MS.store(now, Ordering::Relaxed);
        store3(&LAST_PROBED_POS, *translated);
        LAST_PROBED_PITCH.store(pitch.to_bits(), Ordering::Relaxed);
        v_clip = if need_vertical {
            vertical_clip_probe(camera, original, *translated)
        } else {
            [0.0; 3]
        };
        store3(&LAST_V_CLIP, v_clip);
        h_clip = if need_horizontal {
            horizontal_clip_probe(camera, horizontal, original, *translated)
        } else {
            [0.0; 3]
        };
        store3(&LAST_H_CLIP, h_clip);
        (h_body, v_body) = body_probe(original, *translated);
        store3(&LAST_H_BODY, h_body);
        store3(&LAST_V_BODY, v_body);
    } else {
        tally::bump(&PROBE_REUSES);
        v_clip = if need_vertical {
            load3(&LAST_V_CLIP)
        } else {
            [0.0; 3]
        };
        h_clip = if need_horizontal {
            load3(&LAST_H_CLIP)
        } else {
            [0.0; 3]
        };
        h_body = load3(&LAST_H_BODY);
        v_body = load3(&LAST_V_BODY);
    }
    if crate::math::editcamera::longer(v_body, v_clip) {
        translated[2] += v_body[2];
    } else {
        translated[2] += v_clip[2];
    }
    if crate::math::editcamera::longer(h_body, h_clip) {
        translated[0] += h_body[0];
        translated[1] += h_body[1];
    } else {
        translated[0] += h_clip[0];
        translated[1] += h_clip[1];
    }
}

/// The follow-target basis rebuild, when the target qualifies.
fn follow_target(camera: usize) {
    let target_guid = super::super::objmgr::guid_of_token(c"target");
    if target_guid == 0 {
        return;
    }
    let Some(target) = super::super::objmgr::object_by_guid(target_guid) else {
        return;
    };
    let Some(player) = super::super::objmgr::player() else {
        return;
    };
    let qualifies = match target.object_type() {
        super::super::objmgr::TYPE_PLAYER => !player.can_attack(target),
        super::super::objmgr::TYPE_UNIT => target.is_player_controlled() == Some(false),
        _ => false,
    };
    if !qualifies {
        return;
    }
    let close = super::distance::between_units(player, target, crate::math::reach::Meter::Ranged);
    // The sight test follows the original's truthiness: any non-zero verdict
    // (including the error shape) passes.
    if !(0.0..50.0).contains(&close) || super::insight::unit_in_sight(player, target) == 0 {
        return;
    }
    let mut target_position = target.position();
    target_position[2] += target.collision_box_height();
    let eye = camera_vec3(camera, 0x8);
    if let Some(basis) = crate::math::editcamera::look_at_basis(eye, target_position) {
        set_basis(camera, basis);
    }
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
        publish(original_pos, camera_vec3(camera, 0x14));
        return;
    }
    let Some(unit) = super::super::objmgr::object_by_guid(looking_at_guid(camera))
        .filter(|u| u.is_unit_or_player())
    else {
        // No subject (login, cinematics): the unedited camera IS the state.
        publish(original_pos, camera_vec3(camera, 0x14));
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
    let mut translated = crate::math::editcamera::translate_camera(
        original_pos,
        unit.position(),
        horizontal,
        vertical,
        pin.as_ref(),
    );
    if pitch.abs() > TOLERANCE
        && let Some(basis) = crate::math::editcamera::pitch_basis(camera_vec3(camera, 0x14), pitch)
    {
        set_basis(camera, basis);
    }
    if pin_on
        || vertical.abs() > TOLERANCE
        || horizontal.abs() > TOLERANCE
        || pitch.abs() > TOLERANCE
    {
        validate_against_world(
            camera,
            original_pos,
            &mut translated,
            horizontal,
            vertical,
            pitch,
            pin_on,
        );
    }
    for (i, &v) in translated.iter().enumerate() {
        // SAFETY: `camera` passed the liveness heuristic; `+0x8` is the
        // camera position this feature exists to rewrite.
        unsafe { *((camera + 0x8 + i * 4) as *mut f32) = v };
    }
    if follow {
        follow_target(camera);
    }
    publish(translated, camera_vec3(camera, 0x14));
}

/// One cumulative line for the collision probes, when any has run.
pub fn emit_cumulative() {
    let batches = PROBE_BATCHES.get();
    let reuses = PROBE_REUSES.get();
    if batches | reuses != 0 {
        log::info!(
            target: tally::TARGET,
            "unitxp camera: {batches} probe batches, {reuses} reused",
        );
    }
}
