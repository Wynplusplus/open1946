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
