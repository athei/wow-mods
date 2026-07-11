//! Float-lane region comparison for differential mode.
//!
//! Compares a reimplementation's output region against the original's, lane by
//! lane. Regions are raw little-endian host-memory snapshots, so lanes are
//! decoded from bytes and no alignment is assumed. NaNs compare equal
//! regardless of payload and `-0.0` equals `0.0`: a lane diverges only when the
//! two values are numerically further apart than the allowed ULP distance.

/// A diverging `f32` lane: its index within the region and both raw bit
/// patterns (ours first, the original's second).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaneDiff {
    pub lane: usize,
    pub ours: u32,
    pub orig: u32,
}

/// Map `f32` bits onto a monotonically ordered integer line so the distance
/// between two values is their ULP separation. Both zero encodings map to 0,
/// negatives mirror below it.
fn ordered(bits: u32) -> i64 {
    let magnitude = i64::from(bits & 0x7fff_ffff);
    if bits & 0x8000_0000 == 0 {
        magnitude
    } else {
        -magnitude
    }
}

/// Whether two `f32`s are within `max_ulp` units in the last place of each
/// other. Any two NaNs are equal; a NaN never equals a non-NaN; `-0.0 == 0.0`.
pub fn f32_within_ulp(a: f32, b: f32, max_ulp: u32) -> bool {
    if a.is_nan() || b.is_nan() {
        return a.is_nan() && b.is_nan();
    }
    ordered(a.to_bits()).abs_diff(ordered(b.to_bits())) <= u64::from(max_ulp)
}

/// First diverging `f32` lane between two equal-length regions, or `None` when
/// every lane is within `max_ulp`. Trailing bytes past the last whole lane are
/// ignored (region lengths are validated to whole lanes at build time).
pub fn first_divergence_f32(ours: &[u8], orig: &[u8], max_ulp: u32) -> Option<LaneDiff> {
    let (our_lanes, _) = ours.as_chunks::<4>();
    let (orig_lanes, _) = orig.as_chunks::<4>();
    for (lane, (o, g)) in our_lanes.iter().zip(orig_lanes).enumerate() {
        let ours = u32::from_le_bytes(*o);
        let orig = u32::from_le_bytes(*g);
        if ours == orig {
            continue;
        }
        if !f32_within_ulp(f32::from_bits(ours), f32::from_bits(orig), max_ulp) {
            return Some(LaneDiff { lane, ours, orig });
        }
    }
    None
}

/// First diverging byte offset between two regions compared exactly.
pub fn first_divergence_bytes(ours: &[u8], orig: &[u8]) -> Option<usize> {
    ours.iter().zip(orig).position(|(a, b)| a != b)
}

#[cfg(test)]
mod tests {
    use super::{LaneDiff, f32_within_ulp, first_divergence_bytes, first_divergence_f32};

    #[test]
    fn nans_compare_equal_regardless_of_payload() {
        let quiet = f32::from_bits(0x7fc0_0000);
        let payload = f32::from_bits(0xffc0_0001);
        assert!(f32_within_ulp(quiet, payload, 0));
        assert!(!f32_within_ulp(quiet, 1.0, u32::MAX));
    }

    #[test]
    fn signed_zeros_compare_equal() {
        assert!(f32_within_ulp(0.0, -0.0, 0));
        assert!(f32_within_ulp(-0.0, 0.0, 0));
    }

    #[test]
    fn ulp_boundary_is_inclusive() {
        let next = f32::from_bits(1.0f32.to_bits() + 1);
        assert!(f32_within_ulp(1.0, next, 1));
        assert!(!f32_within_ulp(1.0, next, 0));
    }

    #[test]
    fn ulp_distance_crosses_zero() {
        let tiny_pos = f32::from_bits(1);
        let tiny_neg = f32::from_bits(0x8000_0001);
        assert!(f32_within_ulp(tiny_pos, tiny_neg, 2));
        assert!(!f32_within_ulp(tiny_pos, tiny_neg, 1));
    }

    #[test]
    fn region_reports_first_diverging_lane() {
        let base = [1.0f32, 2.0, 3.0];
        let mut ours = [0u8; 12];
        let mut orig = [0u8; 12];
        for (i, v) in base.iter().enumerate() {
            ours[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
            orig[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        assert_eq!(first_divergence_f32(&ours, &orig, 0), None);

        orig[8..12].copy_from_slice(&4.0f32.to_le_bytes());
        assert_eq!(
            first_divergence_f32(&ours, &orig, 4),
            Some(LaneDiff {
                lane: 2,
                ours: 3.0f32.to_bits(),
                orig: 4.0f32.to_bits(),
            })
        );
    }

    #[test]
    fn region_tolerates_within_ulp_lanes() {
        let ours = 1.0f32.to_le_bytes();
        let orig = f32::from_bits(1.0f32.to_bits() + 2).to_le_bytes();
        assert_eq!(first_divergence_f32(&ours, &orig, 2), None);
        assert!(first_divergence_f32(&ours, &orig, 1).is_some());
    }

    #[test]
    fn bytes_mode_is_exact() {
        assert_eq!(first_divergence_bytes(&[1, 2, 3], &[1, 2, 3]), None);
        assert_eq!(first_divergence_bytes(&[1, 2, 3], &[1, 9, 3]), Some(1));
    }
}
