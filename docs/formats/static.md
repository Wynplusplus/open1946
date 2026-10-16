# Map static object files (`*.static`)

Status: **verified** structure on Slovakia, Moscow and Kuban. All values are big-endian (Java `DataInput`).

A map's `load.ini` lists these under `[static]`.

1. `i32` version = `-65535`.
2. Bridges: `i32 n`, then `n` records of 5 `i32` and 1 `f32` (24 bytes; meaning TODO).
3. Placed objects:
   - `i32 nClasses`, then that many class names (Java modified UTF-8: `u16` length + bytes).
   - If `nClasses > 0`: `i32 n`, then `n` records of `i32 classIndex, f32 x, f32 y, f32 yaw`.
   - Coordinates are world metres. `yaw` is in degrees **clockwise from east** and may contain whole extra turns
     (for example `137430`), so it is taken mod 360. The forward direction is `(cos yaw, −sin yaw)`.
   - Shipped maps use a single class here, `com.maddox.il2.objects.air.Runaway`: runway ends.
4. Houses, trees and ground plates:
   - `i32 nClasses` + names (e.g. `com.maddox.il2.objects.buildings.House$Urban_05`).
   - If `nClasses > 0`: `i32 nBlocks`, then per block `i32 key`, `i32 n`, and `n` records of two `i32`
     (`code`, `pos`). **Verified** on Slovakia (700,499 objects), Kuban and Moscow.
   - `key`: low 16 bits are the cell column `cx`, high 16 bits the cell row `cy`, in 200 m cells from the
     world origin (`x = 200·cx`, `y = 200·cy`; not the height-map row order).
   - `code`: bits 0–14 class index (the runtime replaces an index ≥ `nClasses` with 0), bit 15 "starts
     destroyed", bits 16–31 a signed 16-bit yaw in units of 360/32000 degrees, **counter-clockwise from east**
     (unlike the runway records above).
   - `pos`: low and high signed 16-bit halves are the x and y offsets from the cell corner, in units of
     200/32000 m.
5. If any bytes remain, the airdrome layout follows as three polyline lists: runways, taxiways and parking
   spots. Each list is `i32 nLines`, then per line `i32 nPoints` and `nPoints` × (`f32 x, f32 y`).

## Runways and takeoff
- **Opposite ends.** For a runway end A, its opposite end is an end B lying 800–2500 m behind A along A's
  forward axis, at most 15 m to the side, and facing the other way (dot product of the two directions < −0.9).
- **Picking the runway.** For a `TAKEOFF` waypoint, the runtime picks the runway end nearest to the waypoint
  (ignoring height), and none if the nearest is more than 15 km away. Take `d` = distance to that end's
  opposite end, or 1000 m when it has none.
- **Placing the aircraft.** It is placed `d` metres behind the chosen end, facing along its direction, at
  `ground height + [Gear] H` with nose up by `[Gear] Pitch`.
- **More aircraft.** Further aircraft of the same flight stand at increasing offsets of
  `2·collision radius + 20` m.

## Object classes (`com/maddox/il2/objects/static.ini`)
Each class name maps to the `static.ini` section whose name **ends with** the short class name
(`House$Urban_05` matches `[buildings.House$Urban_05]`).

- `House$*`:
  - `MeshLive` / `MeshDead`: `.sim` files (a hierarchy in `.him` syntax, `him.md`); `MeshDead` may be empty.
  - `equals <section>`: take `AddHeightLive`, `AddHeightDead`, `AlignToLand`, `Body`, `Panzer` from that
    section instead.
  - Placement: `z = ground height + AddHeight (clamped to ±100) + lift`, where `lift` is `−z` of the mesh's
    `Ground_Level` hook, or `−(bounding-box minimum z)` without one. Pitch and roll are zero unless
    `AlignToLand > 0`, which tilts the object to the ground normal.
  - `Body` (`WoodSmall`, `WoodMiddle`, `RockMiddle`, `RockBig`, `RockHuge`, `FuelSmall`, …) and `Panzer`
    set the damage material (TODO: damage model).
- `Plate$*` (runways, taxiways, aprons, platforms): `Mesh <file.sim>`. Placed 0.25 m above the ground and
  always tilted to the ground normal.
- `Furniture$*` sections exist, but the install has no `Furniture` runtime class, so the original cannot
  spawn them; OpenIL2 skips them too.
- The class whose name contains `TreeLine` also feeds the landscape's tree marking (TODO).

The runtime spawns the objects of a 200 m cell only while that cell is near the camera. Drawing distances come
from each mesh's LOD table (`msh.md`).
