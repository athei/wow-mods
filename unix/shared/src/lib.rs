use strum::{EnumCount, VariantArray};

pub mod crumb;
pub mod ffi_boundary;
pub mod ftol;
pub mod identity;
mod log_filter;
mod log_helpers;
mod params;
pub mod trig;
pub mod tsc;

pub use ffi_boundary::{InPtr, InPtrMut, OutPtr, ValueIn, VtableThis};
pub use log_filter::init_logger;
pub use params::{InitLoggerParams, TranslateParams, TranslateStatus};

/// Thunk discriminants shared by the PE side and the unix `.so`.
///
/// Both link this same crate, so the `#[repr(u32)]` ordering *is* the wire ABI.
/// `InitLogger` bootstraps `env_logger` on the unix side (fired once from the PE
/// `DllMain`); `Translate` carries a batch of chat through Apple's
/// `Translation.framework`.
#[repr(u32)]
#[derive(Clone, Copy, EnumCount, VariantArray)]
pub enum Thunks {
    InitLogger,
    Translate,
}

pub trait Thunk {
    const CODE: u32;
}
