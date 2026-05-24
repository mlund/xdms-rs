//! MSB-first bit reader over a byte slice, mirroring the C `getbits` macros.
//!
//! Huffman decoding needs to *peek* a fixed number of bits, look them up, then
//! consume only as many as the matched code used — so peek and consume are
//! separate operations, not a single `read`.

/// Reads up to 16 bits at a time, most-significant bit first, from a byte slice.
///
/// `buf` always holds at least 16 valid low bits after construction or any
/// `consume`, so [`peek`](Self::peek) of up to 16 bits never underflows.
pub struct BitReader<'a> {
    data: &'a [u8],
    /// Index of the next byte to pull into `buf`.
    pos: usize,
    /// Valid bits live in the low `count` bits of `buf`; higher bits are zero.
    buf: u32,
    count: u32,
}

impl<'a> BitReader<'a> {
    /// Creates a reader positioned at the first bit of `data`.
    pub fn new(data: &'a [u8]) -> Self {
        let mut reader = Self {
            data,
            pos: 0,
            buf: 0,
            count: 0,
        };
        reader.refill();
        reader
    }

    /// Pulls the next byte, or zero once past the end — the C reader runs into
    /// the zeroed tail of its buffer, and decoders rely on that padding.
    fn next_byte(&mut self) -> u32 {
        let byte = self.data.get(self.pos).copied().unwrap_or(0);
        self.pos += 1;
        u32::from(byte)
    }

    /// Tops `buf` back up to at least 16 valid bits (the C `DROPBITS` refill).
    fn refill(&mut self) {
        while self.count < 16 {
            self.buf = (self.buf << 8) | self.next_byte();
            self.count += 8;
        }
    }

    /// Returns the next `n` bits (`1..=16`) without consuming them.
    pub const fn peek(&self, n: u32) -> u16 {
        (self.buf >> (self.count - n)) as u16
    }

    /// Discards the next `n` bits and refills the buffer.
    pub fn consume(&mut self, n: u32) {
        self.count -= n;
        self.buf &= (1u32 << self.count) - 1;
        self.refill();
    }

    /// Reads and consumes the next `n` bits (`1..=16`).
    pub fn read(&mut self, n: u32) -> u16 {
        let bits = self.peek(n);
        self.consume(n);
        bits
    }
}

#[cfg(test)]
mod tests {
    use super::BitReader;

    #[test]
    fn reads_bits_msb_first() {
        let mut r = BitReader::new(&[0xA5, 0xC3, 0x0F, 0xF0]);
        assert_eq!(r.read(4), 0xA);
        assert_eq!(r.read(8), 0x5C);
        assert_eq!(r.read(12), 0x30F);
    }

    #[test]
    fn peek_does_not_consume() {
        let mut r = BitReader::new(&[0xA5, 0xC3]);
        assert_eq!(r.peek(4), 0xA);
        assert_eq!(r.peek(4), 0xA);
        r.consume(4);
        assert_eq!(r.peek(4), 0x5);
    }

    #[test]
    fn yields_zero_past_end() {
        // The C reader runs off the packed data into the zeroed buffer tail; we
        // reproduce that by zero-padding past the slice end.
        let mut r = BitReader::new(&[0xFF]);
        assert_eq!(r.read(8), 0xFF);
        assert_eq!(r.read(8), 0x00);
        assert_eq!(r.read(16), 0x0000);
    }
}
