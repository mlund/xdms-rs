//! Per-mode decompressors and the shared sliding-window state they carry.
//!
//! Most modes are two-stage: a mode-specific LZ/LZH/Huffman pass followed by an
//! RLE pass (see [`rle`]). The C kept the window and bit-reader in globals; here
//! that state lives in a single [`Decompressor`] that the drive loop reuses
//! across tracks (DMS lets a track continue from the previous track's state).

mod rle;

use alloc::boxed::Box;

use crate::header::Mode;

/// Usable size of the sliding window. The widest mode (MEDIUM/DEEP) masks
/// positions to `0x3fff`, so indices never reach `0x4000`.
const WINDOW_SIZE: usize = 0x4000;
/// Bytes cleared on reset, matching the C `Init_Decrunchers` (which deliberately
/// leaves the tail untouched between tracks).
const WINDOW_CLEAR: usize = 0x3fc8;
const QUICK_INIT_POS: u16 = 251;
const MEDIUM_INIT_POS: u16 = 0x3fbe;
const DEEP_INIT_POS: u16 = 0x3fc4;

/// Returned by a decompressor when the compressed stream is invalid (truncated,
/// corrupt, or decrypted with the wrong password). The drive loop attaches the
/// track number to turn this into [`crate::Error::BadData`].
#[derive(Debug)]
pub struct Corrupt;

/// Holds the sliding window and per-mode positions that persist across tracks.
pub struct Decompressor {
    window: Box<[u8; WINDOW_SIZE]>,
    quick_pos: u16,
    medium_pos: u16,
    deep_pos: u16,
    heavy_pos: u16,
    heavy_last_match_len: u16,
}

impl Decompressor {
    /// Creates a decompressor with freshly initialised state.
    pub fn new() -> Self {
        let mut decompressor = Self {
            window: Box::new([0u8; WINDOW_SIZE]),
            quick_pos: 0,
            medium_pos: 0,
            deep_pos: 0,
            heavy_pos: 0,
            heavy_last_match_len: 0,
        };
        decompressor.reset();
        decompressor
    }

    /// Reinitialises window positions and clears the window (the C
    /// `Init_Decrunchers`). The drive loop calls this between tracks unless the
    /// track asks to keep state.
    pub fn reset(&mut self) {
        self.quick_pos = QUICK_INIT_POS;
        self.medium_pos = MEDIUM_INIT_POS;
        self.deep_pos = DEEP_INIT_POS;
        self.heavy_pos = 0;
        self.heavy_last_match_len = 0;
        self.window[..WINDOW_CLEAR].fill(0);
    }

    /// Decodes one track's `packed` bytes into `out` (whose length is the track's
    /// unpacked length). State carries over to the next call unless the caller
    /// resets in between.
    // `&mut self` is required once the stateful modes (QUICK/MEDIUM/DEEP/HEAVY)
    // land — they advance the window and positions. None/Simple don't, so clippy
    // can't yet see the mutation.
    #[allow(clippy::needless_pass_by_ref_mut)]
    pub fn unpack_track(
        &mut self,
        mode: Mode,
        packed: &[u8],
        out: &mut [u8],
    ) -> Result<(), Corrupt> {
        match mode {
            Mode::None => {
                let src = packed.get(..out.len()).ok_or(Corrupt)?;
                out.copy_from_slice(src);
                Ok(())
            }
            Mode::Simple => rle::unpack_rle(packed, out),
            // QUICK/MEDIUM/DEEP/HEAVY land in subsequent commits.
            _ => Err(Corrupt),
        }
    }
}

impl Default for Decompressor {
    fn default() -> Self {
        Self::new()
    }
}
