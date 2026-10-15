# OpenIL2

A clean-room reimplementation of the IL-2 Sturmovik 1946 engine in Rust, in the spirit of OpenMW.
It needs a legally owned copy of the game (e.g. GOG); point `IL2_ROOT` at the install directory.

No game files, decompiled code or key material are part of this repository. Format knowledge lives in
`docs/formats/` as prose specifications, and the Rust code is written from those specs.

## Status: milestone 1 (archives + asset viewer)

| Area | State |
|------|-------|
| SFS archives (`il2-sfs`) | All 76 archives, 266,627 files, 0 decode failures |
| Mount order and lookup (`il2-vfs`) | Matches the game: `.rc` mount list, later mounts win, protected dirs |
| Section files, binary section files | Text and binary encodings |
| Meshes `.msh`, hierarchies `.him`, materials `.mat` | 90,525 / 1,555 / 43,464 shipped files parse, 0 failures |
| IMF images | Types 0 and 1 verified (aircraft skins), type 2 (JPEG) implemented |
| Maps | Height map decoding; terrain draped with the overview image |
| Viewer | Aircraft with season skins and alpha test; terrain; orbit camera; headless screenshots |

Not yet: blended/decal layers, LOD switching, collision meshes, land textures, sky, objects, gameplay (see `docs/roadmap.md`).

## Usage

```sh
export IL2_ROOT="$HOME/Games/Heroic/IL-2 Sturmovik 1946"
cargo build --release

# Archive tools
target/release/il2 validate                 # decode every block of every archive
target/release/il2 stat
target/release/il2 cat com/maddox/il2/objects/air.ini
target/release/il2 names --out names.txt    # recover archive path names (~75%)
target/release/il2 parse-all --names names.txt

# Viewer (drag to orbit, wheel to zoom)
target/release/il2-viewer "3DO/Plane/Bf-109G-6Early/hier.him"
target/release/il2-viewer "3do/plane/Il-2-1941Early(Multi1)/hier.him" --season winter
target/release/il2-viewer maps/Slovakia/load.ini
target/release/il2-viewer maps/Slovakia/load.ini --screenshot out.png --zoom 0.1 --target 150000,100000,1500
```

## Tests

```sh
cargo test --workspace
IL2_ROOT=... cargo test --workspace -- --ignored   # against a real install
```
