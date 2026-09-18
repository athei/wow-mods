//! Projection dimensions and coordinate space for the camera collision guard.
//!
//! The renderer uses a diagonal FOV parameter and publishes a float projection
//! matrix. Recovering the near rectangle from those stored values avoids a
//! horizontal-FOV approximation and a stale cached viewport aspect. These are
//! guard dimensions, with an outward allowance, not a replacement for FPTAN.

use super::editcamera::NearPlane;

/// Smallest supported viewport aspect for the collision guard.
const MIN_ASPECT: f64 = 1.0 / 64.0;

/// Largest supported viewport aspect for the collision guard.
const MAX_ASPECT: f64 = 64.0;

/// Positive-float representable steps reserved for outward rounding.
const OUTWARD_STEPS: u32 = 8;

fn valid_aspect(aspect: f64) -> bool {
    (MIN_ASPECT..=MAX_ASPECT).contains(&aspect)
}

/// The finite effective FOV, with the offset disabled or added before clamping.
///
/// The addition and clamp comparisons remain wide until the output float store.
/// A disabled offset is unused and need not be finite. Invalid bounds or an
/// active nonfinite input return `None`; projection validity is checked separately.
pub fn effective_fov(
    base: f32,
    offset: f32,
    disable_offset: bool,
    lo: f32,
    hi: f32,
) -> Option<f32> {
    if !base.is_finite()
        || (!disable_offset && !offset.is_finite())
        || !lo.is_finite()
        || !hi.is_finite()
        || lo > hi
    {
        return None;
    }
    let value = if disable_offset {
        f64::from(base)
    } else {
        f64::from(base) + f64::from(offset)
    };
    Some(super::f64_to_f32(value.clamp(f64::from(lo), f64::from(hi))))
}

/// Aspect from the engine's current four-float viewport rectangle.
///
/// The engine ratio is `(rect[3] - rect[1]) / (rect[2] - rect[0])`. Both
/// differences and the division stay wide until the one float store. Empty,
/// inverted, nonfinite, or unsupported rectangles return `None`, so a caller
/// can reject a camera edit instead of using a stale aspect after a resize.
pub fn viewport_aspect(rect: &[f32; 4]) -> Option<f32> {
    if !rect.iter().all(|v| v.is_finite()) {
        return None;
    }
    let denominator = f64::from(rect[2]) - f64::from(rect[0]);
    let numerator = f64::from(rect[3]) - f64::from(rect[1]);
    if denominator <= 0.0 || numerator <= 0.0 {
        return None;
    }
    let aspect = numerator / denominator;
    valid_aspect(aspect).then(|| super::f64_to_f32(aspect))
}

/// A slightly expanded near rectangle from the renderer's projection arithmetic.
///
/// The diagonal FOV must be in `(0, pi)`, aspect in `[1/64, 64]`, and clip
/// distances positive normal floats with `far > near`. Projection entries
/// narrow to float before recovering the rectangle. Unrepresentable entries or
/// dimensions return `None` rather than supplying unchecked guard geometry.
///
/// Each published dimension advances eight positive-float representable steps.
/// This covers downward float publication rounding and leaves several float
/// ULPs for transcendental and intermediate-rounding differences on the bounded
/// input domain. It is an allowance, not a proven error bound for every FPTAN
/// implementation. In particular, this function does not claim bit equivalence
/// between `libm::tan` and the renderer's transcendental instruction.
///
/// The half extents are recovered using the expanded distance and then expanded
/// themselves. Therefore the volume from the eye to this rectangle contains the
/// unexpanded rectangle recovered from these stored projection coefficients.
pub fn near_plane(diagonal_fov: f32, aspect: f32, near: f32, far: f32) -> Option<NearPlane> {
    let fov = f64::from(diagonal_fov);
    let aspect = f64::from(aspect);
    if !fov.is_finite()
        || fov <= 0.0
        || fov >= core::f64::consts::PI
        || !valid_aspect(aspect)
        || !near.is_normal()
        || near <= 0.0
        || !far.is_normal()
        || far <= near
    {
        return None;
    }
    let near = f64::from(near);
    let far = f64::from(far);
    let h = libm::tan((fov / (aspect * aspect + 1.0).sqrt()) * 0.5);
    let height = h * near;
    let depth = far - near;
    let m00 = super::f64_to_f32(near / (aspect * height));
    let m11 = super::f64_to_f32(near / height);
    let m22 = super::f64_to_f32((near + far) / depth);
    let m32 = super::f64_to_f32((-2.0 * near * far) / depth);
    if ![m00, m11, m22, m32].iter().all(|v| v.is_finite()) || m00 <= 0.0 || m11 <= 0.0 {
        return None;
    }

    let recovered = -f64::from(m32) / (f64::from(m22) + 1.0);
    let distance = outward(recovered)?;
    let half_width = outward(f64::from(distance) / f64::from(m00))?;
    let half_height = outward(f64::from(distance) / f64::from(m11))?;
    Some(NearPlane {
        distance,
        half_width,
        half_height,
    })
}

fn outward(value: f64) -> Option<f32> {
    let rounded = super::f64_to_f32(value);
    if !rounded.is_normal() || rounded <= 0.0 {
        return None;
    }
    let expanded = f32::from_bits(rounded.to_bits().checked_add(OUTWARD_STEPS)?);
    expanded.is_finite().then_some(expanded)
}

/// A validated local-to-world camera rotation and its numerical inverse.
///
/// Camera axes are row vectors: a local row multiplies this row-major matrix
/// on the left. The inverse uses the stored float matrix, rather than assuming
/// its transpose is exact after trigonometric rounding.
pub struct CameraSpace {
    rotation: [f32; 9],
    inverse: [f64; 9],
}

impl CameraSpace {
    /// Validate a finite proper rotation and retain its inverse for write-back.
    ///
    /// Float trigonometric rounding is allowed by a `1e-4` Gram-matrix tolerance.
    /// Scale, shear, reflections and singular matrices cannot describe a camera
    /// transport and are rejected instead of stretching the collision rectangle.
    pub fn from_rotation(rotation: [f32; 9]) -> Option<Self> {
        if !rotation.iter().all(|v| v.is_finite()) {
            return None;
        }
        let m = rotation.map(f64::from);
        for row in 0..3 {
            for other in row..3 {
                let dot = (m[row * 3] * m[other * 3] + m[row * 3 + 1] * m[other * 3 + 1])
                    + m[row * 3 + 2] * m[other * 3 + 2];
                let expected = f64::from(row == other);
                if (dot - expected).abs() > 1e-4 {
                    return None;
                }
            }
        }
        let adj = [
            m[4] * m[8] - m[5] * m[7],
            m[2] * m[7] - m[1] * m[8],
            m[1] * m[5] - m[2] * m[4],
            m[5] * m[6] - m[3] * m[8],
            m[0] * m[8] - m[2] * m[6],
            m[2] * m[3] - m[0] * m[5],
            m[3] * m[7] - m[4] * m[6],
            m[1] * m[6] - m[0] * m[7],
            m[0] * m[4] - m[1] * m[3],
        ];
        let determinant = m[0] * adj[0] + m[1] * adj[3] + m[2] * adj[6];
        if !determinant.is_finite() || determinant < 0.5 {
            return None;
        }
        Some(Self {
            rotation,
            inverse: adj.map(|v| v / determinant),
        })
    }

    /// Transform stored camera rows using the renderer's component fold order.
    ///
    /// Each dot stays wide through its two additions and narrows once. The
    /// first component folds z/y/x; the other two fold z/x/y.
    pub fn local_to_world(&self, basis: [[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
        let m = self.rotation.map(f64::from);
        let transformed = basis.map(|row| {
            let [x, y, z] = row.map(f64::from);
            [
                super::f64_to_f32((z * m[6] + y * m[3]) + x * m[0]),
                super::f64_to_f32((z * m[7] + x * m[1]) + y * m[4]),
                super::f64_to_f32((z * m[8] + x * m[2]) + y * m[5]),
            ]
        });
        finite_basis(transformed)
    }

    /// Convert a world-space candidate to the camera's stored local rows.
    ///
    /// The renderer consumes the resulting floats again, so callers validate
    /// `renderer_basis` of this result before publishing a candidate camera.
    pub fn world_to_local(&self, basis: [[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
        let m = self.inverse;
        let transformed = basis.map(|row| {
            let [x, y, z] = row.map(f64::from);
            core::array::from_fn(|i| super::f64_to_f32((z * m[6 + i] + y * m[3 + i]) + x * m[i]))
        });
        finite_basis(transformed)
    }

    /// Rebuild the rendered frame from transformed forward and up rows.
    ///
    /// The look-at builder spills each forward square before its wide sum,
    /// spills each cross-product component before normalizing it, and scales
    /// by a wide reciprocal. The raw right row is not used by that builder.
    /// The returned right is the camera convention `up cross forward`; it is
    /// the negation of the renderer's mirrored view-side column. Its sign does
    /// not change the symmetric near rectangle, and preserves stored camera
    /// handedness when converting a validated candidate back to local space.
    pub fn renderer_basis(&self, raw: [[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
        let world = self.local_to_world(raw)?;
        let squares = world[0].map(|v| f64::from(v * v));
        let forward_length2 = (squares[1] + squares[2]) + squares[0];
        let up = world[2].map(f64::from);
        let up_length2 = (up[0] * up[0] + up[1] * up[1]) + up[2] * up[2];
        // The renderer rejects either input below its 0.01 squared-length
        // threshold. Returning no frame keeps its fallback out of the guard.
        let threshold = f64::from(0.01f32);
        if forward_length2 < threshold || up_length2 < threshold {
            return None;
        }
        let forward = normalize_with_length2(world[0], forward_length2)?;
        let side = super::vector::c3_vector__cross__672130(&forward, &world[2]);
        let side = normalize_with_length2(side, squared_length_zyx(side))?;
        let up = super::vector::c3_vector__cross__672130(&side, &forward);
        let up = normalize_with_length2(up, squared_length_zyx(up))?;
        Some([forward, side.map(|v| -v), up])
    }
}

fn finite_basis(basis: [[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
    basis
        .iter()
        .flatten()
        .all(|v| v.is_finite())
        .then_some(basis)
}

fn squared_length_zyx(vector: [f32; 3]) -> f64 {
    let [x, y, z] = vector.map(f64::from);
    (z * z + y * y) + x * x
}

fn normalize_with_length2(vector: [f32; 3], length2: f64) -> Option<[f32; 3]> {
    if !length2.is_finite() || length2 <= 0.0 {
        return None;
    }
    let inverse = 1.0 / length2.sqrt();
    let out = vector.map(|v| super::f64_to_f32(f64::from(v) * inverse));
    out.iter().all(|v| v.is_finite()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::{CameraSpace, effective_fov, near_plane, viewport_aspect};

    const IDENTITY: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    const BASIS: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    fn assert_basis_close(actual: [[f32; 3]; 3], expected: [[f32; 3]; 3]) {
        for (actual, expected) in actual.iter().flatten().zip(expected.iter().flatten()) {
            assert!((actual - expected).abs() < 2e-6, "{actual} != {expected}");
        }
    }

    #[test]
    fn identity_transport_preserves_both_coordinate_directions() {
        let space = CameraSpace::from_rotation(IDENTITY).unwrap();
        let basis = [[0.3, 0.4, 0.5], [-0.2, 0.7, 0.1], [0.0, 0.1, 1.0]];
        assert_eq!(space.local_to_world(basis), Some(basis));
        assert_eq!(space.world_to_local(basis), Some(basis));
        assert_eq!(space.renderer_basis(BASIS), Some(BASIS));
    }

    #[test]
    fn quarter_turn_uses_row_vector_matrix_semantics() {
        let rotation = [0.0, -1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        let space = CameraSpace::from_rotation(rotation).unwrap();
        let expected = [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        assert_eq!(space.local_to_world(BASIS), Some(expected));
        assert_eq!(space.world_to_local(expected), Some(BASIS));
        assert_eq!(space.renderer_basis(BASIS), Some(expected));
    }

    #[test]
    fn rounded_transport_inverse_round_trips_a_camera_basis() {
        let rotation = super::super::matrix33::c33_matrix__from_axis_angle__7be490(
            &[0.0, 0.0, 1.0],
            0.731,
            false,
        );
        let space = CameraSpace::from_rotation(rotation).unwrap();
        let local = super::super::editcamera::look_at_basis([0.0; 3], [0.3, 0.4, 0.5]).unwrap();
        let world = space.local_to_world(local).unwrap();
        assert_basis_close(space.world_to_local(world).unwrap(), local);
    }

    #[test]
    fn candidate_write_back_rebuilds_the_validated_rendered_frame() {
        let rotation = super::super::matrix33::c33_matrix__from_axis_angle__7be490(
            &[0.0, 0.0, 1.0],
            -1.17,
            false,
        );
        let space = CameraSpace::from_rotation(rotation).unwrap();
        let world = super::super::editcamera::look_at_basis([0.0; 3], [0.8, -0.2, 0.4]).unwrap();
        let stored = space.world_to_local(world).unwrap();
        let rendered = space.renderer_basis(stored).unwrap();
        assert_basis_close(rendered, world);
        let stored_again = space.world_to_local(rendered).unwrap();
        assert_basis_close(space.renderer_basis(stored_again).unwrap(), rendered);
    }

    #[test]
    fn renderer_forward_keeps_the_individual_square_stores() {
        let space = CameraSpace::from_rotation(IDENTITY).unwrap();
        let forward = [0x3e92_8028, 0xbec1_39fb, 0x3f79_5bb6].map(f32::from_bits);
        let rendered = space.renderer_basis([forward, BASIS[1], BASIS[2]]).unwrap();
        assert_eq!(
            rendered[0].map(f32::to_bits),
            [0x3e87_42fe, 0xbeb2_6727, 0x3f66_3a7f]
        );
        // Keeping every square wide produces z=0x3f663a80 instead.
    }

    #[test]
    fn renderer_reconstructs_the_orthogonal_up_instead_of_trusting_raw_right() {
        let space = CameraSpace::from_rotation(IDENTITY).unwrap();
        let raw = [[2.0, 0.0, 0.0], [7.0, 3.0, 2.0], [1.0, 0.0, 2.0]];
        assert_eq!(space.renderer_basis(raw), Some(BASIS));
    }

    #[test]
    fn invalid_transport_matrices_are_rejected() {
        for matrix in [
            [0.0; 9],
            [1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
            [2.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            [1.0, 0.2, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            [-1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            [f32::INFINITY; 9],
            [f32::NAN; 9],
        ] {
            assert!(CameraSpace::from_rotation(matrix).is_none());
        }
    }

    #[test]
    fn renderer_rejects_nonfinite_short_and_parallel_input_axes() {
        let space = CameraSpace::from_rotation(IDENTITY).unwrap();
        for (forward, up) in [
            ([0.0; 3], BASIS[2]),
            ([0.09, 0.0, 0.0], BASIS[2]),
            (BASIS[0], [0.0; 3]),
            (BASIS[0], [0.0, 0.0, 0.09]),
            (BASIS[0], BASIS[0]),
            ([f32::NAN, 0.0, 0.0], BASIS[2]),
            (BASIS[0], [0.0, f32::INFINITY, 1.0]),
            ([f32::MAX, 0.0, 0.0], BASIS[2]),
        ] {
            assert!(space.renderer_basis([forward, BASIS[1], up]).is_none());
        }
        assert!(space.world_to_local([[f32::NAN; 3]; 3]).is_none());
        assert!(space.local_to_world([[f32::INFINITY; 3]; 3]).is_none());
    }

    #[test]
    fn diagonal_fov_matches_published_projection_dimensions() {
        // Independently calculated dimensions from the stored float matrix for
        // FOV pi/2, near0.1, far1000. The guard must exceed these while keeping
        // its margin small. A horizontal-FOV interpretation gives width0.1.
        for (aspect, width, height) in [
            (1.0, 0.062_050_492_000_131_23, 0.062_050_492_000_131_23),
            (
                16.0 / 9.0,
                0.072_049_876_365_535_24,
                0.040_528_055_020_493_496,
            ),
            (
                9.0 / 16.0,
                0.045_910_556_183_555_475,
                0.081_618_766_548_543_07,
            ),
            (
                32.0 / 9.0,
                0.076_766_991_954_165_89,
                0.021_590_715_499_189_405,
            ),
        ] {
            let plane = near_plane(core::f32::consts::FRAC_PI_2, aspect, 0.1, 1000.0).unwrap();
            for (actual, published) in [
                (plane.distance, 0.099_999_998_510_032_9),
                (plane.half_width, width),
                (plane.half_height, height),
            ] {
                assert!(f64::from(actual) > published);
                assert!(f64::from(actual) < published * 1.000_003);
            }
            assert!(plane.half_width < 0.09);
        }
    }

    #[test]
    fn portrait_and_wide_views_have_the_expected_rectangle_orientation() {
        let portrait = near_plane(1.5, 0.5, 0.2, 500.0).unwrap();
        let wide = near_plane(1.5, 2.0, 0.2, 500.0).unwrap();
        assert!(portrait.half_height > portrait.half_width);
        assert!(wide.half_width > wide.half_height);
    }

    #[test]
    fn current_viewport_replaces_the_pre_resize_aspect() {
        let before = viewport_aspect(&[0.0, 0.0, 1080.0, 1920.0]).unwrap();
        let after = viewport_aspect(&[0.0, 0.0, 1920.0, 1080.0]).unwrap();
        assert_eq!(before, 16.0 / 9.0);
        assert_eq!(after, 9.0 / 16.0);
        let stale = near_plane(1.5, before, 0.1, 1000.0).unwrap();
        let current = near_plane(1.5, after, 0.1, 1000.0).unwrap();
        assert!(current.half_height > stale.half_height * 1.9);
        assert!(current.half_width < stale.half_width);
    }

    #[test]
    fn viewport_difference_stays_wide_until_the_ratio_store() {
        // A float subtraction would round 2^24+1 down before the division and
        // produce 2^-4. Keeping the difference wide produces its predecessor.
        let rect = [-1.0, 0.0, 16_777_216.0, 1_048_576.0];
        let aspect = viewport_aspect(&rect).unwrap();
        assert_eq!(aspect.to_bits(), 0x3d7f_ffff);
    }

    #[test]
    fn effective_fov_covers_offset_clamps_and_disabled_offset() {
        assert_eq!(effective_fov(1.0, 0.5, false, 0.25, 2.0), Some(1.5));
        assert_eq!(effective_fov(1.0, -2.0, false, 0.25, 2.0), Some(0.25));
        assert_eq!(effective_fov(1.0, 2.0, false, 0.25, 2.0), Some(2.0));
        assert_eq!(effective_fov(1.0, f32::NAN, true, 0.25, 2.0), Some(1.0));
        assert_eq!(effective_fov(1.0, 0.5, false, 1.25, 1.25), Some(1.25));
        assert_eq!(
            effective_fov(f32::MAX, f32::MAX, false, 0.25, 2.0),
            Some(2.0)
        );
    }

    #[test]
    fn invalid_fov_inputs_and_bounds_are_rejected() {
        assert!(effective_fov(f32::NAN, 0.0, true, 0.25, 2.0).is_none());
        assert!(effective_fov(1.0, f32::NAN, false, 0.25, 2.0).is_none());
        assert!(effective_fov(1.0, 0.0, false, f32::NAN, 2.0).is_none());
        assert!(effective_fov(1.0, 0.0, false, 0.25, f32::INFINITY).is_none());
        assert!(effective_fov(1.0, 0.0, false, 2.0, 0.25).is_none());
    }

    #[test]
    fn invalid_projection_inputs_are_rejected() {
        for fov in [0.0, -1.0, core::f32::consts::PI, f32::INFINITY, f32::NAN] {
            assert!(near_plane(fov, 1.0, 0.1, 1000.0).is_none());
        }
        for aspect in [0.0, -1.0, 1.0 / 128.0, 128.0, f32::INFINITY, f32::NAN] {
            assert!(near_plane(1.5, aspect, 0.1, 1000.0).is_none());
        }
        for near in [0.0, -1.0, f32::from_bits(1), f32::INFINITY, f32::NAN] {
            assert!(near_plane(1.5, 1.0, near, 1000.0).is_none());
        }
        for far in [0.0, -1.0, 0.1, 0.05, f32::INFINITY, f32::NAN] {
            assert!(near_plane(1.5, 1.0, 0.1, far).is_none());
        }
        assert!(near_plane(1.5, 1.0, f32::MAX * 0.5, f32::MAX).is_none());
    }

    #[test]
    fn invalid_viewports_are_rejected() {
        for rect in [
            [0.0, 0.0, 0.0, 100.0],
            [0.0, 0.0, 100.0, 0.0],
            [100.0, 0.0, 0.0, 100.0],
            [0.0, 100.0, 100.0, 0.0],
            [0.0, 0.0, 100.0, f32::INFINITY],
            [f32::NAN, 0.0, 100.0, 100.0],
            [0.0, 0.0, 1.0, 100.0],
        ] {
            assert!(viewport_aspect(&rect).is_none());
        }
    }
}
