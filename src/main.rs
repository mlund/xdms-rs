//! Command-line front-end for the `xdms` library. A deliberately minimal CLI
//! (no argument-parsing dependencies) whose command letters mirror the reference
//! C `xdms` so existing muscle memory and scripts partly carry over:
//!
//! ```text
//! xdms-rs u <file.dms> [output.adf]   unpack to a raw ADF disk image
//! xdms-rs t <file.dms>                test archive integrity
//! xdms-rs v <file.dms>                view archive information
//! xdms-rs f <file.dms>                view full information (adds banner + FILEID.DIZ)
//! xdms-rs d <file.dms>                show attached FILEID.DIZ
//! xdms-rs b <file.dms>                show attached banner
//! ```
//!
//! The C tool's `z` (gzip output) and `x` (extract files via readdisk) are out
//! of scope for this library and are not implemented.

use std::process::ExitCode;

use xdms::{DmsArchive, Result};

/// The two auxiliary text tracks an archive may carry.
enum Text {
    Banner,
    FileId,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();

    let outcome = match argv.as_slice() {
        ["u", input] => unpack(input, None),
        ["u", input, output] => unpack(input, Some(output)),
        ["t", input] => test(input),
        ["v", input] => view(input, false),
        ["f", input] => view(input, true),
        ["d", input] => show(input, &Text::FileId),
        ["b", input] => show(input, &Text::Banner),
        _ => {
            usage();
            return ExitCode::from(2);
        }
    };

    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn usage() {
    eprintln!(
        "usage: xdms-rs <command> <file.dms> [output.adf]\n\
         \n\
         commands:\n  \
         u   unpack to a raw ADF disk image (output defaults to <file>.adf)\n  \
         t   test archive integrity (CRCs and checksums)\n  \
         v   view archive information\n  \
         f   view full information (adds banner and FILEID.DIZ)\n  \
         d   show attached FILEID.DIZ\n  \
         b   show attached banner\n\
         \n\
         Command letters match the reference C xdms; its z (gzip) and x (extract)\n\
         commands are not supported."
    );
}

/// `u`: decompress every data track to an ADF image. Without an explicit output
/// the name is derived from the input, as the C tool does.
fn unpack(input: &str, output: Option<&str>) -> Result<()> {
    // A leading `+` is the C tool's "explicit output" marker; accept and drop it.
    let output = output.map_or_else(|| adf_name(input), |o| o.trim_start_matches('+').to_owned());
    let summary = xdms::unpack_file(input, &output)?;
    println!("unpacked {} tracks to {output}", summary.tracks);
    if let Some(banner) = summary.banner {
        println!("banner: {banner}");
    }
    if let Some(file_id) = summary.file_id {
        println!("FILEID.DIZ:\n{file_id}");
    }
    Ok(())
}

/// `t`: drive the whole stream, validating CRCs and checksums, writing nothing.
fn test(input: &str) -> Result<()> {
    let summary = DmsArchive::open(input)?.verify()?;
    println!("ok: {} tracks verified", summary.tracks);
    Ok(())
}

/// `v` / `f`: print the header metadata; when `full`, also the banner and
/// FILEID.DIZ text, which only surface once the track stream is driven.
fn view(input: &str, full: bool) -> Result<()> {
    let archive = DmsArchive::open(input)?;
    println!("{}", archive.info());
    if full {
        // Salvage keeps a damaged data track from hiding the text tracks, and a
        // failure here shouldn't suppress the header we already printed.
        match archive.with_salvage(true).verify() {
            Ok(summary) => {
                if let Some(banner) = summary.banner {
                    println!("\nbanner:\n{banner}");
                }
                if let Some(file_id) = summary.file_id {
                    println!("\nFILEID.DIZ:\n{file_id}");
                }
            }
            Err(err) => eprintln!("note: could not read description tracks: {err}"),
        }
    }
    Ok(())
}

/// `d` / `b`: print just one auxiliary text track (extracted while driving).
fn show(input: &str, which: &Text) -> Result<()> {
    let summary = DmsArchive::open(input)?.with_salvage(true).verify()?;
    let (label, text) = match which {
        Text::Banner => ("banner", summary.banner),
        Text::FileId => ("FILEID.DIZ", summary.file_id),
    };
    match text {
        Some(text) => println!("{text}"),
        None => eprintln!("no {label} present"),
    }
    Ok(())
}

/// Derives the default ADF output name: a trailing `.dms` (any case) becomes
/// `.adf`, otherwise `.adf` is appended.
fn adf_name(input: &str) -> String {
    input
        .strip_suffix(".dms")
        .or_else(|| input.strip_suffix(".DMS"))
        .map_or_else(|| format!("{input}.adf"), |stem| format!("{stem}.adf"))
}
