//! MEDIUM — LZ with static-Huffman match distances (ported from `u_medium.c`).
//! Literals are a flag bit plus 8 raw bits; matches decode a distance through the
//! two-level [`D_CODE`]/[`D_LEN`] tables into a 16 KB window.

use super::tables::{D_CODE, D_LEN};
use super::{copy_match, push_window, Corrupt, Decompressor};
use crate::bitreader::BitReader;

/// MEDIUM's window is 16 KB.
const MASK: u16 = 0x3fff;

impl Decompressor {
    /// Decodes a MEDIUM-compressed track into `out` (the first-stage size).
    pub(super) fn unpack_medium(&mut self, packed: &[u8], out: &mut [u8]) -> Result<(), Corrupt> {
        let mut bits = BitReader::new(packed);
        let mut pos = 0;
        while pos < out.len() {
            if bits.read(1) != 0 {
                let byte = bits.read(8) as u8;
                push_window(&mut self.window[..], &mut self.medium_pos, MASK, byte);
                out[pos] = byte;
                pos += 1;
            } else {
                let prefix = bits.read(8) as usize;
                let length = u16::from(D_CODE[prefix]) + 3;
                let distance = decode_medium_distance(&mut bits, prefix);
                copy_match(
                    &mut self.window[..],
                    &mut self.medium_pos,
                    MASK,
                    distance,
                    length,
                    out,
                    &mut pos,
                )?;
            }
        }
        // The C nudges the window position by 66 between tracks.
        self.medium_pos = self.medium_pos.wrapping_add(66) & MASK;
        Ok(())
    }
}

/// Two-level table decode of a match distance from an 8-bit `prefix`.
fn decode_medium_distance(bits: &mut BitReader, prefix: usize) -> u16 {
    let extra = u32::from(D_LEN[prefix]);
    let mid = ((((prefix as u32) << extra) | u32::from(bits.read(extra))) & 0xff) as usize;
    let extra = u32::from(D_LEN[mid]);
    let low = (((mid as u32) << extra) | u32::from(bits.read(extra))) & 0xff;
    ((u32::from(D_CODE[mid]) << 8) | low) as u16
}

#[cfg(test)]
mod tests {
    use super::Decompressor;
    use alloc::vec::Vec;

    /// Encode literals the way MEDIUM expects: flag bit `1` then 8 bits, MSB-first.
    fn encode_literals(bytes: &[u8]) -> Vec<u8> {
        let mut acc = 0u32;
        let mut nbits = 0u32;
        let mut out = Vec::new();
        let mut put = |value: u32, n: u32, out: &mut Vec<u8>| {
            acc = (acc << n) | (value & ((1 << n) - 1));
            nbits += n;
            while nbits >= 8 {
                nbits -= 8;
                out.push((acc >> nbits) as u8);
            }
        };
        for &b in bytes {
            put(1, 1, &mut out);
            put(u32::from(b), 8, &mut out);
        }
        if nbits > 0 {
            out.push((acc << (8 - nbits)) as u8);
        }
        out
    }

    #[test]
    fn round_trips_literals() {
        let data = b"Medium literal path";
        let packed = encode_literals(data);
        let mut d = Decompressor::new();
        let mut out = vec![0u8; data.len()];
        d.unpack_medium(&packed, &mut out).unwrap();
        assert_eq!(&out, data);
    }
}
