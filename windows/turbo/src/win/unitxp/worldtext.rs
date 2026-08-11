//! The combat-text overlay: screen-space text drawn at the presenting scene-end.
//!
//! Replaces the reference overlay's renderer without its runtime baggage: no
//! `d3dx9` load, no GDI fonts, no lost/reset choreography. Each line is
//! rasterized once from one of the client's own `Fonts/` faces
//! ([`crate::typeset`]), uploaded into a managed-pool texture — which
//! survives a device reset by definition — and drawn as two alpha-blended
//! quads (shadow, fill) through the backend device the client already
//! created. Animation curves live in the portable kernel
//! ([`crate::worldtext`]); this module supplies anchors (world-to-screen
//! projection plus the nameplate perspective offset), owns the textures, and
//! restores every device state it touches so the client's render-state
//! shadow cache stays coherent.
//!
//! A line the chosen faces cannot cover is handed back to the stock
//! renderer instead of being drawn with missing glyphs — the reference
//! swallowed those; delegating per line is strictly better and costs one
//! coverage scan at creation.

use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

use crate::{
    win::tally::{self, Counter},
    worldtext as kernel,
};

/// The world-frame pointer slot, the projection entry's `this`.
const WORLD_FRAME_PTR: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0074_b2bc;
/// `WorldFrame::WorldToScreen` — `thiscall(frame, in xyz, out xyz)`, hit in `al`.
const WORLD_TO_SCREEN_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0008_3ee0;
/// The DDC-to-NDC helper — `fastcall(ecx = x out, edx = y out)` + two floats.
const DDC_TO_NDC_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0001_ade0;
/// The `CVar` registry lookup — `fastcall(ecx = name)`, record in `eax`.
const GET_CVAR_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0023_dec0;
/// The backend `IDirect3DDevice9*` slot inside the graphics device object.
const GX_BACKEND_SLOT: usize = 0x38a8;
/// The graphics device's "backend ready" dword the reference gates drawing on.
const GX_READY_SLOT: usize = 0x3a38;
/// The graphics device global holding the live `CGxDevice*`.
const GX_DEVICE_PTR: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0080_ed38;

/// Reference layout constant: the float travel span in 768-line UI units.
const FLOAT_TRAVEL_UI: f64 = 609.0 - 384.0;
/// Reference layout constant: the arc radius in 768-line UI units.
const ARC_RADIUS_UI: f64 = 150.0;
/// Reference shadow offset in pixels.
const SHADOW_WEIGHT: i32 = 2;
/// Shadow alpha at full opacity, the reference's `200`.
const SHADOW_ALPHA: f64 = 200.0;

/// Soft cap on live floating lines; the oldest drops first past it.
///
/// The reference has no cap and a 40-man burst can outrun the 1.9 s decay;
/// each line owns a texture, so the cap bounds texture churn too.
const MAX_FLOATS: usize = 128;

/// Pixel size deltas the reference applies per face role.
const BIG_PX_DELTA: i32 = 15;
/// See [`BIG_PX_DELTA`].
const SMALL_PX_DELTA: i32 = -4;

/// Whether scripts asked for the replacement combat text.
static USE_COMBAT_TEXT: AtomicBool = AtomicBool::new(false);
/// The configured font size, stored and echoed for the settings round-trip.
static FONT_SIZE: AtomicI32 = AtomicI32::new(40);
/// The configured text height above nameplates, f64 bits.
static NAMEPLATE_HEIGHT: AtomicU64 = AtomicU64::new(f64::to_bits(55.0));
/// The configured font name.
static FONT_NAME: LazyLock<Mutex<String>> = LazyLock::new(|| Mutex::new(String::new()));
/// Alternates the arc sweep side, the reference's instance-count parity.
static ARC_PARITY: AtomicU32 = AtomicU32::new(0);
/// Alternates the crit carousel side where the reference rolled a die.
static COIN_PARITY: AtomicU32 = AtomicU32::new(0);

/// Lines drawn through the overlay (armed runs).
static FLOATS_SHOWN: Counter = Counter::zero();
/// See [`FLOATS_SHOWN`].
static CRITS_SHOWN: Counter = Counter::zero();
/// Lines delegated to stock because no face covers them.
static DELEGATED_UNCOVERED: Counter = Counter::zero();
/// Lines dropped because the renderer was not ready (no device, bad rect).
static DELEGATED_UNREADY: Counter = Counter::zero();
/// Texture creation or upload failures.
static TEXTURE_FAILURES: Counter = Counter::zero();
/// Textures deliberately leaked because the device changed under them.
static LEAKED_ON_DEVICE_SWAP: Counter = Counter::zero();
/// Oldest lines dropped by the soft cap.
static CAPPED: Counter = Counter::zero();

/// See [`USE_COMBAT_TEXT`].
pub fn use_combat_text() -> bool {
    USE_COMBAT_TEXT.load(Ordering::Relaxed)
}

/// See [`USE_COMBAT_TEXT`].
pub fn set_use_combat_text(value: bool) {
    USE_COMBAT_TEXT.store(value, Ordering::Relaxed);
}

/// Set the font size, clamped to `[10, 100]`; answers the stored value.
pub fn set_font_size(value: f64) -> i32 {
    let clamped = value.clamp(10.0, 100.0);
    // the command truncates its clamped double to an integer size, as the
    // reference does
    // clamped to [10, 100] above
    let size = clamped as i32;
    FONT_SIZE.store(size, Ordering::Relaxed);
    log::info!(target: crate::win::LOG_TARGET, "worldtext: font size set to {size}");
    size
}

/// The configured base pixel size.
fn font_size() -> i32 {
    FONT_SIZE.load(Ordering::Relaxed)
}

/// Set the nameplate text height, clamped to `[0, 256]`; answers it.
pub fn set_nameplate_height(value: f64) -> f64 {
    let clamped = value.clamp(0.0, 256.0);
    NAMEPLATE_HEIGHT.store(clamped.to_bits(), Ordering::Relaxed);
    clamped
}

/// The configured nameplate text height.
fn nameplate_height() -> f64 {
    f64::from_bits(NAMEPLATE_HEIGHT.load(Ordering::Relaxed))
}

/// Store the font name; answers a copy for the echo.
///
/// The resolution is probed and logged immediately so a name that matches
/// nothing is visible at the moment it was typed, not at the next crit.
pub fn set_font_name(name: String) -> String {
    let echo = {
        let mut slot = FONT_NAME.lock().expect("font name is game-thread only");
        *slot = name;
        slot.clone()
    };
    if let Ok(faces) = &*FACES {
        let stem = normalize_stem(&echo);
        if faces.resolve(&stem).is_some() {
            log::info!(target: crate::win::LOG_TARGET, "worldtext: font name {echo:?} resolved to {stem}");
        } else {
            log::info!(
                target: crate::win::LOG_TARGET,
                "worldtext: font name {echo:?} matches no face; lines fall back to {}",
                FACE_ORDER[0]
            );
        }
    }
    echo
}

// ── Faces ──

/// The face lookup order when no user selection matches.
///
/// The client's stock face first — the one its own initialization registers
/// and the world text draws with — then the other shipped faces.
const FACE_ORDER: [&str; 4] = ["FRIZQT__", "SKURRI", "ARIALN", "MORPHEUS"];

/// Every named face the overlay can draw with.
///
/// The client's own `Fonts/` faces load eagerly. A selected name matching
/// none of them probes the system font directories on demand — the same
/// universe the reference's GDI selection drew from, minus its registry
/// family mapping: matching is by file stem, so the name to type is the
/// font's file name (extension optional), not its family name.
struct Faces {
    faces: Vec<(String, &'static crate::typeset::Face)>,
    /// System faces probed by stem, hits and misses both.
    ///
    /// Misses are cached so a name that resolves nowhere costs one
    /// directory scan, not one per line.
    system: Mutex<Vec<(String, Option<&'static crate::typeset::Face>)>>,
}

impl Faces {
    /// The client face registered under `stem` (already upper-cased), if any.
    fn by_stem(&self, stem: &str) -> Option<&'static crate::typeset::Face> {
        self.faces
            .iter()
            .find(|(name, _)| name == stem)
            .map(|(_, face)| *face)
    }

    /// The comma-joined client face roster, for the log and debug query.
    fn roster(&self) -> String {
        self.faces
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// A system face by upper stem, probing the font directories once.
    fn system_by_stem(&self, stem: &str) -> Option<&'static crate::typeset::Face> {
        if stem.is_empty() {
            return None;
        }
        let mut cache = self.system.lock().expect("faces are game-thread only");
        if let Some((_, cached)) = cache.iter().find(|(name, _)| name == stem) {
            return *cached;
        }
        let loaded = load_system_face(stem);
        match loaded {
            Some(_) => log::info!(
                target: crate::win::LOG_TARGET,
                "worldtext: system face {stem} loaded"
            ),
            None => log::info!(
                target: crate::win::LOG_TARGET,
                "worldtext: no system face matches {stem}; falling back to the client faces"
            ),
        }
        cache.push((String::from(stem), loaded));
        loaded
    }

    /// The face `stem` names, client faces first, then the system probe.
    fn resolve(&self, stem: &str) -> Option<&'static crate::typeset::Face> {
        self.by_stem(stem).or_else(|| self.system_by_stem(stem))
    }

    /// The first face covering `text`, and the name it answers to.
    ///
    /// The user selection (client or system), then [`FACE_ORDER`], then
    /// anything else the client directory held.
    fn pick(&self, text: &str) -> Option<(&'static crate::typeset::Face, String)> {
        let selected = selected_stem();
        let named = core::iter::once(selected.as_str())
            .chain(FACE_ORDER)
            .filter_map(|stem| self.resolve(stem).map(|face| (face, String::from(stem))));
        let rest = self.faces.iter().map(|(name, face)| (*face, name.clone()));
        named.chain(rest).find(|(face, _)| face.covers(text))
    }
}

/// The selected font name normalized to an upper-cased file stem.
fn selected_stem() -> String {
    let name = FONT_NAME.lock().expect("font name is game-thread only");
    normalize_stem(&name)
}

/// Trim a typed font name to the upper-cased stem the registries key on.
fn normalize_stem(name: &str) -> String {
    let trimmed = name.trim();
    let stem = std::path::Path::new(trimmed)
        .file_stem()
        .map_or(trimmed, |s| s.to_str().unwrap_or(trimmed));
    stem.to_uppercase()
}

/// The directories a system face may live in.
///
/// The prefix's own fonts first; under Wine the host filesystem is mapped
/// as the `Z:` drive, which reaches the host's face collection. On a real
/// Windows install the `Z:` paths simply do not exist and the probe falls
/// through.
fn system_font_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(windir) = std::env::var("WINDIR") {
        dirs.push(std::path::PathBuf::from(windir).join("Fonts"));
    }
    // Wine imports the unix environment, so the host home directory reaches
    // the per-user font collection through the drive mapping too.
    if let Ok(home) = std::env::var("HOME") {
        let host_home = format!("Z:{}", home.replace('/', "\\"));
        dirs.push(std::path::PathBuf::from(host_home).join(r"Library\Fonts"));
    }
    dirs.push(std::path::PathBuf::from(
        r"Z:\System\Library\Fonts\Supplemental",
    ));
    dirs.push(std::path::PathBuf::from(r"Z:\System\Library\Fonts"));
    dirs.push(std::path::PathBuf::from(r"Z:\Library\Fonts"));
    dirs
}

/// Load the first system font file whose stem matches, leaked like the rest.
fn load_system_face(stem: &str) -> Option<&'static crate::typeset::Face> {
    for dir in system_font_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_font = path.extension().is_some_and(|e| {
                e.eq_ignore_ascii_case("ttf")
                    || e.eq_ignore_ascii_case("ttc")
                    || e.eq_ignore_ascii_case("otf")
            });
            if !is_font {
                continue;
            }
            let matches = path
                .file_stem()
                .is_some_and(|s| s.to_string_lossy().to_uppercase() == stem);
            if !matches {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            if let Some(face) = crate::typeset::Face::from_bytes(bytes) {
                return Some(&*Box::leak(Box::new(face)));
            }
            log::warn!(
                target: crate::win::LOG_TARGET,
                "worldtext: {} matched but did not parse as a font",
                path.display()
            );
        }
    }
    None
}

/// The face registry, or the reason the overlay is disabled.
///
/// Loaded once on first use; the faces are leaked because lines reference
/// them for the process lifetime. A missing or fontless directory disables
/// the overlay with a reason the debug query serves, exactly the failure
/// ladder scripts already handle.
static FACES: LazyLock<Result<Faces, String>> = LazyLock::new(load_faces);

/// See [`FACES`].
fn load_faces() -> Result<Faces, String> {
    let exe = std::env::current_exe()
        .map_err(|_| String::from("the client executable path could not be resolved"))?;
    let dir = exe
        .parent()
        .ok_or_else(|| String::from("the client executable has no parent directory"))?
        .join("Fonts");
    let entries = std::fs::read_dir(&dir)
        .map_err(|_| String::from("the client Fonts directory could not be opened"))?;
    let mut faces = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("ttf"))
        {
            continue;
        }
        let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().to_uppercase()) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if let Some(face) = crate::typeset::Face::from_bytes(bytes) {
            faces.push((stem, &*Box::leak(Box::new(face))));
        } else {
            log::warn!(
                target: crate::win::LOG_TARGET,
                "worldtext: {} did not parse as a font",
                path.display()
            );
        }
    }
    if faces.is_empty() {
        return Err(String::from(
            "no usable font was found in the client Fonts directory",
        ));
    }
    let registry = Faces {
        faces,
        system: Mutex::new(Vec::new()),
    };
    log::info!(
        target: crate::win::LOG_TARGET,
        "worldtext: client faces loaded: {}; any system font resolves by file name on demand",
        registry.roster()
    );
    Ok(registry)
}

/// Whether the overlay can render: the face registry loaded.
pub fn scene_enabled() -> bool {
    FACES.is_ok()
}

/// The disable reason, empty while the overlay is able to render.
pub fn disable_reason() -> String {
    match &*FACES {
        Ok(_) => String::new(),
        Err(reason) => reason.clone(),
    }
}

// ── Backend device access ──

/// `IDirect3DDevice9` vtable byte offsets (slot index × 4).
const VT_CREATE_TEXTURE: usize = 0x5c;
/// See [`VT_CREATE_TEXTURE`].
const VT_GET_RENDER_STATE: usize = 0xe8;
/// See [`VT_CREATE_TEXTURE`].
const VT_SET_RENDER_STATE: usize = 0xe4;
/// See [`VT_CREATE_TEXTURE`].
const VT_GET_TEXTURE: usize = 0x100;
/// See [`VT_CREATE_TEXTURE`].
const VT_SET_TEXTURE: usize = 0x104;
/// See [`VT_CREATE_TEXTURE`].
const VT_GET_TEXTURE_STAGE_STATE: usize = 0x108;
/// See [`VT_CREATE_TEXTURE`].
const VT_SET_TEXTURE_STAGE_STATE: usize = 0x10c;
/// See [`VT_CREATE_TEXTURE`].
const VT_GET_SAMPLER_STATE: usize = 0x110;
/// See [`VT_CREATE_TEXTURE`].
const VT_SET_SAMPLER_STATE: usize = 0x114;
/// See [`VT_CREATE_TEXTURE`].
const VT_DRAW_PRIMITIVE_UP: usize = 0x14c;
/// See [`VT_CREATE_TEXTURE`].
const VT_GET_FVF: usize = 0x168;
/// See [`VT_CREATE_TEXTURE`].
const VT_SET_FVF: usize = 0x164;
/// See [`VT_CREATE_TEXTURE`].
const VT_GET_VERTEX_SHADER: usize = 0x174;
/// See [`VT_CREATE_TEXTURE`].
const VT_SET_VERTEX_SHADER: usize = 0x170;
/// See [`VT_CREATE_TEXTURE`].
const VT_GET_PIXEL_SHADER: usize = 0x1b0;
/// See [`VT_CREATE_TEXTURE`].
const VT_SET_PIXEL_SHADER: usize = 0x1ac;
/// `IUnknown::Release`, shared by every COM object touched here.
const VT_RELEASE: usize = 0x8;
/// `IDirect3DTexture9::LockRect` / `UnlockRect`.
const VT_TEX_LOCK_RECT: usize = 0x4c;
/// See [`VT_TEX_LOCK_RECT`].
const VT_TEX_UNLOCK_RECT: usize = 0x50;

/// `D3DFMT_A8R8G8B8`.
const FMT_A8R8G8B8: u32 = 21;
/// `D3DPOOL_MANAGED`: survives a device reset, the lifecycle win.
const POOL_MANAGED: u32 = 1;
/// `D3DPT_TRIANGLESTRIP`.
const PT_TRIANGLESTRIP: u32 = 5;
/// `D3DFVF_XYZRHW | D3DFVF_DIFFUSE | D3DFVF_TEX1`.
const FVF_QUAD: u32 = 0x144;

/// The render states the draw pass touches, as `(state, our value)` pairs.
///
/// Z and stencil off, straight alpha blending, no test, no cull, no fog, no
/// lighting, clipping on, full color write, scissor off.
const RENDER_STATES: [(u32, u32); 13] = [
    (7, 0),     // ZENABLE off
    (14, 0),    // ZWRITEENABLE off
    (15, 0),    // ALPHATESTENABLE off
    (19, 5),    // SRCBLEND = SRCALPHA
    (20, 6),    // DESTBLEND = INVSRCALPHA
    (22, 1),    // CULLMODE = NONE
    (27, 1),    // ALPHABLENDENABLE on
    (28, 0),    // FOGENABLE off
    (52, 0),    // STENCILENABLE off
    (136, 1),   // CLIPPING on
    (137, 0),   // LIGHTING off
    (168, 0xf), // COLORWRITEENABLE = all
    (174, 0),   // SCISSORTESTENABLE off
];

/// Stage-0 texture stage states: modulate texture with diffuse, plain UVs.
const STAGE0_STATES: [(u32, u32); 8] = [
    (1, 4),  // COLOROP = MODULATE
    (2, 2),  // COLORARG1 = TEXTURE
    (3, 0),  // COLORARG2 = DIFFUSE
    (4, 4),  // ALPHAOP = MODULATE
    (5, 2),  // ALPHAARG1 = TEXTURE
    (6, 0),  // ALPHAARG2 = DIFFUSE
    (11, 0), // TEXCOORDINDEX = 0
    (24, 0), // TEXTURETRANSFORMFLAGS = DISABLE
];

/// Stage-0 sampler states: clamp addressing, point filtering (quads are 1:1).
const SAMPLER0_STATES: [(u32, u32); 5] = [
    (1, 3), // ADDRESSU = CLAMP
    (2, 3), // ADDRESSV = CLAMP
    (5, 1), // MAGFILTER = POINT
    (6, 1), // MINFILTER = POINT
    (7, 0), // MIPFILTER = NONE
];

/// A pre-transformed, colored, single-textured vertex (`FVF 0x144`).
#[repr(C)]
struct Vertex {
    x: f32,
    y: f32,
    z: f32,
    rhw: f32,
    color: u32,
    u: f32,
    v: f32,
}

/// The backend `IDirect3DDevice9` reached through the graphics device object.
struct Backend {
    dev: usize,
    vtbl: usize,
}

impl Backend {
    /// The backend behind `gx`, `None` when absent or implausible.
    ///
    /// The odd-pointer rejection is the reference's own plausibility check on
    /// this slot.
    fn from_gx(gx: usize) -> Option<Self> {
        if gx == 0 {
            return None;
        }
        // SAFETY: `gx` is the live graphics device object; the backend
        // interface pointer slot sits at the fixed offset.
        let dev = unsafe { *((gx + GX_BACKEND_SLOT) as *const usize) };
        if dev == 0 || dev & 1 != 0 {
            return None;
        }
        // SAFETY: `dev` is a COM interface pointer; its first word is the
        // vtable pointer.
        let vtbl = unsafe { *(dev as *const usize) };
        if vtbl == 0 {
            return None;
        }
        Some(Self { dev, vtbl })
    }

    /// The backend behind the global graphics device pointer.
    fn current() -> Option<Self> {
        // SAFETY: a fixed pointer slot in the live host image (base verified
        // at load), holding the graphics device object or null.
        let gx = unsafe { *(GX_DEVICE_PTR as *const usize) };
        Self::from_gx(gx)
    }

    /// The method pointer at vtable byte offset `off`.
    fn slot(&self, off: usize) -> usize {
        // SAFETY: `vtbl + off` is inside the backend's vtable; every offset
        // used here indexes a method the interface declares.
        unsafe { *((self.vtbl + off) as *const usize) }
    }

    /// `GetRenderState`.
    fn render_state(&self, state: u32) -> u32 {
        let mut value = 0u32;
        let method: extern "stdcall" fn(usize, u32, *mut u32) -> i32 =
            // SAFETY: the vtable slot holds the live method with this
            // prototype; COM methods are stdcall with `this` first.
            unsafe { core::mem::transmute(self.slot(VT_GET_RENDER_STATE)) };
        method(self.dev, state, &raw mut value);
        value
    }

    /// `SetRenderState`.
    fn set_render_state(&self, state: u32, value: u32) {
        let method: extern "stdcall" fn(usize, u32, u32) -> i32 =
            // SAFETY: as in `render_state`, with the setter prototype.
            unsafe { core::mem::transmute(self.slot(VT_SET_RENDER_STATE)) };
        method(self.dev, state, value);
    }

    /// `GetTextureStageState`.
    fn stage_state(&self, stage: u32, ty: u32) -> u32 {
        let mut value = 0u32;
        let method: extern "stdcall" fn(usize, u32, u32, *mut u32) -> i32 =
            // SAFETY: as in `render_state`, with this getter's prototype.
            unsafe { core::mem::transmute(self.slot(VT_GET_TEXTURE_STAGE_STATE)) };
        method(self.dev, stage, ty, &raw mut value);
        value
    }

    /// `SetTextureStageState`.
    fn set_stage_state(&self, stage: u32, ty: u32, value: u32) {
        let method: extern "stdcall" fn(usize, u32, u32, u32) -> i32 =
            // SAFETY: as in `render_state`, with this setter's prototype.
            unsafe { core::mem::transmute(self.slot(VT_SET_TEXTURE_STAGE_STATE)) };
        method(self.dev, stage, ty, value);
    }

    /// `GetSamplerState`.
    fn sampler_state(&self, stage: u32, ty: u32) -> u32 {
        let mut value = 0u32;
        let method: extern "stdcall" fn(usize, u32, u32, *mut u32) -> i32 =
            // SAFETY: as in `render_state`, with this getter's prototype.
            unsafe { core::mem::transmute(self.slot(VT_GET_SAMPLER_STATE)) };
        method(self.dev, stage, ty, &raw mut value);
        value
    }

    /// `SetSamplerState`.
    fn set_sampler_state(&self, stage: u32, ty: u32, value: u32) {
        let method: extern "stdcall" fn(usize, u32, u32, u32) -> i32 =
            // SAFETY: as in `render_state`, with this setter's prototype.
            unsafe { core::mem::transmute(self.slot(VT_SET_SAMPLER_STATE)) };
        method(self.dev, stage, ty, value);
    }

    /// `GetFVF`.
    fn fvf(&self) -> u32 {
        let mut value = 0u32;
        let method: extern "stdcall" fn(usize, *mut u32) -> i32 =
            // SAFETY: as in `render_state`, with this getter's prototype.
            unsafe { core::mem::transmute(self.slot(VT_GET_FVF)) };
        method(self.dev, &raw mut value);
        value
    }

    /// `SetFVF`.
    fn set_fvf(&self, value: u32) {
        let method: extern "stdcall" fn(usize, u32) -> i32 =
            // SAFETY: as in `render_state`, with this setter's prototype.
            unsafe { core::mem::transmute(self.slot(VT_SET_FVF)) };
        method(self.dev, value);
    }

    /// `GetTexture(0)`; the returned interface carries an extra reference.
    fn texture0(&self) -> usize {
        let mut value = 0usize;
        let method: extern "stdcall" fn(usize, u32, *mut usize) -> i32 =
            // SAFETY: as in `render_state`, with this getter's prototype.
            unsafe { core::mem::transmute(self.slot(VT_GET_TEXTURE)) };
        method(self.dev, 0, &raw mut value);
        value
    }

    /// `SetTexture(0, texture)`.
    fn set_texture0(&self, texture: usize) {
        let method: extern "stdcall" fn(usize, u32, usize) -> i32 =
            // SAFETY: as in `render_state`, with this setter's prototype.
            unsafe { core::mem::transmute(self.slot(VT_SET_TEXTURE)) };
        method(self.dev, 0, texture);
    }

    /// `GetVertexShader`; the returned interface carries an extra reference.
    fn vertex_shader(&self) -> usize {
        let mut value = 0usize;
        let method: extern "stdcall" fn(usize, *mut usize) -> i32 =
            // SAFETY: as in `render_state`, with this getter's prototype.
            unsafe { core::mem::transmute(self.slot(VT_GET_VERTEX_SHADER)) };
        method(self.dev, &raw mut value);
        value
    }

    /// `SetVertexShader`.
    fn set_vertex_shader(&self, shader: usize) {
        let method: extern "stdcall" fn(usize, usize) -> i32 =
            // SAFETY: as in `render_state`, with this setter's prototype.
            unsafe { core::mem::transmute(self.slot(VT_SET_VERTEX_SHADER)) };
        method(self.dev, shader);
    }

    /// `GetPixelShader`; the returned interface carries an extra reference.
    fn pixel_shader(&self) -> usize {
        let mut value = 0usize;
        let method: extern "stdcall" fn(usize, *mut usize) -> i32 =
            // SAFETY: as in `render_state`, with this getter's prototype.
            unsafe { core::mem::transmute(self.slot(VT_GET_PIXEL_SHADER)) };
        method(self.dev, &raw mut value);
        value
    }

    /// `SetPixelShader`.
    fn set_pixel_shader(&self, shader: usize) {
        let method: extern "stdcall" fn(usize, usize) -> i32 =
            // SAFETY: as in `render_state`, with this setter's prototype.
            unsafe { core::mem::transmute(self.slot(VT_SET_PIXEL_SHADER)) };
        method(self.dev, shader);
    }

    /// `DrawPrimitiveUP`: one two-triangle strip from four inline vertices.
    fn draw_quad(&self, vertices: &[Vertex; 4]) {
        let method: extern "stdcall" fn(usize, u32, u32, *const Vertex, u32) -> i32 =
            // SAFETY: as in `render_state`, with the draw prototype.
            unsafe { core::mem::transmute(self.slot(VT_DRAW_PRIMITIVE_UP)) };
        method(
            self.dev,
            PT_TRIANGLESTRIP,
            2,
            vertices.as_ptr(),
            u32::try_from(core::mem::size_of::<Vertex>()).expect("vertex stride fits"),
        );
    }

    /// Create a managed-pool ARGB texture and upload `raster` tinted white.
    fn upload(&self, raster: &crate::typeset::Raster) -> Option<TexHandle> {
        let mut texture = 0usize;
        let method: extern "stdcall" fn(
            usize,
            u32,
            u32,
            u32,
            u32,
            u32,
            u32,
            *mut usize,
            usize,
        ) -> i32 =
            // SAFETY: as in `render_state`, with the creation prototype.
            unsafe { core::mem::transmute(self.slot(VT_CREATE_TEXTURE)) };
        let result = method(
            self.dev,
            raster.width,
            raster.height,
            1,
            0,
            FMT_A8R8G8B8,
            POOL_MANAGED,
            &raw mut texture,
            0,
        );
        if result < 0 || texture == 0 {
            return None;
        }
        let handle = TexHandle {
            ptr: texture,
            width: raster.width,
            height: raster.height,
        };

        // SAFETY: `texture` is the COM interface just created; its first
        // word is the vtable pointer.
        let tex_vtbl = unsafe { *(texture as *const usize) };
        #[repr(C)]
        struct LockedRect {
            pitch: i32,
            bits: usize,
        }
        let mut locked = LockedRect { pitch: 0, bits: 0 };
        // SAFETY: `tex_vtbl + 0x4c` is the texture vtable's `LockRect` slot.
        let lock_slot = unsafe { *((tex_vtbl + VT_TEX_LOCK_RECT) as *const usize) };
        let lock: extern "stdcall" fn(usize, u32, *mut LockedRect, usize, u32) -> i32 =
            // SAFETY: the slot holds the live method with this prototype.
            unsafe { core::mem::transmute(lock_slot) };
        if lock(texture, 0, &raw mut locked, 0, 0) < 0 || locked.bits == 0 {
            return None; // dropping `handle` releases the texture
        }
        let pitch = usize::try_from(locked.pitch).unwrap_or(0);
        if pitch >= (raster.width as usize) * 4 {
            for y in 0..raster.height as usize {
                let row = locked.bits + y * pitch;
                for x in 0..raster.width as usize {
                    let alpha = u32::from(raster.coverage[y * raster.width as usize + x]);
                    // SAFETY: `row + x*4` is inside the locked level: `y`
                    // and `x` are bounded by the texture's own extent and
                    // the pitch was checked to hold a full row.
                    unsafe {
                        *((row + x * 4) as *mut u32) = (alpha << 24) | 0x00ff_ffff;
                    }
                }
            }
        }
        // SAFETY: `tex_vtbl + 0x50` is the texture vtable's `UnlockRect` slot.
        let unlock_slot = unsafe { *((tex_vtbl + VT_TEX_UNLOCK_RECT) as *const usize) };
        let unlock: extern "stdcall" fn(usize, u32) -> i32 =
            // SAFETY: the slot holds the live method with this prototype.
            unsafe { core::mem::transmute(unlock_slot) };
        unlock(texture, 0);
        Some(handle)
    }
}

/// `IUnknown::Release` on any COM interface pointer.
fn com_release(object: usize) {
    if object == 0 {
        return;
    }
    // SAFETY: `object` is a live COM interface pointer; its first word is
    // the vtable whose third slot is `Release`.
    let vtbl = unsafe { *(object as *const usize) };
    // SAFETY: `vtbl + 0x8` is the vtable's `Release` slot.
    let release_slot = unsafe { *((vtbl + VT_RELEASE) as *const usize) };
    let release: extern "stdcall" fn(usize) -> u32 =
        // SAFETY: the slot holds the live method with the IUnknown prototype.
        unsafe { core::mem::transmute(release_slot) };
    release(object);
}

/// An owned texture; dropping releases the COM reference.
struct TexHandle {
    ptr: usize,
    width: u32,
    height: u32,
}

impl TexHandle {
    /// Forget the handle without releasing.
    ///
    /// Only for a dead device's leftovers, where a release would call
    /// through a freed interface.
    fn leak(self) {
        core::mem::forget(self);
    }
}

impl Drop for TexHandle {
    fn drop(&mut self) {
        com_release(self.ptr);
    }
}

// ── Overlay state ──

/// A floating line: kernel state plus its texture and tint.
struct Line {
    anim: kernel::Floating,
    guid: u64,
    color: [u8; 3],
    base_alpha: u8,
    player_stick: bool,
    tex: TexHandle,
}

/// A crit's render payload inside its carousel group.
struct CritPayload {
    color: [u8; 3],
    base_alpha: u8,
    tex_normal: TexHandle,
    tex_big: TexHandle,
}

/// A per-unit crit carousel plus its stick GUID.
struct Group {
    guid: u64,
    player_stick: bool,
    carousel: kernel::CritsGroup<CritPayload>,
}

/// Everything the overlay owns, all game-thread accessed.
struct Overlay {
    device: usize,
    floats: Vec<Line>,
    smalls: Vec<Line>,
    groups: Vec<Group>,
}

impl Overlay {
    /// Whether nothing is alive at all.
    fn is_empty(&self) -> bool {
        self.floats.is_empty() && self.smalls.is_empty() && self.groups.is_empty()
    }

    /// Drop every entry, releasing textures against the live device.
    fn clear(&mut self) {
        self.floats.clear();
        self.smalls.clear();
        self.groups.clear();
    }

    /// Leak every texture: the device they were created on is gone.
    fn leak_all(&mut self) {
        let floats = core::mem::take(&mut self.floats);
        let smalls = core::mem::take(&mut self.smalls);
        let groups = core::mem::take(&mut self.groups);
        let mut leaked = 0u32;
        for line in floats.into_iter().chain(smalls) {
            line.tex.leak();
            leaked += 1;
        }
        for mut group in groups {
            for payload in group.carousel.drain_payloads() {
                payload.tex_normal.leak();
                payload.tex_big.leak();
                leaked += 1;
            }
        }
        if leaked > 0 {
            if let Some(armed) = tally::arm() {
                LEAKED_ON_DEVICE_SWAP.add(&armed, leaked);
            }
            log::warn!(
                target: crate::win::LOG_TARGET,
                "worldtext: {leaked} overlay entries leaked on a device swap"
            );
        }
    }

    /// Adopt `device`, leaking leftovers when it actually changed.
    fn adopt_device(&mut self, device: usize) {
        if self.device != device {
            if self.device != 0 {
                self.leak_all();
            }
            self.device = device;
        }
    }
}

/// See [`Overlay`]; a mutex only for the `static`, contention-free in play.
static OVERLAY: Mutex<Overlay> = Mutex::new(Overlay {
    device: 0,
    floats: Vec::new(),
    smalls: Vec::new(),
    groups: Vec::new(),
});

/// Ticks per virtual animation frame, from the calibrated invariant counter.
static TICKS_PER_FRAME: LazyLock<i64> = LazyLock::new(|| {
    let hz = wow_shared::tsc::tsc_hz();
    // The counter frequency is far below 2^62; the division mirrors the
    // reference's integer `frequency / 180`.
    i64::try_from(hz / 180).unwrap_or(1).max(1)
});

/// The current tick count for the animation clock.
fn now_ticks() -> i64 {
    // The counter runs from zero at boot and never approaches the sign bit.
    wow_shared::tsc::rdtsc().cast_signed()
}

// ── Projection and environment ──

/// The client window's client-area size in pixels, when it is sane.
fn client_size() -> Option<(f64, f64)> {
    #[repr(C)]
    struct ClientRect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetClientRect(hwnd: usize, rect: *mut ClientRect) -> i32;
    }
    let hwnd = super::notify::game_window();
    if hwnd == 0 {
        return None;
    }
    let mut rect = ClientRect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: `hwnd` is the client's top-level window handle and the rect
    // out-pointer is live for the call.
    let ok = unsafe { GetClientRect(hwnd, &raw mut rect) };
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    (ok != 0 && width > 0 && height > 0).then(|| (f64::from(width), f64::from(height)))
}

/// A `CVar`'s float value, `default` when absent or unparsable.
fn cvar_f64(name: &core::ffi::CStr, default: f64) -> f64 {
    let lookup: extern "fastcall" fn(*const u8) -> usize =
        // SAFETY: a fixed `.text` entry in the live host image (base
        // verified at load); `__fastcall(ecx = name)`, record in `eax`.
        unsafe { core::mem::transmute(GET_CVAR_VA) };
    let record = lookup(name.as_ptr().cast());
    if record == 0 {
        return default;
    }
    // SAFETY: `record + 0x20` is the CVar record's value-string pointer.
    let value = unsafe { *((record + 0x20) as *const *const core::ffi::c_char) };
    if value.is_null() {
        return default;
    }
    // SAFETY: the value is a NUL-terminated string owned by the registry.
    let value = unsafe { core::ffi::CStr::from_ptr(value) };
    value
        .to_str()
        .ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .unwrap_or(default)
}

/// The effective UI scale divisor, the reference's `CVar` pair read.
fn ui_scale() -> f64 {
    let use_scale = cvar_f64(c"useUiScale", 0.0);
    if use_scale > 0.0 {
        let scale = cvar_f64(c"uiScale", 1.0);
        if scale > 0.0 { scale } else { 1.0 }
    } else {
        1.0
    }
}

/// Creation-time layout environment: window height and UI scale.
struct Env {
    window_height: f64,
    ui_scale: f64,
}

impl Env {
    /// The current environment, `None` without a usable window.
    fn current() -> Option<Self> {
        Some(Self {
            window_height: client_size()?.1,
            ui_scale: ui_scale(),
        })
    }

    /// A 768-line UI measure scaled to this window, the reference formula.
    fn scaled(&self, ui_units: f64) -> f64 {
        ui_units / 768.0 / self.ui_scale * self.window_height
    }

    /// The float travel distance in pixels; the player's own text rises 1.5×.
    fn floating_distance(&self, player_stick: bool) -> i32 {
        let base = trunc_i32(self.scaled(FLOAT_TRAVEL_UI)) / 2;
        if player_stick {
            trunc_i32(f64::from(base) * 1.5)
        } else {
            base
        }
    }
}

/// Project a world position to overlay pixels, the reference's transform.
fn world_to_screen(pos: [f32; 3]) -> Option<(f32, f32)> {
    // SAFETY: a fixed pointer slot in the live host image holding the world
    // frame object while a world is loaded.
    let frame = unsafe { *(WORLD_FRAME_PTR as *const usize) };
    if frame == 0 {
        return None;
    }
    let mut ddc = [0.0f32; 3];
    let mut input = pos;
    let project: extern "thiscall" fn(usize, *mut f32, *mut f32) -> u8 =
        // SAFETY: a fixed `.text` entry in the live host image;
        // `__thiscall(ecx = frame)` with in/out vector pointers, hit in `al`.
        unsafe { core::mem::transmute(WORLD_TO_SCREEN_VA) };
    if project(frame, input.as_mut_ptr(), ddc.as_mut_ptr()) == 0 {
        return None;
    }
    let mut x = -1.0f32;
    let mut y = -1.0f32;
    let to_ndc: extern "fastcall" fn(*mut f32, *mut f32, f32, f32) =
        // SAFETY: a fixed `.text` entry in the live host image;
        // `__fastcall(ecx, edx = out pointers)` plus two stacked floats.
        unsafe { core::mem::transmute(DDC_TO_NDC_VA) };
    to_ndc(&raw mut x, &raw mut y, ddc[0], ddc[1]);

    let (width, height) = client_size()?;
    // SAFETY: `frame + 0x3a4` / `+0x3a0` hold the world frame's left/bottom
    // NDC insets as floats.
    let left_ndc = unsafe { *((frame + 0x3a4) as *const f32) };
    // SAFETY: as above, the bottom inset.
    let bottom_ndc = unsafe { *((frame + 0x3a0) as *const f32) };

    let sx = f64::from(left_ndc) * width + f64::from(x) * width;
    let sy = height - f64::from(y) * height - f64::from(bottom_ndc) * height;
    // screen coordinates, far inside f32 range
    Some((sx as f32, sy as f32))
}

/// Euclidean distance between two world points.
fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// The projected anchor and sight state for a stick GUID.
///
/// Mirrors the reference: the anchor rides the unit's head (collision height
/// capped at 4, or a fraction of it for the player's own text), the screen
/// point takes the nameplate perspective offset, and negative screen
/// coordinates mean "behind the camera, hide". Sight is the camera cache.
fn anchor_for(guid: u64, crit: bool, offset: f64) -> (Option<(i32, i32)>, bool) {
    let Some(unit) = super::super::objmgr::object_by_guid(guid) else {
        return (None, false);
    };
    if !unit.is_unit_or_player() {
        return (None, false);
    }
    let player_guid = super::super::objmgr::active_player_guid();
    let mut pos = unit.position();
    let height = unit.collision_box_height();
    if guid == player_guid {
        pos[2] += if crit { height / 2.0 } else { height / 5.0 };
    } else {
        pos[2] += height.min(4.0);
    }
    let Some((sx, sy)) = world_to_screen(pos) else {
        return (None, false);
    };
    if sx < 0.0 || sy < 0.0 {
        return (None, false);
    }
    let mut sy = sy;
    if guid != player_guid
        && let Some(player) = super::super::objmgr::player()
    {
        let cam = super::camera::translated_position();
        let to_player = distance(cam, player.position());
        let to_target = distance(cam, unit.position());
        if to_target > 0.0 {
            // screen-space offset, far inside f32 range
            let perspective = (offset * f64::from(to_player / to_target)) as f32;
            sy -= perspective;
        }
    }
    let in_sight = super::insight::camera_in_sight(unit) > 0;
    (
        Some((trunc_i32(f64::from(sx)), trunc_i32(f64::from(sy)))),
        in_sight,
    )
}

// ── Creation ──

/// The px sizes for the three face roles at the current setting.
fn px_sizes() -> (f32, f32, f32) {
    let base = font_size();
    (
        to_px(base),
        to_px(base + BIG_PX_DELTA),
        to_px(base + SMALL_PX_DELTA),
    )
}

/// Small positive pixel count to `f32`, exact.
fn to_px(v: i32) -> f32 {
    // Sizes are clamped to [10, 100] plus small deltas; exact in f32.
    // clamped two-digit sizes
    (v.max(1)) as f32
}

/// `f64 -> i32` truncation toward zero, saturating, without x87 help.
fn trunc_i32(v: f64) -> i32 {
    if v >= 2_147_483_647.0 {
        i32::MAX
    } else if v <= -2_147_483_648.0 {
        i32::MIN
    } else {
        // Range-checked right above; SSE2 truncation on every target here.
        // range-checked above
        v as i32
    }
}

/// Unknown world-text types seen, served by the debug query.
static UNKNOWN_TYPES: Mutex<Vec<(i32, String)>> = Mutex::new(Vec::new());

/// The face name lines are currently drawn with, for change logging.
static LAST_PICK: Mutex<String> = Mutex::new(String::new());

/// Log the face serving new lines whenever it changes.
fn note_pick(label: &str) {
    let mut last = LAST_PICK.lock().expect("game-thread only");
    if *last != label {
        log::info!(target: crate::win::LOG_TARGET, "worldtext: drawing new lines with {label}");
        last.clear();
        last.push_str(label);
    }
}

/// The reference's world-text type palette overrides and defaults.
fn line_color(text_type: i32, color: u32) -> [u8; 3] {
    match text_type {
        // Experience text: the reference's pink.
        4 => [0xff, 0x33, 0xcc],
        // Honor text: the reference's gold.
        5 => [239, 191, 4],
        _ => {
            if color != 0 && color & 1 == 0 {
                [
                    u8::try_from((color >> 8) & 0xff).expect("masked to a byte"),
                    u8::try_from((color >> 16) & 0xff).expect("masked to a byte"),
                    u8::try_from((color >> 24) & 0xff).expect("masked to a byte"),
                ]
            } else {
                [255, 255, 255]
            }
        }
    }
}

/// Build a floating line and its texture; `None` falls back to stock.
fn build_line(
    text: &str,
    guid: u64,
    color: [u8; 3],
    base_alpha: u8,
    direction: kernel::Direction,
    small: bool,
) -> Option<Line> {
    let faces = FACES.as_ref().ok()?;
    let Some((face, label)) = faces.pick(text) else {
        tally::bump(&DELEGATED_UNCOVERED);
        return None;
    };
    note_pick(&label);
    let Some(env) = Env::current() else {
        tally::bump(&DELEGATED_UNREADY);
        return None;
    };
    let Some(backend) = Backend::current() else {
        tally::bump(&DELEGATED_UNREADY);
        return None;
    };
    let (base_px, _, small_px) = px_sizes();
    let px = if small { small_px } else { base_px };
    let raster = face.rasterize(text, px);
    let (width, height) = face.measure(text, px);

    let mut overlay = OVERLAY.lock().expect("overlay is game-thread only");
    overlay.adopt_device(backend.dev);
    let Some(tex) = backend.upload(&raster) else {
        tally::bump(&TEXTURE_FAILURES);
        return None;
    };
    drop(overlay);

    let player_guid = super::super::objmgr::active_player_guid();
    let player_stick = guid == player_guid;
    let arc_right = ARC_PARITY.fetch_add(1, Ordering::Relaxed).is_multiple_of(2);
    let spec = kernel::FloatSpec {
        width,
        height,
        floating_distance: env.floating_distance(player_stick),
        arc_radius: env.scaled(ARC_RADIUS_UI),
        arc_towards_right: arc_right,
        start_ticks: now_ticks(),
        ticks_per_frame: *TICKS_PER_FRAME,
    };
    Some(Line {
        anim: kernel::Floating::new(&spec, direction),
        guid,
        color,
        base_alpha,
        player_stick,
        tex,
    })
}

/// Place a new line: seed its rect, fast-forward overlaps, insert, cap.
fn insert_line(mut line: Line, small: bool) {
    let offset = {
        let env = Env::current();
        env.map_or(0.0, |e| e.scaled(nameplate_height()))
    };
    let (anchor, in_sight) = anchor_for(line.guid, false, offset);
    let now = now_ticks();
    let _ = line.anim.tick(now, anchor, in_sight);

    let mut guard = OVERLAY.lock().expect("overlay is game-thread only");
    let overlay = &mut *guard;
    let (list, other) = if small {
        (&mut overlay.smalls, &mut overlay.floats)
    } else {
        (&mut overlay.floats, &mut overlay.smalls)
    };
    let rect = line.anim.rect();
    let overlap = kernel::max_overlap_height(&rect, list.iter().map(|l| l.anim.rect()));
    if overlap > 0 {
        for existing in list.iter_mut() {
            existing.anim.fast_forward(overlap);
        }
    }
    list.push(line);
    let overlap = kernel::max_overlap_height(&rect, other.iter().map(|l| l.anim.rect()));
    if overlap > 0 {
        for existing in other.iter_mut() {
            existing.anim.fast_forward(overlap);
        }
    }
    let live = overlay.floats.len() + overlay.smalls.len();
    if live > MAX_FLOATS {
        tally::bump(&CAPPED);
        if overlay.floats.is_empty() {
            overlay.smalls.remove(0);
        } else {
            overlay.floats.remove(0);
        }
    }
    drop(guard);
}

/// Add a crit to its unit's carousel, building textures for both faces.
fn insert_crit(text: &str, guid: u64, color: [u8; 3], base_alpha: u8) -> bool {
    let Some(faces) = FACES.as_ref().ok() else {
        return false;
    };
    let Some((face, label)) = faces.pick(text) else {
        tally::bump(&DELEGATED_UNCOVERED);
        return false;
    };
    note_pick(&label);
    let Some(backend) = Backend::current() else {
        tally::bump(&DELEGATED_UNREADY);
        return false;
    };
    let (base_px, big_px, _) = px_sizes();
    let raster_normal = face.rasterize(text, base_px);
    let raster_big = face.rasterize(text, big_px);
    // The rect metrics come from the big face, the reference's huge-face
    // measure (its huge differs from big only in GDI weight).
    let (width, height) = face.measure(text, big_px);

    let mut overlay = OVERLAY.lock().expect("overlay is game-thread only");
    overlay.adopt_device(backend.dev);
    let Some(tex_normal) = backend.upload(&raster_normal) else {
        tally::bump(&TEXTURE_FAILURES);
        return false;
    };
    let Some(tex_big) = backend.upload(&raster_big) else {
        tally::bump(&TEXTURE_FAILURES);
        return false;
    };

    let player_guid = super::super::objmgr::active_player_guid();
    let now = now_ticks();
    let spec = kernel::CritSpec {
        width,
        height,
        start_ticks: now,
        ticks_per_frame: *TICKS_PER_FRAME,
    };
    let payload = CritPayload {
        color,
        base_alpha,
        tex_normal,
        tex_big,
    };
    let coin = COIN_PARITY
        .fetch_add(1, Ordering::Relaxed)
        .is_multiple_of(2);

    if let Some(group) = overlay.groups.iter_mut().find(|g| g.guid == guid) {
        group
            .carousel
            .add(kernel::Crit::new(&spec), payload, coin, now);
        return true;
    }
    // Push distances from the reference's five-eights measure at the big px.
    let (push_w, push_h) = face.measure("88888", big_px);
    let mut carousel = kernel::CritsGroup::new(push_w, push_h);
    carousel.add(kernel::Crit::new(&spec), payload, coin, now);
    overlay.groups.push(Group {
        guid,
        player_stick: guid == player_guid,
        carousel,
    });
    true
}

// ── Entry points ──

/// The world-text creation wrapper's overlay half; `true` swallows the call.
///
/// Types 0/1/3 (hits, absorbs, dodges), 4 (experience) and 5 (honor) become
/// floating lines, type 2 becomes a crit; an unknown type is recorded for
/// the debug query and delegated. Every not-ready path answers `false`, so
/// the stock renderer stays the fallback rather than the failure mode.
pub fn intercept(this: *mut u8, text_type: i32, text: *const u8, color: u32) -> bool {
    if !super::settings::in_world() || !use_combat_text() || !scene_enabled() {
        return false;
    }
    if text.is_null() {
        return false;
    }
    // SAFETY: the client passes a NUL-terminated string it owns for the call.
    let raw = unsafe { core::ffi::CStr::from_ptr(text.cast()) };
    let Ok(text) = raw.to_str() else {
        tally::bump(&DELEGATED_UNCOVERED);
        return false;
    };
    // SAFETY: `this + 0x10` is the world-text record's stick GUID.
    let mut guid = unsafe { ((this.addr() + 0x10) as *const u64).read_unaligned() };
    if guid == 0 {
        guid = super::super::objmgr::active_player_guid();
    }
    let color = line_color(text_type, color);

    match text_type {
        0 | 1 | 3 | 4 | 5 => {
            let Some(line) = build_line(text, guid, color, 255, kernel::Direction::Up, false)
            else {
                return false;
            };
            insert_line(line, false);
            tally::bump(&FLOATS_SHOWN);
            true
        }
        2 => {
            let shown = insert_crit(text, guid, color, 255);
            if shown {
                tally::bump(&CRITS_SHOWN);
            }
            shown
        }
        _ => {
            let mut unknown = UNKNOWN_TYPES.lock().expect("game-thread only");
            if unknown.len() < 8 && !unknown.iter().any(|(t, _)| *t == text_type) {
                unknown.push((text_type, text.to_owned()));
            }
            false
        }
    }
}

/// The script-driven small floating text, stuck to the player.
pub fn add_small_floating(
    text: &str,
    color: [u8; 3],
    alpha: u8,
    direction: kernel::Direction,
) -> bool {
    if !super::settings::in_world() || !scene_enabled() {
        return false;
    }
    let guid = super::super::objmgr::active_player_guid();
    let Some(line) = build_line(text, guid, color, alpha, direction, true) else {
        return false;
    };
    insert_line(line, true);
    tally::bump(&FLOATS_SHOWN);
    true
}

/// The script-driven crit text, stuck to the player.
pub fn add_crit_text(text: &str, color: [u8; 3], alpha: u8) -> bool {
    if !super::settings::in_world() || !scene_enabled() {
        return false;
    }
    let guid = super::super::objmgr::active_player_guid();
    let shown = insert_crit(text, guid, color, alpha);
    if shown {
        tally::bump(&CRITS_SHOWN);
    }
    shown
}

/// One draw command: a texture quad at a screen position with a tint.
struct DrawCmd {
    tex: usize,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    color: u32,
}

/// Pack the modulate tint: alpha scaled like the reference's truncation.
fn tint(color: [u8; 3], base_alpha: f64, alpha: f64) -> u32 {
    let a = trunc_i32(base_alpha * alpha).clamp(0, 255).cast_unsigned();
    (a << 24) | (u32::from(color[0]) << 16) | (u32::from(color[1]) << 8) | u32::from(color[2])
}

/// The shadow offset for a line: the reference's player/other split.
const fn shadow_offset(player_stick: bool) -> (i32, i32) {
    if player_stick {
        (SHADOW_WEIGHT, SHADOW_WEIGHT)
    } else {
        (SHADOW_WEIGHT * 2, -SHADOW_WEIGHT)
    }
}

/// Push a line's shadow-plus-fill pair for its texture at `(x, y)`.
fn push_pair(cmds: &mut Vec<DrawCmd>, line: &Line, x: i32, y: i32, alpha: f64) {
    let (dx, dy) = shadow_offset(line.player_stick);
    cmds.push(DrawCmd {
        tex: line.tex.ptr,
        x: x + dx,
        y: y + dy,
        width: line.tex.width,
        height: line.tex.height,
        color: tint([0, 0, 0], SHADOW_ALPHA, alpha),
    });
    cmds.push(DrawCmd {
        tex: line.tex.ptr,
        x,
        y,
        width: line.tex.width,
        height: line.tex.height,
        color: tint(line.color, f64::from(line.base_alpha), alpha),
    });
}

/// The scene-end draw pass: tick everything, then draw what survived.
///
/// Runs between the deferred-batch flush and the displaced scene-end, on the
/// presenting scene-end only. Every device state touched is read first and
/// written back after, so the client's own state shadow stays truthful.
pub fn draw_pass(gx: *mut u8) {
    let gx = gx as usize;
    if gx == 0 {
        return;
    }
    {
        let overlay = OVERLAY.lock().expect("overlay is game-thread only");
        if overlay.is_empty() {
            return;
        }
    }
    // SAFETY: `gx + 0x3a38` is the graphics device's backend-ready dword the
    // reference gates its own draws on.
    let ready = unsafe { *((gx + GX_READY_SLOT) as *const u32) };
    if ready == 0 {
        return;
    }
    let Some(backend) = Backend::from_gx(gx) else {
        return;
    };

    let now = now_ticks();
    let offset = Env::current().map_or(0.0, |e| e.scaled(nameplate_height()));
    let mut cmds: Vec<DrawCmd> = Vec::new();

    let mut guard = OVERLAY.lock().expect("overlay is game-thread only");
    guard.adopt_device(backend.dev);

    // Crit carousels tick first so the float passes can test overlap against
    // this frame's crit rects, the reference's pre-pass.
    let mut crit_cmds: Vec<DrawCmd> = Vec::new();
    let Overlay {
        groups,
        floats,
        smalls,
        ..
    } = &mut *guard;
    groups.retain_mut(|group| {
        let (anchor, in_sight) = anchor_for(group.guid, true, offset);
        let player_stick = group.player_stick;
        group.carousel.tick_all(|crit, payload| {
            let state = crit.tick(now, anchor, in_sight);
            if let kernel::CritTick::Draw {
                rect,
                alpha,
                font,
                centered,
            } = state
            {
                let tex = match font {
                    kernel::CritFont::Normal => &payload.tex_normal,
                    kernel::CritFont::Big => &payload.tex_big,
                };
                let (x, y) = if centered {
                    (
                        i32::midpoint(rect.left, rect.right)
                            - i32::try_from(tex.width).unwrap_or(0) / 2,
                        i32::midpoint(rect.top, rect.bottom)
                            - i32::try_from(tex.height).unwrap_or(0) / 2,
                    )
                } else {
                    (rect.left - 1, rect.top - 1)
                };
                let (dx, dy) = shadow_offset(player_stick);
                crit_cmds.push(DrawCmd {
                    tex: tex.ptr,
                    x: x + dx,
                    y: y + dy,
                    width: tex.width,
                    height: tex.height,
                    color: tint([0, 0, 0], SHADOW_ALPHA, alpha),
                });
                crit_cmds.push(DrawCmd {
                    tex: tex.ptr,
                    x,
                    y,
                    width: tex.width,
                    height: tex.height,
                    color: tint(payload.color, f64::from(payload.base_alpha), alpha),
                });
            }
            state
        })
    });

    // Small floats, then regular floats; both skip lines a crit overlaps.
    for list in [smalls, floats] {
        list.retain_mut(|line| {
            let (anchor, in_sight) = anchor_for(line.guid, false, offset);
            match line.anim.tick(now, anchor, in_sight) {
                kernel::Tick::End => false,
                kernel::Tick::Hidden => true,
                kernel::Tick::Draw { rect, alpha } => {
                    if !groups.iter().any(|g| g.carousel.intersects(&rect)) {
                        push_pair(&mut cmds, line, rect.left - 1, rect.top - 1, alpha);
                    }
                    true
                }
            }
        });
    }
    cmds.append(&mut crit_cmds);
    // The device section below reads only the command list; the textures it
    // names stay alive because their owning entries stayed in the lists and
    // every overlay path runs on this thread.
    drop(guard);
    if cmds.is_empty() {
        return;
    }

    // Save, set, draw, restore.
    let saved_rs: Vec<u32> = RENDER_STATES
        .iter()
        .map(|&(state, _)| backend.render_state(state))
        .collect();
    let saved_tss: Vec<u32> = STAGE0_STATES
        .iter()
        .map(|&(ty, _)| backend.stage_state(0, ty))
        .collect();
    let saved_stage1_colorop = backend.stage_state(1, 1);
    let saved_samp: Vec<u32> = SAMPLER0_STATES
        .iter()
        .map(|&(ty, _)| backend.sampler_state(0, ty))
        .collect();
    let saved_fvf = backend.fvf();
    let saved_texture = backend.texture0();
    let saved_vs = backend.vertex_shader();
    let saved_ps = backend.pixel_shader();

    for &(state, value) in &RENDER_STATES {
        backend.set_render_state(state, value);
    }
    for &(ty, value) in &STAGE0_STATES {
        backend.set_stage_state(0, ty, value);
    }
    backend.set_stage_state(1, 1, 1); // stage 1 COLOROP = DISABLE
    for &(ty, value) in &SAMPLER0_STATES {
        backend.set_sampler_state(0, ty, value);
    }
    backend.set_fvf(FVF_QUAD);
    backend.set_vertex_shader(0);
    backend.set_pixel_shader(0);

    let mut bound = 0usize;
    for cmd in &cmds {
        if cmd.tex != bound {
            backend.set_texture0(cmd.tex);
            bound = cmd.tex;
        }
        let x0 = half_px(cmd.x);
        let y0 = half_px(cmd.y);
        let x1 = x0 + width_px(cmd.width);
        let y1 = y0 + width_px(cmd.height);
        let quad = [
            vertex(x0, y0, cmd.color, 0.0, 0.0),
            vertex(x1, y0, cmd.color, 1.0, 0.0),
            vertex(x0, y1, cmd.color, 0.0, 1.0),
            vertex(x1, y1, cmd.color, 1.0, 1.0),
        ];
        backend.draw_quad(&quad);
    }

    for (&(state, _), &value) in RENDER_STATES.iter().zip(&saved_rs) {
        backend.set_render_state(state, value);
    }
    for (&(ty, _), &value) in STAGE0_STATES.iter().zip(&saved_tss) {
        backend.set_stage_state(0, ty, value);
    }
    backend.set_stage_state(1, 1, saved_stage1_colorop);
    for (&(ty, _), &value) in SAMPLER0_STATES.iter().zip(&saved_samp) {
        backend.set_sampler_state(0, ty, value);
    }
    backend.set_fvf(saved_fvf);
    backend.set_texture0(saved_texture);
    // The getters hand back referenced interfaces; balance them.
    com_release(saved_texture);
    backend.set_vertex_shader(saved_vs);
    com_release(saved_vs);
    backend.set_pixel_shader(saved_ps);
    com_release(saved_ps);
}

/// A quad corner at half-pixel-corrected screen coordinates.
fn vertex(x: f32, y: f32, color: u32, u: f32, v: f32) -> Vertex {
    Vertex {
        x,
        y,
        z: 0.0,
        rhw: 1.0,
        color,
        u,
        v,
    }
}

/// Pixel coordinate to the D3D9 half-texel-corrected vertex position.
fn half_px(v: i32) -> f32 {
    // Screen coordinates: exact in f32 far past any display width.
    // screen coordinates
    (v as f32) - 0.5
}

/// Texture extent as a vertex span.
fn width_px(v: u32) -> f32 {
    // Texture extents are bounded by the display size.
    // texture extents
    v as f32
}

// ── Lifecycle ──

/// The device-release wrapper's half: teardown drops every texture.
///
/// Flag 0 is a reset — managed-pool textures survive and nothing happens.
/// Flag 1 destroys the backend device: everything is released now, while
/// the interface is still alive, and the stored device forgets.
pub fn release_resources(flag: i32) {
    if flag == 1 {
        let mut overlay = OVERLAY.lock().expect("overlay is game-thread only");
        overlay.clear();
        overlay.device = 0;
    }
}

/// Drop every live line (leaving the world, or scripts toggling off).
pub fn clear() {
    OVERLAY.lock().expect("overlay is game-thread only").clear();
}

/// The debug query: unknown types, gating state, and the disable reason.
pub fn debug_text() -> String {
    use core::fmt::Write as _;
    let mut out = String::new();
    let unknown_guard = UNKNOWN_TYPES.lock().expect("game-thread only");
    let unknown = unknown_guard.clone();
    drop(unknown_guard);
    if !unknown.is_empty() {
        out.push_str("Unimplemented world text history:");
        for (ty, text) in unknown {
            let _ = write!(out, "\n{ty} {text}");
        }
        out.push('\n');
    }
    if !super::settings::in_world() {
        out.push_str("Player is not in world\n");
    }
    let overlay = OVERLAY.lock().expect("overlay is game-thread only");
    let (device, floats, smalls, groups) = (
        overlay.device,
        overlay.floats.len(),
        overlay.smalls.len(),
        overlay.groups.len(),
    );
    drop(overlay);
    let _ = writeln!(
        out,
        "Overlay state: device {device:#x}, {floats} floats, {smalls} smalls, {groups} crit groups"
    );
    if let Ok(faces) = &*FACES {
        let _ = writeln!(
            out,
            "Client faces: {} (system fonts resolve by file name on demand)",
            faces.roster()
        );
        let stem = selected_stem();
        let resolution = if faces.resolve(&stem).is_some() {
            "resolves"
        } else {
            "matches no face, lines fall back"
        };
        let _ = writeln!(out, "Selected face: {stem} ({resolution})");
    }
    let reason = disable_reason();
    if !reason.is_empty() {
        out.push_str(&reason);
        out.push('\n');
    }
    out
}

/// One cumulative line for the overlay counters, when any has fired.
pub fn emit_cumulative() {
    let floats = FLOATS_SHOWN.get();
    let crits = CRITS_SHOWN.get();
    let uncovered = DELEGATED_UNCOVERED.get();
    let unready = DELEGATED_UNREADY.get();
    let failures = TEXTURE_FAILURES.get();
    let leaked = LEAKED_ON_DEVICE_SWAP.get();
    let capped = CAPPED.get();
    if floats | crits | uncovered | unready | failures | leaked | capped != 0 {
        log::debug!(
            target: tally::TARGET,
            "unitxp worldtext: {floats} floats, {crits} crits, {uncovered} uncovered, \
             {unready} unready, {failures} tex failures, {leaked} leaked, {capped} capped",
        );
    }
}
