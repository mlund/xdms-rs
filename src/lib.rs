//! Pure-Rust unpacker for **DMS** (Disk Masher System) Amiga disk archives.
//!
//! DMS is the de-facto Amiga format for compressed copies of non-DOS disks
//! (games, demos). This crate decompresses a `.dms` archive into a raw **ADF**
//! disk image, which is what Amiga emulators consume.
//!
//! It is a clean-room port of the public-domain C tool *xDMS* by André Rodrigues
//! de la Rocha (maintained by Heikki Orsila). See the crate README for usage.
//!
//! The crate is `no_std` + `alloc` when built with `default-features = false`;
//! the default `std` feature adds the [`std::io`]-based API.
#![cfg_attr(not(feature = "std"), no_std)]
#![warn(missing_docs)]

extern crate alloc;

mod bitreader;
mod crc;
mod decompress;
mod error;
mod header;

pub use error::{Error, Result};
pub use header::{DiskType, GenInfo, Info, Mode};

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::crc::{checksum16, crc16};
use crate::decompress::{Corrupt, Decompressor};
use crate::header::{TrackHeader, HEADER_LEN, TRACK_HEADER_LEN};

/// Largest size (packed, intermediate, or unpacked) a single track may declare.
const MAX_TRACK_LEN: usize = 32000;
/// The `FILEID.DIZ` description track. Data tracks are numbered below it.
const TRACK_FILE_ID: u16 = 80;
/// The banner track.
const TRACK_BANNER: u16 = 0xffff;
/// A data track must unpack to more than this; smaller "track 0"s are fake boot
/// blocks carrying advertising, not disk data.
const MIN_DATA_TRACK_LEN: u16 = 2048;

/// Outcome of unpacking or verifying an archive.
#[derive(Debug, Clone, Default)]
pub struct Summary {
    /// Number of data tracks written to the ADF image.
    pub tracks: u32,
    /// Banner text, if the archive carries one (captured during the drive).
    pub banner: Option<String>,
    /// `FILEID.DIZ` description text, if present.
    pub file_id: Option<String>,
}

/// A byte source the drive loop reads fixed-size blocks from. Abstracts over a
/// `std::io::Read` and an in-memory slice so the engine stays `no_std`-friendly.
trait Source {
    /// Reads exactly `buf.len()` bytes. `Ok(true)` if filled, `Ok(false)` on a
    /// clean end of input before any byte, `Err(Truncated)` on a partial read.
    fn read_block(&mut self, buf: &mut [u8]) -> Result<bool>;
}

/// A byte sink the drive loop writes decoded tracks to (an ADF `Vec`, a
/// `std::io::Write`, or a discard for `verify`).
trait Sink {
    /// Appends a decoded track's bytes.
    fn write_block(&mut self, data: &[u8]) -> Result<()>;
}

struct SliceSource<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Source for SliceSource<'_> {
    fn read_block(&mut self, buf: &mut [u8]) -> Result<bool> {
        let available = self.data.len() - self.pos;
        if available == 0 {
            return Ok(false);
        }
        if available < buf.len() {
            return Err(Error::Truncated);
        }
        buf.copy_from_slice(&self.data[self.pos..self.pos + buf.len()]);
        self.pos += buf.len();
        Ok(true)
    }
}

impl Sink for Vec<u8> {
    fn write_block(&mut self, data: &[u8]) -> Result<()> {
        self.extend_from_slice(data);
        Ok(())
    }
}

/// Discards everything written — used by [`DmsArchive::verify`].
struct NullSink;

impl Sink for NullSink {
    fn write_block(&mut self, _data: &[u8]) -> Result<()> {
        Ok(())
    }
}

/// Reads every track from `source`, decoding data tracks into `sink` and
/// verifying CRCs and checksums along the way. Shared by all public entry points.
fn drive(
    info: &Info,
    decompressor: &mut Decompressor,
    source: &mut dyn Source,
    sink: &mut dyn Sink,
    password: Option<&str>,
    salvage: bool,
) -> Result<Summary> {
    if info.info.encrypted() && password.is_none() {
        return Err(Error::PasswordRequired);
    }
    decompressor.reset();

    // The password CRC seeds a rotating cipher state that advances across every
    // decrypted track (all but FILEID.DIZ), so it lives outside the loop.
    let mut cipher = password.map(|p| crc16(p.as_bytes()));
    let mut summary = Summary::default();
    let mut header = [0u8; TRACK_HEADER_LEN];
    let mut packed = Vec::new();

    loop {
        if !source.read_block(&mut header)? {
            break; // clean end of archive
        }
        let track = match TrackHeader::try_from(&header[..]) {
            Ok(track) => track,
            // Trailing junk after the last track is normal; stop cleanly.
            Err(Error::NotTrack) => break,
            Err(err) => return Err(err),
        };

        if track.packed_len as usize > MAX_TRACK_LEN
            || track.intermediate_len as usize > MAX_TRACK_LEN
            || track.unpacked_len as usize > MAX_TRACK_LEN
        {
            return Err(Error::TooLarge);
        }

        packed.resize(track.packed_len as usize, 0);
        if !source.read_block(&mut packed)? {
            return Err(Error::Truncated);
        }

        // The stored CRC covers the packed data as written (still encrypted).
        if crc16(&packed) != track.data_crc && !salvage {
            return Err(Error::TrackDataCrc {
                track: track.number,
            });
        }

        // Every track but FILEID.DIZ is decrypted (and advances the cipher).
        if let Some(state) = cipher.as_mut() {
            if track.number != TRACK_FILE_ID {
                decrypt(&mut packed, state);
            }
        }

        if track.number == TRACK_BANNER {
            // Decoded on a throwaway decompressor so it can't disturb the data
            // tracks' shared state (the C never decodes it during unpack).
            summary.banner = decode_text(&track, &packed);
            continue;
        }
        if track.number == TRACK_FILE_ID {
            summary.file_id = decode_text(&track, &packed);
            continue;
        }
        // Fake boot blocks and other small tracks are not part of the ADF.
        if track.number >= TRACK_FILE_ID || track.unpacked_len <= MIN_DATA_TRACK_LEN {
            continue;
        }

        let mode = Mode::try_from(track.mode)?;
        let mut out = vec![0u8; track.unpacked_len as usize];
        match decompressor.unpack_track(
            mode,
            track.flags,
            &packed,
            track.intermediate_len as usize,
            &mut out,
        ) {
            Ok(()) => {
                // The C resets between tracks unless the keep-state flag is set,
                // and only after a successful decode (errors return early).
                if !track.flags.keep_state() {
                    decompressor.reset();
                }
            }
            Err(Corrupt) => {
                if !salvage {
                    return Err(Error::BadData {
                        track: track.number,
                    });
                }
            }
        }

        if checksum16(&out) != track.checksum && !salvage {
            return Err(Error::Checksum {
                track: track.number,
            });
        }

        sink.write_block(&out)?;
        summary.tracks += 1;
    }

    Ok(summary)
}

/// Decrypts `data` in place with DMS's rotating XOR cipher, advancing `state`.
/// `state` carries over between tracks (it is not reset per track).
fn decrypt(data: &mut [u8], state: &mut u16) {
    for byte in data.iter_mut() {
        let stored = u16::from(*byte);
        *byte ^= *state as u8;
        *state = (*state >> 1).wrapping_add(stored);
    }
}

/// Best-effort decode of a banner / FILEID.DIZ track to text. Uses a fresh
/// decompressor (these are auxiliary tracks the C decodes from a clean state) and
/// returns `None` rather than failing the whole archive if it can't be decoded.
fn decode_text(track: &TrackHeader, packed: &[u8]) -> Option<String> {
    let mode = Mode::try_from(track.mode).ok()?;
    let mut out = vec![0u8; track.unpacked_len as usize];
    Decompressor::new()
        .unpack_track(
            mode,
            track.flags,
            packed,
            track.intermediate_len as usize,
            &mut out,
        )
        .ok()?;
    Some(text_from(&out))
}

/// Renders decoded banner/DIZ bytes as text, dropping trailing NUL padding.
fn text_from(bytes: &[u8]) -> String {
    let end = bytes.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Decompresses an in-memory DMS archive into an ADF image.
///
/// Available in `no_std` builds (it needs only `alloc`). For encrypted archives
/// or streaming I/O, use [`DmsArchive`].
pub fn unpack_bytes(dms: &[u8]) -> Result<Vec<u8>> {
    if dms.len() < HEADER_LEN {
        return Err(Error::Truncated);
    }
    let info = Info::try_from(dms)?;
    if info.disk_type == DiskType::Fms {
        return Err(Error::Fms);
    }
    let mut source = SliceSource {
        data: dms,
        pos: HEADER_LEN,
    };
    let mut decompressor = Decompressor::new();
    let mut out = Vec::with_capacity(info.unpacked_size as usize);
    drive(&info, &mut decompressor, &mut source, &mut out, None, false)?;
    Ok(out)
}

#[cfg(feature = "std")]
mod stdio {
    use super::{Error, Result, Source};
    use std::io::{ErrorKind, Read};

    /// Adapts any [`std::io::Read`] to the engine's [`Source`].
    pub struct ReadSource<R>(pub R);

    impl<R: Read> Source for ReadSource<R> {
        fn read_block(&mut self, buf: &mut [u8]) -> Result<bool> {
            let mut filled = 0;
            while filled < buf.len() {
                match self.0.read(&mut buf[filled..]) {
                    Ok(0) => break,
                    Ok(n) => filled += n,
                    Err(e) if e.kind() == ErrorKind::Interrupted => {}
                    Err(e) => return Err(Error::Io(e)),
                }
            }
            match filled {
                0 => Ok(false),
                n if n < buf.len() => Err(Error::Truncated),
                _ => Ok(true),
            }
        }
    }

    /// Adapts any [`std::io::Write`] to the engine's [`super::Sink`].
    pub struct WriteSink<W>(pub W);

    impl<W: std::io::Write> super::Sink for WriteSink<W> {
        fn write_block(&mut self, data: &[u8]) -> Result<()> {
            self.0.write_all(data).map_err(Error::Io)
        }
    }
}

/// A DMS archive read from any [`std::io::Read`].
///
/// `read` parses the header (cheap); the heavy work happens in [`unpack_to`],
/// [`unpack_to_vec`], or [`verify`], each of which drives the single-pass track
/// stream to completion.
///
/// [`unpack_to`]: DmsArchive::unpack_to
/// [`unpack_to_vec`]: DmsArchive::unpack_to_vec
/// [`verify`]: DmsArchive::verify
#[cfg(feature = "std")]
pub struct DmsArchive<R> {
    source: stdio::ReadSource<R>,
    info: Info,
    decompressor: Decompressor,
    password: Option<String>,
    salvage: bool,
}

#[cfg(feature = "std")]
impl DmsArchive<std::io::BufReader<std::fs::File>> {
    /// Opens a `.dms` file by path (buffered).
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        Self::read(std::io::BufReader::new(file))
    }
}

#[cfg(feature = "std")]
impl<R: std::io::Read> DmsArchive<R> {
    /// Reads and validates the 56-byte archive header from `reader`.
    pub fn read(reader: R) -> Result<Self> {
        let mut source = stdio::ReadSource(reader);
        let mut header = [0u8; HEADER_LEN];
        if !source.read_block(&mut header)? {
            return Err(Error::Truncated);
        }
        let info = Info::try_from(&header[..])?;
        if info.disk_type == DiskType::Fms {
            return Err(Error::Fms);
        }
        Ok(Self {
            source,
            info,
            decompressor: Decompressor::new(),
            password: None,
            salvage: false,
        })
    }

    /// Archive metadata from the header (no I/O).
    pub const fn info(&self) -> &Info {
        &self.info
    }

    /// Sets the password used to decrypt an encrypted archive.
    #[must_use]
    pub fn with_password(mut self, password: &str) -> Self {
        self.password = Some(String::from(password));
        self
    }

    /// Enables salvage mode: CRC/checksum mismatches are tolerated and as much
    /// data as possible is recovered (the C `-f` option).
    #[must_use]
    pub const fn with_salvage(mut self, on: bool) -> Self {
        self.salvage = on;
        self
    }

    /// Decompresses every data track to `out` as a raw ADF image.
    pub fn unpack_to(&mut self, out: impl std::io::Write) -> Result<Summary> {
        let mut sink = stdio::WriteSink(out);
        drive(
            &self.info,
            &mut self.decompressor,
            &mut self.source,
            &mut sink,
            self.password.as_deref(),
            self.salvage,
        )
    }

    /// Decompresses every data track into a new ADF byte buffer.
    pub fn unpack_to_vec(&mut self) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(self.info.unpacked_size as usize);
        drive(
            &self.info,
            &mut self.decompressor,
            &mut self.source,
            &mut out,
            self.password.as_deref(),
            self.salvage,
        )?;
        Ok(out)
    }

    /// Checks every track's CRCs and checksums without producing output.
    pub fn verify(&mut self) -> Result<Summary> {
        let mut sink = NullSink;
        drive(
            &self.info,
            &mut self.decompressor,
            &mut self.source,
            &mut sink,
            self.password.as_deref(),
            self.salvage,
        )
    }
}

/// Decompresses a `.dms` file to a `.adf` file.
#[cfg(feature = "std")]
pub fn unpack_file(
    src: impl AsRef<std::path::Path>,
    dst: impl AsRef<std::path::Path>,
) -> Result<Summary> {
    let out = std::fs::File::create(dst)?;
    DmsArchive::open(src)?.unpack_to(std::io::BufWriter::new(out))
}

/// Decompresses DMS data from a reader to a writer.
#[cfg(feature = "std")]
pub fn unpack(src: impl std::io::Read, dst: impl std::io::Write) -> Result<Summary> {
    DmsArchive::read(src)?.unpack_to(dst)
}

/// Decompresses DMS data from a reader into a new ADF byte buffer.
#[cfg(feature = "std")]
pub fn unpack_to_vec(src: impl std::io::Read) -> Result<Vec<u8>> {
    DmsArchive::read(src)?.unpack_to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crc::{checksum16, crc16};
    use alloc::vec;
    use alloc::vec::Vec;

    const TRACK_HEADER_LEN: usize = 20;
    const HEADER_LEN: usize = 56;

    fn track(number: u16, mode: u8, packed: &[u8], unpacked: &[u8]) -> Vec<u8> {
        let mut t = vec![0u8; TRACK_HEADER_LEN];
        t[0..2].copy_from_slice(b"TR");
        t[2..4].copy_from_slice(&number.to_be_bytes());
        t[6..8].copy_from_slice(&(packed.len() as u16).to_be_bytes());
        t[8..10].copy_from_slice(&(packed.len() as u16).to_be_bytes()); // intermediate len
        t[10..12].copy_from_slice(&(unpacked.len() as u16).to_be_bytes());
        t[13] = mode;
        t[14..16].copy_from_slice(&checksum16(unpacked).to_be_bytes());
        t[16..18].copy_from_slice(&crc16(packed).to_be_bytes());
        let hcrc = crc16(&t[0..18]);
        t[18..20].copy_from_slice(&hcrc.to_be_bytes());
        t.extend_from_slice(packed);
        t
    }

    fn archive(tracks: &[Vec<u8>]) -> Vec<u8> {
        archive_with(0, tracks)
    }

    fn archive_with(geninfo: u16, tracks: &[Vec<u8>]) -> Vec<u8> {
        let mut h = vec![0u8; HEADER_LEN];
        h[0..4].copy_from_slice(b"DMS!");
        h[10..12].copy_from_slice(&geninfo.to_be_bytes());
        h[50..52].copy_from_slice(&2u16.to_be_bytes()); // disk type FFS (not FMS)
        let crc = crc16(&h[4..54]);
        h[54..56].copy_from_slice(&crc.to_be_bytes());
        for t in tracks {
            h.extend_from_slice(t);
        }
        h
    }

    /// Forward of the DMS cipher: produces bytes that [`decrypt`] recovers.
    fn encrypt(data: &[u8], mut state: u16) -> Vec<u8> {
        data.iter()
            .map(|&p| {
                let c = p ^ state as u8;
                state = (state >> 1).wrapping_add(u16::from(c));
                c
            })
            .collect()
    }

    #[test]
    fn unpacks_nocomp_and_rle_tracks() {
        let a: Vec<u8> = (0..3000u32).map(|i| i as u8).collect();
        let b = vec![0x5Au8; 3000];
        let rle_b = [0x90, 0xff, 0x5A, 0x0B, 0xB8]; // run of 0x5A, count 3000
        let dms = archive(&[track(0, 0, &a, &a), track(1, 1, &rle_b, &b)]);

        let mut expected = a;
        expected.extend_from_slice(&b);
        assert_eq!(unpack_bytes(&dms).unwrap(), expected);

        let mut arch = DmsArchive::read(&dms[..]).unwrap();
        assert_eq!(arch.info().disk_type, DiskType::Ffs);
        let mut out = Vec::new();
        let summary = arch.unpack_to(&mut out).unwrap();
        assert_eq!(out, expected);
        assert_eq!(summary.tracks, 2);
    }

    #[test]
    fn skips_non_data_tracks() {
        let a = vec![1u8; 3000];
        let banner = track(0xffff, 0, b"hi", b"hi");
        let dms = archive(&[banner, track(0, 0, &a, &a)]);
        assert_eq!(unpack_bytes(&dms).unwrap(), a);
    }

    #[test]
    fn detects_checksum_error() {
        let a = vec![7u8; 3000];
        let mut t = track(0, 0, &a, &a);
        t[14] ^= 0xff; // corrupt stored checksum, then fix the header CRC
        let hcrc = crc16(&t[0..18]);
        t[18..20].copy_from_slice(&hcrc.to_be_bytes());
        let dms = archive(&[t]);
        assert!(matches!(
            unpack_bytes(&dms),
            Err(Error::Checksum { track: 0 })
        ));
    }

    #[test]
    fn salvage_tolerates_checksum_error() {
        let a = vec![7u8; 3000];
        let mut t = track(0, 0, &a, &a);
        t[14] ^= 0xff;
        let hcrc = crc16(&t[0..18]);
        t[18..20].copy_from_slice(&hcrc.to_be_bytes());
        let dms = archive(&[t]);
        let mut arch = DmsArchive::read(&dms[..]).unwrap().with_salvage(true);
        assert_eq!(arch.unpack_to_vec().unwrap(), a);
    }

    #[test]
    fn rejects_non_dms() {
        assert!(matches!(unpack_bytes(&[b'X'; 60]), Err(Error::NotDms)));
    }

    const GENINFO_ENCRYPTED: u16 = 0x02;

    #[test]
    fn decrypts_with_correct_password() {
        let plain = vec![0xC3u8; 3000];
        let seed = crc16(b"secret");
        let cipher = encrypt(&plain, seed);
        let dms = archive_with(GENINFO_ENCRYPTED, &[track(0, 0, &cipher, &plain)]);

        let mut arch = DmsArchive::read(&dms[..]).unwrap().with_password("secret");
        assert_eq!(arch.unpack_to_vec().unwrap(), plain);
    }

    #[test]
    fn encrypted_archive_without_password_is_rejected() {
        let plain = vec![0xC3u8; 3000];
        let cipher = encrypt(&plain, crc16(b"secret"));
        let dms = archive_with(GENINFO_ENCRYPTED, &[track(0, 0, &cipher, &plain)]);
        assert!(matches!(
            DmsArchive::read(&dms[..]).unwrap().unpack_to_vec(),
            Err(Error::PasswordRequired)
        ));
    }

    #[test]
    fn wrong_password_is_detected() {
        let plain = vec![0xC3u8; 3000];
        let cipher = encrypt(&plain, crc16(b"secret"));
        let dms = archive_with(GENINFO_ENCRYPTED, &[track(0, 0, &cipher, &plain)]);
        let result = DmsArchive::read(&dms[..])
            .unwrap()
            .with_password("wrong")
            .unpack_to_vec();
        assert!(matches!(result, Err(Error::Checksum { .. })));
    }

    #[test]
    fn captures_banner_and_file_id() {
        let data = vec![9u8; 3000];
        let dms = archive(&[
            track(0xffff, 0, b"Cracked by nobody", b"Cracked by nobody"),
            track(80, 0, b"FILEID text", b"FILEID text"),
            track(0, 0, &data, &data),
        ]);

        let mut arch = DmsArchive::read(&dms[..]).unwrap();
        let mut adf = Vec::new();
        let summary = arch.unpack_to(&mut adf).unwrap();
        assert_eq!(adf, data); // only the data track reaches the ADF
        assert_eq!(summary.tracks, 1);
        assert_eq!(summary.banner.as_deref(), Some("Cracked by nobody"));
        assert_eq!(summary.file_id.as_deref(), Some("FILEID text"));
    }
}
