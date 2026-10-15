# Hierarchical meshes (`.him`)

Status: **verified** (parses all 1555 shipped files).

A section file (`sectfile.md`) that describes an object as a tree of mesh parts.

- `[_ROOT_]`: object-level data.
  - `VisibilitySphere <radius>`,
  - `CollisionObject sphere <r> <x> <y> <z>`.
- Every other section is a node named by its section name:
  - `Mesh <name>`: mesh file `<name>.msh` in the same directory.
  - `Parent <node>`: `_ROOT_` or the name of another node.
  - `Attaching m00 m01 m02 m10 m11 m12 m20 m21 m22 tx ty tz`: the node's transform relative to its
    parent. The engine reads numbers C-`atof` style; at least one shipped file has trailing junk (`0.12'`).
  - `Hidden`: the node's own mesh starts hidden. Its children are *not* affected (for example, a main gear
    leg is the child of a hidden gear-bay node). Damage-state variants (`*_D1`, `*_D2`, …) start hidden.
  - `Separable`: the part can break off.
  - `CollisionObject <name>`: a collision part of the node's mesh (see the `Co*` sections in `msh.md`).

The rows `m0*`, `m1*`, `m2*` are the images of the local X, Y and Z axes (row-vector convention), so
`p_parent = x·m0 + y·m1 + z·m2 + t`.
