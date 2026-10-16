# Missions (`.mis`)

Status: **partial**. A section file (`sectfile.md`):
- `[MAIN]`:
  - `MAP <path>` is relative to `maps/` (e.g. `Moscow/load.ini`),
  - `TIME <hours>`, `CloudType`, `CloudHeight`,
  - `player <wing>`, `army`, `playerNum <index in the wing>`.
- `[Wing]`: names of all flights.
- `[<wing>]`:
  - `Planes <n>`, `Skill`,
  - `Class air.<Name>` is a runtime aircraft class, `com.maddox.il2.objects.air.<Name>`,
  - `Fuel <percent>`, `weapons <loadout>`.
- `[<wing>_Way]`: waypoints, one per line: `TYPE x y z speed [target] …`.
  - Types: `TAKEOFF`, `NORMFLY`, `LANDING`, `GATTACK`, ….
  - `x, y` are world metres; `z` is altitude in metres; speed is km/h.
- Waypoint types may carry a suffix (`TAKEOFF_002`); compare by prefix.
- `[NStationary]`, `[Buildings]`: placed objects, see below. Ground units (`[Chiefs]`, `[*_Chief_Road]`): TODO.

## Placed objects
**Verified** from the runtime's mission loader.

`[NStationary]` lines: `name class army x y yaw [timeLen delay|country skill slowfire paramFile number]`.
`class` is relative to `com.maddox.il2.objects.`. `yaw` goes through `Orient.set`, so it is **clockwise**
from east (unlike `.static` houses).
- `vehicles.stationary.Stationary$X`, `vehicles.artillery.Artillery$X`: section `[X]` of
  `com/maddox/il2/objects/technics.ini`, mesh `MeshSummer` (`MeshWinter`, `MeshDesert` on those maps,
  defaulting to `MeshSummer`; `Mesh*Damage` when destroyed). Placed on the mesh's `Ground_Level` hook and tilted
  to the ground normal.
- `ships.Ship$X`: section `[X]` of `com/maddox/il2/objects/ships.ini`, `Mesh` (a `.him`). At the water surface
  (`z = 0` on shipped maps).
- `vehicles.planes.Plane$X`: `technics.ini [X] Class air.Y` names an aircraft class; its default model is
  shown with the gear down ("Static aircraft").
- `vehicles.stationary.Smoke$*`, `Campfire$*`, `Siren$*`, `vehicles.lights.Searchlight$*`,
  `vehicles.radios.Beacon$*`: effects and sounds (TODO).

`[Buildings]` lines: `name class alive x y yaw`, `class` relative to `com.maddox.il2.objects.buildings.`
(`House$*`, `Plate$*`; `static.md`). `alive` 0 shows `MeshDead`. Yaw is clockwise. Plates placed by a mission
sit 0.04 m higher than map plates.

### Static aircraft
With the gear down, take the `_ClipLGear`, `_ClipRGear` and `_ClipCGear` hooks (x forward, z up). Let `M` be
the mid-point of the two main wheels and `C` the third wheel: `pitch = −atan2(Mz − Cz, Mx − Cx)` in degrees
(+180° if `Mx < Cx`), and `height` = distance from the origin to the line through `M` and `C`. If a hook is
missing or the flight model has `[Gear] FromIni ≠ 0`, use `[Gear] H` and `Pitch` (default −0.5). The aircraft
stands `height` above the ground, nose up by `pitch`, tilted to the ground normal. OpenIL2 also uses the
`[Gear]` figures when it has no decoded gear animation for the class.


## Aircraft classes
An aircraft class's static data names its flight model and models:
- the string constant `FlightModels/<name>.fmd`,
- one or more `3DO/Plane/<model>/hier.him` paths; the variant without a parenthesised country suffix is the default.

OpenIL2 reads these from the user's install by decrypting the class (`krypto.md`, "Game classes") and scanning
its constant pool. Nothing else in the class is used.
