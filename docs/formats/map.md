# Maps (`maps/<name>/load*.ini`)

Status: height decoding **verified** from the engine's height lookup; texturing is partial.

A map directory holds a `load.ini` (plus seasonal or online variants) in section-file syntax with `key = value` entries.

- `[MAP]`:
  - `HeightMap` (8-bit grayscale TGA),
  - `TypeMap` (8-bit land type per pixel),
  - `ColorMap`, `SmallMap` (shaded 32-bit overview image, same size as the height map), `FarMap`, `ReflMap`.
- `[WORLDPOS]`: `CAMOUFLAGE` (SUMMER/WINTER/DESERT …), `MONTH`, `DECLIN` (latitude), `PRESSURE`, `TEMPERATURE`.
- `[CUBE]`: sky cube faces. `[WOOD]`, `[FIELDS]`, `[ROADS]`, `[WATER]`, `[APPENDIX]`: land texture sets.
  These paths are relative to `maps/_Tex/` (TODO: confirm).
- `[static]`: static object lists (`*.static`). `[text]`: map labels.

## Grid
One height-map pixel is **200 m** square. Pixel column `c` lies at `x = 200·c`, and pixel row `r` (top row
first) at `y = 200·(rows − 1 − r)`, so north is up in the image.

## Height decoding
A height-map byte `v` maps to metres through a piecewise-linear table:

| `v` | metres |
|-----|--------|
| 0–63 | `v` |
| 64–95 | `2v − 64` |
| 96–127 | `4v − 256` |
| 128–159 | `8v − 768` |
| 160–191 | `16(v − 128)` |
| 192–223 | `32(v − 160)` |
| 224–255 | `64(v − 192)` |

Between pixels the engine interpolates over the two triangles of each cell.

## Land types (`TypeMap`)
One byte per height-map pixel. **Verified** from the runtime's road lookup: land-type pixel `(c, r)` covers
`x ∈ [200c, 200c + 200)` and `(rows − 1) − y/200 ∈ [r, r + 1)`, i.e. the height-map cell between height
pixels `(c, r)` and `(c + 1, r + 1)`; its centre is at `(200(c + ½), 200(rows − 1.5 − r))`. The low 5 bits select one of 32 field slots in the order of `[FIELDS]`:
`LowLand0..3`, `MidLand0..3`, `Mount0..3`, `Country0..3`, `City0..3`, `AirField0..3`, `Wood0..3`, `Water0..3`.
Slots 28–31 are water. The upper 3 bits are roads (see below).

`[FIELDS]` values are texture paths relative to `maps/_Tex/`, optionally followed by `,N`. The native loader
accepts only `N ∈ {−2, 1, 2, 4, 8}` (others: warning, treated as 1); OpenIL2 treats it as a tiling scale. Slot 0 (`LowLand0`) is required, and other slots may be empty.

## OpenIL2 terrain rendering (own design, not a port)
- Each fragment blends the textures of the four nearest cells bilinearly.
- Textures repeat every `400 m × N`.
- Water slots reflect the sky colour with a Fresnel term.
- Exponential distance fog towards the horizon colour, and a sky gradient.

## Forests (`[WOOD]`)
Keys: `Wood0`..`Wood4` (canopy layers: `Wood0` dense crowns down to `Wood4` sparse treetops, RGBA with
alpha cut-outs), `WoodMask2`, `WoodMask3`, `WoodMiniMasks` (8-bit edge masks), `SideWood` (a tree
silhouette for forest edges). Paths are relative to `maps/_Tex/`. Wood cells are land-type slots 24–27.

The native landscape renderer that draws forests has not been studied. OpenIL2's own design:
- Within 3 km of the camera, wood cells get five terrain-following canopy shells (7–20 m above ground,
  `Wood0` lowest), alpha-tested, each texture repeating every 160 m.
- Edges are resolved on a 50 m sub-grid from the blended wood indicator plus per-sub-cell noise, so forest
  outlines are not square. Outer sub-cell edges get 20 m high `SideWood` strips.
- Beyond that, the terrain's `Wood*` field texture stands in.

## Roads (`TypeMap` bits 5–7, `[ROADS]`)
**Verified** from the runtime's `HQRoadTypeHere` and its users:
- Bit `0x20`: road, runtime type 1 (wheel friction 0.8, bombs hit it).
- Bit `0x80`: highway, type 2 (friction 0.0).
- Bit `0x40`: railway, type 3 (friction 5.0).
- A road runs from a pixel's centre towards each of its 8 neighbours that has the same bit. The lookup takes
  the point's direction from the pixel centre (octants, with a small dead zone on the boundaries) and
  checks the neighbour in that direction.

`[ROADS]` gives `Road`, `Highway` and `Rail` as texture paths without extension, relative to `maps/_Tex/`
(`+ ".tga"`). Each is a 256×128 atlas: curve and junction pieces on the left, a straight vertical piece in the
right quarter. The native renderer also loads `Roads/RoadMask256.tga` and `Roads/RoadTreeMask.tga`.

OpenIL2 (own design): textured strips between connected pixel centres (a diagonal is skipped when both
orthogonal links around it exist), 7 / 10 / 6 m wide, draped 0.3 m above the terrain within 8 km.
