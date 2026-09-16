//! Native portrait records, model lifetimes, and state restoration.

use super::{api, d3d};
use crate::portrait::Key;

/// A live client allocation whose readable and writable extent was checked at entry.
struct Record {
    address: usize,
    length: usize,
}

impl Record {
    const unsafe fn new(address: usize, length: usize) -> Self {
        Self { address, length }
    }
    fn word(&self, offset: usize) -> u32 {
        assert!(offset + 4 <= self.length);
        // SAFETY: the constructor guarantees this allocation's lifetime and extent.
        unsafe { ((self.address + offset) as *const u32).read_unaligned() }
    }
    fn byte(&self, offset: usize) -> u8 {
        assert!(offset < self.length);
        // SAFETY: the constructor guarantees this allocation's lifetime and extent.
        unsafe { *((self.address + offset) as *const u8) }
    }
    fn set_word(&self, offset: usize, value: u32) {
        assert!(offset + 4 <= self.length);
        // SAFETY: the constructor grants game-thread writes within this live allocation.
        unsafe { ((self.address + offset) as *mut u32).write_unaligned(value) };
    }
    fn set_byte(&self, offset: usize, value: u8) {
        assert!(offset < self.length);
        // SAFETY: the constructor grants game-thread writes within this live allocation.
        unsafe { *((self.address + offset) as *mut u8) = value };
    }
}

const fn global(address: usize) -> u32 {
    // SAFETY: callers name fixed four-byte globals in the verified client image.
    unsafe { (address as *const u32).read_unaligned() }
}

pub fn graphics() -> Option<(usize, usize)> {
    let gx = global(0x00c0_ed38) as usize;
    if gx == 0 {
        return None;
    }
    // SAFETY: the global points to the live graphics object; read its common vtable first.
    let table = unsafe { *(gx as *const usize) };
    if table != 0x0080_9ef8 {
        return None;
    }
    // SAFETY: this is the verified D3D device layout, including its backend pointer.
    let record = unsafe { Record::new(gx, 0x3c04) };
    let device = record.word(0x38a8) as usize;
    (device != 0 && record.word(0x3a38) != 0).then_some((gx, device))
}

const fn guid_low(guid: u64) -> u32 {
    let bytes = guid.to_le_bytes();
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

pub struct Unit {
    record: Record,
    pub key: Key,
    pub guid: u64,
}

impl Unit {
    pub const fn key_for_cache(&self) -> Key {
        match self.key {
            Key::Player(guid) => Key::Player(guid),
            Key::Creature(display) => Key::Creature(display),
        }
    }

    pub unsafe fn new(pointer: usize) -> Self {
        // SAFETY: the portrait entry received a live CGUnit_C from its caller.
        let record = unsafe { Record::new(pointer, 0xd34) };
        // SAFETY: the live unit owns its descriptor block with GUID and type mask.
        let fields = unsafe { Record::new(record.word(8) as usize, 12) };
        let guid = u64::from(fields.word(0)) | (u64::from(fields.word(4)) << 32);
        let key = if fields.word(8) & 0x10 != 0 {
            Key::Player(guid)
        } else {
            // SAFETY: the unit's unit-data pointer includes its display-ID field at 0x1f4.
            let fields = unsafe { Record::new(record.word(0x110) as usize, 0x1f8) };
            Key::Creature(fields.word(0x1f4))
        };
        Self { record, key, guid }
    }

    pub fn pending(&self) -> bool {
        let mut pointer = global(0x00c0_cdd4) as usize;
        while pointer != 0 && pointer & 1 == 0 {
            // SAFETY: the native pending list contains live 16-byte nodes until its tagged sentinel.
            let node = unsafe { Record::new(pointer, 16) };
            let guid = u64::from(node.word(8)) | (u64::from(node.word(12)) << 32);
            if guid == self.guid {
                return true;
            }
            pointer = node.word(4) as usize;
        }
        false
    }

    pub fn ready(&self) -> bool {
        if (api::device_ready())() == 0
            || (api::device_active())() == 0
            || self.record.word(0xcd0) != 0
            || self.record.word(0xccc) != 0
        {
            return false;
        }
        let appearance = self.record.word(0xd30) as usize;
        if appearance != 0 && (api::appearance_ready())(appearance, 0) == 0 {
            return false;
        }
        let model = self.model();
        model != 0 && (api::model_ready())(model, 0, 1) != 0
    }

    fn model(&self) -> usize {
        self.record.word(0xd8) as usize
    }
    pub fn has_camera(&self) -> bool {
        (api::camera_count())(self.model()) != 0
    }

    pub fn cached(&self) -> Option<Portrait> {
        let (base, hash) = match self.key {
            Key::Player(guid) => (0x00c0_ce60, guid_low(guid)),
            Key::Creature(display) => (0x00c0_ce88, display),
        };
        let mask = global(base + 0x24);
        if mask == u32::MAX {
            return None;
        }
        let bucket = global(base + 0x1c) as usize + (hash & mask) as usize * 12;
        // SAFETY: the initialized native hash table owns mask+1 live buckets of 12 bytes each.
        let bucket_record = unsafe { Record::new(bucket, 12) };
        let mut pointer = bucket_record.word(8) as usize;
        while pointer != 0 && pointer & 1 == 0 {
            let player = matches!(self.key, Key::Player(_));
            // SAFETY: the bucket chain contains the corresponding native portrait record type.
            let record = unsafe { Record::new(pointer, if player { 0x38 } else { 0x2c }) };
            if record.word(0) == hash
                && (!player
                    || (record.word(0x18) == guid_low(self.guid)
                        && record.word(0x1c) == (self.guid >> 32) as u32))
            {
                return Some(Portrait { record, player });
            }
            let link = if player {
                (api::player_next())(bucket, pointer)
            } else {
                (api::creature_next())(bucket, pointer)
            };
            // SAFETY: the native next-link accessor returns the live link pair inside this chain.
            let link = unsafe { Record::new(link, 8) };
            pointer = link.word(4) as usize;
        }
        None
    }

    pub fn cache_entry(&self) -> Option<Portrait> {
        if let Some(portrait) = self.cached() {
            return Some(portrait);
        }
        let player = matches!(self.key, Key::Player(_));
        let pointer = match self.key {
            Key::Player(guid) => (api::player_create())(0x00c0_ce60, guid_low(guid), 0, 0),
            Key::Creature(display) => (api::creature_create())(0x00c0_ce88, display, 0, 0),
        };
        if pointer == 0 {
            return None;
        }
        // SAFETY: the native allocator initialized a record of the selected portrait type.
        let record = unsafe { Record::new(pointer, if player { 0x38 } else { 0x2c }) };
        match self.key {
            Key::Player(guid) => {
                record.set_word(0, guid_low(guid));
                record.set_word(0x18, guid_low(guid));
                record.set_word(0x1c, (guid >> 32) as u32);
                record.set_byte(0x20, 1);
            }
            Key::Creature(display) => record.set_word(0, display),
        }
        Some(Portrait { record, player })
    }
}

pub struct Portrait {
    record: Record,
    player: bool,
}

impl Portrait {
    const fn handle_offset(&self) -> usize {
        if self.player { 0x24 } else { 0x18 }
    }
    pub fn handle(&self) -> usize {
        self.record.word(self.handle_offset()) as usize
    }
    pub fn gpu(&self) -> bool {
        let texture = self.texture();
        if texture == 0 {
            return false;
        }
        // SAFETY: the native handle owns this resolved CGxTex and its callback slot.
        let record = unsafe { Record::new(texture, 0x58) };
        record.word(0x44) as usize == texture_callback as *const () as usize
    }
    pub fn dirty(&self) -> bool {
        self.player && self.record.byte(0x20) != 0
    }
    pub fn texture(&self) -> usize {
        let handle = self.handle();
        if handle == 0 {
            0
        } else {
            (api::texture_resolve())(handle, 1, 0)
        }
    }

    pub fn publish(&self, gx: usize, target: d3d::Com) -> Option<usize> {
        let mut handle = self.handle();
        if handle == 0 {
            handle = (api::texture_create())(
                64,
                64,
                1,
                2,
                0x281,
                self.record.address + self.handle_offset() + 4,
                texture_callback as *const () as usize,
                c"Portrait1".as_ptr().cast(),
                0,
            );
            if handle == 0 {
                return None;
            }
            self.record.set_word(
                self.handle_offset(),
                u32::try_from(handle).expect("32-bit native handle"),
            );
        }
        let texture = (api::texture_resolve())(handle, 1, 0);
        if texture == 0 {
            return None;
        }
        // SAFETY: the resolver returns this handle's live CGxTex, owned by the native handle system.
        let texture_record = unsafe { Record::new(texture, 0x58) };
        let old = texture_record.word(0x48) as usize;
        let owns_old = texture_record.byte(1) == 0;
        texture_record.set_word(0x3c, 0x281);
        texture_record.set_word(
            0x44,
            u32::try_from(texture_callback as *const () as usize).expect("32-bit callback"),
        );
        texture_record.set_word(
            0x48,
            u32::try_from(target.into_raw()).expect("32-bit COM pointer"),
        );
        texture_record.set_byte(1, 0);
        (api::texture_updated())(gx, texture);
        // The native handle is unchanged, but its backend texture changed. Force its
        // bindings through the engine so both application and device caches agree.
        (api::states_flush())(gx, 1);
        if owns_old && old != 0 {
            // SAFETY: replacing the native texture slot transfers its previous owned reference here.
            drop(unsafe { d3d::Com::owned(old) });
        }
        if self.player {
            self.record.set_byte(0x20, 0);
        }
        Some(texture)
    }
}

/// Render-target callbacks have no CPU pixels, for any mip or lifecycle command.
const extern "fastcall" fn texture_callback(
    _command: u32,
    _width: u32,
    _height: u32,
    _depth: u32,
    _level: u32,
    _context: usize,
    pitch: *mut u32,
    pixels: *mut usize,
) {
    if !pitch.is_null() {
        // SAFETY: the native texture callback supplies its writable pitch out-parameter.
        unsafe { pitch.write_unaligned(0) };
    }
    if !pixels.is_null() {
        // SAFETY: the native texture callback supplies its writable pixel-pointer out-parameter.
        unsafe { pixels.write_unaligned(0) };
    }
}

pub fn defer(guid: u64) {
    (api::defer_portrait())(&raw const guid);
}
pub fn set_texture(ui: usize, handle: usize) {
    (api::texture_set())(ui, handle);
}
pub fn clear_texture(ui: usize) {
    (api::texture_clear())(ui, c"".as_ptr().cast(), 0, global(0x0087_8cf0), 0);
}

pub fn mask() -> Option<Vec<u8>> {
    let pointer = (api::alpha_mask())(64);
    if pointer == 0 {
        return None;
    }
    // SAFETY: GetAlphaMask(64) returns its persistent byte-array descriptor.
    let record = unsafe { Record::new(pointer, 12) };
    if record.word(4) != 4096 || record.word(8) == 0 {
        return None;
    }
    // SAFETY: the initialized descriptor owns exactly 4096 mask bytes; copy before any callback.
    Some(unsafe { core::slice::from_raw_parts(record.word(8) as *const u8, 4096) }.to_vec())
}

pub unsafe fn depth_range(gx: usize) -> (f32, f32) {
    // SAFETY: the verified ApplyViewport callback passes the live D3D graphics device.
    let record = unsafe { Record::new(gx, 0x3c04) };
    (
        f32::from_bits(record.word(0xf48)),
        f32::from_bits(record.word(0xf4c)),
    )
}

pub unsafe fn viewport_applied(gx: usize) {
    // SAFETY: the verified ApplyViewport callback grants writes to the live device state.
    let record = unsafe { Record::new(gx, 0x3c04) };
    record.set_word(0xf34, 0);
}

struct Scene {
    scene: usize,
    model: usize,
}
impl Drop for Scene {
    fn drop(&mut self) {
        if self.model != 0 {
            (api::model_release())(self.model);
        }
        (api::scene_release())(self.scene);
    }
}

struct EngineState {
    gx: usize,
    projection: [f32; 16],
    matrix: [f32; 16],
    viewport: [f32; 6],
    screen_flag: u32,
    clear_color: u32,
}
impl EngineState {
    fn save(gx: usize) -> Self {
        (api::states_push())(gx);
        let mut state = Self {
            gx,
            // SAFETY: this verified graphics object owns the packed clear color at +0x324.
            clear_color: unsafe { ((gx + 0x324) as *const u32).read_unaligned() },
            projection: [0.0; 16],
            matrix: [0.0; 16],
            viewport: [0.0; 6],
            screen_flag: u32::from((api::screen_flag_get())(1)),
        };
        (api::projection_get())(state.projection.as_mut_ptr());
        (api::matrix_get())(state.matrix.as_mut_ptr());
        // SAFETY: the live CGxDevice owns these six viewport floats, copied before callbacks.
        state.viewport = unsafe { ((gx + 0xf38) as *const [f32; 6]).read_unaligned() };
        (api::screen_flag_set())(1, 0);
        state
    }
}
impl Drop for EngineState {
    fn drop(&mut self) {
        (api::clear_color())(self.clear_color);
        (api::projection_set())(self.projection.as_ptr());
        (api::matrix_set())(self.matrix.as_ptr());
        let v = &self.viewport;
        crate::win::hooks::gx_set_viewport__58af60(v[0], v[1], v[2], v[3], v[4], v[5]);
        (api::screen_flag_set())(1, self.screen_flag);
        (api::states_pop())(self.gx);
        (api::states_flush())(self.gx, 0);
    }
}

pub fn render(unit: &Unit, gx: usize, device: usize) -> Option<d3d::Com> {
    let pass = d3d::Pass::begin(device, mask)?;
    let scene = (api::scene_create())();
    if scene == 0 {
        return None;
    }
    let mut scene = Scene { scene, model: 0 };
    scene.model = (api::model_clone())(scene.scene, unit.model(), 0);
    if scene.model == 0 {
        return None;
    }
    let model = scene.model;
    (api::model_prepare())(model);
    let camera = (api::model_camera())(model, 0);
    if camera == 0 {
        return None;
    }
    (api::model_animate())(model, usize::MAX, 0, 0, 0, 1.0, 0.0, 1);
    let mut screen = [0.0; 4];
    (api::screen_rect())(screen.as_mut_ptr());
    let width = screen[3] - screen[1];
    let height = screen[2] - screen[0];
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let sx = crate::win::hooks::os_gui_scale_x__41ae60(1.0);
    let sy = crate::win::hooks::os_gui_scale_y__41ae70(1.0);
    // Keep the native intermediate precision and explicit f32 camera/viewport stores.
    // Fusing the multiply-subtract changes the rounding before the store.
    let right = (64.0f64 / f64::from(width) * f64::from(sx)) as f32;
    let bottom = (f64::from(sy) - f64::from(sy) / f64::from(height) * 64.0) as f32;
    let aspect = (f64::from(right) / (f64::from(sy) - f64::from(bottom))) as f32;
    let mut left = 0.0;
    let mut low = bottom;
    let mut high = sy;
    let mut right_ndc = right;
    (api::ddc_to_ndc())(&raw mut left, &raw mut low, 0.0, bottom);
    (api::ddc_to_ndc())(&raw mut right_ndc, &raw mut high, right, sy);
    let state = EngineState::save(gx);
    (api::model_load())(model, 1, 1);
    (api::model_link())(model, 1);
    let origin = [0.0; 3];
    crate::win::hooks::c_world_view__build_draw_list__707680(
        scene.scene as *mut core::ffi::c_void,
        origin.as_ptr(),
    );
    let mut eye = [0.0; 3];
    let mut target = [0.0; 3];
    (api::camera_coord())(camera, 7, eye.as_mut_ptr());
    (api::camera_coord())(camera, 8, target.as_mut_ptr());
    let camera_rect = [0.0, 0.0, 1.0, aspect];
    (api::camera_setup())(camera, camera_rect.as_ptr(), 0);
    let active = super::ActivePass::enter(device);
    crate::win::hooks::gx_set_viewport__58af60(left, right_ndc, low, high, 0.0, 1.0);
    (api::clear_color())(0);
    (api::clear())(3);
    (api::model_ready())(model, 1, 1);
    (api::model_link())(model, 1);
    (api::model_render())(model, 1);
    (api::model_light())(model, 0x0052_5b10, 0);
    crate::win::hooks::c_world_view__build_draw_list__707680(
        scene.scene as *mut core::ffi::c_void,
        eye.as_ptr(),
    );
    (api::scene_render())(scene.scene, 0);
    (api::scene_render())(scene.scene, 1);
    drop(active);
    let result = pass.finish();
    drop(state);
    drop(scene);
    result
}

/// A GPU-owned portrait never calls the pixel-upload callback.
pub unsafe fn update_texture(gx: usize, texture: usize) -> bool {
    // SAFETY: the native update hook received this live texture record.
    let record = unsafe { Record::new(texture, 0x58) };
    if record.word(0x44) as usize != texture_callback as *const () as usize {
        return false;
    }
    if record.word(0x48) == 0 {
        // SAFETY: the update hook supplies the live graphics object and its borrowed placeholder.
        let graphics = unsafe { Record::new(gx, 0x3c04) };
        record.set_word(0x48, graphics.word(0x3a68));
        record.set_byte(1, 1);
    }
    if record.byte(1) == 0 {
        (api::texture_updated())(gx, texture);
    }
    true
}

/// Release the owned default-pool target before the native device-release routine.
pub unsafe fn release_texture(gx: usize, texture: usize) {
    // SAFETY: the registry tracks live records, removing them before native destruction.
    let record = unsafe { Record::new(texture, 0x58) };
    // SAFETY: the release callback supplies the live graphics object, including its placeholder.
    let graphics = unsafe { Record::new(gx, 0x3c04) };
    let old = record.word(0x48) as usize;
    let owned = record.byte(1) == 0;
    record.set_word(0x48, graphics.word(0x3a68));
    record.set_byte(1, 1);
    record.set_byte(0, 1);
    if owned && old != 0 {
        // SAFETY: the native slot's single owned reference was detached before this release.
        drop(unsafe { d3d::Com::owned(old) });
    }
}
