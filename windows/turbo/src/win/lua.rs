//! Boundary newtype for the host's Lua C API, shared by script-facing hooks.
//!
//! Argument *reads* decode the state's stack slots directly (top at `+0x8`,
//! base at `+0xc`, 16-byte slots of tag / taint / payload), so a foreign call
//! is classified without calling into the host and without the in-place
//! coercion `lua_tostring` performs on a number slot. Pushes go through the
//! host's own entry points, which carry the taint stamping and the
//! GC-threshold check a hand-rolled push would have to transcribe.
//!
//! Every entry point is `fastcall(ecx = L, edx = second argument)` with any
//! further arguments on the stack, callee-cleaned.

/// `lua_isnumber` — `fastcall(ecx = L, edx = index)`, non-mutating probe.
const LUA_ISNUMBER_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x002f_34d0;
/// `lua_tonumber` — `fastcall(ecx = L, edx = index)`, returns the f64 in `ST0`.
const LUA_TONUMBER_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x002f_3620;
/// `lua_tostring` — `fastcall(ecx = L, edx = index)`; coerces a number IN PLACE.
const LUA_TOSTRING_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x002f_3690;
/// `lua_pushnil` — `fastcall(ecx = L)`.
const LUA_PUSHNIL_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x002f_37f0;
/// `lua_pushnumber` — `fastcall(ecx = L) + stack(f64)`, `RET 8`.
const LUA_PUSHNUMBER_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x002f_3810;
/// `lua_pushlstring` — `fastcall(ecx = L, edx = s) + stack(len)`, `RET 4`.
const LUA_PUSHLSTRING_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x002f_3840;
/// `lua_pushboolean` — `fastcall(ecx = L, edx = b)`, normalizes `b` to 0/1.
const LUA_PUSHBOOLEAN_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x002f_39f0;

/// Slot tag for a string.
const TAG_STRING: i32 = 4;

/// Borrowed handle on the `lua_State` a script hook was dispatched with.
///
/// Constructed once per hook entry from the raw state the host passed in;
/// every method is safe. Argument indices are 1-based, as the C API counts.
pub struct LuaState(i32);

impl LuaState {
    /// Wrap the raw state a script hook received.
    ///
    /// # Safety
    ///
    /// `l` must be the live `lua_State` the host dispatched this call with;
    /// it outlives the hook's call frame.
    pub const unsafe fn from_raw(l: i32) -> Self {
        Self(l)
    }

    const fn addr(&self) -> usize {
        self.0.cast_unsigned() as usize
    }

    /// Address of argument `index`'s 16-byte slot, or `None` past the top.
    fn slot(&self, index: i32) -> Option<usize> {
        let arg = usize::try_from(index).ok().filter(|&i| i >= 1)?;
        let base = self.stack_base();
        // SAFETY: `+0x8` of a live state is its stack top pointer.
        let top = unsafe { *((self.addr() + 0x8) as *const usize) };
        let slot = base + (arg - 1) * 0x10;
        (slot < top).then_some(slot)
    }

    fn stack_base(&self) -> usize {
        // SAFETY: `+0xc` of a live state is its stack base pointer (argument 1).
        unsafe { *((self.addr() + 0xc) as *const usize) }
    }

    /// Number of arguments on the stack, decoded without a host call.
    pub fn argc(&self) -> usize {
        // SAFETY: `+0x8` of a live state is its stack top pointer.
        let top = unsafe { *((self.addr() + 0x8) as *const usize) };
        top.wrapping_sub(self.stack_base()) >> 4
    }

    /// Bytes of the string at `index`, without coercion or allocation.
    ///
    /// `None` for anything but a tag-4 slot — a number is NOT coerced, unlike
    /// `lua_tostring`, which rewrites the slot in place. The slice borrows the
    /// interned string's buffer and is valid until the next stack mutation.
    pub fn str_arg(&self, index: i32) -> Option<&[u8]> {
        let slot = self.slot(index)?;
        // SAFETY: `slot` addresses a live stack slot; `+0x0` is its tag.
        if unsafe { *(slot as *const i32) } != TAG_STRING {
            return None;
        }
        // SAFETY: a tag-4 slot's payload at `+0x8` is the `TString*`.
        let ts = unsafe { *((slot + 0x8) as *const usize) };
        // SAFETY: a `TString`'s length lives at `+0xc`.
        let len = unsafe { *((ts + 0xc) as *const usize) };
        // SAFETY: `+0x10` is the string's byte array, `len` long (interned, so
        // it outlives this call frame unless the stack mutates).
        Some(unsafe { core::slice::from_raw_parts((ts + 0x10) as *const u8, len) })
    }

    /// The number at `index`, coercing a numeric string as the host does.
    ///
    /// `None` when the argument is absent or not convertible — the probe and
    /// the conversion both work on a copy, so the slot is never rewritten.
    pub fn number_arg(&self, index: i32) -> Option<f64> {
        // SAFETY: a fixed `.text` entry in the live host image (base
        // verified at load); the transmuted signature matches the declared
        // prototype (`__fastcall(ecx = L, edx = index)`, register ret).
        let is_number: extern "fastcall" fn(i32, i32) -> i32 =
            unsafe { core::mem::transmute(LUA_ISNUMBER_VA) };
        if is_number(self.0, index) == 0 {
            return None;
        }
        // SAFETY: a fixed `.text` entry in the live host image; the
        // transmuted signature matches the declared prototype
        // (`__fastcall(ecx = L, edx = index)`, f64 returned in `ST0`).
        let to_number: extern "fastcall" fn(i32, i32) -> f64 =
            unsafe { core::mem::transmute(LUA_TONUMBER_VA) };
        Some(to_number(self.0, index))
    }

    /// Owned copy of the string at `index`, coercing a number as the host does.
    ///
    /// Cold paths only: the coercion rewrites a number slot in place, exactly
    /// as `lua_tostring` always has. Invalid UTF-8 is recovered lossily.
    pub fn coerced_str(&self, index: i32) -> Option<String> {
        // SAFETY: a fixed `.text` entry in the live host image; the
        // transmuted signature matches the declared prototype
        // (`__fastcall(ecx = L, edx = index)`, register ret).
        let to_string: extern "fastcall" fn(i32, i32) -> *const u8 =
            unsafe { core::mem::transmute(LUA_TOSTRING_VA) };
        let raw = to_string(self.0, index);
        if raw.is_null() {
            return None;
        }
        // SAFETY: the host returned a NUL-terminated buffer it owns, valid
        // until the next stack mutation.
        let cstr = unsafe { core::ffi::CStr::from_ptr(raw.cast()) };
        Some(String::from_utf8_lossy(cstr.to_bytes()).into_owned())
    }

    /// Push `nil`.
    pub fn push_nil(&self) {
        // SAFETY: a fixed `.text` entry in the live host image; the
        // transmuted signature matches the declared prototype
        // (`__fastcall(ecx = L)`, no return).
        let push: extern "fastcall" fn(i32) = unsafe { core::mem::transmute(LUA_PUSHNIL_VA) };
        push(self.0);
    }

    /// Push a boolean.
    pub fn push_boolean(&self, value: bool) {
        // SAFETY: a fixed `.text` entry in the live host image; the
        // transmuted signature matches the declared prototype
        // (`__fastcall(ecx = L, edx = b)`, no return).
        let push: extern "fastcall" fn(i32, i32) =
            unsafe { core::mem::transmute(LUA_PUSHBOOLEAN_VA) };
        push(self.0, i32::from(value));
    }

    /// Push a number.
    pub fn push_number(&self, value: f64) {
        // SAFETY: a fixed `.text` entry in the live host image; the
        // transmuted signature matches the declared prototype
        // (`__fastcall(ecx = L)` plus the f64 on the stack, `RET 8`).
        let push: extern "fastcall" fn(i32, f64) =
            unsafe { core::mem::transmute(LUA_PUSHNUMBER_VA) };
        push(self.0, value);
    }

    /// Push a string by bytes and explicit length (interior NULs included).
    pub fn push_bytes(&self, bytes: &[u8]) {
        // SAFETY: a fixed `.text` entry in the live host image; the
        // transmuted signature matches the declared prototype
        // (`__fastcall(ecx = L, edx = s)` plus the length, `RET 4`).
        let push: extern "fastcall" fn(i32, *const u8, u32) =
            unsafe { core::mem::transmute(LUA_PUSHLSTRING_VA) };
        push(
            self.0,
            bytes.as_ptr(),
            u32::try_from(bytes.len()).expect("a pushed string is far below 4 GiB"),
        );
    }

    /// Push a string slice.
    pub fn push_str(&self, s: &str) {
        self.push_bytes(s.as_bytes());
    }
}
