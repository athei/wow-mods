//! Generated symbol table for `Wow.exe` (`WoW` 1.12).
//!
//! Emitted by `build.rs` from `symbols.toml`; **do not edit by hand**. The
//! generated type/function names mirror the host's C++ symbols verbatim (e.g.
//! `C3Vector__Normalize`, where `__` marks the `::` boundary), so the
//! `nonstandard_style` lints are suppressed here as they are for any
//! machine-generated symbol table.
#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::pedantic,
    clippy::nursery,
    // The differential-mode dispatch forks the thunk's single call site into two
    // `#[cfg]` arms wrapped in a block; in statement position (void returns) that
    // reads as a redundant block expression. It is the intended shape — exactly
    // one arm survives expansion and it must not bypass the `preserve` shim.
    clippy::unnecessary_operation,
    // The generated `*_diff` capture functions mirror each hook's arity; unlike
    // the `extern` thunks they are plain `fn`s, so they trip the arg-count lint.
    clippy::too_many_arguments
)]

include!(concat!(env!("OUT_DIR"), "/symbols.rs"));
