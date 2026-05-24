//! Run-length decode, the second stage of most modes and the whole of "SIMPLE".

use super::Corrupt;

/// Escape byte introducing a run (or, when followed by `0x00`, a literal `0x90`).
const RLE_ESCAPE: u8 = 0x90;
/// Run-length byte signalling that a 16-bit count follows instead.
const RLE_LONG_COUNT: u8 = 0xff;

/// Expands the RLE stream in `input` into exactly `out.len()` bytes.
///
/// Fails (`Corrupt`) if the input runs out or a run would overflow `out` — the
/// declared unpacked length is the source of truth, so an overrun means the
/// stream is bad.
pub fn unpack_rle(input: &[u8], out: &mut [u8]) -> Result<(), Corrupt> {
    let mut pos = 0usize;
    let mut cursor = input.iter().copied();
    let mut next = || cursor.next().ok_or(Corrupt);

    while pos < out.len() {
        let byte = next()?;
        if byte != RLE_ESCAPE {
            out[pos] = byte;
            pos += 1;
            continue;
        }
        let run = next()?;
        if run == 0 {
            // 0x90 0x00 encodes a literal escape byte.
            out[pos] = RLE_ESCAPE;
            pos += 1;
            continue;
        }
        let value = next()?;
        let count = if run == RLE_LONG_COUNT {
            (usize::from(next()?) << 8) | usize::from(next()?)
        } else {
            usize::from(run)
        };
        let span = out.get_mut(pos..pos + count).ok_or(Corrupt)?;
        span.fill(value);
        pos += count;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::unpack_rle;

    fn rle(input: &[u8], out_len: usize) -> Vec<u8> {
        let mut out = vec![0u8; out_len];
        unpack_rle(input, &mut out).unwrap();
        out
    }

    #[test]
    fn copies_plain_literals() {
        assert_eq!(rle(&[1, 2, 3], 3), [1, 2, 3]);
    }

    #[test]
    fn escaped_zero_emits_literal_marker() {
        assert_eq!(rle(&[0x90, 0x00], 1), [0x90]);
    }

    #[test]
    fn short_run_repeats_value() {
        assert_eq!(rle(&[0x90, 0x05, 0x41], 5), [0x41; 5]);
    }

    #[test]
    fn long_run_uses_16bit_count() {
        // 0x90 0xff <value> <count_hi> <count_lo>
        assert_eq!(rle(&[0x90, 0xff, 0x42, 0x01, 0x00], 256), [0x42; 256]);
    }

    #[test]
    fn mixes_literals_and_runs() {
        assert_eq!(
            rle(&[0x41, 0x90, 0x03, 0x42, 0x43], 5),
            [0x41, 0x42, 0x42, 0x42, 0x43]
        );
    }

    #[test]
    fn run_overflowing_output_is_corrupt() {
        let mut out = [0u8; 3];
        assert!(unpack_rle(&[0x90, 0x05, 0x41], &mut out).is_err());
    }

    #[test]
    fn exhausted_input_is_corrupt() {
        let mut out = [0u8; 3];
        assert!(unpack_rle(&[0x41], &mut out).is_err());
    }
}
