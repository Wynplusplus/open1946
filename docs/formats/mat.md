# Materials (`.mat`)

Status: **verified** syntax (parses all 43464 shipped files). Rendering semantics are partly inferred.

A section file read by the engine's native reader (case-insensitive keys).

- `[ClassInfo]`:
  - `ClassName TMaterial`,
  - `BasedOn <file.mat>`: inherit every key not given here. The path is relative to this material's directory.
- `[General]`: `tfDoubleSide`, `tfShouldSort` (needs depth sorting / translucent), `tfDropShadow`, `tfGameTimer`.
- `[LightParams]`: `Ambient`, `Diffuse`, `Specular`, `SpecularPow`, `Shine`.
- `[Layer<N>]`, with `N` from 0 and contiguous:
  - `TextureName <file>`: relative to the material's directory, usually `.tga`.
  - `PaletteName`, `Frame`, `VisibleDistanceNear/Far`.
  - `TextureCoordScale u0 v0 su sv`.
  - `ColorScale r g b a`.
  - `AlphaTestVal` (used when `tfTestA 1`).
  - flags: `tfWrapX/Y`, `tfMinLinear`, `tfMagLinear`, `tfMipMap`, `tfBlend`, `tfBlendAdd`, `tfTestA`, `tfTestZ`,
    `tfModulate`, `tfNoTexture`, `tfNoWriteZ`, `tfDepthOffset`, `tfTranspBorder`, `tfTestZEqual`, `tfNoCompress16Bit`,
    `tfCompressMajorAlpha`, `tfNoDegradation`, `tfAnimatePalette`, `tfAnimateSkippedFrames`, `tfUpDateClear`.
- Sections with a leading underscore (`[_Layer1]`) are disabled.

## Texture channels
The engine chooses the GPU format from the image's bytes per pixel:

| bytes/pixel | format |
|-----:|--------|
| 1 (grayscale TGA) | **alpha only**: color comes from the material/lighting, and the gray value is alpha |
| 2 | luminance + alpha |
| 3 | RGB |
| 4 | RGBA |
| palettized | color index |

So a black-on-white grayscale decal combined with `tfTestA` cuts out its black parts.

## Render state (inferred)
- `tfBlend 1`: standard alpha blending (`src·a + dst·(1−a)`), drawn after opaque geometry.
- `tfBlendAdd 1`: additive blending.
- `tfTestA 1`: discard fragments with alpha below `AlphaTestVal` (default 0.5).
- `tfDepthOffset 1`, in `[General]` or in a layer: polygon offset so decals win over the coplanar skin.
  `il2setup.ini` defaults are `PolygonOffsetFactor=-0.0625`, `PolygonOffsetUnits=-1`.
