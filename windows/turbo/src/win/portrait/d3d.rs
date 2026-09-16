//! Owned D3D9 portrait targets and scoped state restoration.

use std::cell::RefCell;

pub struct Com(crate::portrait::OwnedResource);

impl Com {
    /// Take one existing reference, including a nullable getter result.
    pub const unsafe fn owned(pointer: usize) -> Self {
        // SAFETY: the caller transfers one live IUnknown reference or null.
        Self(unsafe { crate::portrait::OwnedResource::new(pointer, release_com) })
    }
    pub const fn pointer(&self) -> usize {
        self.0.pointer()
    }
    pub fn into_raw(self) -> usize {
        self.0.into_raw()
    }
}

fn release_com(pointer: usize) {
    let release: extern "stdcall" fn(usize) -> u32 =
        // SAFETY: the owner holds a live IUnknown reference; slot 2 is Release.
        unsafe { core::mem::transmute(slot(pointer, 2)) };
    release(pointer);
}

const fn slot(object: usize, index: usize) -> usize {
    // SAFETY: every caller holds a live interface through an owner or the active device callback.
    let table = unsafe { *(object as *const usize) };
    // SAFETY: callers use published D3D9 interface slots with their exact method signatures.
    unsafe { *((table as *const usize).wrapping_add(index)) }
}

#[repr(C)]
#[derive(Default)]
struct Viewport {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    near: f32,
    far: f32,
}

#[repr(C)]
#[derive(Default)]
struct LockedRect {
    pitch: i32,
    pixels: *mut u8,
}

#[repr(C)]
struct Vertex {
    x: f32,
    y: f32,
    z: f32,
    rhw: f32,
    u: f32,
    v: f32,
}

struct Resources {
    device: usize,
    mask: Com,
    depth: Option<Com>,
}

thread_local! {
    static RESOURCES: RefCell<Option<Resources>> = const { RefCell::new(None) };
}

pub fn release_resources(teardown: bool) {
    RESOURCES.with_borrow_mut(|storage| {
        if teardown {
            *storage = None;
        } else if let Some(resources) = storage {
            resources.depth = None;
        }
    });
}

fn reference(object: usize) -> Com {
    let add_ref: extern "stdcall" fn(usize) -> u32 =
        // SAFETY: the caller owns this live interface; slot 1 is AddRef.
        unsafe { core::mem::transmute(slot(object, 1)) };
    add_ref(object);
    // SAFETY: AddRef supplied this owner with a distinct reference.
    unsafe { Com::owned(object) }
}

fn texture(device: usize, target: bool) -> Option<Com> {
    let mut output = 0;
    let method: extern "stdcall" fn(usize,u32,u32,u32,u32,u32,u32,*mut usize,usize) -> i32 =
        // SAFETY: device is live and slot 23 is IDirect3DDevice9::CreateTexture.
        unsafe { core::mem::transmute(slot(device,23)) };
    let result = method(
        device,
        64,
        64,
        1,
        u32::from(target),
        21,
        u32::from(!target),
        &raw mut output,
        0,
    );
    // SAFETY: a non-null creation result transfers one reference, also released on failure.
    let texture = unsafe { Com::owned(output) };
    (result >= 0 && output != 0).then_some(texture)
}

#[repr(C)]
#[derive(Default)]
struct CreationParameters {
    adapter: u32,
    kind: u32,
    window: usize,
    behavior: u32,
}

#[repr(C)]
#[derive(Default)]
struct DisplayMode {
    width: u32,
    height: u32,
    refresh: u32,
    format: u32,
}

fn compatible_depth(device: usize, format: u32) -> bool {
    let get_direct3d: extern "stdcall" fn(usize, *mut usize) -> i32 =
        // SAFETY: device is live; slot 6 returns its parent IDirect3D9 with one reference.
        unsafe { core::mem::transmute(slot(device, 6)) };
    let get_creation: extern "stdcall" fn(usize, *mut CreationParameters) -> i32 =
        // SAFETY: slot 9 writes the published four-field creation-parameter structure.
        unsafe { core::mem::transmute(slot(device, 9)) };
    let get_mode: extern "stdcall" fn(usize, u32, *mut DisplayMode) -> i32 =
        // SAFETY: slot 8 writes the published display-mode structure for swap chain zero.
        unsafe { core::mem::transmute(slot(device, 8)) };
    let mut parent = 0;
    let result = get_direct3d(device, &raw mut parent);
    // SAFETY: a non-null GetDirect3D result owns one reference.
    let parent = unsafe { Com::owned(parent) };
    let mut creation = CreationParameters::default();
    let mut mode = DisplayMode::default();
    if result < 0
        || parent.pointer() == 0
        || get_creation(device, &raw mut creation) < 0
        || get_mode(device, 0, &raw mut mode) < 0
    {
        return false;
    }
    let check_format: extern "stdcall" fn(usize, u32, u32, u32, u32, u32, u32) -> i32 =
        // SAFETY: the owned parent is IDirect3D9; slot 10 is CheckDeviceFormat.
        unsafe { core::mem::transmute(slot(parent.pointer(), 10)) };
    let check_match: extern "stdcall" fn(usize, u32, u32, u32, u32, u32) -> i32 =
        // SAFETY: slot 12 is CheckDepthStencilMatch with adapter, device, and three formats.
        unsafe { core::mem::transmute(slot(parent.pointer(), 12)) };
    check_format(
        parent.pointer(),
        creation.adapter,
        creation.kind,
        mode.format,
        1,
        3,
        21,
    ) >= 0
        && check_match(
            parent.pointer(),
            creation.adapter,
            creation.kind,
            mode.format,
            21,
            format,
        ) >= 0
}

fn depth_surface(device: usize) -> Option<Com> {
    let method: extern "stdcall" fn(usize,u32,u32,u32,u32,u32,i32,*mut usize,usize) -> i32 =
        // SAFETY: device is live and slot 29 is CreateDepthStencilSurface.
        unsafe { core::mem::transmute(slot(device,29)) };
    for format in [75, 80] {
        if !compatible_depth(device, format) {
            continue;
        }
        let mut output = 0;
        let result = method(device, 64, 64, format, 0, 0, 1, &raw mut output, 0);
        // SAFETY: a successful creation transfers one reference.
        let surface = unsafe { Com::owned(output) };
        if result >= 0 && output != 0 {
            return Some(surface);
        }
    }
    None
}

fn set_target(object: usize, index: u32, surface: usize) -> bool {
    let method: extern "stdcall" fn(usize,u32,usize) -> i32 =
        // SAFETY: the caller retains this interface; slot 37 has the declared D3D9 signature.
        unsafe { core::mem::transmute(slot(object,37)) };
    method(object, index, surface) >= 0
}

fn get_target(object: usize, index: u32, output: *mut usize) -> bool {
    let method: extern "stdcall" fn(usize,u32,*mut usize) -> i32 =
        // SAFETY: the caller retains this interface; slot 38 has the declared D3D9 signature.
        unsafe { core::mem::transmute(slot(object,38)) };
    method(object, index, output) >= 0
}

fn set_depth(object: usize, surface: usize) -> bool {
    let method: extern "stdcall" fn(usize,usize) -> i32 =
        // SAFETY: the caller retains this interface; slot 39 has the declared D3D9 signature.
        unsafe { core::mem::transmute(slot(object,39)) };
    method(object, surface) >= 0
}

fn get_depth(object: usize, output: *mut usize) -> bool {
    let method: extern "stdcall" fn(usize,*mut usize) -> i32 =
        // SAFETY: the caller retains this interface; slot 40 has the declared D3D9 signature.
        unsafe { core::mem::transmute(slot(object,40)) };
    method(object, output) >= 0
}

fn set_viewport(object: usize, viewport: *const Viewport) -> bool {
    let method: extern "stdcall" fn(usize,*const Viewport) -> i32 =
        // SAFETY: the caller retains this interface; slot 47 has the declared D3D9 signature.
        unsafe { core::mem::transmute(slot(object,47)) };
    method(object, viewport) >= 0
}

fn get_viewport(object: usize, viewport: *mut Viewport) -> bool {
    let method: extern "stdcall" fn(usize,*mut Viewport) -> i32 =
        // SAFETY: the caller retains this interface; slot 48 has the declared D3D9 signature.
        unsafe { core::mem::transmute(slot(object,48)) };
    method(object, viewport) >= 0
}

fn set_state(object: usize, state: u32, value: u32) -> bool {
    let method: extern "stdcall" fn(usize,u32,u32) -> i32 =
        // SAFETY: the caller retains this interface; slot 57 has the declared D3D9 signature.
        unsafe { core::mem::transmute(slot(object,57)) };
    method(object, state, value) >= 0
}

fn create_state_block(object: usize, kind: u32, output: *mut usize) -> bool {
    let method: extern "stdcall" fn(usize,u32,*mut usize) -> i32 =
        // SAFETY: the caller retains this interface; slot 59 has the declared D3D9 signature.
        unsafe { core::mem::transmute(slot(object,59)) };
    method(object, kind, output) >= 0
}

fn set_texture(object: usize, stage: u32, texture: usize) -> bool {
    let method: extern "stdcall" fn(usize,u32,usize) -> i32 =
        // SAFETY: the caller retains this interface; slot 65 has the declared D3D9 signature.
        unsafe { core::mem::transmute(slot(object,65)) };
    method(object, stage, texture) >= 0
}

fn set_stage(object: usize, stage: u32, parameter: u32, value: u32) -> bool {
    let method: extern "stdcall" fn(usize,u32,u32,u32) -> i32 =
        // SAFETY: the caller retains this interface; slot 67 has the declared D3D9 signature.
        unsafe { core::mem::transmute(slot(object,67)) };
    method(object, stage, parameter, value) >= 0
}

fn set_sampler(object: usize, sampler: u32, state: u32, value: u32) -> bool {
    let method: extern "stdcall" fn(usize,u32,u32,u32) -> i32 =
        // SAFETY: the caller retains this interface; slot 69 has the declared D3D9 signature.
        unsafe { core::mem::transmute(slot(object,69)) };
    method(object, sampler, state, value) >= 0
}

fn draw(object: usize, kind: u32, count: u32, data: *const Vertex, stride: u32) -> bool {
    let method: extern "stdcall" fn(usize,u32,u32,*const Vertex,u32) -> i32 =
        // SAFETY: the caller retains this interface; slot 83 has the declared D3D9 signature.
        unsafe { core::mem::transmute(slot(object,83)) };
    method(object, kind, count, data, stride) >= 0
}

fn set_fvf(object: usize, fvf: u32) -> bool {
    let method: extern "stdcall" fn(usize,u32) -> i32 =
        // SAFETY: the caller retains this interface; slot 89 has the declared D3D9 signature.
        unsafe { core::mem::transmute(slot(object,89)) };
    method(object, fvf) >= 0
}

fn set_vertex_shader(object: usize, shader: usize) -> bool {
    let method: extern "stdcall" fn(usize,usize) -> i32 =
        // SAFETY: the caller retains this interface; slot 92 has the declared D3D9 signature.
        unsafe { core::mem::transmute(slot(object,92)) };
    method(object, shader) >= 0
}

fn set_pixel_shader(object: usize, shader: usize) -> bool {
    let method: extern "stdcall" fn(usize,usize) -> i32 =
        // SAFETY: the caller retains this interface; slot 107 has the declared D3D9 signature.
        unsafe { core::mem::transmute(slot(object,107)) };
    method(object, shader) >= 0
}

fn surface_level(object: usize, level: u32, output: *mut usize) -> bool {
    let method: extern "stdcall" fn(usize,u32,*mut usize) -> i32 =
        // SAFETY: the caller retains this interface; slot 18 has the declared D3D9 signature.
        unsafe { core::mem::transmute(slot(object,18)) };
    method(object, level, output) >= 0
}

fn lock_texture(
    object: usize,
    level: u32,
    locked: *mut LockedRect,
    rect: usize,
    flags: u32,
) -> bool {
    let method: extern "stdcall" fn(usize,u32,*mut LockedRect,usize,u32) -> i32 =
        // SAFETY: the caller retains this interface; slot 19 has the declared D3D9 signature.
        unsafe { core::mem::transmute(slot(object,19)) };
    method(object, level, locked, rect, flags) >= 0
}

fn unlock_texture(object: usize, level: u32) -> bool {
    let method: extern "stdcall" fn(usize,u32) -> i32 =
        // SAFETY: the caller retains this interface; slot 20 has the declared D3D9 signature.
        unsafe { core::mem::transmute(slot(object,20)) };
    method(object, level) >= 0
}

fn apply_state(object: usize) -> bool {
    let method: extern "stdcall" fn(usize) -> i32 =
        // SAFETY: the caller retains this interface; slot 5 has the declared D3D9 signature.
        unsafe { core::mem::transmute(slot(object,5)) };
    method(object) >= 0
}

fn resources(device: usize, mask: impl FnOnce() -> Option<Vec<u8>>) -> Option<(Com, Com)> {
    RESOURCES.with_borrow_mut(|storage| {
        if storage
            .as_ref()
            .is_none_or(|resources| resources.device != device)
        {
            *storage = None;
            let mask = mask()?;
            if mask.len() != 4096 {
                return None;
            }
            let mask_texture = texture(device, false)?;
            let mut locked = LockedRect::default();
            if !lock_texture(mask_texture.pointer(), 0, &raw mut locked, 0, 0) {
                return None;
            }
            let good = locked.pitch >= 256 && !locked.pixels.is_null();
            if good {
                for (row, pixels) in mask.as_chunks::<64>().0.iter().enumerate() {
                    for (column, &alpha) in pixels.iter().enumerate() {
                        let offset = row * locked.pitch.cast_unsigned() as usize + column * 4;
                        // SAFETY: successful 64x64 lock, checked pitch, and in-bounds row/column.
                        unsafe {
                            locked
                                .pixels
                                .wrapping_add(offset)
                                .cast::<u32>()
                                .write_unaligned(crate::portrait::mask_pixel(alpha));
                        };
                    }
                }
            }
            if !unlock_texture(mask_texture.pointer(), 0) || !good {
                return None;
            }
            *storage = Some(Resources {
                device,
                mask: mask_texture,
                depth: None,
            });
        }
        let resources = storage.as_mut()?;
        if resources.depth.is_none() {
            resources.depth = Some(depth_surface(device)?);
        }
        Some((
            reference(resources.mask.pointer()),
            reference(resources.depth.as_ref()?.pointer()),
        ))
    })
}

pub struct Pass {
    device: usize,
    previous_target: Com,
    previous_depth: Com,
    previous_viewport: Viewport,
    target: Option<Com>,
    surface: Com,
    mask: Com,
    depth: Com,
    restored: bool,
}

impl Pass {
    pub fn begin(device: usize, mask: impl FnOnce() -> Option<Vec<u8>>) -> Option<Self> {
        // A portrait has one color output. Leave an application MRT pass entirely untouched.
        for index in 1..4 {
            let mut other = 0;
            let _ = get_target(device, index, &raw mut other);
            // SAFETY: GetRenderTarget transfers a reference when a target exists.
            let other = unsafe { Com::owned(other) };
            if other.pointer() != 0 {
                return None;
            }
        }
        let target = texture(device, true)?;
        let (mask, depth) = resources(device, mask)?;
        let mut surface = 0;
        let surface_ok = surface_level(target.pointer(), 0, &raw mut surface);
        // SAFETY: GetSurfaceLevel transfers a reference when non-null.
        let surface = unsafe { Com::owned(surface) };
        if !surface_ok || surface.pointer() == 0 {
            return None;
        }
        let mut previous_target = 0;
        let target_ok = get_target(device, 0, &raw mut previous_target);
        // SAFETY: GetRenderTarget transfers a reference when non-null.
        let previous_target = unsafe { Com::owned(previous_target) };
        if !target_ok || previous_target.pointer() == 0 {
            return None;
        }
        let mut previous_depth = 0;
        let _ = get_depth(device, &raw mut previous_depth);
        // SAFETY: GetDepthStencilSurface transfers a reference when non-null.
        let previous_depth = unsafe { Com::owned(previous_depth) };
        let mut previous_viewport = Viewport::default();
        if !get_viewport(device, &raw mut previous_viewport) {
            return None;
        }
        let pass = Self {
            device,
            previous_target,
            previous_depth,
            previous_viewport,
            target: Some(target),
            surface,
            mask,
            depth,
            restored: false,
        };
        if !set_depth(device, 0)
            || !set_target(device, 0, pass.surface.pointer())
            || !set_depth(device, pass.depth.pointer())
            || !viewport(device, 0.0, 1.0)
        {
            return None;
        }
        Some(pass)
    }

    pub fn finish(mut self) -> Option<Com> {
        let mut block = 0;
        let created = create_state_block(self.device, 1, &raw mut block);
        // SAFETY: CreateStateBlock transfers one reference when non-null.
        let block = unsafe { Com::owned(block) };
        if !created || block.pointer() == 0 {
            return None;
        }
        let rendered = self.apply_mask();
        let restored = apply_state(block.pointer());
        let targets_restored = self.restore();
        if rendered && restored && targets_restored && ready(self.device) {
            self.target.take()
        } else {
            None
        }
    }

    fn restore(&mut self) -> bool {
        if self.restored {
            return true;
        }
        let restored = set_depth(self.device, 0)
            & set_target(self.device, 0, self.previous_target.pointer())
            & set_depth(self.device, self.previous_depth.pointer())
            & set_viewport(self.device, &raw const self.previous_viewport);
        self.restored = restored;
        if !restored {
            crate::defer_log!(target: "wow", log::Level::Warn,
                "GPU portrait state restoration failed; device recovery is required");
        }
        restored
    }

    fn apply_mask(&self) -> bool {
        let device = self.device;
        let mut ok = set_vertex_shader(device, 0)
            & set_pixel_shader(device, 0)
            & set_fvf(device, 0x104)
            & set_texture(device, 0, self.mask.pointer())
            & viewport(device, 0.0, 1.0);
        for (state, value) in [
            (7, 0),
            (8, 3),
            (26, 0),
            (128, 0),
            (152, 0),
            (161, 0),
            (162, u32::MAX),
            (14, 0),
            (15, 0),
            (19, 2),
            (20, 1),
            (22, 1),
            (27, 0),
            (28, 0),
            (52, 0),
            (136, 0),
            (137, 0),
            (168, 8),
            (174, 0),
            (194, 0),
            (206, 0),
        ] {
            ok &= set_state(device, state, value);
        }
        for (state, value) in [(1, 2), (2, 2), (4, 2), (5, 2), (11, 0), (24, 0), (28, 1)] {
            ok &= set_stage(device, 0, state, value);
        }
        ok &= set_stage(device, 1, 1, 1) & set_stage(device, 1, 4, 1);
        for (state, value) in [(1, 3), (2, 3), (5, 1), (6, 1), (7, 0), (11, 0)] {
            ok &= set_sampler(device, 0, state, value);
        }
        if !ok {
            return false;
        }
        let vertices = [
            Vertex {
                x: -0.5,
                y: -0.5,
                z: 0.0,
                rhw: 1.0,
                u: 0.0,
                v: 0.0,
            },
            Vertex {
                x: 63.5,
                y: -0.5,
                z: 0.0,
                rhw: 1.0,
                u: 1.0,
                v: 0.0,
            },
            Vertex {
                x: -0.5,
                y: 63.5,
                z: 0.0,
                rhw: 1.0,
                u: 0.0,
                v: 1.0,
            },
            Vertex {
                x: 63.5,
                y: 63.5,
                z: 0.0,
                rhw: 1.0,
                u: 1.0,
                v: 1.0,
            },
        ];
        draw(device, 5, 2, vertices.as_ptr(), 24)
    }
}

impl Drop for Pass {
    fn drop(&mut self) {
        self.restore();
    }
}

pub fn viewport(device: usize, near: f32, far: f32) -> bool {
    let viewport = Viewport {
        x: 0,
        y: 0,
        width: 64,
        height: 64,
        near,
        far,
    };
    set_viewport(device, &raw const viewport)
}

pub fn ready(device: usize) -> bool {
    let test: extern "stdcall" fn(usize) -> i32 =
        // SAFETY: the device is live; slot 3 is TestCooperativeLevel.
        unsafe { core::mem::transmute(slot(device, 3)) };
    test(device) >= 0
}
