//! Frame-rate caps for the foreground and backgrounded game.
//!
//! Runs at the presenting scene-end, only in world (uncapped frame rates load
//! faster, so the limiter waits for the world). The wait strategy is a timed
//! kernel delay for the bulk of the frame budget with a short spin for the
//! tail, and the next-frame target accumulates so a slow frame borrows from
//! the next rather than resetting the cadence. Both caps default off; on the
//! translated-macOS stack the display layer's own cap is the better tool for
//! the foreground, while the background cap is useful everywhere.

use core::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;

/// The client's window accessor — `fastcall(ecx = 0)`, `HWND` in `eax`.
const GET_GAME_WINDOW_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0003_5c30;

/// Foreground and background frame intervals in TSC ticks, zero = uncapped.
static TARGET_INTERVAL: AtomicU64 = AtomicU64::new(0);
/// See [`TARGET_INTERVAL`].
static BACKGROUND_INTERVAL: AtomicU64 = AtomicU64::new(0);
/// The accumulated next-frame target, TSC ticks.
static NEXT_FRAME_TIME: AtomicU64 = AtomicU64::new(0);

/// Set by the present wrapper, consumed by the scene-end wrapper.
///
/// Under a software cursor the scene ends twice per frame; only the one that
/// follows a present runs the limiter.
static IS_PRESENTING: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// The present wrapper's half of the double-scene-end dedup.
pub fn mark_presenting() {
    IS_PRESENTING.store(true, Ordering::Relaxed);
}

/// The scene-end wrapper's half: whether this scene-end follows a present.
pub fn take_presenting() -> bool {
    let presenting = IS_PRESENTING.load(Ordering::Relaxed);
    if presenting {
        IS_PRESENTING.store(false, Ordering::Relaxed);
    }
    presenting
}

unsafe extern "system" {
    fn GetModuleHandleA(module_name: *const u8) -> usize;
    fn GetProcAddress(module: usize, proc_name: *const u8) -> usize;
    fn SwitchToThread() -> i32;
}

#[link(name = "user32")]
unsafe extern "system" {
    fn GetForegroundWindow() -> usize;
}

/// The timing facts resolved once, on the first capped frame.
struct Timing {
    /// `NtDelayExecution`, zero when unavailable (pure spin then).
    delay_fn: usize,
    /// One timer-resolution period in TSC ticks.
    resolution: u64,
    /// The spin-tail budget in TSC ticks.
    spin: u64,
}

static TIMING: LazyLock<Timing> = LazyLock::new(|| {
    let hz = wow_shared::tsc::tsc_hz();
    // SAFETY: the module handle query takes a NUL-terminated literal and
    // loads nothing.
    let ntdll = unsafe { GetModuleHandleA(c"ntdll.dll".as_ptr().cast()) };
    let delay_fn = if ntdll == 0 {
        0
    } else {
        // SAFETY: the handle is live and the name is a literal; a missing
        // export answers zero, handled below.
        unsafe { GetProcAddress(ntdll, c"NtDelayExecution".as_ptr().cast()) }
    };
    // A one-millisecond timer period, when the multimedia timer service is
    // loaded to grant it; otherwise assume the coarse 64 Hz scheduler.
    // SAFETY: query only; loads nothing.
    let winmm = unsafe { GetModuleHandleA(c"winmm.dll".as_ptr().cast()) };
    let mut resolution = hz / 64;
    if winmm != 0 {
        // SAFETY: the handle is live and the name is a literal.
        let begin_period = unsafe { GetProcAddress(winmm, c"timeBeginPeriod".as_ptr().cast()) };
        if begin_period != 0 {
            // SAFETY: the export's published signature (period in
            // milliseconds, status back).
            let begin: extern "stdcall" fn(u32) -> u32 =
                unsafe { core::mem::transmute(begin_period) };
            if begin(1) == 0 {
                resolution = hz / 1000;
            }
        }
    }
    Timing {
        delay_fn,
        resolution,
        spin: hz / 250,
    }
});

fn now_ticks() -> u64 {
    // SAFETY: `rdtsc` reads the timestamp counter; no memory is touched.
    unsafe { core::arch::x86::_rdtsc() }
}

fn game_in_foreground() -> bool {
    // SAFETY: a fixed `.text` entry in the live host image (base verified
    // at load); the transmuted signature matches the declared prototype
    // (`__fastcall(ecx = index)`, handle in `eax`).
    let get_window: extern "fastcall" fn(i32) -> usize =
        unsafe { core::mem::transmute(GET_GAME_WINDOW_VA) };
    // SAFETY: plain user32 query with no arguments.
    let foreground = unsafe { GetForegroundWindow() };
    foreground == get_window(0)
}

/// Set the foreground cap from frames per second; answers the effective cap.
pub fn set_target(fps: f64) -> f64 {
    set_interval(&TARGET_INTERVAL, fps)
}

/// Set the background cap from frames per second; answers the effective cap.
pub fn set_background(fps: f64) -> f64 {
    set_interval(&BACKGROUND_INTERVAL, fps)
}

/// The current foreground cap in frames per second, zero for uncapped.
pub fn target() -> f64 {
    current(&TARGET_INTERVAL)
}

/// The current background cap in frames per second, zero for uncapped.
pub fn background() -> f64 {
    current(&BACKGROUND_INTERVAL)
}

fn set_interval(slot: &AtomicU64, fps: f64) -> f64 {
    let hz = wow_shared::tsc::tsc_hz();
    let interval = if fps < 1.0 {
        0
    } else if fps > 500.0 {
        hz / 500
    } else {
        // the command truncates its bounded positive double to an integer
        // divisor, as the reference does; the guards above leave 1.0..=500.0
        // or a NaN, and both widths truncate that range identically and
        // answer a NaN with zero, so taking the divisor 32 bits wide keeps
        // the conversion a single register convert rather than a round trip
        // through memory
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let divisor = fps as u32;
        hz / u64::from(divisor)
    };
    slot.store(interval, Ordering::Relaxed);
    current(slot)
}

fn current(slot: &AtomicU64) -> f64 {
    let interval = slot.load(Ordering::Relaxed);
    if interval == 0 {
        return 0.0;
    }
    // the answer reports a frames-per-second ratio; the tick counts are far
    // below the 52-bit exact range
    #[allow(clippy::cast_precision_loss)]
    let fps = (wow_shared::tsc::tsc_hz() / interval) as f64;
    fps
}

/// The limiter, run at the tail of the presenting scene-end while in world.
pub fn limit() {
    // Both caps default off, so the uncapped client answers on two relaxed
    // loads: the foreground query (a `user32` call plus the client's own
    // window accessor) only decides which of the two non-zero intervals
    // applies, and either selection would return here anyway.
    let foreground_interval = TARGET_INTERVAL.load(Ordering::Relaxed);
    let background_interval = BACKGROUND_INTERVAL.load(Ordering::Relaxed);
    if foreground_interval == 0 && background_interval == 0 {
        return;
    }
    let in_foreground = game_in_foreground();
    let interval = if in_foreground {
        foreground_interval
    } else {
        background_interval
    };
    if interval == 0 {
        return;
    }
    let timing = &*TIMING;
    let next = NEXT_FRAME_TIME.load(Ordering::Relaxed);
    let mut now = now_ticks();
    let mut sleep = next.wrapping_sub(now) as i64;
    if timing.resolution > timing.spin {
        sleep -= timing.resolution.cast_signed();
    }
    if sleep > timing.spin.cast_signed() && timing.delay_fn != 0 {
        sleep -= timing.spin.cast_signed();
        // Convert ticks to the kernel's 100 ns units, negative for relative.
        // Split into quotient and remainder rather than widening to 128 bits,
        // which costs a software divide: truncating division splits exactly,
        // `trunc(a*C/hz) == (a/hz)*C + trunc((a%hz)*C/hz)` for a, hz > 0, and
        // `sleep` here is positive and never more than one frame interval
        // (one second at the slowest cap the setter accepts), so `sleep/hz`
        // is a handful of seconds' worth of 100 ns units and `sleep%hz` is
        // below `hz`, so both products stay far inside `i64` at any real
        // counter frequency.
        let hz = wow_shared::tsc::tsc_hz().cast_signed();
        let mut delay = -((sleep / hz) * 10_000_000 + (sleep % hz) * 10_000_000 / hz);
        let nt_delay: extern "stdcall" fn(u8, *mut i64) -> i32 =
            // SAFETY: the export's published signature (alertable flag and a
            // relative 100 ns interval).
            unsafe { core::mem::transmute(timing.delay_fn) };
        nt_delay(0, &raw mut delay);
    }
    now = now_ticks();
    while now < next {
        if !in_foreground {
            // A backgrounded game offers its slice to whoever wants it.
            // SAFETY: plain scheduler yield with no arguments.
            unsafe { SwitchToThread() };
        }
        core::hint::spin_loop();
        now = now_ticks();
    }
    let accumulated = if now < next.wrapping_add(interval) {
        next.wrapping_add(interval)
    } else {
        now.wrapping_add(interval)
    };
    NEXT_FRAME_TIME.store(accumulated, Ordering::Relaxed);
}
