//! Camera-offset arithmetic behind the camera-editing feature.
//!
//! Pure kernels: the shoulder/height translation of the camera position, the
//! look-at and pitch basis rebuilds, the near-clip-plane probe endpoints and
//! the collision corrections that pull an offset camera back out of a wall.
//! New arithmetic (the feature is not a client function); the reference
//! semantics are preserved exactly, including the revert-when-too-close rule.

/// The float tolerance shared by every near-zero test here.
const TOLERANCE: f32 = 1e-5;

/// An offset camera corrects to this distance in front of a hit surface.
const KEEP_DISTANCE_FROM_WALL: f32 = 0.2;

fn length(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = length(v);
    if len > 0.0 {
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        v
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn almost_zero(v: [f32; 3]) -> bool {
    let len_sq = f64::from(v[0]) * f64::from(v[0])
        + f64::from(v[1]) * f64::from(v[1])
        + f64::from(v[2]) * f64::from(v[2]);
    len_sq <= f64::from(TOLERANCE) * f64::from(TOLERANCE)
}

/// An orthonormal camera basis looking from `eye` at `target`.
///
/// Rows in the camera record's order: forward, right, up. `None` when the
/// two positions coincide (the caller leaves the basis untouched).
pub fn look_at_basis(eye: [f32; 3], target: [f32; 3]) -> Option<[[f32; 3]; 3]> {
    let forward = [target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]];
    basis_from_forward(forward)
}

/// The camera basis after raising the forward vector's z by `delta`.
pub fn pitch_basis(forward: [f32; 3], delta: f32) -> Option<[[f32; 3]; 3]> {
    basis_from_forward([forward[0], forward[1], forward[2] + delta])
}

fn basis_from_forward(forward: [f32; 3]) -> Option<[[f32; 3]; 3]> {
    if almost_zero(forward) {
        return None;
    }
    let forward = normalize(forward);
    let right = normalize(cross([0.0, 0.0, 1.0], forward));
    let up = normalize(cross(forward, right));
    Some([forward, right, up])
}

/// The pin-height inputs.
///
/// How far the eye sits above the unit's origin, and the collision-box
/// height that replaces it.
pub struct PinHeight {
    /// Camera target height above the unit origin (negative disables).
    pub eye_height: f32,
    /// The unit's collision-box height.
    pub box_height: f32,
}

/// The offset camera position: shoulder offset, height offset, pinned height.
///
/// `camera` and `unit` are the unedited camera and its subject. Inside half a
/// yard of planar distance (first person) nothing moves. The shoulder offset
/// displaces perpendicular to the planar camera-to-unit line; the pin swap
/// replaces the eye height with the collision-box height (callers skip it
/// while mounted by passing `None`).
pub fn translate_camera(
    camera: [f32; 3],
    unit: [f32; 3],
    horizontal: f32,
    vertical: f32,
    pin: Option<&PinHeight>,
) -> [f32; 3] {
    let mut result = camera;
    let planar = [unit[0] - camera[0], unit[1] - camera[1], 0.0];
    let planar_distance = length(planar);
    if planar_distance < 0.5 {
        return result;
    }
    if let Some(pin) = pin
        && pin.eye_height >= 0.0
    {
        result[2] -= pin.eye_height;
        result[2] += pin.box_height;
    }
    if horizontal.abs() > TOLERANCE {
        if horizontal > 0.0 {
            result[0] = horizontal.abs() * (unit[1] - camera[1]) / planar_distance + camera[0];
            result[1] = horizontal.abs() * (camera[0] - unit[0]) / planar_distance + camera[1];
        } else {
            result[0] = horizontal.abs() * (camera[1] - unit[1]) / planar_distance + camera[0];
            result[1] = horizontal.abs() * (unit[0] - camera[0]) / planar_distance + camera[1];
        }
    }
    if vertical.abs() > TOLERANCE {
        result[2] += vertical;
    }
    result
}

/// The near-clip probe segment for one translation axis.
///
/// Both endpoints sit on the near plane (`forward * near_clip` ahead of the
/// unedited and edited camera positions), displaced to the plane corner the
/// translation moved toward: `axis` is the camera right or up vector and
/// `signed_half` the half-extent with the translation's sign.
pub fn probe_endpoints(
    original: [f32; 3],
    translated: [f32; 3],
    forward: [f32; 3],
    axis: [f32; 3],
    signed_half: f32,
    near_clip: f32,
) -> ([f32; 3], [f32; 3]) {
    let forward = normalize(forward);
    let axis = normalize(axis);
    let mut from = original;
    let mut to = translated;
    for i in 0..3 {
        from[i] += forward[i] * near_clip + axis[i] * signed_half;
        to[i] += forward[i] * near_clip + axis[i] * signed_half;
    }
    (from, to)
}

/// The correction for a near-clip probe hit at fraction `hit`.
///
/// Pulls the translation back past the surface plus the keep-distance; when
/// the probe hit closer than the keep-distance allows, the whole translation
/// reverts instead. `horizontal` selects which axes the correction carries.
pub fn clip_correction(
    from: [f32; 3],
    to: [f32; 3],
    hit: f32,
    original: [f32; 3],
    translated: [f32; 3],
    horizontal: bool,
) -> [f32; 3] {
    let delta = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let mut correction = if horizontal {
        [-(delta[0] * (1.0 - hit)), -(delta[1] * (1.0 - hit)), 0.0]
    } else {
        [0.0, 0.0, -(delta[2] * (1.0 - hit))]
    };
    let reached = if horizontal {
        [delta[0] * hit, delta[1] * hit, 0.0]
    } else {
        [0.0, 0.0, delta[2] * hit]
    };
    if length(reached) >= KEEP_DISTANCE_FROM_WALL {
        let unit = normalize(correction);
        for i in 0..3 {
            correction[i] += unit[i] * KEEP_DISTANCE_FROM_WALL;
        }
    } else if horizontal {
        correction = [
            original[0] - translated[0],
            original[1] - translated[1],
            0.0,
        ];
    } else {
        correction = [0.0, 0.0, original[2] - translated[2]];
    }
    correction
}

/// The camera-body sweep corrections for a hit at fraction `hit`.
///
/// Returns the horizontal (x, y) and vertical (z) parts separately: the
/// caller compares each against the near-clip correction of the same axis
/// and applies whichever is larger.
pub fn body_corrections(
    original: [f32; 3],
    translated: [f32; 3],
    hit: f32,
) -> ([f32; 3], [f32; 3]) {
    let horizontal = [
        -((translated[0] - original[0]) * (1.0 - hit)),
        -((translated[1] - original[1]) * (1.0 - hit)),
        0.0,
    ];
    let vertical = [0.0, 0.0, -((translated[2] - original[2]) * (1.0 - hit))];
    (horizontal, vertical)
}

/// Length compare used to pick the larger of two corrections.
pub fn longer(a: [f32; 3], b: [f32; 3]) -> bool {
    length(a) > length(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(a: [f32; 3], b: [f32; 3]) {
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() < 1e-5, "{a:?} != {b:?}");
        }
    }

    #[test]
    fn look_at_is_orthonormal() {
        let [f, r, u] = look_at_basis([0.0, 0.0, 0.0], [3.0, 4.0, 0.0]).unwrap();
        assert!((length(f) - 1.0).abs() < 1e-6);
        assert!((length(r) - 1.0).abs() < 1e-6);
        assert!((length(u) - 1.0).abs() < 1e-6);
        let dot = f[0] * r[0] + f[1] * r[1] + f[2] * r[2];
        assert!(dot.abs() < 1e-6);
        assert_close(f, [0.6, 0.8, 0.0]);
    }

    #[test]
    fn look_at_degenerate_is_none() {
        assert!(look_at_basis([1.0, 2.0, 3.0], [1.0, 2.0, 3.0]).is_none());
    }

    #[test]
    fn translate_shoulder_moves_perpendicular() {
        // Camera at origin looking down +x: rightward offset moves -y.
        let camera = [0.0, 0.0, 0.0];
        let unit = [10.0, 0.0, 0.0];
        let right = translate_camera(camera, unit, 1.0, 0.0, None);
        assert_close(right, [0.0, -1.0, 0.0]);
        let left = translate_camera(camera, unit, -1.0, 0.0, None);
        assert_close(left, [0.0, 1.0, 0.0]);
    }

    #[test]
    fn translate_first_person_is_untouched() {
        let camera = [0.0, 0.0, 0.0];
        let unit = [0.3, 0.0, 0.0];
        assert_close(translate_camera(camera, unit, 2.0, 2.0, None), camera);
    }

    #[test]
    fn translate_pin_swaps_heights() {
        let camera = [0.0, 0.0, 5.0];
        let unit = [10.0, 0.0, 0.0];
        let pin = PinHeight {
            eye_height: 1.8,
            box_height: 2.4,
        };
        let out = translate_camera(camera, unit, 0.0, 0.0, Some(&pin));
        assert_close(out, [0.0, 0.0, 5.0 - 1.8 + 2.4]);
    }

    #[test]
    fn pitch_raises_forward_and_stays_orthonormal() {
        let [f, r, u] = pitch_basis([1.0, 0.0, 0.0], 0.3).unwrap();
        assert!(f[2] > 0.0);
        assert!((length(f) - 1.0).abs() < 1e-6);
        let dot = f[0] * r[0] + f[1] * r[1] + f[2] * r[2];
        assert!(dot.abs() < 1e-6);
        assert!(u[2] > 0.0);
    }

    #[test]
    fn probe_endpoints_sit_on_the_near_plane_corner() {
        let (from, to) = probe_endpoints(
            [0.0, 0.0, 0.0],
            [0.0, 2.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            0.5,
            0.3,
        );
        assert_close(from, [0.3, 0.0, 0.5]);
        assert_close(to, [0.3, 2.0, 0.5]);
    }

    #[test]
    fn clip_correction_pulls_back_or_reverts() {
        let original = [0.0, 0.0, 0.0];
        let translated = [2.0, 0.0, 0.0];
        // A hit at 0.5: one yard reached, past the keep distance: pull back
        // the unreached yard plus the keep distance.
        let pulled = clip_correction(original, translated, 0.5, original, translated, true);
        assert_close(pulled, [-1.2, 0.0, 0.0]);
        // A hit at 0.05: only 0.1 reached, under the keep distance: revert.
        let reverted = clip_correction(original, translated, 0.05, original, translated, true);
        assert_close(reverted, [-2.0, 0.0, 0.0]);
    }

    #[test]
    fn body_corrections_split_axes() {
        let (h, v) = body_corrections([0.0, 0.0, 0.0], [4.0, 2.0, 1.0], 0.75);
        assert_close(h, [-1.0, -0.5, 0.0]);
        assert_close(v, [0.0, 0.0, -0.25]);
        assert!(longer(h, v));
    }
}
