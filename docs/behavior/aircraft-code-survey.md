# Survey: where aircraft behavior lives

Measured on v4.13.4 (2026-10-15). The question was how much of an aircraft's behavior is data that OpenIL2
can read, and how much is code that has to be reimplemented.

## Data (readable today)
- **Flight models** (`.fmd`, 158 referenced): `[Aircraft]` type and size, `[Mass]`, `[Controls]` (which
  controls exist, default trims), `[Squares]` (surface areas), `[Arm]` (lever arms, CG shifts), `[Toughness]`
  (damage per part), `[Engine]` (family + sub-model), `[Gear]`, `[Params]` (critical AoA, Vmax at altitude,
  climb, turn time, control sensitivity), `[Polares]` (lift/drag polar parameters for clean and flapped
  configurations, plus moment coefficient tables), `[SOUND]`.
- **Engines** (`.emd`, 27 families, many sub-models each): power, boost, RPM limits, reduction gear,
  propeller diameter and pitch governor, supercharger stages and ATA curves, temperatures and overheat timing.
- **Meshes and hierarchies**: parts, damage states, hooks for guns, bombs and lights.

## Shared engine code (reimplement once)
`com.maddox.il2.fm`: about 20k lines in 13 main classes, among them motor/engine (4.4k), aircraft state and
damage effects (3.8k), flight-model core and polars (2.6k), gear and ground handling (1.8k), and controls (1.6k).
It is the same for every aircraft, driven by the data above. It will be specified in `docs/behavior/` and
reimplemented in Rust.

## Per-aircraft code (539 source files, ~187k lines, including cockpits)
Most frequent overrides, by count across classes:
- **Animation**: `moveGear` (378), `moveFlap`, `moveSteering`, `moveRudder`, `moveAileron`, `moveElevator`,
  `moveFan`, `moveWheelSink`. Mostly "rotate node X by f(control) degrees", which fits a data table.
- **Cockpits**: `reflectWorldToInstruments`, `reflectCockpitState`, `toggleLight`, focus enter/leave.
  Gauge-needle mapping is also table-like.
- **Damage**: `hitBone`, `cutFM`, `doWoundPilot`, `doMurderPilot`.
- **Weapons**: `moveGun`, `clipAnglesGun`, `turretAngles`, `doGunFire`.
- `update()` (151 aircraft): mostly reads engine and AoA state for visual effects (soot, radiators, slats,
  turrets, arrestor hooks). A few touch inputs: automatic flaps, power, nitro.

## Conclusion
Flight *physics* is generic code plus data, so it can be reimplemented once. Per-aircraft code is large but
repetitive, mostly animation and cockpit glue, and can be converted into per-aircraft data tables (generated
by an analysis-side tool, reviewed by hand) plus a small set of Rust behaviors for the special cases. A full
rewrite is feasible; cockpits and per-aircraft animation are the bulk of the volume, not the physics.
