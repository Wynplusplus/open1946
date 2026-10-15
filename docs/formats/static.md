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
4. Houses and trees:
   - `i32 nClasses` + names.
   - If `nClasses > 0`: `i32 nBlocks`, then per block `i32 key` (low 16 bits x cell, high 16 bits y cell),
     `i32 n`, and `n` pairs of `i32` (object code, packed position; TODO).
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
