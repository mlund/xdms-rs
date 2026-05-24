//! DEEP — LZ with an *adaptive* Huffman tree for the character/length symbol
//! (ported from `u_deep.c`, itself from Okumura's LZHUF). The tree's frequencies
//! update after every symbol and the tree is periodically rebuilt, so this state
//! persists across tracks until reset. Match distances use the static
//! [`D_CODE`]/[`D_LEN`] tables.

use super::tables::{D_CODE, D_LEN};
use super::{Corrupt, Decompressor, DEEP_N_CHAR as N_CHAR, DEEP_T as T};
use crate::bitreader::BitReader;

/// Root node of the Huffman tree.
const R: usize = T - 1;
/// The tree is rebuilt once a frequency reaches this.
const MAX_FREQ: u16 = 0x8000;
/// DEEP's window is 16 KB.
const MASK: u16 = 0x3fff;
/// Match lengths are `code - 253` (code 256 -> length 3), as in the C.
const LENGTH_BIAS: u16 = 253;

impl Decompressor {
    /// Decodes a DEEP-compressed track into `out` (the first-stage size).
    pub(super) fn unpack_deep(&mut self, packed: &[u8], out: &mut [u8]) -> Result<(), Corrupt> {
        let mut bits = BitReader::new(packed);
        if self.deep_init {
            self.init_deep_tree();
        }
        let mut pos = 0;
        while pos < out.len() {
            let code = self.decode_char(&mut bits);
            if code < 256 {
                let byte = code as u8;
                self.push_deep(byte);
                out[pos] = byte;
                pos += 1;
            } else {
                let length = code - LENGTH_BIAS;
                let distance = decode_deep_distance(&mut bits);
                let mut src = self.deep_pos.wrapping_sub(distance).wrapping_sub(1);
                for _ in 0..length {
                    let byte = self.window[(src & MASK) as usize];
                    self.push_deep(byte);
                    src = src.wrapping_add(1);
                    *out.get_mut(pos).ok_or(Corrupt)? = byte;
                    pos += 1;
                }
            }
        }
        self.deep_pos = self.deep_pos.wrapping_add(60) & MASK;
        Ok(())
    }

    fn push_deep(&mut self, byte: u8) {
        self.window[(self.deep_pos & MASK) as usize] = byte;
        self.deep_pos = self.deep_pos.wrapping_add(1);
    }

    /// Builds the initial balanced Huffman tree (the C `Init_DEEP_Tabs`).
    fn init_deep_tree(&mut self) {
        for i in 0..N_CHAR {
            self.deep_freq[i] = 1;
            self.deep_son[i] = (i + T) as u16;
            self.deep_prnt[i + T] = i as u16;
        }
        let mut i = 0;
        let mut j = N_CHAR;
        while j <= R {
            self.deep_freq[j] = self.deep_freq[i].wrapping_add(self.deep_freq[i + 1]);
            self.deep_son[j] = i as u16;
            self.deep_prnt[i] = j as u16;
            self.deep_prnt[i + 1] = j as u16;
            i += 2;
            j += 1;
        }
        self.deep_freq[T] = 0xffff;
        self.deep_prnt[R] = 0;
        self.deep_init = false;
    }

    /// Walks the tree from the root, one bit per step, to a leaf symbol, then
    /// updates frequencies.
    fn decode_char(&mut self, bits: &mut BitReader) -> u16 {
        let mut node = self.deep_son[R];
        while (node as usize) < T {
            node = self.deep_son[(node + bits.read(1)) as usize];
        }
        let symbol = node - T as u16;
        self.update(symbol);
        symbol
    }

    /// Increments a symbol's frequency and restores the sibling ordering,
    /// rebuilding the tree first if a frequency would overflow.
    fn update(&mut self, symbol: u16) {
        if self.deep_freq[R] == MAX_FREQ {
            self.reconstruct();
        }
        let mut c = self.deep_prnt[symbol as usize + T];
        loop {
            self.deep_freq[c as usize] = self.deep_freq[c as usize].wrapping_add(1);
            let k = self.deep_freq[c as usize];

            // If this node now outweighs the next, swap it up the ordering.
            let mut l = c + 1;
            if k > self.deep_freq[l as usize] {
                loop {
                    l += 1;
                    if k <= self.deep_freq[l as usize] {
                        break;
                    }
                }
                l -= 1;
                self.deep_freq[c as usize] = self.deep_freq[l as usize];
                self.deep_freq[l as usize] = k;

                let i = self.deep_son[c as usize];
                self.deep_prnt[i as usize] = l;
                if (i as usize) < T {
                    self.deep_prnt[i as usize + 1] = l;
                }

                let j = self.deep_son[l as usize];
                self.deep_son[l as usize] = i;
                self.deep_prnt[j as usize] = c;
                if (j as usize) < T {
                    self.deep_prnt[j as usize + 1] = c;
                }
                self.deep_son[c as usize] = j;

                c = l;
            }

            c = self.deep_prnt[c as usize];
            if c == 0 {
                break;
            }
        }
    }

    /// Halves all frequencies and rebuilds the tree (the C `reconst`).
    fn reconstruct(&mut self) {
        // Gather leaves into the low half, halving their (rounded-up) frequency.
        let mut j = 0;
        for i in 0..T {
            if self.deep_son[i] as usize >= T {
                self.deep_freq[j] = self.deep_freq[i].wrapping_add(1) / 2;
                self.deep_son[j] = self.deep_son[i];
                j += 1;
            }
        }
        // Rebuild internal nodes, keeping freq[] sorted by insertion.
        let mut i = 0;
        let mut j = N_CHAR;
        while j < T {
            let f = self.deep_freq[i].wrapping_add(self.deep_freq[i + 1]);
            self.deep_freq[j] = f;
            let mut k = j - 1;
            while f < self.deep_freq[k] {
                k -= 1;
            }
            k += 1;
            self.deep_freq.copy_within(k..j, k + 1);
            self.deep_freq[k] = f;
            self.deep_son.copy_within(k..j, k + 1);
            self.deep_son[k] = i as u16;
            i += 2;
            j += 1;
        }
        // Re-link parents.
        for i in 0..T {
            let k = self.deep_son[i] as usize;
            self.deep_prnt[k] = i as u16;
            if k < T {
                self.deep_prnt[k + 1] = i as u16;
            }
        }
    }
}

/// Decodes a match distance from the static tables (the C `DecodePosition`).
fn decode_deep_distance(bits: &mut BitReader) -> u16 {
    let prefix = bits.read(8) as usize;
    let high = u16::from(D_CODE[prefix]) << 8;
    let extra = u32::from(D_LEN[prefix]);
    let low = (((prefix as u32) << extra) | u32::from(bits.read(extra))) & 0xff;
    high | low as u16
}
