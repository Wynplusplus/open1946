# Roadmap

## M1: archives and asset viewer (done, see README)
Open items carried forward:
- Blended and additive material layers, multi-layer materials, depth offset (decals).
- LOD selection by distance.
- Name recovery for the remaining ~25% of entries (sounds, some map data, `cod/` entries).
- `0xCA` archives beyond `filesserver.SFS`; the one unidentified protected directory in `vfs.md`.

## M2: world
Terrain with land-type texturing (`TypeMap` + `[FIELDS]`), water, forests, roads; sky cube, fog and sun;
static objects (`*.static`, `static.ini`); mission (`.mis`) loading; free camera.

## M3: flight
Flight-model tables (`krypto.md`: container `gui/game/buttons`), a generic flight model, joystick input,
aircraft systems. Per-aircraft behavior that lives in Java code is documented in `docs/behavior/` and
reimplemented as data or Rust.

## M4: aircraft depth
Cockpits, HUD, weapons and ballistics, damage model (hidden `_D1.._D3` nodes, `Separable` parts), sound.

## M5: simulation
AI pilots and gunners, ground and naval units, Quick Mission Builder, menus and UI (`i18n`).

## M6: everything else
Campaigns (DGen/NGen), track recording (`.ntrk`), multiplayer, DeviceLink.
