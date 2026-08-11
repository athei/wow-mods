use core::ffi::c_void;

use strum::{EnumCount, VariantArray};
use wow_shared::Thunks;

mod handlers;

/// `log` target for every call in this `.so`.
///
/// The PE side fires a one-shot `InitLogger` thunk from `DllMain` (see
/// `handlers::init_logger_handler`) before any other call, which registers
/// `env_logger`; a handler that ran before it would silently no-op.
const LOG_TARGET: &str = "wow";

#[unsafe(no_mangle)]
pub static __wine_unix_call_funcs: [UnixCallFn; Thunks::COUNT] = DISPATCH_TABLE;

#[unsafe(no_mangle)]
pub static __wine_unix_call_wow64_funcs: [UnixCallFn; Thunks::COUNT] = DISPATCH_TABLE;

type UnixCallFn = unsafe extern "C" fn(*mut c_void) -> i32;

const DISPATCH_TABLE: [UnixCallFn; Thunks::COUNT] = build_dispatch_table();

/// Wrap a handler in an `@autoreleasepool`.
///
/// Every dispatch drains any autoreleased Apple objects the
/// Translation.framework call left on the thread. Wine's unix-call dispatcher
/// sets up no pool, so without this wrap they would live until thread exit.
/// Each macro invocation defines a uniquely-scoped `extern "C"` wrapper.
macro_rules! arp {
    ($inner:path) => {{
        extern "C" fn arp_wrap(args: *mut c_void) -> i32 {
            wow_shared::crumb!(stringify!($inner), args as usize as u64);
            objc2::rc::autoreleasepool(|_| $inner(args))
        }
        arp_wrap as UnixCallFn
    }};
}

const fn dispatch(code: Thunks) -> UnixCallFn {
    match code {
        Thunks::InitLogger => arp!(handlers::init_logger_handler),
        Thunks::Translate => arp!(handlers::translate_handler),
    }
}

const fn build_dispatch_table() -> [UnixCallFn; Thunks::COUNT] {
    extern "C" fn unimplemented_thunk(_args: *mut c_void) -> i32 {
        unimplemented!("Called unimplemented thunk.")
    }

    let mut table = [unimplemented_thunk as UnixCallFn; Thunks::COUNT];
    let variants = Thunks::VARIANTS;
    let mut i = 0;
    while i < variants.len() {
        table[variants[i] as usize] = dispatch(variants[i]);
        i += 1;
    }
    table
}
