//! Minimal command-line example: decode a `.dms` archive to a `.adf` disk image.
//!
//! ```text
//! cargo run --example dms2adf -- input.dms output.adf
//! ```
//!
//! This is a thin demonstration of the library API, not a full CLI.

use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, input, output] = args.as_slice() else {
        eprintln!("usage: dms2adf <input.dms> <output.adf>");
        exit(2);
    };
    match xdms::unpack_file(input, output) {
        Ok(summary) => {
            println!("unpacked {} tracks to {output}", summary.tracks);
            if let Some(banner) = summary.banner {
                println!("banner: {banner}");
            }
            if let Some(file_id) = summary.file_id {
                println!("FILEID.DIZ: {file_id}");
            }
        }
        Err(err) => {
            eprintln!("error: {err}");
            exit(1);
        }
    }
}
