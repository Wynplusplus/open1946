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
- Collision geometry (see "Collision parts").

## Collision parts
**Verified** on a sample of 3000 shipped meshes (860 parts): every collision part is a single-frame
`Mesh` and every file has one block.

- `[CoCommon]`: `NBlocks <n>`.
- `[CoCommon_b<B>]`: `NParts <n>` for block `B`.
- `[CoCommon_b<B>p<P>]`: `Type Mesh`, `NFrames <n>`, `Name <name>` for part `P`.
- `[CoVer<F>_b<B>p<P>]`: the part's vertices in frame `F`, rows `x y z` in mesh coordinates.
- `[CoNeiCnt_b<B>p<P>]`: per vertex, how many neighbours it has; `[CoNei_b<B>p<P>]`: the neighbour
  indices of all vertices in order (vertex adjacency, for convex queries).
- `[CoFac_b<B>p<P>]`: triangles, rows of three vertex indices.

The parts are closed (convex in the samples looked at) hulls. On aircraft, a chunk's mesh holds the
chunk's own hull, named `x<part>` (`xcf1`, `xwinglin1`, `xengine1`), and internal parts named
`xx<what>` (`xxtank1`, `xxeng1`, `xxcontrols5`, `xxarmorp1`, `xxspar…`, `xxpnm`, `xxcannon01`) that
the class's damage code interprets (`behavior/damage.md`). The `.him` node lists the parts it uses
with `CollisionObject`; a leading `.` (`.xxtank1`) is not part of the name.
