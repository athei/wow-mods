//! Boundary newtype for the `WoW` 1.12 Lua C API.
//!
//! Each entry resolves a hardcoded VA to a typed `fastcall` fn pointer via a
//! `LazyLock` (one transmute per process, not per call). The [`LuaState`]
//! newtype concentrates the unsafe surface: a single `unsafe fn from_raw`
//! asserts that the inner pointer is a live Lua state, and every method is
//! safe to call from the dispatcher.
//!
//! Only the three selectors the live addon actually exercises are exposed:
//! `gettop`, `tostring`, `pushstring`. No boolean/number/nil since
//! `WoWTranslate` only marshals strings. [`LuaState::string_at_eq`] is a
//! non-allocating `tostring` for the command match on the dispatcher's hot
//! path, where every foreign `UnitXP` call is rejected on its first byte.

use core::ffi::{CStr, c_char, c_void};
use std::{ffi::CString, sync::LazyLock};

use crate::unit_xp_addrs::{P_LUA_GETTOP, P_LUA_PUSHSTRING, P_LUA_TOSTRING};

type LuaGettop = unsafe extern "fastcall" fn(*mut c_void) -> i32;
type LuaTostring = unsafe extern "fastcall" fn(*mut c_void, i32) -> *const c_char;
type LuaPushstring = unsafe extern "fastcall" fn(*mut c_void, *const c_char);

static LUA_GETTOP: LazyLock<LuaGettop> = LazyLock::new(|| {
    // SAFETY: WoW 1.12 client has `lua_gettop` at the fixed VA `P_LUA_GETTOP`
    // with `__fastcall` ABI matching `LuaGettop`.
    unsafe { core::mem::transmute::<usize, LuaGettop>(P_LUA_GETTOP) }
});

static LUA_TOSTRING: LazyLock<LuaTostring> = LazyLock::new(|| {
    // SAFETY: WoW 1.12 client has `lua_tostring` at the fixed VA
    // `P_LUA_TOSTRING` with `__fastcall` ABI matching `LuaTostring`.
    unsafe { core::mem::transmute::<usize, LuaTostring>(P_LUA_TOSTRING) }
});

static LUA_PUSHSTRING: LazyLock<LuaPushstring> = LazyLock::new(|| {
    // SAFETY: WoW 1.12 client has `lua_pushstring` at the fixed VA
    // `P_LUA_PUSHSTRING` with `__fastcall` ABI matching `LuaPushstring`.
    unsafe { core::mem::transmute::<usize, LuaPushstring>(P_LUA_PUSHSTRING) }
});

/// Borrowed Lua-state handle. Constructed once per hook entry from the raw
/// pointer `WoW` passes in; methods on it are safe.
pub struct LuaState(*mut c_void);

impl LuaState {
    /// # Safety
    /// `ptr` must be the live Lua-state pointer `WoW` passed to a hook
    /// entry. The state outlives the call frame the hook owns.
    #[must_use]
    pub const unsafe fn from_raw(ptr: *mut c_void) -> Self {
        Self(ptr)
    }

    pub fn gettop(&self) -> i32 {
        // SAFETY: `self.0` is a live Lua state per `from_raw`'s contract.
        unsafe { (*LUA_GETTOP)(self.0) }
    }

    /// Whether the string at `index` equals `expected`, without allocating.
    ///
    /// The dispatcher runs this against every `UnitXP` call any addon makes,
    /// so it must not do what [`tostring`](Self::tostring) does (strlen, UTF-8
    /// scan, heap copy) just to reject a foreign command at its first byte.
    /// Non-string at `index` (Lua reports null) is a non-match.
    pub fn string_at_eq(&self, index: i32, expected: &CStr) -> bool {
        // SAFETY: `self.0` is a live Lua state per `from_raw`'s contract.
        let raw = unsafe { (*LUA_TOSTRING)(self.0, index) };
        if raw.is_null() {
            return false;
        }
        // Compare `expected`'s trailing NUL too, so a Lua string that merely
        // starts with `expected` doesn't match.
        let mut got = raw.cast::<u8>();
        for &want in expected.to_bytes_with_nul() {
            // SAFETY: `got` advances one byte per matched byte of `expected`,
            // all of which are non-NUL, so the NUL terminating Lua's own buffer
            // lies at or after `got` — the read is in bounds.
            if unsafe { *got } != want {
                return false;
            }
            got = got.wrapping_add(1);
        }
        true
    }

    /// Read the string at `index` on the Lua stack. Returns `None` only when
    /// Lua reports null (non-string at index); bytes that aren't valid UTF-8
    /// (e.g. chat truncated mid-codepoint at the wire byte cap) are recovered
    /// lossily rather than rejected.
    pub fn tostring(&self, index: i32) -> Option<String> {
        // SAFETY: `self.0` is a live Lua state per `from_raw`'s contract.
        let raw = unsafe { (*LUA_TOSTRING)(self.0, index) };
        if raw.is_null() {
            return None;
        }
        // SAFETY: WoW's `lua_tostring` returns a NUL-terminated C string
        // backed by Lua's own buffer; valid for the duration of this call
        // before the next Lua stack mutation.
        let cstr = unsafe { CStr::from_ptr(raw) };
        Some(String::from_utf8_lossy(cstr.to_bytes()).into_owned())
    }

    /// Push `s` onto the Lua stack. Interior NULs are stripped at the
    /// `CString` boundary; Lua copies the bytes immediately so the
    /// `CString` is free to drop on return.
    pub fn pushstring(&self, s: &str) {
        let owned: Vec<u8> = s.bytes().filter(|&b| b != 0).collect();
        let Ok(cstring) = CString::new(owned) else {
            // CString::new only fails on interior NUL, which the filter
            // above strips. Logged as defensive — shouldn't fire.
            wow_shared::log_once_warn!(
                target: crate::LOG_TARGET,
                "pushstring: CString::new rejected sanitized input",
            );
            return;
        };
        // SAFETY: `self.0` is a live Lua state; `cstring.as_ptr()` is a
        // NUL-terminated C string. Lua copies into its own buffer before
        // returning, so `cstring` may drop immediately after.
        unsafe { (*LUA_PUSHSTRING)(self.0, cstring.as_ptr()) };
    }
}
