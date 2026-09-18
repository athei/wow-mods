//! Projection and coordinate space used by the active world camera.
//!
//! The viewport rectangle is read before the renderer updates the camera's
//! cached aspect. Transport rotation is snapshotted once per edit so collision
//! queries and the eventual local-basis write use the same coordinate space.

use crate::math::editcamera::NearPlane;

const BASE: usize = crate::win::EXPECTED_IMAGE_BASE;
const WORLD_FRAME: usize = BASE + 0x74_b2bc;
const CAMERA_VTABLE: usize = BASE + 0x40_8a9c;

fn field<T: Copy>(address: usize) -> T {
    // SAFETY: this private reader is used only for pinned fields in a verified
    // live camera/frame, or initialized constants in the supported image.
    unsafe { (address as *const T).read_unaligned() }
}

/// The near rectangle for the active frame's projection, or an invalid state.
pub(super) fn current_projection(camera: usize) -> Option<NearPlane> {
    if field::<usize>(camera) != CAMERA_VTABLE {
        return None;
    }
    let frame = field::<usize>(WORLD_FRAME);
    if frame == 0 || frame & 3 != 0 || field::<usize>(frame + 0x65b8) != camera {
        return None;
    }
    let aspect = crate::math::camera_projection::viewport_aspect(&field(frame + 0x390))?;
    // The FOV getter adds the dynamic offset unless flag 8 disables it, then
    // clamps the wide sum. This reproduces its data contract without invoking
    // the renderer or changing its cached matrices.
    let fov = crate::math::camera_projection::effective_fov(
        field(camera + 0x40),
        field(camera + 0x10c),
        field::<u32>(camera + 0x90) & 8 != 0,
        field(BASE + 0x40_89bc),
        field(BASE + 0x40_89b8),
    )?;
    crate::math::camera_projection::near_plane(
        fov,
        aspect,
        field(camera + 0x38),
        field(camera + 0x3c),
    )
}

/// A camera's local-to-world basis transform for one update.
pub(super) struct CameraSpace {
    transform: crate::math::camera_projection::CameraSpace,
}

impl CameraSpace {
    /// Resolve the optional transport using existing object access.
    pub(super) fn snapshot(camera: usize) -> Option<Self> {
        let guid = field::<u64>(camera + 0x98);
        let rotation = if guid == 0 {
            [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
        } else {
            let object = crate::win::objmgr::object_by_guid(guid)?;
            if object.object_type() != 5 {
                return None;
            }
            let angle = object.object_facing()?;
            // The engine passes skip-normalize=1 for this unit axis. The
            // pure kernel's boolean has the opposite meaning: normalize=false.
            crate::math::matrix33::c33_matrix__from_axis_angle__7be490(
                &[0.0, 0.0, 1.0],
                angle,
                false,
            )
        };
        Some(Self {
            transform: crate::math::camera_projection::CameraSpace::from_rotation(rotation)?,
        })
    }

    /// Convert a world-space candidate back to the camera's stored rows.
    pub(super) fn world_to_local(&self, basis: [[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
        self.transform.world_to_local(basis)
    }

    /// Rebuild the rendered frame from effective forward and up.
    pub(super) fn renderer_basis(&self, raw: [[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
        self.transform.renderer_basis(raw)
    }
}
