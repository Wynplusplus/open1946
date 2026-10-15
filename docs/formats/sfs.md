# SFS archive format

Status: **verified** against `files.SFS` (v4.13.4 GOG). All values are little-endian.

The game mounts many `*.SFS` files into a single virtual file system. Files inside are not stored by name,
only by a 64-bit hash of their path, so a reader has to be told the path it is looking for.

## Header (256 bytes at offset 0)

| Offset | Type   | Meaning |
|-------:|--------|---------|
| 0x00   | [u8;4] | magic `"SFS\0"` |
| 0x04   | u32    | version: `0xC9` or `0xCA`. If `version & 3 != 0` the index and block table are XOR-scrambled. |
| 0x08   | u32    | checksum: sum of all 256 header bytes, with this field taken as zero |
| 0x0C   | u32    | number of index entries |
| 0x10   | u32    | file offset of the index (`entries * 32` bytes) |
| 0x14   | u32    | file offset of the block table |
| 0x18   | u32    | file offset where compressed data starts (equals block table entry 0) |
| 0x1C   | u32    | size of the logical (uncompressed) data stream |
| 0x20   | u16    | maximum path length used inside the archive (only the low 16 bits are meaningful) |
| 0x22.. |        | copyright banner, padding |

### Scramble key
The key is 8 bytes: the `HashState` (see below) run over all 256 *raw* header bytes, with
`lo` stored as u32 LE followed by `hi` as u32 LE.

Version `0xCA` uses the hash of the archive's **lower-case file name** (for example `filesserver.sfs`)
instead of the header hash. Header bytes `0x08..0x20` are also scrambled with this key, using the same
`(i + i/3)` pattern. Here `i` counts from 0 at offset 0x08, and the unscramble happens before the checksum
test. In v4.13.4 only `filesserver.SFS` uses `0xCA`.

### Index (scrambled)
Byte `i` of the index is XORed with `key[(i + i/3) mod 8]`, where `/` is integer division.

Each entry is 32 bytes. Entries are sorted ascending by `hash_lo` (unsigned), so lookups can binary-search.

| Offset | Type | Meaning |
|-------:|------|---------|
| 0x00 | u32 | path hash, low word |
| 0x04 | u32 | path hash, high word |
| 0x08 | u32 | unknown (not needed for reading) |
| 0x0C | u32 | offset of the file in the logical data stream |
| 0x10 | u32 | file size in bytes |
| 0x14 | u32 | always 2 in shipped archives; the original reader zeroes it at runtime |
| 0x18 | u32 | attributes: `0x10` directory, `0x20` file (DOS-style) |
| 0x1C | u32 | probably a content checksum (identical files share a value); the reader zeroes it |

### Block table (scrambled)
The logical data stream is cut into 32 KiB (`0x8000`) blocks.
`n = ceil(stream_size / 0x8000)`, and the table holds `n + 1` u32 absolute file offsets.
Byte `i` is XORed with `key[(i + i/5) mod 8]`.

Block `b` occupies the bytes `[table[b], table[b+1])` of the archive file. Call that length `len`.

- `len >= 0x8000`: the block is stored raw. The first `0x8000` bytes are the data.
- otherwise, the first byte is a method tag:
  - `1`: LZSS (below) over the remaining `len - 1` bytes.
  - anything else: a u16 sub-method follows. `0` means stored, `8` means a raw deflate stream (RFC 1951, no zlib header).

Each block decompresses to 0x8000 bytes, except the last one.

### LZSS
This is classic LZSS:
- 4096-byte ring buffer, zero-filled, write position starting at `0xFEE`.
- A flag byte controls the next 8 items, least significant bit first. Bit 1 means a literal byte.
- Bit 0 means a 2-byte reference `b0 b1`: position = `b0 | (b1 & 0xF0) << 4`, length = `(b1 & 0x0F) + 3`.
- Every output byte is also written to the ring.

## Path hash (`HashState`)
Two independent 32-bit lanes `lo`, `hi`, both starting at 0. For each input byte `c`:

```
lo = ((lo << 8) | c) ^ TLO[lo >> 24]
hi = ((hi << 8) | c) ^ THI[hi >> 24]
```

Each table is XOR-linear (`T[a ^ b] = T[a] ^ T[b]`), so it is fully defined by its values at the powers of two:

| bit | TLO          | THI          |
|----:|--------------|--------------|
| 0   | `0x23788D5E` | `0x1434182E` |
| 1   | `0x46F11ABC` | `0x3C5C2872` |
| 2   | `0x0DE23578` | `0x6C8C48CA` |
| 3   | `0x38BCE7AE` | `0x4D2C89BA` |
| 4   | `0x7179CF5C` | `0x1A591374` |
| 5   | `0x62F39EB8` | `0x20863EC6` |
| 6   | `0x669FB02E` | `0x553865A2` |
| 7   | `0x6E47ED02` | `0x3E44D36A` |

The same hash also derives the scramble key (above).

### Path normalization
To build the lookup key, the game:
1. converts the path to upper case,
2. uses `\` as the separator,
3. hashes the single-byte (Windows-1252) bytes, with no leading separator.

Example: `com/maddox/il2/objects/air.ini` becomes `COM\MADDOX\IL2\OBJECTS\AIR.INI`, which hashes to `lo=0x2C47365D, hi=0xF3CC1777`.

## Content-level encryption
Some files inside the archives (for example many under `com/maddox/...`) are still scrambled after
decompression. That is a separate layer applied by the Java runtime and is documented in its own spec
once analyzed.
