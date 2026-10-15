# Content encryption ("Krypto" layer)

Status: **verified** for game classes; **documented, not yet implemented** for data files.

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
Flight-model section files are not stored under their own names. They sit inside a container entry at
path `gui/game/buttons`, keyed by `IntHash(lowercase(name) + "d2w0")`. They are XOR-decoded with a key
derived from `IntHash(lowercase(name) + "ogh9")` and the entry length (a variant of `key(h)`). The container
format (`InOutStreams`) and the exact variant are still TODO.

## Other users
Aircraft data tables (`"adt"`), localized strings and a few lists use the same scheme with other salts
(TODO when needed).
