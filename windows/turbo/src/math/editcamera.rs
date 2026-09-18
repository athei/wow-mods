//! Camera-offset arithmetic behind the camera-editing feature.
//!
//! Pure kernels: the shoulder/height translation of the camera position, the
//! look-at and pitch basis rebuilds, and up to three translation collision
//! probes. New arithmetic for the feature, rather than a reimplementation of
//! a client function. These rays do not cover a rotating camera volume.

/// The float tolerance shared by every near-zero test here.
const TOLERANCE: f32 = 1e-5;

/// Clearance along the translation path before a hit surface.
const KEEP_DISTANCE_FROM_WALL: f64 = 0.2;

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
    if !forward.iter().all(|v| v.is_finite()) || almost_zero(forward) {
        return None;
    }
    let forward = normalize(forward);
    if almost_zero(forward) {
        return None;
    }
    let mut right = cross([0.0, 0.0, 1.0], forward);
    if length(right) == 0.0 {
        // A vertical view has no preferred horizontal heading. Use the y
        // axis as its reference so neither of the remaining rows collapses.
        right = cross([0.0, 1.0, 0.0], forward);
    }
    let right = normalize(right);
    let up = normalize(cross(forward, right));
    Some([forward, right, up])
}

/// The eye position and the forward, right and up rows of a camera.
pub struct CameraPose {
    /// World-space eye position.
    pub position: [f32; 3],
    /// Camera basis rows in forward, right, up order.
    pub basis: [[f32; 3]; 3],
}

/// The dimensions of a camera's rectangular near plane.
pub struct NearPlane {
    /// Distance from the eye along the forward row.
    pub distance: f32,
    /// Extent on either side of the plane center along the right row.
    pub half_width: f32,
    /// Extent on either side of the plane center along the up row.
    pub half_height: f32,
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

/// Whether translation requires checking, including invalid coordinates.
pub fn position_changed(from: &[f32; 3], to: &[f32; 3]) -> bool {
    from.iter()
        .zip(to)
        .any(|(a, b)| !a.is_finite() || !b.is_finite() || (a - b).abs() > TOLERANCE)
}

/// Correct a translation with at most three rays and no geometry allocation.
///
/// Probe the eye and the leading near-plane edge for each translated axis.
/// All rays follow the same translation. One minimum hit fraction, including
/// wall clearance, shortens the whole edit along that path without resweeps.
/// No result is cached across updates. Rotation and the space between these
/// rays remain unchecked; this is deliberately not a swept-volume guard.
/// Invalid inputs reject the edit before the callback is called.
pub fn correct_translation(
    original: &CameraPose,
    edited: &mut CameraPose,
    plane: &NearPlane,
    mut trace: impl FnMut(&[f32; 3], &[f32; 3]) -> Option<f32>,
) -> bool {
    if !original
        .position
        .iter()
        .chain(&edited.position)
        .all(|v| v.is_finite())
        || !edited.basis.iter().all(|row| {
            row.iter().all(|v| v.is_finite()) && length(*row).is_finite() && length(*row) > 0.0
        })
        || ![plane.distance, plane.half_width, plane.half_height]
            .iter()
            .all(|v| v.is_finite() && *v > 0.0)
    {
        return false;
    }
    let delta = core::array::from_fn::<_, 3, _>(|i| {
        f64::from(edited.position[i]) - f64::from(original.position[i])
    });
    let distance_sq = delta.iter().map(|v| v * v).sum::<f64>();
    if distance_sq > 150.0 * 150.0 {
        return false;
    }
    if !position_changed(&original.position, &edited.position) {
        return true;
    }
    let [forward, right, up] = edited.basis.map(normalize);
    let mut rays = [(original.position, edited.position); 3];
    let mut count = 1;
    for (axis, half, needed) in [
        (
            right,
            plane.half_width,
            delta[0].abs() > f64::from(TOLERANCE) || delta[1].abs() > f64::from(TOLERANCE),
        ),
        (up, plane.half_height, delta[2].abs() > f64::from(TOLERANCE)),
    ] {
        if !needed {
            continue;
        }
        let along = delta
            .iter()
            .zip(axis)
            .map(|(d, a)| d * f64::from(a))
            .sum::<f64>();
        let signed_half = if along > 0.0 { half } else { -half };
        let offset = core::array::from_fn::<_, 3, _>(|i| {
            forward[i] * plane.distance + axis[i] * signed_half
        });
        rays[count] = (
            core::array::from_fn(|i| original.position[i] + offset[i]),
            core::array::from_fn(|i| edited.position[i] + offset[i]),
        );
        count += 1;
    }
    if !rays[..count]
        .iter()
        .all(|(from, to)| from.iter().chain(to).all(|v| v.is_finite()))
    {
        return false;
    }
    let mut closest = None::<f64>;
    for (from, to) in &rays[..count] {
        if let Some(hit) = trace(from, to) {
            if !hit.is_finite() || hit < 0.0 {
                return false;
            }
            if hit <= 1.0 {
                let hit = f64::from(hit);
                closest = Some(closest.map_or(hit, |prior| prior.min(hit)));
                if hit == 0.0 {
                    break;
                }
            }
        }
    }
    if let Some(hit) = closest {
        let fraction = (hit - KEEP_DISTANCE_FROM_WALL / distance_sq.sqrt()).max(0.0);
        edited.position = core::array::from_fn(|i| {
            (f64::from(original.position[i]) + delta[i] * fraction) as f32
        });
    }
    true
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
    fn vertical_view_keeps_three_unit_basis_rows() {
        for z in [-1.0, 1.0] {
            let basis = look_at_basis([0.0; 3], [0.0, 0.0, z]).unwrap();
            for row in basis {
                assert!((length(row) - 1.0).abs() < 1e-6);
            }
            assert_close(cross(basis[0], basis[1]), basis[2]);
        }
    }

    #[test]
    fn invalid_direction_does_not_build_a_basis() {
        assert!(pitch_basis([1.0, 0.0, 0.0], f32::NAN).is_none());
        assert!(look_at_basis([0.0; 3], [f32::INFINITY, 0.0, 0.0]).is_none());
        assert!(look_at_basis([0.0; 3], [f32::MAX; 3]).is_none());
    }

    #[test]
    fn almost_vertical_view_preserves_its_heading() {
        let basis = look_at_basis([0.0; 3], [1e-6, 0.0, 1.0]).unwrap();
        assert_close(basis[1], [0.0, 1.0, 0.0]);
        assert_close(cross(basis[0], basis[1]), basis[2]);
    }

    fn stock_pose() -> CameraPose {
        CameraPose {
            position: [0.0; 3],
            basis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        }
    }

    const PLANE: NearPlane = NearPlane {
        distance: 0.5,
        half_width: 0.5,
        half_height: 0.5,
    };

    #[test]
    fn ray_budget_is_zero_for_rotation_and_at_most_three_for_translation() {
        let original = stock_pose();
        for (position, expected) in [
            ([0.0; 3], 0),
            ([0.0, 1.0, 0.0], 2),
            ([0.0, 0.0, 1.0], 2),
            ([0.0, 1.0, 1.0], 3),
        ] {
            let mut edited = CameraPose {
                position,
                basis: pitch_basis(original.basis[0], 0.3).unwrap(),
            };
            let mut calls = 0;
            assert!(correct_translation(
                &original,
                &mut edited,
                &PLANE,
                |_, _| {
                    calls += 1;
                    None
                }
            ));
            assert_eq!(calls, expected);
            assert_eq!(edited.position, position);
        }
    }

    #[test]
    fn leading_edge_uses_displacement_and_moving_geometry_is_rechecked() {
        let original = stock_pose();
        for blocked in [false, true] {
            let mut edited = CameraPose {
                position: translate_camera(original.position, [10.0, 0.0, 0.0], 1.0, 0.0, None),
                basis: original.basis,
            };
            let mut saw_leading_edge = false;
            assert!(correct_translation(
                &original,
                &mut edited,
                &PLANE,
                |from, to| {
                    if from[1] < -0.4 {
                        saw_leading_edge = true;
                        assert_eq!(from[1], -0.5);
                        assert_eq!(to[1], -1.5);
                        if blocked {
                            return Some(0.7);
                        }
                    }
                    None
                }
            ));
            assert!(saw_leading_edge);
            assert_close(
                edited.position,
                [0.0, if blocked { -0.5 } else { -1.0 }, 0.0],
            );
        }
    }

    #[test]
    fn endpoint_hits_keep_clearance_and_diagonal_corrections_share_one_fraction() {
        let original = stock_pose();
        let mut edited = CameraPose {
            position: [0.0, 3.0, 4.0],
            basis: original.basis,
        };
        let mut hits = [Some(1.0), Some(0.8), Some(0.5)].into_iter();
        assert!(correct_translation(
            &original,
            &mut edited,
            &PLANE,
            |_, _| hits.next().unwrap()
        ));
        assert_close(edited.position, [0.0, 1.38, 1.84]);
        let mut endpoint = CameraPose {
            position: [0.0, 1.0, 0.0],
            basis: original.basis,
        };
        assert!(correct_translation(
            &original,
            &mut endpoint,
            &PLANE,
            |_, _| Some(1.0)
        ));
        assert_close(endpoint.position, [0.0, 0.8, 0.0]);
        let mut close = CameraPose {
            position: [0.0, 0.1, 0.0],
            basis: original.basis,
        };
        let mut calls = 0;
        assert!(correct_translation(
            &original,
            &mut close,
            &PLANE,
            |_, _| {
                calls += 1;
                Some(0.0)
            }
        ));
        assert_eq!(calls, 1);
        assert_eq!(close.position, original.position);
    }

    #[test]
    fn invalid_and_overlong_inputs_never_reach_a_world_query() {
        let original = stock_pose();
        for position in [
            [f32::NAN, 0.0, 0.0],
            [f32::INFINITY, 0.0, 0.0],
            [151.0, 0.0, 0.0],
        ] {
            assert!(position_changed(&original.position, &position));
            let mut edited = CameraPose {
                position,
                basis: original.basis,
            };
            assert!(!correct_translation(
                &original,
                &mut edited,
                &PLANE,
                |_, _| panic!("invalid query")
            ));
        }
        let mut edited = CameraPose {
            position: [0.0, 1.0, 0.0],
            basis: [[0.0; 3]; 3],
        };
        assert!(!correct_translation(
            &original,
            &mut edited,
            &PLANE,
            |_, _| panic!("invalid basis")
        ));
        edited.basis = original.basis;
        let plane = NearPlane {
            distance: f32::NAN,
            half_width: 0.5,
            half_height: 0.5,
        };
        assert!(!correct_translation(
            &original,
            &mut edited,
            &plane,
            |_, _| panic!("invalid plane")
        ));
        let position = edited.position;
        assert!(!correct_translation(
            &original,
            &mut edited,
            &PLANE,
            |_, _| Some(f32::NAN)
        ));
        assert_eq!(edited.position, position);
    }
}
