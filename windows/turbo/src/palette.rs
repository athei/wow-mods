//! Bit-preserving packing of the three used columns of a bone matrix.

/// Transpose four three-word rows into three shader constant vectors.
///
/// Words carry float bits, including NaN payloads. The unused fourth column
/// never enters this operation, so packing performs no floating arithmetic.
pub const fn pack(rows: [[u32; 3]; 4]) -> [u32; 12] {
    [
        rows[0][0], rows[1][0], rows[2][0], rows[3][0], rows[0][1], rows[1][1], rows[2][1],
        rows[3][1], rows[0][2], rows[1][2], rows[2][2], rows[3][2],
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn packs_float_payloads_without_arithmetic() {
        let rows = [
            [0, 0x8000_0000, 0x7f80_0000],
            [0xff80_0000, 0x7fc0_0123, 0x7f80_0123],
            [1, 0xffff_ffff, 0x3f80_0000],
            [0x0080_0000, 0x8080_0000, 0x807f_ffff],
        ];
        let packed = super::pack(rows);
        for column in 0..3 {
            for row in 0..4 {
                assert_eq!(packed[column * 4 + row], rows[row][column]);
            }
        }
    }
}
