//! QUICK — a small-window LZ (ported from `u_quick.c`). One flag bit picks
//! between a literal byte and a back-reference into a 256-byte window.

use super::{Corrupt, Decompressor};
use crate::bitreader::BitReader;

/// QUICK's window is 256 bytes.
const MASK: u16 = 0xff;

impl Decompressor {
    /// Decodes a QUICK-compressed track into `out` (length = the first-stage
    /// size, before the RLE pass).
    pub(super) fn unpack_quick(&mut self, packed: &[u8], out: &mut [u8]) -> Result<(), Corrupt> {
        let mut bits = BitReader::new(packed);
        let mut pos = 0;
        while pos < out.len() {
            if bits.read(1) != 0 {
                let byte = bits.read(8) as u8;
                self.push_quick(byte);
                out[pos] = byte;
                pos += 1;
            } else {
                let length = u32::from(bits.read(2)) + 2;
                let distance = bits.read(8);
                let mut src = self.quick_pos.wrapping_sub(distance).wrapping_sub(1);
                for _ in 0..length {
                    let byte = self.window[(src & MASK) as usize];
                    self.push_quick(byte);
                    src = src.wrapping_add(1);
                    *out.get_mut(pos).ok_or(Corrupt)? = byte;
                    pos += 1;
                }
            }
        }
        // The C nudges the window position by 5 between tracks.
        self.quick_pos = self.quick_pos.wrapping_add(5) & MASK;
        Ok(())
    }

    fn push_quick(&mut self, byte: u8) {
        self.window[(self.quick_pos & MASK) as usize] = byte;
        self.quick_pos = self.quick_pos.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::Decompressor;
    use alloc::vec;

    /// Minimal QUICK encoder for tests: literal = `1` + 8 bits, match = `0` +
    /// 2-bit (len-2) + 8-bit distance. Bits are packed MSB-first to match the
    /// reader.
    #[derive(Default)]
    struct BitWriter {
        out: alloc::vec::Vec<u8>,
        acc: u32,
        nbits: u32,
    }
    impl BitWriter {
        fn put(&mut self, value: u32, n: u32) {
            self.acc = (self.acc << n) | (value & ((1 << n) - 1));
            self.nbits += n;
            while self.nbits >= 8 {
                self.nbits -= 8;
                self.out.push((self.acc >> self.nbits) as u8);
            }
        }
        fn finish(mut self) -> alloc::vec::Vec<u8> {
            if self.nbits > 0 {
                self.out.push((self.acc << (8 - self.nbits)) as u8);
            }
            self.out
        }
    }

    #[test]
    fn round_trips_literals_and_a_match() {
        // Encode "AB" as literals, then a length-3 match at distance 2 -> "ABA".
        let mut w = BitWriter::default();
        for byte in [b'A', b'B'] {
            w.put(1, 1);
            w.put(u32::from(byte), 8);
        }
        w.put(0, 1); // match
        w.put(3 - 2, 2); // length 3
        w.put(2 - 1, 8); // distance: i = pos - dist - 1, so dist field = 1 references 'A'
        let packed = w.finish();

        let mut d = Decompressor::new();
        let mut out = vec![0u8; 5];
        d.unpack_quick(&packed, &mut out).unwrap();
        assert_eq!(&out, b"ABABA");
    }
}
