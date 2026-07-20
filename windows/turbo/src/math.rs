//! Portable numeric reimplementations of `Wow.exe` math.
//!
//! No FFI and no host-image access — just the arithmetic, so this module builds
//! and is unit-tested on every target (including `x86_64-apple-darwin` under the
//! same Rosetta translation the shipped 32-bit DLL runs under). The 32-bit FFI
//! adapters that feed these kernels live in `win::hooks`.
//!
//! One submodule per `Wow.exe` math family. Each kernel is **plain portable
//! Rust left for LLVM to auto-vectorize** — the parity experiment measures how
//! close that gets to the reference DLL's hand-written SSE before any intrinsics
//! are introduced. Kernels compute in `f32` (matching the reference's SSE lane
//! width) unless a specific function must track an x87 80-bit original, in which
//! case it computes in `f64` and narrows through [`f64_to_f32`].

// <<MODULES>> managed by tools/re/assemble.py — do not edit between sentinels.
pub mod aabb;
pub mod boundsfit;
pub mod collision;
pub mod frustum;
pub mod gx;
pub mod light;
pub mod lua;
pub mod lua_gc;
pub mod m2;
pub mod matrix33;
pub mod matrix34;
pub mod matrix44;
pub mod misc;
pub mod movement;
pub mod object;
pub mod particle;
pub mod plane;
pub mod quaternion;
pub mod spline;
pub mod ui;
pub mod vector;
pub mod weather;
pub mod world;
// <</MODULES>>

// FMOD3 MPEG synthesis-filterbank dewindow (`fmod__mixer_fpu`) — the one fmod
// reimpl, hooked at runtime in `win::fmod` rather than via `symbols.toml` (fmod
// is a separate packed module, not `Wow.exe`). Hand-maintained, not generated —
// outside the assemble.py sentinels.
pub mod fmod_mixer;

// libm-free polynomial sin/cos shared by the trig kernels (Euler/axis-angle/
// quaternion/movement-arc). Hand-maintained, not generated — outside the
// assemble.py sentinels.
pub mod trig;

// Lua string-library literal fast-path kernels (gsub/gfind prefilter). Hand-
// maintained, not generated — outside the assemble.py sentinels.
pub mod strlib;

// Differential-mode comparator: compiled only when the harness exists
// (`wow_turbo_diff` builds) or under test, so the default DLL carries no dead code.
// Outside the assemble.py sentinels — it is hand-maintained, not generated.
#[cfg(any(wow_turbo_diff, test))]
pub mod ulp;

/// Narrowing `f64 → f32`.
///
/// A kernel that computes in `f64` to track an x87 80-bit original narrows each
/// result into an `f32` field, where the mantissa loss is acceptable. Most
/// kernels compute directly in `f32` and never need this.
#[allow(clippy::cast_possible_truncation)]
const fn f64_to_f32(v: f64) -> f32 {
    v as f32
}
