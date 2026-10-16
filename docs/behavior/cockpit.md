# Cockpit view

## Which cockpit an aircraft uses

An aircraft class names its cockpit in its static initializer as the property `cockpitClass`
(one class, or an array of classes for multi-crew aircraft whose first entry is the pilot's
seat). The cockpit class's constructor passes its hierarchy file, a path under `3do/cockpit/`
ending in `.him`, to the cockpit base class. OpenIL2 finds the first such string constant in the
cockpit class.

Of the 404 classes in `air.ini`, 241 have a cockpit. The rest are AI-only types and the "ace"
variants (`BF_109G6GRAF`, `LA_7KOJEDUB`, …), which the original doesn't let the player fly from
the cockpit either. For these OpenIL2 falls back to the chase view.

## Placement

The cockpit actor sits on the aircraft with a hook that copies only the orientation. Its origin
is the aircraft's origin and its axes are the body axes, so the cockpit mesh is in body
coordinates. The pilot's eye is the cockpit mesh's `CAMERA` hook (`CAMERAAIM` is the eye when
aiming through the gunsight). The world camera takes the same point: the aircraft position plus
the rotated `CAMERA` offset. Head movement turns the view around that point.

The original draws the cockpit in its own pass with its own camera and near clip plane. OpenIL2
does the same: the world is drawn with a 0.5 m near plane, then depth is cleared and the cockpit
is drawn with a 2 cm near plane, so instruments close to the eye neither clip nor fight with
distant terrain for depth precision. The default horizontal field of view is 90°.

## Materials

Cockpits use untextured layers (`tfNoTexture 1`) that draw their `ColorScale` colour. An example
is the `Mask` material: black with zero alpha and blending on, which hides geometry without
being seen. See `docs/formats/mat.md`.

Cockpits ship 1×1 placeholder textures for the aircraft's skin materials (`Gloss1D0o`,
`Matt1D0o`, `Gloss1D1o`, …, `OverlayD0o`). Most cockpit constructors replace them with the
aircraft's own materials (`mesh.materialReplace("Gloss1D0o", aircraft.material(...))`), so the
fuselage seen from inside wears the outside skin. OpenIL2 does this for every cockpit by name.

Materials named `…_night` are the lit versions of gauge faces, drawn as extra face groups.
`setNightMats(on)` enables them when the pilot switches on the panel lights. They are off at
start, so OpenIL2 leaves them out.

## OpenIL2 controls

F1 cockpit, F2 free orbit camera, F3 chase camera. In the cockpit, dragging with the left mouse
button turns the head (up to 150° either side, 35° down, 80° up). These limits are OpenIL2's own.
`IL2_HIDE=node,node` hides cockpit nodes (diagnostics).

## Instruments

Each cockpit class moves its own needles. The constructor registers one or more interpolator
objects (`interpPut`). Each game tick, an interpolator's `tick()` copies "new" values to "old" and
samples fresh ones from the flight model, usually with first-order smoothing such as
`new = (10·old + x)/11`. Every frame, `reflectWorldToInstruments(f)` sets mesh nodes from them:
`mesh.chunkSetAngles("Z_…", a, b, c)`, `chunkSetLocate`, `chunkVisible`, with `f` being the
fraction between the last two ticks. Gauge scales are often piecewise linear tables read with
`floatindex`, and the code clamps and maps ranges with `cvt(x, x0, x1, y0, y1)`.

OpenIL2 runs this code from the user's installed classes (an extension of the animation importer,
`docs/behavior/animation.md`, the "Importer" section). The design is OpenIL2's own:

- The cockpit object is constructed once with its own field initializers and constructor body.
  Engine base constructors are skipped. Inner classes (the interpolator and the variable holder)
  and three world helpers, `AnglesFork` (angle smoothing that wraps), `Pitot` (indicated
  airspeed) and `Atmosphere`, are interpreted too.
- Everything reached through the flight model (`this.fm`) or other engine classes is symbolic.
  Field reads, array elements and calls extend a path, for example
  `fm.EI.engines[0].getRPM()` or `fm.actor.getGunByHookName("_MGUN01").countBullets()`. A
  number read at a path comes from a per-frame table filled from the simulation, and is 0 when
  the table doesn't have it. `World.getPlayerFM()` and `fm.actor.FM` are the same flight model.
  `isChunkVisible(…)` answers true and `materialFind(…)` answers −1 (no night materials).
- A helper call that the interpreter can't follow returns its type's default, so the remaining
  instruments still move.
- Interpolators step at the simulation tick (30 ms) and `reflectWorldToInstruments(1)` runs
  each frame. All 241 cockpits construct and update this way.

## World values

The table uses the original's units. The table below is OpenIL2's mapping from its simulation:

| Path | Value |
|------|-------|
| `fm.getAltitude()`, `fm.Loc.z` | altitude above sea level, m |
| `fm.getSpeedKMH()` | true airspeed along the body x axis, km/h (`Pitot.Indicator` turns it into IAS) |
| `fm.getVertSpeed()` | climb rate, m/s |
| `fm.Or.getKren()` | bank, degrees, right wing down positive |
| `fm.Or.getTangage()` | pitch, degrees, nose up positive |
| `fm.Or.getYaw()` | heading counter-clockwise from east (+x), degrees |
| `fm.Or.azimut()` | 360° − yaw (cockpit code adds its own 90° offsets for compass cards) |
| `fm.getBallAccel()` | gravity minus acceleration in body axes, m/s² (slip ball) |
| `fm.getW()` | body angular rates, rad/s |
| `fm.M.fuel`, `fm.M.maxFuel` | fuel, kg |
| `fm.CT.*Control`, `fm.CT.get…()` | control positions as the pilot sets them, −1..1 or 0..1 |
| `fm.EI.engines[i].getRPM()` | engine rpm |
| `fm.EI.engines[i].getManifoldPressure()` | manifold pressure, atmospheres |
| `fm.EI.engines[i].tOilIn`, `tOilOut`, `tWaterOut` | temperatures, °C |
| `fm.EI.engines[i].getPropPhi()` | propeller blade angle, rad |
| `World.getTimeofDay()` | hours since midnight |
| `World.cur().Atm.ro0`, `T0`, `P0` | sea-level density 1.225 kg/m³, 288.15 K, 101325 Pa |

`IL2_TRACE=1 openil2 --screenshot … --cockpit` lists the paths a cockpit read that have no value
yet. The example `cargo run -p il2-data --example instruments -- --all` runs every cockpit.
