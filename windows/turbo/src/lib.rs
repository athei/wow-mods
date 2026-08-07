//! `wow_turbo.dll` — a native performance mod for `Wow.exe` (`WoW` 1.12).
//!
//! Replaces hot client functions with faster Rust reimplementations.
//!
//! The original machine code stays in place for everything not yet ported; each
//! ported function gets a `jmp` patched into its prologue redirecting to a Rust
//! reimplementation, which can still call back into the untouched code through
//! the trampoline `MinHook` preserves. Function addresses, signatures and hook
//! coverage are tracked in `symbols.toml`, which the build script turns into the
//! generated symbol table.
//!
//! Injected next to `wow.exe` via `version.dll`'s `dlls.txt`, the same path
//! `wow_translate` uses.
//!
//! This file is the one place the target split is declared. The crate splits
//! by portability:
//!
//! * `math` — the portable numeric reimplementations. They depend on nothing
//!   Windows-specific, so the crate also builds for `x86_64-apple-darwin` and
//!   they are unit-tested there under the same Rosetta translation the shipped
//!   32-bit DLL runs under (not a native-aarch64 stand-in).
//! * `win` — everything 32-bit and host-image-specific: the generated symbol
//!   table, the FFI adapters the thunks dispatch to, and the `DllMain` entry.
//!   It exists only for the shipped `i686` target.

#[cfg(any(target_arch = "x86", test))]
mod math;

/// Portable Storm/MPQ reimplementation kernels (currently the zlib sector-inflate codec).
///
/// Like `math`, they depend on nothing Windows-specific and are host-unit-tested
/// under the same toolchain.
#[cfg(any(target_arch = "x86", test))]
mod storm;

/// Portable visible-item write-coalescing kernel.
///
/// Suppresses the appearance flicker behind the descriptor-write hooks. Like
/// `math` and `storm`, it depends on nothing Windows-specific and is
/// host-unit-tested under the same toolchain.
#[cfg(any(target_arch = "x86", test))]
mod transmog;

#[cfg(target_arch = "x86")]
mod win;
