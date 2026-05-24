//! DMS archive- and track-header parsing and the typed fields they decode to.
//!
//! All multi-byte fields are big-endian (DMS is a 68k/Amiga format); decoding is
//! therefore host-endian-independent. Parsing is exposed as `TryFrom<&[u8]>`,
//! which is also where length, magic, and CRC validation live.

use core::fmt;

use crate::crc::crc16;
use crate::error::Error;

/// Length of the DMS archive header, in bytes.
pub const HEADER_LEN: usize = 56;
/// Length of a DMS track header, in bytes.
pub const TRACK_HEADER_LEN: usize = 20;

const MAGIC_ARCHIVE: &[u8] = b"DMS!";
const MAGIC_TRACK: &[u8] = b"TR";

/// Compression mode of a single track (the C `cmode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Stored without compression.
    None,
    /// RLE only ("SIMPLE").
    Simple,
    /// QUICK: small-window LZ.
    Quick,
    /// MEDIUM: LZ with static Huffman distances.
    Medium,
    /// DEEP: LZ with an adaptive Huffman tree.
    Deep,
    /// HEAVY1: LZH with a 4 KB dictionary.
    Heavy1,
    /// HEAVY2: LZH with an 8 KB dictionary.
    Heavy2,
}

impl TryFrom<u8> for Mode {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self, Error> {
        Ok(match value {
            0 => Self::None,
            1 => Self::Simple,
            2 => Self::Quick,
            3 => Self::Medium,
            4 => Self::Deep,
            5 => Self::Heavy1,
            6 => Self::Heavy2,
            other => return Err(Error::UnknownMode(other)),
        })
    }
}

/// Filesystem/format the archived disk holds (the C `disktype`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskType {
    /// AmigaOS 1.x OFS (or a non-DOS disk).
    Ofs,
    /// AmigaOS 2.0 FFS.
    Ffs,
    /// AmigaOS 3.0 OFS, international mode.
    OfsIntl,
    /// AmigaOS 3.0 FFS, international mode.
    FfsIntl,
    /// AmigaOS 3.0 OFS with directory cache.
    OfsDirCache,
    /// AmigaOS 3.0 FFS with directory cache.
    FfsDirCache,
    /// FMS Amiga system file (not a DMS disk image).
    Fms,
    /// A value not used by any known DMS version.
    Unknown(u16),
}

impl From<u16> for DiskType {
    fn from(value: u16) -> Self {
        match value {
            0 | 1 => Self::Ofs,
            2 => Self::Ffs,
            3 => Self::OfsIntl,
            4 => Self::FfsIntl,
            5 => Self::OfsDirCache,
            6 => Self::FfsDirCache,
            7 => Self::Fms,
            other => Self::Unknown(other),
        }
    }
}

impl fmt::Display for DiskType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Ofs => "AmigaOS 1.0 OFS",
            Self::Ffs => "AmigaOS 2.0 FFS",
            Self::OfsIntl => "AmigaOS 3.0 OFS / International",
            Self::FfsIntl => "AmigaOS 3.0 FFS / International",
            Self::OfsDirCache => "AmigaOS 3.0 OFS / Dir Cache",
            Self::FfsDirCache => "AmigaOS 3.0 FFS / Dir Cache",
            Self::Fms => "FMS Amiga System File",
            Self::Unknown(_) => "Unknown",
        })
    }
}

/// Archive-wide "general info" flags (the C `geninfo` bitfield).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenInfo(pub u16);

impl GenInfo {
    /// Empty (zero) blocks were dropped during compression.
    pub const fn no_zero(self) -> bool {
        self.0 & 0x01 != 0
    }
    /// Track data is encrypted (needs a password).
    pub const fn encrypted(self) -> bool {
        self.0 & 0x02 != 0
    }
    /// The archive was produced by appending to an existing one.
    pub const fn appends(self) -> bool {
        self.0 & 0x04 != 0
    }
    /// A banner track is present.
    pub const fn banner(self) -> bool {
        self.0 & 0x08 != 0
    }
    /// The disk is high-density.
    pub const fn hd(self) -> bool {
        self.0 & 0x10 != 0
    }
    /// The disk holds an MS-DOS filesystem.
    pub const fn ms_dos(self) -> bool {
        self.0 & 0x20 != 0
    }
    /// Created on a fixed device.
    pub const fn dev_fixed(self) -> bool {
        self.0 & 0x40 != 0
    }
    /// Produced by a registered copy of DMS.
    pub const fn registered(self) -> bool {
        self.0 & 0x80 != 0
    }
    /// A `FILEID.DIZ` description track is present.
    pub const fn file_id(self) -> bool {
        self.0 & 0x0100 != 0
    }
}

impl From<u16> for GenInfo {
    fn from(value: u16) -> Self {
        Self(value)
    }
}

impl From<GenInfo> for u16 {
    fn from(value: GenInfo) -> Self {
        value.0
    }
}

/// Per-track control flags (the C `flags` byte).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackFlags(pub u8);

impl TrackFlags {
    /// Keep decompressor state from the previous track (do not reinitialise).
    pub const fn keep_state(self) -> bool {
        self.0 & 0x01 != 0
    }
    /// HEAVY: rebuild the Huffman trees from this track's stream.
    pub const fn heavy_rebuild_trees(self) -> bool {
        self.0 & 0x02 != 0
    }
    /// HEAVY: apply an RLE pass after the LZH stage.
    pub const fn heavy_rle(self) -> bool {
        self.0 & 0x04 != 0
    }
    /// HEAVY: use the 8 KB dictionary (HEAVY2) rather than 4 KB (HEAVY1).
    pub const fn heavy_big_dict(self) -> bool {
        self.0 & 0x08 != 0
    }
}

impl From<u8> for TrackFlags {
    fn from(value: u8) -> Self {
        Self(value)
    }
}

impl From<TrackFlags> for u8 {
    fn from(value: TrackFlags) -> Self {
        value.0
    }
}

/// Metadata from the 56-byte archive header.
#[derive(Debug, Clone)]
pub struct Info {
    /// DMS version that created the archive, encoded as `major * 100 + minor`.
    pub creator_version: u16,
    /// Creation time as a Unix timestamp (seconds since the epoch).
    pub date: u32,
    /// Lowest track number present (may be wrong on appended archives).
    pub first_track: u16,
    /// Highest track number present (may be wrong on appended archives).
    pub last_track: u16,
    /// Total packed size of all tracks.
    pub packed_size: u32,
    /// Total unpacked size (typically 901,120 for a standard DD disk).
    pub unpacked_size: u32,
    /// Filesystem/format of the archived disk.
    pub disk_type: DiskType,
    /// Compression mode used by most tracks; `None` if the byte is unrecognised.
    pub default_mode: Option<Mode>,
    /// Archive-wide flags.
    pub info: GenInfo,
}

impl TryFrom<&[u8]> for Info {
    type Error = Error;

    fn try_from(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::Truncated);
        }
        if &bytes[0..4] != MAGIC_ARCHIVE {
            return Err(Error::NotDms);
        }
        let stored = be16(bytes, HEADER_LEN - 2);
        // The header CRC covers everything between the magic and the CRC itself.
        if crc16(&bytes[4..HEADER_LEN - 2]) != stored {
            return Err(Error::HeaderCrc);
        }
        let default_mode = u8::try_from(be16(bytes, 52))
            .ok()
            .and_then(|mode| Mode::try_from(mode).ok());
        Ok(Self {
            creator_version: be16(bytes, 46),
            date: be32(bytes, 12),
            first_track: be16(bytes, 16),
            last_track: be16(bytes, 18),
            // pkfsize/unpkfsize are 3-byte fields; bytes 20 and 24 are unused.
            packed_size: be24(bytes, 21),
            unpacked_size: be24(bytes, 25),
            disk_type: DiskType::from(be16(bytes, 50)),
            default_mode,
            info: GenInfo::from(be16(bytes, 10)),
        })
    }
}

/// The 20-byte header preceding each track's packed data. Crate-internal.
#[derive(Debug, Clone, Copy)]
pub struct TrackHeader {
    /// Track number; 80 = `FILEID.DIZ`, `0xFFFF` = banner.
    pub number: u16,
    /// Packed length as stored in the archive (the bytes that follow).
    pub packed_len: u16,
    /// Length after the first decompression stage (before any RLE pass).
    pub intermediate_len: u16,
    /// Final length after all stages.
    pub unpacked_len: u16,
    /// Control flags.
    pub flags: TrackFlags,
    /// Raw compression-mode byte; converted to [`Mode`] at decode time so an
    /// odd non-data track never blocks parsing.
    pub mode: u8,
    /// Checksum of the unpacked data.
    pub checksum: u16,
    /// CRC of the packed data (as stored, i.e. still encrypted if applicable).
    pub data_crc: u16,
}

impl TryFrom<&[u8]> for TrackHeader {
    type Error = Error;

    fn try_from(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < TRACK_HEADER_LEN {
            return Err(Error::Truncated);
        }
        if &bytes[0..2] != MAGIC_TRACK {
            return Err(Error::NotTrack);
        }
        let stored = be16(bytes, TRACK_HEADER_LEN - 2);
        if crc16(&bytes[0..TRACK_HEADER_LEN - 2]) != stored {
            return Err(Error::TrackHeaderCrc);
        }
        Ok(Self {
            number: be16(bytes, 2),
            packed_len: be16(bytes, 6),
            intermediate_len: be16(bytes, 8),
            unpacked_len: be16(bytes, 10),
            flags: TrackFlags::from(bytes[12]),
            mode: bytes[13],
            checksum: be16(bytes, 14),
            data_crc: be16(bytes, 16),
        })
    }
}

fn be16(bytes: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([bytes[at], bytes[at + 1]])
}

fn be24(bytes: &[u8], at: usize) -> u32 {
    (u32::from(bytes[at]) << 16) | (u32::from(bytes[at + 1]) << 8) | u32::from(bytes[at + 2])
}

fn be32(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

#[cfg(test)]
mod tests {
    use super::{DiskType, Info, Mode, TrackHeader};
    use crate::crc::crc16;
    use crate::error::Error;

    fn archive_header() -> [u8; 56] {
        let mut h = [0u8; 56];
        h[0..4].copy_from_slice(b"DMS!");
        h[10..12].copy_from_slice(&0x0102u16.to_be_bytes()); // encrypted + FILEID.DIZ
        h[12..16].copy_from_slice(&0x1234_5678u32.to_be_bytes()); // date
        h[16..18].copy_from_slice(&2u16.to_be_bytes()); // first track
        h[18..20].copy_from_slice(&83u16.to_be_bytes()); // last track
        h[21..24].copy_from_slice(&[0x01, 0x02, 0x03]); // packed size (3 bytes)
        h[25..28].copy_from_slice(&[0x0D, 0xC0, 0x00]); // unpacked size = 901120
        h[46..48].copy_from_slice(&123u16.to_be_bytes()); // creator version 1.23
        h[50..52].copy_from_slice(&4u16.to_be_bytes()); // disk type
        h[52..54].copy_from_slice(&6u16.to_be_bytes()); // default mode = HEAVY2
        let crc = crc16(&h[4..54]);
        h[54..56].copy_from_slice(&crc.to_be_bytes());
        h
    }

    #[test]
    fn parses_archive_header() {
        let info = Info::try_from(&archive_header()[..]).unwrap();
        assert_eq!(info.creator_version, 123);
        assert_eq!(info.date, 0x1234_5678);
        assert_eq!(info.first_track, 2);
        assert_eq!(info.last_track, 83);
        assert_eq!(info.packed_size, 0x0001_0203);
        assert_eq!(info.unpacked_size, 901_120);
        assert_eq!(info.disk_type, DiskType::FfsIntl);
        assert_eq!(info.default_mode, Some(Mode::Heavy2));
        assert!(info.info.encrypted());
        assert!(info.info.file_id());
        assert!(!info.info.banner());
    }

    #[test]
    fn rejects_bad_magic() {
        let mut h = archive_header();
        h[0] = b'X';
        assert!(matches!(Info::try_from(&h[..]), Err(Error::NotDms)));
    }

    #[test]
    fn rejects_bad_header_crc() {
        let mut h = archive_header();
        h[55] ^= 0xff;
        assert!(matches!(Info::try_from(&h[..]), Err(Error::HeaderCrc)));
    }

    #[test]
    fn rejects_truncated_header() {
        assert!(matches!(
            Info::try_from(&[0u8; 10][..]),
            Err(Error::Truncated)
        ));
    }

    #[test]
    fn unknown_disk_type_round_trips_value() {
        assert_eq!(DiskType::from(42), DiskType::Unknown(42));
    }

    #[test]
    fn unknown_mode_is_error() {
        assert!(matches!(Mode::try_from(9), Err(Error::UnknownMode(9))));
    }

    fn track_header() -> [u8; 20] {
        let mut t = [0u8; 20];
        t[0..2].copy_from_slice(b"TR");
        t[2..4].copy_from_slice(&5u16.to_be_bytes()); // number
        t[6..8].copy_from_slice(&100u16.to_be_bytes()); // packed len
        t[8..10].copy_from_slice(&200u16.to_be_bytes()); // intermediate len
        t[10..12].copy_from_slice(&5000u16.to_be_bytes()); // unpacked len
        t[12] = 0x05; // flags: keep_state | heavy_rle
        t[13] = 5; // mode = HEAVY1
        t[14..16].copy_from_slice(&0xABCDu16.to_be_bytes()); // checksum
        t[16..18].copy_from_slice(&0x1234u16.to_be_bytes()); // data crc
        let crc = crc16(&t[0..18]);
        t[18..20].copy_from_slice(&crc.to_be_bytes());
        t
    }

    #[test]
    fn parses_track_header() {
        let th = TrackHeader::try_from(&track_header()[..]).unwrap();
        assert_eq!(th.number, 5);
        assert_eq!(th.packed_len, 100);
        assert_eq!(th.intermediate_len, 200);
        assert_eq!(th.unpacked_len, 5000);
        assert_eq!(th.mode, 5);
        assert_eq!(th.checksum, 0xABCD);
        assert_eq!(th.data_crc, 0x1234);
        assert!(th.flags.keep_state());
        assert!(th.flags.heavy_rle());
        assert!(!th.flags.heavy_big_dict());
    }

    #[test]
    fn track_bad_magic_is_not_track() {
        let mut t = track_header();
        t[0] = b'Z';
        assert!(matches!(
            TrackHeader::try_from(&t[..]),
            Err(Error::NotTrack)
        ));
    }

    #[test]
    fn track_bad_crc() {
        let mut t = track_header();
        t[19] ^= 0xff;
        assert!(matches!(
            TrackHeader::try_from(&t[..]),
            Err(Error::TrackHeaderCrc)
        ));
    }
}
