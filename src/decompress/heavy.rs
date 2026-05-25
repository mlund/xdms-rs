//! HEAVY1/HEAVY2 — LZH (Lempel-Ziv + Huffman), ported from `u_heavy.c` and the
//! LHA-derived table builder in `maketbl.c`.
//!
//! A character code under 256 is a literal; 256 and up is a match length, paired
//! with a position decoded from a second Huffman tree. HEAVY1 uses a 4 KB
//! dictionary, HEAVY2 an 8 KB one. The Huffman trees are rebuilt from the stream
//! only when the track's rebuild flag is set; otherwise the previous track's
//! tables are reused.

use super::{copy_match, push_window, Corrupt, Decompressor, HEAVY_NC, HEAVY_NPT};
use crate::bitreader::BitReader;

/// Character codes below this are literals; the rest index internal tree nodes.
const N1: u16 = HEAVY_NC as u16;
/// Match lengths are `code - OFFSET`: 253 = 256 - 3, the minimum match length
/// (so code 256 means length 3).
const OFFSET: u16 = 253;

impl Decompressor {
    /// Decodes a HEAVY-compressed track into `out` (length = the first-stage
    /// size). `big_dict` selects HEAVY2's 8 KB window over HEAVY1's 4 KB;
    /// `rebuild` rebuilds the Huffman trees from this track's stream.
    pub(super) fn unpack_heavy(
        &mut self,
        big_dict: bool,
        rebuild: bool,
        packed: &[u8],
        out: &mut [u8],
    ) -> Result<(), Corrupt> {
        // np = position-tree code count = log2(window) + 2 (HEAVY1: 4 KB -> 14,
        // HEAVY2: 8 KB -> 15); mask is window_size - 1.
        let (np, mask): (u16, u16) = if big_dict { (15, 0x1fff) } else { (14, 0x0fff) };
        let mut bits = BitReader::new(packed);

        if rebuild {
            self.read_tree_c(&mut bits)?;
            self.read_tree_p(&mut bits, np)?;
        }

        let mut pos = 0;
        while pos < out.len() {
            let code = self.decode_c(&mut bits);
            if code < 256 {
                let byte = code as u8;
                push_window(&mut self.window[..], &mut self.heavy_pos, mask, byte);
                out[pos] = byte;
                pos += 1;
            } else {
                let length = code - OFFSET;
                let distance = self.decode_p(&mut bits, np);
                copy_match(
                    &mut self.window[..],
                    &mut self.heavy_pos,
                    mask,
                    distance,
                    length,
                    out,
                    &mut pos,
                )?;
            }
        }
        Ok(())
    }

    /// Decodes one character/length code (a 12-bit table lookup, falling back to
    /// a tree walk for longer codes).
    fn decode_c(&self, bits: &mut BitReader) -> u16 {
        let mut node = self.c_table[bits.peek(12) as usize];
        if node < N1 {
            bits.consume(u32::from(self.c_len[node as usize]));
        } else {
            bits.consume(12);
            let path = bits.peek(16);
            let mut probe = 0x8000u16;
            loop {
                node = if path & probe != 0 {
                    self.right[node as usize]
                } else {
                    self.left[node as usize]
                };
                probe >>= 1;
                if node < N1 {
                    break;
                }
            }
            bits.consume(u32::from(self.c_len[node as usize]) - 12);
        }
        node
    }

    /// Decodes one match position. Codes shorter than `np-1` extend with raw
    /// bits; the special last code reuses the previous distance (the C
    /// `heavy_lastlen`).
    fn decode_p(&mut self, bits: &mut BitReader, np: u16) -> u16 {
        let mut node = self.pt_table[bits.peek(8) as usize];
        if node < np {
            bits.consume(u32::from(self.pt_len[node as usize]));
        } else {
            bits.consume(8);
            let path = bits.peek(16);
            let mut probe = 0x8000u16;
            loop {
                node = if path & probe != 0 {
                    self.right[node as usize]
                } else {
                    self.left[node as usize]
                };
                probe >>= 1;
                if node < np {
                    break;
                }
            }
            bits.consume(u32::from(self.pt_len[node as usize]) - 8);
        }

        if node != np - 1 {
            if node > 0 {
                let extra = node - 1;
                node = bits.peek(u32::from(extra)) | (1 << extra);
                bits.consume(u32::from(extra));
            }
            self.heavy_last_match_len = node;
        }
        self.heavy_last_match_len
    }

    /// Reads the character/length Huffman tree from the stream and builds its
    /// decode table.
    fn read_tree_c(&mut self, bits: &mut BitReader) -> Result<(), Corrupt> {
        let count = bits.read(9);
        if count > 0 {
            if count as usize > HEAVY_NC {
                return Err(Corrupt);
            }
            for i in 0..count as usize {
                self.c_len[i] = bits.read(5) as u8;
            }
            self.c_len[count as usize..].fill(0);
            make_table(
                self.left.as_mut_slice(),
                self.right.as_mut_slice(),
                N1,
                &self.c_len,
                12,
                self.c_table.as_mut_slice(),
            )
        } else {
            // No tree: every code maps to the single symbol that follows.
            let symbol = bits.read(9);
            self.c_len.fill(0);
            self.c_table.fill(symbol);
            Ok(())
        }
    }

    /// Reads the position Huffman tree from the stream and builds its table.
    fn read_tree_p(&mut self, bits: &mut BitReader, np: u16) -> Result<(), Corrupt> {
        let count = bits.read(5);
        if count > 0 {
            if count as usize > HEAVY_NPT {
                return Err(Corrupt);
            }
            for i in 0..count as usize {
                self.pt_len[i] = bits.read(4) as u8;
            }
            self.pt_len[count as usize..].fill(0);
            make_table(
                self.left.as_mut_slice(),
                self.right.as_mut_slice(),
                np,
                &self.pt_len,
                8,
                &mut self.pt_table,
            )
        } else {
            let symbol = bits.read(5);
            self.pt_len.fill(0);
            self.pt_table.fill(symbol);
            Ok(())
        }
    }
}

/// Builds an LHA-style Huffman decode `table` from per-symbol bit lengths.
///
/// Codes no longer than `tablebits` resolve in a single lookup; longer codes are
/// represented as a binary tree threaded through `left`/`right`, whose nodes the
/// decoder walks bit by bit.
fn make_table(
    left: &mut [u16],
    right: &mut [u16],
    nchar: u16,
    bitlen: &[u8],
    tablebits: u16,
    table: &mut [u16],
) -> Result<(), Corrupt> {
    let table_size = 1u16 << tablebits;
    let mut builder = TableBuilder {
        blen: bitlen,
        table,
        left,
        right,
        n: nchar,
        avail: nchar,
        table_size,
        bit: table_size >> 1,
        max_depth: tablebits + 1,
        depth: 1,
        len: 1,
        c: -1,
        codeword: 0,
    };
    builder.build()?; // left subtree
    builder.build()?; // right subtree
    if builder.codeword != table_size {
        return Err(Corrupt);
    }
    Ok(())
}

/// Recursive state for [`make_table`] (the C carried this in file-scope statics).
struct TableBuilder<'a> {
    blen: &'a [u8],
    table: &'a mut [u16],
    left: &'a mut [u16],
    right: &'a mut [u16],
    n: u16,
    avail: u16,
    table_size: u16,
    bit: u16,
    max_depth: u16,
    depth: u16,
    len: u16,
    c: i32,
    codeword: u16,
}

impl TableBuilder<'_> {
    /// Visits one node of the canonical-Huffman walk, filling table entries for
    /// short codes and threading `left`/`right` for long ones. Returns the node
    /// value (a symbol or an internal-node index).
    fn build(&mut self) -> Result<u16, Corrupt> {
        let mut node = 0u16;
        if self.len == self.depth {
            // At the current code length, assign table slots to matching symbols.
            loop {
                self.c += 1;
                if self.c >= i32::from(self.n) {
                    break;
                }
                if self.blen[self.c as usize] == self.len as u8 {
                    let start = self.codeword;
                    self.codeword += self.bit;
                    if self.codeword > self.table_size {
                        return Err(Corrupt);
                    }
                    self.table[start as usize..self.codeword as usize].fill(self.c as u16);
                    return Ok(self.c as u16);
                }
            }
            self.c = -1;
            self.len += 1;
            self.bit >>= 1;
        }

        self.depth += 1;
        if self.depth < self.max_depth {
            self.build()?;
            self.build()?;
        } else if self.depth > 32 {
            return Err(Corrupt);
        } else {
            node = self.avail;
            self.avail += 1;
            if node >= 2 * self.n - 1 {
                return Err(Corrupt);
            }
            let l = self.build()?;
            self.left[node as usize] = l;
            let r = self.build()?;
            self.right[node as usize] = r;
            if self.codeword >= self.table_size {
                return Err(Corrupt);
            }
            if self.depth == self.max_depth {
                self.table[self.codeword as usize] = node;
                self.codeword += 1;
            }
        }
        self.depth -= 1;
        Ok(node)
    }
}
