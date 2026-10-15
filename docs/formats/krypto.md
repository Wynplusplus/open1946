# Content encryption ("Krypto" layer)

Status: **verified** for game classes and flight-model data; not yet implemented in Rust.

Some archive entries are XOR-scrambled after SFS decompression. All variants share one building block:

- **Key table `K`**: 1024 bytes stored in the runtime's `Finger` class. OpenIL2 must read it from the
  user's own install at runtime (see the class bootstrap below); it is never shipped.
- **`IntHash(s)`**: the 32-bit low lane of the path hash (`TLO` table in `sfs.md`). It runs over the
  UTF-16 code units of a string, low byte first, then high byte, and the result is read as a signed 32-bit int.
- **Key slice `key(h)`**: with `v = |h|` (as a signed 32-bit int),
  `len = max(10, v mod 16 + 14)` and `start = v mod 1024`, the key is `K[start .. start+len]` (wrapping),
  taking the low 8 bits of each value.
- **Stream XOR**: byte `i` of the stream is XORed with `key[(i + 1) mod len]`, so the key starts at its
  **second** element. One exception: game classes start at the first element (see below).

## Game classes
A class `com.maddox.X.Y` is stored at archive path `cod/<IntHash("sdw" + name + "cwc2w9e")>` (decimal, signed).
Its bytes are XORed with `key(IntHash(...))` starting at key element 0. A standard class-file header
`CA FE BA BE 00 00 00 2F` (major version 47) is then prepended.

The five bootstrap classes that implement this loader are stored under fixed hashes. They are scrambled
with keys embedded in the launcher executable, using the same XOR scheme starting at element 0.

## Flight-model tables
Status: **verified**. All 158 flight models referenced by aircraft and all 27 engine families decode.

Flight-model (`FlightModels/<name>.fmd`) and engine (`FlightModels/<family>.emd`) section files are not stored under
their own names. They sit inside one container entry at archive path `gui/game/buttons` (see "Stream container"
below).

To read `name` (for example `FlightModels/Bf-109G-6Early.fmd`):
1. `l = lowercase(name)`.
2. Open the container stream named by the decimal string of `IntHash(l + "d2w0")`.
3. The key comes from `h = IntHash(l + "ogh9")` and the stream length `n`. With integer division and
   truncating `mod` on non-negative values (`h` and `n` taken as absolute values):
   - `len = max(10, (n + h/5) mod 16 + 14)`,
   - `start = (n + h/19) mod 1024`,
   - the key is `K[start ..]` (wrapping), taking the low byte of each value.
4. Stream XOR with the key, starting at key element 1 (see "Stream XOR" above).
5. The result is a Windows-1252 section file.

## Stream container (`InOutStreams`)
A named-stream archive. All integers are **big-endian**.

- Header (16 bytes): `i32 signature = 0x4A3B2C1D`, `i32 version = 101`,
  `i32 compression` (0 none, 1 LZSS, 2 zip), `i32 blockSize` (2048..=131072).
- Data blocks follow the header back to back.
- Then a block-length table: one `i32` per block. Block `k` starts at file offset `16 + sum(len[0..k])`.
- Then the entries, until the tailer: a Java modified-UTF-8 name (`u16` length + bytes), `i32` stream
  length, `i32` block count, then that many `i32` block indices.
- Tailer (last 8 bytes): `i32` size of the block-length table, `i32` size of the entry list.

Stream block `k` holds `min(blockSize, length − k·blockSize)` bytes. A stored block shorter than that is
compressed. For zip, that means a `u16` method (8 = raw deflate, 0 = stored) followed by the payload, just
like SFS method-2 blocks without the leading tag byte. The shipped container uses zip with 32 KiB blocks
and holds 386 streams.

## Other users
Aircraft data tables (`"adt"`), localized strings and a few lists use the same scheme with other salts
(TODO when needed).
