// Adapter and kernel names mirror the host's C++ symbols verbatim, with `__`
// standing in for the `::`, so the whole module is non-snake-case by construction.
#![allow(non_snake_case)]

/// Accumulate a point delta into a bounds-fit accumulator.
///
/// Running centroid sum (`sum`) plus a K-scaled diagonal accumulator (`diag`).
/// K = 3.544907808303833.
pub fn bounds_fit__accumulate_centroid__71bc70(
    sum: &[f32; 3],
    diag: &[f32; 3],
    delta: &[f32; 3],
) -> ([f32; 3], [f32; 3]) {
    const K: f32 = 3.544_907_8_f32;
    let new_sum = [sum[0] + delta[0], sum[1] + delta[1], sum[2] + delta[2]];
    let new_diag = [
        delta[0] * K + diag[0],
        delta[1] * K + diag[1],
        delta[2] * K + diag[2],
    ];
    (new_sum, new_diag)
}

#[cfg(test)]
mod tests_bounds_fit__accumulate_centroid__71bc70 {
    use super::bounds_fit__accumulate_centroid__71bc70 as f;

    const K: f32 = 3.544_907_8_f32;

    #[test]
    fn zero_delta_is_identity() {
        let s = [1.0_f32, 2.0, 3.0];
        let d = [4.0_f32, 5.0, 6.0];
        let (ns, nd) = f(&s, &d, &[0.0, 0.0, 0.0]);
        assert_eq!(ns, s);
        assert_eq!(nd, d);
    }

    #[test]
    fn known_value() {
        let (ns, nd) = f(&[0.0, 0.0, 0.0], &[0.0, 0.0, 0.0], &[1.0, -2.0, 0.5]);
        assert_eq!(ns, [1.0, -2.0, 0.5]);
        for i in 0..3 {
            let expect = [1.0_f32, -2.0, 0.5][i] * K;
            assert!((nd[i] - expect).abs() <= 1e-5 * expect.abs().max(1.0));
        }
    }

    #[test]
    fn additivity_two_deltas_equal_sum() {
        let a = [0.5_f32, -1.5, 2.25];
        let b = [-3.0_f32, 4.0, 0.125];
        let (s1, d1) = f(&[0.0; 3], &[0.0; 3], &a);
        let (s2, d2) = f(&s1, &d1, &b);
        let combined = [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
        let (sc, dc) = f(&[0.0; 3], &[0.0; 3], &combined);
        for i in 0..3 {
            assert!((s2[i] - sc[i]).abs() <= 1e-4);
            assert!((d2[i] - dc[i]).abs() <= 1e-3 * dc[i].abs().max(1.0));
        }
    }
}

/// Accumulate a 3-vector delta into the running normal/offset sum of a bounds-fit accumulator.
pub fn bounds_fit__accumulate_normal_sum__71bf60(sum: &[f32; 3], delta: &[f32; 3]) -> [f32; 3] {
    [sum[0] + delta[0], sum[1] + delta[1], sum[2] + delta[2]]
}

#[cfg(test)]
mod tests_bounds_fit__accumulate_normal_sum__71bf60 {
    use super::bounds_fit__accumulate_normal_sum__71bf60 as f;

    #[test]
    fn zero_is_identity() {
        let s = [1.5_f32, -2.5, 3.0];
        assert_eq!(f(&s, &[0.0, 0.0, 0.0]), s);
    }

    #[test]
    fn known_value() {
        assert_eq!(f(&[1.0, 2.0, 3.0], &[10.0, -20.0, 0.5]), [11.0, -18.0, 3.5]);
    }

    #[test]
    fn commutes_with_order() {
        let a = [0.25_f32, -1.0, 4.0];
        let b = [-3.5_f32, 2.0, 0.125];
        let ab = f(&f(&[0.0; 3], &a), &b);
        let ba = f(&f(&[0.0; 3], &b), &a);
        for i in 0..3 {
            assert!((ab[i] - ba[i]).abs() <= 1e-6);
        }
    }
}

/// Compute the inverse depth-range scale `1.0 / (far - near)`.
///
/// Stored by `BoundsFit::SetDepthRange`. The near/far/direction values are stored
/// verbatim by the caller; this is the sole computed term.
pub fn bounds_fit__set_depth_range__71c110(near_depth: f32, far_depth: f32) -> f32 {
    1.0_f32 / (far_depth - near_depth)
}

#[cfg(test)]
mod tests_bounds_fit__set_depth_range__71c110 {
    use super::bounds_fit__set_depth_range__71c110 as f;

    #[test]
    fn known_value() {
        assert_eq!(f(0.0, 4.0), 0.25_f32);
        assert_eq!(f(1.0, 2.0), 1.0_f32);
    }

    #[test]
    fn inverse_relationship() {
        for &(n, fr) in &[(0.5_f32, 10.0_f32), (-3.0, 7.0), (100.0, 1000.0)] {
            let s = f(n, fr);
            assert!((s * (fr - n) - 1.0).abs() <= 1e-5);
        }
    }

    #[test]
    fn negative_range_negates_scale() {
        assert!(f(5.0, 1.0) < 0.0);
        assert_eq!(f(5.0, 1.0), -f(1.0, 5.0));
    }
}

/// Per-object moment accumulation deltas: (moment[12], wsum[3], shX[9], shY[9], shZ[9]).
#[cfg(test)]
type MomentAccum = ([f32; 12], [f32; 3], [f32; 9], [f32; 9], [f32; 9]);

/// The host constants one moment accumulation folds in, gathered once per call.
///
/// `sh_dc` is the band-0 DC term, `k` the band-2 scales K2..K7, `c3` and `one`
/// the band-2 zonal pair, `k1` the per-sample weight scale, and `wq_c` the
/// fourth moment row's recombination weights. They are grouped because the
/// caller reads all six from fixed `.rdata` addresses before any block is
/// touched, and the basis below consumes them together.
pub struct MomentConsts {
    pub sh_dc: f32,
    pub k: [f32; 6],
    pub c3: f32,
    pub one: f32,
    pub k1: f32,
    pub wq_c: [f32; 3],
}

/// The per-sample direction basis every accumulator block folds against.
///
/// `v` is the optionally rotated direction, `row_w` the four moment row weights
/// (`weight` plus the recombined `wq`), `band` the nine real SH basis values of
/// `v`, and `sh_w` the three per-axis band scales `w{x,y,z}*k1`.
pub struct MomentBasis {
    pub v: [f32; 3],
    pub row_w: [f32; 4],
    pub band: [f32; 9],
    pub sh_w: [f32; 3],
}

/// The direction basis for one weighted sample of the SH9 moment accumulator.
///
/// Optionally rotates `vec` by the column-major 3x3 `rot` (rows
/// `(rot[0],rot[3],rot[6])`, `(rot[1],rot[4],rot[7])`,
/// `(rot[2],rot[5],rot[8])`), recombines the fourth moment row's weight
/// `wq = w2*c0 + w0*c1 + w1*c2`, and evaluates the nine real SH basis values of
/// the rotated direction in the original's negated-component form (index 0 is
/// the band-0 DC constant, 1..3 band-1, 4..8 band-2). It is split from the fold
/// so a caller can build it once and then read, combine and write back each
/// accumulator block in turn, rather than copying all five blocks in up front.
pub fn bounds_fit__moment_basis__71bce0(
    weight: &[f32; 3],
    vec: &[f32; 3],
    rotate: bool,
    rot: &[f32; 9],
    consts: &MomentConsts,
) -> MomentBasis {
    // Optional 3x3 rotation: v = rot * vec (rot column-major, so each output row
    // dots a strided row of `rot` with `vec`).
    let v = if rotate {
        [
            rot[0] * vec[0] + rot[3] * vec[1] + rot[6] * vec[2],
            rot[1] * vec[0] + rot[4] * vec[1] + rot[7] * vec[2],
            rot[2] * vec[0] + rot[5] * vec[1] + rot[8] * vec[2],
        ]
    } else {
        *vec
    };
    let [vx, vy, vz] = v;

    // Fourth moment row uses a recombined weight wq = w2*c0 + w0*c1 + w1*c2.
    let wq = weight[2] * consts.wq_c[0] + weight[0] * consts.wq_c[1] + weight[1] * consts.wq_c[2];

    let [k2, k3, k4, k5, k6, k7] = consts.k;
    let band = [
        consts.sh_dc,
        (-vy) * k7,
        k6 * (-vz),
        (-vx) * k7,
        (-vy) * (-vx) * k5,
        (-vz) * (-vy) * k4,
        ((-vz) * (-vz) * consts.c3 - consts.one) * k3,
        (-vz) * (-vx) * k4,
        (vx * vx - vy * vy) * k2,
    ];

    MomentBasis {
        v,
        row_w: [weight[0], weight[1], weight[2], wq],
        band,
        sh_w: [
            weight[0] * consts.k1,
            weight[1] * consts.k1,
            weight[2] * consts.k1,
        ],
    }
}

/// Folds one weighted sample into the 3x4 moment block.
///
/// Each of the four rows accumulates `row_w[r] * (vx,vy,vz)` onto the matching
/// row of `prior`; the multiply and the add stay separate operations.
pub fn bounds_fit__moment_block__71bce0(basis: &MomentBasis, prior: &[f32; 12]) -> [f32; 12] {
    let [vx, vy, vz] = basis.v;
    let mut out = [0.0f32; 12];
    for ((dst, w), src) in out
        .chunks_exact_mut(3)
        .zip(basis.row_w)
        .zip(prior.chunks_exact(3))
    {
        dst[0] = w * vx + src[0];
        dst[1] = w * vy + src[1];
        dst[2] = w * vz + src[2];
    }
    out
}

/// Folds the sample weights into the running per-axis weight sums.
pub fn bounds_fit__weight_sums__71bce0(weight: &[f32; 3], prior: &[f32; 3]) -> [f32; 3] {
    core::array::from_fn(|i| prior[i] + weight[i])
}

/// Folds one 9-coefficient SH band against the sample's basis.
///
/// Each coefficient takes `scale * band[j]` onto its prior value, the multiply
/// and the add left as separate operations.
pub fn bounds_fit__sh_band__71bce0(basis: &MomentBasis, prior: &[f32; 9], scale: f32) -> [f32; 9] {
    core::array::from_fn(|j| scale * basis.band[j] + prior[j])
}

/// Spherical-harmonics (SH9) second-moment accumulator for an ambient/bounds-fit accumulator.
///
/// Optionally rotates the direction `vec` by the column-major 3x3
/// `rot` (rows `(rot[0],rot[3],rot[6])`, `(rot[1],rot[4],rot[7])`,
/// `(rot[2],rot[5],rot[8])`) when `rotate` is set, then folds `weight` into: (a)
/// the 3x4 moment block `moment` (`weight[i] * v` for the first three rows plus a
/// fourth row weighted by `wq = w2*c0 + w0*c1 + w1*c2`), (b) the running weight
/// sums `wsum`, and (c) three 9-coefficient SH bands `sh_x/sh_y/sh_z` (each `+=
/// w{x,y,z}*k1 * band[k]` over the 9 real SH basis values of `v`). Returns the
/// updated blocks. `sh_dc`, `k` (band-2 scales K2..K7), `c3`, `one`, `k1`, and
/// `wq_c` are host constants.
///
/// This is the composition the four pieces above make, in one call and taking
/// all five accumulator blocks at once. The shipped caller does not use it: it
/// builds the basis and then reads, folds and writes back one block at a time,
/// which is what keeps the blocks out of stack temporaries. What is left is the
/// oracle the pinned-value tests below compare against, so it is scoped to test
/// builds rather than shipped with no caller.
// The parameters enumerate the accumulator blocks and the host constants the
// original reached through its object pointer and static data. The count is
// set by what the original touches, not by a design choice here.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub fn bounds_fit__accumulate_moment__71bce0(
    weight: &[f32; 3],
    vec: &[f32; 3],
    rotate: bool,
    rot: &[f32; 9],
    moment: &[f32; 12],
    wsum: &[f32; 3],
    sh_x: &[f32; 9],
    sh_y: &[f32; 9],
    sh_z: &[f32; 9],
    sh_dc: f32,
    k: &[f32; 6],
    c3: f32,
    one: f32,
    k1: f32,
    wq_c: &[f32; 3],
) -> MomentAccum {
    let consts = MomentConsts {
        sh_dc,
        k: *k,
        c3,
        one,
        k1,
        wq_c: *wq_c,
    };
    let basis = bounds_fit__moment_basis__71bce0(weight, vec, rotate, rot, &consts);
    (
        bounds_fit__moment_block__71bce0(&basis, moment),
        bounds_fit__weight_sums__71bce0(weight, wsum),
        bounds_fit__sh_band__71bce0(&basis, sh_x, basis.sh_w[0]),
        bounds_fit__sh_band__71bce0(&basis, sh_y, basis.sh_w[1]),
        bounds_fit__sh_band__71bce0(&basis, sh_z, basis.sh_w[2]),
    )
}

#[cfg(test)]
mod tests_bounds_fit__accumulate_moment__71bce0 {
    use super::bounds_fit__accumulate_moment__71bce0 as f;

    // The SH band scale the client passes in, reproduced bit-exactly; trimming
    // digits changes the value the reimplementation is tested against.
    #[allow(clippy::excessive_precision)]
    const K1: f32 = 2.956_793_1_f32;
    const K2: f32 = 0.546_274_24_f32;
    const K3: f32 = 0.315_391_6_f32;
    const K4: f32 = -1.092_548_5_f32;
    const K5: f32 = 1.092_548_5_f32;
    const K6: f32 = 0.488_602_52_f32;
    const K7: f32 = -0.488_602_52_f32;
    const SH_DC: f32 = 0.282_094_8_f32;
    const C3: f32 = 3.0_f32;
    const ONE: f32 = 1.0_f32;
    // The host's `wq` recombination weights, reproduced bit-exactly; trimming
    // digits changes the value the reimplementation is tested against.
    #[allow(clippy::excessive_precision)]
    const WQ_C: [f32; 3] = [0.072_168_998_f32, 0.212_671_f32, 0.715_16_f32];
    const K: [f32; 6] = [K2, K3, K4, K5, K6, K7];
    const ID3: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

    // Wrapper over the reimplementation: the parameters are the subset of that
    // function's ABI-sized signature the tests vary, so the count is inherited
    // from the item under test rather than chosen here.
    #[allow(clippy::too_many_arguments)]
    fn call(
        weight: &[f32; 3],
        vec: &[f32; 3],
        rotate: bool,
        rot: &[f32; 9],
        moment: &[f32; 12],
        wsum: &[f32; 3],
        sh_x: &[f32; 9],
        sh_y: &[f32; 9],
        sh_z: &[f32; 9],
    ) -> super::MomentAccum {
        f(
            weight, vec, rotate, rot, moment, wsum, sh_x, sh_y, sh_z, SH_DC, &K, C3, ONE, K1, &WQ_C,
        )
    }

    #[test]
    fn zero_weight_is_identity() {
        let (m, w, sx, sy, sz) = call(
            &[0.0, 0.0, 0.0],
            &[0.3, -0.7, 0.5],
            false,
            &ID3,
            &[1.0; 12],
            &[2.0; 3],
            &[3.0; 9],
            &[4.0; 9],
            &[5.0; 9],
        );
        assert_eq!(m, [1.0; 12]);
        assert_eq!(w, [2.0; 3]);
        assert_eq!(sx, [3.0; 9]);
        assert_eq!(sy, [4.0; 9]);
        assert_eq!(sz, [5.0; 9]);
    }

    #[test]
    fn identity_rotation_equals_no_rotation() {
        let weight = [0.5_f32, -1.0, 2.0];
        let vec = [0.4_f32, 0.6, -0.8];
        let a = call(
            &weight, &vec, false, &ID3, &[0.0; 12], &[0.0; 3], &[0.0; 9], &[0.0; 9], &[0.0; 9],
        );
        let b = call(
            &weight, &vec, true, &ID3, &[0.0; 12], &[0.0; 3], &[0.0; 9], &[0.0; 9], &[0.0; 9],
        );
        assert_eq!(a, b);
    }

    #[test]
    fn weight_sums_accumulate() {
        let (_, w, ..) = call(
            &[1.0, 2.0, 3.0],
            &[0.0, 0.0, 1.0],
            false,
            &ID3,
            &[0.0; 12],
            &[10.0, 20.0, 30.0],
            &[0.0; 9],
            &[0.0; 9],
            &[0.0; 9],
        );
        assert_eq!(w, [11.0, 22.0, 33.0]);
    }

    #[test]
    fn moment_first_row_is_weighted_direction() {
        let (m, ..) = call(
            &[2.0, 0.0, 0.0],
            &[0.1, 0.2, 0.3],
            false,
            &ID3,
            &[0.0; 12],
            &[0.0; 3],
            &[0.0; 9],
            &[0.0; 9],
            &[0.0; 9],
        );
        assert!((m[0] - 2.0 * 0.1).abs() <= 1e-6);
        assert!((m[1] - 2.0 * 0.2).abs() <= 1e-6);
        assert!((m[2] - 2.0 * 0.3).abs() <= 1e-6);
    }

    #[test]
    fn fourth_moment_row_uses_wq() {
        let weight = [1.0_f32, 2.0, 3.0];
        let wq = weight[2] * WQ_C[0] + weight[0] * WQ_C[1] + weight[1] * WQ_C[2];
        let (m, ..) = call(
            &weight,
            &[1.0, 0.0, 0.0],
            false,
            &ID3,
            &[0.0; 12],
            &[0.0; 3],
            &[0.0; 9],
            &[0.0; 9],
            &[0.0; 9],
        );
        assert!((m[9] - wq).abs() <= 1e-6);
        assert!(m[10].abs() <= 1e-6);
        assert!(m[11].abs() <= 1e-6);
    }

    #[test]
    fn sh_dc_band_is_constant_times_weight() {
        let weight = [1.5_f32, 0.0, 0.0];
        let (_, _, sx, ..) = call(
            &weight,
            &[0.3, 0.5, -0.7],
            false,
            &ID3,
            &[0.0; 12],
            &[0.0; 3],
            &[0.0; 9],
            &[0.0; 9],
            &[0.0; 9],
        );
        assert!((sx[0] - weight[0] * K1 * SH_DC).abs() <= 1e-6);
    }

    #[test]
    fn sh_band2_zonal_known_value() {
        // band[6] = (3 vz^2 - 1) * K3; for v = +z it is (3-1)*K3.
        let weight = [1.0_f32, 0.0, 0.0];
        let (_, _, sx, ..) = call(
            &weight,
            &[0.0, 0.0, 1.0],
            false,
            &ID3,
            &[0.0; 12],
            &[0.0; 3],
            &[0.0; 9],
            &[0.0; 9],
            &[0.0; 9],
        );
        let band6 = (3.0_f32 * 1.0 - 1.0) * K3;
        assert!((sx[6] - weight[0] * K1 * band6).abs() <= 1e-6);
    }

    #[test]
    fn rotation_permutes_axes() {
        // 90-degree rotation about z (column-major): v' = (-vy, vx, vz). The
        // rotated path must match feeding v' directly with no rotation.
        let rot = [0.0_f32, 1.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        let weight = [0.7_f32, 1.3, -0.9];
        let vec = [0.2_f32, 0.5, -0.4];
        let vprime = [-vec[1], vec[0], vec[2]];
        let rotated = call(
            &weight, &vec, true, &rot, &[0.0; 12], &[0.0; 3], &[0.0; 9], &[0.0; 9], &[0.0; 9],
        );
        let direct = call(
            &weight, &vprime, false, &ID3, &[0.0; 12], &[0.0; 3], &[0.0; 9], &[0.0; 9], &[0.0; 9],
        );
        for i in 0..12 {
            assert!((rotated.0[i] - direct.0[i]).abs() <= 1e-5, "moment[{i}]");
        }
        for i in 0..9 {
            assert!((rotated.2[i] - direct.2[i]).abs() <= 1e-5, "sh_x[{i}]");
            assert!((rotated.3[i] - direct.3[i]).abs() <= 1e-5, "sh_y[{i}]");
            assert!((rotated.4[i] - direct.4[i]).abs() <= 1e-5, "sh_z[{i}]");
        }
    }

    #[test]
    fn additivity_two_accumulations() {
        // Folding two samples (fixed v) equals folding their summed weights.
        let v = [0.3_f32, -0.6, 0.74];
        let wa = [0.5_f32, 1.0, -0.5];
        let wb = [2.0_f32, -1.0, 3.0];
        let s1 = call(
            &wa, &v, false, &ID3, &[0.0; 12], &[0.0; 3], &[0.0; 9], &[0.0; 9], &[0.0; 9],
        );
        let s2 = call(&wb, &v, false, &ID3, &s1.0, &s1.1, &s1.2, &s1.3, &s1.4);
        let wsum = [wa[0] + wb[0], wa[1] + wb[1], wa[2] + wb[2]];
        let combined = call(
            &wsum, &v, false, &ID3, &[0.0; 12], &[0.0; 3], &[0.0; 9], &[0.0; 9], &[0.0; 9],
        );
        for i in 0..12 {
            assert!((s2.0[i] - combined.0[i]).abs() <= 1e-4, "moment[{i}]");
        }
        for i in 0..3 {
            assert!((s2.1[i] - combined.1[i]).abs() <= 1e-5, "wsum[{i}]");
        }
        for i in 0..9 {
            assert!((s2.2[i] - combined.2[i]).abs() <= 1e-4, "sh_x[{i}]");
            assert!((s2.3[i] - combined.3[i]).abs() <= 1e-4, "sh_y[{i}]");
            assert!((s2.4[i] - combined.4[i]).abs() <= 1e-4, "sh_z[{i}]");
        }
    }
}

/// A fixed 4-slot max-heap keyed by squared distance.
///
/// Each key is paired with an opaque object handle. Mirrors the in-struct heap a
/// bounds-fit accumulator keeps in `objs`/`keys`/`count` for the `kind == 1`
/// (far-object) path. `repr(C)` because that mirror is literal: the three fields
/// land at +0, +0x10 and +0x20, which is the accumulator's `this+0x160`,
/// `this+0x170` and `this+0x180`, so the hook updates the client's own storage
/// through this type instead of copying it in and back out.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FarHeap {
    pub objs: [u32; 4],
    pub keys: [f32; 4],
    pub count: u32,
}

/// Insert `(dist_sq, object)` into the fixed 4-slot max-heap, in place.
///
/// When the heap has free room (`count < 4`) the entry is appended and sifted up
/// (a parent strictly less than `dist_sq` is pushed down). When full, the entry
/// replaces the root only if `dist_sq` is strictly less than the current maximum
/// (`keys[0]`), then sifts down toward the larger child while `dist_sq` is
/// strictly less than that child.
///
/// Updating `heap` rather than returning a fresh one is what lets the caller
/// hand over the client's own heap fields: the sift already read and wrote one
/// working copy in the original's slot order, and the rejecting path writes
/// nothing where the by-value form stored every slot back over its own bits.
// The sift-up guard is written `!(dist_sq > keys[parent])` to keep the
// original's ordered test: a NaN key stops the loop, where `dist_sq <=
// keys[parent]` would let it run on. The NaN polarity is load-bearing.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn bounds_fit__add_object__71bf90(heap: &mut FarHeap, dist_sq: f32, object: u32) {
    let count = heap.count;

    if count < 4 {
        // Sift-up: continue while the new key is strictly greater than its parent
        // (max-heap), i.e. swap the parent down until `dist_sq <= parent`.
        let mut idx = count as usize;
        while idx != 0 {
            let parent = (idx - 1) >> 1;
            // Continue only while `dist_sq > keys[parent]` (strict). Equality or
            // less stops, matching `FCOM; TEST AH,0x41; JNP`.
            if !(dist_sq > heap.keys[parent]) {
                break;
            }
            heap.objs[idx] = heap.objs[parent];
            heap.keys[idx] = heap.keys[parent];
            idx = parent;
        }
        heap.objs[idx] = object;
        heap.keys[idx] = dist_sq;
        heap.count = count + 1;
        return;
    }

    // Full: only displace the current maximum when strictly closer than it.
    if dist_sq < heap.keys[0] {
        let mut idx = 0usize;
        loop {
            let right = idx * 2 + 2;
            let mut child = idx * 2 + 1;
            if right < 4 {
                // Pick the larger child (`left <= right` selects right).
                if heap.keys[child] <= heap.keys[right] {
                    child = right;
                }
            } else if child >= 4 {
                break;
            }
            // Stop sifting once `dist_sq >= keys[child]` (place here); continue
            // only while strictly less than the chosen (larger) child.
            if heap.keys[child] <= dist_sq {
                break;
            }
            heap.objs[idx] = heap.objs[child];
            heap.keys[idx] = heap.keys[child];
            idx = child;
        }
        heap.objs[idx] = object;
        heap.keys[idx] = dist_sq;
    }
}

#[cfg(test)]
mod tests_bounds_fit__add_object__71bf90 {
    use super::{FarHeap, bounds_fit__add_object__71bf90 as f};

    fn empty() -> FarHeap {
        FarHeap {
            objs: [0; 4],
            keys: [0.0; 4],
            count: 0,
        }
    }

    fn is_max_heap(h: &FarHeap) -> bool {
        let n = h.count as usize;
        for i in 1..n {
            let p = (i - 1) >> 1;
            if h.keys[p] < h.keys[i] {
                return false;
            }
        }
        true
    }

    #[test]
    fn fills_then_root_is_max() {
        let mut h = empty();
        for (k, o) in [(3.0_f32, 30), (1.0, 10), (4.0, 40), (2.0, 20)] {
            f(&mut h, k, o);
        }
        assert_eq!(h.count, 4);
        assert!(is_max_heap(&h));
        assert_eq!(h.keys[0], 4.0);
        assert_eq!(h.objs[0], 40);
    }

    #[test]
    fn full_keeps_four_smallest() {
        let mut h = empty();
        for (k, o) in [(5.0_f32, 1), (8.0, 2), (1.0, 3), (9.0, 4)] {
            f(&mut h, k, o);
        }
        for (k, o) in [(2.0_f32, 5), (0.5, 6)] {
            f(&mut h, k, o);
        }
        assert_eq!(h.count, 4);
        assert!(is_max_heap(&h));
        let mut got: std::vec::Vec<f32> = h.keys.to_vec();
        got.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(got, std::vec![0.5, 1.0, 2.0, 5.0]);
    }

    #[test]
    fn full_rejects_value_not_smaller_than_max() {
        let mut h = empty();
        for (k, o) in [(5.0_f32, 1), (8.0, 2), (1.0, 3), (9.0, 4)] {
            f(&mut h, k, o);
        }
        let before = h;
        f(&mut h, 9.0, 99);
        assert_eq!(h, before);
        f(&mut h, 100.0, 99);
        assert_eq!(h, before);
    }

    #[test]
    fn object_handle_tracks_its_key() {
        let mut h = empty();
        for (k, o) in [(7.0_f32, 70), (3.0, 30), (5.0, 50), (1.0, 10)] {
            f(&mut h, k, o);
        }
        for (k, o) in [(7.0_f32, 70), (3.0, 30), (5.0, 50), (1.0, 10)] {
            let idx = h.keys.iter().position(|&v| v == k).unwrap();
            assert_eq!(h.objs[idx], o);
        }
    }

    #[test]
    fn order_independent_membership() {
        let a = {
            let mut h = empty();
            for (k, o) in [(4.0_f32, 1), (2.0, 2), (6.0, 3), (1.0, 4), (3.0, 5)] {
                f(&mut h, k, o);
            }
            let mut v = h.keys.to_vec();
            v.sort_by(|x, y| x.partial_cmp(y).unwrap());
            v
        };
        let b = {
            let mut h = empty();
            for (k, o) in [(3.0_f32, 5), (1.0, 4), (6.0, 3), (2.0, 2), (4.0, 1)] {
                f(&mut h, k, o);
            }
            let mut v = h.keys.to_vec();
            v.sort_by(|x, y| x.partial_cmp(y).unwrap());
            v
        };
        assert_eq!(a, b);
        assert_eq!(a, std::vec![1.0, 2.0, 3.0, 4.0]);
    }
}

/// Rotate an accumulated fit direction by the upper-left 3x3 of a column-major 4x4.
///
/// Linear, no translation. Then normalize it when its magnitude is at least `eps`.
/// `norm_k` is the normalize numerator (`v *= norm_k / |v|`, stock 1.0); below
/// `eps` the direction is left as the rotated (un-normalized) value. Returns the
/// finalized direction.
pub fn bounds_fit__finalize_planes__71c160(
    dir: &[f32; 3],
    mat: &[f32; 16],
    eps: f32,
    norm_k: f32,
) -> [f32; 3] {
    let dx = dir[0];
    let dy = dir[1];
    let dz = dir[2];

    // Column-major upper-left 3x3 linear transform: new[r] = M[r]*dx + M[4+r]*dy
    // + M[8+r]*dz. Matches the source's strided reads (no translation column).
    let rx = dx * mat[0] + dy * mat[4] + dz * mat[8];
    let ry = dx * mat[1] + dy * mat[5] + dz * mat[9];
    let rz = dx * mat[2] + dy * mat[6] + dz * mat[10];

    let mag = (rx * rx + ry * ry + rz * rz).sqrt();
    // x87 `FCOMP eps` then `TEST AH,0x5; JNP` normalizes only when `eps <= |mag|`
    // (mag is already non-negative from the sqrt, so `abs` is a no-op here).
    if mag.abs() >= eps {
        let inv = norm_k / mag;
        [rx * inv, ry * inv, rz * inv]
    } else {
        [rx, ry, rz]
    }
}

#[cfg(test)]
mod tests_bounds_fit__finalize_planes__71c160 {
    use super::bounds_fit__finalize_planes__71c160 as f;

    const ID: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ];

    #[test]
    fn identity_normalizes_to_unit() {
        let r = f(&[3.0, 0.0, 4.0], &ID, 1.0e-10, 1.0);
        assert!((r[0] - 0.6).abs() <= 1e-6);
        assert!((r[1] - 0.0).abs() <= 1e-6);
        assert!((r[2] - 0.8).abs() <= 1e-6);
    }

    #[test]
    fn result_is_unit_length() {
        let r = f(&[1.0, 2.0, -2.0], &ID, 1.0e-10, 1.0);
        let m = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt();
        assert!((m - 1.0).abs() <= 1e-6);
    }

    #[test]
    fn below_epsilon_left_unnormalized() {
        let r = f(&[1.0e-6, 0.0, 0.0], &ID, 1.0e-3, 1.0);
        assert_eq!(r, [1.0e-6, 0.0, 0.0]);
    }

    #[test]
    fn column_major_rotation_90z() {
        let rot = [
            0.0, 1.0, 0.0, 0.0, //
            -1.0, 0.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ];
        let r = f(&[1.0, 0.0, 0.0], &rot, 1.0e-10, 1.0);
        assert!((r[0] - 0.0).abs() <= 1e-6);
        assert!((r[1] - 1.0).abs() <= 1e-6);
        assert!((r[2] - 0.0).abs() <= 1e-6);
    }

    #[test]
    fn norm_k_scales_output() {
        let a = f(&[3.0, 0.0, 4.0], &ID, 1.0e-10, 1.0);
        let b = f(&[3.0, 0.0, 4.0], &ID, 1.0e-10, 2.0);
        for i in 0..3 {
            assert!((b[i] - 2.0 * a[i]).abs() <= 1e-6);
        }
    }
}
