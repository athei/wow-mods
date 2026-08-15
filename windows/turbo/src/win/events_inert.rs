//! The event gauge's stand-in in a build without the diagnostic layer.
//!
//! `events.rs` holds the gauge itself and is compiled only under
//! `cfg(wow_turbo_perf)`; this file takes its place otherwise, so the module
//! path is the same in both builds and no call site carries a `cfg`.
//!
//! Only one entry point has to survive. Every hook the gauge observes through
//! is `armed_only` in the manifest and is dropped from the symbol table
//! entirely in this build, so those adapters are gone with it. [`time_body`]
//! is the exception: it is called from a reimplementation that installs
//! unconditionally, and it has to keep running the body it wraps.

/// Run a handler body, timing nothing.
///
/// The gauge's version brackets the body and attributes its cost to the script
/// file that owns it; with no gauge to report to, the chunk lookup is not
/// performed and the body is called exactly as the unarmed path calls it.
pub fn time_body<T>(_chunk: impl FnOnce() -> (usize, u32), body: impl FnOnce() -> T) -> T {
    body()
}
