//! Hardcoded `WoW` 1.12 client function-pointer addresses.
//!
//! Each is paired with the prologue bytes it is expected to carry.
//!
//! All values are in-process VAs as observed by the running client, lifted
//! verbatim from the reference C++ implementation at
//! `wow-translate/dll/src/lua_interface.cpp:34-47`. These are 1.12-only —
//! the addon they pair with (`WoWTranslate`) is itself 1.12-only, so this
//! module is gated `cfg(target_arch = "x86")` at the parent level.
//!
//! The signatures are entry-anchored IDA-style byte patterns matched against
//! each VA before the `UnitXP` patch goes in (see `hook::install`); `??`
//! wildcards the address-bearing bytes of a relative branch. A VA that carries
//! neither its captured prologue nor an inline detour is not the function these
//! addresses were captured from — patching or calling through it corrupts the
//! host, so that disables the DLL. An already-detoured prologue is expected and
//! chained onto, not refused: `UnitXP` is bound by several 1.12 mods.

pub const P_UNIT_XP: usize = 0x0051_7350;
/// `PUSH EBP; MOV EBP,ESP; SUB ESP,0x8; PUSH ESI; MOV ESI,ECX; MOV EDX,0x1`.
pub const UNIT_XP_SIG: &str = "55 8B EC 83 EC 08 56 8B F1 BA 01 00 00 00";

pub const P_LUA_GETTOP: usize = 0x006F_3070;
/// The whole function: `MOV EAX,[ECX+8]; SUB EAX,[ECX+0xc]; SAR EAX,4; RET`.
pub const LUA_GETTOP_SIG: &str = "8B 41 08 2B 41 0C C1 F8 04 C3";

pub const P_LUA_TOSTRING: usize = 0x006F_3690;
/// `PUSH ESI; PUSH EDI; MOV EDI,ECX; CALL rel32; MOV ESI,EAX; TEST ESI,ESI`.
pub const LUA_TOSTRING_SIG: &str = "56 57 8B F9 E8 ?? ?? ?? ?? 8B F0 85 F6";

pub const P_LUA_PUSHSTRING: usize = 0x006F_3890;
/// `TEST EDX,EDX; PUSH ESI; MOV ESI,ECX; JNZ +6; POP ESI; JMP rel32`.
pub const LUA_PUSHSTRING_SIG: &str = "85 D2 56 8B F1 75 06 5E E9 ?? ?? ?? ??";
