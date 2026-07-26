//! `plane` family kernels.
// Adapter and kernel names mirror the host's C++ symbols verbatim, with `__`
// standing in for the `::`, so the whole module is non-snake-case by construction.
#![allow(non_snake_case)]

/// Builds a `C4Plane` (`[Nx, Ny, Nz, D]`) from a triangle's three corners.
///
/// `edge1 = p2 - p0`, `edge2 = p1 - p0`; the unnormalized normal is the cross
/// product `edge2 × edge1`. The normal is scaled by `1.0 / |n|` (the reference
/// reciprocal constant is `1.0`) and `D = -dot(normal, p0)`, so the plane
/// equation `dot(N, x) + D = 0` holds for every point `x` on the triangle. A
/// degenerate (collinear) triangle yields a zero-length normal and propagates
/// `inf`/`NaN` exactly as the original does.
pub fn c4_plane__from_triangle__637480(p0: &[f32; 3], p1: &[f32; 3], p2: &[f32; 3]) -> [f32; 4] {
    // edge1 = p2 - p0, edge2 = p1 - p0 (per the reference annotation).
    let e1x = p2[0] - p0[0];
    let e1y = p2[1] - p0[1];
    let e1z = p2[2] - p0[2];
    let e2x = p1[0] - p0[0];
    let e2y = p1[1] - p0[1];
    let e2z = p1[2] - p0[2];

    // normal = edge2 × edge1.
    let nx = e2y * e1z - e2z * e1y;
    let ny = e2z * e1x - e1z * e2x;
    let nz = e1y * e2x - e2y * e1x;

    let scale = 1.0_f32 / (nx * nx + ny * ny + nz * nz).sqrt();
    let nnx = scale * nx;
    let nny = scale * ny;
    let nnz = scale * nz;
    // `D` sums z first, then y, then x (`0x63752e`..`0x63753b`: the z product is
    // formed, `FXCH`'d under the y product, and `FADDP st(2)` folds them before
    // the x product arrives). Left to right over x, y, z is a different number.
    let d = -((nnz * p0[2] + nny * p0[1]) + nnx * p0[0]);

    [nnx, nny, nnz, d]
}

#[cfg(test)]
mod tests_c4_plane__from_triangle__637480 {
    use super::c4_plane__from_triangle__637480 as f;

    fn dot3(a: &[f32; 3], b: &[f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    #[test]
    fn normal_is_unit_length() {
        for (p0, p1, p2) in [
            ([0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            ([1.0, 2.0, 3.0], [4.0, 0.0, -1.0], [-2.0, 5.0, 2.0]),
            ([10.0, -3.0, 7.0], [10.5, -3.2, 9.0], [11.0, -2.0, 7.5]),
        ] {
            let pl = f(&p0, &p1, &p2);
            let n = [pl[0], pl[1], pl[2]];
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-5, "len {len} for {pl:?}");
        }
    }

    #[test]
    fn all_three_corners_lie_on_plane() {
        // plane(x) = dot(N, x) + D must be ~0 for each triangle corner.
        for (p0, p1, p2) in [
            ([0.0f32, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 3.0, 0.0]),
            ([1.0, 2.0, 3.0], [4.0, 0.0, -1.0], [-2.0, 5.0, 2.0]),
            ([-5.0, 1.0, 8.0], [3.0, -2.0, 0.5], [0.0, 0.0, 4.0]),
        ] {
            let pl = f(&p0, &p1, &p2);
            let n = [pl[0], pl[1], pl[2]];
            for p in [&p0, &p1, &p2] {
                let v = dot3(&n, p) + pl[3];
                assert!(v.abs() < 1e-4, "corner off-plane by {v}, plane {pl:?}");
            }
        }
    }

    #[test]
    fn normal_orthogonal_to_both_edges() {
        let p0 = [1.0f32, 2.0, 3.0];
        let p1 = [4.0, 0.0, -1.0];
        let p2 = [-2.0, 5.0, 2.0];
        let pl = f(&p0, &p1, &p2);
        let n = [pl[0], pl[1], pl[2]];
        let e1 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
        let e2 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        assert!(dot3(&n, &e1).abs() < 1e-4, "n·edge1 = {}", dot3(&n, &e1));
        assert!(dot3(&n, &e2).abs() < 1e-4, "n·edge2 = {}", dot3(&n, &e2));
    }

    #[test]
    fn known_xy_plane() {
        // Triangle in the z=0 plane wound so normal = edge2 × edge1.
        // p0=origin, p1=+x, p2=+y → edge1=+y, edge2=+x → edge2×edge1 = x̂×ŷ = +ẑ.
        let pl = f(&[0.0, 0.0, 0.0], &[1.0, 0.0, 0.0], &[0.0, 1.0, 0.0]);
        assert!(pl[0].abs() < 1e-6, "{pl:?}");
        assert!(pl[1].abs() < 1e-6, "{pl:?}");
        assert!((pl[2] - 1.0).abs() < 1e-6, "{pl:?}");
        assert!(pl[3].abs() < 1e-6, "{pl:?}");
    }

    #[test]
    fn known_offset_plane_d() {
        // Same winding but offset to z=5: normal stays +ẑ, D = -5.
        let pl = f(&[0.0, 0.0, 5.0], &[1.0, 0.0, 5.0], &[0.0, 1.0, 5.0]);
        assert!((pl[2] - 1.0).abs() < 1e-6, "{pl:?}");
        assert!((pl[3] + 5.0).abs() < 1e-5, "{pl:?}");
    }

    #[test]
    fn plane_distance_sums_z_then_y_then_x() {
        // Corners with full non-terminating mantissas, so the two groupings of
        // the `D` dot land on different bits. Bit patterns, not decimals: a
        // rounded literal is a different float and stops separating them.
        let p0 = [
            f32::from_bits(0xc0b6_dcce),
            f32::from_bits(0x40e5_86a5),
            f32::from_bits(0x424b_9eff),
        ];
        let p1 = [
            f32::from_bits(0xc083_e75b),
            f32::from_bits(0x3f70_e248),
            f32::from_bits(0x4127_c764),
        ];
        let p2 = [
            f32::from_bits(0xc217_5cf0),
            f32::from_bits(0x3fb6_eaad),
            f32::from_bits(0x4179_5ff4),
        ];
        let pl = f(&p0, &p1, &p2);
        let (nnx, nny, nnz) = (pl[0], pl[1], pl[2]);

        let stock = -((nnz * p0[2] + nny * p0[1]) + nnx * p0[0]);
        let left_to_right = -((nnx * p0[0] + nny * p0[1]) + nnz * p0[2]);
        assert_eq!(pl[3].to_bits(), stock.to_bits());
        assert_ne!(
            stock.to_bits(),
            left_to_right.to_bits(),
            "fixture no longer separates the two summation orders"
        );
    }

    #[test]
    fn degenerate_triangle_propagates_non_finite() {
        // Collinear corners → zero-length normal → 1.0/0.0 scale → non-finite.
        let pl = f(&[0.0, 0.0, 0.0], &[1.0, 1.0, 1.0], &[2.0, 2.0, 2.0]);
        assert!(
            pl.iter().any(|c| !c.is_finite()),
            "expected non-finite component, got {pl:?}"
        );
    }
}

/// Parametric segment-vs-plane intersection.
///
/// `ray` packs a parametric line: point `ray[0..3]` and a per-step direction
/// `ray[3..6]`. `plane` packs `normal = plane[0..3]` and offset `plane[3]`.
/// The denominator `denom = dot(plane.normal, ray.direction)` selects the path:
///
/// * Near-parallel (`|denom| < parallel_eps`, an f64 host threshold): if the
///   point's signed distance `|dot(plane.normal, ray.point) + offset| >=
///   hit_eps` the segment never crosses (case `0`); otherwise it is coincident
///   (case `1`) at parameter `0`, intersection = `ray.point`.
/// * Otherwise (case `2`) solve `t = -(dot(plane.normal, ray.point) + offset) /
///   denom` and return point `ray.point + t * ray.direction`. When the
///   numerator's magnitude is below `hit_eps` the parameter collapses to the
///   `t_zero` constant instead of the division.
///
/// Returns `(case, t, point)`; `t`/`point` are unused for case `0`.
pub fn plane_intersect_ray_param__7c22b0(
    ray: &[f32; 6],
    plane: &[f32; 4],
    parallel_eps: f64,
    hit_eps: f32,
    t_zero: f32,
) -> (u32, f32, [f32; 3]) {
    // denom = ray[4]*plane[1] + ray[5]*plane[2] + ray[3]*plane[0]
    //       = dot(plane.normal, ray.direction), in the binary's lane order.
    let mut denom = ray[4] * plane[1];
    denom += ray[5] * plane[2];
    denom += ray[3] * plane[0];

    // The host threshold is a double; the x87 original promotes |denom| to
    // compare against it, so the parallel test is performed in f64.
    if f64::from(denom.abs()) < parallel_eps {
        // Signed distance of the ray point to the plane:
        // ray[1]*plane[1] + ray[2]*plane[2] + ray[0]*plane[0] + plane[3].
        let mut dist = ray[1] * plane[1];
        dist += ray[2] * plane[2];
        dist += ray[0] * plane[0];
        dist += plane[3];
        let dist = dist.abs();
        if dist < hit_eps {
            (1, 0.0, [ray[0], ray[1], ray[2]])
        } else {
            (0, 0.0, [ray[0], ray[1], ray[2]])
        }
    } else {
        // num = ray[0]*plane[0] + ray[2]*plane[2] + ray[1]*plane[1] + plane[3]
        //     = dot(plane.normal, ray.point) + offset, binary lane order.
        let mut num = ray[0] * plane[0];
        num += ray[2] * plane[2];
        num += ray[1] * plane[1];
        num += plane[3];
        let t = if num.abs() < hit_eps {
            t_zero
        } else {
            -(num / denom)
        };
        let point = [
            ray[0] + t * ray[3],
            ray[1] + t * ray[4],
            ray[2] + t * ray[5],
        ];
        (2, t, point)
    }
}

#[cfg(test)]
mod tests_plane_intersect_ray_param__7c22b0 {
    use super::plane_intersect_ray_param__7c22b0 as f;

    const PEPS: f64 = 1.0e-6;
    const HEPS: f32 = 1.0e-4;
    const TZERO: f32 = 0.0;

    fn plane_eval(plane: &[f32; 4], p: &[f32; 3]) -> f32 {
        plane[0] * p[0] + plane[1] * p[1] + plane[2] * p[2] + plane[3]
    }

    #[test]
    fn crosses_z_plane_at_known_t() {
        // ray point (0,0,3), direction (0,0,-1); plane z=0 -> normal (0,0,1).
        // num = 1*3 = 3, denom = 1*-1 = -1, t = -(3/-1) = 3.
        let ray = [0.0, 0.0, 3.0, 0.0, 0.0, -1.0];
        let plane = [0.0, 0.0, 1.0, 0.0];
        let (case, t, p) = f(&ray, &plane, PEPS, HEPS, TZERO);
        assert_eq!(case, 2);
        assert!((t - 3.0).abs() < 1e-5, "t={t}");
        // point = (0,0,3)+3*(0,0,-1) = (0,0,0): on the plane.
        assert!(plane_eval(&plane, &p).abs() < 1e-4, "p={p:?}");
    }

    #[test]
    fn intersection_point_lies_on_plane() {
        // Generic non-axis-aligned crossing: returned point must satisfy the
        // plane equation regardless of which axis the normal favours
        // (multi-axis guard against a transposed normal/direction map).
        let plane = [0.3f32, -0.8, 0.5, 1.25];
        for ray in [
            [1.0f32, 2.0, -3.0, 0.4, -1.1, 0.9],
            [-4.0, 0.5, 6.0, -0.2, 0.7, -1.3],
            [10.0, -10.0, 10.0, 1.3, 0.2, -0.6],
        ] {
            let (case, _t, p) = f(&ray, &plane, PEPS, HEPS, TZERO);
            assert_eq!(case, 2, "ray {ray:?}");
            assert!(
                plane_eval(&plane, &p).abs() < 1e-3,
                "point {p:?} off plane by {}",
                plane_eval(&plane, &p)
            );
        }
    }

    #[test]
    fn t_reconstructs_point() {
        // point == ray.point + t*direction exactly (metamorphic on the parameter).
        let ray = [2.0f32, -1.0, 4.0, 0.5, 0.25, -0.75];
        let plane = [0.0f32, 1.0, 0.0, -2.0]; // y = 2
        let (case, t, p) = f(&ray, &plane, PEPS, HEPS, TZERO);
        assert_eq!(case, 2);
        let expect = [
            ray[0] + t * ray[3],
            ray[1] + t * ray[4],
            ray[2] + t * ray[5],
        ];
        for (a, b) in p.iter().zip(expect.iter()) {
            assert_eq!(a.to_bits(), b.to_bits());
        }
    }

    #[test]
    fn parallel_and_off_plane_misses() {
        // direction parallel to the plane and the point off the plane -> miss.
        // normal (0,0,1); direction (1,0,0) -> denom = 0 -> parallel.
        let ray = [0.0, 0.0, 5.0, 1.0, 0.0, 0.0];
        let plane = [0.0, 0.0, 1.0, 0.0]; // z=0; point z=5 far off
        let (case, _t, _p) = f(&ray, &plane, PEPS, HEPS, TZERO);
        assert_eq!(case, 0);
    }

    #[test]
    fn parallel_and_on_plane_is_coincident() {
        // Parallel direction but the point lies on the plane -> coincident
        // (case 1), t=0, point=ray.point.
        let ray = [3.0, -2.0, 0.0, 1.0, 0.0, 0.0];
        let plane = [0.0, 0.0, 1.0, 0.0]; // z=0; point z=0 on-plane
        let (case, t, p) = f(&ray, &plane, PEPS, HEPS, TZERO);
        assert_eq!(case, 1);
        assert_eq!(t.to_bits(), 0.0f32.to_bits());
        assert_eq!(p, [3.0, -2.0, 0.0]);
    }

    #[test]
    fn tiny_numerator_uses_t_zero() {
        // point essentially on the plane so |num| < hit_eps, but direction not
        // parallel -> divide branch picks the t_zero fallback, not the division.
        let tzero = 7.5f32; // sentinel to prove the fallback is taken
        let ray = [0.0, 0.0, 0.0, 0.0, 0.0, 1.0]; // point on z=0, direction crosses
        let plane = [0.0, 0.0, 1.0, 0.0];
        let (case, t, _p) = f(&ray, &plane, PEPS, HEPS, tzero);
        assert_eq!(case, 2);
        assert_eq!(t.to_bits(), tzero.to_bits());
    }

    #[test]
    fn boundary_distance_equal_to_eps_misses() {
        // Parallel branch: a signed distance exactly == hit_eps must NOT count
        // as a hit (binary uses strict `<`, so equality falls to case 0).
        let ray = [0.0, 0.0, HEPS, 1.0, 0.0, 0.0]; // z offset == hit_eps
        let plane = [0.0, 0.0, 1.0, 0.0];
        let (case, _t, _p) = f(&ray, &plane, PEPS, HEPS, TZERO);
        assert_eq!(case, 0, "distance == eps must miss (strict <)");
    }
}

/// `Geometry::ReducePlanarPointsToRepresentative`.
///
/// Validate up to four points against a fixed reference plane, then collapse
/// them to one representative.
///
/// `pts` is the first `count` (1..=4) input `C3Vectors` (`[x, y, z, _unused]`); the
/// return is `Some(repr_xyz)` on success or `None` when the count is out of range
/// or any point fails the z-plane tolerance.
///
/// Validation: every point's `z` must satisfy `|z − Z_REF| < EPS` (strict, and
/// **NaN rejects**) — this reproduces the x87 `FLD;FSUB;FABS;FCOMP;FNSTSW;TEST
/// AH,5;JP` polarity, where `AH & 5` tests `C0`(<) and `C2`(unordered): `>= EPS`
/// or `NaN` both set an even 1-count ⇒ `JP` taken ⇒ reject. Written `!(dz < EPS)`
/// (not `dz >= EPS`) so NaN routes to reject.
///
/// Collapse, keyed on `count`:
/// * `1` — the point itself.
/// * `2` — `(p0 + p1) * K` per component, where `K = 0x3f237868` (≈0.6385560, the
///   raw factor at `0x80e024` — *not* an exact 0.5 midpoint). The x87 sums in
///   80-bit then applies one `FMUL`; tracked in `f64` and narrowed once.
/// * `3` — bucket each of the three points: `|y| < EPS` ⇒ y-bucket, else
///   `|x| < EPS` ⇒ x-bucket, else ignored (NaN falls to the `else` arm, matching
///   the `JP` fall-through). Emit the sole y-bucket point when exactly one exists,
///   otherwise the first x-bucket point (stock reads an uninitialised slot when
///   the x-bucket is empty; we default to `pts[0]`, the benign in-bounds choice).
/// * `4` — the origin `(0, 0, 0)`.
///
/// The `count` guard and the two OOB/uninitialised-stack quirks noted above are
/// the stock behaviour; only the latent stack-OOB write is neutralised (we use an
/// in-bounds bucket), and it is never *read*, so results are identical.
// The z gate is written `!(dz < EPS)` so a NaN `dz` takes the reject branch, as
// the original's `FCOMP`/`JP` pair does; the suggested `dz >= EPS` is false for
// NaN and would let such a point through.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn geometry__reduce_planar_points_to_representative__636610(
    pts: &[[f32; 4]],
    count: u32,
) -> Option<[f32; 3]> {
    // Plane z reference at 0x80e014: -0.4756366.
    const Z_REF: f32 = f32::from_bits(0xbef3_86a4);
    // z / axis tolerance at 0x8026bc: 2^-20.
    const EPS: f32 = f32::from_bits(0x3580_0000);
    // Case-2 scale factor at 0x80e024: 0.6385560.
    const K: f32 = f32::from_bits(0x3f23_7868);

    // count guard: valid only for 1..=4 (stock JZ on 0, JA on >4 -> reject).
    if count == 0 || count > 4 {
        return None;
    }
    let n = count as usize;

    // z-plane tolerance: reject on the first point that is NOT strictly within EPS
    // (>= EPS or NaN both reject), matching the x87 FCOMP;TEST AH,5;JP polarity.
    for p in &pts[..n] {
        let dz = (p[2] - Z_REF).abs();
        if !(dz < EPS) {
            return None;
        }
    }

    Some(match count {
        1 => [pts[0][0], pts[0][1], pts[0][2]],
        2 => {
            // (p0 + p1) * K per component; 80-bit x87 add+mul tracked in f64.
            let blend =
                |a: f32, b: f32| super::f64_to_f32((f64::from(a) + f64::from(b)) * f64::from(K));
            [
                blend(pts[0][0], pts[1][0]),
                blend(pts[0][1], pts[1][1]),
                blend(pts[0][2], pts[1][2]),
            ]
        }
        3 => {
            // Axis bucketing over the three points (y-bucket has priority).
            let mut y_bucket = [0usize; 3];
            let mut y_count = 0usize;
            let mut x_bucket = [0usize; 3];
            let mut x_count = 0usize;
            for (i, p) in pts[..3].iter().enumerate() {
                if p[1].abs() < EPS {
                    y_bucket[y_count] = i;
                    y_count += 1;
                } else if p[0].abs() < EPS {
                    x_bucket[x_count] = i;
                    x_count += 1;
                }
            }
            let sel = if y_count == 1 {
                y_bucket[0]
            } else {
                x_bucket[0]
            };
            [pts[sel][0], pts[sel][1], pts[sel][2]]
        }
        // count == 4 -> origin.
        _ => [0.0, 0.0, 0.0],
    })
}

#[cfg(test)]
mod tests_geometry__reduce_planar_points_to_representative__636610 {
    use super::geometry__reduce_planar_points_to_representative__636610 as reduce;

    // Match the kernel's baked-in reference constants.
    const Z_REF: f32 = f32::from_bits(0xbef3_86a4); // -0.4756366
    const EPS: f32 = f32::from_bits(0x3580_0000); // 2^-20
    const K: f32 = f32::from_bits(0x3f23_7868); // 0.6385560

    // A point exactly on the reference plane (z == Z_REF, so |dz| = 0 < EPS).
    fn on_plane(x: f32, y: f32) -> [f32; 4] {
        [x, y, Z_REF, 0.0]
    }

    #[test]
    fn count_zero_and_over_four_reject() {
        assert!(reduce(&[], 0).is_none());
        let four = [on_plane(0.0, 0.0); 4];
        assert!(reduce(&four, 5).is_none());
        assert!(reduce(&four, 100).is_none());
    }

    #[test]
    fn count_one_is_identity() {
        let p = on_plane(3.5, -2.25);
        let r = reduce(std::slice::from_ref(&p), 1).unwrap();
        assert_eq!(r, [3.5, -2.25, Z_REF]);
    }

    #[test]
    fn z_out_of_tolerance_rejects() {
        // z well past EPS from the reference -> reject.
        let p = [1.0, 2.0, Z_REF + EPS * 2.0, 0.0];
        assert!(reduce(std::slice::from_ref(&p), 1).is_none());
    }

    #[test]
    fn z_equal_to_eps_rejects_strict_less() {
        // |z - Z_REF| == EPS must reject (stock uses strict `<`).
        let p = [0.0, 0.0, Z_REF + EPS, 0.0];
        assert!(reduce(std::slice::from_ref(&p), 1).is_none());
    }

    #[test]
    fn nan_z_rejects() {
        // NaN in z -> |dz| is NaN -> !(NaN < EPS) is true -> reject.
        let p = [0.0, 0.0, f32::NAN, 0.0];
        assert!(reduce(std::slice::from_ref(&p), 1).is_none());
        // NaN only in a later point still rejects the whole set.
        let pts = [
            on_plane(0.0, 0.0),
            on_plane(1.0, 1.0),
            [0.0, 0.0, f32::NAN, 0.0],
        ];
        assert!(reduce(&pts, 3).is_none());
    }

    #[test]
    fn count_two_blends_with_k_factor() {
        let a = on_plane(2.0, -4.0);
        let b = on_plane(6.0, 10.0);
        let pts = [a, b];
        let r = reduce(&pts, 2).unwrap();
        // Independent oracle: (p0 + p1) * K, component-wise, single-rounded.
        let oracle = |x: f32, y: f32| ((f64::from(x) + f64::from(y)) * f64::from(K)) as f32;
        assert_eq!(r[0].to_bits(), oracle(2.0, 6.0).to_bits());
        assert_eq!(r[1].to_bits(), oracle(-4.0, 10.0).to_bits());
        assert_eq!(r[2].to_bits(), oracle(Z_REF, Z_REF).to_bits());
        // K is the game factor, so this is NOT the arithmetic midpoint.
        assert_ne!(r[0].to_bits(), 4.0f32.to_bits());
    }

    #[test]
    fn count_four_is_origin() {
        let pts = [on_plane(9.0, 9.0); 4];
        assert_eq!(reduce(&pts, 4).unwrap(), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn count_three_single_y_hit_picks_that_point() {
        // Point 1 has |y| < EPS (and any x); points 0,2 have |y| >= EPS.
        let p0 = on_plane(5.0, 1.0);
        let p1 = on_plane(7.0, 0.0); // sole y-bucket member
        let p2 = on_plane(9.0, 2.0);
        let pts = [p0, p1, p2];
        let r = reduce(&pts, 3).unwrap();
        assert_eq!(r, [7.0, 0.0, Z_REF]);
    }

    #[test]
    fn count_three_zero_y_hits_picks_first_x_hit() {
        // No point has |y| < EPS; points 1 and 2 have |x| < EPS -> first is p1.
        let p0 = on_plane(5.0, 3.0);
        let p1 = on_plane(0.0, 4.0); // first x-bucket member
        let p2 = on_plane(0.0, 8.0);
        let pts = [p0, p1, p2];
        let r = reduce(&pts, 3).unwrap();
        assert_eq!(r, [0.0, 4.0, Z_REF]);
    }

    #[test]
    fn count_three_two_y_hits_falls_through_to_x_bucket() {
        // Two y-hits (y_count == 2, not 1) -> the y-branch is skipped; the only
        // x-bucket member is p2 (its |y| >= EPS so it is x-eligible, |x| < EPS).
        let p0 = on_plane(1.0, 0.0); // y-hit
        let p1 = on_plane(2.0, 0.0); // y-hit
        let p2 = on_plane(0.0, 5.0); // x-hit
        let pts = [p0, p1, p2];
        let r = reduce(&pts, 3).unwrap();
        assert_eq!(r, [0.0, 5.0, Z_REF]);
    }

    #[test]
    fn count_three_no_buckets_defaults_to_first_point() {
        // No y-hit and no x-hit -> x-bucket empty -> default index 0 (pts[0]).
        let p0 = on_plane(5.0, 5.0);
        let p1 = on_plane(6.0, 6.0);
        let p2 = on_plane(7.0, 7.0);
        let pts = [p0, p1, p2];
        let r = reduce(&pts, 3).unwrap();
        assert_eq!(r, [5.0, 5.0, Z_REF]);
    }

    #[test]
    fn count_three_nan_y_routes_to_x_bucket() {
        // NaN in y must fall to the x-check (JP fall-through), not the y-bucket;
        // here p1 has NaN y but |x| < EPS, so it becomes the sole x-bucket member.
        // (z stays on-plane so validation passes.)
        let p0 = on_plane(5.0, 3.0);
        let p1 = [0.0, f32::NAN, Z_REF, 0.0];
        let p2 = on_plane(9.0, 8.0);
        let pts = [p0, p1, p2];
        let r = reduce(&pts, 3).unwrap();
        assert_eq!(r[0].to_bits(), 0.0f32.to_bits());
        assert!(r[1].is_nan());
        assert_eq!(r[2], Z_REF);
    }
}
