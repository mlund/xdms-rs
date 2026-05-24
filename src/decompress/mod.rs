//! Per-mode decompressors and the shared sliding-window state they carry.
//!
//! Most modes are two-stage: a mode-specific LZ/LZH/Huffman pass followed by an
//! RLE pass (see [`rle`]). The C kept the window and bit-reader in globals; here
//! that state lives in a single [`Decompressor`] that the drive loop reuses
//! across tracks (DMS lets a track continue from the previous track's state).

mod deep;
mod heavy;
mod medium;
mod quick;
mod rle;
mod tables;

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::header::{Mode, TrackFlags};

/// Usable size of the sliding window. The widest mode (MEDIUM/DEEP) masks
/// positions to `0x3fff`, so indices never reach `0x4000`.
const WINDOW_SIZE: usize = 0x4000;
/// Bytes cleared on reset, matching the C `Init_Decrunchers` (which deliberately
/// leaves the tail untouched between tracks).
const WINDOW_CLEAR: usize = 0x3fc8;
const QUICK_INIT_POS: u16 = 251;
const MEDIUM_INIT_POS: u16 = 0x3fbe;
const DEEP_INIT_POS: u16 = 0x3fc4;

/// Number of HEAVY character/length codes (the C `NC`).
const HEAVY_NC: usize = 510;
/// Number of HEAVY position-tree codes (the C `NPT`).
const HEAVY_NPT: usize = 20;

/// DEEP symbol count: 256 literals minus the threshold plus the lookahead size
/// (the C `N_CHAR = 256 - THRESHOLD + F`).
const DEEP_N_CHAR: usize = 256 - 2 + 60;
/// DEEP Huffman table size (the C `T = N_CHAR * 2 - 1`).
const DEEP_T: usize = DEEP_N_CHAR * 2 - 1;

/// Returned by a decompressor when the compressed stream is invalid (truncated,
/// corrupt, or decrypted with the wrong password). The drive loop attaches the
/// track number to turn this into [`crate::Error::BadData`].
#[derive(Debug)]
pub struct Corrupt;

/// Holds the sliding window and per-mode positions/tables that persist across
/// tracks. HEAVY's Huffman tables are rebuilt per track only when asked; DEEP's
/// adaptive tree evolves continuously.
pub struct Decompressor {
    window: Box<[u8; WINDOW_SIZE]>,
    quick_pos: u16,
    medium_pos: u16,
    deep_pos: u16,
    heavy_pos: u16,
    heavy_last_match_len: u16,
    // HEAVY Huffman state. `left`/`right` hold internal-node children and are
    // shared by the character and position trees (their index ranges don't
    // overlap, so both survive between decode_c/decode_p calls).
    c_len: [u8; HEAVY_NC],
    pt_len: [u8; HEAVY_NPT],
    c_table: Box<[u16; 4096]>,
    pt_table: [u16; 256],
    left: Box<[u16; 2 * HEAVY_NC - 1]>,
    right: Box<[u16; 2 * HEAVY_NC - 1 + 9]>,
    // DEEP adaptive-Huffman tree.
    deep_freq: Box<[u16; DEEP_T + 1]>,
    deep_prnt: Box<[u16; DEEP_T + DEEP_N_CHAR]>,
    deep_son: Box<[u16; DEEP_T]>,
    deep_init: bool,
    /// Reused first-stage buffer for the two-stage modes, so processing many
    /// tracks doesn't allocate one per track.
    scratch: Vec<u8>,
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
            c_len: [0; HEAVY_NC],
            pt_len: [0; HEAVY_NPT],
            c_table: Box::new([0; 4096]),
            pt_table: [0; 256],
            left: Box::new([0; 2 * HEAVY_NC - 1]),
            right: Box::new([0; 2 * HEAVY_NC - 1 + 9]),
            deep_freq: Box::new([0; DEEP_T + 1]),
            deep_prnt: Box::new([0; DEEP_T + DEEP_N_CHAR]),
            deep_son: Box::new([0; DEEP_T]),
            deep_init: true,
            scratch: Vec::new(),
        };
        decompressor.reset();
        decompressor
    }

    /// Reinitialises window positions and clears the window (the C
    /// `Init_Decrunchers`). The drive loop calls this between tracks unless the
    /// track asks to keep state. HEAVY's tables persist/rebuild per the track
    /// flags; DEEP's tree is flagged for rebuild here, as in the C.
    pub fn reset(&mut self) {
        self.quick_pos = QUICK_INIT_POS;
        self.medium_pos = MEDIUM_INIT_POS;
        self.deep_pos = DEEP_INIT_POS;
        self.heavy_pos = 0;
        self.heavy_last_match_len = 0;
        self.deep_init = true;
        self.window[..WINDOW_CLEAR].fill(0);
    }

    /// Decodes one track's `packed` bytes into `out` (whose length is the track's
    /// unpacked length). `intermediate_len` is the size after the first stage
    /// (the C `pklen2`). State carries over to the next call unless the caller
    /// resets in between.
    pub fn unpack_track(
        &mut self,
        mode: Mode,
        flags: TrackFlags,
        packed: &[u8],
        intermediate_len: usize,
        out: &mut [u8],
    ) -> Result<(), Corrupt> {
        match mode {
            Mode::None => {
                let src = packed.get(..out.len()).ok_or(Corrupt)?;
                out.copy_from_slice(src);
                Ok(())
            }
            Mode::Simple => rle::unpack_rle(packed, out),
            Mode::Quick => self.with_scratch(intermediate_len, |me, stage1| {
                me.unpack_quick(packed, stage1)?;
                rle::unpack_rle(stage1, out)
            }),
            Mode::Medium => self.with_scratch(intermediate_len, |me, stage1| {
                me.unpack_medium(packed, stage1)?;
                rle::unpack_rle(stage1, out)
            }),
            Mode::Deep => self.with_scratch(intermediate_len, |me, stage1| {
                me.unpack_deep(packed, stage1)?;
                rle::unpack_rle(stage1, out)
            }),
            Mode::Heavy1 | Mode::Heavy2 => self.with_scratch(intermediate_len, |me, stage1| {
                me.unpack_heavy(
                    mode == Mode::Heavy2,
                    flags.heavy_rebuild_trees(),
                    packed,
                    stage1,
                )?;
                if flags.heavy_rle() {
                    rle::unpack_rle(stage1, out)
                } else {
                    let src = stage1.get(..out.len()).ok_or(Corrupt)?;
                    out.copy_from_slice(src);
                    Ok(())
                }
            }),
        }
    }

    /// Runs `decode` over a reusable, zeroed scratch buffer of `len` bytes — the
    /// first stage for the two-stage modes (QUICK/MEDIUM/DEEP/HEAVY). The buffer
    /// is borrowed out of `self` for the call and returned afterwards (keeping its
    /// capacity), so back-to-back tracks reuse one allocation.
    fn with_scratch<R>(&mut self, len: usize, decode: impl FnOnce(&mut Self, &mut [u8]) -> R) -> R {
        let mut scratch = core::mem::take(&mut self.scratch);
        scratch.clear();
        scratch.resize(len, 0);
        let result = decode(self, &mut scratch);
        self.scratch = scratch;
        result
    }
}

/// Writes `byte` to the sliding `window` at the current position and advances it.
///
/// Shared by every LZ-family mode; `pos` and `mask` differ per mode (each keeps
/// its own position field and window size), so they are passed in.
fn push_window(window: &mut [u8], pos: &mut u16, mask: u16, byte: u8) {
    window[(*pos & mask) as usize] = byte;
    *pos = pos.wrapping_add(1);
}

/// Copies a back-reference of `length` bytes starting `distance + 1` behind the
/// current position: each byte is read from the window, pushed back onto it, and
/// emitted to `out` at `*out_pos` (which advances). Shared by QUICK/MEDIUM/DEEP/
/// HEAVY, whose match decoding differs only in how `distance`/`length` arrive.
fn copy_match(
    window: &mut [u8],
    pos: &mut u16,
    mask: u16,
    distance: u16,
    length: u16,
    out: &mut [u8],
    out_pos: &mut usize,
) -> Result<(), Corrupt> {
    let mut src = pos.wrapping_sub(distance).wrapping_sub(1);
    for _ in 0..length {
        let byte = window[(src & mask) as usize];
        push_window(window, pos, mask, byte);
        src = src.wrapping_add(1);
        *out.get_mut(*out_pos).ok_or(Corrupt)? = byte;
        *out_pos += 1;
    }
    Ok(())
}
