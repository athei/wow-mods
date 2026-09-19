//! Slot writes over the client's live device and dirty/undo records.

const DEVICE: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0080_ed38;
const RECORD_VA: usize = crate::win::EXPECTED_IMAGE_BASE + 0x0019_4010;

/// Set one slot using the device selected at entry.
///
/// The caller supplies a valid slot and value for the current device. All
/// records stay client-owned; callbacks may replace the global device without
/// changing the device or state record already selected by this call.
pub fn set(index: u32, value: u32) {
    // SAFETY: the verified fixed image maps this device global for its lifetime.
    let device = Device(unsafe { (DEVICE as *const u32).read() });
    let state = device.read(0x2824).wrapping_add(index.wrapping_mul(24));
    if read(state) == value {
        return;
    }
    device.record(index, state);
    write(state, value);
    if value != 0 && (23..=30).contains(&index) {
        // SAFETY: nonzero texture slots contain a live texture record. The
        // flag byte is checked before the activation byte, as in the client.
        let flags = unsafe { (value.wrapping_add(60) as *const u8).read() };
        if flags & 128 != 0 {
            // SAFETY: the same live texture record contains the first byte.
            let active = unsafe { (value as *const u8).read() };
            if active != 0 {
                let address = read(device.read(0));
                // SAFETY: the live device's first virtual method is thiscall
                // with one texture argument and four bytes of stack cleanup.
                let activate: extern "thiscall" fn(u32, u32) =
                    unsafe { core::mem::transmute(address) };
                activate(device.0, value);
            }
        }
    }
}

/// A device borrowed for a single slot write.
struct Device(u32);

impl Device {
    const fn read(&self, offset: u32) -> u32 {
        read(self.0.wrapping_add(offset))
    }

    const fn write(&self, offset: u32, value: u32) {
        write(self.0.wrapping_add(offset), value);
    }

    #[inline]
    fn record(&self, index: u32, state: u32) {
        if read(state.wrapping_add(20)) == 0 {
            let count = self.read(0x28);
            let next = count.wrapping_add(1);
            if next > self.read(0x24) {
                self.record_with_growth(index);
                return;
            }
            let destination = self.read(0x2c).wrapping_add(count.wrapping_mul(4));
            self.write(0x28, next);
            write(destination, index);
            write(state.wrapping_add(20), 1);
        }
        if read(state.wrapping_add(16)) != self.read(0x18) {
            let count = self.read(8);
            let next = count.wrapping_add(1);
            if next > self.read(4) {
                // A completed dirty append is already visible to the stock
                // helper. It continues with undo recording exactly once.
                self.record_with_growth(index);
                return;
            }
            let destination = self.read(12).wrapping_add(count.wrapping_mul(24));
            self.write(8, next);
            write(destination, index);
            for word in 0..5 {
                write(
                    destination.wrapping_add(4 + word * 4),
                    read(state.wrapping_add(word * 4)),
                );
            }
            write(state.wrapping_add(16), self.read(0x18));
        }
    }

    fn record_with_growth(&self, index: u32) {
        // SAFETY: installation verifies the entire helper. Its thiscall ABI
        // takes this device and one slot argument, with four-byte cleanup.
        // Growth helpers remain live, including their callbacks and mutations.
        let record: extern "thiscall" fn(u32, u32) = unsafe { core::mem::transmute(RECORD_VA) };
        record(self.0, index);
    }
}

const fn read(address: u32) -> u32 {
    // SAFETY: this module only reads aligned fields in the current device,
    // its live state table, or its live virtual-method table. The caller owns
    // the device's mutation exclusion for this synchronous slot operation.
    unsafe { (address as *const u32).read() }
}

const fn write(address: u32, value: u32) {
    // SAFETY: the device owns the selected state and dirty/undo allocations.
    // Capacity is checked before each append; callbacks run only through the
    // stock helper, which handles allocation and its observable mutations.
    unsafe { (address as *mut u32).write(value) }
}
