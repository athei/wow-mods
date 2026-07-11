//! Branchless single-precision trig — re-exported from `wow_shared::trig`.
//!
//! The kernel (tuned Cody–Waite sin/cos + Cephes atan/atan2/acos), its
//! rationale (why hand-rolled beats both the C-libm libcall and the `libm`
//! crate here), and its tests moved to `wow-shared` so the d3d9
//! fixed-function state builder can use the same libcall-free trig; this
//! module keeps the `crate::math::trig::*` paths stable for the kernels.
pub use wow_shared::trig::{acos, atan2, sin, sin_cos};
