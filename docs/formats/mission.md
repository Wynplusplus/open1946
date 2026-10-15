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
- Ground units (`[Chiefs]`, `[*_Chief_Road]`), static objects (`[NStationary]`), `[Buildings]`, etc.: TODO.

## Aircraft classes
An aircraft class's static data names its flight model and models:
- the string constant `FlightModels/<name>.fmd`,
- one or more `3DO/Plane/<model>/hier.him` paths; the variant without a parenthesised country suffix is the default.

OpenIL2 reads these from the user's install by decrypting the class (`krypto.md`, "Game classes") and scanning
its constant pool. Nothing else in the class is used.
