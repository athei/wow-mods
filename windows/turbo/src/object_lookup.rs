//! GUID lookup over the client's 32-bit intrusive hash table.

/// Find an active object without retaining state between lookups.
///
/// `read` supplies words from one stable manager and its live bucket/node records.
/// Address arithmetic wraps at the client's 32-bit width. The caller owns the
/// exclusion against mutation and the lifetime of the returned borrowed object.
pub fn lookup(manager: u32, low: u32, high: u32, read: impl FnMut(u32) -> u32) -> u32 {
    lookup_observed(manager, low, high, read, |_| {})
}

/// One completed comparison cohort or live node visit during a lookup.
pub enum Probe {
    Visit,
    HashMiss,
    LowMiss,
    HighMiss,
}

/// Observe traversal without changing the order or extent of memory reads.
///
/// The observer must not mutate the manager or any reachable node.
pub fn lookup_observed(
    manager: u32,
    low: u32,
    high: u32,
    mut read: impl FnMut(u32) -> u32,
    mut observe: impl FnMut(Probe),
) -> u32 {
    let mask = read(manager.wrapping_add(0x24));
    if mask == u32::MAX {
        return 0;
    }
    let buckets = read(manager.wrapping_add(0x1c));
    let bucket = buckets.wrapping_add((low & mask).wrapping_mul(12));
    let mut node = read(bucket.wrapping_add(8));
    if !live(node) {
        return 0;
    }
    if matches(node, low, high, &mut read, &mut observe) {
        return node;
    }

    // A first-node hit never needs the link offset. Once traversal starts,
    // the bucket metadata stays invariant for this call, just like its nodes.
    let next_offset = read(bucket).wrapping_add(4);
    loop {
        node = read(node.wrapping_add(next_offset));
        if !live(node) {
            return 0;
        }
        if matches(node, low, high, &mut read, &mut observe) {
            return node;
        }
    }
}

const fn live(node: u32) -> bool {
    node != 0 && node & 1 == 0
}

fn matches(
    node: u32,
    low: u32,
    high: u32,
    read: &mut impl FnMut(u32) -> u32,
    observe: &mut impl FnMut(Probe),
) -> bool {
    observe(Probe::Visit);
    if read(node.wrapping_add(0x18)) != low {
        observe(Probe::HashMiss);
        return false;
    }
    if read(node.wrapping_add(0x30)) != low {
        observe(Probe::LowMiss);
        return false;
    }
    if read(node.wrapping_add(0x34)) != high {
        observe(Probe::HighMiss);
        return false;
    }
    true
}

/// Resolve a nonzero GUID and retain only objects matching the descriptor type mask.
///
/// `resolve` and `read` share the caller's manager-mutation exclusion. The
/// descriptor pointer at object+8 and its flags at descriptor+8 remain live
/// through both reads. A zero mask still resolves and reads the descriptor.
pub fn typed(
    low: u32,
    high: u32,
    mask: u32,
    resolve: impl FnOnce(u32, u32) -> u32,
    mut read: impl FnMut(u32) -> u32,
) -> u32 {
    if low == 0 && high == 0 {
        return 0;
    }
    let object = resolve(low, high);
    if object == 0 {
        return 0;
    }
    let descriptor = read(object.wrapping_add(8));
    if read(descriptor.wrapping_add(8)) & mask == 0 {
        return 0;
    }
    object
}

#[cfg(test)]
mod tests;
