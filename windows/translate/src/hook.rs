//! `UnitXP` detour and subcommand dispatch.
//!
//! Hooks `WoW` 1.12's `UnitXP` Lua C function at the fixed VA
//! [`unit_xp_addrs::P_UNIT_XP`] via `MinHook`. Calls whose first stack arg
//! is `"WoWTranslate"` route to the local dispatcher; everything else falls
//! through to the original `UnitXP` (the pointer `MinHook` hands back).
//!
//! Only the three subcommands the live addon issues are reimplemented (per
//! a grep of the addon's API module): `ping`, `translate_async`, `poll`.
//! Unknown subcommands log once and return `error|unknown subcommand`, which
//! is what the C++ implementation this replaces did.

use core::ffi::c_void;
use std::sync::OnceLock;

use crate::{
    LOG_TARGET,
    lua_ffi::LuaState,
    queue,
    unit_xp_addrs::{
        LUA_GETTOP_SIG, LUA_PUSHSTRING_SIG, LUA_TOSTRING_SIG, P_LUA_GETTOP, P_LUA_PUSHSTRING,
        P_LUA_TOSTRING, P_UNIT_XP, UNIT_XP_SIG,
    },
};

type UnitXpFn = extern "fastcall" fn(*mut c_void) -> i32;

const UNIT_XP_LABEL: &str = "UnitXP";

/// Every hardcoded VA this DLL touches, with the prologue it was captured from.
///
/// `UnitXP` is the one that gets patched; the three `lua_*` entries are only
/// ever called, but they run on *every* pass-through, so a wrong client build
/// would crash through them even with a verified patch target. All four are
/// checked before anything is installed.
const VERIFIED_SITES: [(usize, &str, &str); 4] = [
    (P_UNIT_XP, UNIT_XP_SIG, UNIT_XP_LABEL),
    (P_LUA_GETTOP, LUA_GETTOP_SIG, "lua_gettop"),
    (P_LUA_TOSTRING, LUA_TOSTRING_SIG, "lua_tostring"),
    (P_LUA_PUSHSTRING, LUA_PUSHSTRING_SIG, "lua_pushstring"),
];

/// First bytes of an inline detour.
///
/// `jmp rel32` (what `MinHook` itself writes, and every hand-rolled 1.12 hook) and
/// `jmp [addr]`. `jmp rel8` is absent on purpose — it cannot reach a detour, so
/// accepting it would only widen the window for mistaking a wrong client build for
/// a hooked one.
const DETOUR_PROLOGUES: [&str; 2] = ["E9", "FF 25"];

/// Pointer to the unhooked `UnitXP`, returned by `MinHook::create_hook` as a trampoline.
///
/// Runtime-set in [`install`], hence `OnceLock` over `LazyLock` per the convention.
static ORIGINAL_UNIT_XP: OnceLock<UnitXpFn> = OnceLock::new();

pub fn install() {
    if !sites_resolve() {
        return;
    }

    let detour = detoured_unit_xp as *mut c_void;

    // SAFETY: `P_UNIT_XP` is the fixed VA of WoW 1.12's `UnitXP` — its prologue
    // just resolved — whose `fastcall(*mut c_void) -> i32` ABI matches
    // `detoured_unit_xp`. If another mod detoured it first, `MinHook` relocates
    // the displaced jump into our trampoline, so the pass-through lands in that
    // mod's detour and the chain is preserved.
    let Some(trampoline) = (unsafe { wow_hook::create_hook(P_UNIT_XP, detour, UNIT_XP_LABEL) })
    else {
        return;
    };

    // SAFETY: the trampoline is a callable thunk with the same calling
    // convention as the original `UnitXP`.
    let original: UnitXpFn = unsafe { core::mem::transmute::<*mut c_void, UnitXpFn>(trampoline) };
    // Publish the trampoline before enabling so a foreign `UnitXP` call landing
    // the instant the patch goes live always has an original to fall through to.
    let _ = ORIGINAL_UNIT_XP.set(original);

    // SAFETY: `P_UNIT_XP` is the hook just created above.
    let _ = unsafe { wow_hook::enable_hook(P_UNIT_XP, UNIT_XP_LABEL) };
}

/// Whether all four hardcoded VAs still hold the functions their addresses were captured from.
///
/// A site resolves two ways: its captured prologue is intact, or the prologue is
/// an inline detour, meaning another mod hooked the function ahead of us. The
/// second is not a failure. `UnitXP` is the channel 1.12 mods borrow to expose
/// native calls to Lua — several bind it, and we must stack on top rather than
/// bow out, or whichever mod loads first silently wins. Chaining is sound in
/// both directions: hooking a detoured prologue relocates the displaced jump
/// into our trampoline, and a mod that hooks us later inherits our detour as
/// its own original.
///
/// What remains worth refusing is a prologue that is neither — a stale VA on a
/// client build these addresses were never captured from. Patching that corrupts
/// the host, and calling through the `lua_*` entries would crash on the first
/// pass-through. Refusing degrades cleanly instead: with no hook, `ping` reaches
/// the real `UnitXP`, the addon sets `dllAvailable = false` and shows originals.
fn sites_resolve() -> bool {
    let mut all_resolve = true;
    for &(va, sig, label) in &VERIFIED_SITES {
        // SAFETY: each VA lies in the 1.12 client's mapped code section;
        // `signature_matches` reads at most the signature's token count of bytes.
        if unsafe { wow_hook::signature_matches(va, sig) } {
            continue;
        }
        if is_detoured(va) {
            log::info!(
                target: LOG_TARGET,
                "{label} @ {va:#010x} already detoured by another mod — chaining through it",
            );
            continue;
        }
        log::warn!(
            target: LOG_TARGET,
            "{label} prologue unrecognized at {va:#010x} — wrong client build; refusing to patch",
        );
        all_resolve = false;
    }
    all_resolve
}

/// Whether the function at `va` begins with an unconditional jump.
///
/// Someone already patched its entry.
fn is_detoured(va: usize) -> bool {
    DETOUR_PROLOGUES.iter().any(|sig| {
        // SAFETY: `va` lies in the 1.12 client's mapped code section;
        // `signature_matches` reads at most the signature's token count of bytes.
        unsafe { wow_hook::signature_matches(va, sig) }
    })
}

extern "fastcall" fn detoured_unit_xp(state: *mut c_void) -> i32 {
    if state.is_null() {
        return 0;
    }
    // SAFETY: WoW's hook entry guarantees `state` is the live Lua-state
    // pointer for the duration of this call.
    let lua = unsafe { LuaState::from_raw(state) };

    dispatch_wow_translate(&lua).unwrap_or_else(|| call_original(state))
}

fn call_original(state: *mut c_void) -> i32 {
    ORIGINAL_UNIT_XP.get().map_or_else(
        || {
            // Hook fired before `ORIGINAL_UNIT_XP` was populated. `install`
            // sets it between `create_hook` and `enable_hook`, so the patch
            // isn't live until it holds — but log defensively and return zero
            // results rather than pass the call nowhere.
            wow_shared::log_once_warn!(
                target: LOG_TARGET,
                "UnitXP detour fired before original captured",
            );
            0
        },
        |original| original(state),
    )
}

fn dispatch_wow_translate(lua: &LuaState) -> Option<i32> {
    if lua.gettop() < 1 {
        return None;
    }
    // Every `UnitXP` call in the process reaches this compare — pfUI's per-frame
    // distance checks among them — so it stays allocation-free; a foreign
    // command is rejected on its first byte.
    if !lua.string_at_eq(1, c"WoWTranslate") {
        return None;
    }
    Some(if lua.string_at_eq(2, c"ping") {
        handle_ping(lua)
    } else if lua.string_at_eq(2, c"translate_async") {
        handle_translate_async(lua)
    } else if lua.string_at_eq(2, c"poll") {
        handle_poll(lua)
    } else {
        // Cold: allocate only to name the offender.
        let other = lua.tostring(2).unwrap_or_default();
        wow_shared::log_once_warn!(
            target: LOG_TARGET,
            "unknown subcommand: {other}",
        );
        lua.pushstring("error|unknown subcommand");
        1
    })
}

fn handle_ping(lua: &LuaState) -> i32 {
    lua.pushstring("pong");
    1
}

fn handle_translate_async(lua: &LuaState) -> i32 {
    // Args: 1=cmd, 2=subcmd, 3=requestId, 4=text, [5=srcLang, 6=tgtLang]
    if lua.gettop() < 4 {
        lua.pushstring("error|requestId and text required");
        return 1;
    }
    let Some(req_id) = lua.tostring(3) else {
        lua.pushstring("error|invalid requestId");
        return 1;
    };
    let Some(text) = lua.tostring(4) else {
        lua.pushstring("error|invalid text");
        return 1;
    };
    if text.is_empty() {
        lua.pushstring("error|empty text");
        return 1;
    }

    // Default zh-Hans → en for backward compat with the original DLL.
    let src_lang = lua.tostring(5).unwrap_or_else(|| "zh-Hans".to_owned());
    let tgt_lang = lua.tostring(6).unwrap_or_else(|| "en".to_owned());

    lua.pushstring(
        match queue::enqueue(queue::Request {
            req_id,
            text,
            src_lang,
            tgt_lang,
        }) {
            Ok(()) => "ok",
            Err(queue::EnqueueError::Full) => "error|queue full",
            Err(queue::EnqueueError::Closed) => "error|queue closed",
        },
    );
    1
}

/// Maximum completed results returned in one `poll`.
///
/// Far above the addon's in-flight cap; just a guard on the response string size.
const POLL_MAX_RECORDS: usize = 64;
/// Field separator within a record (ASCII Unit Separator).
///
/// Cannot occur in chat text or Apple-translation output, so — unlike `|` — it
/// never collides with translated content. Must match the Lua parser.
const FIELD_SEP: char = '\u{1f}';
/// Record separator between results (ASCII Record Separator).
const RECORD_SEP: char = '\u{1e}';

fn handle_poll(lua: &LuaState) -> i32 {
    let batch = queue::drain_completed(POLL_MAX_RECORDS);
    if batch.is_empty() {
        lua.pushstring("");
        return 1;
    }
    // `{reqId}\x1F{translation}\x1F{error}` per record, records joined by \x1E.
    let mut s = String::new();
    for (i, c) in batch.iter().enumerate() {
        if i > 0 {
            s.push(RECORD_SEP);
        }
        s.push_str(&c.req_id);
        s.push(FIELD_SEP);
        s.push_str(&c.translation);
        s.push(FIELD_SEP);
        s.push_str(&c.error);
    }
    lua.pushstring(&s);
    1
}
