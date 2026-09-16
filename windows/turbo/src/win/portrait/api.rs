//! Verified native portrait helpers.

const DEVICE_READY_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0018_a260;
const DEVICE_ACTIVE_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0018_a280;
const DEFER_PORTRAIT_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0012_4e10;
const APPEARANCE_READY_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0007_7860;
const MODEL_READY_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0031_0450;
const CAMERA_COUNT_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0031_34d0;
const SCENE_CREATE_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0030_6e10;
const MODEL_CLONE_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0030_7400;
const MODEL_PREPARE_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0007_a230;
const MODEL_CAMERA_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0031_3540;
const MODEL_ANIMATE_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0031_21a0;
const PROJECTION_GET_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0018_b090;
const MATRIX_GET_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0018_b0b0;
const PROJECTION_SET_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0018_b040;
const MATRIX_SET_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0018_b050;
const STATES_PUSH_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0019_3950;
const STATES_POP_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0019_3a50;
const STATES_FLUSH_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0019_4210;
const SCREEN_RECT_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0018_a250;
const SCREEN_FLAG_GET_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0018_a210;
const SCREEN_FLAG_SET_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0018_a1e0;
const DDC_TO_NDC_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0001_ade0;
const MODEL_LOAD_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0031_03d0;
const MODEL_LINK_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0031_0b90;
const CAMERA_COORD_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x003a_bf80;
const CAMERA_SETUP_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x003a_da40;
const CLEAR_COLOR_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0018_a920;
const CLEAR_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0018_a970;
const MODEL_RENDER_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0031_0c50;
const MODEL_LIGHT_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0031_34b0;
const SCENE_RENDER_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0030_8900;
const MODEL_RELEASE_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0031_03a0;
const SCENE_RELEASE_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0030_7320;
const ALPHA_MASK_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0012_58a0;
const PLAYER_NEXT_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0012_72b0;
const CREATURE_NEXT_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0012_7360;
const PLAYER_CREATE_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0012_6ad0;
const CREATURE_CREATE_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0012_6d70;
const TEXTURE_CREATE_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0004_9bf0;
const TEXTURE_RESOLVE_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0004_acf0;
const TEXTURE_UPDATED_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0019_4c80;
const TEXTURE_SET_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0037_0300;
const TEXTURE_CLEAR_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0037_0200;

pub fn verified() -> bool {
    [
        (DEVICE_READY_VA, "8B 0D ?? ?? ?? ?? 85 C9 75 03 33 C0"),
        (DEVICE_ACTIVE_VA, "8B 0D ?? ?? ?? ?? 85 C9 75 03 33 C0"),
        (DEFER_PORTRAIT_VA, "A1 ?? ?? ?? ?? A8 01 53 56 57 8B F9"),
        (APPEARANCE_READY_VA, "55 8B EC 56 8B F1 8A 46 3C 84 C0 57"),
        (MODEL_READY_VA, "55 8B EC 53 8B 5D 08 56 8B F1 8B 46 10"),
        (CAMERA_COUNT_VA, "56 8B F1 8B 46 10 85 C0 75 07 6A 00"),
        (SCENE_CREATE_VA, "6A 00 68 21 02 00 00 68 C8 30 87 00"),
        (MODEL_CLONE_VA, "55 8B EC 53 56 57 8B 7D 08 33 C0 85 FF"),
        (
            MODEL_PREPARE_VA,
            "55 8B EC 83 EC 0C 56 6A 0F 8B F1 E8 ?? ?? ?? ??",
        ),
        (MODEL_CAMERA_VA, "55 8B EC 56 8B F1 8B 46 10 85 C0 75 07"),
        (MODEL_ANIMATE_VA, "55 8B EC 83 EC 18 53 56 8B F1 8B 46 10"),
        (PROJECTION_GET_VA, "51 8B 0D ?? ?? ?? ?? E8 ?? ?? ?? ??"),
        (MATRIX_GET_VA, "51 8B 0D ?? ?? ?? ?? E8 ?? ?? ?? ??"),
        (
            PROJECTION_SET_VA,
            "8B C1 8B 0D ?? ?? ?? ?? 8B 11 50 FF 52 70",
        ),
        (MATRIX_SET_VA, "8B C1 8B 0D ?? ?? ?? ?? 8B 11 50 FF 52 74"),
        (STATES_PUSH_VA, "55 8B EC 83 EC 08 53 56 8B F1 8B 46 08"),
        (STATES_POP_VA, "55 8B EC 83 EC 18 53 56 8B F1 8B 4E 1C"),
        (STATES_FLUSH_VA, "55 8B EC 51 53 56 8B 75 08 85 F6 8B D9"),
        (SCREEN_RECT_VA, "8B C1 8B 0D ?? ?? ?? ?? 8B 11 50 FF 52 60"),
        (SCREEN_FLAG_GET_VA, "83 F9 0A 7C 0A 6A 57 E8 ?? ?? ?? ??"),
        (
            SCREEN_FLAG_SET_VA,
            "8B C1 83 F8 0A 7C 08 6A 57 E8 ?? ?? ?? ??",
        ),
        (
            DDC_TO_NDC_VA,
            "55 8B EC 85 C9 74 0B D9 45 08 D8 35 ?? ?? ?? ??",
        ),
        (MODEL_LOAD_VA, "55 8B EC 53 8B 5D 08 56 57 8B F9 8B 47 10"),
        (MODEL_LINK_VA, "55 8B EC 8B 45 08 85 C0 8B 41 44 74 25"),
        (CAMERA_COORD_VA, "55 8B EC 56 57 8B 7D 08 85 FF 74 19"),
        (CAMERA_SETUP_VA, "55 8B EC 85 C9 75 0B 6A 57 E8 ?? ?? ?? ??"),
        (
            CLEAR_COLOR_VA,
            "55 8B EC 51 8B 4D 08 8B C4 89 08 8B 0D ?? ?? ?? ??",
        ),
        (CLEAR_VA, "8B C1 8B 0D ?? ?? ?? ?? 8B 11 50 FF 52 6C"),
        (MODEL_RENDER_VA, "55 8B EC 8B 81 CC 01 00 00 85 C0 74 0D"),
        (
            MODEL_LIGHT_VA,
            "55 8B EC 8B 45 08 8B 55 0C 89 81 BC 03 00 00",
        ),
        (SCENE_RENDER_VA, "55 8B EC B8 5C 33 00 00 E8 ?? ?? ?? ??"),
        (
            MODEL_RELEASE_VA,
            "56 8B F1 8B 06 48 89 06 75 16 E8 ?? ?? ?? ??",
        ),
        (
            SCENE_RELEASE_VA,
            "56 8B F1 8B 06 48 89 06 75 16 E8 ?? ?? ?? ??",
        ),
        (ALPHA_MASK_VA, "55 8B EC 81 EC 04 05 00 00 A0 ?? ?? ?? ??"),
        (PLAYER_NEXT_VA, "55 8B EC 8B 55 08 85 D2 74 08 8B 01"),
        (CREATURE_NEXT_VA, "55 8B EC 8B 55 08 85 D2 74 08 8B 01"),
        (
            PLAYER_CREATE_VA,
            "55 8B EC 83 EC 08 53 56 8B F1 83 7E 24 FF",
        ),
        (
            CREATURE_CREATE_VA,
            "55 8B EC 83 EC 08 53 56 8B F1 83 7E 24 FF",
        ),
        (
            TEXTURE_CREATE_VA,
            "55 8B EC 51 53 56 57 6A 00 6A FE 68 04 56 83 00",
        ),
        (TEXTURE_RESOLVE_VA, "55 8B EC 56 8B F1 85 F6 75 0E 6A 57"),
        (TEXTURE_UPDATED_VA, "55 8B EC 53 8B 5D 08 80 3B 00 57 8B F9"),
        (TEXTURE_SET_VA, "55 8B EC 56 8B F1 8B 8E CC 00 00 00"),
        (TEXTURE_CLEAR_VA, "55 8B EC 83 EC 14 53 56 8B 75 08 57"),
        (0x0052_5b10, "55 8B EC 83 EC 18 56 8B F2 8D 45 F4"),
    ]
    .into_iter()
    .all(|(address, signature)| {
        // SAFETY: all addresses belong to the fixed client image checked before hook installation.
        let verified = unsafe { wow_hook::signature_matches(address, signature) };
        if !verified {
            log::warn!(target: "wow", "GPU portrait helper signature mismatch at {address:#010x}");
        }
        verified
    })
}

pub fn device_ready() -> extern "cdecl" fn() -> u32 {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(DEVICE_READY_VA) }
}

pub fn device_active() -> extern "cdecl" fn() -> u32 {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(DEVICE_ACTIVE_VA) }
}

pub fn defer_portrait() -> extern "fastcall" fn(*const u64) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(DEFER_PORTRAIT_VA) }
}

pub fn appearance_ready() -> extern "thiscall" fn(usize, usize) -> u8 {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(APPEARANCE_READY_VA) }
}

pub fn model_ready() -> extern "thiscall" fn(usize, u32, u32) -> u32 {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(MODEL_READY_VA) }
}

pub fn camera_count() -> extern "thiscall" fn(usize) -> u32 {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(CAMERA_COUNT_VA) }
}

pub fn scene_create() -> extern "cdecl" fn() -> usize {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(SCENE_CREATE_VA) }
}

pub fn model_clone() -> extern "thiscall" fn(usize, usize, u32) -> usize {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(MODEL_CLONE_VA) }
}

pub fn model_prepare() -> extern "thiscall" fn(usize) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(MODEL_PREPARE_VA) }
}

pub fn model_camera() -> extern "thiscall" fn(usize, u32) -> usize {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(MODEL_CAMERA_VA) }
}

pub fn model_animate() -> extern "thiscall" fn(usize, usize, u32, u32, u32, f32, f32, u32) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(MODEL_ANIMATE_VA) }
}

pub fn projection_get() -> extern "fastcall" fn(*mut f32) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(PROJECTION_GET_VA) }
}

pub fn matrix_get() -> extern "fastcall" fn(*mut f32) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(MATRIX_GET_VA) }
}

pub fn projection_set() -> extern "fastcall" fn(*const f32) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(PROJECTION_SET_VA) }
}

pub fn matrix_set() -> extern "fastcall" fn(*const f32) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(MATRIX_SET_VA) }
}

pub fn states_push() -> extern "thiscall" fn(usize) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(STATES_PUSH_VA) }
}

pub fn states_pop() -> extern "thiscall" fn(usize) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(STATES_POP_VA) }
}

pub fn states_flush() -> extern "thiscall" fn(usize, u32) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(STATES_FLUSH_VA) }
}

pub fn screen_rect() -> extern "fastcall" fn(*mut f32) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(SCREEN_RECT_VA) }
}

pub fn screen_flag_get() -> extern "fastcall" fn(u32) -> u8 {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(SCREEN_FLAG_GET_VA) }
}

pub fn screen_flag_set() -> extern "fastcall" fn(u32, u32) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(SCREEN_FLAG_SET_VA) }
}

pub fn ddc_to_ndc() -> extern "fastcall" fn(*mut f32, *mut f32, f32, f32) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(DDC_TO_NDC_VA) }
}

pub fn model_load() -> extern "thiscall" fn(usize, u32, u32) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(MODEL_LOAD_VA) }
}

pub fn model_link() -> extern "thiscall" fn(usize, u32) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(MODEL_LINK_VA) }
}

pub fn camera_coord() -> extern "fastcall" fn(usize, u32, *mut f32) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(CAMERA_COORD_VA) }
}

pub fn camera_setup() -> extern "fastcall" fn(usize, *const f32, u32) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(CAMERA_SETUP_VA) }
}

pub fn clear_color() -> extern "stdcall" fn(u32) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(CLEAR_COLOR_VA) }
}

pub fn clear() -> extern "fastcall" fn(u32) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(CLEAR_VA) }
}

pub fn model_render() -> extern "thiscall" fn(usize, u32) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(MODEL_RENDER_VA) }
}

pub fn model_light() -> extern "thiscall" fn(usize, usize, usize) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(MODEL_LIGHT_VA) }
}

pub fn scene_render() -> extern "thiscall" fn(usize, u32) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(SCENE_RENDER_VA) }
}

pub fn model_release() -> extern "thiscall" fn(usize) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(MODEL_RELEASE_VA) }
}

pub fn scene_release() -> extern "thiscall" fn(usize) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(SCENE_RELEASE_VA) }
}

pub fn alpha_mask() -> extern "fastcall" fn(u32) -> usize {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(ALPHA_MASK_VA) }
}

pub fn player_next() -> extern "thiscall" fn(usize, usize) -> usize {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(PLAYER_NEXT_VA) }
}

pub fn creature_next() -> extern "thiscall" fn(usize, usize) -> usize {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(CREATURE_NEXT_VA) }
}

pub fn player_create() -> extern "thiscall" fn(usize, u32, u32, u32) -> usize {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(PLAYER_CREATE_VA) }
}

pub fn creature_create() -> extern "thiscall" fn(usize, u32, u32, u32) -> usize {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(CREATURE_CREATE_VA) }
}

pub fn texture_create()
-> extern "fastcall" fn(u32, u32, u32, u32, u32, usize, usize, *const u8, u32) -> usize {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(TEXTURE_CREATE_VA) }
}

pub fn texture_resolve() -> extern "fastcall" fn(usize, u32, usize) -> usize {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(TEXTURE_RESOLVE_VA) }
}

pub fn texture_updated() -> extern "thiscall" fn(usize, usize) -> () {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(TEXTURE_UPDATED_VA) }
}

pub fn texture_set() -> extern "thiscall" fn(usize, usize) -> u32 {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(TEXTURE_SET_VA) }
}

pub fn texture_clear() -> extern "thiscall" fn(usize, *const u8, u32, u32, u32) -> u32 {
    // SAFETY: initialize verifies this entry; callers use its checked native argument contract.
    unsafe { core::mem::transmute(TEXTURE_CLEAR_VA) }
}
