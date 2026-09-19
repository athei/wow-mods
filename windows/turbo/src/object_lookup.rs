//! GUID lookup over the client's 32-bit intrusive hash table.

/// Find an active object without retaining state between lookups.
///
/// `read` supplies words from one stable manager and its live bucket/node records.
/// Address arithmetic wraps at the client's 32-bit width. The caller owns the
/// exclusion against mutation and the lifetime of the returned borrowed object.
pub fn lookup(manager: u32, low: u32, high: u32, mut read: impl FnMut(u32) -> u32) -> u32 {
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
    if matches(node, low, high, &mut read) {
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
        if matches(node, low, high, &mut read) {
            return node;
        }
    }
}

const fn live(node: u32) -> bool {
    node != 0 && node & 1 == 0
}

fn matches(node: u32, low: u32, high: u32, read: &mut impl FnMut(u32) -> u32) -> bool {
    read(node.wrapping_add(0x18)) == low
        && read(node.wrapping_add(0x30)) == low
        && read(node.wrapping_add(0x34)) == high
}

#[cfg(test)]
mod tests;
