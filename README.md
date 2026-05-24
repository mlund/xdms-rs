# xdms-rs

A pure-Rust, dependency-free library for unpacking **DMS** (Disk Masher System)
Amiga disk archives into raw **ADF** disk images.

## Origin

A clean-room Rust port of **xDMS**, the portable public-domain DMS unpacker
written by André Rodrigues de la Rocha and maintained by Heikki Orsila. DMS was
the de-facto Amiga standard for storing non-DOS disks (games, demos); emulators
expect ADF/ADZ instead. The original C decoder is the reference for this port.
Dual-licensed **MIT OR Apache-2.0**.

## Usage

```rust,no_run
// One-liner: decompress a .dms to a .adf
xdms::unpack_file("disk.dms", "disk.adf")?;

// Or get the bytes directly
let adf: Vec<u8> = xdms::unpack_to_vec(std::io::stdin().lock())?;
# Ok::<(), xdms::Error>(())
```

```rust,no_run
use xdms::DmsArchive;

// Full control: metadata, password, salvage mode, custom sink
let mut archive = DmsArchive::open("disk.dms")?.with_password("secret");
println!("{}", archive.info());          // human-readable summary
let summary = archive.unpack_to(std::io::stdout().lock())?;
println!("{} tracks", summary.tracks);

// Integrity check only (no output):
DmsArchive::open("disk.dms")?.verify()?;
# Ok::<(), xdms::Error>(())
```

## Test coverage

All seven compression modes are implemented. How each is verified:

| Mode | Coverage |
| --- | --- |
| NOCOMP | synthetic tests + the RLE fixture's banner/FILEID.DIZ tracks |
| SIMPLE (RLE) | **byte-exact** committed fixture from the independent [adf2dms] encoder, plus round-trip unit tests |
| QUICK | round-trip unit test (literals + a match) |
| MEDIUM | round-trip unit test (literal path) |
| DEEP | faithful port — ⚠️ **no end-to-end fixture** (no public DMS sample uses this obsolete mode) |
| HEAVY1 | **byte-exact** vs the reference C `xdms` on a real public-domain disk |
| HEAVY2 | **byte-exact** vs the reference C `xdms` on a real public-domain disk |

Encryption, banner / FILEID.DIZ capture, salvage mode, and header/CRC/checksum
validation each have tests too.

Fixtures are never committed as large blobs: the HEAVY disk images download on
demand (`cargo run --example fetch_fixtures`) into a gitignored directory and
their tests skip when absent, while the tiny RLE archive is committed under
`tests/data/`. "Byte-exact" means the decoded ADF's SHA-256 matches the reference
output. Run the suite with `cargo test`.

[adf2dms]: https://github.com/dlitz/adf2dms

## Performance

Pure decode throughput vs the reference C `xdms`, both decoding the same
public-domain disks to memory (Rust `--release`, C `-O2`, Apple arm64, startup
amortized over 2000 iterations). Output is byte-identical.

| Disk (mode) | Rust | C | speedup |
| --- | --- | --- | --- |
| GoldenFleece (HEAVY2) | 1.29 ms · 699 MB/s | 1.64 ms · 550 MB/s | 1.27× |
| Gory_Story (HEAVY1) | 4.04 ms · 223 MB/s | 4.71 ms · 191 MB/s | 1.17× |

Ballpark, single-machine numbers — but the idiomatic, bounds-checked port keeps
pace with, and slightly beats, the C reference. (HEAVY1/HEAVY2 are what real disks
use; they're also the only modes with byte-exact fixtures.)

## `no_std`

The crate is `no_std` + `alloc` when built with `default-features = false`; the
`std` feature (on by default) adds the `std::io` `Read`/`Write` API.

## Design goals

The port was planned around a few deliberate choices:

- **Deep modules, small surface.** A handful of types (`DmsArchive`, `Info`,
  `Summary`, `Error`) and a few one-liner functions; every decompressor, bit
  reader, Huffman table, and sliding window is hidden behind one `Decompressor`.
- **Idiomatic, expressive Rust — not a C transliteration.** The C's magic
  integers become enums/newtypes (`Mode`, `DiskType`, `GenInfo`, `TrackFlags`),
  its `#define`s become named constants, and byte parsing/validation lives in
  `From`/`TryFrom` impls. Names say what they mean (`packed_len`, not `pklen1`).
- **Very few dependencies — in fact zero.** No runtime *or* dev dependencies;
  CRC-16, the bit reader, the Huffman builders, and even the tests' SHA-256 are
  implemented in-crate.
- **`core`/`alloc`-first.** The engine uses only `core` and `alloc`; `std::io`
  is confined behind a default `std` feature, so a `no_std` build is mechanical.
- **Faithful and byte-exact.** Output is validated to match the reference C
  `xdms` bit-for-bit; decode loops mirror the original algorithms closely.
- **Test-driven, with an oracle.** Built red→green→refactor, using the original
  C binary (and the independent `adf2dms` encoder) as golden references.
- **Portable and linted.** Pure-Rust engine with no platform APIs; CI covers
  Linux/macOS/Windows plus a `no_std` build, with Clippy's nursery lints on.
- **Comments explain *why*, not *what*** — reserved for format quirks and
  porting rationale.

Assistance of Claude Opus 4.7 was used, adhering to the
[LLVM AI tool use policy](https://llvm.org/docs/AIToolPolicy.html).


## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at
your option.
