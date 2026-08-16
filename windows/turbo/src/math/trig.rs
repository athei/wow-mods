//! Branchless single-precision trig — re-exported from `wow_shared::trig`.
//!
//! The kernel (tuned Cody–Waite sin/cos + Cephes atan/atan2/acos), its
//! rationale (why hand-rolled beats both the C-libm libcall and the `libm`
//! crate here), and its tests moved to `wow-shared` so the d3d9
//! fixed-function state builder can use the same libcall-free trig; this
//! module keeps the `crate::math::trig::*` paths stable for the kernels.
pub use wow_shared::trig::{acos, atan2, sin, sin_cos, sin_cos3};

/// The packed `sin_cos3` is pinned here rather than in `wow-shared`.
///
/// Its lane-packed body is `x86`/`x86_64`-only, and this crate's tests are the
/// ones that run on an x86 target (`x86_64-apple-darwin`, under the same
/// translation the shipped 32-bit image uses); `wow-shared`'s own tests run on
/// the native host, where they would exercise the portable arm and prove
/// nothing about the packing.
#[cfg(test)]
mod tests_sin_cos3 {
    use super::{sin_cos, sin_cos3};

    /// Assert the packed triple is bit-identical to three scalar calls.
    fn same(x: [f32; 3]) {
        let (ps, pc) = sin_cos3(x);
        for i in 0..3 {
            let (s, c) = sin_cos(x[i]);
            assert_eq!(ps[i].to_bits(), s.to_bits(), "sin lane {i}, x={:e}", x[i]);
            assert_eq!(pc[i].to_bits(), c.to_bits(), "cos lane {i}, x={:e}", x[i]);
        }
    }

    #[test]
    fn dense_sweep_matches_the_scalar_kernel_bit_for_bit() {
        // Several periods, both signs, stepped so the triples do not stay in
        // one octant relative to each other.
        let n = 6000;
        for i in 0..n {
            let a = (i as f32 / n as f32) * 64.0 - 32.0;
            same([a, -a * 0.5, a + 0.797]);
        }
    }

    #[test]
    fn octant_boundaries_match() {
        // The reduction indexes octants of pi/4, so walk the boundaries and
        // their immediate neighbours in both directions.
        let quarter = core::f32::consts::FRAC_PI_4;
        for k in -40..40 {
            let b = quarter * k as f32;
            for d in [-2.0e-6_f32, -1.0e-7, 0.0, 1.0e-7, 2.0e-6] {
                same([b + d, b - d, -b + d]);
            }
        }
    }

    #[test]
    fn signed_zeros_subnormals_and_specials_match() {
        let odd = [
            0.0_f32,
            -0.0,
            f32::MIN_POSITIVE,
            -f32::MIN_POSITIVE,
            1e-45, // smallest subnormal
            -1e-45,
            f32::EPSILON,
            8191.999,
            -8191.999,
            8192.0, // exactly the fast-path limit: the whole triple goes scalar
            -8192.0,
            1.0e9,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
        ];
        // Every ordered triple of the awkward values, so a lane that leaves the
        // fast path is tested next to lanes that would have stayed on it.
        for &a in &odd {
            for &b in &odd {
                for &c in &odd {
                    let (ps, pc) = sin_cos3([a, b, c]);
                    for (i, &x) in [a, b, c].iter().enumerate() {
                        let (s, c_) = sin_cos(x);
                        assert_eq!(ps[i].is_nan(), s.is_nan(), "sin nan-ness lane {i}, x={x:e}");
                        assert_eq!(
                            pc[i].is_nan(),
                            c_.is_nan(),
                            "cos nan-ness lane {i}, x={x:e}"
                        );
                        if !s.is_nan() {
                            assert_eq!(ps[i].to_bits(), s.to_bits(), "sin lane {i}, x={x:e}");
                            assert_eq!(pc[i].to_bits(), c_.to_bits(), "cos lane {i}, x={x:e}");
                        }
                    }
                }
            }
        }
    }
}
