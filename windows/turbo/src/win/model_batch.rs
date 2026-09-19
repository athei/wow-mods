//! Model batch submission and bone-palette packing.

use core::marker::PhantomData;

/// A raw record view that does not impose aliasing or alignment on client data.
struct Record<'a, const N: usize>(*mut u8, PhantomData<&'a u8>);

impl<const N: usize> Record<'_, N> {
    /// Borrow a record for the current submission.
    ///
    /// # Safety
    /// The address must denote N live bytes for the view's lifetime. Fields read
    /// must be initialized; fields written must be writable. Stock helper calls
    /// may mutate the record, so this view deliberately creates no references.
    const unsafe fn new(address: usize) -> Self {
        Self(address as *mut u8, PhantomData)
    }

    const fn word<const O: usize>(&self) -> u32 {
        const { assert!(O + 4 <= N) }
        // SAFETY: the compile-time bound lies within the borrowed record.
        let p = unsafe { self.0.add(O) };
        // SAFETY: construction guarantees an initialized word at this field.
        unsafe { p.cast::<u32>().read_unaligned() }
    }

    const fn half<const O: usize>(&self) -> u16 {
        const { assert!(O + 2 <= N) }
        // SAFETY: the compile-time bound lies within the borrowed record.
        let p = unsafe { self.0.add(O) };
        // SAFETY: construction guarantees an initialized halfword here.
        unsafe { p.cast::<u16>().read_unaligned() }
    }

    const fn triple<const O: usize>(&self) -> [u32; 3] {
        const { assert!(O + 12 <= N) }
        // SAFETY: the compile-time bound lies within the borrowed record.
        let p = unsafe { self.0.add(O) };
        // SAFETY: only the three initialized matrix columns are read.
        unsafe { p.cast::<[u32; 3]>().read_unaligned() }
    }

    const fn set<const O: usize>(&mut self, value: u32) {
        const { assert!(O + 4 <= N) }
        // SAFETY: the compile-time bound lies within the borrowed record.
        let p = unsafe { self.0.add(O) };
        // SAFETY: construction guarantees this field is writable.
        unsafe { p.cast::<u32>().write_unaligned(value) };
    }
}

/// One submission's live batch graph.
///
/// The dispatcher owns the stack-local state and the referenced model records
/// throughout submission. Shadow pointers are read only after the matching
/// type/instance guards establish that the previous descriptor is valid.
pub struct Batch<'a>(Record<'a, 0x3358>);

impl Batch<'_> {
    /// Borrow the dispatcher's current model batch.
    ///
    /// # Safety
    /// `this` must be the live batch graph supplied to the model commit entry:
    /// current descriptors, model tables and selected bones are valid; previous
    /// records are valid when their change guards match. The state is writable,
    /// palette storage does not alias the source matrices/remap, and helpers
    /// preserve the graph's lifetime. No concurrent mutation is permitted.
    pub unsafe fn new(this: *mut u8) -> Self {
        // SAFETY: the entry contract provides the full writable batch record.
        Self(unsafe { Record::new(this as usize) })
    }

    const fn type_changed(&self) -> bool {
        self.0.word::<0x3308>() != self.0.word::<0x330c>()
    }

    const fn instance_changed(&self) -> bool {
        self.0.word::<0x3310>() != self.0.word::<0x3314>()
    }

    const fn shared_changed(&self) -> bool {
        self.0.word::<0x3318>() != self.0.word::<0x331c>()
    }

    const fn descriptor(&self) -> Record<'_, 0x40> {
        // SAFETY: the current descriptor is live under Batch's entry contract.
        unsafe { Record::new(self.0.word::<0x3300>() as usize) }
    }

    const fn previous_descriptor(&self) -> Record<'_, 0x40> {
        // SAFETY: callers first establish that the batch type is unchanged.
        unsafe { Record::new(self.0.word::<0x3304>() as usize) }
    }

    const fn mesh(&self) -> Record<'_, 0x10> {
        // SAFETY: initialization stores the live descriptor's mesh pointer.
        unsafe { Record::new(self.0.word::<0x3340>() as usize) }
    }

    const fn owner(&self) -> Record<'_, 0x1034> {
        // SAFETY: the shared owner is live throughout this submission.
        unsafe { Record::new(self.0.word::<0x44>() as usize) }
    }

    const fn model(&self) -> Record<'_, 0xa4> {
        // SAFETY: the loaded model tables outlive the submission.
        unsafe { Record::new(self.0.word::<0x48>() as usize) }
    }

    fn initialize(&mut self) {
        let definition = self.descriptor().word::<0x2c>();
        self.0.set::<0x3338>(definition);
        self.0.set::<0x3340>(self.descriptor().word::<0x30>());
        // SAFETY: the current descriptor owns a live batch definition.
        let definition: Record<'_, 0x14> = unsafe { Record::new(definition as usize) };
        let material = self.model().word::<0x88>() + u32::from(definition.half::<0xa>()) * 4;
        self.0.set::<0x3348>(material);
        let flags = self.owner().word::<4>();
        self.0.set::<0x32f0>(flags & 8);
        self.0.set::<0x32f8>(u32::from(
            flags & 0x10 != 0 && self.descriptor().word::<0x3c>() != u32::MAX,
        ));
    }

    fn palette_changed(&self) -> bool {
        if self.type_changed() || self.instance_changed() {
            return true;
        }
        // SAFETY: the matching type and instance make the previous mesh live.
        let previous: Record<'_, 0x10> = unsafe { Record::new(self.0.word::<0x3344>() as usize) };
        self.mesh().half::<0xe>() != previous.half::<0xe>()
            || u32::from(self.mesh().half::<0xc>()) > self.0.word::<0x3354>()
    }

    fn pack_palette(&mut self) {
        let count = self.mesh().half::<0xc>();
        // Zero bones still extend the dirty range, without reading a matrix.
        if count != 0 {
            let start = usize::from(self.mesh().half::<0xe>());
            let remap = self.model().word::<0x90>() as usize;
            // SAFETY: the current instance outlives this submission.
            let instance: Record<'_, 0x98> =
                unsafe { Record::new(self.0.word::<0x3310>() as usize) };
            let matrices = instance.word::<0x94>() as usize;
            for i in 0..usize::from(count) {
                // SAFETY: the mesh selects count initialized remap entries.
                let index: Record<'_, 2> = unsafe { Record::new(remap + 2 * (start + i)) };
                // SAFETY: each selected bone has four rows with three live
                // columns, at 64-byte stride. No unused fourth column is read.
                let matrix: Record<'_, 60> =
                    unsafe { Record::new(matrices + 64 * usize::from(index.half::<0>())) };
                let packed = crate::palette::pack([
                    matrix.triple::<0>(),
                    matrix.triple::<16>(),
                    matrix.triple::<32>(),
                    matrix.triple::<48>(),
                ]);
                // SAFETY: the dispatcher's palette has space for count bones
                // and is disjoint from the heap matrices and remap table.
                let out = unsafe { self.0.0.add(0x240 + 48 * i) };
                // SAFETY: each bone owns these 48 writable output bytes.
                unsafe { out.cast::<[u32; 12]>().write_unaligned(packed) };
            }
        }
        self.0.set::<0x3240>(self.0.word::<0x3240>().min(31));
        self.0
            .set::<0x3244>(self.0.word::<0x3244>().max(31 + 3 * u32::from(count)));
        self.0.set::<0x3354>(u32::from(count));
    }

    fn bind_programs(&mut self) {
        if self.0.word::<0x32f0>() == 0 {
            bind(0x40, 0);
        } else {
            if self.palette_changed() {
                self.pack_palette();
            }
            if self.type_changed()
                || self.descriptor().word::<0x38>() != self.previous_descriptor().word::<0x38>()
            {
                let table = self.owner().word::<0x102c>();
                let index = self.descriptor().word::<0x38>();
                // SAFETY: the enabled vertex program indexes the owner's table.
                let program: Record<'_, 4> = unsafe { Record::new((table + index * 4) as usize) };
                bind(0x40, program.word::<0>());
            }
            self.upload::<0x3240, 0x3244>(0, 0x50);
        }
        if self.0.word::<0x32f8>() == 0 {
            bind(0x3f, 0);
        } else {
            if self.type_changed()
                || self.descriptor().word::<0x3c>() != self.previous_descriptor().word::<0x3c>()
            {
                let table = self.owner().word::<0x1030>();
                let index = self.descriptor().word::<0x3c>();
                // SAFETY: the enabled pixel program indexes the owner's table.
                let program: Record<'_, 4> = unsafe { Record::new((table + index * 4) as usize) };
                bind(0x3f, program.word::<0>());
            }
            self.upload::<0x32e8, 0x32ec>(1, 0x3248);
        }
    }

    fn upload<const START: usize, const END: usize>(&mut self, stage: u32, base: usize) {
        let start = self.0.word::<START>();
        let end = self.0.word::<END>();
        if start < end {
            // SAFETY: the dirty interval denotes initialized constant vectors
            // in the corresponding vertex or pixel region of this batch.
            let data = unsafe { self.0.0.add(base + 16 * start as usize) };
            // SAFETY: fixed host entry, ECX stage, EDX start, two stack args,
            // ret 8. The backend consumes the vectors before returning.
            let upload: extern "fastcall" fn(u32, u32, *const u8, u32) =
                unsafe { core::mem::transmute(crate::win::EXPECTED_IMAGE_BASE + 0x18_b330) };
            upload(stage, start, data, end - start);
            self.0.set::<START>(u32::MAX);
            self.0.set::<END>(0);
        }
    }

    fn bind_buffers(&self) {
        if self.descriptor().word::<0x34>() != 0 {
            if self.type_changed() || self.instance_changed() {
                // SAFETY: fixed instance-buffer entry, ECX instance, plain ret.
                let prepare: extern "fastcall" fn(u32) -> u32 =
                    unsafe { core::mem::transmute(crate::win::EXPECTED_IMAGE_BASE + 0x31_9930) };
                prepare(self.0.word::<0x3310>());
            }
        } else if self.type_changed()
            || self.shared_changed()
            || self.previous_descriptor().word::<0x34>() != 0
        {
            // SAFETY: fixed shared-buffer entry, ECX shared model, plain ret.
            let prepare: extern "fastcall" fn(u32) -> u32 =
                unsafe { core::mem::transmute(crate::win::EXPECTED_IMAGE_BASE + 0x31_dab0) };
            prepare(self.0.word::<0x3318>());
        }
        if self.0.word::<0x32f0>() != 0 {
            if self.type_changed() || self.shared_changed() {
                // SAFETY: fixed skinned-buffer entry, ECX shared model, plain ret.
                let prepare: extern "fastcall" fn(u32) -> u32 =
                    unsafe { core::mem::transmute(crate::win::EXPECTED_IMAGE_BASE + 0x31_dc80) };
                prepare(self.0.word::<0x3318>());
            }
        } else if self.type_changed()
            || self.instance_changed()
            || self.0.word::<0x3340>() != self.0.word::<0x3344>()
        {
            self.bind_unskinned();
        }
    }

    fn bind_unskinned(&self) {
        // SAFETY: the current descriptor owns this batch definition.
        let definition: Record<'_, 0x14> =
            unsafe { Record::new(self.descriptor().word::<0x2c>() as usize) };
        if definition.half::<0xe>() == 2 {
            let table = self.model().word::<0xa0>() as usize;
            // SAFETY: a two-texture definition selects two live lookup entries.
            let textures: Record<'_, 4> =
                unsafe { Record::new(table + 2 * usize::from(definition.half::<0x12>())) };
            let first = textures.half::<0>();
            let second = textures.half::<2>();
            if first != u16::MAX && second != u16::MAX && first != second {
                // SAFETY: fixed dual-UV entry, ECX instance, stack mesh, ret 4.
                let prepare: extern "thiscall" fn(u32, u32) -> u32 =
                    unsafe { core::mem::transmute(crate::win::EXPECTED_IMAGE_BASE + 0x31_9ac0) };
                prepare(self.0.word::<0x3310>(), self.0.word::<0x3340>());
                return;
            }
        }
        // SAFETY: fixed vertex preparation entry, ECX instance, stack selector
        // and mesh, ret 8. Its failure result does not suppress submission.
        let prepare: extern "thiscall" fn(u32, u32, u32) -> u32 =
            unsafe { core::mem::transmute(crate::win::EXPECTED_IMAGE_BASE + 0x31_9b20) };
        prepare(
            self.0.word::<0x3310>(),
            self.descriptor().word::<0x34>(),
            self.0.word::<0x3340>(),
        );
    }

    fn draw(&self) {
        let mesh = self.mesh();
        let minimum = if self.0.word::<0x32f0>() == 0 {
            0
        } else {
            mesh.half::<4>()
        };
        let maximum = minimum.wrapping_add(mesh.half::<6>()).wrapping_sub(1);
        let descriptor = [
            3,
            u32::from(mesh.half::<8>()),
            u32::from(mesh.half::<0xa>()) | u32::from(minimum) << 16,
            u32::from(maximum),
        ];
        // SAFETY: fixed draw entry, ECX descriptor, EDX mode. It consumes the
        // fourteen initialized descriptor bytes synchronously, with plain ret.
        let draw: extern "fastcall" fn(*const u32, u32) =
            unsafe { core::mem::transmute(crate::win::EXPECTED_IMAGE_BASE + 0x18_a830) };
        draw(descriptor.as_ptr(), 1);
    }

    /// Commit the current model descriptor and submit its indexed draw.
    pub fn commit(&mut self) {
        self.initialize();
        // SAFETY: fixed texture preparation entry, ECX batch, plain ret.
        let setup: extern "fastcall" fn(*mut u8) =
            unsafe { core::mem::transmute(crate::win::EXPECTED_IMAGE_BASE + 0x30_b740) };
        setup(self.0.0);
        super::hooks::c_gx_batch__update_fog_state__70baf0(self.0.0);
        super::hooks::c_particle_emitter__apply_render_state__70c190(self.0.0.cast());
        self.bind_programs();
        self.bind_buffers();
        self.draw();
    }
}

fn bind(slot: u32, value: u32) {
    // SAFETY: fixed cached-state setter, ECX slot, EDX value, plain ret.
    let bind: extern "fastcall" fn(u32, u32) =
        unsafe { core::mem::transmute(crate::win::EXPECTED_IMAGE_BASE + 0x18_9e80) };
    bind(slot, value);
}
