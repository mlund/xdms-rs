# xdms

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

## `no_std`

The crate is `no_std` + `alloc` when built with `default-features = false`; the
`std` feature (on by default) adds the `std::io` `Read`/`Write` API.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at
your option.
