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
