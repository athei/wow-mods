// Adapter and kernel names mirror the host's C++ symbols verbatim, with `__`
// standing in for the `::`, so the whole module is non-snake-case by construction.
#![allow(non_snake_case)]

/// `CWorldBsp::AabbTriangleOverlap` separating-axis test along the three coordinate axes.
///
/// `aabb` is `[min_x, min_y, min_z, max_x, max_y, max_z]`; `v0`/`v1`/`v2` are
/// the triangle vertices. Returns `1` if a separating axis is found (the box
/// and triangle are disjoint), `0` if no coordinate axis separates them
/// (potential overlap).
///
/// Mirrors the reference's sign-bit arithmetic exactly: per axis it looks at
/// the sign bit of `(max - vert)` and of `(vert - min)` for each vertex, and a
/// separating axis is one where all three verts lie strictly above `max`
/// (every `max - vert` is negative) or all three lie strictly below `min`
/// (every `vert - min` is negative). A vertex exactly on a face yields `+0.0`
/// (sign bit clear) and therefore does *not* separate — matching the SSE
/// `subps`/`movmskps` lane behaviour, including the `-0.0` and NaN edge cases
/// that a plain `>`/`<` comparison would get subtly wrong.
pub fn c_world_bsp__aabb_triangle_overlap__6b8b70(
    aabb: &[f32; 6],
    v0: &[f32; 3],
    v1: &[f32; 3],
    v2: &[f32; 3],
) -> u32 {
    const SIGN: u32 = 0x8000_0000;
    let verts = [v0, v1, v2];
    for axis in 0..3 {
        let min = aabb[axis];
        let max = aabb[axis + 3];
        let mut all_above_max = SIGN;
        let mut all_below_min = SIGN;
        for v in verts {
            let c = v[axis];
            all_above_max &= (max - c).to_bits() & SIGN;
            all_below_min &= (c - min).to_bits() & SIGN;
        }
        if all_above_max != 0 || all_below_min != 0 {
            return 1;
        }
    }
    0
}

#[cfg(test)]
mod tests_c_world_bsp__aabb_triangle_overlap__6b8b70 {
    use super::c_world_bsp__aabb_triangle_overlap__6b8b70 as f;

    // A unit box centred at the origin: min = (-1,-1,-1), max = (1,1,1).
    const BOX: [f32; 6] = [-1.0, -1.0, -1.0, 1.0, 1.0, 1.0];

    #[test]
    fn triangle_inside_overlaps() {
        // All three verts strictly inside the box -> no separating axis -> 0.
        assert_eq!(
            f(&BOX, &[0.0, 0.0, 0.0], &[0.5, 0.0, 0.0], &[0.0, 0.5, 0.0]),
            0
        );
    }

    #[test]
    fn triangle_entirely_above_max_x_separates() {
        // Every vertex has x > 1 -> separating axis on X -> 1.
        assert_eq!(
            f(&BOX, &[2.0, 0.0, 0.0], &[3.0, 0.5, 0.0], &[2.5, -0.5, 0.0]),
            1
        );
    }

    #[test]
    fn triangle_entirely_below_min_y_separates() {
        // Every vertex has y < -1 -> separating axis on Y -> 1.
        assert_eq!(
            f(
                &BOX,
                &[0.0, -2.0, 0.0],
                &[0.5, -3.0, 0.0],
                &[-0.5, -2.5, 0.0]
            ),
            1
        );
    }

    #[test]
    fn triangle_entirely_above_max_z_separates() {
        assert_eq!(
            f(&BOX, &[0.0, 0.0, 5.0], &[0.5, 0.0, 6.0], &[0.0, 0.5, 5.5]),
            1
        );
    }

    #[test]
    fn straddling_max_does_not_separate() {
        // One vertex above max-x, two below -> not all above -> no separation
        // on X; and since they straddle every axis there is no separating axis.
        assert_eq!(
            f(&BOX, &[2.0, 0.0, 0.0], &[0.0, 0.0, 0.0], &[-0.5, 0.0, 0.0]),
            0
        );
    }

    #[test]
    fn vertex_exactly_on_max_face_does_not_separate() {
        // All verts at x == max: `max - vert == +0.0`, sign bit clear, so the
        // axis is NOT counted as separating (boundary-inclusive on max side).
        assert_eq!(
            f(&BOX, &[1.0, 0.0, 0.0], &[1.0, 0.5, 0.0], &[1.0, -0.5, 0.0]),
            0
        );
    }

    #[test]
    fn vertex_exactly_on_min_face_does_not_separate() {
        // `vert - min == +0.0` for x == min: not separating on the min side.
        assert_eq!(
            f(
                &BOX,
                &[-1.0, 0.0, 0.0],
                &[-1.0, 0.5, 0.0],
                &[-1.0, -0.5, 0.0]
            ),
            0
        );
    }

    #[test]
    fn just_beyond_max_separates() {
        // Strictly beyond the max face on every vertex -> separates.
        let eps = 1.0 + f32::EPSILON;
        assert_eq!(
            f(&BOX, &[eps, 0.0, 0.0], &[eps, 0.5, 0.0], &[eps, -0.5, 0.0]),
            1
        );
    }

    #[test]
    fn translation_invariance() {
        // Translating box and triangle together by the same vector preserves
        // the overlap verdict (the test is purely relative per axis).
        let tri0 = [0.0f32, 0.0, 0.0];
        let tri1 = [2.0f32, 0.0, 0.0];
        let tri2 = [0.0f32, 2.0, 0.0];
        let base = f(&BOX, &tri0, &tri1, &tri2);
        let shift = 10.0f32;
        let sbox = [
            BOX[0] + shift,
            BOX[1] + shift,
            BOX[2] + shift,
            BOX[3] + shift,
            BOX[4] + shift,
            BOX[5] + shift,
        ];
        let st0 = [tri0[0] + shift, tri0[1] + shift, tri0[2] + shift];
        let st1 = [tri1[0] + shift, tri1[1] + shift, tri1[2] + shift];
        let st2 = [tri2[0] + shift, tri2[1] + shift, tri2[2] + shift];
        assert_eq!(base, f(&sbox, &st0, &st1, &st2));
    }

    #[test]
    fn separation_only_needs_one_axis() {
        // Triangle clears the box on X (all x > max) even though Y/Z overlap.
        assert_eq!(
            f(&BOX, &[2.0, 0.0, 0.0], &[2.0, 0.5, 0.5], &[2.0, -0.5, -0.5]),
            1
        );
    }

    #[test]
    fn degenerate_point_triangle_inside() {
        // All three verts coincident and inside -> overlapping.
        let p = [0.25f32, -0.25, 0.5];
        assert_eq!(f(&BOX, &p, &p, &p), 0);
    }

    #[test]
    fn degenerate_point_triangle_outside() {
        // All three verts coincident and outside on +x -> separated.
        let p = [9.0f32, 0.0, 0.0];
        assert_eq!(f(&BOX, &p, &p, &p), 1);
    }
}

/// OBB-vs-OBB overlap test via the Separating Axis Theorem (Gottschalk).
///
/// Each box is 15 contiguous `f32`: center `[0..3]`, half-extents `[3..6]`, and
/// three orientation rows (local axes) at `[6..9]`, `[9..12]`, `[12..15]`.
/// Returns `1` when the two oriented boxes overlap and `0` once a separating
/// axis is found. Contact is inclusive: a projected distance exactly equal to
/// the radius sum still counts as overlap, matching the reference's `<` test.
pub fn collide_box_box__7c3780(a: &[f32; 15], b: &[f32; 15]) -> i32 {
    let ae = [a[3], a[4], a[5]];
    let be = [b[3], b[4], b[5]];
    let aa = [
        [a[6], a[7], a[8]],
        [a[9], a[10], a[11]],
        [a[12], a[13], a[14]],
    ];
    let ba = [
        [b[6], b[7], b[8]],
        [b[9], b[10], b[11]],
        [b[12], b[13], b[14]],
    ];

    // R[i][j] = dot(A.axis_i, B.axis_j); AbsR = |R|.
    let mut r = [[0.0f32; 3]; 3];
    let mut absr = [[0.0f32; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            let d = aa[i][0] * ba[j][0] + aa[i][1] * ba[j][1] + aa[i][2] * ba[j][2];
            r[i][j] = d;
            absr[i][j] = d.abs();
        }
    }

    // Center delta and its projection into A's frame.
    let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let t = [
        d[0] * aa[0][0] + d[1] * aa[0][1] + d[2] * aa[0][2],
        d[0] * aa[1][0] + d[1] * aa[1][1] + d[2] * aa[1][2],
        d[0] * aa[2][0] + d[1] * aa[2][1] + d[2] * aa[2][2],
    ];

    // 3 face axes of A.
    if absr[0][0] * be[0] + absr[0][1] * be[1] + absr[0][2] * be[2] + ae[0] < t[0].abs() {
        return 0;
    }
    if absr[1][0] * be[0] + absr[1][1] * be[1] + absr[1][2] * be[2] + ae[1] < t[1].abs() {
        return 0;
    }
    if absr[2][0] * be[0] + absr[2][1] * be[1] + absr[2][2] * be[2] + ae[2] < t[2].abs() {
        return 0;
    }

    // 3 face axes of B.
    let tb = (d[0] * ba[0][0] + d[1] * ba[0][1] + d[2] * ba[0][2]).abs();
    if absr[0][0] * ae[0] + absr[1][0] * ae[1] + absr[2][0] * ae[2] + be[0] < tb {
        return 0;
    }
    let tb = (d[0] * ba[1][0] + d[1] * ba[1][1] + d[2] * ba[1][2]).abs();
    if absr[0][1] * ae[0] + absr[1][1] * ae[1] + absr[2][1] * ae[2] + be[1] < tb {
        return 0;
    }
    let tb = (d[0] * ba[2][0] + d[1] * ba[2][1] + d[2] * ba[2][2]).abs();
    if absr[0][2] * ae[0] + absr[1][2] * ae[1] + absr[2][2] * ae[2] + be[2] < tb {
        return 0;
    }

    // 9 edge-edge cross axes (A.axis_i x B.axis_j).
    if absr[2][0] * ae[1] + absr[1][0] * ae[2] + absr[0][2] * be[1] + absr[0][1] * be[2]
        < (t[2] * r[1][0] - t[1] * r[2][0]).abs()
    {
        return 0;
    }
    if absr[2][1] * ae[1] + absr[1][1] * ae[2] + absr[0][2] * be[0] + absr[0][0] * be[2]
        < (t[2] * r[1][1] - t[1] * r[2][1]).abs()
    {
        return 0;
    }
    if absr[2][2] * ae[1] + absr[1][2] * ae[2] + absr[0][1] * be[0] + absr[0][0] * be[1]
        < (t[2] * r[1][2] - t[1] * r[2][2]).abs()
    {
        return 0;
    }
    if absr[2][0] * ae[0] + absr[0][0] * ae[2] + absr[1][2] * be[1] + absr[1][1] * be[2]
        < (t[0] * r[2][0] - t[2] * r[0][0]).abs()
    {
        return 0;
    }
    if absr[2][1] * ae[0] + absr[0][1] * ae[2] + absr[1][2] * be[0] + absr[1][0] * be[2]
        < (t[0] * r[2][1] - t[2] * r[0][1]).abs()
    {
        return 0;
    }
    if absr[2][2] * ae[0] + absr[0][2] * ae[2] + absr[1][1] * be[0] + absr[1][0] * be[1]
        < (t[0] * r[2][2] - t[2] * r[0][2]).abs()
    {
        return 0;
    }
    if absr[1][0] * ae[0] + absr[0][0] * ae[1] + absr[2][2] * be[1] + absr[2][1] * be[2]
        < (t[1] * r[0][0] - t[0] * r[1][0]).abs()
    {
        return 0;
    }
    if absr[1][1] * ae[0] + absr[0][1] * ae[1] + absr[2][2] * be[0] + absr[2][0] * be[2]
        < (t[1] * r[0][1] - t[0] * r[1][1]).abs()
    {
        return 0;
    }
    if absr[1][2] * ae[0] + absr[0][2] * ae[1] + absr[2][1] * be[0] + absr[2][0] * be[1]
        < (t[1] * r[0][2] - t[0] * r[1][2]).abs()
    {
        return 0;
    }

    1
}

#[cfg(test)]
mod tests_collide_box_box__7c3780 {
    use super::collide_box_box__7c3780;

    fn aabb(c: [f32; 3], e: [f32; 3]) -> [f32; 15] {
        [
            c[0], c[1], c[2], e[0], e[1], e[2], 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0,
        ]
    }

    fn rot_z_box(c: [f32; 3], e: [f32; 3], ang: f32) -> [f32; 15] {
        let (s, co) = crate::math::trig::sin_cos(ang);
        [
            c[0], c[1], c[2], e[0], e[1], e[2], co, s, 0.0, -s, co, 0.0, 0.0, 0.0, 1.0,
        ]
    }

    #[test]
    fn identical_boxes_overlap() {
        let b = aabb([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        assert_eq!(collide_box_box__7c3780(&b, &b), 1);
    }

    #[test]
    fn disjoint_on_each_axis() {
        let a = aabb([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        assert_eq!(
            collide_box_box__7c3780(&a, &aabb([3.0, 0.0, 0.0], [1.0, 1.0, 1.0])),
            0
        );
        assert_eq!(
            collide_box_box__7c3780(&a, &aabb([0.0, 5.0, 0.0], [1.0, 1.0, 1.0])),
            0
        );
        assert_eq!(
            collide_box_box__7c3780(&a, &aabb([0.0, 0.0, -10.0], [1.0, 1.0, 1.0])),
            0
        );
    }

    #[test]
    fn touching_is_inclusive_overlap() {
        // separation exactly equal to the radius sum -> overlap (rb == t, not <)
        let a = aabb([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let b = aabb([2.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        assert_eq!(collide_box_box__7c3780(&a, &b), 1);
    }

    #[test]
    fn overlapping_corner() {
        let a = aabb([0.0, 0.0, 0.0], [2.0, 2.0, 2.0]);
        let b = aabb([1.0, 1.0, 1.0], [2.0, 2.0, 2.0]);
        assert_eq!(collide_box_box__7c3780(&a, &b), 1);
    }

    #[test]
    fn rotated_thin_boxes_separated_by_cross_axis() {
        let a = aabb([0.0, 0.0, 0.0], [0.5, 4.0, 0.5]);
        let b = rot_z_box(
            [6.0, 0.0, 0.0],
            [0.5, 4.0, 0.5],
            std::f32::consts::FRAC_PI_4,
        );
        assert_eq!(collide_box_box__7c3780(&a, &b), 0);
    }

    #[test]
    fn rotation_invariance_of_concentric_overlap() {
        for k in 0..8 {
            let ang = (k as f32) * std::f32::consts::FRAC_PI_4 / 3.0 + 0.1;
            let a = rot_z_box([0.0, 0.0, 0.0], [1.0, 2.0, 1.5], ang);
            let b = rot_z_box([0.3, -0.2, 0.1], [1.0, 1.0, 1.0], ang * 0.5);
            assert_eq!(collide_box_box__7c3780(&a, &b), 1, "k={k}");
        }
    }

    #[test]
    fn overlap_relation_is_symmetric() {
        let a = rot_z_box([0.0, 0.0, 0.0], [1.0, 2.0, 1.5], 0.6);
        let b = rot_z_box([1.5, 0.5, 0.0], [1.0, 1.0, 1.0], 1.1);
        assert_eq!(
            collide_box_box__7c3780(&a, &b),
            collide_box_box__7c3780(&b, &a)
        );
    }
}

/// Möller-Trumbore line-segment vs triangle test, returning `[t, u, v]` on a hit.
///
/// `segment` packs the ray origin in `[0..3]` and direction in `[3..6]`; `v0`,
/// `v1`, `v2` are the triangle's three vertices. `edge_tol` widens the accepted
/// barycentric band to `[-edge_tol, 1 + edge_tol]`. The reconstructed arithmetic
/// mirrors the reference exactly: `pvec = edge2 × dir`, `qvec = tvec × edge1`,
/// the determinant must lie outside the near-zero band `(-1e-6, 1e-6)`, the
/// barycentrics `u` and `u + v` (not the ray parameter `t`) stay within the
/// tolerance band, and the returned `t = dot(qvec, edge2) / det` is left
/// unclamped. Returns `None` on a miss.
// The band tests are ordered-negative — `v < lo`, `u + v > hi` — so a miss needs
// an ordered compare and a NaN barycentric reaches the hit path, as in the
// original: its `FCOM` + `TEST AH,5`/`JNP` and `TEST AH,0x41`/`JZ` pairs all
// leave the unordered case falling through.
pub fn collide_line_triangle_indexed16__7c29f0(
    segment: &[f32; 6],
    v0: &[f32; 3],
    v1: &[f32; 3],
    v2: &[f32; 3],
    edge_tol: f32,
) -> Option<[f32; 3]> {
    // Acceptance band for the barycentrics (triangle u/v widened by the tol).
    let t_min = -edge_tol;
    let t_max = edge_tol + 1.0_f32;

    let dir = [segment[3], segment[4], segment[5]];

    // edge1 = v1 - v0, edge2 = v2 - v0.
    let e1x = v1[0] - v0[0];
    let e1y = v1[1] - v0[1];
    let e1z = v1[2] - v0[2];
    let e2x = v2[0] - v0[0];
    let e2y = v2[1] - v0[1];
    let e2z = v2[2] - v0[2];

    // pvec = edge2 × dir.
    let px = e2z * dir[1] - e2y * dir[2];
    let py = e2x * dir[2] - e2z * dir[0];
    let pz = e2y * dir[0] - e2x * dir[1];

    // det = dot(pvec, edge1).
    let det = px * e1x + py * e1y + pz * e1z;

    // Reject determinants inside the near-zero band (-1e-6, 1e-6).
    if det > -1.0e-6_f32 && det < 1.0e-6_f32 {
        return None;
    }

    let inv_det = 1.0_f32 / det;

    // tvec = origin - v0.
    let tx = segment[0] - v0[0];
    let ty = segment[1] - v0[1];
    let tz = segment[2] - v0[2];

    // u = dot(tvec, pvec) * invDet.
    let u = (tx * px + ty * py + tz * pz) * inv_det;
    if u < t_min || u > t_max {
        return None;
    }

    // qvec = tvec × edge1.
    let qx = ty * e1z - tz * e1y;
    let qy = tz * e1x - e1z * tx;
    let qz = e1y * tx - ty * e1x;

    // v = dot(dir, qvec) * invDet.
    let v = (qx * dir[0] + qy * dir[1] + qz * dir[2]) * inv_det;
    if v < t_min {
        return None;
    }
    if u + v > t_max {
        return None;
    }

    // t = dot(qvec, edge2) * invDet (the ray parameter; intentionally unclamped).
    let t = (qx * e2x + qy * e2y + qz * e2z) * inv_det;
    Some([t, u, v])
}

#[cfg(test)]
mod tests_collide_line_triangle_indexed16__7c29f0 {
    use super::collide_line_triangle_indexed16__7c29f0 as f;

    fn reference(
        o: [f32; 3],
        d: [f32; 3],
        v0: [f32; 3],
        v1: [f32; 3],
        v2: [f32; 3],
        tol: f32,
    ) -> Option<[f32; 3]> {
        let seg = [o[0], o[1], o[2], d[0], d[1], d[2]];
        f(&seg, &v0, &v1, &v2, tol)
    }

    /// Law: a NaN barycentric reaches the hit path, not a miss.
    ///
    /// The original's four band compares are the unordered-tolerant `TEST AH,5`
    /// / `TEST AH,0x41` pairs, so an unordered result falls through all of them.
    /// Pinned because the positive in-band predicate that would reject NaN reads
    /// more naturally and is the easy thing to reintroduce.
    #[test]
    fn nan_reaches_the_hit_path() {
        let seg = [0.25f32, 0.25, 1.0, 0.0, 0.0, -1.0];
        let r = f(
            &seg,
            &[f32::NAN, 0.0, 0.0],
            &[1.0, 0.0, 0.0],
            &[0.0, 1.0, 0.0],
            0.0,
        )
        .expect("NaN must not miss");
        assert!(r.iter().any(|c| c.is_nan()), "{r:?}");
    }

    #[test]
    fn hits_centroid_of_unit_triangle() {
        // Triangle in the z = 0 plane; ray from above straight down hits the
        // interior point (0.25, 0.25, 0) at t = 1.
        let v0 = [0.0_f32, 0.0, 0.0];
        let v1 = [1.0_f32, 0.0, 0.0];
        let v2 = [0.0_f32, 1.0, 0.0];
        let o = [0.25_f32, 0.25, 1.0];
        let d = [0.0_f32, 0.0, -1.0];
        let hit = reference(o, d, v0, v1, v2, 0.0).expect("should hit");
        let [t, u, v] = hit;
        assert!((t - 1.0).abs() < 1e-5, "t = {t}");
        assert!((u - 0.25).abs() < 1e-5, "u = {u}");
        assert!((v - 0.25).abs() < 1e-5, "v = {v}");
    }

    #[test]
    fn reconstructs_intersection_point() {
        // For any hit, o + t*d must equal v0 + u*edge1 + v*edge2.
        let v0 = [0.0_f32, 0.0, 0.0];
        let v1 = [2.0_f32, 0.0, 0.0];
        let v2 = [0.0_f32, 3.0, 0.0];
        let o = [0.5_f32, 0.5, 2.0];
        let d = [0.0_f32, 0.0, -4.0];
        let [t, u, v] = reference(o, d, v0, v1, v2, 0.0).expect("hit");
        for k in 0..3 {
            let p_ray = o[k] + t * d[k];
            let e1 = v1[k] - v0[k];
            let e2 = v2[k] - v0[k];
            let p_bary = v0[k] + u * e1 + v * e2;
            assert!(
                (p_ray - p_bary).abs() < 1e-4,
                "axis {k}: {p_ray} vs {p_bary}"
            );
        }
    }

    #[test]
    fn misses_when_outside_triangle() {
        let v0 = [0.0_f32, 0.0, 0.0];
        let v1 = [1.0_f32, 0.0, 0.0];
        let v2 = [0.0_f32, 1.0, 0.0];
        let o = [5.0_f32, 5.0, 1.0];
        let d = [0.0_f32, 0.0, -1.0];
        assert!(reference(o, d, v0, v1, v2, 0.0).is_none());
    }

    #[test]
    fn misses_parallel_ray() {
        // Ray parallel to the triangle plane ⇒ degenerate determinant ⇒ miss.
        let v0 = [0.0_f32, 0.0, 0.0];
        let v1 = [1.0_f32, 0.0, 0.0];
        let v2 = [0.0_f32, 1.0, 0.0];
        let o = [0.25_f32, 0.25, 1.0];
        let d = [1.0_f32, 0.0, 0.0];
        assert!(reference(o, d, v0, v1, v2, 0.0).is_none());
    }

    #[test]
    fn t_parameter_is_not_clamped_to_segment() {
        // The acceptance band gates the barycentrics (u and u+v), NOT the ray
        // parameter t. A through-the-interior ray whose intersection lies far
        // past the unit segment still reports a hit with the full t (= 5 here).
        let v0 = [0.0_f32, 0.0, 0.0];
        let v1 = [1.0_f32, 0.0, 0.0];
        let v2 = [0.0_f32, 1.0, 0.0];
        let o = [0.25_f32, 0.25, 5.0];
        let d = [0.0_f32, 0.0, -1.0];
        let [t, u, v] = reference(o, d, v0, v1, v2, 0.0).expect("interior hit");
        assert!((t - 5.0).abs() < 1e-4, "t = {t}");
        assert!((u - 0.25).abs() < 1e-5, "u = {u}");
        assert!((v - 0.25).abs() < 1e-5, "v = {v}");
    }

    #[test]
    fn edge_tolerance_admits_near_miss() {
        // Barycentric just past the edge (u ≈ -0.05): rejected at tol = 0,
        // accepted at tol = 0.1.
        let v0 = [0.0_f32, 0.0, 0.0];
        let v1 = [1.0_f32, 0.0, 0.0];
        let v2 = [0.0_f32, 1.0, 0.0];
        let o = [-0.05_f32, 0.25, 1.0];
        let d = [0.0_f32, 0.0, -1.0];
        assert!(reference(o, d, v0, v1, v2, 0.0).is_none());
        assert!(reference(o, d, v0, v1, v2, 0.1).is_some());
    }

    #[test]
    fn hits_through_back_face() {
        // The determinant test only excludes the near-zero band, not sign, so a
        // ray crossing the plane from the other side still hits.
        let v0 = [0.0_f32, 0.0, 0.0];
        let v1 = [1.0_f32, 0.0, 0.0];
        let v2 = [0.0_f32, 1.0, 0.0];
        let o = [0.25_f32, 0.25, -1.0];
        let d = [0.0_f32, 0.0, 1.0];
        let [t, _u, _v] = reference(o, d, v0, v1, v2, 0.0).expect("back-face hit");
        assert!((t - 1.0).abs() < 1e-5, "t = {t}");
    }
}

/// Möller-Trumbore line-segment vs triangle intersection (non-culling).
///
/// `segment` is a `C3Ray`: `[ox, oy, oz, dx, dy, dz]` (origin then direction).
/// `v0`/`v1`/`v2` are the triangle vertices already gathered from the vertex
/// array. The barycentric `u`,`v` and ray parameter `t` are accepted within the
/// band `[-edge_tol, 1 + edge_tol]` (the reference's `±edge_tol` slack on a unit
/// triangle/ray). Returns `Some((t, u, v))` on a hit, `None` on a miss. A
/// near-parallel ray (`|det| < 1e-6`) is rejected. Computed in `f32`; the
/// original is x87 throughout and compares each band against an accumulator it
/// never rounded to `f32`, so a value sitting exactly on a band edge can
/// classify differently here.
pub fn collide_line_triangle_indexed32__7c2c40(
    segment: &[f32; 6],
    v0: &[f32; 3],
    v1: &[f32; 3],
    v2: &[f32; 3],
    edge_tol: f32,
) -> Option<(f32, f32, f32)> {
    const EPS: f32 = 1.0e-6;
    let t_min = -edge_tol;
    let t_max = edge_tol + 1.0;

    let dir = [segment[3], segment[4], segment[5]];

    let edge1 = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
    let edge2 = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];

    // pvec = cross(dir, edge2)
    let pvec = [
        dir[1] * edge2[2] - dir[2] * edge2[1],
        dir[2] * edge2[0] - dir[0] * edge2[2],
        dir[0] * edge2[1] - dir[1] * edge2[0],
    ];

    // det = dot(pvec, edge1)
    let det = pvec[0] * edge1[0] + pvec[1] * edge1[1] + pvec[2] * edge1[2];

    // Reject near-parallel: keep only |det| >= EPS (two-sided, non-culling).
    if det > -EPS && det < EPS {
        return None;
    }
    let inv_det = 1.0 / det;

    // tvec = origin - v0
    let tvec = [segment[0] - v0[0], segment[1] - v0[1], segment[2] - v0[2]];

    // u = dot(tvec, pvec) * inv_det
    let u = (tvec[0] * pvec[0] + tvec[1] * pvec[1] + tvec[2] * pvec[2]) * inv_det;
    // Negative predicate, so a miss needs an ordered compare. The original's
    // band tests are the unordered-tolerant form — `FCOM` then `FNSTSW AX` with
    // `TEST AH,5`/`JNP` for the lower bound and `TEST AH,0x41`/`JZ` for the
    // upper — and an unordered result sets both tested bits, so a NaN `u` takes
    // neither miss branch and reaches the hit epilogue with NaN outputs. A
    // positive `u >= t_min && u <= t_max` would reject NaN instead.
    if u < t_min || u > t_max {
        return None;
    }

    // qvec = cross(tvec, edge1)
    let qvec = [
        tvec[1] * edge1[2] - tvec[2] * edge1[1],
        tvec[2] * edge1[0] - tvec[0] * edge1[2],
        tvec[0] * edge1[1] - tvec[1] * edge1[0],
    ];

    // v = dot(dir, qvec) * inv_det
    let v = (dir[0] * qvec[0] + dir[1] * qvec[1] + dir[2] * qvec[2]) * inv_det;
    // Lower bound on `v`, upper bound on `u + v` (the original tests the sum
    // against `t_max`); negative predicates for the same fall-through reason as
    // the `u` band.
    if v < t_min || u + v > t_max {
        return None;
    }

    // t = dot(edge2, qvec) * inv_det
    let t = (qvec[0] * edge2[0] + qvec[1] * edge2[1] + qvec[2] * edge2[2]) * inv_det;

    Some((t, u, v))
}

#[cfg(test)]
mod tests_collide_line_triangle_indexed32__7c2c40 {
    use super::collide_line_triangle_indexed32__7c2c40 as f;

    // A unit right-triangle in the z=0 plane; a ray shot straight down -z from
    // above its centroid must hit at the known barycentrics.
    #[test]
    fn ray_hits_triangle_centroid() {
        let v0 = [0.0f32, 0.0, 0.0];
        let v1 = [1.0f32, 0.0, 0.0];
        let v2 = [0.0f32, 1.0, 0.0];
        // origin above (1/3,1/3,1), direction -z.
        let seg = [1.0 / 3.0f32, 1.0 / 3.0, 1.0, 0.0, 0.0, -1.0];
        let hit = f(&seg, &v0, &v1, &v2, 0.0).expect("should hit");
        let (t, u, v) = hit;
        assert!((u - 1.0 / 3.0).abs() < 1e-5, "u={u}");
        assert!((v - 1.0 / 3.0).abs() < 1e-5, "v={v}");
        assert!((t - 1.0).abs() < 1e-5, "t={t}");
    }

    // Reconstructing the hit point from barycentrics must reproduce the geometric
    // intersection point on the triangle plane.
    #[test]
    fn barycentric_point_matches_geometry() {
        let v0 = [0.0f32, 0.0, 0.0];
        let v1 = [2.0f32, 0.0, 0.0];
        let v2 = [0.0f32, 3.0, 0.0];
        // Aim at (0.5, 0.6, 0) coming down -z from z=5.
        let target = [0.5f32, 0.6, 0.0];
        let seg = [target[0], target[1], 5.0, 0.0, 0.0, -1.0];
        let (t, u, v) = f(&seg, &v0, &v1, &v2, 0.0).expect("should hit");
        // P = v0 + u*(v1-v0) + v*(v2-v0)
        let px = v0[0] + u * (v1[0] - v0[0]) + v * (v2[0] - v0[0]);
        let py = v0[1] + u * (v1[1] - v0[1]) + v * (v2[1] - v0[1]);
        assert!((px - target[0]).abs() < 1e-5, "px={px}");
        assert!((py - target[1]).abs() < 1e-5, "py={py}");
        // origin.z + t*dir.z = 5 + t*(-1) = 0 => t = 5
        assert!((t - 5.0).abs() < 1e-4, "t={t}");
    }

    /// Law: a NaN barycentric falls through the band tests and reports a hit.
    ///
    /// The original compares each band with `FCOM` + `FNSTSW AX` + a `TEST AH`
    /// mask, and an unordered result sets every bit those masks test, so neither
    /// miss branch is taken and the hit epilogue runs with NaN outputs. Pinned
    /// because the positive in-band predicate that would reject NaN reads more
    /// naturally and is the easy thing to reintroduce.
    #[test]
    fn nan_barycentric_reaches_the_hit_path() {
        let v0 = [f32::NAN, 0.0, 0.0];
        let v1 = [1.0f32, 0.0, 0.0];
        let v2 = [0.0f32, 1.0, 0.0];
        let seg = [1.0 / 3.0f32, 1.0 / 3.0, 1.0, 0.0, 0.0, -1.0];
        let (t, u, v) = f(&seg, &v0, &v1, &v2, 0.0).expect("NaN must not miss");
        assert!(u.is_nan(), "u={u}");
        assert!(v.is_nan() || t.is_nan(), "v={v} t={t}");
    }

    // A ray parallel to the triangle plane has near-zero determinant => miss.
    #[test]
    fn parallel_ray_misses() {
        let v0 = [0.0f32, 0.0, 0.0];
        let v1 = [1.0f32, 0.0, 0.0];
        let v2 = [0.0f32, 1.0, 0.0];
        // Direction +x lies in the z=0 plane => parallel.
        let seg = [-1.0f32, 0.25, 0.0, 1.0, 0.0, 0.0];
        assert!(f(&seg, &v0, &v1, &v2, 0.0).is_none());
    }

    // A ray aimed outside the triangle (u+v > 1) misses with zero tolerance.
    #[test]
    fn outside_triangle_misses() {
        let v0 = [0.0f32, 0.0, 0.0];
        let v1 = [1.0f32, 0.0, 0.0];
        let v2 = [0.0f32, 1.0, 0.0];
        // (0.9, 0.9) is well outside (u+v=1.8 > 1).
        let seg = [0.9f32, 0.9, 1.0, 0.0, 0.0, -1.0];
        assert!(f(&seg, &v0, &v1, &v2, 0.0).is_none());
    }

    // A slightly-outside aim hits only when edge_tol admits it.
    #[test]
    fn edge_tolerance_admits_slightly_outside() {
        let v0 = [0.0f32, 0.0, 0.0];
        let v1 = [1.0f32, 0.0, 0.0];
        let v2 = [0.0f32, 1.0, 0.0];
        // u = -0.05 (just left of edge v0->v2); needs edge_tol >= 0.05.
        let seg = [-0.05f32, 0.5, 1.0, 0.0, 0.0, -1.0];
        assert!(f(&seg, &v0, &v1, &v2, 0.0).is_none(), "tol 0 should reject");
        let hit = f(&seg, &v0, &v1, &v2, 0.1);
        assert!(hit.is_some(), "tol 0.1 should admit");
        let (_, u, _) = hit.unwrap();
        assert!((u + 0.05).abs() < 1e-5, "u={u}");
    }

    // Reversing the ray direction flips the sign of t (plane behind the origin).
    #[test]
    fn reversed_direction_flips_t_sign() {
        let v0 = [0.0f32, 0.0, 0.0];
        let v1 = [1.0f32, 0.0, 0.0];
        let v2 = [0.0f32, 1.0, 0.0];
        let origin = [0.25f32, 0.25, 2.0];
        let fwd = [origin[0], origin[1], origin[2], 0.0, 0.0, -1.0];
        let bwd = [origin[0], origin[1], origin[2], 0.0, 0.0, 1.0];
        let (tf, _, _) = f(&fwd, &v0, &v1, &v2, 0.0).expect("fwd hits");
        assert!(tf > 0.0, "tf={tf}");
        let (tb, _, _) = f(&bwd, &v0, &v1, &v2, 0.0).expect("bwd hits plane behind");
        assert!((tf + tb).abs() < 1e-4, "tf={tf} tb={tb}");
    }
}

/// Möller–Trumbore line-segment vs triangle intersection.
///
/// `segment` is a ray packed as origin in `[0..3]` and direction in `[3..6]`;
/// `tri` holds the three triangle vertices contiguously (`v0`,`v1`,`v2`). The
/// barycentric `u`,`v` and the parameter `t` are accepted inside a band widened
/// by `edge_tol`: `u`,`v` ≥ `-edge_tol`, `u`,`v`,`u+v` ≤ `1 + edge_tol`. The
/// determinant test is two-sided (`|det| ≥ 1e-6`), so front and back faces both
/// register. Returns `Some([t, u, v])` on a hit and `None` on a miss, matching
/// the reference's boolean result; `t`,`u`,`v` are the values the original
/// writes to its `outT`/`outUV` out-parameters.
// Ordered-negative band tests, as in the reference — `v < lo`, `u + v > hi` —
// so a miss needs an ordered compare and a NaN barycentric reaches the hit path.
// The near-parallel gate below needs the same shape for the same reason: the
// original's first compare branches to the continue path on unordered, so a NaN
// determinant is not treated as near-parallel.
pub fn collide_line_triangle__7c27d0(
    segment: &[f32; 6],
    tri: &[f32; 9],
    edge_tol: f32,
) -> Option<[f32; 3]> {
    // The two fixed `.data` thresholds from the host image: `1.0` and `1e-6`.
    const ONE: f32 = 1.0;
    const EPS: f32 = 1.0e-6;

    let t_min = -edge_tol;
    let t_max = edge_tol + ONE;

    let dir = [segment[3], segment[4], segment[5]];

    // Triangle edges from `v0`.
    let e1 = [tri[3] - tri[0], tri[4] - tri[1], tri[5] - tri[2]];
    let e2 = [tri[6] - tri[0], tri[7] - tri[1], tri[8] - tri[2]];

    // pvec = cross(dir, e2).
    let pvec = [
        e2[2] * dir[1] - e2[1] * dir[2],
        e2[0] * dir[2] - e2[2] * dir[0],
        e2[1] * dir[0] - e2[0] * dir[1],
    ];

    // det = dot(e1, pvec). Two-sided: reject only the parallel band |det| < EPS.
    let det = pvec[0] * e1[0] + pvec[1] * e1[1] + pvec[2] * e1[2];
    if det > -EPS && det < EPS {
        return None;
    }
    let inv_det = ONE / det;

    // tvec = origin - v0.
    let tvec = [
        segment[0] - tri[0],
        segment[1] - tri[1],
        segment[2] - tri[2],
    ];

    // u = dot(tvec, pvec) * inv_det.
    let u = (tvec[0] * pvec[0] + tvec[1] * pvec[1] + tvec[2] * pvec[2]) * inv_det;
    if u < t_min || u > t_max {
        return None;
    }

    // qvec = cross(tvec, e1).
    let qvec = [
        tvec[1] * e1[2] - tvec[2] * e1[1],
        tvec[2] * e1[0] - tvec[0] * e1[2],
        tvec[0] * e1[1] - tvec[1] * e1[0],
    ];

    // v = dot(dir, qvec) * inv_det.
    let v = (qvec[0] * dir[0] + qvec[1] * dir[1] + qvec[2] * dir[2]) * inv_det;
    if v < t_min {
        return None;
    }
    if u + v > t_max {
        return None;
    }

    // t = dot(qvec, e2) * inv_det.
    let t = (qvec[0] * e2[0] + qvec[1] * e2[1] + qvec[2] * e2[2]) * inv_det;
    Some([t, u, v])
}

#[cfg(test)]
mod tests_collide_line_triangle__7c27d0 {
    use super::collide_line_triangle__7c27d0;

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    // Unit triangle in the z=0 plane: v0=(0,0,0), v1=(1,0,0), v2=(0,1,0).
    const TRI: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];

    /// Law: a NaN barycentric or determinant reaches the hit path, not a miss.
    ///
    /// Every band compare in the original is `FCOM` + `FNSTSW AX` + a `TEST AH`
    /// mask whose bits an unordered result all sets, so neither miss branch is
    /// taken; the near-parallel gate likewise branches to the continue path on
    /// unordered. Pinned because the positive in-band predicate that would
    /// reject NaN reads more naturally and is the easy thing to reintroduce.
    #[test]
    fn nan_reaches_the_hit_path() {
        const NAN_TRI: [f32; 9] = [f32::NAN, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let seg = [0.25, 0.25, 1.0, 0.0, 0.0, -1.0];
        let r = collide_line_triangle__7c27d0(&seg, &NAN_TRI, 0.0).expect("NaN must not miss");
        assert!(r.iter().any(|c| c.is_nan()), "{r:?}");
    }

    #[test]
    fn hits_center_known_values() {
        // Ray from (0.25,0.25,1) heading -z hits at t=1, u=0.25, v=0.25.
        let seg = [0.25, 0.25, 1.0, 0.0, 0.0, -1.0];
        let r = collide_line_triangle__7c27d0(&seg, &TRI, 0.0).expect("center hit");
        assert!(close(r[0], 1.0, 1e-5), "t={}", r[0]);
        assert!(close(r[1], 0.25, 1e-5), "u={}", r[1]);
        assert!(close(r[2], 0.25, 1e-5), "v={}", r[2]);
    }

    #[test]
    fn point_inside_iff_bary_valid() {
        // For an unscaled -z ray that lands at (x,y,1), u==x and v==y exactly
        // (the intersection's barycentric coords equal the planar position).
        let inside = [(0.1f32, 0.1f32), (0.4, 0.4), (0.05, 0.9), (0.49, 0.49)];
        for (x, y) in inside {
            let seg = [x, y, 1.0, 0.0, 0.0, -1.0];
            let r = collide_line_triangle__7c27d0(&seg, &TRI, 0.0)
                .unwrap_or_else(|| panic!("expected hit at ({x},{y})"));
            assert!(close(r[1], x, 1e-6), "u={} x={}", r[1], x);
            assert!(close(r[2], y, 1e-6), "v={} y={}", r[2], y);
            // Barycentric validity: u>=0, v>=0, u+v<=1.
            assert!(r[1] >= -1e-6 && r[2] >= -1e-6 && r[1] + r[2] <= 1.0 + 1e-6);
        }
    }

    #[test]
    fn outside_triangle_misses() {
        // Lands outside the unit triangle (u+v > 1).
        let outside = [(0.8f32, 0.8f32), (1.5, 0.0), (-0.2, 0.3), (0.0, 1.4)];
        for (x, y) in outside {
            let seg = [x, y, 1.0, 0.0, 0.0, -1.0];
            assert!(
                collide_line_triangle__7c27d0(&seg, &TRI, 0.0).is_none(),
                "({x},{y}) should miss"
            );
        }
    }

    #[test]
    fn edge_tol_widens_acceptance() {
        // Just outside (u+v=1.1): a miss at tol=0 becomes a hit at tol=0.2.
        let seg = [0.55, 0.55, 1.0, 0.0, 0.0, -1.0];
        assert!(collide_line_triangle__7c27d0(&seg, &TRI, 0.0).is_none());
        assert!(collide_line_triangle__7c27d0(&seg, &TRI, 0.2).is_some());
    }

    #[test]
    fn parallel_ray_is_culled() {
        // Direction lies in the triangle plane: det ~ 0, must be rejected.
        let seg = [0.25, 0.25, 1.0, 1.0, 0.0, 0.0];
        assert!(collide_line_triangle__7c27d0(&seg, &TRI, 0.0).is_none());
    }

    #[test]
    fn two_sided_backface_hits() {
        // Same target from below (+z direction): two-sided test still hits.
        let seg = [0.25, 0.25, -1.0, 0.0, 0.0, 1.0];
        let r = collide_line_triangle__7c27d0(&seg, &TRI, 0.0).expect("backface hit");
        assert!(close(r[1], 0.25, 1e-5) && close(r[2], 0.25, 1e-5));
        // t is positive distance along the (downward-from-below) ray to z=0.
        assert!(close(r[0], 1.0, 1e-5), "t={}", r[0]);
    }

    #[test]
    fn translation_invariance() {
        // Translating ray origin and all triangle verts by the same offset
        // leaves the barycentric u,v (and the hit/miss verdict) unchanged.
        let seg = [0.3, 0.2, 1.0, 0.0, 0.0, -1.0];
        let base = collide_line_triangle__7c27d0(&seg, &TRI, 0.0).expect("hit");
        let off = [10.0f32, -7.0, 3.0];
        let seg2 = [
            seg[0] + off[0],
            seg[1] + off[1],
            seg[2] + off[2],
            seg[3],
            seg[4],
            seg[5],
        ];
        let mut tri2 = TRI;
        for i in 0..3 {
            tri2[i * 3] += off[0];
            tri2[i * 3 + 1] += off[1];
            tri2[i * 3 + 2] += off[2];
        }
        let moved = collide_line_triangle__7c27d0(&seg2, &tri2, 0.0).expect("hit");
        assert!(
            close(moved[1], base[1], 1e-5),
            "u {} vs {}",
            moved[1],
            base[1]
        );
        assert!(
            close(moved[2], base[2], 1e-5),
            "v {} vs {}",
            moved[2],
            base[2]
        );
    }
}

/// Builds the 9 clip-plane equations (`Ax+By+Cz+D`, 36 contiguous `f32`).
///
/// Bounds the swept-collision prism around `center` with horizontal `radius`
/// and vertical `height`.
///
/// Plane layout (4 floats each `[A, B, C, D]`): four axis-aligned side
/// planes (-X/+X/+Y/-Y), one top cap (+Z), then four angled side planes
/// whose normals use the fixed cos/sin pair (≈0.8796419 / 0.4756366) that
/// rounds the box toward an octagonal bound. The `D` terms offset each plane
/// by `center` and `radius`/`height`.
pub fn collision_build_prism_planes__631440(
    center: &[f32; 3],
    height: f32,
    radius: f32,
) -> [f32; 36] {
    // Angled-side normal components: the X/Y lobe and the shared -Z tilt.
    // These are the `.data` constants the original folds in (0x3f613036,
    // 0x3ef386a4); the third multiplier (0x007ffd74) is 0.0.
    const C: f32 = 0.879_641_9; // 0x3f613036
    const S: f32 = 0.475_636_6; // 0x3ef386a4

    let cx = center[0];
    let cy = center[1];
    let cz = center[2];

    [
        // -X side plane: D = cx - radius
        -1.0,
        0.0,
        0.0,
        cx - radius,
        // +X side plane: D = -cx - radius
        1.0,
        0.0,
        0.0,
        -cx - radius,
        // +Y side plane: D = -cy - radius
        0.0,
        1.0,
        0.0,
        -cy - radius,
        // -Y side plane: D = cy - radius
        0.0,
        -1.0,
        0.0,
        cy - radius,
        // +Z top cap: D = -cz - height
        0.0,
        0.0,
        1.0,
        -cz - height,
        // angled (-C, 0, -S): D = C*cx + S*cz
        -C,
        0.0,
        -S,
        C * cx + S * cz,
        // angled (+C, 0, -S): D = -C*cx + S*cz
        C,
        0.0,
        -S,
        S * cz - C * cx,
        // angled (0, +C, -S): D = S*cz - C*cy
        0.0,
        C,
        -S,
        S * cz - C * cy,
        // angled (0, -C, -S): D = C*cy + S*cz
        0.0,
        -C,
        -S,
        C * cy + S * cz,
    ]
}

#[cfg(test)]
mod tests_collision_build_prism_planes__631440 {
    use super::collision_build_prism_planes__631440 as f;

    const C: f32 = 0.879_641_9;
    const S: f32 = 0.475_636_6;

    #[test]
    fn axis_aligned_normals_are_unit() {
        let p = f(&[0.0, 0.0, 0.0], 5.0, 2.0);
        let n: [[f32; 3]; 5] = [
            [p[0], p[1], p[2]],
            [p[4], p[5], p[6]],
            [p[8], p[9], p[10]],
            [p[12], p[13], p[14]],
            [p[16], p[17], p[18]],
        ];
        for v in n {
            let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-6, "{v:?} len {len}");
        }
    }

    #[test]
    fn angled_normals_are_unit() {
        let p = f(&[1.0, 2.0, 3.0], 4.0, 1.5);
        for base in [20, 24, 28, 32] {
            let v = [p[base], p[base + 1], p[base + 2]];
            let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-4, "plane@{base} {v:?} len {len}");
        }
    }

    #[test]
    fn side_plane_d_terms_known() {
        let cx = 3.0_f32;
        let cy = -4.0_f32;
        let cz = 7.0_f32;
        let r = 2.0_f32;
        let h = 5.0_f32;
        let p = f(&[cx, cy, cz], h, r);
        assert_eq!(p[3], cx - r);
        assert_eq!(p[7], -cx - r);
        assert_eq!(p[11], -cy - r);
        assert_eq!(p[15], cy - r);
        assert_eq!(p[19], -cz - h);
    }

    #[test]
    fn angled_plane_d_terms_known() {
        let cx = 3.0_f32;
        let cy = -4.0_f32;
        let cz = 7.0_f32;
        let p = f(&[cx, cy, cz], 5.0, 2.0);
        assert!((p[23] - (C * cx + S * cz)).abs() < 1e-5);
        assert!((p[27] - (S * cz - C * cx)).abs() < 1e-5);
        assert!((p[31] - (S * cz - C * cy)).abs() < 1e-5);
        assert!((p[35] - (C * cy + S * cz)).abs() < 1e-5);
    }

    #[test]
    fn opposite_side_normals_negate() {
        let p = f(&[0.5, 1.0, -2.0], 3.0, 1.0);
        assert_eq!([p[0], p[1], p[2]], [-p[4], -p[5], -p[6]]);
        assert_eq!([p[8], p[9], p[10]], [-p[12], -p[13], -p[14]]);
    }

    #[test]
    fn center_origin_radius_zero_known() {
        let p = f(&[0.0, 0.0, 0.0], 0.0, 0.0);
        assert_eq!(p[3], 0.0);
        assert_eq!(p[7], 0.0);
        assert_eq!(p[11], 0.0);
        assert_eq!(p[15], 0.0);
        assert_eq!(p[19], 0.0);
    }
}

/// Builds the 5 clip planes (4 swept-edge side planes + 1 cap plane).
///
/// The face is a quad swept along `face_offset`.
///
/// `verts` are the quad's 4 corner positions in ring order. For each ring edge
/// `i` the swept side plane is the triangle plane through `A`, `B`, `A+offset`
/// (A = verts[i], B = verts[(i+1)%4]), oriented so the opposite (previous-ring)
/// vertex lies on the negative side. The cap plane uses `cap_normal` and passes
/// through `verts[0]+face_offset`. Each plane is `[Nx,Ny,Nz,D]` with unit normal
/// and `D = -dot(N,point)`.
///
/// Returns the planes and how many leading entries the original had written when
/// it returned: 5 on success, and on a degenerate edge (swept area below `2^-20`)
/// the number of side planes already finished. The original writes each side plane
/// straight through its out pointer during that edge's iteration (`ECX = ESI`,
/// `ESI += 0x10`), so the ones before the bail are left behind and only the cap is
/// skipped.
///
/// Widths and orders, off the bytes rather than the algebra. The two edge vectors
/// and the three cross components each go through an `f32` stack slot on their way
/// into `0x4549a0`, so they narrow; but the cross components are formed wide from
/// exact `f32 * f32` products and narrow only at that store, the squared area is
/// summed wide from the narrowed components in the order `(y² + z²) + x²`, the
/// orientation dot runs `(z + x) + y` wide with no store, and the cap plane's
/// point is never stored at all — `0x6331a1`..`0x6331b1` leaves all three sums on
/// the stack and its `D` is built from them wide, dotted `(z + y) + x`.
pub fn collision_build_quad_face_clip_planes__632f80(
    verts: &[[f32; 3]; 4],
    cap_normal: &[f32; 3],
    face_offset: &[f32; 3],
) -> ([[f32; 4]; 5], usize) {
    // Degenerate-area epsilon at `0x8026bc` (2^-20) and orientation threshold
    // at `0x7ffd74` (0.0), folded in as binary-exact literals.
    // The digit string is what makes the literal land on `0x35800000`; a shorter
    // one is a different f32 and a different degeneracy threshold.
    #[allow(clippy::excessive_precision)]
    const AREA_EPS: f32 = 9.536_743_16e-7; // 0x35800000
    const FLIP_THRESH: f32 = 0.0;

    let mut planes = [[0.0f32; 4]; 5];

    for i in 0..4 {
        let a = verts[i];
        let b = verts[(i + 1) & 3];
        let p = verts[(i + 3) & 3]; // (i - 1) mod 4 = previous ring vertex
        let asw = [
            a[0] + face_offset[0],
            a[1] + face_offset[1],
            a[2] + face_offset[2],
        ];

        // e1 = Asw - A (= face_offset), e2 = B - A; degenerate test on |e1 x e2|^2.
        let e1 = [asw[0] - a[0], asw[1] - a[1], asw[2] - a[2]];
        let e2 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let cross = |u: f32, v: f32, w: f32, t: f32| {
            super::f64_to_f32(f64::from(u) * f64::from(v) - f64::from(w) * f64::from(t))
        };
        let cx = cross(e1[2], e2[1], e1[1], e2[2]);
        let cy = cross(e2[2], e1[0], e1[2], e2[0]);
        let cz = cross(e1[1], e2[0], e2[1], e1[0]);
        let sq = |v: f32| f64::from(v) * f64::from(v);
        let area_sq = (sq(cy) + sq(cz)) + sq(cx);
        if area_sq.abs() < f64::from(AREA_EPS) {
            return (planes, i);
        }

        // Plane from triangle (A, B, Asw), the original's own `0x637480` call.
        let mut plane = super::plane::c4_plane__from_triangle__637480(&a, &b, &asw);

        // Orient: flip if the previous-ring vertex is on the positive side.
        let d = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
        let term = |k: usize| f64::from(d[k]) * f64::from(plane[k]);
        let side = (term(2) + term(0)) + term(1);
        if side > f64::from(FLIP_THRESH) {
            plane = [-plane[0], -plane[1], -plane[2], -plane[3]];
        }
        planes[i] = plane;
    }

    // Cap plane: cap_normal through verts[0] + face_offset, that point kept wide.
    let v0 = verts[0];
    let cp = [
        f64::from(v0[0]) + f64::from(face_offset[0]),
        f64::from(v0[1]) + f64::from(face_offset[1]),
        f64::from(v0[2]) + f64::from(face_offset[2]),
    ];
    let term = |k: usize| cp[k] * f64::from(cap_normal[k]);
    let d = super::f64_to_f32(-((term(2) + term(1)) + term(0)));
    planes[4] = [cap_normal[0], cap_normal[1], cap_normal[2], d];

    (planes, 5)
}

#[cfg(test)]
mod tests_collision_build_quad_face_clip_planes__632f80 {
    use super::collision_build_quad_face_clip_planes__632f80;

    /// The five planes, asserting the original ran to completion.
    fn all_five(verts: &[[f32; 3]; 4], cap: &[f32; 3], off: &[f32; 3]) -> [[f32; 4]; 5] {
        let (planes, written) = collision_build_quad_face_clip_planes__632f80(verts, cap, off);
        assert_eq!(written, 5, "expected all five planes");
        planes
    }

    fn dot4_point(plane: &[f32; 4], p: &[f32; 3]) -> f32 {
        plane[0] * p[0] + plane[1] * p[1] + plane[2] * p[2] + plane[3]
    }

    // A unit axis-aligned square in the z=0 plane, swept upward (+z).
    fn unit_quad() -> [[f32; 3]; 4] {
        [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ]
    }

    #[test]
    fn side_planes_are_unit_normalized() {
        let q = unit_quad();
        let cap = [0.0, 0.0, 1.0];
        let off = [0.0, 0.0, 2.0];
        let planes = all_five(&q, &cap, &off);
        for plane in planes.iter().take(4) {
            let len = (plane[0] * plane[0] + plane[1] * plane[1] + plane[2] * plane[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-5, "normal not unit: {len}");
        }
    }

    #[test]
    fn cap_plane_passes_through_swept_v0_with_cap_normal() {
        let q = unit_quad();
        let cap = [0.0, 0.0, 1.0];
        let off = [0.0, 0.0, 3.0];
        let planes = all_five(&q, &cap, &off);
        let capp = planes[4];
        assert_eq!(capp[0].to_bits(), cap[0].to_bits());
        assert_eq!(capp[1].to_bits(), cap[1].to_bits());
        assert_eq!(capp[2].to_bits(), cap[2].to_bits());
        // verts[0]+off = (0,0,3) lies on the cap plane.
        let swept_v0 = [q[0][0] + off[0], q[0][1] + off[1], q[0][2] + off[2]];
        assert!(dot4_point(&capp, &swept_v0).abs() < 1e-5);
    }

    #[test]
    fn side_planes_orient_inward_face_interior_negative() {
        // The 4 base corners (and their swept tops) must lie on the negative
        // (inside) side of every side plane: planes face outward.
        let q = unit_quad();
        let cap = [0.0, 0.0, 1.0];
        let off = [0.0, 0.0, 2.0];
        let planes = all_five(&q, &cap, &off);
        // Centroid of the swept box interior.
        let center = [0.5, 0.5, 1.0];
        for plane in planes.iter().take(4) {
            assert!(
                dot4_point(plane, &center) < 0.0,
                "center on wrong side: {}",
                dot4_point(plane, &center)
            );
        }
    }

    #[test]
    fn each_side_plane_contains_its_edge_endpoints() {
        let q = unit_quad();
        let cap = [0.0, 0.0, 1.0];
        let off = [0.0, 0.0, 2.0];
        let planes = all_five(&q, &cap, &off);
        for i in 0..4 {
            let a = q[i];
            let b = q[(i + 1) & 3];
            let asw = [a[0] + off[0], a[1] + off[1], a[2] + off[2]];
            for v in [a, b, asw] {
                assert!(
                    dot4_point(&planes[i], &v).abs() < 1e-4,
                    "edge endpoint off plane {i}: {}",
                    dot4_point(&planes[i], &v)
                );
            }
        }
    }

    #[test]
    fn known_value_first_side_plane() {
        // Edge 0: A=(0,0,0) B=(1,0,0), swept +z. Triangle (A,B,Asw=(0,0,2)).
        // (B-A)=(1,0,0), (Asw-A)=(0,0,2); cross=(0,-2,0); normalized=(0,-1,0).
        // Previous vertex P=verts[3]=(0,1,0); d=P-A=(0,1,0); dot(d,N)=-1 < 0 => no
        // flip. So plane normal=(0,-1,0), D=-dot(N,A)=0.
        let q = unit_quad();
        let cap = [0.0, 0.0, 1.0];
        let off = [0.0, 0.0, 2.0];
        let planes = all_five(&q, &cap, &off);
        let p0 = planes[0];
        assert!((p0[0] - 0.0).abs() < 1e-6);
        assert!((p0[1] - (-1.0)).abs() < 1e-6);
        assert!((p0[2] - 0.0).abs() < 1e-6);
        assert!(p0[3].abs() < 1e-6);
    }

    #[test]
    fn degenerate_zero_offset_returns_none() {
        // Zero sweep => e1=0 => cross=0 => area below epsilon => None.
        let q = unit_quad();
        let cap = [0.0, 0.0, 1.0];
        let off = [0.0, 0.0, 0.0];
        assert_eq!(
            collision_build_quad_face_clip_planes__632f80(&q, &cap, &off).1,
            0
        );
    }

    /// A degenerate edge leaves the side planes finished before it.
    ///
    /// The original writes each side plane straight through its out pointer during
    /// that edge's own iteration, so a quad whose edge 1 collapses still reports
    /// plane 0. Returning nothing would be a different observable region.
    #[test]
    fn a_late_degenerate_edge_keeps_the_planes_before_it() {
        // Edge 0 spans (0,0,0)-(1,0,0); edges 1 and 2 collapse because verts 1, 2
        // and 3 coincide, and the sweep is along +z so edge 0's area is fine.
        let q = [
            [0.0f32, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
        ];
        let (planes, written) =
            collision_build_quad_face_clip_planes__632f80(&q, &[0.0, 0.0, 1.0], &[0.0, 0.0, 2.0]);
        assert_eq!(written, 1);
        assert!(planes[0].iter().any(|v| *v != 0.0), "plane 0 not written");
        assert_eq!(planes[1], [0.0; 4]);
    }

    /// One sweep case: the quad, the cap normal, the sweep offset.
    type Case = ([[f32; 3]; 4], [f32; 3], [f32; 3]);

    /// Quads, cap normals and offsets that separate the shape below.
    fn sweep() -> [Case; 48] {
        // Sign and all 23 mantissa bits kept, exponent forced: values in +/-[1, 2).
        // Mapping [1, 2) onto [-1, 1) by `v * 2 - 3` would leave every value a
        // multiple of 2^-22, and sums of those are exact in f32 - which is enough
        // to stop several of the shapes below from separating at all.
        let spread = |bits: u32| f32::from_bits((bits & 0x807f_ffff) | 0x3f80_0000);
        let mut out = [([[0.0f32; 3]; 4], [0.0f32; 3], [0.0f32; 3]); 48];
        let mut w = 0x2f5c_71d3_u32;
        let mut next = move || {
            w = w.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            w
        };
        for (quad, cap, off) in &mut out {
            for vertex in quad.iter_mut() {
                for component in vertex.iter_mut() {
                    *component = spread(next());
                }
            }
            for component in cap.iter_mut() {
                *component = spread(next());
            }
            for component in off.iter_mut() {
                *component = spread(next());
            }
        }
        out
    }

    /// The cap plane's `D` with its point narrowed to `f32` before the dot.
    ///
    /// The one shape fact in this function a test can see, and that scope is
    /// deliberate. Of everything here, the side planes come from `0x637480`
    /// (pinned in `plane.rs`) and `D` is the only number this body forms itself.
    /// The cross components, the squared area and the orientation dot are each
    /// consumed by a single comparison and never stored, so they can only be
    /// observed when they move a decision, and they do not: 200k quads including
    /// ones deliberately squashed towards coplanar produced no sign disagreement
    /// between the two orientation-dot groupings.
    ///
    /// The cap dot's own order is unobservable for the same reason a reassociation
    /// usually is. Regrouping three `f64` products perturbs the sum by about
    /// 2^-53 relative, and the `f32` store that follows is granular to 2^-24, so
    /// it survives only under heavy cancellation: 0 of 2M random cases. Narrowing
    /// the point instead perturbs it by 2^-24, which lands 35% of the time. Order
    /// is fixed here because it is what the bytes do; width is fixed *and* pinned.
    fn narrowed_cap_point(
        verts: &[[f32; 3]; 4],
        cap_normal: &[f32; 3],
        face_offset: &[f32; 3],
    ) -> f32 {
        let v0 = verts[0];
        let cp = [
            f64::from(v0[0] + face_offset[0]),
            f64::from(v0[1] + face_offset[1]),
            f64::from(v0[2] + face_offset[2]),
        ];
        let term = |k: usize| cp[k] * f64::from(cap_normal[k]);
        super::super::f64_to_f32(-((term(2) + term(1)) + term(0)))
    }

    #[test]
    fn separates_from_a_narrowed_cap_point() {
        let separating = sweep()
            .iter()
            .filter(|(quad, cap, off)| {
                let (faithful, written) =
                    collision_build_quad_face_clip_planes__632f80(quad, cap, off);
                written == 5
                    && faithful[4][3].to_bits() != narrowed_cap_point(quad, cap, off).to_bits()
            })
            .count();
        assert!(
            separating > 0,
            "sweep no longer separates the wide cap point from a narrowed one"
        );
    }
}

/// Parametric ray-vs-plane intersection time `t`.
///
/// `plane = [nx, ny, nz, d]`, `motion` = ray direction (with `motion[3]` an
/// extra additive bias on the numerator), `point` = ray origin. Computes
/// `num = dot(n, motion) + motion[3]` and `den = dot(n, point)`, returning
/// `num / den` (the `t` where the ray crosses the plane), or just `num` when
/// `|den|` is below the parallel-ray epsilon. The original accumulates on the
/// x87 stack and returns a `double`, so the ratio is formed in `f64`.
pub fn collision_ray_plane_intersect_time__6329e0(
    plane: &[f32; 4],
    motion: &[f32; 4],
    point: &[f32; 3],
) -> f64 {
    // 0x008029d4 — parallel-ray guard (2^-22).
    const EPS: f64 = 2.384_185_791_015_625e-7;

    let numerator = plane[0] * motion[0] + plane[1] * motion[1] + plane[2] * motion[2] + motion[3];
    let denominator = motion[0] * point[0] + point[1] * motion[1] + point[2] * motion[2];

    let num = f64::from(numerator);
    let den = f64::from(denominator);
    if den.abs() < EPS { num } else { num / den }
}

#[cfg(test)]
mod tests_collision_ray_plane_intersect_time__6329e0 {
    use super::collision_ray_plane_intersect_time__6329e0 as f;

    #[test]
    fn known_ratio() {
        // num = 1*1 + 0 + 0 + 0 = 1 ; den = 1*2 + 0 + 0 = 2 ; t = 0.5
        let t = f(
            &[1.0, 0.0, 0.0, 0.0],
            &[1.0, 0.0, 0.0, 0.0],
            &[2.0, 0.0, 0.0],
        );
        assert!((t - 0.5).abs() < 1e-12, "{t}");
    }

    #[test]
    fn numerator_bias_applies() {
        // num = (2*1) + motion[3] bias(3) = 5 ; den = dot(motion, point) = 1*2 = 2 ; t = 2.5
        let t = f(
            &[2.0, 0.0, 0.0, 0.0],
            &[1.0, 0.0, 0.0, 3.0],
            &[2.0, 0.0, 0.0],
        );
        assert!((t - 2.5).abs() < 1e-12, "{t}");
    }

    #[test]
    fn parallel_returns_numerator() {
        // den = dot(motion, point) = 0 (motion ⟂ point) ⇒ returns numerator.
        let num_only = f(
            &[1.0, 1.0, 1.0, 4.0],
            &[0.0, 1.0, 0.0, 4.0],
            &[5.0, 0.0, 7.0],
        );
        // numerator = (1*0 + 1*1 + 1*0) + 4 = 5
        assert!((num_only - 5.0).abs() < 1e-12, "{num_only}");
    }

    #[test]
    fn full_dot_products() {
        let plane = [0.5_f32, -1.0, 2.0, 0.25];
        let motion = [1.0_f32, 2.0, -1.0, 0.5];
        let point = [3.0_f32, 1.0, -2.0];
        let t = f(&plane, &motion, &point);
        // num = 0.5·1 − 1·2 − 2·1 + 0.5 = −3.0 ; den = 1·3 + 1·2 + 2·1 = 7.0
        let expected = -3.0 / 7.0;
        assert!((t - expected).abs() < 1e-6, "{t}");
    }

    #[test]
    fn scaling_den_inversely_scales_t() {
        let plane = [1.0_f32, 0.0, 0.0, 0.0];
        let p1 = [4.0_f32, 0.0, 0.0];
        let p2 = [8.0_f32, 0.0, 0.0];
        let t1 = f(&plane, &[1.0, 0.0, 0.0, 0.0], &p1);
        let t2 = f(&plane, &[1.0, 0.0, 0.0, 0.0], &p2);
        // den doubled ⇒ t halved.
        assert!((t1 - 2.0 * t2).abs() < 1e-12, "{t1} {t2}");
    }
}

/// One axis of the squared-threshold variant (`dir` 0).
///
/// Directional outside gap, far-wall gap, and the boundary-proximity latch
/// against `r2`.
///
/// Strict compares route an unordered (NaN) coordinate to the inside arm with
/// no gap contribution; min/max ties and unordered operands resolve to `g_hi`,
/// both matching the stock routine.
fn sphere_axis_terms_sq(s: f32, lo: f32, hi: f32, r2: f32) -> (f32, f32, bool) {
    let d_lo = s - lo;
    let g_lo = d_lo * d_lo;
    let d_hi = s - hi;
    let g_hi = d_hi * d_hi;
    let far = if g_lo > g_hi { g_lo } else { g_hi };
    if s < lo {
        (g_lo, far, true)
    } else if s > hi {
        (g_hi, far, true)
    } else {
        let near = if g_lo < g_hi { g_lo } else { g_hi };
        (0.0, far, near <= r2)
    }
}

/// One axis of the linear-threshold variant (`dir` 1).
///
/// Directional outside gap and the proximity latch against the *linear* radius
/// (distinct from the squared form when `radius` is negative).
fn sphere_axis_terms_lin(s: f32, lo: f32, hi: f32, radius: f32) -> (f32, bool) {
    if s < lo {
        let d = s - lo;
        (d * d, true)
    } else if s > hi {
        let d = s - hi;
        (d * d, true)
    } else {
        (0.0, s - lo <= radius || hi - s <= radius)
    }
}

/// One axis of the contain-test variant (`dir` 2).
///
/// Directional outside gap and the far-wall gap.
fn sphere_axis_out_far(s: f32, lo: f32, hi: f32) -> (f32, f32) {
    let d_lo = s - lo;
    let g_lo = d_lo * d_lo;
    let d_hi = s - hi;
    let g_hi = d_hi * d_hi;
    let far = if g_lo > g_hi { g_lo } else { g_hi };
    let out = if s < lo {
        g_lo
    } else if s > hi {
        g_hi
    } else {
        0.0
    };
    (out, far)
}

/// One axis of the plain overlap variant (`dir` 3).
///
/// The directional outside gap (the closest-point-on-box term).
fn sphere_axis_out(s: f32, lo: f32, hi: f32) -> f32 {
    if s < lo {
        let d = s - lo;
        d * d
    } else if s > hi {
        let d = s - hi;
        d * d
    } else {
        0.0
    }
}

/// Tests whether a sphere reaches into a neighboring grid cell along one of four directions.
///
/// The sphere is center `sphere[0..2]` in the XY plane, radius `sphere[3]`;
/// the four directions are `dir` 0..=3. The cell is given by its two opposite
/// corners, `cell_min` and `cell_max` (only the X/Y components are consulted).
/// Returns `true` (the original's nonzero low byte) when the sphere overlaps
/// that edge.
///
/// Per axis let `g_lo = (s-lo)²`, `g_hi = (s-hi)²`, `out` the directional
/// (closest-point) term and `far = max(g_lo, g_hi)`; with `r2 = radius²`:
/// `dir` 3 is `Σout ≤ r2` (sphere meets the cell rectangle); `dir` 2 adds
/// `r2 ≤ Σfar` (the far corner is not strictly inside the sphere); `dir` 1 is
/// `dir` 3 plus a latch that some axis is outside or within the *linear*
/// radius of a wall (sphere meets the rectangle's boundary); `dir` 0 combines
/// the `dir`-2 gates with the latch in its squared form (`min(g) ≤ r2`).
/// Branch polarity (outside-tests are ordered-strict, so a NaN center or bound
/// counts as inside), tie/unordered min-max selection (resolving to `g_hi`),
/// and the X-then-Y accumulation order mirror the stock routine.
pub fn sphere_test_cell_direction__7c2040(
    cell_min: &[f32; 3],
    cell_max: &[f32; 3],
    sphere: &[f32; 4],
    dir: i32,
) -> bool {
    let radius = sphere[3];
    let r2 = radius * radius;
    match dir {
        0 => {
            let (out_x, far_x, hit_x) =
                sphere_axis_terms_sq(sphere[0], cell_min[0], cell_max[0], r2);
            let (out_y, far_y, hit_y) =
                sphere_axis_terms_sq(sphere[1], cell_min[1], cell_max[1], r2);
            (hit_x || hit_y) && out_x + out_y <= r2 && r2 <= far_x + far_y
        }
        1 => {
            let (out_x, hit_x) = sphere_axis_terms_lin(sphere[0], cell_min[0], cell_max[0], radius);
            let (out_y, hit_y) = sphere_axis_terms_lin(sphere[1], cell_min[1], cell_max[1], radius);
            (hit_x || hit_y) && out_x + out_y <= r2
        }
        2 => {
            let (out_x, far_x) = sphere_axis_out_far(sphere[0], cell_min[0], cell_max[0]);
            let (out_y, far_y) = sphere_axis_out_far(sphere[1], cell_min[1], cell_max[1]);
            out_x + out_y <= r2 && r2 <= far_x + far_y
        }
        3 => {
            sphere_axis_out(sphere[0], cell_min[0], cell_max[0])
                + sphere_axis_out(sphere[1], cell_min[1], cell_max[1])
                <= r2
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests_sphere_test_cell_direction__7c2040 {
    use super::sphere_test_cell_direction__7c2040 as f;

    // A cell spanning [0,1]×[0,1] (min corner at origin, max at (1,1)).
    const MIN: [f32; 3] = [0.0, 0.0, 0.0];
    const MAX: [f32; 3] = [1.0, 1.0, 0.0];

    #[test]
    fn dir3_center_inside_no_directional_gap() {
        // Center inside the cell on both axes ⇒ no directional gap ⇒ gap(0) <= r2.
        let s = [0.5f32, 0.5, 0.0, 0.25];
        assert!(f(&MIN, &MAX, &s, 3));
    }

    #[test]
    fn dir3_far_beyond_max_no_overlap() {
        // Center far past max on X (hi < s) accumulates a large gap > r2.
        let s = [5.0f32, 0.5, 0.0, 0.25];
        assert!(!f(&MIN, &MAX, &s, 3));
    }

    #[test]
    fn dir3_just_reaches_max() {
        // Center 0.1 past the X max, radius 0.2 ⇒ gap = 0.01 < r2 = 0.04 ⇒ overlap.
        let s = [1.1f32, 0.5, 0.0, 0.2];
        assert!(f(&MIN, &MAX, &s, 3));
        // Shrink the radius so the gap exceeds it.
        let s2 = [1.1f32, 0.5, 0.0, 0.05];
        assert!(!f(&MIN, &MAX, &s2, 3));
    }

    #[test]
    fn dir3_monotone_in_radius() {
        // For a fixed geometry just outside the cell, a larger radius can only
        // turn a miss into a hit, never the reverse.
        let base = [1.3f32, 0.5, 0.0, 0.0];
        let mut prev = false;
        for &rad in &[0.0f32, 0.1, 0.2, 0.3, 0.5, 1.0] {
            let mut s = base;
            s[3] = rad;
            let now = f(&MIN, &MAX, &s, 3);
            assert!(now || !prev, "non-monotone at rad {rad}");
            prev = now;
        }
    }

    #[test]
    fn dir1_inside_within_radius_latches() {
        // Inside the cell with both edges within `radius` ⇒ latch hit, gap 0 <= r2.
        let s = [0.5f32, 0.5, 0.0, 0.6];
        assert!(f(&MIN, &MAX, &s, 1));
    }

    #[test]
    fn dir1_inside_far_from_edges_no_latch() {
        // Inside but both edges farther than radius and no gap accumulated ⇒ no hit.
        let s = [0.5f32, 0.5, 0.0, 0.1];
        assert!(!f(&MIN, &MAX, &s, 1));
    }

    #[test]
    fn dir1_just_past_min_overlaps() {
        // Center 0.1 below the X min, radius 0.2 ⇒ gap 0.01 <= r2, latched.
        let s = [-0.1f32, 0.5, 0.0, 0.2];
        // On Y it is inside; with radius 0.2 the Y edges (0.3 and 0.5 away) exceed
        // it, so only the X gap drives the result.
        assert!(f(&MIN, &MAX, &s, 1));
    }

    #[test]
    fn dir2_inside_overlaps() {
        // Center inside ⇒ inner gap 0 <= r2; outer is the larger per-axis gap
        // (here 0.5 for a centered point). Overlap needs r2 <= outer, so a
        // moderate radius (r2 = 0.25) passes both gates.
        let s = [0.5f32, 0.5, 0.0, 0.5];
        assert!(f(&MIN, &MAX, &s, 2));
    }

    #[test]
    fn dir2_inside_large_radius_outer_fails() {
        // Center inside but radius large ⇒ outer (~0.5) < r2 (4.0) fails the
        // outer gate — `outer >= r2` requires the sphere not to overrun the cell.
        let s = [0.5f32, 0.5, 0.0, 2.0];
        assert!(!f(&MIN, &MAX, &s, 2));
    }

    #[test]
    fn dir0_inside_overlaps() {
        // Off-center inside both axes: the smaller per-axis squared gap (0.01)
        // drives the proximity latch when within r2; outer (1.62) >= r2 and the
        // zero inner gap pass the remaining gates with a moderate radius.
        let s = [0.9f32, 0.9, 0.0, 0.5];
        assert!(f(&MIN, &MAX, &s, 0));
    }

    #[test]
    fn dir0_far_outside_no_overlap() {
        let s = [10.0f32, 10.0, 0.0, 0.1];
        assert!(!f(&MIN, &MAX, &s, 0));
    }

    #[test]
    fn invalid_dir_returns_false() {
        let s = [0.5f32, 0.5, 0.0, 0.5];
        assert!(!f(&MIN, &MAX, &s, 4));
        assert!(!f(&MIN, &MAX, &s, -1));
        assert!(!f(&MIN, &MAX, &s, 99));
    }

    #[test]
    fn zero_radius_dir3_only_exact_corner() {
        // radius 0 ⇒ r2 = 0; gap <= 0 only when no directional gap accrues
        // (center within [min,max] on every axis it is tested).
        let inside = [0.5f32, 0.5, 0.0, 0.0];
        assert!(f(&MIN, &MAX, &inside, 3));
        let outside = [2.0f32, 0.5, 0.0, 0.0];
        assert!(!f(&MIN, &MAX, &outside, 3));
    }

    /// `dir` 0 latches an inside axis at the squared boundary `near == r2`.
    ///
    /// Non-strict, matching the stock `setae`: center `(0.25, 0.5)` with
    /// `r = 0.25` has `near = 0.0625 == r2` and must report a hit.
    #[test]
    fn dir0_inside_boundary_is_inclusive() {
        let s = [0.25f32, 0.5, 0.0, 0.25];
        assert!(f(&MIN, &MAX, &s, 0));
    }

    /// A NaN center coordinate routes to the inside arm.
    ///
    /// Ordered-strict outside-tests are both false, so `dir` 3 can still hit
    /// on the other axis — the stock behaviour, where a NaN axis contributes
    /// no gap.
    #[test]
    fn nan_center_routes_inside() {
        let s = [f32::NAN, 1.05, 0.0, 0.1];
        // Y gap = 0.0025 <= r2 = 0.01; X (NaN) contributes nothing → hit.
        assert!(f(&MIN, &MAX, &s, 3));
        // dir 1: Y is outside (hi < s) so it latches and the gap passes.
        assert!(f(&MIN, &MAX, &s, 1));
        // dir 0/2 gate on the far corner, which goes NaN → no hit.
        assert!(!f(&MIN, &MAX, &s, 0));
        assert!(!f(&MIN, &MAX, &s, 2));
    }

    /// A negative radius separates the linear (`dir` 1) and squared (`dir` 0) proximity thresholds.
    ///
    /// Inside with wall distance 0.3, `r = -0.5` ⇒ the linear latch
    /// `0.3 <= -0.5` fails but the squared latch `0.09 <= 0.25` holds.
    #[test]
    fn negative_radius_linear_vs_squared_latch() {
        // dir 1 (linear): no axis outside and no wall within -0.5 → no latch.
        let s = [0.3f32, 0.5, 0.0, -0.5];
        assert!(!f(&MIN, &MAX, &s, 1));
        // dir 0 (squared): near = 0.09 <= r2 = 0.25 latches, gates pass.
        assert!(f(&MIN, &MAX, &s, 0));
    }

    /// Out-of-range `dir` values report no hit (the original's unsigned bound).
    #[test]
    fn out_of_range_dir_is_false() {
        let s = [0.5f32, 0.5, 0.0, 0.25];
        for d in [-1, 4, 99, i32::MIN] {
            assert!(!f(&MIN, &MAX, &s, d));
        }
    }
}

/// Squared distance from a point to a triangle given as a base point plus two edge vectors.
///
/// `p0` is the query point and `p1` is the triangle's anchor vertex; `edge_a` and `edge_b`
/// are the two triangle edges emanating from that vertex. Solves the 2x2 Gram system and
/// branches across the seven Voronoi regions, returning the region's squared distance.
// The Voronoi region tests stay `!(x >= 0.0)` so a NaN Gram intermediate picks
// the branch the original's ordered compare picks; `x < 0.0` picks the other.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn sq_dist_point_triangle_edges__6dc900(
    p0: &[f32; 3],
    p1: &[f32; 3],
    edge_a: &[f32; 3],
    edge_b: &[f32; 3],
) -> f32 {
    // d = p1 - p0
    let dx = p1[0] - p0[0];
    let dy = p1[1] - p0[1];
    let dz = p1[2] - p0[2];

    let aa = edge_a[0] * edge_a[0] + edge_a[1] * edge_a[1] + edge_a[2] * edge_a[2];
    let cc = edge_b[0] * edge_b[0] + edge_b[1] * edge_b[1] + edge_b[2] * edge_b[2];
    let dd = dx * dx + dy * dy + dz * dz; // |d|^2
    let ac = edge_a[0] * edge_b[0] + edge_a[1] * edge_b[1] + edge_a[2] * edge_b[2];
    let da = dx * edge_a[0] + dy * edge_a[1] + dz * edge_a[2];
    let db = dx * edge_b[0] + dy * edge_b[1] + dz * edge_b[2];

    let det = (cc * aa - ac * ac).abs();
    let s = db * ac - da * cc;
    let t = da * ac - db * aa;

    // ZERO/ONE/NEG are the three immutable .rdata float constants (0.0, 1.0, -1.0).
    const ZERO: f32 = 0.0;
    const ONE: f32 = 1.0;
    const NEG: f32 = -1.0;

    // `s + t > det`  (stock form `(s+t<det)==(s+t==det)`).
    if s + t > det {
        if ZERO <= s {
            if t < ZERO {
                let tmp = da + aa;
                if db + ac < tmp {
                    let num = tmp - (db + ac);
                    let den = (aa - (ac + ac)) + cc;
                    if den <= num {
                        return (db + db + dd + cc).abs();
                    }
                    let frac = num / den;
                    let inv = ONE - frac;
                    return ((da + da + inv * aa + frac * ac) * inv
                        + (db + db + frac * cc + ac * inv) * frac
                        + dd)
                        .abs();
                }
                if !(tmp >= ZERO) {
                    return (da + da + dd + aa).abs();
                }
                if ZERO <= da {
                    return dd.abs();
                }
                return ((NEG / aa) * da * da + dd).abs();
            }
            let num2 = ((db + cc) - ac) - da;
            if !(num2 >= ZERO) {
                return (db + db + dd + cc).abs();
            }
            let den2 = (aa - (ac + ac)) + cc;
            if den2 <= num2 {
                return (da + da + dd + aa).abs();
            }
            let frac = num2 / den2;
            let inv = ONE - frac;
            return ((da + da + frac * aa + inv * ac) * frac
                + (db + db + inv * cc + frac * ac) * inv
                + dd)
                .abs();
        } else {
            let lhs = db + cc;
            if lhs <= da + ac {
                if !(lhs >= ZERO) {
                    return (db + db + dd + cc).abs();
                }
                if db < ZERO {
                    return (NEG / cc) * db * db + dd;
                }
                return dd.abs();
            }
            let num = lhs - (da + ac);
            let den = (aa - (ac + ac)) + cc;
            if den <= num {
                return (da + da + dd + aa).abs();
            }
            let frac = num / den;
            let inv = ONE - frac;
            return ((da + da + frac * aa + inv * ac) * frac
                + (db + db + inv * cc + frac * ac) * inv
                + dd)
                .abs();
        }
    }

    // s + t <= det
    if ZERO <= s {
        if ZERO <= t {
            let inv_det = ONE / det;
            let ss = s * inv_det;
            let tt = inv_det * t;
            return ((da + da + ss * aa + tt * ac) * ss + (db + db + tt * cc + ss * ac) * tt + dd)
                .abs();
        }
        if ZERO <= da {
            return dd.abs();
        }
    } else {
        if ZERO <= t {
            if db < ZERO {
                if cc <= -db {
                    return (db + db + dd + cc).abs();
                }
                return ((NEG / cc) * db * db + dd).abs();
            }
            return dd.abs();
        }
        if ZERO <= da {
            if db < ZERO {
                if cc <= -db {
                    return (db + db + dd + cc).abs();
                }
                return (NEG / cc) * db * db + dd;
            }
            return dd.abs();
        }
    }

    if aa <= -da {
        return (da + da + dd + aa).abs();
    }
    ((NEG / aa) * da * da + dd).abs()
}

#[cfg(test)]
mod tests_sq_dist_point_triangle_edges__6dc900 {
    use super::sq_dist_point_triangle_edges__6dc900 as f;

    // Brute-force reference: closest squared distance from query point to the triangle
    // (a, a+edge_a, a+edge_b), sampling the barycentric domain densely.
    fn brute(p0: &[f32; 3], a: &[f32; 3], ea: &[f32; 3], eb: &[f32; 3]) -> f32 {
        let mut best = f64::INFINITY;
        let n = 400;
        for i in 0..=n {
            let u = i as f64 / n as f64;
            for j in 0..=n {
                let v = j as f64 / n as f64;
                if u + v > 1.0 {
                    continue;
                }
                let mut s = 0.0f64;
                for k in 0..3 {
                    let pt = a[k] as f64 + u * ea[k] as f64 + v * eb[k] as f64;
                    let d = pt - p0[k] as f64;
                    s += d * d;
                }
                if s < best {
                    best = s;
                }
            }
        }
        best as f32
    }

    fn check(p0: [f32; 3], a: [f32; 3], ea: [f32; 3], eb: [f32; 3]) {
        let got = f(&p0, &a, &ea, &eb);
        let want = brute(&p0, &a, &ea, &eb);
        let tol = 1e-3 * (1.0 + want.abs());
        assert!(
            (got - want).abs() <= tol,
            "p0={p0:?} a={a:?} ea={ea:?} eb={eb:?}: got {got}, brute {want}"
        );
        assert!(got >= -1e-4, "negative distance {got}");
    }

    #[test]
    fn region_interior() {
        check(
            [0.3, 0.3, 1.0],
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        );
    }

    #[test]
    fn vertex_a_region() {
        check(
            [-1.0, -1.0, 0.0],
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        );
    }

    #[test]
    fn vertex_b_region() {
        check(
            [2.0, -0.5, 0.0],
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        );
    }

    #[test]
    fn vertex_c_region() {
        check(
            [-0.5, 2.0, 0.0],
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        );
    }

    #[test]
    fn edge_ab_region() {
        check(
            [0.5, -1.0, 0.3],
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        );
    }

    #[test]
    fn edge_ac_region() {
        check(
            [-1.0, 0.5, 0.3],
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        );
    }

    #[test]
    fn edge_bc_region() {
        check(
            [0.8, 0.8, 0.2],
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        );
    }

    #[test]
    fn known_value_above_origin() {
        let got = f(
            &[0.0, 0.0, 2.0],
            &[0.0, 0.0, 0.0],
            &[1.0, 0.0, 0.0],
            &[0.0, 1.0, 0.0],
        );
        assert!((got - 4.0).abs() < 1e-4, "got {got}");
    }

    #[test]
    fn random_fuzz() {
        let mut state: u64 = 0x1234_5678_9abc_def0;
        let mut rnd = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) as f32 / (1u32 << 31) as f32) * 4.0 - 2.0
        };
        for _ in 0..2000 {
            let a = [rnd(), rnd(), rnd()];
            let ea = [rnd(), rnd(), rnd()];
            let eb = [rnd(), rnd(), rnd()];
            let p0 = [rnd(), rnd(), rnd()];
            let aa = ea[0] * ea[0] + ea[1] * ea[1] + ea[2] * ea[2];
            let cc = eb[0] * eb[0] + eb[1] * eb[1] + eb[2] * eb[2];
            let acd = ea[0] * eb[0] + ea[1] * eb[1] + ea[2] * eb[2];
            let det = (aa * cc - acd * acd).abs();
            if det < 0.05 || aa < 0.05 || cc < 0.05 {
                continue;
            }
            check(p0, a, ea, eb);
        }
    }
}

/// Builds the four clip planes bounding one swept-prism triangle face.
///
/// Three extruded edge planes plus one cap plane (16 contiguous floats,
/// `[nx, ny, nz, d]` each).
///
/// `tri` are the face's three vertices (already de-indexed). For each ring edge
/// `i` (A = `tri[i]`, B = `tri[(i+1)%3]`, opposite = `tri[(i+2)%3]`) the swept
/// side plane is the triangle plane through `A`, `B`, `A+extrude`, flipped so the
/// opposite vertex stays on the negative side. `extrude` is the sweep offset;
/// `face_plane` is the cap plane's normal, offset through `tri[0]+extrude`.
/// `flip_thresh` is the host orientation threshold (`0.0`).
///
/// Returns the planes and how many leading entries the original had written when
/// it returned: 4 on success, and on a degenerate edge (swept area below
/// `area_eps`) the number of side planes already finished, since each is written
/// straight through the out pointer during its own iteration.
///
/// The triangle sibling of `collision_build_quad_face_clip_planes__632f80`, and
/// the same shape facts apply with one difference the algebra hides: **this one's
/// orientation dot runs `(z + y) + x`** (`0x632614`..`0x632627`) where the quad's
/// runs `(z + x) + y`. The cross components are also stored to the other slots, so
/// the original computes them with the signs swapped throughout; that is invisible
/// (they only feed a squared magnitude) and the faithful expressions are written
/// here so both siblings read alike.
pub fn collision_build_face_edge_planes__632460(
    tri: &[[f32; 3]; 3],
    extrude: &[f32; 3],
    face_plane: &[f32; 3],
    area_eps: f32,
    flip_thresh: f32,
) -> ([[f32; 4]; 4], usize) {
    let mut planes = [[0.0f32; 4]; 4];

    for i in 0..3 {
        let a = tri[i];
        let b = tri[(i + 1) % 3];
        let opp = tri[(i + 2) % 3];
        let asw = [a[0] + extrude[0], a[1] + extrude[1], a[2] + extrude[2]];

        // Degenerate test on |e1 x e2|^2 with e1 = Asw - A (= extrude), e2 = B - A.
        let e1 = [asw[0] - a[0], asw[1] - a[1], asw[2] - a[2]];
        let e2 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let cross = |u: f32, v: f32, w: f32, t: f32| {
            super::f64_to_f32(f64::from(u) * f64::from(v) - f64::from(w) * f64::from(t))
        };
        let cx = cross(e1[2], e2[1], e1[1], e2[2]);
        let cy = cross(e2[2], e1[0], e1[2], e2[0]);
        let cz = cross(e1[1], e2[0], e2[1], e1[0]);
        let sq = |v: f32| f64::from(v) * f64::from(v);
        let area_sq = (sq(cy) + sq(cz)) + sq(cx);
        if area_sq.abs() < f64::from(area_eps) {
            return (planes, i);
        }

        // Plane from triangle (A, B, Asw), the original's own `0x637480` call, then
        // inward-flip if the opposite vertex is on the positive side.
        let mut plane = super::plane::c4_plane__from_triangle__637480(&a, &b, &asw);
        let d = [opp[0] - a[0], opp[1] - a[1], opp[2] - a[2]];
        let term = |k: usize| f64::from(d[k]) * f64::from(plane[k]);
        let side = (term(2) + term(1)) + term(0);
        if side > f64::from(flip_thresh) {
            plane = [-plane[0], -plane[1], -plane[2], -plane[3]];
        }
        planes[i] = plane;
    }

    // Cap plane: `face_plane` normal through tri[0] + extrude, that point kept wide.
    let mid = [
        f64::from(tri[0][0]) + f64::from(extrude[0]),
        f64::from(tri[0][1]) + f64::from(extrude[1]),
        f64::from(tri[0][2]) + f64::from(extrude[2]),
    ];
    let term = |k: usize| mid[k] * f64::from(face_plane[k]);
    let cap_d = super::f64_to_f32(-((term(2) + term(1)) + term(0)));
    planes[3] = [face_plane[0], face_plane[1], face_plane[2], cap_d];

    (planes, 4)
}

#[cfg(test)]
mod tests_collision_build_face_edge_planes__632460 {
    use super::collision_build_face_edge_planes__632460 as build;

    /// The four planes, asserting the original ran to completion.
    fn all_four(
        tri: &[[f32; 3]; 3],
        extrude: &[f32; 3],
        face_plane: &[f32; 3],
        area_eps: f32,
        flip_thresh: f32,
    ) -> [[f32; 4]; 4] {
        let (planes, written) = build(tri, extrude, face_plane, area_eps, flip_thresh);
        assert_eq!(written, 4, "expected all four planes");
        planes
    }

    // The oracle repeats the kernel's degeneracy epsilon at full precision so both
    // land on the same stored f32; trimming digits would test a different value.
    #[allow(clippy::excessive_precision)]
    const AREA_EPS: f32 = 9.5367431640625e-7;

    fn dot4(plane: &[f32; 4], p: &[f32; 3]) -> f32 {
        plane[0] * p[0] + plane[1] * p[1] + plane[2] * p[2] + plane[3]
    }

    fn unit_tri() -> [[f32; 3]; 3] {
        [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
    }

    #[test]
    fn edge_planes_unit_normalized() {
        let t = unit_tri();
        let planes = all_four(&t, &[0.0, 0.0, 2.0], &[0.0, 0.0, 1.0], AREA_EPS, 0.0);
        for plane in planes.iter().take(3) {
            let len = (plane[0] * plane[0] + plane[1] * plane[1] + plane[2] * plane[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-5, "len={len}");
        }
    }

    #[test]
    fn each_edge_plane_contains_its_endpoints() {
        let t = unit_tri();
        let ex = [0.0, 0.0, 2.0];
        let planes = all_four(&t, &ex, &[0.0, 0.0, 1.0], AREA_EPS, 0.0);
        for i in 0..3 {
            let a = t[i];
            let b = t[(i + 1) % 3];
            let asw = [a[0] + ex[0], a[1] + ex[1], a[2] + ex[2]];
            for v in [a, b, asw] {
                assert!(dot4(&planes[i], &v).abs() < 1e-4);
            }
        }
    }

    #[test]
    fn opposite_vertex_on_negative_side() {
        let t = unit_tri();
        let planes = all_four(&t, &[0.0, 0.0, 2.0], &[0.0, 0.0, 1.0], AREA_EPS, 0.0);
        for i in 0..3 {
            let opp = t[(i + 2) % 3];
            assert!(dot4(&planes[i], &opp) <= 1e-4);
        }
    }

    #[test]
    fn cap_plane_uses_face_normal_through_swept_v0() {
        let t = unit_tri();
        let ex = [0.0, 0.0, 3.0];
        let face = [0.0, 0.0, 1.0];
        let planes = all_four(&t, &ex, &face, AREA_EPS, 0.0);
        let cap = planes[3];
        assert_eq!(cap[0].to_bits(), face[0].to_bits());
        assert_eq!(cap[1].to_bits(), face[1].to_bits());
        assert_eq!(cap[2].to_bits(), face[2].to_bits());
        let swept_v0 = [t[0][0] + ex[0], t[0][1] + ex[1], t[0][2] + ex[2]];
        assert!(dot4(&cap, &swept_v0).abs() < 1e-5);
    }

    #[test]
    fn degenerate_zero_extrude_returns_none() {
        let t = unit_tri();
        assert_eq!(
            build(&t, &[0.0, 0.0, 0.0], &[0.0, 0.0, 1.0], AREA_EPS, 0.0).1,
            0
        );
    }

    #[test]
    fn known_value_edge0_plane() {
        let t = unit_tri();
        let planes = all_four(&t, &[0.0, 0.0, 2.0], &[0.0, 0.0, 1.0], AREA_EPS, 0.0);
        let p = planes[0];
        assert!(p[0].abs() < 1e-6);
        assert!((p[1] + 1.0).abs() < 1e-6, "{}", p[1]);
        assert!(p[2].abs() < 1e-6);
        assert!(p[3].abs() < 1e-6);
    }
}

/// Sutherland-Hodgman clip of a convex polygon (max 15 vertices) by one plane, in place.
///
/// `verts` holds up to 15 vertices contiguously (3 floats each, 45 total); `tags`
/// holds one classification float per vertex (15 total). `count` is the live
/// vertex count. `plane = [nx, ny, nz, d]`; `plane_index` is the tag written to
/// vertices created on the clip plane. A vertex's signed side is `s = -(dot
/// (plane.xyz, v) + plane.w)` (interior is `s >= 0`). The polygon is rejected
/// (count `0`) when its near side never reaches `eps_neg` or its far side never
/// reaches `eps_pos` (the `+/-1/720` band), or when fewer than 3 vertices
/// survive. Returns the new vertex count.
// Both band tests stay negated so a NaN distance takes the early return the
// original takes; `min_dist > eps_neg` / `max_dist < eps_pos` would instead
// fall through into the clip loop.
//
// The 15-vertex buffer has NO headroom: the worst case fills it exactly. One
// clip emits at most `count + 1` (a convex polygon has two sign transitions,
// and the band gates guarantee a vertex on each side), every polygon is seeded
// at 3, and the deepest chain is `collision_sweep_polygon_against_faces__632700`
// with 4 face planes then `collision_ray_polygon_sweep_distance__632830` with 8
// prism planes: 3 + 4 + 8 = 15, highest index written 14. `clip_emit_vertex`
// does not bounds-check, and it does not need to. Adding a plane to either
// caller overflows this. Do not "fix" that with a runtime check, which would
// silently clamp instead of surfacing the mistake; grow the buffer, and grow
// stock's matching 15-slot scratch reasoning with it.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn collision_clip_polygon_by_plane__6318c0(
    verts: &mut [f32; 45],
    tags: &mut [f32; 15],
    count: usize,
    plane: &[f32; 4],
    plane_index: f32,
    eps_neg: f32,
    eps_pos: f32,
) -> usize {
    if count == 0 {
        return 0;
    }
    let mut dist = [0.0f32; 15];
    let mut min_dist = f32::MAX;
    let mut max_dist = -f32::MAX;
    for i in 0..count {
        let v = &verts[i * 3..i * 3 + 3];
        let d = -(v[0] * plane[0] + v[1] * plane[1] + v[2] * plane[2] + plane[3]);
        if d < min_dist {
            min_dist = d;
        }
        if max_dist < d {
            max_dist = d;
        }
        dist[i] = d;
    }
    if !(min_dist <= eps_neg) {
        return count;
    }
    if !(eps_pos <= max_dist) {
        return 0;
    }

    let saved_verts = *verts;
    let saved_tags = *tags;
    let mut out = 0usize;
    for cur in 0..count {
        let prev = (cur + count - 1) % count;
        let d_prev = dist[prev];
        let d_cur = dist[cur];
        let p = &saved_verts[prev * 3..prev * 3 + 3];
        let c = &saved_verts[cur * 3..cur * 3 + 3];
        if d_prev >= 0.0 {
            if d_cur >= 0.0 {
                clip_emit_vertex(verts, tags, &mut out, c, saved_tags[cur]);
            } else if d_prev > eps_pos {
                // Leaving the half-space. The original builds this one through
                // two `C3Vector::Set` calls, so all three products become `f32`
                // stack arguments first, and it emits the crossing alone.
                let x = clip_crossing_narrow(p, c, d_prev, d_cur);
                clip_emit_vertex(verts, tags, &mut out, &x, plane_index);
            }
        } else if d_cur >= 0.0 {
            if d_cur > eps_pos {
                // Entering the half-space. The original inlines the stores here
                // and rounds each component differently, then falls through to
                // emit the current vertex as well.
                let x = clip_crossing_mixed(p, c, d_prev, d_cur);
                clip_emit_vertex(verts, tags, &mut out, &x, plane_index);
            }
            clip_emit_vertex(verts, tags, &mut out, c, saved_tags[cur]);
        }
    }
    if out > 2 { out } else { 0 }
}

/// The crossing parameter `t = d_prev / (d_cur - d_prev)`.
///
/// Shared by both crossing shapes and rounded the same way in each: the original
/// leaves the denominator on the x87 stack and stores only the quotient, so the
/// subtraction happens at register width and the divide's result is the single
/// narrowing.
fn clip_crossing_t(d_prev: f32, d_cur: f32) -> f32 {
    super::f64_to_f32(f64::from(d_prev) / (f64::from(d_cur) - f64::from(d_prev)))
}

/// One crossing component whose `delta * t` product reaches an `f32` slot.
///
/// The delta itself stays at register width into the multiply; the product is
/// stored and reloaded, so the final subtraction reads a narrowed operand.
fn cross_store_product(pi: f32, ci: f32, tf: f64) -> f32 {
    let dt = super::f64_to_f32((f64::from(ci) - f64::from(pi)) * tf);
    super::f64_to_f32(f64::from(pi) - f64::from(dt))
}

/// One crossing component that never reaches memory before its single store.
fn cross_wide(pi: f32, ci: f32, tf: f64) -> f32 {
    super::f64_to_f32(f64::from(pi) - (f64::from(ci) - f64::from(pi)) * tf)
}

/// One crossing component whose *delta* reaches an `f32` slot before the multiply.
///
/// The mirror image of [`cross_store_product`]: here the subtraction is stored
/// and reloaded, and the product and final subtraction then stay wide.
fn cross_store_delta(pi: f32, ci: f32, tf: f64) -> f32 {
    let d = super::f64_to_f32(f64::from(ci) - f64::from(pi));
    super::f64_to_f32(f64::from(pi) - f64::from(d) * tf)
}

/// Interpolated plane crossing, in the mixed widths of the inlined block.
///
/// `x = p - (c - p) * t`, but each component rounds differently and the
/// difference is not decorative: the original spills `dx * t`, spills `dz`
/// before its multiply, and leaves the whole y chain in a register until its
/// single store. Reproducing one shape for all three is what this used to do.
fn clip_crossing_mixed(p: &[f32], c: &[f32], d_prev: f32, d_cur: f32) -> [f32; 3] {
    let tf = f64::from(clip_crossing_t(d_prev, d_cur));
    [
        cross_store_product(p[0], c[0], tf),
        cross_wide(p[1], c[1], tf),
        cross_store_delta(p[2], c[2], tf),
    ]
}

/// Interpolated plane crossing, narrowed at every step.
///
/// The block that builds its result through two `C3Vector::Set` calls: all three
/// `delta * t` products become `f32` stack arguments before the subtraction, so
/// every component takes the [`cross_store_product`] shape. The setter itself is
/// three integer stores and adds no rounding of its own.
fn clip_crossing_narrow(p: &[f32], c: &[f32], d_prev: f32, d_cur: f32) -> [f32; 3] {
    let tf = f64::from(clip_crossing_t(d_prev, d_cur));
    [
        cross_store_product(p[0], c[0], tf),
        cross_store_product(p[1], c[1], tf),
        cross_store_product(p[2], c[2], tf),
    ]
}

fn clip_emit_vertex(
    verts: &mut [f32; 45],
    tags: &mut [f32; 15],
    out: &mut usize,
    v: &[f32],
    tag: f32,
) {
    let i = *out;
    tags[i] = tag;
    verts[i * 3] = v[0];
    verts[i * 3 + 1] = v[1];
    verts[i * 3 + 2] = v[2];
    *out += 1;
}

#[cfg(test)]
mod tests_collision_clip_polygon_by_plane__6318c0 {
    use super::collision_clip_polygon_by_plane__6318c0 as clip;

    // The negative half of the kernel's `+/-1/720` clip band, reproduced
    // bit-exactly; a shorter literal is a different f32 and a different test.
    #[allow(clippy::excessive_precision)]
    const EN: f32 = -0.0013888889225199819;
    // The positive half of the same band, likewise reproduced bit-exactly.
    #[allow(clippy::excessive_precision)]
    const EP: f32 = 0.0013888889225199819;

    fn make(verts: &[[f32; 3]]) -> ([f32; 45], [f32; 15], usize) {
        let mut v = [0.0f32; 45];
        let mut t = [0.0f32; 15];
        for (i, p) in verts.iter().enumerate() {
            v[i * 3] = p[0];
            v[i * 3 + 1] = p[1];
            v[i * 3 + 2] = p[2];
            t[i] = 100.0 + i as f32;
        }
        (v, t, verts.len())
    }

    fn signed(plane: &[f32; 4], v: &[f32; 3]) -> f32 {
        -(v[0] * plane[0] + v[1] * plane[1] + v[2] * plane[2] + plane[3])
    }

    #[test]
    fn fully_interior_polygon_is_untouched() {
        let (mut v, mut t, n) = make(&[
            [0.0, 0.0, -1.0],
            [2.0, 0.0, -1.0],
            [2.0, 2.0, -1.0],
            [0.0, 2.0, -1.0],
        ]);
        assert_eq!(
            clip(&mut v, &mut t, n, &[0.0, 0.0, 1.0, 0.0], 7.0, EN, EP),
            4
        );
    }

    #[test]
    fn fully_outside_polygon_is_rejected() {
        let (mut v, mut t, n) = make(&[[0.0, 0.0, 1.0], [2.0, 0.0, 1.0], [2.0, 2.0, 1.0]]);
        assert_eq!(
            clip(&mut v, &mut t, n, &[0.0, 0.0, 1.0, 0.0], 7.0, EN, EP),
            0
        );
    }

    #[test]
    fn half_clipped_triangle_keeps_interior_side() {
        let (mut v, mut t, n) = make(&[[0.0, 0.0, -1.0], [2.0, 0.0, 1.0], [-2.0, 0.0, 1.0]]);
        let plane = [0.0, 0.0, 1.0, 0.0];
        let out = clip(&mut v, &mut t, n, &plane, 7.0, EN, EP);
        assert!(out >= 3, "out={out}");
        for i in 0..out {
            let vv = [v[i * 3], v[i * 3 + 1], v[i * 3 + 2]];
            assert!(signed(&plane, &vv) >= EN - 1e-5);
        }
    }

    #[test]
    fn crossing_vertices_carry_plane_index_tag() {
        let (mut v, mut t, n) = make(&[[0.0, 0.0, -1.0], [2.0, 0.0, 1.0], [-2.0, 0.0, 1.0]]);
        let pidx = 42.0;
        let out = clip(&mut v, &mut t, n, &[0.0, 0.0, 1.0, 0.0], pidx, EN, EP);
        assert!((0..out).any(|i| t[i].to_bits() == pidx.to_bits()));
    }

    #[test]
    fn crossing_lands_on_the_plane() {
        for x in [
            super::clip_crossing_mixed(&[0.0, 0.0, -1.0], &[4.0, 0.0, 3.0], 1.0, -3.0),
            super::clip_crossing_narrow(&[0.0, 0.0, -1.0], &[4.0, 0.0, 3.0], 1.0, -3.0),
        ] {
            assert!((x[0] - 1.0).abs() < 1e-6);
            assert!((x[2] - 0.0).abs() < 1e-6);
        }
    }

    /// Law: the two crossing arms round differently, and neither rounds per step.
    ///
    /// The entering arm is the inlined block, whose y component stays in a
    /// register from its subtraction to a single store while x spills its
    /// product and z spills its delta. The leaving arm goes through two
    /// `C3Vector::Set` calls, so every component spills its product. Pinned
    /// because one shared formula for both reads natural and is what was here.
    #[test]
    fn crossing_arms_round_differently() {
        // Bit patterns, not decimal literals: a rounded decimal is a different
        // float and stops separating the shapes.
        let p = [
            f32::from_bits(0xbef3_d112),
            f32::from_bits(0x3f19_046e),
            f32::from_bits(0x4087_bf55),
        ];
        let c = [
            f32::from_bits(0xbeaf_df24),
            f32::from_bits(0x3da0_96db),
            f32::from_bits(0x3f5f_b485),
        ];
        let (d_prev, d_cur) = (f32::from_bits(0x3ec1_4448), f32::from_bits(0xbf7b_3698));
        let mixed = super::clip_crossing_mixed(&p, &c, d_prev, d_cur);
        let narrow = super::clip_crossing_narrow(&p, &c, d_prev, d_cur);

        // x takes the same shape in both arms, so it must agree.
        assert_eq!(mixed[0].to_bits(), narrow[0].to_bits());
        // y and z do not: if these ever match, the arms have collapsed.
        assert!(
            mixed[1].to_bits() != narrow[1].to_bits() || mixed[2].to_bits() != narrow[2].to_bits(),
            "the two arms produced identical y and z; the shapes have collapsed"
        );

        // And neither is the old all-f32 form.
        let t = d_prev / (d_cur - d_prev);
        let per_step = [
            p[0] - (c[0] - p[0]) * t,
            p[1] - (c[1] - p[1]) * t,
            p[2] - (c[2] - p[2]) * t,
        ];
        assert!(
            (0..3).any(|i| per_step[i].to_bits() != mixed[i].to_bits()),
            "inputs no longer separate the entering arm from per-step f32"
        );
    }
}

/// Inside-test of a world position against three side clip-planes.
///
/// `planes` is a 16-float buffer of 4 plane equations (`nx, ny, nz, d` each);
/// only the first three (the box side faces) are tested. Returns `1` when the
/// position lies on the inner side of every side plane (each
/// `dot(n, pos) + d <= threshold`), else `0`. The rejection is strict: a single
/// plane with `eval > threshold` (`threshold < eval`) makes the position clear.
pub fn collision_plane_box_clearance_test__6335d0(
    planes: &[f32; 16],
    pos: &[f32; 3],
    threshold: f32,
) -> u32 {
    for plane in planes.chunks_exact(4).take(3) {
        let eval = plane[0] * pos[0] + plane[1] * pos[1] + plane[2] * pos[2] + plane[3];
        if threshold < eval {
            return 0;
        }
    }
    1
}

#[cfg(test)]
mod plane_box_clearance_tests {
    use super::collision_plane_box_clearance_test__6335d0 as f;

    // A unit box centered at the origin: three side planes facing +x, +y, +z,
    // each `dot(n, p) - 1 <= 0` inside. threshold = 0.
    const BOX: [f32; 16] = [
        1.0, 0.0, 0.0, -1.0, // +x face: x <= 1
        0.0, 1.0, 0.0, -1.0, // +y face: y <= 1
        0.0, 0.0, 1.0, -1.0, // +z face: z <= 1
        0.0, 0.0, 0.0, 0.0, // unused 4th plane
    ];

    #[test]
    fn inside_returns_one() {
        assert_eq!(f(&BOX, &[0.0, 0.0, 0.0], 0.0), 1);
        assert_eq!(f(&BOX, &[0.5, 0.5, 0.5], 0.0), 1);
    }

    #[test]
    fn on_boundary_is_inside() {
        // eval == threshold (0) must NOT reject (<= semantics).
        assert_eq!(f(&BOX, &[1.0, 0.0, 0.0], 0.0), 1);
    }

    #[test]
    fn outside_one_plane_returns_zero() {
        assert_eq!(f(&BOX, &[1.5, 0.0, 0.0], 0.0), 0);
        assert_eq!(f(&BOX, &[0.0, 2.0, 0.0], 0.0), 0);
        assert_eq!(f(&BOX, &[0.0, 0.0, 1.0001], 0.0), 0);
    }

    #[test]
    fn fourth_plane_ignored() {
        // A 4th plane that would reject is never evaluated.
        let mut planes = BOX;
        planes[12] = 1.0;
        planes[15] = -0.1;
        assert_eq!(f(&planes, &[0.5, 0.5, 0.5], 0.0), 1);
    }

    #[test]
    fn threshold_shifts_acceptance() {
        assert_eq!(f(&BOX, &[2.0, 0.0, 0.0], 1.0), 1); // eval = 2-1 = 1 <= 1
        assert_eq!(f(&BOX, &[2.0001, 0.0, 0.0], 1.0), 0);
    }

    #[test]
    fn metamorphic_strictly_inside_stays_inside() {
        let mut state: u64 = 0xdead_beef_cafe_babe;
        let mut rnd = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 40) as f32 / (1u32 << 24) as f32 // [0,1)
        };
        for _ in 0..5000 {
            let p = [rnd() * 0.9, rnd() * 0.9, rnd() * 0.9];
            assert_eq!(f(&BOX, &p, 0.0), 1, "p={p:?}");
        }
    }
}

// One distance pass: for each polygon vertex (read as a plane normal) compute the
// sweep time against `motion` (the folded ray-plane intersect kernel); optionally
// track the closest non-negative time; returns whether every vertex stayed
// outside the `sep_eps` band.
fn sweep_distance_pass(
    verts: &[f32; 45],
    count: usize,
    motion: &[f32; 4],
    point: &[f32; 3],
    sep_eps: f32,
    best_so_far: &mut f32,
    track_best: bool,
) -> bool {
    let mut all_outside = true;
    for i in 0..count {
        let normal = [verts[i * 3], verts[i * 3 + 1], verts[i * 3 + 2], 0.0];
        let t = collision_ray_plane_intersect_time__6329e0(&normal, motion, point);
        if track_best && t < f64::from(*best_so_far) {
            *best_so_far = if t <= 0.0 { 0.0 } else { t as f32 };
        }
        if t > f64::from(sep_eps) {
            all_outside = false;
        }
    }
    all_outside
}

fn sweep_finish(best: &mut f32, best_so_far: f32) -> u32 {
    if best_so_far < *best {
        *best = best_so_far;
        1
    } else {
        0
    }
}

/// Nearest sweep distance at which a moving point first enters a convex polygon.
///
/// `verts`/`tags` are the polygon storage (vertices contiguous, 3 floats each;
/// one tag per vertex); `count` is the live vertex count (updated in place as the
/// polygon is re-clipped). `ray_origin` is the swept point's start. `vert_array`
/// is the bounding plane set (`[nx, ny, nz, d]` per plane); `vert_base` selects
/// the sweep plane (`motion = vert_array[vert_base]`) and is skipped during the
/// re-clip. `best` is the running best distance (lowered on a closer hit). The
/// two separation epsilons gate the cheap and re-clip "all outside" tests; the
/// clip epsilons feed the folded clip. Returns `1` on a hit, else `0`.
// The eleven parameters are the original's calling convention: polygon storage,
// ray origin, plane set with its base index, the running best and four
// epsilons. That is an ABI fact, not a signature that wants a params struct.
#[allow(clippy::too_many_arguments)]
pub fn collision_ray_polygon_sweep_distance__632830(
    verts: &mut [f32; 45],
    tags: &mut [f32; 15],
    count: &mut usize,
    ray_origin: &[f32; 3],
    vert_array: &[[f32; 4]],
    vert_base: usize,
    best: &mut f32,
    sep_far_eps: f32,
    sep_near_eps: f32,
    clip_eps_neg: f32,
    clip_eps_pos: f32,
) -> u32 {
    let motion = vert_array[vert_base];
    let mut best_so_far = *best;
    let mut live = *count;

    if live != 0 {
        let all_outside = sweep_distance_pass(
            verts,
            live,
            &motion,
            ray_origin,
            sep_far_eps,
            &mut best_so_far,
            true,
        );
        if !all_outside {
            return sweep_finish(best, best_so_far);
        }
    }

    for (idx, plane) in vert_array.iter().take(9).enumerate() {
        if idx == vert_base {
            continue;
        }
        live = collision_clip_polygon_by_plane__6318c0(
            verts,
            tags,
            live,
            plane,
            idx as f32,
            clip_eps_neg,
            clip_eps_pos,
        );
        *count = live;
        if live == 0 {
            return 0;
        }
    }
    *count = live;

    let all_outside = sweep_distance_pass(
        verts,
        live,
        &motion,
        ray_origin,
        sep_near_eps,
        &mut best_so_far,
        false,
    );
    if all_outside {
        return 0;
    }
    sweep_finish(best, best_so_far)
}

#[cfg(test)]
mod tests_collision_ray_polygon_sweep_distance__632830 {
    use super::collision_ray_polygon_sweep_distance__632830 as sweep;

    const SEP_FAR: f32 = -9.536_743e-7;
    const SEP_NEAR: f32 = -0.027_777_778;
    const CN: f32 = -0.0013888889;
    const CP: f32 = 0.0013888889;

    #[test]
    fn hit_lowers_best_when_cheap_pass_finds_inner() {
        let mut v = [0.0f32; 45];
        let mut t = [0.0f32; 15];
        v[0] = 1.0;
        let mut best = 10.0f32;
        let mut count = 1usize;
        let va = [[1.0f32, 0.0, 0.0, 0.0]; 9];
        let r = sweep(
            &mut v,
            &mut t,
            &mut count,
            &[4.0, 0.0, 0.0],
            &va,
            0,
            &mut best,
            SEP_FAR,
            SEP_NEAR,
            CN,
            CP,
        );
        assert_eq!(r, 1);
        assert!((best - 0.25).abs() < 1e-5, "best={best}");
    }

    #[test]
    fn no_hit_when_best_not_improved() {
        let mut v = [0.0f32; 45];
        let mut t = [0.0f32; 15];
        v[0] = 1.0;
        let mut best = 0.1f32;
        let mut count = 1usize;
        let va = [[1.0f32, 0.0, 0.0, 0.0]; 9];
        let r = sweep(
            &mut v,
            &mut t,
            &mut count,
            &[4.0, 0.0, 0.0],
            &va,
            0,
            &mut best,
            SEP_FAR,
            SEP_NEAR,
            CN,
            CP,
        );
        assert_eq!(r, 0);
        assert!((best - 0.1).abs() < 1e-6);
    }
}

/// Sweeps a clipped polygon against a set of world faces, recording the closest hit.
///
/// `faces` packs each candidate face as 13 floats: the face plane `[nx, ny, nz,
/// d]` (its normal feeds the front-face test) then a 9-float (3-vertex) polygon.
/// For each *front* face (`normal . sweep_dir <= front_eps`) the 3-vertex polygon
/// is clipped against the first `clip_plane_count` of `clip_planes`, then swept
/// along `vert_array[vert_base]` from `sweep_dir`. The smallest swept distance
/// `<= best` wins, updating `best` and writing that face's index to `out_face`.
/// Returns `1` if any hit was recorded. All epsilons are the host band /
/// separation constants.
// The thirteen parameters are the original's calling convention: face set, clip
// plane set, sweep basis, the two out-params and five band constants — an ABI
// fact, not a signature that wants a params struct. The front-face test also
// stays `!(dot <= front_eps)`, so a NaN dot processes the face as it does in
// the original, where `dot > front_eps` would skip it.
#[allow(clippy::too_many_arguments, clippy::neg_cmp_op_on_partial_ord)]
pub fn collision_sweep_polygon_against_faces__632700(
    faces: &[[f32; 13]],
    sweep_dir: &[f32; 3],
    clip_planes: &[[f32; 4]],
    clip_plane_count: usize,
    vert_array: &[[f32; 4]],
    vert_base: usize,
    best: &mut f32,
    out_face: &mut u32,
    front_eps: f32,
    sep_far: f32,
    sep_near: f32,
    clip_eps_neg: f32,
    clip_eps_pos: f32,
) -> u32 {
    let mut hit = 0u32;
    for (idx, face) in faces.iter().enumerate() {
        let dot = face[0] * sweep_dir[0] + face[1] * sweep_dir[1] + face[2] * sweep_dir[2];
        if !(dot <= front_eps) {
            continue;
        }
        // Fresh scratch polygon: 3 verts copied from the face, tags = -1
        // (0xffffffff), count = 3.
        let mut verts = [0.0f32; 45];
        let mut tags = [f32::from_bits(0xffff_ffff); 15];
        verts[..9].copy_from_slice(&face[4..13]);

        let mut count = 3usize;
        for (i, plane) in clip_planes.iter().take(clip_plane_count).enumerate() {
            count = collision_clip_polygon_by_plane__6318c0(
                &mut verts,
                &mut tags,
                count,
                plane,
                i as f32,
                clip_eps_neg,
                clip_eps_pos,
            );
            if count == 0 {
                break;
            }
        }
        if count == 0 {
            continue;
        }

        let mut local = f32::MAX;
        let got = collision_ray_polygon_sweep_distance__632830(
            &mut verts,
            &mut tags,
            &mut count,
            sweep_dir,
            vert_array,
            vert_base,
            &mut local,
            sep_far,
            sep_near,
            clip_eps_neg,
            clip_eps_pos,
        );
        if got != 0 && local <= *best {
            *best = local;
            *out_face = idx as u32;
            hit = 1;
        }
    }
    hit
}

#[cfg(test)]
mod tests_collision_sweep_polygon_against_faces__632700 {
    use super::collision_sweep_polygon_against_faces__632700 as sweep_faces;

    // The front-face epsilon repeats the host's stored constant at full precision;
    // trimming digits would change the value under test.
    #[allow(clippy::excessive_precision)]
    const FRONT: f32 = -9.999_999_7e-6;
    const SEP_FAR: f32 = -9.536_743e-7;
    const SEP_NEAR: f32 = -0.027_777_778;
    const CN: f32 = -0.0013888889;
    const CP: f32 = 0.0013888889;

    #[test]
    fn back_facing_face_is_skipped() {
        let f = [[
            1.0f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        ]];
        let mut best = 10.0f32;
        let mut of = 99u32;
        let r = sweep_faces(
            &f,
            &[5.0, 0.0, 0.0],
            &[],
            0,
            &[[1.0, 0.0, 0.0, 0.0]; 9],
            0,
            &mut best,
            &mut of,
            FRONT,
            SEP_FAR,
            SEP_NEAR,
            CN,
            CP,
        );
        assert_eq!(r, 0);
        assert_eq!(of, 99);
    }

    #[test]
    fn front_face_clip_wipeout_records_no_hit() {
        let mut f = [0.0f32; 13];
        f[0] = -1.0;
        f[4..13].copy_from_slice(&[0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0]);
        let faces = [f];
        // Interior s = -(z + 100) < -eps for every vertex: the polygon clips away.
        let clip = [[0.0f32, 0.0, 1.0, 100.0]];
        let mut best = 10.0f32;
        let mut of = 99u32;
        let r = sweep_faces(
            &faces,
            &[5.0, 0.0, 0.0],
            &clip,
            1,
            &[[1.0, 0.0, 0.0, 0.0]; 9],
            0,
            &mut best,
            &mut of,
            FRONT,
            SEP_FAR,
            SEP_NEAR,
            CN,
            CP,
        );
        assert_eq!(r, 0);
    }

    #[test]
    fn front_face_hit_records_and_lowers_best() {
        let mut f = [0.0f32; 13];
        f[0] = -1.0;
        f[4..13].copy_from_slice(&[0.5, 0.0, 0.0, 0.5, 1.0, 0.0, 0.5, 0.0, 1.0]);
        let faces = [f];
        let va = [[1.0f32, 0.0, 0.0, 0.0]; 9];
        let mut best = 100.0f32;
        let mut of = 99u32;
        let r = sweep_faces(
            &faces,
            &[4.0, 0.0, 0.0],
            &[],
            0,
            &va,
            0,
            &mut best,
            &mut of,
            FRONT,
            SEP_FAR,
            SEP_NEAR,
            CN,
            CP,
        );
        assert_eq!(r, 1);
        assert!(best < 100.0, "best={best}");
        assert_eq!(of, 0);
    }
}

/// Builds one collision-plane record from a transformed triangle.
///
/// The unit face normal `n = normalize((q1 - q0) x (q2 - q0))` and the plane
/// constant `d = -dot(n, q0)`, returned as `[n_x, n_y, n_z, d]`.
///
/// `target_len` is the host's normalize constant (the length the precise
/// normalizer scales to); `precise` selects between the reference's two builders,
/// and it is the arm taken when the flag at `0xc9e388` is **zero**. That flag is
/// set to 1 whenever `CPUID` reports SSE (`0x6920a7`), so on any real processor
/// the inline arm below is the one that runs.
///
/// That arm is not this function's own code: `0x671cc0` emits the identical
/// sixty-six floating-point and SSE operations that `0x6aadc0` does, so it calls
/// `tile_collect_triangle_plane__6aadc0`. Notably it ends in a raw `rsqrtss`
/// (`0x671e5a`) with no Newton-Raphson step, which a full `1.0 / sqrt` does not
/// approximate — that is a different algorithm, not a rounding difference, and
/// computing it "more precisely" is a divergence rather than an upgrade.
pub fn build_mesh_triangle_planes__671cc0(
    q0: &[f32; 3],
    q1: &[f32; 3],
    q2: &[f32; 3],
    target_len: f64,
    precise: bool,
) -> [f32; 4] {
    if precise {
        let e1 = [q1[0] - q0[0], q1[1] - q0[1], q1[2] - q0[2]];
        let e2 = [q2[0] - q0[0], q2[1] - q0[1], q2[2] - q0[2]];
        let mut n = super::vector::c3_vector__cross__672130(&e1, &e2);
        super::vector::normalize3(&mut n, target_len);
        let d = -super::vector::c3_vector__dot__602630(&n, q0);
        return [n[0], n[1], n[2], d];
    }
    super::world::tile_collect_triangle_plane__6aadc0(q0, q1, q2)
}

#[cfg(test)]
mod tests_build_mesh_triangle_planes__671cc0 {
    use super::build_mesh_triangle_planes__671cc0 as plane;

    /// Relative error of a single `rsqrtss` with no Newton-Raphson step.
    ///
    /// The instruction is specified to within 1.5 * 2^-12, so anything the inline
    /// arm normalizes is unit only to about that. The tests here assert the
    /// original's accuracy; tightening them would pin a reimplementation that had
    /// quietly upgraded the algorithm.
    const RSQRTSS_TOL: f32 = 4.0e-4;

    fn eval(p: &[f32; 4], v: &[f32; 3]) -> f32 {
        p[0] * v[0] + p[1] * v[1] + p[2] * v[2] + p[3]
    }

    #[test]
    fn known_axis_aligned_plane() {
        // Counter-clockwise triangle in the z = 5 plane: n = +z, d = -5.
        let q0 = [0.0_f32, 0.0, 5.0];
        let q1 = [1.0_f32, 0.0, 5.0];
        let q2 = [0.0_f32, 1.0, 5.0];
        // The inline arm scales by a raw `rsqrtss` estimate, which is a ~12-bit
        // approximation: its unit normal is only unit to about 2^-12, and the
        // tolerances here are the original's precision, not a target for ours.
        for (precise, tol) in [(true, 1e-6_f32), (false, RSQRTSS_TOL)] {
            let p = plane(&q0, &q1, &q2, 1.0, precise);
            assert!(p[0].abs() < tol && p[1].abs() < tol, "{p:?}");
            assert!((p[2] - 1.0).abs() < tol, "{p:?}");
            assert!((p[3] + 5.0).abs() < 5.0 * tol, "{p:?}");
        }
    }

    #[test]
    fn all_three_vertices_lie_on_the_plane() {
        let q0 = [1.5_f32, -2.0, 0.25];
        let q1 = [4.0_f32, 1.0, -3.0];
        let q2 = [-2.0_f32, 5.0, 2.0];
        for precise in [true, false] {
            let p = plane(&q0, &q1, &q2, 1.0, precise);
            for v in [&q0, &q1, &q2] {
                assert!(eval(&p, v).abs() < 1e-4, "precise={precise} {p:?} {v:?}");
            }
        }
    }

    #[test]
    fn normal_is_unit_length() {
        let q0 = [10.0_f32, 20.0, 30.0];
        let q1 = [11.0_f32, 22.0, 29.0];
        let q2 = [9.5_f32, 21.0, 31.5];
        for precise in [true, false] {
            let p = plane(&q0, &q1, &q2, 1.0, precise);
            let len = (f64::from(p[0]) * f64::from(p[0])
                + f64::from(p[1]) * f64::from(p[1])
                + f64::from(p[2]) * f64::from(p[2]))
            .sqrt();
            let tol = if precise {
                1e-5
            } else {
                f64::from(RSQRTSS_TOL)
            };
            assert!((len - 1.0).abs() < tol, "precise={precise} len={len}");
        }
    }

    #[test]
    fn winding_flips_the_plane() {
        let q0 = [1.0_f32, 2.0, 3.0];
        let q1 = [4.0_f32, -1.0, 2.0];
        let q2 = [0.0_f32, 1.0, -2.0];
        let p = plane(&q0, &q1, &q2, 1.0, true);
        let q = plane(&q0, &q2, &q1, 1.0, true);
        for i in 0..4 {
            assert!((p[i] + q[i]).abs() < 1e-5, "{p:?} vs {q:?}");
        }
    }

    #[test]
    fn paths_agree_within_f32_tolerance() {
        let q0 = [3.0_f32, -7.0, 11.0];
        let q1 = [5.5_f32, -6.0, 10.0];
        let q2 = [2.0_f32, -8.5, 12.5];
        let a = plane(&q0, &q1, &q2, 1.0, true);
        let b = plane(&q0, &q1, &q2, 1.0, false);
        // The two arms are different algorithms, not two roundings of one: the
        // inline one's reciprocal square root carries ~12 bits, so they agree only
        // to that, scaled by the lane's magnitude.
        for i in 0..4 {
            let tol = RSQRTSS_TOL * a[i].abs().max(1.0);
            assert!((a[i] - b[i]).abs() < tol, "lane {i}: {a:?} vs {b:?}");
        }
    }

    #[test]
    fn degenerate_triangle_propagates_nan() {
        // Collinear points: zero-length normal divides by zero.
        let q0 = [0.0_f32, 0.0, 0.0];
        let q1 = [1.0_f32, 1.0, 1.0];
        let q2 = [2.0_f32, 2.0, 2.0];
        for precise in [true, false] {
            let p = plane(&q0, &q1, &q2, 1.0, precise);
            assert!(p[0].is_nan() && p[1].is_nan() && p[2].is_nan(), "{p:?}");
        }
    }

    #[test]
    fn rotation_invariance_of_offset() {
        // Rotating the triangle around z keeps |d| (distance to origin)
        // for a plane parallel to z.
        let d0 = 4.0_f32;
        let q0 = [d0, 0.0, 0.0];
        let q1 = [d0, 1.0, 0.0];
        let q2 = [d0, 0.0, 1.0];
        let p = plane(&q0, &q1, &q2, 1.0, true);
        let (s, c) = (0.6_f32, 0.8_f32); // 3-4-5 rotation
        let rot = |v: &[f32; 3]| [c * v[0] - s * v[1], s * v[0] + c * v[1], v[2]];
        let r = plane(&rot(&q0), &rot(&q1), &rot(&q2), 1.0, true);
        assert!((p[3] - r[3]).abs() < 1e-5, "{p:?} vs {r:?}");
    }
}

/// One interior-node step of `CollideBspNode_TraceSegment`.
///
/// How a segment relates to an axis-aligned split plane, and (for a straddle)
/// where it crosses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BspTraceStep {
    /// The segment misses the node's clip slab on the split axis — stop.
    Reject,
    /// An endpoint sits inside the plane band.
    ///
    /// Descend the high child with the full segment (slab min raised to the
    /// split), then the low child (slab max lowered to the split).
    Band,
    /// Both endpoints are above the band: descend only the high child.
    HighOnly,
    /// Both endpoints are below the band: descend only the low child.
    LowOnly,
    /// The segment straddles the plane entering from the high side.
    ///
    /// The high child takes `[p0, mid]`, then the low child takes `[mid, p1]`.
    SplitHighFirst { mid: [f32; 3] },
    /// The segment straddles the plane entering from the low side.
    ///
    /// The low child takes `[p0, mid]`, then the high child takes `[mid, p1]`.
    SplitLowFirst { mid: [f32; 3] },
}

/// Classifies a traced segment against one BSP split plane.
///
/// `seg` packs the segment start in `[0..3]` and end in `[3..6]`; `clip` is
/// the node's clip slab as `[min_x, min_y, min_z, max_x, max_y, max_z]`;
/// `axis` (0..3) and `split` come from the node. `band_lo`/`band_hi` bound
/// the on-plane tolerance band for `p - split`, and `front_gate` decides
/// which side a straddling segment enters from. Every compare keeps the
/// reference's exact NaN routing: the slab rejects only on two ordered
/// less-than results, the band/above/below classifications are ordered (a
/// NaN distance falls through to the straddle path), and the straddle
/// midpoint is the per-component f32 lerp `p0 + (p1 - p0) * fa / (fa - fb)`.
pub fn collide_bsp_node_trace_segment__6bc370(
    seg: &[f32; 6],
    clip: &[f32; 6],
    axis: usize,
    split: f32,
    band_lo: f32,
    band_hi: f32,
    front_gate: f32,
) -> BspTraceStep {
    use BspTraceStep as Step;
    let a = axis;

    // Slab early-out: reject only when BOTH endpoints test ordered-below on a
    // side (a NaN difference passes, matching the unordered branch).
    if seg[a] - clip[a] < band_lo && seg[a + 3] - clip[a] < band_lo {
        return Step::Reject;
    }
    if clip[a + 3] - seg[a] < band_hi && clip[a + 3] - seg[a + 3] < band_hi {
        return Step::Reject;
    }

    let fa = seg[a] - split;
    let fb = seg[a + 3] - split;

    // An endpoint inside the on-plane band visits both children un-split.
    if (fa >= band_lo && fa <= band_hi) || (fb >= band_lo && fb <= band_hi) {
        return Step::Band;
    }
    if fa > band_hi && fb > band_hi {
        return Step::HighOnly;
    }
    if fa < band_lo && fb < band_lo {
        return Step::LowOnly;
    }

    // Straddle: per-component f32 lerp to the plane crossing.
    let t = fa / (fa - fb);
    let mid = [
        seg[0] + (seg[3] - seg[0]) * t,
        seg[1] + (seg[4] - seg[1]) * t,
        seg[2] + (seg[5] - seg[2]) * t,
    ];
    if fa > front_gate {
        Step::SplitHighFirst { mid }
    } else {
        Step::SplitLowFirst { mid }
    }
}

#[cfg(test)]
mod tests_collide_bsp_node_trace_segment__6bc370 {
    use super::{BspTraceStep as Step, collide_bsp_node_trace_segment__6bc370 as step};

    const CLIP: [f32; 6] = [-10.0, -10.0, -10.0, 10.0, 10.0, 10.0];
    const E_LO: f32 = -0.01;
    const E_HI: f32 = 0.01;
    const GATE: f32 = 0.0;

    fn seg(p0: [f32; 3], p1: [f32; 3]) -> [f32; 6] {
        [p0[0], p0[1], p0[2], p1[0], p1[1], p1[2]]
    }

    #[test]
    fn rejects_segment_outside_slab() {
        // Both endpoints far below the slab minimum on x.
        let s = seg([-20.0, 0.0, 0.0], [-15.0, 0.0, 0.0]);
        assert_eq!(step(&s, &CLIP, 0, 0.0, E_LO, E_HI, GATE), Step::Reject);
        // Both endpoints far above the slab maximum on x.
        let s = seg([20.0, 0.0, 0.0], [15.0, 0.0, 0.0]);
        assert_eq!(step(&s, &CLIP, 0, 0.0, E_LO, E_HI, GATE), Step::Reject);
    }

    #[test]
    fn one_endpoint_in_slab_passes() {
        let s = seg([-20.0, 0.0, 0.0], [0.0, 0.0, 0.0]);
        assert_ne!(step(&s, &CLIP, 0, 5.0, E_LO, E_HI, GATE), Step::Reject);
    }

    #[test]
    fn both_above_visits_high_only() {
        let s = seg([3.0, 0.0, 0.0], [5.0, 0.0, 0.0]);
        assert_eq!(step(&s, &CLIP, 0, 1.0, E_LO, E_HI, GATE), Step::HighOnly);
    }

    #[test]
    fn both_below_visits_low_only() {
        let s = seg([-3.0, 0.0, 0.0], [-5.0, 0.0, 0.0]);
        assert_eq!(step(&s, &CLIP, 0, 1.0, E_LO, E_HI, GATE), Step::LowOnly);
    }

    #[test]
    fn endpoint_on_plane_takes_band() {
        let s = seg([1.0, 0.0, 0.0], [5.0, 0.0, 0.0]);
        assert_eq!(step(&s, &CLIP, 0, 1.0, E_LO, E_HI, GATE), Step::Band);
        // Band edges are inclusive (`>=`/`<=`).
        let s = seg([1.0 + E_HI, 0.0, 0.0], [5.0, 0.0, 0.0]);
        assert_eq!(step(&s, &CLIP, 0, 1.0, E_LO, E_HI, GATE), Step::Band);
    }

    #[test]
    fn straddle_from_high_side_splits_high_first() {
        let s = seg([4.0, 1.0, 2.0], [-2.0, 7.0, 8.0]);
        match step(&s, &CLIP, 0, 1.0, E_LO, E_HI, GATE) {
            Step::SplitHighFirst { mid } => {
                // The crossing lies on the plane and on the segment.
                assert!((mid[0] - 1.0).abs() < 1e-5, "mid = {mid:?}");
                // t = fa / (fa - fb) = 3 / 6 = 0.5 on the other axes.
                assert!((mid[1] - 4.0).abs() < 1e-5, "mid = {mid:?}");
                assert!((mid[2] - 5.0).abs() < 1e-5, "mid = {mid:?}");
            }
            other => panic!("expected SplitHighFirst, got {other:?}"),
        }
    }

    #[test]
    fn straddle_from_low_side_splits_low_first() {
        let s = seg([-2.0, 0.0, 0.0], [4.0, 6.0, 0.0]);
        match step(&s, &CLIP, 0, 1.0, E_LO, E_HI, GATE) {
            Step::SplitLowFirst { mid } => {
                assert!((mid[0] - 1.0).abs() < 1e-5, "mid = {mid:?}");
                assert!((mid[1] - 3.0).abs() < 1e-5, "mid = {mid:?}");
            }
            other => panic!("expected SplitLowFirst, got {other:?}"),
        }
    }

    #[test]
    fn split_axis_selection() {
        // Same geometry, classified on y instead of x.
        let s = seg([0.0, 4.0, 0.0], [0.0, -2.0, 0.0]);
        assert!(matches!(
            step(&s, &CLIP, 1, 1.0, E_LO, E_HI, GATE),
            Step::SplitHighFirst { .. }
        ));
        assert_eq!(step(&s, &CLIP, 2, 1.0, E_LO, E_HI, GATE), Step::LowOnly);
    }

    #[test]
    fn nan_distance_falls_through_to_straddle() {
        // A NaN start coordinate is unordered in every band/side compare and
        // must reach the straddle path (taking the low-first branch).
        let s = seg([f32::NAN, 0.0, 0.0], [4.0, 0.0, 0.0]);
        assert!(matches!(
            step(&s, &CLIP, 0, 1.0, E_LO, E_HI, GATE),
            Step::SplitLowFirst { .. }
        ));
    }

    #[test]
    fn nan_does_not_reject_slab() {
        let s = seg([f32::NAN, 0.0, 0.0], [f32::NAN, 0.0, 0.0]);
        assert_ne!(step(&s, &CLIP, 0, 1.0, E_LO, E_HI, GATE), Step::Reject);
    }
}

/// `CollideLeaf_RayTriangleMesh` vertex-outcode kernel.
///
/// The 6-bit AABB outcode of vertex `v` against the box spanned by segment
/// endpoints `a` and `b`.
///
/// Per axis the low/high bounds are picked with a single ordered `>` test — an
/// unordered (NaN) endpoint pair keeps `(a, b)` unswapped, exactly like the
/// reference's compare-and-swap. Bits: `0x20`/`0x10` = below-low/above-high on
/// x, `0x08`/`0x04` on y, `0x02`/`0x01` on z. Both bound tests are strict and
/// ordered, so a vertex exactly on a face (or a NaN component) sets no bits.
/// A triangle whose three vertex outcodes share a set bit lies entirely
/// outside one face of the box and can be trivially rejected.
pub fn collide_leaf_ray_triangle_mesh__6b88e0(a: &[f32; 3], b: &[f32; 3], v: &[f32; 3]) -> u8 {
    let mut code = 0_u8;
    let bits: [(u8, u8); 3] = [(0x20, 0x10), (0x08, 0x04), (0x02, 0x01)];
    for ((&vc, (&pa, &pb)), (below_bit, above_bit)) in
        v.iter().zip(a.iter().zip(b.iter())).zip(bits)
    {
        // Swap only on an ordered `pa > pb`; NaN keeps the original order.
        let (lo, hi) = if pa > pb { (pb, pa) } else { (pa, pb) };
        if vc < lo {
            code |= below_bit;
        }
        if vc > hi {
            code |= above_bit;
        }
    }
    code
}

#[cfg(test)]
mod tests_collide_leaf_ray_triangle_mesh__6b88e0 {
    use super::collide_leaf_ray_triangle_mesh__6b88e0 as outcode;

    const A: [f32; 3] = [-1.0, -2.0, -3.0];
    const B: [f32; 3] = [1.0, 2.0, 3.0];

    #[test]
    fn inside_box_is_zero() {
        assert_eq!(outcode(&A, &B, &[0.0, 0.0, 0.0]), 0);
        assert_eq!(outcode(&A, &B, &[0.5, -1.5, 2.5]), 0);
    }

    #[test]
    fn each_face_sets_its_bit() {
        assert_eq!(outcode(&A, &B, &[-2.0, 0.0, 0.0]), 0x20);
        assert_eq!(outcode(&A, &B, &[2.0, 0.0, 0.0]), 0x10);
        assert_eq!(outcode(&A, &B, &[0.0, -3.0, 0.0]), 0x08);
        assert_eq!(outcode(&A, &B, &[0.0, 3.0, 0.0]), 0x04);
        assert_eq!(outcode(&A, &B, &[0.0, 0.0, -4.0]), 0x02);
        assert_eq!(outcode(&A, &B, &[0.0, 0.0, 4.0]), 0x01);
    }

    #[test]
    fn corner_combines_bits() {
        assert_eq!(outcode(&A, &B, &[-2.0, 3.0, 4.0]), 0x20 | 0x04 | 0x01);
        assert_eq!(outcode(&A, &B, &[2.0, -3.0, -4.0]), 0x10 | 0x08 | 0x02);
    }

    #[test]
    fn on_face_is_strictly_inside() {
        // Strict compares: a vertex exactly on a bound sets no bit.
        assert_eq!(outcode(&A, &B, &[1.0, 2.0, 3.0]), 0);
        assert_eq!(outcode(&A, &B, &[-1.0, -2.0, -3.0]), 0);
    }

    #[test]
    fn endpoint_order_is_irrelevant_when_finite() {
        // The box spans min/max per axis regardless of which endpoint holds it.
        let mixed_a = [1.0_f32, -2.0, 3.0];
        let mixed_b = [-1.0_f32, 2.0, -3.0];
        for v in [
            [0.0_f32, 0.0, 0.0],
            [-2.0, 3.0, 4.0],
            [2.0, -3.0, -4.0],
            [0.5, 1.5, -2.5],
        ] {
            assert_eq!(
                outcode(&mixed_a, &mixed_b, &v),
                outcode(&A, &B, &v),
                "v = {v:?}"
            );
        }
    }

    #[test]
    fn nan_vertex_component_sets_no_bits_on_that_axis() {
        assert_eq!(outcode(&A, &B, &[f32::NAN, 0.0, 0.0]), 0);
        assert_eq!(outcode(&A, &B, &[f32::NAN, 3.0, 0.0]), 0x04);
    }

    #[test]
    fn nan_endpoint_keeps_unswapped_order() {
        // x bounds become (NaN, 1.0): the below test is always false, the
        // above test still compares against 1.0.
        let a = [f32::NAN, -2.0, -3.0];
        assert_eq!(outcode(&a, &B, &[2.0, 0.0, 0.0]), 0x10);
        assert_eq!(outcode(&a, &B, &[-5.0, 0.0, 0.0]), 0);
    }
}

/// `Collision::BuildSweepPrism` vertex builder.
///
/// The 9 swept-prism corner vertices (`[x, y, z]` x9 = 27 contiguous `f32`).
///
/// Vertex 0 is `center` verbatim. The lower ring (vertices 1-4) sits at
/// `center.z + radius * K` and the upper ring (vertices 5-8) at
/// `center.z + height`; both rings span `center.xy +/- radius`. `K` is the
/// `.rdata` constant at `0x80c740` (`0x3fecb91b`). The stock keeps the
/// `radius * K` product on the x87 stack un-narrowed across the whole lower
/// ring and rounds only once after adding `center.z`, so the lower-ring `z` is
/// computed in `f64` and narrowed; every other component is a single `f32` add.
pub fn collision_build_sweep_prism_verts__631be0(
    center: &[f32; 3],
    height: f32,
    radius: f32,
) -> [f32; 27] {
    // Constant at `0x80c740` = `0x3fecb91b`. Full precision so the literal
    // round-trips to the exact stored f32 (a shorter form lands one ULP low).
    // Those digits are the value, so the literal cannot be shortened.
    #[allow(clippy::excessive_precision)]
    const K: f32 = 1.849_398_970_603_942_9;

    let cx = center[0];
    let cy = center[1];
    let cz = center[2];

    // x87 path: `FLD radius; FMUL [K]` leaves the product at 80-bit precision on
    // the FPU stack, and each lower-ring `z` adds `cz` to it before the single
    // narrowing `FSTP`. An `f32 * f32` is exact in `f64`, so this tracks the
    // un-narrowed product faithfully.
    let rtk = f64::from(radius) * f64::from(K);
    let lower_z = super::f64_to_f32(rtk + f64::from(cz));
    // Upper ring: `FLD height; FADD cz` — a single f32 add.
    let upper_z = height + cz;

    [
        // vertex 0: center copied verbatim
        cx,
        cy,
        cz,
        // vertex 1: (-r+cx, -r+cy, lower_z)
        -radius + cx,
        -radius + cy,
        lower_z,
        // vertex 2: (-r+cx, +r+cy, lower_z)
        -radius + cx,
        radius + cy,
        lower_z,
        // vertex 3: (+r+cx, +r+cy, lower_z)
        radius + cx,
        radius + cy,
        lower_z,
        // vertex 4: (+r+cx, -r+cy, lower_z)
        radius + cx,
        -radius + cy,
        lower_z,
        // vertex 5: (-r+cx, -r+cy, upper_z)
        -radius + cx,
        -radius + cy,
        upper_z,
        // vertex 6: (-r+cx, +r+cy, upper_z)
        -radius + cx,
        radius + cy,
        upper_z,
        // vertex 7: (+r+cx, +r+cy, upper_z)
        radius + cx,
        radius + cy,
        upper_z,
        // vertex 8: (+r+cx, -r+cy, upper_z)
        radius + cx,
        -radius + cy,
        upper_z,
    ]
}

#[cfg(test)]
mod tests_collision_build_sweep_prism__631be0 {
    use super::collision_build_sweep_prism_verts__631be0 as f;

    /// Independent oracle.
    ///
    /// Builds the 9 vertices from an explicit table of per-vertex
    /// `(sign_x, sign_y, z)` selectors rather than the kernel's hand-unrolled
    /// layout, so a transposed component or wrong sign diverges.
    fn oracle(center: &[f32; 3], height: f32, radius: f32) -> [f32; 27] {
        // The oracle repeats the kernel's `0x3fecb91b` at full precision so both land
        // on the same stored f32; a shorter literal lands one ULP low.
        #[allow(clippy::excessive_precision)]
        const K: f32 = 1.849_398_970_603_942_9;
        let [cx, cy, cz] = *center;
        let lz = ((f64::from(radius) * f64::from(K)) + f64::from(cz)) as f32;
        let uz = height + cz;
        let ring: [(f32, f32, f32); 8] = [
            (-1.0, -1.0, lz),
            (-1.0, 1.0, lz),
            (1.0, 1.0, lz),
            (1.0, -1.0, lz),
            (-1.0, -1.0, uz),
            (-1.0, 1.0, uz),
            (1.0, 1.0, uz),
            (1.0, -1.0, uz),
        ];
        let mut out = [0.0_f32; 27];
        out[0] = cx;
        out[1] = cy;
        out[2] = cz;
        for (i, &(sx, sy, z)) in ring.iter().enumerate() {
            let b = 3 + i * 3;
            out[b] = sx * radius + cx;
            out[b + 1] = sy * radius + cy;
            out[b + 2] = z;
        }
        out
    }

    #[test]
    fn matches_oracle() {
        for &(c, h, r) in &[
            ([0.0_f32, 0.0, 0.0], 5.0_f32, 2.0_f32),
            ([1.0, 2.0, 3.0], 5.0, 2.0),
            ([-4.5, 10.25, -7.0], 12.0, 1.5),
            ([100.0, -200.0, 50.0], 0.0, 3.25),
        ] {
            assert_eq!(f(&c, h, r), oracle(&c, h, r), "c={c:?} h={h} r={r}");
        }
    }

    #[test]
    fn known_values() {
        let v = f(&[1.0, 2.0, 3.0], 5.0, 2.0);
        // vertex 0 = center verbatim
        assert_eq!([v[0], v[1], v[2]], [1.0, 2.0, 3.0]);
        // lower-ring z = (2.0 * K) + 3.0 narrowed to f32 (0x40d65c8e)
        let lz = v[5];
        assert_eq!(lz.to_bits(), 0x40d6_5c8e);
        // upper-ring z = height(5) + cz(3)
        assert_eq!(v[17], 8.0);
        // lower-ring xy: center -/+ radius
        assert_eq!([v[3], v[4]], [-1.0, 0.0]); // (-r+cx, -r+cy)
        assert_eq!([v[9], v[10]], [3.0, 4.0]); // (+r+cx, +r+cy)
        // every lower-ring z equal, every upper-ring z equal
        for i in 1..=4 {
            assert_eq!(v[i * 3 + 2], lz);
        }
        for i in 5..=8 {
            assert_eq!(v[i * 3 + 2], 8.0);
        }
    }
}
/// `PolyClip::ClipToScreenRect` 3-way band classifier.
///
/// Class `1` (inside) when `d > band_hi`, class `2` (outside) when
/// `d < band_lo`, else class `0` (on-plane: kept, never a crossing endpoint).
///
/// Both stock compares are strict `FCOMP`s that route the unordered result to
/// class 0: the first (`TEST AH,0x41; JNZ`) accepts class 1 only for an
/// ordered `d > band_hi`; the second (`TEST AH,0x5; JP`) accepts class 2 only
/// for an ordered `d < band_lo` (NaN parity-jumps to class 0). The natural
/// strict `>`/`<` chain reproduces that polarity exactly — a NaN distance
/// fails both compares and classifies 0. Do NOT rewrite with `<=`/`>=`: that
/// would misroute NaN and the band boundaries.
pub fn poly_clip_classify__6b4a80(d: f32, band_hi: f32, band_lo: f32) -> u32 {
    if d > band_hi {
        1
    } else if d < band_lo {
        2
    } else {
        0
    }
}

/// `PolyClip::ClipToScreenRect` plane distance.
///
/// The homogeneous dot of one packed `(x, y, w)` polygon vertex with one
/// 3-coefficient clip plane, in the stock x87 op order `(w*c2 + x*c0) + y*c1`
/// (with `w` as the third dot component). The x87 keeps the sum at extended
/// precision and narrows once at the f32 store — tracked here in `f64`
/// (products of two f32 are exact) with the same single narrowing.
pub fn poly_clip_plane_distance__6b4a80(v: &[f32; 3], plane: &[f32; 3]) -> f32 {
    let d = (f64::from(v[2]) * f64::from(plane[2]) + f64::from(v[0]) * f64::from(plane[0]))
        + f64::from(v[1]) * f64::from(plane[1]);
    super::f64_to_f32(d)
}

/// `PolyClip::ClipToScreenRect` edge-crossing parameter `t = d_i / (d_i - d_next)`.
///
/// (FSUB + FDIVR) over the two stored f32 plane distances. Stock keeps `t`
/// un-narrowed on the x87 stack across all three lerped components — mirrored
/// by returning `f64`. The driver only forms a crossing for opposite non-zero
/// classes, so the denominator is nonzero in practice; a degenerate equal pair
/// divides to ±inf/NaN exactly like stock (no added guard).
pub fn poly_clip_crossing_t__6b4a80(d_i: f32, d_next: f32) -> f64 {
    f64::from(d_i) / (f64::from(d_i) - f64::from(d_next))
}

/// One component of the `PolyClip::ClipToScreenRect` homogeneous crossing lerp.
///
/// `out = (v_next - v_i) * t + v_i` (FSUB/FMUL/FADD chain), narrowed once at
/// the f32 store.
pub fn poly_clip_lerp_component__6b4a80(v_i: f32, v_next: f32, t: f64) -> f32 {
    super::f64_to_f32((f64::from(v_next) - f64::from(v_i)) * t + f64::from(v_i))
}

/// The scratch regions [`poly_clip_to_screen_rect__6b4a80`] writes.
///
/// The two ping-pong polygon buffers (packed 3 f32 per vertex) and the
/// per-vertex plane-distance / class arrays (both also written at index
/// `[count]` for the wrap sentinel). The live adapter points these at the
/// game's fixed static globals (0xcbe138 / 0xcbe1f8 / 0xcbe0f0 / 0xcbe0b0);
/// host tests use local arrays. Like stock, NO region is capacity-checked.
pub struct PolyClipScratch {
    pub buf_a: *mut f32,
    pub buf_b: *mut f32,
    pub dist: *mut f32,
    pub class: *mut u32,
}

/// Raw 3-dword vertex copy between the packed (stride-0xc) polygon buffers.
///
/// Per-component interleaved read-then-write dword copies in stock's exact
/// order, bit-exact (NaN payloads preserved).
///
/// # Safety
/// `src` slot `i` must be readable and `dst` slot `j` writable.
unsafe fn poly_clip_copy_vertex(src: *const f32, i: usize, dst: *mut f32, j: usize) {
    for k in 0..3 {
        // SAFETY: caller contract — slot `i` component `k` is readable.
        let bits = unsafe { src.wrapping_add(i * 3 + k).cast::<u32>().read_unaligned() };
        // SAFETY: caller contract — slot `j` component `k` is writable.
        unsafe {
            dst.wrapping_add(j * 3 + k)
                .cast::<u32>()
                .write_unaligned(bits);
        }
    }
}

/// `PolyClip::ClipToScreenRect` (0x6b4a80) Sutherland–Hodgman driver.
///
/// Stages the caller's stride-0x10 `(x, y, _, w)` vertices into ping buffer A
/// as packed `(x, y, w)` triples (raw dword copies; component `[2]`/z
/// skipped), then clips against the four `planes` rows (stride 4 f32,
/// coefficients at `[0..3]`, `[3]` unused padding), ping-ponging A→B→A→B→A.
/// Per plane it stores every source vertex's f32 plane distance and 3-way
/// class (wrap sentinel copied to slot `[src_count]` — a self-copy of slot 0
/// when the count is 0), then emits class-0/1 vertices verbatim and a lerped
/// crossing point for every edge `i → (i+1) % src_count` (integer modulo)
/// whose next class is non-zero and differs from `class[i]`. A crossing is
/// never tested from a class-0 vertex, faithfully reproducing stock's
/// asymmetry (an outside→on-plane or on-plane→outside edge emits no
/// intersection). `*out_count` is rewritten after every surviving plane,
/// exactly like stock; a surviving polygon writes its final buffer's own
/// address to `*out_poly` (pointer identity — always buffer A, the plane-3
/// destination), a fully-clipped or empty one writes `*out_poly = 0` then
/// `*out_count = 0`.
///
/// # Safety
/// `verts` must address `count` readable stride-0x10 vertices (it is never
/// dereferenced when `count == 0`); `planes` must address 4 rows of 4 f32;
/// `out_poly` / `out_count` must be writable. The scratch regions must be
/// writable for every index the UNGUARDED algorithm forms: like stock there
/// is no capacity check anywhere, so an oversized `count` (or a
/// crossing-heavy emit pass) walks past the region ends exactly as the
/// original overflows its static globals.
// The eight parameters are the original's calling convention: vertex pointer
// and count, plane pointer, the band pair, the scratch regions and the two
// out-params. That is an ABI fact, not a signature that wants a params struct.
#[allow(clippy::too_many_arguments)]
pub unsafe fn poly_clip_to_screen_rect__6b4a80(
    verts: *const f32,
    count: u32,
    planes: *const f32,
    band_hi: f32,
    band_lo: f32,
    scratch: &PolyClipScratch,
    out_poly: *mut *mut f32,
    out_count: *mut u32,
) {
    let bufs = [scratch.buf_a, scratch.buf_b];
    let mut counts = [count, 0_u32];

    // Stage the input into ping buffer A: components [0]=x, [1]=y, [3]=w of
    // each stride-0x10 vertex, copied as raw unaligned dwords (bit-exact) at
    // output stride 3 — never a packed [f32; 4] load.
    for i in 0..count as usize {
        for (k, comp) in [0_usize, 1, 3].into_iter().enumerate() {
            // SAFETY: per-component unaligned dword read of input vertex `i`
            // (caller contract).
            let bits = unsafe {
                verts
                    .wrapping_add(i * 4 + comp)
                    .cast::<u32>()
                    .read_unaligned()
            };
            // SAFETY: interleaved write into buffer A slot `i` (contract).
            unsafe {
                scratch
                    .buf_a
                    .wrapping_add(i * 3 + k)
                    .cast::<u32>()
                    .write_unaligned(bits);
            }
        }
    }

    let mut plane = 0_u32;
    loop {
        let src_idx = (plane & 1) as usize;
        let dst_idx = (plane.wrapping_sub(1) & 1) as usize;
        let src_count = counts[src_idx] as usize;
        let src = bufs[src_idx];
        let dst = bufs[dst_idx];

        // Current plane coefficients [c0, c1, c2] (row stride 4 f32; `plane`
        // never exceeds 3).
        let row = planes.wrapping_add(plane as usize * 4);
        let p = [
            // SAFETY: coefficient 0 of a valid plane row (caller contract).
            unsafe { row.read_unaligned() },
            // SAFETY: coefficient 1 of a valid plane row (caller contract).
            unsafe { row.wrapping_add(1).read_unaligned() },
            // SAFETY: coefficient 2 of a valid plane row (caller contract).
            unsafe { row.wrapping_add(2).read_unaligned() },
        ];

        // Classify every source vertex: store the narrowed f32 distance, then
        // the 3-way class.
        for i in 0..src_count {
            let vp = src.wrapping_add(i * 3);
            let v = [
                // SAFETY: source vertex `i` x (caller contract, unguarded).
                unsafe { vp.read_unaligned() },
                // SAFETY: source vertex `i` y (caller contract, unguarded).
                unsafe { vp.wrapping_add(1).read_unaligned() },
                // SAFETY: source vertex `i` w (caller contract, unguarded).
                unsafe { vp.wrapping_add(2).read_unaligned() },
            ];
            let d = poly_clip_plane_distance__6b4a80(&v, &p);
            // SAFETY: dist slot `i` is writable (contract, unguarded).
            unsafe { scratch.dist.wrapping_add(i).write_unaligned(d) };
            let c = poly_clip_classify__6b4a80(d, band_hi, band_lo);
            // SAFETY: class slot `i` is writable (contract, unguarded).
            unsafe { scratch.class.wrapping_add(i).write_unaligned(c) };
        }
        // Wrap sentinel at slot [src_count]: a copy of slot 0, in stock's
        // read-both-then-write-both order (a self-copy of whatever is
        // resident when `src_count == 0`).
        // SAFETY: class slot 0 is readable (caller contract).
        let c0 = unsafe { scratch.class.read_unaligned() };
        // SAFETY: dist slot 0 is readable (caller contract).
        let d0 = unsafe { scratch.dist.read_unaligned() };
        // SAFETY: dist sentinel slot `src_count` is writable (contract).
        unsafe { scratch.dist.wrapping_add(src_count).write_unaligned(d0) };
        // SAFETY: class sentinel slot `src_count` is writable (contract).
        unsafe { scratch.class.wrapping_add(src_count).write_unaligned(c0) };
        if src_count == 0 {
            break;
        }

        // Emit pass: class-0/1 vertices verbatim; crossings only from a
        // non-zero class into a different non-zero class.
        let mut emit = 0_usize;
        for i in 0..src_count {
            // SAFETY: class slot `i` was written by the classify pass above.
            let ci = unsafe { scratch.class.wrapping_add(i).read_unaligned() };
            if ci == 0 {
                // SAFETY: vertex `i` readable, dst slot `emit` writable
                // (caller contract, unguarded like stock).
                unsafe { poly_clip_copy_vertex(src, i, dst, emit) };
                emit += 1;
            } else {
                if ci == 1 {
                    // SAFETY: vertex `i` readable, dst slot `emit` writable
                    // (caller contract, unguarded like stock).
                    unsafe { poly_clip_copy_vertex(src, i, dst, emit) };
                    emit += 1;
                }
                // SAFETY: class slot `i + 1` (at most the sentinel) was
                // written above.
                let cn = unsafe { scratch.class.wrapping_add(i + 1).read_unaligned() };
                if cn != 0 && cn != ci {
                    let next = (i + 1) % src_count;
                    // SAFETY: dist slot `i` was written above.
                    let d_i = unsafe { scratch.dist.wrapping_add(i).read_unaligned() };
                    // SAFETY: dist slot `next` (< src_count) was written above.
                    let d_next = unsafe { scratch.dist.wrapping_add(next).read_unaligned() };
                    let t = poly_clip_crossing_t__6b4a80(d_i, d_next);
                    // Per-component read-then-write in stock's order (the x87
                    // chain reads v[next][k] then v[i][k], stores dst[k], then
                    // moves to the next component).
                    for k in 0..3 {
                        // SAFETY: vertex `next` component `k` is readable
                        // (caller contract, unguarded).
                        let v_next = unsafe { src.wrapping_add(next * 3 + k).read_unaligned() };
                        // SAFETY: vertex `i` component `k` is readable
                        // (caller contract, unguarded).
                        let v_i = unsafe { src.wrapping_add(i * 3 + k).read_unaligned() };
                        let mixed = poly_clip_lerp_component__6b4a80(v_i, v_next, t);
                        // SAFETY: dst slot `emit` component `k` is writable
                        // (caller contract, unguarded).
                        unsafe { dst.wrapping_add(emit * 3 + k).write_unaligned(mixed) };
                    }
                    emit += 1;
                }
            }
        }
        if emit == 0 {
            break;
        }
        // Stock rewrites the running count through `out_count` after EVERY
        // surviving plane, not just the last — kept for byte-faithful
        // behavior under any aliasing.
        // SAFETY: `out_count` is writable (caller contract).
        unsafe { out_count.write_unaligned(emit as u32) };
        counts[dst_idx] = emit as u32;
        plane += 1;
        if plane >= 4 {
            // SAFETY: `out_poly` is writable (caller contract). Pointer
            // identity: the caller receives the destination buffer's own
            // address.
            unsafe { out_poly.write_unaligned(bufs[dst_idx]) };
            return;
        }
    }
    // Fully clipped (or empty input): null polygon then zero count, in
    // stock's write order.
    // SAFETY: `out_poly` is writable (caller contract).
    unsafe { out_poly.write_unaligned(core::ptr::null_mut()) };
    // SAFETY: `out_count` is writable (caller contract).
    unsafe { out_count.write_unaligned(0) };
}

#[cfg(test)]
mod tests_poly_clip_to_screen_rect__6b4a80 {
    use super::{
        PolyClipScratch, poly_clip_classify__6b4a80 as classify,
        poly_clip_crossing_t__6b4a80 as crossing_t, poly_clip_lerp_component__6b4a80 as lerp_c,
        poly_clip_plane_distance__6b4a80 as plane_dist,
        poly_clip_to_screen_rect__6b4a80 as clip_raw,
    };

    /// Band thresholds: `lo < 0 < hi`, mirroring the game's ±eps band.
    const HI: f32 = 1.0e-5;
    const LO: f32 = -1.0e-5;

    /// A `w`-scaled unit screen rect (`x+w`, `w-x`, `y+w`, `w-y`).
    ///
    /// Row stride 4 with the unused padding float, mirroring the game table
    /// layout.
    #[rustfmt::skip]
    const PLANES: [f32; 16] = [
         1.0,  0.0, 1.0, 0.0,
        -1.0,  0.0, 1.0, 0.0,
         0.0,  1.0, 1.0, 0.0,
         0.0, -1.0, 1.0, 0.0,
    ];

    /// Drives the raw driver over local scratch arrays and returns the clipped polygon.
    ///
    /// Returns `None` when fully clipped away. Asserts the stock pointer
    /// contract: a surviving polygon is handed back as ping buffer A's own
    /// address (the plane-3 destination), a clipped-away one as null +
    /// count 0.
    fn clip(verts: &[[f32; 4]]) -> Option<Vec<[f32; 3]>> {
        let mut buf_a = [0.0_f32; 96];
        let mut buf_b = [0.0_f32; 96];
        let mut dist = [0.0_f32; 33];
        let mut class = [0_u32; 33];
        let scratch = PolyClipScratch {
            buf_a: buf_a.as_mut_ptr(),
            buf_b: buf_b.as_mut_ptr(),
            dist: dist.as_mut_ptr(),
            class: class.as_mut_ptr(),
        };
        let mut out_poly: *mut f32 = core::ptr::dangling_mut();
        let mut out_count = 0xdead_beef_u32;
        // SAFETY: every scratch region is sized for the <=32-vertex test
        // polygons (32 verts + wrap sentinel).
        unsafe {
            clip_raw(
                verts.as_ptr().cast::<f32>(),
                verts.len() as u32,
                PLANES.as_ptr(),
                HI,
                LO,
                &scratch,
                &mut out_poly,
                &mut out_count,
            );
        }
        if out_poly.is_null() {
            assert_eq!(out_count, 0, "fully-clipped exit zeroes the count");
            return None;
        }
        assert_eq!(
            out_poly,
            buf_a.as_mut_ptr(),
            "a surviving polygon is returned as ping buffer A's own address"
        );
        Some(
            (0..out_count as usize)
                .map(|i| [buf_a[i * 3], buf_a[i * 3 + 1], buf_a[i * 3 + 2]])
                .collect(),
        )
    }

    /// Independent Sutherland–Hodgman oracle.
    ///
    /// `Vec`-based, one plane at a time, with its own inline f64 math in the
    /// same op order — none of the driver's ping-pong buffers, wrap sentinel
    /// or raw-pointer index arithmetic.
    fn oracle(verts: &[[f32; 4]]) -> Option<Vec<[f32; 3]>> {
        let mut cur: Vec<[f32; 3]> = verts.iter().map(|v| [v[0], v[1], v[3]]).collect();
        for p in 0..4 {
            let (c0, c1, c2) = (
                f64::from(PLANES[p * 4]),
                f64::from(PLANES[p * 4 + 1]),
                f64::from(PLANES[p * 4 + 2]),
            );
            let n = cur.len();
            let d: Vec<f32> = cur
                .iter()
                .map(|v| {
                    ((f64::from(v[2]) * c2 + f64::from(v[0]) * c0) + f64::from(v[1]) * c1) as f32
                })
                .collect();
            let cls: Vec<u32> = d
                .iter()
                .map(|&d| {
                    if d > HI {
                        1
                    } else if d < LO {
                        2
                    } else {
                        0
                    }
                })
                .collect();
            let mut next_poly = Vec::new();
            for i in 0..n {
                let j = (i + 1) % n;
                if cls[i] == 0 {
                    next_poly.push(cur[i]);
                    continue;
                }
                if cls[i] == 1 {
                    next_poly.push(cur[i]);
                }
                if cls[j] != 0 && cls[j] != cls[i] {
                    let t = f64::from(d[i]) / (f64::from(d[i]) - f64::from(d[j]));
                    let mix =
                        |a: f32, b: f32| ((f64::from(b) - f64::from(a)) * t + f64::from(a)) as f32;
                    next_poly.push([
                        mix(cur[i][0], cur[j][0]),
                        mix(cur[i][1], cur[j][1]),
                        mix(cur[i][2], cur[j][2]),
                    ]);
                }
            }
            if next_poly.is_empty() {
                return None;
            }
            cur = next_poly;
        }
        Some(cur)
    }

    #[test]
    fn classify_band_polarity() {
        assert_eq!(classify(1.0, HI, LO), 1);
        assert_eq!(classify(-1.0, HI, LO), 2);
        assert_eq!(classify(0.0, HI, LO), 0);
        // Strict compares: exactly on a threshold is on-plane, not in/out.
        assert_eq!(classify(HI, HI, LO), 0);
        assert_eq!(classify(LO, HI, LO), 0);
        // One ULP past either threshold flips.
        assert_eq!(classify(f32::from_bits(HI.to_bits() + 1), HI, LO), 1);
        assert_eq!(classify(f32::from_bits(LO.to_bits() + 1), HI, LO), 2);
        // The unordered FCOMP result routes to class 0 in both stock compares.
        assert_eq!(classify(f32::NAN, HI, LO), 0);
    }

    #[test]
    fn plane_distance_uses_w_as_third_component() {
        // (w*c2 + x*c0) + y*c1 with v = [x, y, w].
        assert_eq!(plane_dist(&[3.0, 4.0, 5.0], &[2.0, 1.0, 0.5]), 12.5);
        // A NaN component poisons the dot even against a zero coefficient.
        assert!(plane_dist(&[f32::NAN, 0.0, 1.0], &[0.0, 0.0, 1.0]).is_nan());
    }

    #[test]
    fn crossing_t_and_lerp_component() {
        assert_eq!(crossing_t(1.0, -1.0), 0.5);
        assert_eq!(crossing_t(-1.0, 1.0), 0.5);
        assert_eq!(crossing_t(1.0, -3.0), 0.25);
        assert_eq!(lerp_c(1.0, 3.0, 0.25), 1.5);
        assert_eq!(lerp_c(-1.0, 1.0, 0.5), 0.0);
        // Degenerate equal distances divide to infinity, like stock.
        assert_eq!(crossing_t(1.0, 1.0), f64::INFINITY);
    }

    #[test]
    fn fully_inside_passes_through_bit_exact() {
        let verts = [
            [-0.5_f32, -0.5, 9.0, 1.0], // z (component [2]) must be ignored
            [0.5, -0.5, -3.0, 1.0],
            [0.5, 0.5, 7.5, 1.0],
            [-0.5, 0.5, 0.25, 1.0],
        ];
        let out = clip(&verts).unwrap();
        let want: Vec<[f32; 3]> = verts.iter().map(|v| [v[0], v[1], v[3]]).collect();
        assert_eq!(out, want);
    }

    #[test]
    fn fully_outside_clips_away() {
        let verts = [
            [-3.0_f32, 0.0, 0.0, 1.0],
            [-2.0, 0.0, 0.0, 1.0],
            [-2.5, 0.5, 0.0, 1.0],
        ];
        assert_eq!(clip(&verts), None);
    }

    #[test]
    fn empty_input_clips_away() {
        assert_eq!(clip(&[]), None);
    }

    #[test]
    fn crossing_emits_interpolated_vertices() {
        // A(inside) → B(outside x < -1) → C(inside): plane 0 emits A, the
        // A→B crossing, the B→C crossing (B itself dropped), then C — all
        // exact halves, so the assert is byte-exact.
        let verts = [
            [0.0_f32, 0.0, 0.0, 1.0],
            [-2.0, 0.0, 0.0, 1.0],
            [0.0, 0.5, 0.0, 1.0],
        ];
        let out = clip(&verts).unwrap();
        assert_eq!(
            out,
            vec![
                [0.0, 0.0, 1.0],
                [-1.0, 0.0, 1.0],
                [-1.0, 0.25, 1.0],
                [0.0, 0.5, 1.0],
            ]
        );
    }

    #[test]
    fn on_plane_vertex_emits_without_crossing_check() {
        // D sits exactly on the left edge (d = 0 ⇒ class 0): it is emitted
        // verbatim and its outgoing edge D→E is NEVER tested for a crossing
        // (stock skips the check for class 0), so no extra vertex appears
        // between D and the outside vertex E; the only crossing is E→F.
        let verts = [
            [-1.0_f32, 0.0, 0.0, 1.0], // D: on-plane (class 0)
            [-2.0, 0.0, 0.0, 1.0],     // E: outside (class 2)
            [0.0, 0.0, 0.0, 1.0],      // F: inside (class 1)
        ];
        let out = clip(&verts).unwrap();
        assert_eq!(
            out,
            vec![[-1.0, 0.0, 1.0], [-1.0, 0.0, 1.0], [0.0, 0.0, 1.0]]
        );
    }

    #[test]
    fn nan_vertex_classifies_on_plane_and_copies_bit_exact() {
        // Signaling-NaN x: every plane distance is NaN ⇒ class 0 on all four
        // planes ⇒ the vertex is kept verbatim through four raw dword copies
        // (payload intact) and no crossing is ever formed against it.
        let snan = f32::from_bits(0x7f80_0001);
        let verts = [
            [0.0_f32, -0.5, 0.0, 1.0],
            [snan, 0.25, 0.0, 1.0],
            [0.5, 0.25, 0.0, 1.0],
        ];
        let out = clip(&verts).unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], [0.0, -0.5, 1.0]);
        assert_eq!(out[1][0].to_bits(), 0x7f80_0001);
        assert_eq!(out[1][1], 0.25);
        assert_eq!(out[1][2], 1.0);
        assert_eq!(out[2], [0.5, 0.25, 1.0]);
    }

    #[test]
    fn mid_pipeline_full_clip_zeroes_the_running_count() {
        // Survives planes 0/1 (`*out_count` written 3 twice), dies at plane 2
        // (y < -1 everywhere): the exit overwrites both outs with null/0.
        let verts = [
            [0.0_f32, -2.0, 0.0, 1.0],
            [0.5, -2.0, 0.0, 1.0],
            [0.0, -3.0, 0.0, 1.0],
        ];
        assert_eq!(clip(&verts), None);
    }

    #[test]
    fn thirty_two_vertices_no_capacity_guard() {
        // 32-gon strictly inside: all four planes pass all 32 vertices
        // through — the driver trusts the caller's sizing exactly like stock
        // (test scratch is sized 32 verts + sentinel).
        let verts: Vec<[f32; 4]> = (0..32)
            .map(|i| {
                let th = (i as f32) * (core::f32::consts::TAU / 32.0);
                [0.5 * th.cos(), 0.5 * th.sin(), 0.0, 1.0]
            })
            .collect();
        let out = clip(&verts).unwrap();
        assert_eq!(out.len(), 32);
        let want: Vec<[f32; 3]> = verts.iter().map(|v| [v[0], v[1], v[3]]).collect();
        assert_eq!(out, want);
    }

    #[test]
    fn matches_independent_oracle() {
        let polys: &[&[[f32; 4]]] = &[
            // convex quad straddling the left edge
            &[
                [-1.5_f32, -0.4, 0.0, 1.0],
                [0.3, -0.6, 0.0, 1.0],
                [0.4, 0.7, 0.0, 1.0],
                [-1.7, 0.5, 0.0, 1.0],
            ],
            // nonconvex polygon crossing the left plane twice
            &[
                [-2.0_f32, -0.5, 0.0, 1.0],
                [0.0, 0.3, 0.0, 1.0],
                [-1.8, 0.6, 0.0, 1.0],
                [0.4, 0.8, 0.0, 1.0],
                [0.7, -0.7, 0.0, 1.0],
            ],
            // straddles two corners, with non-unit homogeneous w
            &[
                [-1.5_f32, -1.5, 0.0, 1.0],
                [1.5, -1.5, 0.0, 2.0],
                [1.5, 1.5, 0.0, 1.0],
                [-1.5, 1.5, 0.0, 2.0],
            ],
            // survives three planes, fully outside the fourth
            &[
                [-0.5_f32, 2.0, 0.0, 1.0],
                [0.5, 2.0, 0.0, 1.0],
                [0.0, 3.0, 0.0, 1.0],
            ],
        ];
        let bits = |v: &[f32; 3]| [v[0].to_bits(), v[1].to_bits(), v[2].to_bits()];
        for (k, poly) in polys.iter().enumerate() {
            let got = clip(poly).map(|v| v.iter().map(bits).collect::<Vec<_>>());
            let want = oracle(poly).map(|v| v.iter().map(bits).collect::<Vec<_>>());
            assert_eq!(got, want, "poly #{k}");
        }
    }
}

/// One recorded-hit disposition for the swept-volume driver's per-face fold.
///
/// `Collision_SweepVolumeAgainstWorldPlanes` @0x632ba0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepHitAction {
    /// `best − band > t` strictly ordered.
    ///
    /// t is decisively nearer — the hit list restarts with this face at
    /// index 0.
    ReplaceFirst,
    /// Within the band (`best + band > t` strictly ordered).
    ///
    /// Append at the current count.
    Append,
    /// Too far (or any unordered compare): record nothing.
    Ignore,
}

/// Early-out predicate (0x632ba9): `|dist| < eps` ordered.
///
/// The stock `TEST AH,0x5; JP` CONTINUES on unordered, so a NaN dist proceeds
/// into the query — `<` reproduces that (false on NaN).
pub fn sweep_volume_early_out__632ba0(dist: f32, eps: f32) -> bool {
    dist.abs() < eps
}

/// Motion-length clamp (0x632cb7): use `dist` only when strictly greater than the floor.
///
/// Equal, less, and NaN (stock `TEST AH,0x41; JNZ`) take the floor.
pub fn sweep_volume_motion_scale__632ba0(dist: f32, floor: f32) -> f32 {
    if dist > floor { dist } else { floor }
}

/// Front-face gate (0x632dc0).
///
/// The prism face is swept iff `motion·normal > thr` OR the compare is
/// unordered — the stock `TEST AH,0x41; JNP skip` skips on `<` and `==` (odd
/// parity) and processes on `>` and NaN (even parity). The dot folds in the
/// exact x87 order `(mz·pz + my·py) + mx·px` at extended precision (f64 here).
// `!(dot <= thr)` is the `TEST AH,0x41; JNP` polarity documented above: a NaN
// dot processes the face, where the `dot > thr` rewrite would skip it.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn sweep_face_is_front__632ba0(plane: &[f32; 4], motion: &[f32; 3], thr: f32) -> bool {
    let dot = f64::from(motion[2]) * f64::from(plane[2])
        + f64::from(motion[1]) * f64::from(plane[1])
        + f64::from(motion[0]) * f64::from(plane[0]);
    !(dot <= f64::from(thr))
}

/// Band fold for one face hit (0x632e44/0x632e7c).
///
/// Both compares are `TEST AH,0x41; JNZ` — the strict ordered `>` falls
/// through, everything else (including NaN t) takes the other arm; a NaN t
/// therefore reaches the Append check and fails it into `Ignore`.
/// `best ± band` are f32 single-rounded exactly like the stock
/// FSUB/FADD-then-FCOMP.
pub fn sweep_hit_action__632ba0(best: f32, t: f32, band: f32) -> SweepHitAction {
    if best - band > t {
        SweepHitAction::ReplaceFirst
    } else if best + band > t {
        SweepHitAction::Append
    } else {
        SweepHitAction::Ignore
    }
}

/// Nearest-hit update gate (0x632eb6): strictly `t < best` ordered.
///
/// Equal and NaN skip (`TEST AH,0x5; JP` is even-parity on both).
pub fn sweep_hit_improves__632ba0(best: f32, t: f32) -> bool {
    t < best
}

/// Final out-dist select (0x632f3a).
///
/// `best` when `best >= band` (the stock even-parity `JP` on zero flag bits),
/// else the no-hit default. (NaN best would take `best` in stock; unreachable
/// — best only updates on ordered strict improvements from a non-NaN start.)
pub fn sweep_best_to_out_dist__632ba0(best: f32, band: f32, default: f32) -> f32 {
    if best >= band { best } else { default }
}

#[cfg(test)]
mod tests_sweep_volume__632ba0 {
    use super::{
        SweepHitAction, sweep_best_to_out_dist__632ba0, sweep_face_is_front__632ba0,
        sweep_hit_action__632ba0, sweep_hit_improves__632ba0, sweep_volume_early_out__632ba0,
        sweep_volume_motion_scale__632ba0,
    };

    /// NaN dist proceeds into the query (stock JP continues on unordered).
    #[test]
    fn early_out_polarity() {
        assert!(sweep_volume_early_out__632ba0(0.0001, 0.001));
        assert!(!sweep_volume_early_out__632ba0(0.001, 0.001));
        assert!(!sweep_volume_early_out__632ba0(-5.0, 0.001));
        assert!(!sweep_volume_early_out__632ba0(f32::NAN, 0.001));
    }

    /// Equal and NaN take the floor; only strictly-greater keeps dist.
    #[test]
    fn motion_scale_polarity() {
        assert_eq!(sweep_volume_motion_scale__632ba0(2.0, 1.0), 2.0);
        assert_eq!(sweep_volume_motion_scale__632ba0(1.0, 1.0), 1.0);
        assert_eq!(sweep_volume_motion_scale__632ba0(0.5, 1.0), 1.0);
        assert_eq!(sweep_volume_motion_scale__632ba0(f32::NAN, 1.0), 1.0);
    }

    /// Front-face: process on strict `>` and on NaN; skip `<` and `==`.
    #[test]
    fn front_face_polarity() {
        let m = [1.0f32, 0.0, 0.0];
        assert!(sweep_face_is_front__632ba0(&[0.5, 0.0, 0.0, 0.0], &m, 0.0));
        assert!(!sweep_face_is_front__632ba0(
            &[-0.5, 0.0, 0.0, 0.0],
            &m,
            0.0
        ));
        assert!(!sweep_face_is_front__632ba0(&[0.0, 0.0, 0.0, 0.0], &m, 0.0));
        assert!(sweep_face_is_front__632ba0(
            &[f32::NAN, 0.0, 0.0, 0.0],
            &m,
            0.0
        ));
        // Fold order: dominant z term first, then y, then x (matches the
        // FADDP chain; associativity-sensitive near cancellation).
        let p = [1.0f32, 1.0, -1.0, 0.0];
        let mv = [3.0f32, 5.0, 8.0];
        let want = (-f64::from(mv[2]) + f64::from(mv[1]) * 1.0) + f64::from(mv[0]) * 1.0;
        assert_eq!(sweep_face_is_front__632ba0(&p, &mv, 0.0), want > 0.0);
    }

    /// Band fold: decisive replace, in-band append (both boundaries), too-far and NaN ignore.
    #[test]
    fn hit_action_bands() {
        let (best, band) = (10.0f32, 1.0f32);
        assert_eq!(
            sweep_hit_action__632ba0(best, 5.0, band),
            SweepHitAction::ReplaceFirst
        );
        // t == best - band: replace's strict '>' fails => append arm.
        assert_eq!(
            sweep_hit_action__632ba0(best, 9.0, band),
            SweepHitAction::Append
        );
        assert_eq!(
            sweep_hit_action__632ba0(best, 10.5, band),
            SweepHitAction::Append
        );
        // t == best + band: append's strict '>' fails => ignore.
        assert_eq!(
            sweep_hit_action__632ba0(best, 11.0, band),
            SweepHitAction::Ignore
        );
        assert_eq!(
            sweep_hit_action__632ba0(best, 20.0, band),
            SweepHitAction::Ignore
        );
        assert_eq!(
            sweep_hit_action__632ba0(best, f32::NAN, band),
            SweepHitAction::Ignore
        );
    }

    /// Update gate: strict only; equal and NaN keep the old best.
    #[test]
    fn improves_polarity() {
        assert!(sweep_hit_improves__632ba0(1.0, 0.5));
        assert!(!sweep_hit_improves__632ba0(1.0, 1.0));
        assert!(!sweep_hit_improves__632ba0(1.0, 2.0));
        assert!(!sweep_hit_improves__632ba0(1.0, f32::NAN));
    }

    /// Out-dist: best at/above the band passes through; below takes default.
    #[test]
    fn out_dist_select() {
        assert_eq!(sweep_best_to_out_dist__632ba0(2.0, 1.0, 9.0), 2.0);
        assert_eq!(sweep_best_to_out_dist__632ba0(1.0, 1.0, 9.0), 1.0);
        assert_eq!(sweep_best_to_out_dist__632ba0(0.5, 1.0, 9.0), 9.0);
    }
}

/// Back-face gate of `Collision_SweepBoxFaces` 0x632280.
///
/// Fold `dot(face_normal, sweep_dir)` in the stock FADDP order — `z·dz + x·dx`
/// first, then `+ y·dy` — at f64 (x87-extended emulation; each f32·f32 product
/// is exact in f64), and PROCESS the face iff `dot > threshold` **or the
/// compare is unordered**: stock `FCOMP; TEST AH,0x41; JNP` (0x6322f7) skips
/// only on ordered `<=`, so a NaN dot falls through to processing. Port as
/// `!(dot <= thr)` — never "normalize" to `>`.
// The negation is the `TEST AH,0x41; JNP` polarity documented above: a NaN dot
// falls through to processing, which the `dot > thr` rewrite would drop.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn sweep_box_face_is_front__632280(normal: &[f32; 3], dir: &[f32; 3], thr: f32) -> bool {
    let dot = (f64::from(normal[2]) * f64::from(dir[2]) + f64::from(normal[0]) * f64::from(dir[0]))
        + f64::from(normal[1]) * f64::from(dir[1]);
    !(dot <= f64::from(thr))
}

/// Lower contact band of 0x632280.
///
/// RESET (overwrite contact slot 0, `*outContactCount = 1`) iff
/// `*bestDist − eps > hitDist` strictly, ordered. Stock
/// `FSUB; FCOMP; TEST AH,0x41; JNZ` (0x632368) branches to the append-band
/// check on `<=`, `==` **and unordered** — a NaN hitDist (or bestDist) never
/// takes the reset arm. The subtraction runs at f64 (extended emulation).
pub fn sweep_box_band_reset__632280(best: f32, eps: f32, hit: f32) -> bool {
    f64::from(best) - f64::from(eps) > f64::from(hit)
}

/// Upper contact band of 0x632280.
///
/// APPEND the tied contact iff `hitDist < *bestDist + eps` strictly, ordered.
/// Stock `FADD; FCOMP; TEST AH,0x41; JNZ` (0x6323a1) skips the append on `>=`
/// and unordered alike — NaN never appends. The addition runs at f64.
pub fn sweep_box_band_append__632280(best: f32, eps: f32, hit: f32) -> bool {
    f64::from(hit) < f64::from(best) + f64::from(eps)
}

/// New-best gate of 0x632280.
///
/// Update `*bestDist` (and swap the winning contact into slot 0) iff
/// `hitDist < *bestDist` strictly. Stock `FLD; FCOMP; TEST AH,0x5; JP`
/// (0x6323d0) skips on ordered `>=` and on unordered — equality and NaN both
/// keep the old best. Two m32 loads, no arithmetic => exact f32 compare.
pub fn sweep_box_new_best__632280(hit: f32, best: f32) -> bool {
    hit < best
}

#[cfg(test)]
mod tests_sweep_box_faces__632280 {
    use super::*;

    #[test]
    fn front_gate_polarity() {
        let dir = [1.0f32, 0.0, 0.0];
        // dot = 1.0 vs thr 0.5 => process; dot = 0.5 (equal) => skip;
        // dot = 0.0 => skip.
        assert!(sweep_box_face_is_front__632280(&[1.0, 0.0, 0.0], &dir, 0.5));
        assert!(!sweep_box_face_is_front__632280(
            &[0.5, 0.0, 0.0],
            &dir,
            0.5
        ));
        assert!(!sweep_box_face_is_front__632280(
            &[0.0, 1.0, 0.0],
            &dir,
            0.5
        ));
        // NaN dot (or NaN threshold) is unordered => PROCESS.
        assert!(sweep_box_face_is_front__632280(
            &[f32::NAN, 0.0, 0.0],
            &dir,
            0.5
        ));
        assert!(sweep_box_face_is_front__632280(
            &[1.0, 0.0, 0.0],
            &dir,
            f32::NAN
        ));
    }

    #[test]
    fn front_gate_fold_order_is_zxy() {
        // Associativity-sensitive near cancellation: the stock fold is
        // (z·dz + x·dx) + y·dy. With magnitudes chosen so f64 rounding of
        // the first pair matters, the reference must use the same order.
        let n = [1.0f32, 1e-8, -1.0];
        let d = [3.0f32, 5.0, 3.0];
        let want = (f64::from(n[2]) * f64::from(d[2]) + f64::from(n[0]) * f64::from(d[0]))
            + f64::from(n[1]) * f64::from(d[1]);
        assert_eq!(sweep_box_face_is_front__632280(&n, &d, 0.0), want > 0.0);
    }

    #[test]
    fn band_reset_polarity() {
        // best - eps > hit strictly => reset.
        assert!(sweep_box_band_reset__632280(1.0, 0.125, 0.5));
        // Equality does NOT reset (JNZ takes the band arm on C3).
        assert!(!sweep_box_band_reset__632280(1.0, 0.125, 0.875));
        assert!(!sweep_box_band_reset__632280(1.0, 0.125, 0.9));
        // NaN on either side never resets.
        assert!(!sweep_box_band_reset__632280(f32::NAN, 0.125, 0.5));
        assert!(!sweep_box_band_reset__632280(1.0, 0.125, f32::NAN));
    }

    #[test]
    fn band_append_polarity() {
        // hit < best + eps strictly => append.
        assert!(sweep_box_band_append__632280(1.0, 0.125, 1.0));
        // Equality skips (JNZ on C3), NaN skips.
        assert!(!sweep_box_band_append__632280(1.0, 0.125, 1.125));
        assert!(!sweep_box_band_append__632280(1.0, 0.125, 2.0));
        assert!(!sweep_box_band_append__632280(1.0, 0.125, f32::NAN));
        assert!(!sweep_box_band_append__632280(f32::NAN, 0.125, 1.0));
    }

    #[test]
    fn new_best_polarity() {
        assert!(sweep_box_new_best__632280(0.5, 1.0));
        // Equality and NaN keep the old best (TEST AH,0x5; JP).
        assert!(!sweep_box_new_best__632280(1.0, 1.0));
        assert!(!sweep_box_new_best__632280(2.0, 1.0));
        assert!(!sweep_box_new_best__632280(f32::NAN, 1.0));
        assert!(!sweep_box_new_best__632280(1.0, f32::NAN));
    }
}

/// Sweep-vector rotation of `Collision_GatherWorldTriangles` 0x631e70 (block 0x631f2b).
///
/// Row-vector × the world transform's 3×3. Per lane `c` the stock FADDP fold
/// is `(m[c+8]·sz + m[c+4]·sy) + m[c]·sx`, carried at f64 (x87-extended
/// emulation; each f32·f32 product is exact in f64) and narrowed by that
/// lane's FSTP m32.
pub fn gather_rotate_sweep__631e70(m: &[f32; 16], s: &[f32; 3]) -> [f32; 3] {
    let (sx, sy, sz) = (f64::from(s[0]), f64::from(s[1]), f64::from(s[2]));
    let lane = |c: usize| -> f32 {
        super::f64_to_f32(
            (f64::from(m[c + 8]) * sz + f64::from(m[c + 4]) * sy) + f64::from(m[c]) * sx,
        )
    };
    [lane(0), lane(1), lane(2)]
}

/// Swept-position fold of 0x631e70 (block 0x631f95).
///
/// `pos + sweep`, one extended FADD + FSTP m32 per lane => f64 add, narrowed
/// per lane.
pub fn gather_swept_pos__631e70(pos: &[f32; 3], s: &[f32; 3]) -> [f32; 3] {
    [
        super::f64_to_f32(f64::from(pos[0]) + f64::from(s[0])),
        super::f64_to_f32(f64::from(pos[1]) + f64::from(s[1])),
        super::f64_to_f32(f64::from(pos[2]) + f64::from(s[2])),
    ]
}

/// Capsule query AABB of 0x631e70 (block 0x631fbd).
///
/// `[px−r, py−r, pz, px+r, py+r, pz+h]` — the z span is `[z, z+height]`, NOT
/// symmetric (min.z is a raw copy of the swept z). One extended FSUB/FADD +
/// FSTP m32 per arithmetic lane.
pub fn gather_capsule_aabb__631e70(p: &[f32; 3], radius: f32, height: f32) -> [f32; 6] {
    let r = f64::from(radius);
    [
        super::f64_to_f32(f64::from(p[0]) - r),
        super::f64_to_f32(f64::from(p[1]) - r),
        p[2],
        super::f64_to_f32(f64::from(p[0]) + r),
        super::f64_to_f32(f64::from(p[1]) + r),
        super::f64_to_f32(f64::from(p[2]) + f64::from(height)),
    ]
}

/// Triangle-normal rotation by the inverted world transform's 3×3 (0x631e70 block 0x6321cd).
///
/// Per lane `c` the fold order DIFFERS from the sweep rotation:
/// `(inv[c]·nx + inv[c+8]·nz) + inv[c+4]·ny` — x first, then z, then y. f64
/// fold, per-lane FSTP m32 narrow.
pub fn gather_rotate_normal__631e70(inv: &[f32; 16], n: &[f32; 3]) -> [f32; 3] {
    let (nx, ny, nz) = (f64::from(n[0]), f64::from(n[1]), f64::from(n[2]));
    let lane = |c: usize| -> f32 {
        super::f64_to_f32(
            (f64::from(inv[c]) * nx + f64::from(inv[c + 8]) * nz) + f64::from(inv[c + 4]) * ny,
        )
    };
    [lane(0), lane(1), lane(2)]
}

/// Plane-distance rebuild of 0x631e70 (block 0x632249).
///
/// `d = −(((nz·vz) + ny·vy) + nx·vx)` — folded z, y, x at f64, FCHS on the
/// extended value, then a single FSTP m32.
pub fn gather_plane_d__631e70(n: &[f32; 3], v: &[f32; 3]) -> f32 {
    super::f64_to_f32(
        -((f64::from(n[2]) * f64::from(v[2]) + f64::from(n[1]) * f64::from(v[1]))
            + f64::from(n[0]) * f64::from(v[0])),
    )
}

#[cfg(test)]
mod tests_gather_world_triangles__631e70 {
    use super::*;

    const IDENT: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];

    #[test]
    fn rotate_sweep_identity_is_identity() {
        assert_eq!(
            gather_rotate_sweep__631e70(&IDENT, &[3.0, -5.0, 7.0]),
            [3.0, -5.0, 7.0]
        );
    }

    #[test]
    fn rotate_sweep_uses_columns() {
        // Row-vector × M: lane c = m[c]·sx + m[c+4]·sy + m[c+8]·sz (column c).
        let mut m = IDENT;
        m[1] = 2.0; // row 0, col 1 — feeds lane 1 from sx.
        assert_eq!(
            gather_rotate_sweep__631e70(&m, &[1.0, 0.0, 0.0]),
            [1.0, 2.0, 0.0]
        );
    }

    #[test]
    fn rotate_sweep_fold_order_zyx() {
        // Associativity-sensitive: the stock lane fold is (z-term + y-term)
        // + x-term at f64; a different order rounds differently here.
        let m = {
            let mut m = IDENT;
            m[0] = 1.0;
            m[4] = 1e-8;
            m[8] = -1.0;
            m
        };
        let s = [3.0f32, 5.0, 3.0];
        let want = super::super::f64_to_f32(
            (f64::from(m[8]) * f64::from(s[2]) + f64::from(m[4]) * f64::from(s[1]))
                + f64::from(m[0]) * f64::from(s[0]),
        );
        assert_eq!(gather_rotate_sweep__631e70(&m, &s)[0], want);
    }

    #[test]
    fn capsule_box_z_is_asymmetric() {
        let bx = gather_capsule_aabb__631e70(&[10.0, 20.0, 30.0], 2.0, 5.0);
        assert_eq!(bx, [8.0, 18.0, 30.0, 12.0, 22.0, 35.0]);
    }

    #[test]
    fn swept_pos_adds_lanewise() {
        assert_eq!(
            gather_swept_pos__631e70(&[1.0, 2.0, 3.0], &[0.5, -0.25, 4.0]),
            [1.5, 1.75, 7.0]
        );
    }

    #[test]
    fn rotate_normal_uses_columns_xzy_fold() {
        // Same column mapping as the sweep rotation, different fold order.
        assert_eq!(
            gather_rotate_normal__631e70(&IDENT, &[0.25, -1.0, 8.0]),
            [0.25, -1.0, 8.0]
        );
        let mut inv = IDENT;
        inv[2] = 3.0; // row 0, col 2 — feeds lane 2 from nx.
        assert_eq!(
            gather_rotate_normal__631e70(&inv, &[1.0, 0.0, 0.0]),
            [1.0, 0.0, 3.0]
        );
    }

    #[test]
    fn plane_d_is_negated_dot() {
        assert_eq!(
            gather_plane_d__631e70(&[0.0, 0.0, 1.0], &[5.0, 6.0, 7.0]),
            -7.0
        );
        assert_eq!(
            gather_plane_d__631e70(&[1.0, 2.0, 3.0], &[1.0, 1.0, 1.0]),
            -6.0
        );
    }
}

/// Direction rotation by the world transform for `CMovement::BuildSweptBoundsAndQueryWorld`.
///
/// (@0x633840, 0x633907): per-component `((m[hi]·z + m[mid]·y) + m[lo]·x)`
/// column dots, folded wide with one narrow each — the stock FADDP pairing.
pub fn swept_rotate_dir__633840(m: &[f32; 16], dir: [f32; 3]) -> [f32; 3] {
    let x = f64::from(dir[0]);
    let y = f64::from(dir[1]);
    let z = f64::from(dir[2]);
    let c = |lo: usize, mid: usize, hi: usize| {
        super::f64_to_f32((f64::from(m[hi]) * z + f64::from(m[mid]) * y) + f64::from(m[lo]) * x)
    };
    [c(0, 4, 8), c(1, 5, 9), c(2, 6, 10)]
}

/// Base capsule AABB (0x633982–0x633a01): `{x−r, y−r, z, x+r, y+r, z+h}`.
///
/// z-min is the RAW origin dword (no x87 pass); every other lane folds wide
/// and narrows once.
pub fn swept_base_box__633840(origin: [f32; 3], r: f32, h: f32) -> [f32; 6] {
    let x = f64::from(origin[0]);
    let y = f64::from(origin[1]);
    let z = f64::from(origin[2]);
    let r = f64::from(r);
    [
        super::f64_to_f32(x - r),
        super::f64_to_f32(y - r),
        origin[2],
        super::f64_to_f32(x + r),
        super::f64_to_f32(y + r),
        super::f64_to_f32(z + f64::from(h)),
    ]
}

/// `dir · k` with the scalar WIDE across all three lanes.
///
/// The stock keeps the reach/step product on the stack through the three
/// FMULs, one narrow per lane.
pub fn swept_scaled_dir__633840(dir: [f32; 3], k: f64) -> [f32; 3] {
    [
        super::f64_to_f32(f64::from(dir[0]) * k),
        super::f64_to_f32(f64::from(dir[1]) * k),
        super::f64_to_f32(f64::from(dir[2]) * k),
    ]
}

/// Component sum `a + b`.
///
/// One narrow per lane (the mid-point/center Set folds at 0x633b9d/0x633d6b).
pub fn swept_add3__633840(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        super::f64_to_f32(f64::from(a[0]) + f64::from(b[0])),
        super::f64_to_f32(f64::from(a[1]) + f64::from(b[1])),
        super::f64_to_f32(f64::from(a[2]) + f64::from(b[2])),
    ]
}

/// Wide product / difference / sum single-lane folds.
///
/// Half-step, radius scale, falling z-adjust, hover drop — each one
/// FMUL/FSUB/FADD + FSTP.
pub fn swept_mul__633840(a: f32, b: f32) -> f32 {
    super::f64_to_f32(f64::from(a) * f64::from(b))
}
pub fn swept_sub__633840(a: f32, b: f32) -> f32 {
    super::f64_to_f32(f64::from(a) - f64::from(b))
}
pub fn swept_add__633840(a: f32, b: f32) -> f32 {
    super::f64_to_f32(f64::from(a) + f64::from(b))
}

/// Falling-arm lateral expand (0x633ac4–0x633b0c).
///
/// `v = motionZ · g` stays WIDE through all four adjustments; applied only on
/// `v > zero` ORDERED (`TEST AH,0x41; JNZ` skips on `<=`, `==`, NaN). `box4`
/// is `{minX, maxX, minY, maxY}`.
pub fn swept_fall_expand__633840(
    box4: [f32; 4],
    motion_z: f32,
    g: f32,
    zero: f32,
) -> Option<[f32; 4]> {
    let v = f64::from(motion_z) * f64::from(g);
    if v > f64::from(zero) {
        Some([
            super::f64_to_f32(f64::from(box4[0]) - v),
            super::f64_to_f32(f64::from(box4[1]) + v),
            super::f64_to_f32(f64::from(box4[2]) - v),
            super::f64_to_f32(f64::from(box4[3]) + v),
        ])
    } else {
        None
    }
}

/// Swim-arm finish (0x633c04–0x633c43).
///
/// x/y lanes expand by the WIDE `r · g` product; the z-max grows by
/// `max(height, rr)` with the pick polarity `height < rr` ORDERED → `rr`, NaN
/// keeps `height` (the `TEST AH,0x5; JP` shape at 0x633c33).
pub fn swept_swim_finish__633840(bx: [f32; 6], rr: f32, h: f32) -> [f32; 6] {
    let r = f64::from(rr);
    let z_ext = if h < rr { r } else { f64::from(h) };
    [
        super::f64_to_f32(f64::from(bx[0]) - r),
        super::f64_to_f32(f64::from(bx[1]) - r),
        bx[2],
        super::f64_to_f32(f64::from(bx[3]) + r),
        super::f64_to_f32(f64::from(bx[4]) + r),
        super::f64_to_f32(z_ext + f64::from(bx[5])),
    ]
}

/// Ground-arm mid box (0x633d93–0x633de9).
///
/// `{mid ∓ R, mid.z}` with `R = r·g + halfStep` WIDE across all four lanes.
pub fn swept_ground_midbox__633840(mid: [f32; 3], r: f32, g: f32, hs: f32) -> [f32; 6] {
    let rr = f64::from(r) * f64::from(g) + f64::from(hs);
    let x = f64::from(mid[0]);
    let y = f64::from(mid[1]);
    [
        super::f64_to_f32(x - rr),
        super::f64_to_f32(y - rr),
        mid[2],
        super::f64_to_f32(x + rr),
        super::f64_to_f32(y + rr),
        mid[2],
    ]
}

/// Ground-arm floor drop (0x633e35–0x633e4b).
///
/// `a8 − (scalar + step·g)` — the FADDP folds the delegate return with the
/// wide product, one narrow.
pub fn swept_ground_floor__633840(a8: f32, step: f32, g: f32, s: f32) -> f32 {
    super::f64_to_f32(f64::from(a8) - (f64::from(s) + f64::from(step) * f64::from(g)))
}

/// Plane pull-back rebase (0x633f6e–0x63400a).
///
/// Rotates the plane normal by the inverse transform's 3×3
/// (`((i[c]·x + i[c+8]·z) + i[c+4]·y)` per column, each narrowed) and refolds
/// `d = −((nz'·v.z + ny'·v.y) + nx'·v.x)` against the already-transformed
/// first vertex, using the NARROWED rotated components. Returns
/// `[nx', ny', nz', d]`.
pub fn swept_plane_rebase__633840(inv: &[f32; 16], n: [f32; 3], v: [f32; 3]) -> [f32; 4] {
    let x = f64::from(n[0]);
    let y = f64::from(n[1]);
    let z = f64::from(n[2]);
    let c = |i: usize| {
        super::f64_to_f32(
            (f64::from(inv[i]) * x + f64::from(inv[i + 8]) * z) + f64::from(inv[i + 4]) * y,
        )
    };
    let nx = c(0);
    let ny = c(1);
    let nz = c(2);
    let d = super::f64_to_f32(
        -((f64::from(nz) * f64::from(v[2]) + f64::from(ny) * f64::from(v[1]))
            + f64::from(nx) * f64::from(v[0])),
    );
    [nx, ny, nz, d]
}

#[cfg(test)]
mod tests_swept_bounds__633840 {
    use super::*;

    #[test]
    fn rotate_dir_column_dots() {
        let mut m = [0.0f32; 16];
        // Columns: col0 = (1,2,3), col1 = (4,5,6), col2 = (7,8,9).
        m[0] = 1.0;
        m[1] = 2.0;
        m[2] = 3.0;
        m[4] = 4.0;
        m[5] = 5.0;
        m[6] = 6.0;
        m[8] = 7.0;
        m[9] = 8.0;
        m[10] = 9.0;
        let got = swept_rotate_dir__633840(&m, [1.0, 1.0, 1.0]);
        assert_eq!(got, [12.0, 15.0, 18.0]);
    }

    #[test]
    fn base_box_lanes() {
        let got = swept_base_box__633840([10.0, 20.0, 30.0], 2.0, 5.0);
        assert_eq!(got, [8.0, 18.0, 30.0, 12.0, 22.0, 35.0]);
        // z-min is the raw origin dword — NaN payload preserved.
        let nan = f32::from_bits(0x7fc0_1234);
        let got = swept_base_box__633840([0.0, 0.0, nan], 1.0, 1.0);
        assert_eq!(got[2].to_bits(), nan.to_bits());
    }

    #[test]
    fn fall_expand_polarity() {
        let b = [0.0f32, 10.0, 0.0, 10.0];
        // v = 2*1 > 0 -> applied wide.
        let got = swept_fall_expand__633840(b, 2.0, 1.0, 0.0).unwrap();
        assert_eq!(got, [-2.0, 12.0, -2.0, 12.0]);
        // v == 0, v < 0, NaN all skip.
        assert!(swept_fall_expand__633840(b, 0.0, 1.0, 0.0).is_none());
        assert!(swept_fall_expand__633840(b, -1.0, 1.0, 0.0).is_none());
        assert!(swept_fall_expand__633840(b, f32::NAN, 1.0, 0.0).is_none());
    }

    #[test]
    fn swim_finish_zext_pick() {
        let bx = [0.0f32, 0.0, 5.0, 10.0, 10.0, 20.0];
        // h >= rr keeps h; h < rr picks rr; NaN h keeps h (NaN result).
        let got = swept_swim_finish__633840(bx, 2.0, 3.0);
        assert_eq!(got[5].to_bits(), 23.0f32.to_bits());
        let got = swept_swim_finish__633840(bx, 4.0, 3.0);
        assert_eq!(got[5].to_bits(), 24.0f32.to_bits());
        let got = swept_swim_finish__633840(bx, 4.0, f32::NAN);
        assert!(got[5].is_nan());
        assert_eq!(got[0].to_bits(), (-4.0f32).to_bits());
    }

    #[test]
    fn ground_midbox_wide_radius() {
        let got = swept_ground_midbox__633840([10.0, 20.0, 30.0], 2.0, 1.5, 1.0);
        // R = 2*1.5 + 1 = 4.
        assert_eq!(got, [6.0, 16.0, 30.0, 14.0, 24.0, 30.0]);
    }

    #[test]
    fn ground_floor_fold() {
        // 100 - (3 + 2*4) = 89.
        let got = swept_ground_floor__633840(100.0, 2.0, 4.0, 3.0);
        assert_eq!(got.to_bits(), 89.0f32.to_bits());
    }

    #[test]
    fn plane_rebase_folds() {
        let mut inv = [0.0f32; 16];
        // Identity 3x3.
        inv[0] = 1.0;
        inv[5] = 1.0;
        inv[10] = 1.0;
        let got = swept_plane_rebase__633840(&inv, [0.0, 0.0, 1.0], [3.0, 4.0, 5.0]);
        assert_eq!(got[0].to_bits(), 0.0f32.to_bits());
        assert_eq!(got[2].to_bits(), 1.0f32.to_bits());
        // d = -(n · v) = -5.
        assert_eq!(got[3].to_bits(), (-5.0f32).to_bits());
    }

    #[test]
    fn add_wide_fold() {
        assert_eq!(swept_add_wide__633840(1.0, 2.5).to_bits(), 3.5f32.to_bits());
    }

    #[test]
    fn scalar_folds() {
        assert_eq!(swept_mul__633840(3.0, 0.5).to_bits(), 1.5f32.to_bits());
        assert_eq!(swept_sub__633840(3.0, 0.5).to_bits(), 2.5f32.to_bits());
        assert_eq!(swept_add__633840(3.0, 0.5).to_bits(), 3.5f32.to_bits());
        let got = swept_scaled_dir__633840([1.0, 2.0, 3.0], 0.5);
        assert_eq!(got, [0.5, 1.0, 1.5]);
        let got = swept_add3__633840([1.0, 2.0, 3.0], [0.25, 0.5, 0.75]);
        assert_eq!(got, [1.25, 2.5, 3.75]);
    }
}

/// `a + k` with `k` the WIDE pick from the vertical max (0x633e27).
///
/// One narrow into the z-max store.
pub fn swept_add_wide__633840(a: f32, k: f64) -> f32 {
    super::f64_to_f32(k + f64::from(a))
}

/// Wide time-share for `CMovement::StepGroundMove` (@0x6367b0, 0x636993).
///
/// `(hitT / remaining) · remainingMs`, kept WIDE across the slide-iter compare
/// and the consumed/remaining updates.
pub fn ground_time_spent__6367b0(hit_t: f32, remaining: f32, remaining_ms: f32) -> f64 {
    f64::from(hit_t) / f64::from(remaining) * f64::from(remaining_ms)
}

/// `a − k` with `k` wide (the consumed/remaining folds), one narrow.
pub fn ground_sub_wide__6367b0(a: f32, k: f64) -> f32 {
    super::f64_to_f32(f64::from(a) - k)
}

/// 2D distance shrink (0x6369b7).
///
/// `rem2d − sqrt(mx² + my²)`, the squares folded wide (`FADDP` pairing x
/// then y), one narrow.
pub fn ground_2d_shrink__6367b0(rem2d: f32, mx: f32, my: f32) -> f32 {
    let s = f64::from(mx) * f64::from(mx) + f64::from(my) * f64::from(my);
    super::f64_to_f32(f64::from(rem2d) - s.sqrt())
}

/// `a·b + c` wide fold.
///
/// One narrow (the ground-stick probe distance at 0x636dd7 and relatives).
pub fn ground_muladd__6367b0(a: f32, b: f32, c: f32) -> f32 {
    super::f64_to_f32(f64::from(a) * f64::from(b) + f64::from(c))
}

/// The redirect fold after a slope-climb / wall-pushout (0x636c20–0x636cf9).
///
/// Faithful asymmetries: `x = md.x·rem + climb.x` and
/// `y = climb.y + md.y·rem` fold wide, but the z product NARROWS FIRST
/// (`FSTP [ebp-0xb4]` at 0x636c32) before `climb.z` adds; the x/y squares
/// narrow individually before both the 3D and 2D length folds; the
/// normalizations divide 1.0 by the NARROWED lengths and scale the wide
/// reciprocal into each lane. The 2D pair renormalizes only when
/// `|len2d| >= eps2d` OR the compare is unordered (`TEST AH,0x5; JNP`
/// skips exclusively on ordered less). Returns
/// `(new_move, len, dir_norm, len2d, dir2d_opt)`.
// `!(|len2d| < eps2d)` is the `TEST AH,0x5; JNP` polarity documented above: a
// NaN 2D length still renormalizes, where `|len2d| >= eps2d` would skip it.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn ground_new_move__6367b0(
    md: [f32; 3],
    rem: f32,
    climb: [f32; 3],
    one: f32,
    eps2d: f32,
) -> ([f32; 3], f32, [f32; 3], f32, Option<[f32; 2]>) {
    let r = f64::from(rem);
    let nmx = super::f64_to_f32(f64::from(md[0]) * r + f64::from(climb[0]));
    let nmy = super::f64_to_f32(f64::from(climb[1]) + f64::from(md[1]) * r);
    let zp = super::f64_to_f32(f64::from(md[2]) * r);
    let nmz = super::f64_to_f32(f64::from(climb[2]) + f64::from(zp));
    let sqx = super::f64_to_f32(f64::from(nmx) * f64::from(nmx));
    let sqy = super::f64_to_f32(f64::from(nmy) * f64::from(nmy));
    let len_w = (f64::from(nmz) * f64::from(nmz) + f64::from(sqy) + f64::from(sqx)).sqrt();
    let len = super::f64_to_f32(len_w);
    let inv = f64::from(one) / f64::from(len);
    let dir = [
        super::f64_to_f32(f64::from(nmx) * inv),
        super::f64_to_f32(f64::from(nmy) * inv),
        super::f64_to_f32(f64::from(nmz) * inv),
    ];
    let len2d_w = (f64::from(sqy) + f64::from(sqx)).sqrt();
    let len2d = super::f64_to_f32(len2d_w);
    // `TEST AH,0x5; JNP` skips ONLY on ordered strictly-less; NaN falls
    // through into the renormalize.
    let dir2d = if !(len2d_w.abs() < f64::from(eps2d)) {
        let inv2 = f64::from(one) / f64::from(len2d);
        Some([
            super::f64_to_f32(f64::from(nmx) * inv2),
            super::f64_to_f32(f64::from(nmy) * inv2),
        ])
    } else {
        None
    };
    ([nmx, nmy, nmz], len, dir, len2d, dir2d)
}

/// Hover lift fold (0x63706e): `(totalMs·ms2s)·rise + hoverDownT` wide, one narrow.
pub fn ground_hover_lift__6367b0(total_ms: f32, ms2s: f32, rise: f32, hdt: f32) -> f32 {
    super::f64_to_f32(f64::from(total_ms) * f64::from(ms2s) * f64::from(rise) + f64::from(hdt))
}

#[cfg(test)]
mod tests_step_ground_move__6367b0 {
    use super::*;

    #[test]
    fn time_spent_wide() {
        let w = ground_time_spent__6367b0(1.0, 4.0, 8.0);
        assert!((w - 2.0).abs() < 1.0e-12);
        assert_eq!(ground_sub_wide__6367b0(8.0, w).to_bits(), 6.0f32.to_bits());
    }

    #[test]
    fn shrink_and_muladd() {
        assert_eq!(
            ground_2d_shrink__6367b0(10.0, 3.0, 4.0).to_bits(),
            5.0f32.to_bits()
        );
        assert_eq!(
            ground_muladd__6367b0(2.0, 3.0, 4.0).to_bits(),
            10.0f32.to_bits()
        );
        assert_eq!(
            ground_hover_lift__6367b0(1000.0, 0.001, 2.0, 0.5).to_bits(),
            2.5f32.to_bits()
        );
    }

    #[test]
    fn share_and_rescale_folds() {
        assert_eq!(
            swept_time_share__6367b0(500.0, 1000.0, 8.0).to_bits(),
            4.0f32.to_bits()
        );
        assert_eq!(swept_sub_wrap__6367b0(2.5).to_bits(), 2.5f32.to_bits());
        assert_eq!(
            ground_rescale_ms__6367b0(2.0, 4.0, 100.0).to_bits(),
            50.0f32.to_bits()
        );
        assert_eq!(
            ground_ceiling_scale__6367b0(3.0, 6.0, 10.0).to_bits(),
            5.0f32.to_bits()
        );
    }

    #[test]
    fn new_move_folds_and_normalize() {
        // md = +x, rem 3, climb (1, 0, 0): nm = (4, 0, 0).
        let (nm, len, dir, len2d, d2) =
            ground_new_move__6367b0([1.0, 0.0, 0.0], 3.0, [1.0, 0.0, 0.0], 1.0, 1.0e-4);
        assert_eq!(nm, [4.0, 0.0, 0.0]);
        assert_eq!(len.to_bits(), 4.0f32.to_bits());
        assert_eq!(dir, [1.0, 0.0, 0.0]);
        assert_eq!(len2d.to_bits(), 4.0f32.to_bits());
        assert_eq!(d2.unwrap(), [1.0, 0.0]);
        // Pure-z move: 2D length under eps -> no renormalize.
        let (_, _, _, _, d2) = ground_new_move__6367b0([0.0, 0.0, 1.0], 2.0, [0.0; 3], 1.0, 1.0e-4);
        assert!(d2.is_none());
        // NaN lanes fall through INTO the renormalize (unordered compare).
        let (_, _, _, _, d2) =
            ground_new_move__6367b0([f32::NAN, 0.0, 0.0], 1.0, [0.0; 3], 1.0, 1.0e-4);
        assert!(d2.unwrap()[0].is_nan());
    }
}

/// Narrow a wide intermediate.
///
/// The `FST` spill of a running f64 chain — the continue-time `remainingMs`
/// store and the hover-lift space clamp.
pub fn swept_sub_wrap__6367b0(w: f64) -> f32 {
    super::f64_to_f32(w)
}

/// Continue-time distance share (0x63694a).
///
/// `left/total · dist` folded from the WIDE `left`, one narrow.
pub fn swept_time_share__6367b0(left: f64, total: f32, dist: f32) -> f32 {
    super::f64_to_f32(left / f64::from(total) * f64::from(dist))
}

/// Post-climb time rescale (0x636c14): `remaining/before · remainingMs`, one narrow.
pub fn ground_rescale_ms__6367b0(remaining: f32, before: f32, remaining_ms: f32) -> f32 {
    super::f64_to_f32(f64::from(remaining) / f64::from(before) * f64::from(remaining_ms))
}

/// Step-up ceiling rescale (0x636d59).
///
/// `allowed/nmz · remaining` with `allowed` WIDE, one narrow.
pub fn ground_ceiling_scale__6367b0(allowed: f64, nmz: f32, remaining: f32) -> f32 {
    super::f64_to_f32(allowed / f64::from(nmz) * f64::from(remaining))
}

/// Speed derivation for `CMovement::StepMovement` (@0x634040, 0x634062).
///
/// `|moveDelta|` folds `((x·x + y·y) + z·z)` wide,
/// `speedPerSec = mag / (deltaMs · ms2s)` narrows once, and when
/// `|mag| >= eps` OR the compare is unordered (`TEST AH,0x5; JNP` skips only
/// on ordered less) the direction normalizes through the wide `1/mag`
/// reciprocal. Returns `(speed_per_sec, Some(dir))`.
// `!(|mag| < eps)` is the `TEST AH,0x5; JNP` polarity documented above: a NaN
// magnitude still normalizes, where `|mag| >= eps` would skip it.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn step_speed_and_dir__634040(
    delta: [f32; 3],
    delta_ms: u32,
    ms2s: f32,
    eps: f32,
    one: f32,
) -> (f32, Option<[f32; 3]>) {
    let x = f64::from(delta[0]);
    let y = f64::from(delta[1]);
    let z = f64::from(delta[2]);
    let mag = (x * x + y * y + z * z).sqrt();
    let speed = super::f64_to_f32(mag / (f64::from(delta_ms) * f64::from(ms2s)));
    let dir = if !(mag.abs() < f64::from(eps)) {
        let k = f64::from(one) / mag;
        Some([
            super::f64_to_f32(k * x),
            super::f64_to_f32(k * y),
            super::f64_to_f32(k * z),
        ])
    } else {
        None
    };
    (speed, dir)
}

/// Substep distance (0x634122).
///
/// `fv · ms2s · speedPerSec` from the zero-extended remaining milliseconds,
/// one narrow.
pub fn step_distance__634040(fv: u32, ms2s: f32, speed: f32) -> f32 {
    super::f64_to_f32(f64::from(fv) * f64::from(ms2s) * f64::from(speed))
}

#[cfg(test)]
mod tests_step_movement__634040 {
    use super::{step_distance__634040 as dist, step_speed_and_dir__634040 as sd};

    #[test]
    fn speed_and_normalize() {
        let (speed, d) = sd([3.0, 0.0, 4.0], 1000, 0.001, 1.0e-6, 1.0);
        assert_eq!(speed.to_bits(), 5.0f32.to_bits());
        let d = d.unwrap();
        assert_eq!(d[0].to_bits(), 0.6f32.to_bits());
        assert_eq!(d[2].to_bits(), 0.8f32.to_bits());
        // Sub-eps magnitude keeps the zero direction; NaN normalizes.
        let (_, d) = sd([0.0; 3], 1000, 0.001, 1.0e-6, 1.0);
        assert!(d.is_none());
        let (_, d) = sd([f32::NAN, 0.0, 0.0], 1000, 0.001, 1.0e-6, 1.0);
        assert!(d.unwrap()[0].is_nan());
    }

    #[test]
    fn distance_fold() {
        assert_eq!(dist(500, 0.001, 8.0).to_bits(), 4.0f32.to_bits());
    }
}

/// Offset-delta front end for `CMovement::StepMovementWithOffset` (@0x616cb0).
///
/// `d = (anchor + offset) − pos` per lane, with the stock's asymmetry pinned —
/// the x/y sums stay WIDE into the subtract (one narrow each), but the z sum
/// spills narrowed (`FSTP [ebp-0x10]` at 0x616cd2) before its subtract.
pub fn step_offset_delta__616cb0(anchor: [f32; 3], off: [f32; 3], pos: [f32; 3]) -> [f32; 3] {
    let zsum = super::f64_to_f32(f64::from(anchor[2]) + f64::from(off[2]));
    [
        super::f64_to_f32(f64::from(anchor[0]) + f64::from(off[0]) - f64::from(pos[0])),
        super::f64_to_f32(f64::from(anchor[1]) + f64::from(off[1]) - f64::from(pos[1])),
        super::f64_to_f32(f64::from(zsum) - f64::from(pos[2])),
    ]
}

#[cfg(test)]
mod tests_step_offset__616cb0 {
    #[test]
    fn delta_fold() {
        let d =
            super::step_offset_delta__616cb0([1.0, 2.0, 3.0], [0.5, 0.25, 0.125], [1.0, 1.0, 1.0]);
        assert_eq!(d[0].to_bits(), 0.5f32.to_bits());
        assert_eq!(d[1].to_bits(), 1.25f32.to_bits());
        assert_eq!(d[2].to_bits(), 2.125f32.to_bits());
    }
}

/// WMO-group trace-segment normalize folds (0x6b9359..0x6b9385).
///
/// From the squared delta magnitude, `len = sqrt(sqmag)` (80-bit on the x87
/// stack), `len_dist = len × dist` and `inv_len = one / len`, each narrowed
/// once. The subsequent direction normalize is a separate
/// `ScaleInPlace(delta, inv_len)` hook call. Returns `(inv_len, len_dist)`.
pub fn trace_seg_normalize__6b92b0(sqmag: f32, dist: f32, one: f32) -> (f32, f32) {
    let len = f64::from(sqmag).sqrt();
    (
        super::f64_to_f32(f64::from(one) / len),
        super::f64_to_f32(len * f64::from(dist)),
    )
}

#[cfg(test)]
mod tests_trace_seg_normalize__6b92b0 {
    use super::trace_seg_normalize__6b92b0 as norm;

    #[test]
    fn splits_inv_len_and_len_dist() {
        // sqmag 16 -> len 4; inv 1/4 = 0.25; len*dist = 4*2.5 = 10.
        let (inv, ld) = norm(16.0, 2.5, 1.0);
        assert_eq!(inv, 0.25);
        assert_eq!(ld, 10.0);
    }

    #[test]
    fn zero_length_yields_inf_inv() {
        let (inv, ld) = norm(0.0, 3.0, 1.0);
        assert_eq!(inv, f32::INFINITY);
        assert_eq!(ld, 0.0);
    }
}

/// Water-interaction level computation (0x6030e0..0x603156).
///
/// The surface level `state × surfScale` and the ripple level
/// `state × rippleScale` stay 80-bit for their comparisons (returned as
/// `f64`), while the submerge offset `state × surfScale − off` is narrowed
/// once (`FSTP m32`). Returns `(surface_wide, offset_narrowed, ripple_wide)`.
pub fn water_levels__6030c0(
    state: f32,
    surf_scale: f32,
    off: f32,
    ripple_scale: f32,
) -> (f64, f32, f64) {
    let a = f64::from(state);
    let surface = a * f64::from(surf_scale);
    let offset = super::f64_to_f32(surface - f64::from(off));
    let ripple = a * f64::from(ripple_scale);
    (surface, offset, ripple)
}

#[cfg(test)]
mod tests_water_levels__6030c0 {
    use super::water_levels__6030c0 as levels;

    #[test]
    fn computes_surface_offset_ripple() {
        // state 2, surf 3, off 1, ripple 4 -> (6, 5, 8).
        let (s, o, r) = levels(2.0, 3.0, 1.0, 4.0);
        assert_eq!(s, 6.0);
        assert_eq!(o, 5.0);
        assert_eq!(r, 8.0);
    }
}
