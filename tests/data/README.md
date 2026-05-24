# Committed test fixtures

Small DMS archives committed directly (unlike the large HEAVY disk images, which
download on demand into the gitignored `tests/fixtures/`). Kept tiny on purpose.

## `rle_small.dms` (2225 bytes)

A two-cylinder disk compressed with **SIMPLE (RLE)**, carrying a banner and a
`FILEID.DIZ`. It gives the test suite real RLE coverage from an *independent*
encoder ([adf2dms](https://github.com/dlitz/adf2dms)) rather than only the
crate's own synthetic streams. Decodes to a 22528-byte ADF
(SHA-256 `d032deb4…9f98c`).

Regenerate (needs Python with `crccheck`, and a checkout of adf2dms):

```sh
python3 - <<'PY'
buf = bytearray()
for cyl in range(2):
    buf += bytes([0xAA]) * 4000                       # long run
    buf += bytes([0x90]) * 300                        # runs of the RLE escape byte
    buf += bytes((i*7 + cyl) & 0xff for i in range(1000))  # varied literals
    buf += bytes(11264 - 5300)                         # zero padding
open("rle_src.adf", "wb").write(bytes(buf))
PY
printf 'Made with adf2dms' > b.txt
printf 'Small RLE test disk.\nTwo cylinders.' > f.txt
PYTHONPATH=/path/to/adf2dms python3 -m adf2dms rle_src.adf -b b.txt -a f.txt -o rle_small.dms -f
```
