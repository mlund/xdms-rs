//! Downloads the integration-test fixtures listed in `tests/fixtures.txt` into
//! the gitignored `tests/fixtures/`.
//!
//! Run with `cargo run --example fetch_fixtures`. It shells out to the system
//! `curl` (present on Windows 10+, macOS, and Linux) so the project needs no HTTP
//! dependency. Fixtures are real public-domain DMS disk images; expected output
//! hashes live in the manifest and are checked by the integration tests.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(root.join("tests/fixtures.txt")).expect("read manifest");
    let dir = root.join("tests/fixtures");
    std::fs::create_dir_all(&dir).expect("create fixtures dir");

    for line in manifest.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let (Some(name), Some(url)) = (fields.next(), fields.next()) else {
            panic!("bad manifest line: {line}");
        };

        let dest = dir.join(name);
        if dest.exists() {
            println!("have {name}");
            continue;
        }
        println!("fetching {name} ...");
        let status = Command::new("curl")
            .args(["-fsSL", "-A", "Mozilla/5.0", "-o"])
            .arg(&dest)
            .arg(url)
            .status()
            .expect("run curl (is it installed?)");
        assert!(status.success(), "download failed for {name}");
    }

    println!("fixtures ready in {}", dir.display());
}
