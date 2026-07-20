//! `frustum` family kernels.
#![allow(
    non_snake_case,
    // intentional NaN-reject via `!(a >= b)`
    // (differs from `a < b` on NaN), bit-exact source constants kept verbatim,
    // and ABI-dictated parameter counts.
    clippy::neg_cmp_op_on_partial_ord,
    clippy::excessive_precision,
    clippy::too_many_arguments
)]

/// Plane-distance epsilon baked into `Wow.exe` (`.rdata` @0x8101b4).
const FRUSTUM_PLANE_EPS: f32 = -0.019_444_443_3;

/// Classifies a point against six frustum planes, producing a clip outcode.
///
/// For each plane `p` (`[a, b, c, d]`, stride 4) with signed distance
/// `a*x + b*y + c*z + d`, a distance below the epsilon means the point is behind
/// that plane: the plane's bit `1 << p` is OR-ed into the mask. Returns
/// `(last_failing_bit, mask)` — the bit of the last failing plane (0 if fully
/// inside) plus the full outcode mask.
pub fn c_world_frustum__classify_point__686c20(planes: &[f32; 24], point: &[f32; 3]) -> (u32, u32) {
    let x = point[0];
    let y = point[1];
    let z = point[2];

    let mut out_mask: u32 = 0;
    let mut last_bit: u32 = 0;
    for p in 0..6u32 {
        let base = (p as usize) * 4;
        let dist =
            planes[base] * x + planes[base + 1] * y + planes[base + 2] * z + planes[base + 3];
        if dist < FRUSTUM_PLANE_EPS {
            let bit = 1u32 << p;
            out_mask |= bit;
            last_bit = bit;
        }
    }
    (last_bit, out_mask)
}

#[cfg(test)]
mod tests_c_world_frustum__classify_point__686c20 {
    use super::c_world_frustum__classify_point__686c20;

    fn box_frustum() -> [f32; 24] {
        [
            -1.0, 0.0, 0.0, 10.0, // p0: x <= 10
            1.0, 0.0, 0.0, 10.0, // p1: x >= -10
            0.0, -1.0, 0.0, 10.0, // p2: y <= 10
            0.0, 1.0, 0.0, 10.0, // p3: y >= -10
            0.0, 0.0, -1.0, 10.0, // p4: z <= 10
            0.0, 0.0, 1.0, 10.0, // p5: z >= -10
        ]
    }

    #[test]
    fn inside_point_zero_mask() {
        let (ret, mask) = c_world_frustum__classify_point__686c20(&box_frustum(), &[0.0, 0.0, 0.0]);
        assert_eq!(mask, 0);
        assert_eq!(ret, 0);
    }

    #[test]
    fn behind_plus_x_sets_bit0() {
        let (ret, mask) =
            c_world_frustum__classify_point__686c20(&box_frustum(), &[100.0, 0.0, 0.0]);
        assert_eq!(mask, 0b000001);
        assert_eq!(ret, 0b000001);
    }

    #[test]
    fn behind_minus_x_sets_bit1() {
        let (ret, mask) =
            c_world_frustum__classify_point__686c20(&box_frustum(), &[-100.0, 0.0, 0.0]);
        assert_eq!(mask, 0b000010);
        assert_eq!(ret, 0b000010);
    }

    #[test]
    fn corner_violates_two_planes_ret_is_last() {
        // x=100 (p0), z=100 (p4) -> mask bits 0 and 4; ret is the last failing plane (p4).
        let (ret, mask) =
            c_world_frustum__classify_point__686c20(&box_frustum(), &[100.0, 0.0, 100.0]);
        assert_eq!(mask, (1 << 0) | (1 << 4));
        assert_eq!(ret, 1 << 4);
    }

    #[test]
    fn boundary_within_eps_inside() {
        let (ret, mask) =
            c_world_frustum__classify_point__686c20(&box_frustum(), &[10.0, 0.0, 0.0]);
        assert_eq!(mask, 0);
        assert_eq!(ret, 0);
    }

    #[test]
    fn just_past_boundary_outside() {
        let (ret, mask) =
            c_world_frustum__classify_point__686c20(&box_frustum(), &[10.5, 0.0, 0.0]);
        assert_eq!(mask, 1);
        assert_eq!(ret, 1);
    }
}

/// Tests an oriented AABB against six frustum planes; returns 3 if the box may
/// be visible, 0 if any plane fully rejects it.
///
/// `planes` is 6 planes of 4 floats `[a, b, c, d]` (stride 4). `aabb` is two
/// `C3Vector`s at `[0..3]` and `[3..6]`. `orientation` is a row-major 3x3 matrix
/// (9 floats). `translation` is a 3-vector. The support corner is chosen per the
/// sign of the orientation-transformed plane normal: a negative component selects
/// `aabb[0..3]`, a non-negative one selects `aabb[3..6]`.
///
/// The plane distance is evaluated in the box's local frame: with `tn = O*n`
/// (the same transformed normal that picks the support corner), the world-space
/// distance `n . (O^T*c + t) + d` is computed as `tn . c + (n . t + d)` — equal
/// by the transpose identity `(O*n) . c == n . (O^T*c)` for any 3x3 `O` — which
/// removes the corner's transform back to world space (one 3x3 matrix-vector
/// product and one dot product per plane). Only final-`dist` rounding differs
/// from the world-space form; the classification can move solely within a
/// few-ULP band around the plane epsilon.
pub fn c_world_frustum__test_oriented_box__6869c0(
    planes: &[f32; 24],
    aabb: &[f32; 6],
    orientation: &[f32; 9],
    translation: &[f32; 3],
) -> u32 {
    let o = orientation;
    let t = translation;
    let extent_a = [aabb[0], aabb[1], aabb[2]];
    let extent_b = [aabb[3], aabb[4], aabb[5]];

    for p in 0..6 {
        let base = p * 4;
        let na = planes[base];
        let nb = planes[base + 1];
        let nc = planes[base + 2];
        let nd = planes[base + 3];

        // Transform the plane normal into the box's local frame: tn = orientation * n.
        let tnx = o[0] * na + o[1] * nb + o[2] * nc;
        let tny = o[3] * na + o[4] * nb + o[5] * nc;
        let tnz = o[6] * na + o[7] * nb + o[8] * nc;

        // Select the support corner from the sign bit of each local-normal
        // component (faithful to the original's integer `tn >> 31`, signed zero
        // included): negative picks `aabb[0..3]`, non-negative picks `aabb[3..6]`.
        let cx = if tnx.is_sign_negative() {
            extent_a[0]
        } else {
            extent_b[0]
        };
        let cy = if tny.is_sign_negative() {
            extent_a[1]
        } else {
            extent_b[1]
        };
        let cz = if tnz.is_sign_negative() {
            extent_a[2]
        } else {
            extent_b[2]
        };

        // Plane evaluated at the box translation, plus the support corner's
        // local projection onto the transformed normal.
        let plane_at_t = na * t[0] + nb * t[1] + nc * t[2] + nd;
        let dist = tnx * cx + tny * cy + tnz * cz + plane_at_t;
        if dist < FRUSTUM_PLANE_EPS {
            return 0;
        }
    }
    3
}

#[cfg(test)]
mod tests_c_world_frustum__test_oriented_box__6869c0 {
    use super::{FRUSTUM_PLANE_EPS, c_world_frustum__test_oriented_box__6869c0};

    // A frustum bounding the cube [-10,10]^3; inward normals, dist >= 0 inside.
    fn box_frustum() -> [f32; 24] {
        [
            -1.0, 0.0, 0.0, 10.0, // x <= 10
            1.0, 0.0, 0.0, 10.0, // x >= -10
            0.0, -1.0, 0.0, 10.0, // y <= 10
            0.0, 1.0, 0.0, 10.0, // y >= -10
            0.0, 0.0, -1.0, 10.0, // z <= 10
            0.0, 0.0, 1.0, 10.0, // z >= -10
        ]
    }

    fn identity() -> [f32; 9] {
        [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
    }

    #[test]
    fn box_inside_returns_three() {
        let aabb = [1.0f32, 1.0, 1.0, -1.0, -1.0, -1.0];
        let r = c_world_frustum__test_oriented_box__6869c0(
            &box_frustum(),
            &aabb,
            &identity(),
            &[0.0, 0.0, 0.0],
        );
        assert_eq!(r, 3);
    }

    #[test]
    fn box_far_outside_returns_zero() {
        let aabb = [1.0f32, 1.0, 1.0, -1.0, -1.0, -1.0];
        let r = c_world_frustum__test_oriented_box__6869c0(
            &box_frustum(),
            &aabb,
            &identity(),
            &[100.0, 0.0, 0.0],
        );
        assert_eq!(r, 0);
    }

    #[test]
    fn corner_selection_uses_signed_extent() {
        // For p0 (normal -1) the +x extent (aabb[0]=+2) is the support corner, so a
        // box centred at x=10 has its tested corner at world x=12, dist=-2 -> reject.
        let aabb = [2.0f32, 2.0, 2.0, -2.0, -2.0, -2.0];
        let r = c_world_frustum__test_oriented_box__6869c0(
            &box_frustum(),
            &aabb,
            &identity(),
            &[10.0, 0.0, 0.0],
        );
        assert_eq!(r, 0);
        // Pull the centre back so the +x corner lands inside -> visible.
        let r2 = c_world_frustum__test_oriented_box__6869c0(
            &box_frustum(),
            &aabb,
            &identity(),
            &[7.0, 0.0, 0.0],
        );
        assert_eq!(r2, 3);
    }

    #[test]
    fn rotation_invariance_unit_box() {
        let c = core::f32::consts::FRAC_1_SQRT_2;
        let rot = [c, -c, 0.0, c, c, 0.0, 0.0, 0.0, 1.0];
        let aabb = [3.0f32, 3.0, 3.0, -3.0, -3.0, -3.0];
        let r = c_world_frustum__test_oriented_box__6869c0(
            &box_frustum(),
            &aabb,
            &rot,
            &[0.0, 0.0, 0.0],
        );
        assert_eq!(r, 3);
    }

    #[test]
    fn corner_on_plane_within_eps_visible() {
        let aabb = [0.0f32, 0.0, 0.0, 0.0, 0.0, 0.0];
        let r = c_world_frustum__test_oriented_box__6869c0(
            &box_frustum(),
            &aabb,
            &identity(),
            &[10.0, 0.0, 0.0],
        );
        // dist at x=10 plane = 0 > eps -> visible.
        assert_eq!(r, 3);
        let r2 = c_world_frustum__test_oriented_box__6869c0(
            &box_frustum(),
            &aabb,
            &identity(),
            &[10.1, 0.0, 0.0],
        );
        assert_eq!(r2, 0);
    }

    #[test]
    fn eps_is_negative_small() {
        const { assert!(FRUSTUM_PLANE_EPS < 0.0 && FRUSTUM_PLANE_EPS > -0.1) };
    }

    /// The pre-fold world-space formulation, kept verbatim as the differential
    /// oracle: round-trip the support corner to world space and evaluate the
    /// plane there. Equal to the folded form by the transpose identity up to
    /// final-sum rounding.
    fn world_space_oracle(planes: &[f32; 24], aabb: &[f32; 6], o: &[f32; 9], t: &[f32; 3]) -> u32 {
        let extent_a = [aabb[0], aabb[1], aabb[2]];
        let extent_b = [aabb[3], aabb[4], aabb[5]];
        for p in 0..6 {
            let base = p * 4;
            let (na, nb, nc, nd) = (
                planes[base],
                planes[base + 1],
                planes[base + 2],
                planes[base + 3],
            );
            let tnx = o[0] * na + o[1] * nb + o[2] * nc;
            let tny = o[3] * na + o[4] * nb + o[5] * nc;
            let tnz = o[6] * na + o[7] * nb + o[8] * nc;
            let cx = if tnx.is_sign_negative() {
                extent_a[0]
            } else {
                extent_b[0]
            };
            let cy = if tny.is_sign_negative() {
                extent_a[1]
            } else {
                extent_b[1]
            };
            let cz = if tnz.is_sign_negative() {
                extent_a[2]
            } else {
                extent_b[2]
            };
            let wx = o[0] * cx + o[3] * cy + o[6] * cz + t[0];
            let wy = o[1] * cx + o[4] * cy + o[7] * cz + t[1];
            let wz = o[2] * cx + o[5] * cy + o[8] * cz + t[2];
            let dist = na * wx + nb * wy + nc * wz + nd;
            if dist < FRUSTUM_PLANE_EPS {
                return 0;
            }
        }
        3
    }

    fn check(planes: &[f32; 24], aabb: &[f32; 6], o: &[f32; 9], t: &[f32; 3], what: &str) {
        let got = c_world_frustum__test_oriented_box__6869c0(planes, aabb, o, t);
        let want = world_space_oracle(planes, aabb, o, t);
        assert_eq!(got, want, "{what}: aabb={aabb:?} o={o:?} t={t:?}");
    }

    /// Structured battery: degenerate/inverted/straddling/NaN/signed-zero/
    /// non-orthonormal inputs agree with the world-space formulation.
    #[test]
    fn differential_structured() {
        let f = box_frustum();
        let id = identity();
        let shear = [1.0, 0.4, 0.0, 0.0, 2.0, 0.0, 0.3, 0.0, 0.5];
        // Degenerate point box on each face plane and just past it.
        for axis in 0..3 {
            let mut t = [0.0f32; 3];
            t[axis] = 10.0;
            check(&f, &[0.0; 6], &id, &t, "point on plane");
            t[axis] = 10.1;
            check(&f, &[0.0; 6], &id, &t, "point past plane");
        }
        // Inverted extents (min/max swapped relative to convention).
        check(
            &f,
            &[-2.0, -2.0, -2.0, 2.0, 2.0, 2.0],
            &id,
            &[9.0, 0.0, 0.0],
            "inverted extents",
        );
        // Straddling each plane.
        for axis in 0..3 {
            let mut t = [0.0f32; 3];
            for s in [-1.0f32, 1.0] {
                t[axis] = s * 10.0;
                check(
                    &f,
                    &[1.5, 1.5, 1.5, -1.5, -1.5, -1.5],
                    &id,
                    &t,
                    "straddling",
                );
            }
        }
        // NaN in each input class keeps (never rejects spuriously) and agrees.
        let nan = f32::NAN;
        let mut p_nan = f;
        p_nan[0] = nan;
        check(&p_nan, &[1.0; 6], &id, &[0.0; 3], "NaN normal lane");
        let mut p_dnan = f;
        p_dnan[3] = nan;
        check(&p_dnan, &[1.0; 6], &id, &[0.0; 3], "NaN plane d");
        check(
            &f,
            &[nan, 1.0, 1.0, -1.0, -1.0, -1.0],
            &id,
            &[0.0; 3],
            "NaN aabb lane",
        );
        let mut o_nan = id;
        o_nan[4] = nan;
        check(
            &f,
            &[1.0, 1.0, 1.0, -1.0, -1.0, -1.0],
            &o_nan,
            &[0.0; 3],
            "NaN orientation lane",
        );
        check(
            &f,
            &[1.0, 1.0, 1.0, -1.0, -1.0, -1.0],
            &id,
            &[nan, 0.0, 0.0],
            "NaN translation",
        );
        // Signed-zero lanes drive the corner pick through the sign bit.
        let mut o_negz = id;
        o_negz[0] = -0.0;
        check(
            &f,
            &[1.0, 2.0, 3.0, -4.0, -5.0, -6.0],
            &o_negz,
            &[0.5, 0.5, 0.5],
            "signed-zero lane",
        );
        // Non-orthonormal orientation: the fold needs only the transpose identity.
        check(
            &f,
            &[1.0, 1.0, 1.0, -1.0, -1.0, -1.0],
            &shear,
            &[2.0, -3.0, 1.0],
            "shear orientation",
        );
    }

    /// Seeded random sweep at game-scale magnitudes: the folded form and the
    /// world-space oracle classify identically away from the epsilon knife edge.
    #[test]
    fn differential_random_sweep() {
        let mut state = 0x243f_6a88_85a3_08d3_u64;
        let next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut unit = {
            let mut n = next;
            move || (n() >> 11) as f32 / (1_u64 << 53) as f32
        };
        for _ in 0..50_000 {
            let mut planes = [0.0f32; 24];
            for p in 0..6 {
                let (a, b, c) = (unit() - 0.5, unit() - 0.5, unit() - 0.5);
                let len = (a * a + b * b + c * c).sqrt().max(1e-3);
                planes[p * 4] = a / len;
                planes[p * 4 + 1] = b / len;
                planes[p * 4 + 2] = c / len;
                planes[p * 4 + 3] = (unit() - 0.5) * 500.0;
            }
            let aabb = [
                unit() * 30.0,
                unit() * 30.0,
                unit() * 30.0,
                -unit() * 30.0,
                -unit() * 30.0,
                -unit() * 30.0,
            ];
            let mut o = [0.0f32; 9];
            for cell in &mut o {
                *cell = (unit() - 0.5) * 2.0;
            }
            let t = [
                (unit() - 0.5) * 500.0,
                (unit() - 0.5) * 500.0,
                (unit() - 0.5) * 500.0,
            ];
            check(&planes, &aabb, &o, &t, "random sweep");
        }
    }
}

/// Tests a bounding sphere against six frustum planes; returns 3 if the sphere
/// may be visible, 0 if any plane fully rejects it.
///
/// `planes` is 6 planes of 4 floats `[a, b, c, d]` (stride 4). `sphere` is
/// `[x, y, z, radius]`. A plane rejects when the signed distance of the centre
/// is below the negated radius (`a*x + b*y + c*z + d < -radius`).
pub fn c_world_frustum__test_sphere__686b80(planes: &[f32; 24], sphere: &[f32; 4]) -> u32 {
    let x = sphere[0];
    let y = sphere[1];
    let z = sphere[2];
    let neg_r = -sphere[3];

    for p in 0..6 {
        let base = p * 4;
        let dist =
            planes[base] * x + planes[base + 1] * y + planes[base + 2] * z + planes[base + 3];
        if dist < neg_r {
            return 0;
        }
    }
    3
}

#[cfg(test)]
mod tests_c_world_frustum__test_sphere__686b80 {
    use super::c_world_frustum__test_sphere__686b80;

    fn box_frustum() -> [f32; 24] {
        [
            -1.0, 0.0, 0.0, 10.0, // x <= 10
            1.0, 0.0, 0.0, 10.0, // x >= -10
            0.0, -1.0, 0.0, 10.0, // y <= 10
            0.0, 1.0, 0.0, 10.0, // y >= -10
            0.0, 0.0, -1.0, 10.0, // z <= 10
            0.0, 0.0, 1.0, 10.0, // z >= -10
        ]
    }

    #[test]
    fn sphere_inside_returns_three() {
        let r = c_world_frustum__test_sphere__686b80(&box_frustum(), &[0.0, 0.0, 0.0, 1.0]);
        assert_eq!(r, 3);
    }

    #[test]
    fn sphere_far_outside_returns_zero() {
        let r = c_world_frustum__test_sphere__686b80(&box_frustum(), &[100.0, 0.0, 0.0, 1.0]);
        assert_eq!(r, 0);
    }

    #[test]
    fn sphere_partially_overlapping_visible() {
        // centre exactly on x=10 plane, dist there = 0 >= -radius -> visible
        let r = c_world_frustum__test_sphere__686b80(&box_frustum(), &[10.0, 0.0, 0.0, 1.0]);
        assert_eq!(r, 3);
    }

    #[test]
    fn sphere_just_touching_visible_then_rejected() {
        // centre at x = 10 + r: dist = -r, equals -radius, NOT < -radius -> visible
        let r1 = c_world_frustum__test_sphere__686b80(&box_frustum(), &[11.0, 0.0, 0.0, 1.0]);
        assert_eq!(r1, 3);
        let r2 = c_world_frustum__test_sphere__686b80(&box_frustum(), &[11.001, 0.0, 0.0, 1.0]);
        assert_eq!(r2, 0);
    }

    #[test]
    fn larger_radius_more_lenient() {
        let pos_small = [12.0f32, 0.0, 0.0, 1.0];
        let pos_big = [12.0f32, 0.0, 0.0, 5.0];
        assert_eq!(
            c_world_frustum__test_sphere__686b80(&box_frustum(), &pos_small),
            0
        );
        assert_eq!(
            c_world_frustum__test_sphere__686b80(&box_frustum(), &pos_big),
            3
        );
    }
}

/// `Frustum::CullBox` — test an axis-aligned box against a 6-plane frustum.
///
/// `planes` is six planes laid out `[nx, ny, nz, d]` each (stride 4 floats);
/// `box_min_max` is the min corner (indices 0..3) followed by the max corner
/// (indices 3..6). For every plane the *positive vertex* (the box corner
/// farthest along the plane normal) is selected component-wise — the max corner
/// where a normal component's sign bit is clear, the min corner where it is set
/// (so `-0.0` selects the min, matching the original's `bits >> 31` pick). The
/// signed distance of that vertex to the plane is `n·p + d`; if it is below
/// `threshold` for *any* plane the whole box is outside that half-space and the
/// box is culled (returns 0). If no plane culls, the box is inside or
/// intersecting and the function returns 3.
pub fn frustum__cull_box__686940(
    planes: &[f32; 24],
    box_min_max: &[f32; 6],
    threshold: f32,
) -> u32 {
    let min = [box_min_max[0], box_min_max[1], box_min_max[2]];
    let max = [box_min_max[3], box_min_max[4], box_min_max[5]];
    for p in 0..6 {
        let nx = planes[p * 4];
        let ny = planes[p * 4 + 1];
        let nz = planes[p * 4 + 2];
        let d = planes[p * 4 + 3];
        // Sign bit of each normal component selects the positive vertex: clear
        // (>= +0.0) -> max corner, set (< 0, incl. -0.0) -> min corner.
        let px = if nx.to_bits() >> 31 == 0 {
            max[0]
        } else {
            min[0]
        };
        let py = if ny.to_bits() >> 31 == 0 {
            max[1]
        } else {
            min[1]
        };
        let pz = if nz.to_bits() >> 31 == 0 {
            max[2]
        } else {
            min[2]
        };
        let dist = px * nx + py * ny + pz * nz + d;
        if dist < threshold {
            return 0;
        }
    }
    3
}

#[cfg(test)]
mod tests_frustum__cull_box__686940 {
    use super::frustum__cull_box__686940 as cull;

    // A symmetric unit-cube frustum centered on the origin: the six axis-aligned
    // planes x>=-1, x<=1, y>=-1, y<=1, z>=-1, z<=1 expressed as inward-normal
    // planes `n·p + d >= 0`. For the `+axis` faces the inward normal points in
    // `-axis` with `d = 1`; for the `-axis` faces it points in `+axis`, `d = 1`.
    fn unit_box_frustum() -> [f32; 24] {
        [
            1.0, 0.0, 0.0, 1.0, // +X face: x >= -1  -> (1,0,0)·p + 1 >= 0
            -1.0, 0.0, 0.0, 1.0, // -X face: x <=  1  -> (-1,0,0)·p + 1 >= 0
            0.0, 1.0, 0.0, 1.0, // +Y
            0.0, -1.0, 0.0, 1.0, // -Y
            0.0, 0.0, 1.0, 1.0, // +Z
            0.0, 0.0, -1.0, 1.0, // -Z
        ]
    }

    fn aabb(cx: f32, cy: f32, cz: f32, h: f32) -> [f32; 6] {
        [cx - h, cy - h, cz - h, cx + h, cy + h, cz + h]
    }

    #[test]
    fn inside_box_returns_3() {
        let f = unit_box_frustum();
        let b = aabb(0.0, 0.0, 0.0, 0.25);
        assert_eq!(cull(&f, &b, 0.0), 3);
    }

    #[test]
    fn intersecting_box_returns_3() {
        // Straddles the +X plane (extends from 0.5 to 1.5): not fully outside any
        // plane, so it is kept.
        let f = unit_box_frustum();
        let b = aabb(1.0, 0.0, 0.0, 0.5);
        assert_eq!(cull(&f, &b, 0.0), 3);
    }

    #[test]
    fn fully_outside_plus_x_returns_0() {
        // Centered at x = 3, half-extent 0.5 -> min x = 2.5 > 1: the positive
        // vertex along the -X face normal (min corner) still has -X·p + 1 < 0.
        let f = unit_box_frustum();
        let b = aabb(3.0, 0.0, 0.0, 0.5);
        assert_eq!(cull(&f, &b, 0.0), 0);
    }

    #[test]
    fn fully_outside_minus_y_returns_0() {
        let f = unit_box_frustum();
        let b = aabb(0.0, -3.0, 0.0, 0.5);
        assert_eq!(cull(&f, &b, 0.0), 0);
    }

    #[test]
    fn fully_outside_diagonal_returns_0() {
        let f = unit_box_frustum();
        let b = aabb(5.0, 5.0, 5.0, 0.5);
        assert_eq!(cull(&f, &b, 0.0), 0);
    }

    // Metamorphic: a box strictly inside stays inside as it shrinks (monotone in
    // extent).
    #[test]
    fn shrinking_inside_box_stays_inside() {
        let f = unit_box_frustum();
        for &h in &[0.5f32, 0.25, 0.1, 0.01] {
            let b = aabb(0.0, 0.0, 0.0, h);
            assert_eq!(cull(&f, &b, 0.0), 3, "h={h}");
        }
    }

    // Translating an inside box far enough along one axis eventually culls it;
    // the transition is monotone (once culled, farther stays culled).
    #[test]
    fn translation_monotone_cull() {
        let f = unit_box_frustum();
        let mut seen_cull = false;
        for i in 0..20 {
            let cx = 0.1 * i as f32; // 0.0 .. 1.9
            let b = aabb(cx, 0.0, 0.0, 0.2);
            let r = cull(&f, &b, 0.0);
            if seen_cull {
                assert_eq!(r, 0, "cx={cx} should stay culled");
            }
            if r == 0 {
                seen_cull = true;
            }
        }
        assert!(seen_cull, "far translation must eventually cull");
    }

    // The positive-vertex pick must honor the *sign bit*, so a negative-zero
    // normal component reads the min corner, not the max (and must not panic).
    #[test]
    fn negative_zero_normal_picks_min() {
        let mut planes = [0.0f32; 24];
        // plane 0: (-0.0, 0, 1, 0). The Z term decides; the X term is 0 either
        // way. planes 1..6 stay (0,0,0,0): dist 0 >= threshold(-1) -> no cull.
        planes[0] = -0.0;
        planes[2] = 1.0;
        let b = [0.0f32, 0.0, 1.0, 0.0, 0.0, 2.0]; // z in [1, 2]
        // z normal positive -> picks max z = 2 -> dist 2 >= -1: inside.
        assert_eq!(cull(&planes, &b, -1.0), 3);
    }

    // Known-value: a single active plane, exact arithmetic.
    #[test]
    fn known_value_single_plane() {
        let mut planes = [0.0f32; 24];
        // plane 0: normal (1,0,0), d = -2 -> inside iff max_x - 2 >= threshold.
        planes[0] = 1.0;
        planes[3] = -2.0;
        // remaining planes (0,0,0,0): dist 0; with threshold 0 they pass (>=).
        let b_keep = [0.0f32, 0.0, 0.0, 3.0, 0.0, 0.0]; // max_x 3 -> 3-2=1 >= 0
        assert_eq!(cull(&planes, &b_keep, 0.0), 3);
        let b_cull = [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0]; // max_x 1 -> 1-2=-1 < 0
        assert_eq!(cull(&planes, &b_cull, 0.0), 0);
    }
}

/// Per-component minimum of two 3-vectors `a` and `b`, written into the result.
///
/// Component 0 keeps `a` only when `a < b` (strict), otherwise `b`; components 1
/// and 2 keep `a` only when `b > a` (strict), otherwise `b`. Each lane therefore
/// resolves to the smaller value, with `b` taken on ties and whenever a compare
/// is unordered. The asymmetric predicates mirror the original's two distinct
/// x87 compare idioms (`FCOMPP` with the `C0|C3` mask for the upper two lanes
/// versus a `FCOMP` with the `C0|C2` parity test for the first lane), so a NaN
/// in either input resolves to `b` on every lane exactly as the reference does.
pub fn frustum_compute_c3_vector__699250(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    let x = if a[0] < b[0] { a[0] } else { b[0] };
    let y = if b[1] > a[1] { a[1] } else { b[1] };
    let z = if b[2] > a[2] { a[2] } else { b[2] };
    [x, y, z]
}

#[cfg(test)]
mod tests_frustum_compute_c3_vector__699250 {
    use super::frustum_compute_c3_vector__699250 as cmin;

    #[test]
    fn known_value_componentwise_min() {
        // a smaller on x, b smaller on y and z.
        let a = [1.0f32, 9.0, 7.0];
        let b = [4.0f32, 2.0, 3.0];
        assert_eq!(cmin(&a, &b), [1.0, 2.0, 3.0]);
    }

    #[test]
    fn known_value_other_direction() {
        let a = [8.0f32, 1.0, 2.0];
        let b = [3.0f32, 5.0, 6.0];
        assert_eq!(cmin(&a, &b), [3.0, 1.0, 2.0]);
    }

    // The result is the per-component minimum on every lane (verified against an
    // independent oracle over a multi-axis grid that exercises all sign/order
    // combinations, not just a single axis).
    #[test]
    fn matches_min_oracle_multi_axis() {
        let samples = [
            [-3.0f32, 4.5, -7.25],
            [0.0, -0.0, 100.0],
            [12.5, -12.5, 0.001],
            [-100.0, 0.5, -0.5],
            [2.0, 2.0, 2.0],
        ];
        for a in samples {
            for b in samples {
                let r = cmin(&a, &b);
                for i in 0..3 {
                    let want = if a[i] <= b[i] { a[i] } else { b[i] };
                    assert_eq!(r[i], want, "lane {i}: a={a:?} b={b:?}");
                }
            }
        }
    }

    // Idempotence: min(v, v) == v on every lane (also exercises the tie path on
    // all three lanes simultaneously).
    #[test]
    fn idempotent_on_equal_inputs() {
        for v in [[1.5f32, -2.0, 9.0], [-4.0, 4.0, 0.0], [0.25, 0.25, 0.25]] {
            assert_eq!(cmin(&v, &v), v);
        }
    }

    // The minimum value on each lane is independent of argument order, even
    // though the tie-break source differs.
    #[test]
    fn value_is_order_independent() {
        let a = [1.0f32, -3.0, 8.0];
        let b = [-2.0f32, 5.0, 8.0];
        let ab = cmin(&a, &b);
        let ba = cmin(&b, &a);
        assert_eq!(ab[0], ba[0]);
        assert_eq!(ab[1], ba[1]);
        assert_eq!(ab[2], ba[2]);
    }

    // Boundary equality on a single lane: exactly-equal components resolve to the
    // tie-break source for that lane and leave the other lanes as their min.
    #[test]
    fn boundary_equal_lane() {
        let a = [5.0f32, 5.0, 1.0];
        let b = [5.0f32, 2.0, 5.0];
        assert_eq!(cmin(&a, &b), [5.0, 2.0, 1.0]);
    }

    // A NaN in either input resolves to `b` on the affected lane (matching the
    // x87 unordered branch), without panicking.
    #[test]
    fn nan_resolves_to_b() {
        let nan = f32::NAN;
        let a = [nan, nan, nan];
        let b = [1.0f32, 2.0, 3.0];
        assert_eq!(cmin(&a, &b), [1.0, 2.0, 3.0]);
    }
}

/// The `|w|` underflow replacement for the portal perspective divide: the raw
/// dword `0x3727c5ac` (~1.0000000e-5) MOV'd over the clipped vertex's `w` when
/// `|w| < DAT_00801360` (0x6b49bf / 0x6b49d3). Note it is NOT the usual
/// `0x801360` epsilon itself and it is unsigned — a small negative `w` is
/// replaced by the positive constant, flipping the vertex across the origin
/// exactly as stock does.
const PORTAL_W_REPLACEMENT: f32 = f32::from_bits(0x3727_c5ac);

/// Portal plane normal from the portal's first three world verts
/// (`CMapObj::ComputePortalNdcRect` 0x6b4809..0x6b48c7): `n = e1 × e2` with
/// `e1 = v1 - v0`, `e2 = v2 - v0`, then normalized.
///
/// x87 fidelity notes:
/// - `e1.xyz` and `e2.z` stay on the x87 stack (f64 here); `e2.x`/`e2.y` are
///   spilled to f32 slots before use (FSTP @0x6b4829/0x6b4832) — the narrowing
///   is reproduced exactly.
/// - Component product order: `nx = e2z*e1y - e1z*e2y`, `ny = e1z*e2x - e2z*e1x`,
///   `nz = e2y*e1x - e1y*e2x` (each FSUBP's ST1 - ST0), narrowed at each FSTP.
/// - `len² = (nz² + ny²) + nx²` (0x6b4868..0x6b487c) over the reloaded f32
///   components.
/// - `use_rsqrt` mirrors the `DAT_00c9e388` SSE toggle: non-zero spills `len²`
///   to f32 and takes raw `RSQRTSS` (12-bit approximation, no refinement,
///   0x6b4880..0x6b48a5); zero takes `FSQRT` + `FDIVR one` (0x6b48aa..0x6b48c7,
///   `one` is the 1.0 constant at 0x7ff9d8, read live by the adapter).
/// - Degenerate (collinear) verts give `len² = 0` ⇒ `inv = +inf` ⇒ the zero
///   components normalize to NaN (`0 * inf`), exactly as stock.
pub fn c_map_obj__portal_plane_normal__6b46f0(
    v0: &[f32; 3],
    v1: &[f32; 3],
    v2: &[f32; 3],
    use_rsqrt: bool,
    one: f32,
) -> [f32; 3] {
    let e1x = f64::from(v1[0]) - f64::from(v0[0]);
    let e1y = f64::from(v1[1]) - f64::from(v0[1]);
    let e1z = f64::from(v1[2]) - f64::from(v0[2]);
    let e2x = f64::from(super::f64_to_f32(f64::from(v2[0]) - f64::from(v0[0])));
    let e2y = f64::from(super::f64_to_f32(f64::from(v2[1]) - f64::from(v0[1])));
    let e2z = f64::from(v2[2]) - f64::from(v0[2]);

    let nx = super::f64_to_f32(e2z * e1y - e1z * e2y);
    let ny = super::f64_to_f32(e1z * e2x - e2z * e1x);
    let nz = super::f64_to_f32(e2y * e1x - e1y * e2x);

    let len2 = (f64::from(nz) * f64::from(nz) + f64::from(ny) * f64::from(ny))
        + f64::from(nx) * f64::from(nx);

    let inv = if use_rsqrt {
        f64::from(super::world::rsqrt_ss__6aadc0(super::f64_to_f32(len2)))
    } else {
        f64::from(one) / len2.sqrt()
    };
    [
        super::f64_to_f32(f64::from(nx) * inv),
        super::f64_to_f32(f64::from(ny) * inv),
        super::f64_to_f32(f64::from(nz) * inv),
    ]
}

/// Camera-on-portal-plane test (0x6b48cc..0x6b4930): plane
/// `d = -((nnx*v0.x + nnz*v0.z) + nny*v0.y)` (FCHS then FSTP f32 @0x6b48fe),
/// then `dist = |((nnz*cam.z + nny*cam.y) + nnx*cam.x) + d|` kept on the x87
/// stack (never narrowed) and FCOMP'd against `eps` (`_DAT_008029d0`).
///
/// NaN polarity (0x6b4925 `TEST AH,0x41; JP`): the full-screen-force arm is
/// entered only on the ORDERED `dist <= eps` (C0-only or C3-only ⇒ odd parity ⇒
/// JP not taken); an unordered compare sets C0|C3 (even parity) and takes the
/// JP to the normal min/max path — i.e. NaN means "camera not on plane". Rust
/// `<=` is false on NaN, encoding exactly that.
pub fn c_map_obj__portal_camera_on_plane__6b46f0(
    nn: &[f32; 3],
    v0: &[f32; 3],
    cam: &[f32; 3],
    eps: f32,
) -> bool {
    let d = super::f64_to_f32(
        -((f64::from(nn[0]) * f64::from(v0[0]) + f64::from(nn[2]) * f64::from(v0[2]))
            + f64::from(nn[1]) * f64::from(v0[1])),
    );
    let dist = (((f64::from(nn[2]) * f64::from(cam[2]) + f64::from(nn[1]) * f64::from(cam[1]))
        + f64::from(nn[0]) * f64::from(cam[0]))
        + f64::from(d))
    .abs();
    dist <= f64::from(eps)
}

/// `C3Vector::DominantAxis` (0x7bf680, trivial leaf reimplemented inline):
/// index (0 = x, 1 = y, 2 = z) of the component with the largest |value|.
///
/// Each `FCOM`/`TEST AH,0x41` pair falls through only on the ORDERED strict
/// `>`; equal or unordered (NaN) takes the "not greater" branch — a NaN lane
/// therefore never wins over a later candidate. Rust `>` (false on NaN/equal)
/// encodes both branches.
pub fn c3_vector__dominant_axis__7bf680(v: &[f32; 3]) -> u32 {
    let ax = v[0].abs();
    let ay = v[1].abs();
    let az = v[2].abs();
    if ax > ay {
        if ax > az { 0 } else { 2 }
    } else if ay > az {
        1
    } else {
        2
    }
}

/// One iteration of the portal perspective-divide + NDC-rect min/max loop
/// (0x6b49c0..0x6b4a4a). `vert = [x, y, w]` is one clipped homogeneous vertex;
/// `rect = [minY, minX, maxY, maxX]` mirrors `outEntry+4/+8/+0xc/+0x10`.
/// Returns the divided `[x', y', w']` the adapter writes back into the
/// clipper's static buffer, exactly as stock mutates it in place.
///
/// - `w` clamp (`FABS; FCOMP DAT_00801360; TEST AH,0x5; JP`): the replacement
///   store happens only on the ORDERED strict `|w| < w_eps`; a NaN `w` skips
///   the clamp and propagates NaN through the divide.
/// - `inv = one / w` stays on the x87 stack (f64 here, `one` = the 1.0 at
///   0x7ff9d8); each product narrows to f32 at its FSTP.
/// - min updates use `TEST AH,0x5` (ordered `<`), max updates `TEST AH,0x41`
///   (ordered `>`), in the stock order minX, maxX, minY, maxY — a NaN lane
///   never updates the rect.
pub fn c_map_obj__portal_ndc_accumulate__6b46f0(
    vert: &[f32; 3],
    rect: &mut [f32; 4],
    w_eps: f32,
    one: f32,
) -> [f32; 3] {
    let mut w = vert[2];
    if w.abs() < w_eps {
        w = PORTAL_W_REPLACEMENT;
    }
    let inv = f64::from(one) / f64::from(w);
    let x = super::f64_to_f32(f64::from(vert[0]) * inv);
    let y = super::f64_to_f32(f64::from(vert[1]) * inv);
    if x < rect[1] {
        rect[1] = x;
    }
    if x > rect[3] {
        rect[3] = x;
    }
    if y < rect[0] {
        rect[0] = y;
    }
    if y > rect[2] {
        rect[2] = y;
    }
    [x, y, w]
}

#[cfg(test)]
mod tests_c_map_obj__compute_portal_ndc_rect__6b46f0 {
    use super::{
        PORTAL_W_REPLACEMENT, c_map_obj__portal_camera_on_plane__6b46f0,
        c_map_obj__portal_ndc_accumulate__6b46f0, c_map_obj__portal_plane_normal__6b46f0,
        c3_vector__dominant_axis__7bf680,
    };

    /// Stock rect init bit patterns: min lanes +FLT_MAX, max lanes -FLT_MAX.
    fn fresh_rect() -> [f32; 4] {
        [
            f32::from_bits(0x7f7f_ffff),
            f32::from_bits(0x7f7f_ffff),
            f32::from_bits(0xff7f_ffff),
            f32::from_bits(0xff7f_ffff),
        ]
    }

    // Happy path vs an independent f64 oracle: a visible clipped polygon's rect
    // is the min/max of x/w, y/w over all vertices.
    #[test]
    fn visible_portal_rect_matches_oracle() {
        let verts: [[f32; 3]; 4] = [
            [2.0, 1.0, 2.0],
            [-3.0, 0.5, 4.0],
            [0.5, -1.5, 1.0],
            [1.0, 2.5, 5.0],
        ];
        let mut rect = fresh_rect();
        for v in &verts {
            let back = c_map_obj__portal_ndc_accumulate__6b46f0(v, &mut rect, 0.001, 1.0);
            // Unclamped w is passed back unchanged.
            assert_eq!(back[2].to_bits(), v[2].to_bits());
        }
        let (mut min_x, mut max_x) = (f64::MAX, f64::MIN);
        let (mut min_y, mut max_y) = (f64::MAX, f64::MIN);
        for v in &verts {
            let x = f64::from(v[0]) / f64::from(v[2]);
            let y = f64::from(v[1]) / f64::from(v[2]);
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
        assert!((f64::from(rect[1]) - min_x).abs() < 1e-6);
        assert!((f64::from(rect[3]) - max_x).abs() < 1e-6);
        assert!((f64::from(rect[0]) - min_y).abs() < 1e-6);
        assert!((f64::from(rect[2]) - max_y).abs() < 1e-6);
    }

    // w-clamp boundary: strictly below the epsilon replaces w with the raw
    // 0x3727c5ac constant (even for negative w — the store is unsigned);
    // exactly at the epsilon there is no clamp (ordered strict `<`).
    #[test]
    fn w_clamp_boundary() {
        let w_eps = 0.001f32;
        let mut rect = fresh_rect();

        let below = [1.0f32, 1.0, 0.000_999];
        let out = c_map_obj__portal_ndc_accumulate__6b46f0(&below, &mut rect, w_eps, 1.0);
        assert_eq!(out[2].to_bits(), PORTAL_W_REPLACEMENT.to_bits());
        assert_eq!(
            out[0],
            super::super::f64_to_f32(1.0 / f64::from(PORTAL_W_REPLACEMENT))
        );

        // Negative small w is replaced by the POSITIVE constant (stock quirk).
        let mut rect2 = fresh_rect();
        let neg = [1.0f32, 1.0, -0.000_5];
        let out2 = c_map_obj__portal_ndc_accumulate__6b46f0(&neg, &mut rect2, w_eps, 1.0);
        assert_eq!(out2[2].to_bits(), PORTAL_W_REPLACEMENT.to_bits());
        assert!(out2[0] > 0.0);

        // |w| == w_eps exactly: no clamp.
        let mut rect3 = fresh_rect();
        let at = [1.0f32, 1.0, 0.001];
        let out3 = c_map_obj__portal_ndc_accumulate__6b46f0(&at, &mut rect3, w_eps, 1.0);
        assert_eq!(out3[2].to_bits(), 0.001f32.to_bits());
    }

    // A NaN w skips the clamp, produces NaN x/y, and never updates the rect
    // (ordered-only min/max compares).
    #[test]
    fn nan_w_never_updates_rect() {
        let mut rect = fresh_rect();
        let init = rect;
        let v = [1.0f32, 2.0, f32::NAN];
        let out = c_map_obj__portal_ndc_accumulate__6b46f0(&v, &mut rect, 0.001, 1.0);
        assert!(out[0].is_nan() && out[1].is_nan() && out[2].is_nan());
        assert_eq!(rect.map(f32::to_bits), init.map(f32::to_bits));
    }

    // Camera-on-plane epsilon boundary: the force arm is entered on the ordered
    // `dist <= eps` — strictly below AND exactly equal both force; just above
    // does not.
    #[test]
    fn camera_on_plane_force_boundary() {
        let nn = [1.0f32, 0.0, 0.0];
        let v0 = [0.0f32, 0.0, 0.0];
        let eps = 0.01f32;
        assert!(c_map_obj__portal_camera_on_plane__6b46f0(
            &nn,
            &v0,
            &[0.005, 3.0, -7.0],
            eps
        ));
        assert!(c_map_obj__portal_camera_on_plane__6b46f0(
            &nn,
            &v0,
            &[0.01, 3.0, -7.0],
            eps
        ));
        assert!(!c_map_obj__portal_camera_on_plane__6b46f0(
            &nn,
            &v0,
            &[0.011, 3.0, -7.0],
            eps
        ));
    }

    // NaN plane distance (e.g. the NaN normal a degenerate portal produces)
    // routes to "camera NOT on plane": the force arm is skipped.
    #[test]
    fn nan_plane_distance_skips_force() {
        let nn = [f32::NAN, f32::NAN, f32::NAN];
        assert!(!c_map_obj__portal_camera_on_plane__6b46f0(
            &nn,
            &[0.0, 0.0, 0.0],
            &[0.0, 0.0, 0.0],
            0.01
        ));
    }

    // Dominant-axis selection incl. tie and NaN polarity: ties and NaN lanes
    // lose the ordered strict `>` and fall toward the later axis.
    #[test]
    fn dominant_axis_selection() {
        assert_eq!(c3_vector__dominant_axis__7bf680(&[3.0, -2.0, 1.0]), 0);
        assert_eq!(c3_vector__dominant_axis__7bf680(&[1.0, -5.0, 2.0]), 1);
        assert_eq!(c3_vector__dominant_axis__7bf680(&[1.0, 2.0, -3.0]), 2);
        // |x| == |y|: x is not strictly greater, y wins over a smaller z.
        assert_eq!(c3_vector__dominant_axis__7bf680(&[2.0, -2.0, 1.0]), 1);
        // All equal: falls through to z.
        assert_eq!(c3_vector__dominant_axis__7bf680(&[2.0, 2.0, 2.0]), 2);
        // NaN x never compares greater; y still beats a smaller z.
        assert_eq!(c3_vector__dominant_axis__7bf680(&[f32::NAN, 2.0, 1.0]), 1);
        assert_eq!(c3_vector__dominant_axis__7bf680(&[f32::NAN, 1.0, 2.0]), 2);
    }

    // Plane normal, x87 path (selector == 0): unit length and direction match
    // an f64 cross-product oracle.
    #[test]
    fn plane_normal_sqrt_path_matches_oracle() {
        let v0 = [1.0f32, 2.0, 3.0];
        let v1 = [4.0f32, 2.5, 3.0];
        let v2 = [1.5f32, 6.0, 3.5];
        let nn = c_map_obj__portal_plane_normal__6b46f0(&v0, &v1, &v2, false, 1.0);
        let e1 = [3.0f64, 0.5, 0.0];
        let e2 = [0.5f64, 4.0, 0.5];
        let cx = e1[1] * e2[2] - e1[2] * e2[1];
        let cy = e1[2] * e2[0] - e1[0] * e2[2];
        let cz = e1[0] * e2[1] - e1[1] * e2[0];
        let len = (cx * cx + cy * cy + cz * cz).sqrt();
        assert!((f64::from(nn[0]) - cx / len).abs() < 1e-6);
        assert!((f64::from(nn[1]) - cy / len).abs() < 1e-6);
        assert!((f64::from(nn[2]) - cz / len).abs() < 1e-6);
    }

    // Plane normal, rsqrtss path (selector != 0): the 12-bit SSE estimate —
    // unit length only within the rsqrt tolerance (and NOT bit-equal to the
    // x87 1/sqrt path), but pointing the same direction.
    #[test]
    fn plane_normal_rsqrt_path_is_the_sse_estimate() {
        let v0 = [1.0f32, 2.0, 3.0];
        let v1 = [4.0f32, 2.5, 3.0];
        let v2 = [1.5f32, 6.0, 3.5];
        let fast = c_map_obj__portal_plane_normal__6b46f0(&v0, &v1, &v2, true, 1.0);
        let slow = c_map_obj__portal_plane_normal__6b46f0(&v0, &v1, &v2, false, 1.0);
        let len2 = fast
            .iter()
            .map(|c| f64::from(*c) * f64::from(*c))
            .sum::<f64>();
        assert!((len2.sqrt() - 1.0).abs() < 1e-3);
        let dot = fast
            .iter()
            .zip(slow.iter())
            .map(|(a, b)| f64::from(*a) * f64::from(*b))
            .sum::<f64>();
        assert!(dot > 0.999);
    }

    // Degenerate (collinear) portal verts: len² = 0, rsqrt/1÷sqrt = +inf, and
    // the zero components normalize to NaN (0 × inf) on both paths — the NaN
    // that then skips the full-screen force.
    #[test]
    fn degenerate_portal_normal_is_nan() {
        let v0 = [0.0f32, 0.0, 0.0];
        let v1 = [1.0f32, 0.0, 0.0];
        let v2 = [2.0f32, 0.0, 0.0];
        for use_rsqrt in [false, true] {
            let nn = c_map_obj__portal_plane_normal__6b46f0(&v0, &v1, &v2, use_rsqrt, 1.0);
            assert!(nn[0].is_nan() && nn[1].is_nan() && nn[2].is_nan());
        }
    }
}

/// Unit-box `[0,1]³` vertex outcode for the 24-plane polygon clipper
/// (`World_ClipPolygonToFrustum24` @0x686db0) and
/// `CMovement__ClipVerticalStepAgainstHull`.
///
/// Input/output are raw f32 BIT PATTERNS: the stock body copies the x/y/z
/// lanes as untouched dwords (NaN payloads and `-0.0` pass through verbatim)
/// and stores `1.0 - v` for the far lanes with a single x87 rounding — which
/// `1.0f32 - x` reproduces exactly (the extended-precision intermediate of a
/// two-f32 subtract is exact, so both round the true result once). The
/// outcode packs each lane's raw SIGN BIT (so `-0.0` counts as negative —
/// float compares would get that wrong) into bits 31..26 in lane order
/// x, 1−x, y, 1−y, z, 1−z; the stock ADDs are equivalent to OR (disjoint
/// bits). Returns the 7-dword out region `{6 lanes, outcode}`; the outcode
/// dword is also the stock function's EAX at return.
pub fn world_compute_vertex_outcode__686d20(v_bits: &[u32; 3]) -> [u32; 7] {
    let far = |b: u32| (1.0f32 - f32::from_bits(b)).to_bits();
    let d = [
        v_bits[0],
        far(v_bits[0]),
        v_bits[1],
        far(v_bits[1]),
        v_bits[2],
        far(v_bits[2]),
    ];
    let code = (d[0] & 0x8000_0000)
        + ((d[1] >> 1) & 0x4000_0000)
        + ((d[2] >> 2) & 0x2000_0000)
        + ((d[3] >> 3) & 0x1000_0000)
        + ((d[4] >> 4) & 0x0800_0000)
        + ((d[5] >> 5) & 0x0400_0000);
    [d[0], d[1], d[2], d[3], d[4], d[5], code]
}

#[cfg(test)]
mod tests_world_compute_vertex_outcode__686d20 {
    use super::world_compute_vertex_outcode__686d20 as f;

    fn bits(v: [f32; 3]) -> [u32; 3] {
        [v[0].to_bits(), v[1].to_bits(), v[2].to_bits()]
    }

    /// A point strictly inside the unit box: all six distances positive,
    /// outcode 0.
    #[test]
    fn inside_point_has_zero_outcode() {
        let out = f(&bits([0.25, 0.5, 0.75]));
        assert_eq!(f32::from_bits(out[0]), 0.25);
        assert_eq!(f32::from_bits(out[1]), 0.75);
        assert_eq!(f32::from_bits(out[2]), 0.5);
        assert_eq!(f32::from_bits(out[3]), 0.5);
        assert_eq!(f32::from_bits(out[4]), 0.75);
        assert_eq!(f32::from_bits(out[5]), 0.25);
        assert_eq!(out[6], 0);
    }

    /// Each violated slab sets exactly its documented bit, in lane order
    /// x→31, 1−x→30, y→29, 1−y→28, z→27, 1−z→26.
    #[test]
    fn per_slab_bits() {
        assert_eq!(f(&bits([-0.5, 0.5, 0.5]))[6], 0x8000_0000);
        assert_eq!(f(&bits([1.5, 0.5, 0.5]))[6], 0x4000_0000);
        assert_eq!(f(&bits([0.5, -0.5, 0.5]))[6], 0x2000_0000);
        assert_eq!(f(&bits([0.5, 1.5, 0.5]))[6], 0x1000_0000);
        assert_eq!(f(&bits([0.5, 0.5, -0.5]))[6], 0x0800_0000);
        assert_eq!(f(&bits([0.5, 0.5, 1.5]))[6], 0x0400_0000);
        // All-outside corner: every low-side bit plus no far bits can't
        // happen geometrically, but a huge negative point sets the three
        // near bits only.
        assert_eq!(
            f(&bits([-9.0, -9.0, -9.0]))[6],
            0x8000_0000 | 0x2000_0000 | 0x0800_0000
        );
    }

    /// `-0.0` input: the raw sign bit counts as "outside" for the near slab
    /// (bit 31) even though `-0.0 < 0.0` is false — this is why the pack must
    /// read `to_bits()`, not compare floats. The far lane `1 - (-0.0) = 1.0`
    /// stays positive.
    #[test]
    fn negative_zero_counts_as_negative() {
        let out = f(&bits([-0.0, 0.5, 0.5]));
        assert_eq!(out[0], (-0.0f32).to_bits());
        assert_eq!(out[6], 0x8000_0000);
    }

    /// Boundary values: exactly 0.0 and 1.0 have positive/zero lanes on both
    /// sides (`+0.0` sign bit clear) — no bits set.
    #[test]
    fn boundaries_are_inside() {
        assert_eq!(f(&bits([0.0, 1.0, 0.0]))[6], 0);
    }

    /// NaN passes through the copy lanes bit-exact (payload preserved) and
    /// propagates through the `1 - v` lanes; the outcode then reflects the
    /// NaN's raw sign bit per lane, matching the stock integer pack.
    #[test]
    fn nan_payload_and_sign_flow_through() {
        let qnan_neg = f32::from_bits(0xFFC0_1234);
        let out = f(&bits([qnan_neg, 0.5, 0.5]));
        assert_eq!(out[0], 0xFFC0_1234, "copy lane must preserve the payload");
        assert!(f32::from_bits(out[1]).is_nan());
        // Near-slab bit from the raw sign; far-slab bit from the sign of
        // `1 - NaN` (a quiet NaN whose sign follows the propagated operand).
        assert_eq!(out[6] & 0x8000_0000, 0x8000_0000);
    }

    /// The `1 - v` lanes round exactly once (subnormal-adjacent and
    /// sub-ULP inputs match the reference computation bit-for-bit).
    #[test]
    fn far_lane_single_rounding() {
        for &x in &[1e-8f32, 0.333_333_34, 0.999_999_94, 1.000_000_1, 123.456] {
            let out = f(&bits([x, 0.5, 0.5]));
            assert_eq!(out[1], (1.0 - x).to_bits(), "x={x}");
        }
    }
}
