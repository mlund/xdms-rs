//! The crate's error type and `Result` alias.

use core::fmt;

/// Convenient `Result` alias for fallible `xdms` operations.
pub type Result<T> = core::result::Result<T, Error>;

/// Everything that can go wrong while reading or decompressing a DMS archive.
///
/// Variants carrying a `track` field name the offending track so callers can
/// report it. The enum is `#[non_exhaustive]`: match with a wildcard arm.
#[non_exhaustive]
#[derive(Debug)]
pub enum Error {
    /// The data did not begin with the `DMS!` archive magic.
    NotDms,
    /// The archive header CRC did not match its stored value.
    HeaderCrc,
    /// A track header lacked the `TR` magic. Used internally to mark the end of
    /// valid track data (DMS files may carry trailing junk), so it rarely
    /// surfaces to callers.
    NotTrack,
    /// A track header CRC did not match its stored value.
    TrackHeaderCrc,
    /// A track's stored packed-data CRC did not match.
    TrackDataCrc {
        /// Track number that failed.
        track: u16,
    },
    /// A track's unpacked checksum did not match — corrupt data or wrong password.
    Checksum {
        /// Track number that failed.
        track: u16,
    },
    /// Decompression produced invalid output — corrupt stream or wrong password.
    BadData {
        /// Track number that failed.
        track: u16,
    },
    /// A track used an unrecognised compression mode (the contained byte).
    UnknownMode(u8),
    /// The archive is encrypted but no password was supplied.
    PasswordRequired,
    /// The data is an FMS archive (disk type 7), not a DMS disk image.
    Fms,
    /// A header or track was shorter than the format requires.
    Truncated,
    /// A track's declared size exceeded the maximum a track may occupy.
    TooLarge,
    /// An underlying I/O error (only present with the `std` feature).
    #[cfg(feature = "std")]
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotDms => f.write_str("not a DMS archive"),
            Self::HeaderCrc => f.write_str("archive header CRC mismatch"),
            Self::NotTrack => f.write_str("not a track header"),
            Self::TrackHeaderCrc => f.write_str("track header CRC mismatch"),
            Self::TrackDataCrc { track } => write!(f, "packed-data CRC mismatch on track {track}"),
            Self::Checksum { track } => write!(f, "checksum mismatch on track {track}"),
            Self::BadData { track } => write!(f, "invalid compressed data on track {track}"),
            Self::UnknownMode(mode) => write!(f, "unknown compression mode {mode}"),
            Self::PasswordRequired => f.write_str("archive is encrypted; a password is required"),
            Self::Fms => f.write_str("FMS archive, not a DMS disk image"),
            Self::Truncated => f.write_str("truncated header or track"),
            Self::TooLarge => f.write_str("track larger than the maximum supported size"),
            #[cfg(feature = "std")]
            Self::Io(err) => write!(f, "I/O error: {err}"),
        }
    }
}

impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            #[cfg(feature = "std")]
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

#[cfg(feature = "std")]
impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

#[cfg(test)]
mod tests {
    use super::Error;

    #[test]
    fn display_is_nonempty_and_names_track() {
        assert!(!Error::NotDms.to_string().is_empty());
        assert!(Error::Checksum { track: 7 }.to_string().contains('7'));
    }

    #[cfg(feature = "std")]
    #[test]
    fn converts_from_io_error() {
        let io = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof");
        assert!(matches!(Error::from(io), Error::Io(_)));
    }
}
