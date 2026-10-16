# Roadmap

## M1: archives and asset viewer (done, see README)
Open items carried forward:
- Blended and additive material layers, multi-layer materials, depth offset (decals).
- LOD selection by distance.
- Name recovery for the remaining ~25% of entries (sounds, some map data, `cod/` entries).
- `0xCA` archives beyond `filesserver.SFS`; the one unidentified protected directory in `vfs.md`.

## M2: world (done)
Done: land-type texturing, water, sky gradient and fog, map objects (`*.static` houses, tree lines, plates),
mission start positions.
Done since: forest canopy, roads and rails, bridges, mission `[NStationary]` and `[Buildings]` objects.
Also done: sun and sky colours from time, date and latitude; chase and free orbit cameras.
Not shipped: the `[CUBE]` sky textures named in `load.ini` are not in the install.
Moved on: bridge ramps (M4, with damage), smoke/fire effects (M4, with the effects system). Gear animation
is not decoded for some classes (e.g. La-5FN): M3.

## M3: flight (done)
Flight model, engines of every type (piston, jet, rocket, booster, pulse jet, tow), emergency boost, fuel,
wave drag, controls (`controls.md`), engine management, landing gear (`gear.md`), joystick, part animation.
All 404 aircraft fly (`validation.md`). Not modelled yet: mixture and magnetos, negative-g cut-out, water
landings and carrier decks (with M5), structural failure (with damage, M4).

## M4: aircraft depth
Cockpits, HUD, weapons and ballistics, damage model (hidden `_D1.._D3` nodes, `Separable` parts), sound.

## M5: simulation
AI pilots and gunners, ground and naval units, Quick Mission Builder, menus and UI (`i18n`).

## M6: everything else
Campaigns (DGen/NGen), track recording (`.ntrk`), multiplayer, DeviceLink.
