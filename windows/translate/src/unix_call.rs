//! Thin PE-side wrapper around `wow_mods.dll`'s `wow_mods_unix_call` entry.
//!
//! `wow_translate.dll` is a native game-side mod (loaded from `mods/` by path),
//! so it cannot pair a `.so` itself. It reaches the unix side by raw-dylib
//! importing the dispatcher from the `wow_mods.dll` builtin bridge, which owns
//! the unixlib pairing. Importing this symbol makes Wine auto-load (and thus
//! initialize) `wow_mods.dll` before this mod's `DllMain` runs.

use core::ffi::c_void;

use wow_shared::Thunk;

#[link(name = "wow_mods", kind = "raw-dylib")]
unsafe extern "C" {
    fn wow_mods_unix_call(code: u32, args: *mut c_void) -> i32;
}

pub fn call<T: Thunk>(params: &mut T) -> i32 {
    // SAFETY: `params` is a live `&mut T` and `T::CODE` is the matching thunk
    // discriminant for `T`; the unix-side dispatcher casts back to `*mut T`
    // using the same code.
    unsafe { wow_mods_unix_call(T::CODE, std::ptr::from_mut::<T>(params).cast::<c_void>()) }
}
