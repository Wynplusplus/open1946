# Binary section files

Status: **verified** on aircraft meshes (v4.13.4).

A compact encoding of a section file (see `sectfile.md`) that the engine accepts wherever it accepts a text
section file. Most shipped `.msh` meshes use it. All values are little-endian.

## Header (28 bytes)

| Offset | Type | Meaning |
|-------:|------|---------|
| 0x00 | u8   | `0x01` (binary marker) |
| 0x01 | u16  | `"BS"` (`0x5342`) |
| 0x03 | u8   | 0 |
| 0x04 | u32  | total file size |
| 0x08 | u32  | number of sections (`S`, at least 1) |
| 0x0C | u32  | number of lines (`L`) |
| 0x10 | u32  | offset of the section table |
| 0x14 | u32  | offset of a section-name lookup table (only speeds up name lookups; not needed for decoding) |
| 0x18 | u32  | offset of the line table |

## Section table (`S` x 12 bytes)

| Offset | Type | Meaning |
|-------:|------|---------|
| 0 | u32 | file offset of the section name |
| 4 | u32 | index of the first line |
| 8 | u16 | number of lines |
| 10 | u8 | column type (below) |
| 11 | u8 | delta flag |

A section name is stored as a byte `n` followed by `n` bytes: the name including its brackets, then a NUL.
Example: `09 "[Common]" 00`. All section names are stored back to back after the header.

## Line table (`L + 1` x u32)
The values are **deltas**: the running sum of the first `k+1` values gives the file offset of line `k`.
Line `k` runs to the start of line `k+1`, and the final sum equals the file size. The lines of one section are
contiguous.

## Delta flag
When the delta flag is non-zero, the section's whole data region (all its lines back to back) is delta-coded
and must be prefix-summed before decoding:
- column type 1: running sum over bytes (mod 256),
- column type 2: running sum over i16 (wrapping).

Other column types with the flag set are invalid.

## Column types
- **0: tokens.** Each line is a sequence of tagged values:
  - `00` + i16: integer,
  - `01` + f32: float,
  - `n` (`n >= 2`) + `n-1` bytes: a string.

  The first token is the key, the rest is the value, mirroring a text line.
- **1: u8 columns.** Every byte of the line is one number.
- **2: i16 columns.**
- **3: 24-bit fixed point.** Every 3 bytes form a signed 24-bit integer `v`; the number is `v / 65536`.
- **4: f32 columns.**

For types 1–4, a line has no key: it is a row of numbers, whose count follows from the line length.
