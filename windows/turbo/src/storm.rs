//! Portable Storm (MPQ) reimplementation kernels.
//!
//! Host-testable, with no Windows/host-image dependency, mirroring
//! `crate::math`. The FFI adapters that present these to the generated thunks
//! live in `crate::win::hooks`.

pub mod filecache;
pub mod inflate;
