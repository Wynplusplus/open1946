# Meshes (`.msh`)

Status: **verified** on aircraft part meshes. The section layout is the same for text and binary encodings
(`sectfile.md`, `binsect.md`). In the text form each line holds a whitespace-separated row; in the binary
form numeric sections are column-typed.

Coordinates are in metres, Z up. An aircraft's nose points along −X (to be confirmed against hooks).

## Sections

- `[Common]`
  - `NumBones <n>`,
  - `FramesType Single|...`,
  - `NumFrames <n>` (vertex-animation frames).
- `[LOD]`: one row per LOD level, holding the distance up to which that level is used.
  Level 0 uses the unprefixed sections; level `i ≥ 1` uses sections prefixed with `LOD<i>_`.
- `[Hooks]`: rows `<name> <parent>`, where the parent is usually `<BASE>`.
- `[HookLoc]`: one row per hook, 12 numbers: a 3×3 rotation matrix (row-major), then the translation.
- `[Materials]`: one material name per row. The file is `<name>.mat` in the mesh's directory.
- `[FaceGroups]`:
  - first row `<numVertices> <numFaces>`,
  - then one row per group: `material startVertex vertexCount startFace faceCount 0`.
- `[Vertices_Frame<k>]`: one row per vertex, `x y z nx ny nz`.
- `[MaterialMapping]`: one row per vertex, `u v`.
- `[Faces]`: one row per triangle, three vertex indices **relative to the face group's `startVertex`**.
- `[ShVertices_Frame<k>]` / `[ShFaces]`: a simplified closed hull for stencil shadows.
  Indices are absolute into the shadow vertices.
- `[CoCommon]`, `[CoCommon_b<B>]`, `[CoCommon_b<B>p<P>]`, `[CoVer0_b<B>p<P>]`, `[CoNeiCnt_...]`, `[CoNei_...]`,
  `[CoFac_...]`: collision geometry, organized in blocks and parts. Each part has a `Type` (e.g. `Mesh`) and a
  `Name`; the names are referenced by `CollisionObject` in `.him` files. (Details TODO.)
