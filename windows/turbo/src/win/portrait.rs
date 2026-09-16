//! Model portraits rendered directly into native cached texture handles.

mod api;
mod client;
mod d3d;

use core::sync::atomic::{AtomicBool, Ordering};
use std::cell::{Cell, RefCell};

use super::tally::{self, Accum, Counter};
use crate::portrait::Registry;

static RELEASE_VERIFIED: AtomicBool = AtomicBool::new(false);
static ENABLED: AtomicBool = AtomicBool::new(false);
static REQUESTS: Counter = Counter::zero();
static HITS: Counter = Counter::zero();
static RENDERS: Counter = Counter::zero();
static REGENERATIONS: Counter = Counter::zero();
static FAILURES: Counter = Counter::zero();
static TICKS: Accum = Accum::zero();
static PEAK: Accum = Accum::zero();
static REPORTED_FAILURE: AtomicBool = AtomicBool::new(false);

thread_local! {
    static CACHE: RefCell<Registry> = RefCell::new(Registry::default());
    static ACTIVE_DEVICE: Cell<usize> = const { Cell::new(0) };
    static ENTERED: Cell<bool> = const { Cell::new(false) };
}

/// Inspect the release entry before the shared overlay hook can chain over a foreign detour.
pub fn prepare() {
    // SAFETY: startup has already checked the client image base; this is its release entry.
    let verified = unsafe {
        wow_hook::signature_matches(
            0x0059_9900,
            "55 8B EC 56 57 8B 7D 08 57 8B F1 E8 ?? ?? ?? ??",
        )
    };
    RELEASE_VERIFIED.store(verified, Ordering::Relaxed);
}

pub fn initialize() {
    let Some((_, own_base)) = wow_hook::module_of(initialize as *const () as usize) else {
        return;
    };
    let installed = [
        0x0052_4f60,
        0x005a_1100,
        0x005a_0850,
        0x005a_0d70,
        0x0059_9900,
    ]
    .into_iter()
    .all(|address| {
        wow_hook::detour_target(address)
            .and_then(wow_hook::detour_endpoint)
            .and_then(wow_hook::module_of)
            .is_some_and(|(_, base)| base == own_base)
    });
    let enabled = installed && RELEASE_VERIFIED.load(Ordering::Relaxed) && api::verified();
    ENABLED.store(enabled, Ordering::Relaxed);
    log::info!(target: "wow", "GPU portraits: {}", if enabled { "enabled" } else { "disabled (required hooks or helper signatures unavailable)" });
}

struct EntryGuard;
impl Drop for EntryGuard {
    fn drop(&mut self) {
        ENTERED.set(false);
    }
}

struct ActivePass;
impl ActivePass {
    fn enter(device: usize) -> Self {
        ACTIVE_DEVICE.set(device);
        Self
    }
}
impl Drop for ActivePass {
    fn drop(&mut self) {
        ACTIVE_DEVICE.set(0);
    }
}

pub fn set_portrait(ui: usize, unit: usize) {
    if !ENABLED.load(Ordering::Relaxed) || ui == 0 || unit == 0 || client::graphics().is_none() {
        (super::symbols::originals::set_portrait_texture__524f60())(ui, unit);
        return;
    }
    // SAFETY: the verified portrait entry supplies a live unit for this synchronous callback.
    let unit = unsafe { client::Unit::new(unit) };
    if ENTERED.replace(true) {
        client::defer(unit.guid);
        return;
    }
    let _entry = EntryGuard;
    let timed = tally::arm().map(|armed| (armed, wow_shared::tsc::rdtsc()));
    if let Some((armed, _)) = &timed {
        REQUESTS.bump(armed);
    }
    let outcome = render_or_reuse(ui, &unit);
    if let Some((armed, start)) = timed {
        let ticks = wow_shared::tsc::rdtsc().wrapping_sub(start);
        TICKS.add(&armed, ticks);
        PEAK.max(&armed, ticks);
        let guid = unit.guid;
        let key = unit.key_for_cache();
        crate::defer_log!(target: tally::TARGET, log::Level::Info,
            "portrait request: guid={guid:#018x} key={key:?} outcome={outcome} duration_ms={}",
            super::hooks::clock_ticks_to_ms(ticks));
    }
}

fn render_or_reuse(ui: usize, unit: &client::Unit) -> &'static str {
    if unit.pending() {
        return "pending";
    }
    let cached = unit.cached();
    let texture = cached.as_ref().map_or(0, client::Portrait::texture);
    let owned = CACHE.with_borrow(|cache| cache.contains(texture));
    if let Some(portrait) = &cached {
        let valid = CACHE.with_borrow(|cache| cache.valid(texture, &unit.key));
        if portrait.handle() != 0 && !portrait.dirty() && (valid || (!owned && !portrait.gpu())) {
            CACHE.with_borrow_mut(|cache| cache.observe(texture, unit.guid));
            client::set_texture(ui, portrait.handle());
            if let Some(armed) = tally::arm() {
                HITS.bump(&armed);
            }
            return "cache-hit";
        }
    }
    if cached.as_ref().is_some_and(client::Portrait::dirty) {
        CACHE.with_borrow_mut(|cache| cache.invalidate(texture));
    }
    if !unit.ready() {
        client::defer(unit.guid);
        return "model-not-ready";
    }
    if !unit.has_camera() {
        client::clear_texture(ui);
        return "no-portrait-camera";
    }
    let Some((gx, device)) = client::graphics() else {
        client::defer(unit.guid);
        return "device-not-ready";
    };
    let Some(target) = client::render(unit, gx, device) else {
        failed(ui, unit, cached.as_ref());
        return "render-failed";
    };
    let Some(portrait) = unit.cache_entry() else {
        failed(ui, unit, None);
        return "cache-allocation-failed";
    };
    let Some(texture) = portrait.publish(gx, target) else {
        failed(ui, unit, Some(&portrait));
        return "publication-failed";
    };
    CACHE.with_borrow_mut(|cache| cache.publish(texture, unit.key_for_cache(), unit.guid));
    client::set_texture(ui, portrait.handle());
    if let Some(armed) = tally::arm() {
        RENDERS.bump(&armed);
        if owned {
            REGENERATIONS.bump(&armed);
        }
    }
    if owned { "regenerated" } else { "rendered" }
}

fn failed(ui: usize, unit: &client::Unit, cached: Option<&client::Portrait>) {
    if let Some(armed) = tally::arm() {
        FAILURES.bump(&armed);
    }
    if !REPORTED_FAILURE.swap(true, Ordering::Relaxed) {
        crate::defer_log!(target: "wow",log::Level::Warn,"GPU portrait creation failed; retaining an existing portrait or deferring its refresh");
    }
    if let Some(portrait) = cached
        && portrait.handle() != 0
    {
        client::set_texture(ui, portrait.handle());
    }
    client::defer(unit.guid);
}

pub fn apply_viewport(gx: usize) {
    let device = ACTIVE_DEVICE.get();
    if device == 0 {
        (super::symbols::originals::c_gx_device_d3d__apply_viewport__5a1100())(gx);
        return;
    }
    // SAFETY: the verified viewport callback passes the live graphics object.
    let (near, far) = unsafe { client::depth_range(gx) };
    if d3d::viewport(device, near, far) {
        // SAFETY: the hardware accepted this live device's viewport, so its dirty flag can be cleared.
        unsafe { client::viewport_applied(gx) };
    }
}

pub fn destroy_texture(texture: usize) {
    CACHE.with_borrow_mut(|cache| cache.remove(texture));
}

pub fn update_texture(gx: usize, texture: usize) -> bool {
    if !ENABLED.load(Ordering::Relaxed) || texture == 0 {
        return false;
    }
    // SAFETY: the verified texture-update callback passes a live native texture.
    unsafe { client::update_texture(gx, texture) }
}

pub fn release_resources(gx: usize, teardown: i32) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    d3d::release_resources(teardown != 0);
    let textures = CACHE.with_borrow(Registry::textures);
    for texture in textures {
        // SAFETY: entries are removed by the native texture-destruction hook before deallocation.
        unsafe { client::release_texture(gx, texture) };
    }
    let guids = CACHE.with_borrow_mut(Registry::reset);
    for guid in guids {
        client::defer(guid);
    }
    REPORTED_FAILURE.store(false, Ordering::Relaxed);
}

pub fn emit_cumulative() {
    let requests = REQUESTS.get();
    if requests == 0 {
        return;
    }
    crate::defer_log!(target: tally::TARGET,log::Level::Info,
        "portrait: requests={} hits={} renders={} regenerations={} failures={} total_ms={} peak_ms={}",
        requests,HITS.get(),RENDERS.get(),REGENERATIONS.get(),FAILURES.get(),
        super::hooks::clock_ticks_to_ms(TICKS.get()),super::hooks::clock_ticks_to_ms(PEAK.get()));
}
