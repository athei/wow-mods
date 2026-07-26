//! Float-lane region comparison for differential mode.
//!
//! Compares a reimplementation's output region against the original's, lane by
//! lane. Regions are raw little-endian host-memory snapshots, so lanes are
//! decoded from bytes and no alignment is assumed. NaNs compare equal
//! regardless of payload and `-0.0` equals `0.0`: a lane diverges only when the
//! two values are numerically further apart than the allowed ULP distance.

/// A diverging `f32` lane.
///
/// Its index within the region and both raw bit patterns (ours first, the
/// original's second).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaneDiff {
    pub lane: usize,
    pub ours: u32,
    pub orig: u32,
}

/// Map `f32` bits onto a monotonically ordered integer line.
///
/// The distance between two values is their ULP separation. Both zero
/// encodings map to 0, negatives mirror below it.
fn ordered(bits: u32) -> i64 {
    let magnitude = i64::from(bits & 0x7fff_ffff);
    if bits & 0x8000_0000 == 0 {
        magnitude
    } else {
        -magnitude
    }
}

/// Whether two `f32`s are within `max_ulp` units in the last place of each other.
///
/// Any two NaNs are equal; a NaN never equals a non-NaN; `-0.0 == 0.0`.
pub fn f32_within_ulp(a: f32, b: f32, max_ulp: u32) -> bool {
    if a.is_nan() || b.is_nan() {
        return a.is_nan() && b.is_nan();
    }
    ordered(a.to_bits()).abs_diff(ordered(b.to_bits())) <= u64::from(max_ulp)
}

/// First diverging `f32` lane between two equal-length regions.
///
/// `None` when every lane is within `max_ulp` or `max_abs`. Trailing bytes past
/// the last whole lane are ignored (region lengths are validated to whole lanes at
/// build time).
// The absolute escape is written `!(diff <= max_abs)` rather than
// `diff > max_abs` because the two differ on NaN, and the difference decides
// whether a NaN lane is reported. `NaN <= max_abs` is false, so the negated form
// yields true and the lane is reported; `NaN > max_abs` is also false, which would
// silence it. A comparator that hides a NaN divergence is worse than no
// comparator, so the negation is load-bearing.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn first_divergence_f32(
    ours: &[u8],
    orig: &[u8],
    max_ulp: u32,
    max_abs: f32,
) -> Option<LaneDiff> {
    let (our_lanes, _) = ours.as_chunks::<4>();
    let (orig_lanes, _) = orig.as_chunks::<4>();
    for (lane, (o, g)) in our_lanes.iter().zip(orig_lanes).enumerate() {
        let ours = u32::from_le_bytes(*o);
        let orig = u32::from_le_bytes(*g);
        if ours == orig {
            continue;
        }
        let (o, g) = (f32::from_bits(ours), f32::from_bits(orig));
        // ULP is the wrong ruler for a lane produced by cancellation: as the true
        // value approaches zero the ULP distance grows without bound while the
        // absolute error stays put. `max_abs` is the escape, and a lane has to
        // exceed BOTH to count, so neither ruler alone can hide a difference.
        if !f32_within_ulp(o, g, max_ulp) && !((o - g).abs() <= max_abs) {
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
        assert_eq!(first_divergence_f32(&ours, &orig, 0, 0.0), None);

        orig[8..12].copy_from_slice(&4.0f32.to_le_bytes());
        assert_eq!(
            first_divergence_f32(&ours, &orig, 4, 0.0),
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
        assert_eq!(first_divergence_f32(&ours, &orig, 2, 0.0), None);
        assert!(first_divergence_f32(&ours, &orig, 1, 0.0).is_some());
    }

    #[test]
    fn bytes_mode_is_exact() {
        assert_eq!(first_divergence_bytes(&[1, 2, 3], &[1, 2, 3]), None);
        assert_eq!(first_divergence_bytes(&[1, 2, 3], &[1, 9, 3]), Some(1));
    }
}
