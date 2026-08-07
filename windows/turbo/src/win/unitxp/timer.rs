//! Script timers: arm a Lua chunk to run after a delay, optionally repeating.
//!
//! Threadless where the original used a worker thread: arm, disarm and the
//! per-frame pump all run on the game thread, so a sorted schedule plus a
//! live-timer map is the whole machine. Due chunks are collected under the
//! lock and executed after it is released, so a chunk that disarms or re-arms
//! timers (including itself) cannot deadlock; a repeating timer reschedules
//! before its chunk runs, and one that fell behind fires once per pump.
//!
//! Each chunk runs through the client's own script runner, bracketed by the
//! execution-state save/restore the taint machinery expects, with the state
//! captured when the timer was armed.

use std::{
    collections::BinaryHeap,
    sync::{LazyLock, Mutex},
};

use rustc_hash::FxHashMap;

/// The script runner — `fastcall(ecx = L)`, reads the chunk from stack slot 1.
const RUN_SCRIPT_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0008_b980;
/// The script context accessor — no arguments, the state in `eax`.
const GET_CONTEXT_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0030_40d0;
/// `lua_insert` — `fastcall(ecx = L, edx = index)`.
const LUA_INSERT_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x002f_31a0;
/// `lua_remove` — `fastcall(ecx = L, edx = index)`.
const LUA_REMOVE_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x002f_30d0;
/// The taint execution-state word and its nesting counter.
const EXEC_STATE: usize = crate::win::EXPECTED_IMAGE_BASE + 0x008e_eac0;
/// See [`EXEC_STATE`].
const EXEC_COUNTER: usize = crate::win::EXPECTED_IMAGE_BASE + 0x008e_eac4;

/// One armed timer.
struct Entry {
    script: String,
    period_ms: u64,
    /// The taint execution state captured at arm time.
    exec_state: u32,
}

/// The schedule and the live map, both keyed by the timer id.
struct Timers {
    /// Min-heap of (due engine-ms, id).
    schedule: BinaryHeap<core::cmp::Reverse<(u64, u32)>>,
    live: FxHashMap<u32, Entry>,
    next_id: u32,
}

static TIMERS: LazyLock<Mutex<Timers>> = LazyLock::new(|| {
    Mutex::new(Timers {
        schedule: BinaryHeap::new(),
        live: FxHashMap::default(),
        next_id: 1,
    })
});

fn now_ms() -> u64 {
    u64::from(super::super::objmgr::game_tick_ms())
}

/// Arm a timer; the id (never zero) identifies it to `disarm`.
pub fn arm(delay_ms: u64, period_ms: u64, script: &str) -> u32 {
    if script.is_empty() {
        return 0;
    }
    // SAFETY: `EXEC_STATE` is the host's execution-state word at the verified
    // image base.
    let exec_state = unsafe { *(EXEC_STATE as *const u32) };
    let mut timers = TIMERS.lock().expect("timer state is game-thread only");
    let id = timers.next_id;
    timers.next_id = timers.next_id.wrapping_add(1).max(1);
    timers.live.insert(
        id,
        Entry {
            script: String::from(script),
            period_ms,
            exec_state,
        },
    );
    timers
        .schedule
        .push(core::cmp::Reverse((now_ms().wrapping_add(delay_ms), id)));
    id
}

/// Disarm a timer; whether it was live.
pub fn disarm(id: u32) -> bool {
    let mut timers = TIMERS.lock().expect("timer state is game-thread only");
    timers.live.remove(&id).is_some()
}

/// The number of live timers.
pub fn live_count() -> usize {
    let timers = TIMERS.lock().expect("timer state is game-thread only");
    timers.live.len()
}

/// The per-frame pump, run from the world-render wrapper.
pub fn pump() {
    let now = now_ms();
    let mut due: Vec<(String, u32)> = Vec::new();
    {
        let mut timers = TIMERS.lock().expect("timer state is game-thread only");
        while let Some(&core::cmp::Reverse((when, id))) = timers.schedule.peek() {
            if when > now {
                break;
            }
            timers.schedule.pop();
            let Some(entry) = timers.live.get(&id) else {
                // Disarmed; the schedule entry was its last trace.
                continue;
            };
            due.push((entry.script.clone(), entry.exec_state));
            if entry.period_ms > 0 {
                // Reschedule before running; a timer that fell behind skips
                // ahead so it fires once per pump, not once per missed period.
                let mut next = when.wrapping_add(entry.period_ms);
                if next <= now {
                    next = now.wrapping_add(entry.period_ms);
                }
                timers.schedule.push(core::cmp::Reverse((next, id)));
            } else {
                timers.live.remove(&id);
            }
        }
        drop(timers);
    }
    for (script, exec_state) in due {
        run_script(&script, exec_state);
    }
}

/// Run a chunk through the client's script runner under a taint bracket.
///
/// The runner reads its chunk from stack slot 1, and the stack is not
/// guaranteed clean here, so the chunk is pushed and rotated into slot 1,
/// then removed after the run — the original's exact protocol.
fn run_script(script: &str, exec_state: u32) {
    // SAFETY: a fixed `.text` entry in the live host image (base verified
    // at load); the transmuted signature matches the declared prototype
    // (no arguments, the state in `eax`).
    let get_context: extern "fastcall" fn() -> i32 =
        unsafe { core::mem::transmute(GET_CONTEXT_VA) };
    let l = get_context();
    if l == 0 {
        return;
    }
    // SAFETY: `l` is the live script context the client just answered with.
    let lua = unsafe { super::super::lua::LuaState::from_raw(l) };
    // The execution-state bracket: save, count in, adopt the armed state.
    // SAFETY: `EXEC_STATE` is the host's execution-state word.
    let saved = unsafe { *(EXEC_STATE as *const u32) };
    // SAFETY: `EXEC_COUNTER` is its nesting counter.
    let counter = unsafe { *(EXEC_COUNTER as *const i32) }.wrapping_add(1);
    // SAFETY: writing the counter back, as the bracket helper does.
    unsafe { *(EXEC_COUNTER as *mut i32) = counter };
    if counter != 0 {
        // SAFETY: adopting the armed execution state under a live bracket.
        unsafe { *(EXEC_STATE as *mut u32) = exec_state };
    }
    lua.push_str(script);
    // SAFETY: a fixed `.text` entry in the live host image; the transmuted
    // signature matches the declared prototype
    // (`__fastcall(ecx = L, edx = index)`, no return).
    let insert: extern "fastcall" fn(i32, i32) = unsafe { core::mem::transmute(LUA_INSERT_VA) };
    insert(l, 1);
    // SAFETY: a fixed `.text` entry in the live host image; the transmuted
    // signature matches the declared prototype (`__fastcall(ecx = L)`).
    let run: extern "fastcall" fn(i32) -> i32 = unsafe { core::mem::transmute(RUN_SCRIPT_VA) };
    run(l);
    // SAFETY: a fixed `.text` entry in the live host image; the transmuted
    // signature matches the declared prototype
    // (`__fastcall(ecx = L, edx = index)`, no return).
    let remove: extern "fastcall" fn(i32, i32) = unsafe { core::mem::transmute(LUA_REMOVE_VA) };
    remove(l, 1);
    // Close the bracket: restore under the counter, count out, floor at zero.
    // SAFETY: `EXEC_COUNTER` as above.
    let counter = unsafe { *(EXEC_COUNTER as *const i32) };
    if counter != 0 {
        // SAFETY: restoring the saved execution state.
        unsafe { *(EXEC_STATE as *mut u32) = saved };
    }
    // SAFETY: counting the bracket out, floored as the helper floors it.
    unsafe { *(EXEC_COUNTER as *mut i32) = counter.wrapping_sub(1).max(0) };
}
