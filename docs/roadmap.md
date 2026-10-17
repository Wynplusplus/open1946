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

## M4: aircraft depth (done)
Cockpits with every instrument (`cockpit.md`), HUD, weapons and ballistics (`weapons.md`), house and aircraft
damage (`damage.md`), particle effects (`effects.md`), sound (`sound.md`).
Moved on to M5, which brings something to shoot at: bullet and blast hits on aircraft (the per-class
detailed damage bones `xx…` and the generic chunk damage), smoke and fire from damaged engines and wrecks,
bridge ramps.

## M5: simulation (done)
AI flights from missions on the simplified AI flight model: takeoff, waypoint autopilot, formation,
air combat with gunnery, evasion, level bombing, landing (`ai.md`). Also gunners in turret arcs from
the classes' code, bullet and blast hits on aircraft, anti-aircraft guns, and moving ground and naval
columns from `[Chiefs]` (`ground.md`). Menus with single missions and briefings, and a Quick Mission
Builder on the install's templates (`menus.md`).
Interim designs to be replaced by ports of the runtime: air combat and ground-attack maneuvers
(`Maneuver`), formations (`AirGroup`), `ChiefGround` road movement with bridges and collisions,
armed ground units and ships, mission objectives and results.
Not yet: carrier operations, water landings, the original GUI screens.

## M6: everything else (done)
DeviceLink per the install's `DeviceLink.txt` (`devicelink.md`). Track recording and playback in
OpenIL2's own state format (`formats/track.md`). Mission objectives and results, Pilot Career
through the static campaigns, and dynamic campaigns from OpenIL2's own generator on the DGen data
(`campaign.md`). Multiplayer over OpenIL2's own UDP protocol (`multiplayer.md`).

## Open: fidelity to the original
What works today through OpenIL2's own designs, and would be replaced by ports of the runtime:
- the original network protocol: playing `records/*.ntrk` and `*.TRK`, and joining original
  servers;
- AI: the runtime's `Maneuver` (air combat, ground attack, takeoff and landing maneuvers) and
  `AirGroup` formations; ground and naval units with `ChiefGround` road formations, bridges,
  armed vehicles and ships;
- the detailed per-class damage model (`xx…` bones, `hitBone`) and the meshes' collision shapes;
- DGen's own generator logic (front movement, squadron rosters, ranks, awards);
- the original GUI screens; carrier operations (decks, arrestor wires, catapults).
