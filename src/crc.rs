//! Integrity primitives used throughout the DMS format: a CRC-16 over header and
//! packed track data, and a plain 16-bit additive checksum over unpacked data.

/// Reflected CRC-16/ARC lookup table.
///
/// Generated rather than transcribed: the original C carried a 256-entry literal
/// table, but deriving it from the polynomial removes any chance of a copy error
/// and is verified end-to-end by the ARC check value in the tests.
const CRC16_TABLE: [u16; 256] = {
    // Reflected CRC-16/IBM (a.k.a. ARC) polynomial.
    const POLY: u16 = 0xA001;
    let mut table = [0u16; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut crc = i as u16;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ POLY
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

/// CRC-16 over `data`, as DMS computes it for the archive header, each track
/// header, and each track's packed payload (init 0, reflected, no final xor).
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &byte in data {
        crc = CRC16_TABLE[usize::from((crc ^ u16::from(byte)) & 0xff)] ^ (crc >> 8);
    }
    crc
}

/// 16-bit additive checksum over `data` (sum of bytes, wrapping at 2^16). DMS
/// stores this for the *unpacked* track data to catch decompression errors.
pub fn checksum16(data: &[u8]) -> u16 {
    data.iter()
        .fold(0u16, |sum, &byte| sum.wrapping_add(u16::from(byte)))
}

#[cfg(test)]
mod tests {
    use super::{checksum16, crc16};

    #[test]
    fn crc16_matches_arc_check_value() {
        // DMS uses CRC-16/ARC (reflected, poly 0xA001, init 0); its canonical
        // check value over b"123456789" is 0xBB3D.
        assert_eq!(crc16(b"123456789"), 0xBB3D);
    }

    #[test]
    fn crc16_of_empty_is_zero() {
        assert_eq!(crc16(b""), 0);
    }

    #[test]
    fn checksum16_is_byte_sum_modulo_65536() {
        assert_eq!(checksum16(b"123456789"), 477);
        // Wraps at 16 bits, matching the C `USHORT` accumulator.
        assert_eq!(checksum16(&[0xff; 300]), (255u32 * 300 % 0x10000) as u16);
    }
}
