//! Oracle tests against real public-domain DMS files.
//!
//! Fixtures are downloaded on demand (see `examples/fetch_fixtures.rs`) into the
//! gitignored `tests/fixtures/`. Each present fixture is decoded and its ADF
//! hashed; the expected hashes in `tests/fixtures.txt` were produced once by the
//! reference C xdms. Absent fixtures are skipped so `cargo test` stays green
//! without network access.

use std::path::PathBuf;

struct Fixture {
    name: String,
    adf_sha256: String,
    adf_len: usize,
}

fn manifest() -> Vec<Fixture> {
    let text = include_str!("fixtures.txt");
    text.lines()
        .filter(|line| !line.trim_start().starts_with('#') && !line.trim().is_empty())
        .map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            assert!(f.len() == 5, "bad manifest line: {line}");
            Fixture {
                name: f[0].to_string(),
                adf_sha256: f[3].to_string(),
                adf_len: f[4].parse().expect("adf_len"),
            }
        })
        .collect()
}

#[test]
fn fixtures_decode_to_expected_adf() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut checked = 0;
    for fixture in manifest() {
        let path = dir.join(&fixture.name);
        if !path.exists() {
            eprintln!("skipping {} (not downloaded)", fixture.name);
            continue;
        }
        let dms = std::fs::read(&path).unwrap();
        let adf = xdms::unpack_bytes(&dms)
            .unwrap_or_else(|e| panic!("decoding {} failed: {e}", fixture.name));
        assert_eq!(adf.len(), fixture.adf_len, "{} ADF length", fixture.name);
        assert_eq!(
            hex(&sha256(&adf)),
            fixture.adf_sha256,
            "{} ADF content mismatch vs C xdms",
            fixture.name
        );

        // The streaming API and integrity check must agree too.
        let mut archive = xdms::DmsArchive::read(std::io::Cursor::new(&dms)).unwrap();
        archive
            .verify()
            .unwrap_or_else(|e| panic!("verify {} failed: {e}", fixture.name));
        checked += 1;
    }
    if checked == 0 {
        eprintln!("no fixtures present; run `cargo run --example fetch_fixtures`");
    }
}

/// Committed small fixture (`tests/data/rle_small.dms`): real SIMPLE/RLE output
/// from the independent adf2dms encoder, carrying a banner and FILEID.DIZ. Always
/// runs (no download needed). See `tests/data/README.md` for provenance.
#[test]
fn committed_rle_fixture() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/rle_small.dms");
    let dms = std::fs::read(path).unwrap();

    let adf = xdms::unpack_bytes(&dms).unwrap();
    assert_eq!(adf.len(), 22528);
    assert_eq!(
        hex(&sha256(&adf)),
        "d032deb4a5008b234b68968699ab2a897250f0f32eeae0aea4b23fa99049f98c"
    );

    let mut archive = xdms::DmsArchive::read(std::io::Cursor::new(&dms)).unwrap();
    let summary = archive.unpack_to(std::io::sink()).unwrap();
    assert_eq!(summary.tracks, 2);
    assert_eq!(summary.banner.as_deref(), Some("Made with adf2dms"));
    assert_eq!(
        summary.file_id.as_deref(),
        Some("Small RLE test disk.\nTwo cylinders.")
    );
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Minimal SHA-256 so the test suite needs no external crate or tool.
fn sha256(data: &[u8]) -> [u8; 32] {
    #[rustfmt::skip]
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (word, bytes) in w.iter_mut().zip(chunk.chunks_exact(4)) {
            *word = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(v);
        }
    }

    let mut out = [0u8; 32];
    for (chunk, word) in out.chunks_exact_mut(4).zip(h) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[test]
fn sha256_self_check() {
    // NIST vector for "abc".
    assert_eq!(
        hex(&sha256(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
