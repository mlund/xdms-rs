//! Per-mode decompressors and the shared sliding-window state they carry.
//!
//! Most modes are two-stage: a mode-specific LZ/LZH/Huffman pass followed by an
//! RLE pass (see [`rle`]). The C kept the window and bit-reader in globals; here
//! that state will live in a single `Decompressor` struct.

mod rle;

/// Returned by a decompressor when the compressed stream is invalid (truncated,
/// corrupt, or decrypted with the wrong password). The drive loop attaches the
/// track number to turn this into [`crate::Error::BadData`].
#[derive(Debug)]
pub struct Corrupt;
