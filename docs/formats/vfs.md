# Virtual file system (mounting and lookup)

Status: **verified** from the runtime's mount and open logic (v4.13.4).

## Mount order
1. The launcher mounts `files.SFS` before anything else.
2. During startup the game runs the command script `.rc`, which is read from the mounted archives. Its lines of the form
   `@sfs MOUNT <archive> [<as path>]` (an `@` prefix is optional) mount the remaining archives in order.
   File names are case-insensitive; the stock script uses lower case (`fb_3do.sfs`).
   `filesserver.SFS` and the `*server.SFS` archives are not mounted by the client script.

## Lookup
Every mounted entry goes into one global table keyed by the 64-bit path hash (see `sfs.md`).
If an entry with the same hash already exists, the **later mount takes precedence**.
This is how the `…p.sfs` patch archives override their base archives.

`open(path)`:
1. Look up `hash(path)` in the archive table. If found, use it.
2. If not found, fall back to the real file system, relative to the game directory, **unless**:
   - the first path component is a protected top-level directory. Known ones: `3do`, `maps`, `gui`, `cod`,
     `com`, `effects`, `presets`; one more is not yet identified. These are compared via their
     `LongFN` hash, case-insensitively.
   - the path has no directory component and starts with `.rc`.

So, unlike most games, loose files **cannot override** archived assets in the stock runtime.
OpenIL2 follows these rules by default and may offer an explicit override directory for modding.

Paths containing a character `>= 0x80` bypass the archives entirely and go straight to the file system.
