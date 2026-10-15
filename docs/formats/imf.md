# IMF images

Status: **verified** for type 1 (aircraft skins). Type 2 is inferred from the decoder's structure.

Many textures named `*.tga` (aircraft camouflage skins in particular) are actually IMF images, recognised by
their magic rather than their extension. All values are little-endian.

## Header (12 bytes)

| Offset | Type | Meaning |
|-------:|------|---------|
| 0 | [u8;6] | `"IMF" 1A "10"` |
| 6 | u8 | 0 |
| 7 | u8 | image type: 0 = RGB, 1 = RGB + alpha, 2 = JPEG RGB + alpha |
| 8 | u16 | width (1..=6144) |
| 10 | u16 | height (1..=4096) |

## Filtered planes
A *filtered plane* with `bpp` bytes per pixel is stored as `height` filter bytes (one per row, all rows
first), followed by `width * height * bpp` bytes of filtered pixel data, rows top to bottom.
Filters are those of PNG (RFC 2083), with `bpp` as the filter distance:
0 None, 1 Sub, 2 Up, 3 Average, 4 Paeth.

## Layout by type
- **0**: an RGB plane (`bpp = 3`, byte order R, G, B) at offset 12. Fully opaque.
- **1**: an RGB plane at offset 12, followed by an alpha plane (`bpp = 1`).
- **2**: a u32 `n` at offset 12, then `n` bytes of baseline JPEG holding the RGB image, then an alpha plane.

The file size must equal exactly the sum of the parts; the engine rejects anything else as damaged.
