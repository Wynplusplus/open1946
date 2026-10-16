# Controls

Status: **specified from the runtime's control code** for the parts below.

## Which controls exist (`.fmd [Controls]`)
Each key is 0 or 1; other values make the flight model invalid.
- **Surfaces and trim.** `CAileron`, `CElevator`, `CRudder`, plus trims `CAileronTrim`, `CElevatorTrim`,
  `CRudderTrim`. The trim levers start at `DefaultAileronTrim`, `DefaultElevatorTrim` and
  `DefaultRudderTrim`, and can be moved only where the trim control exists.
- **Flaps.** `CFlap`. `CFlapPos` 0–3: with 3 the lever has four positions (0, 0.2 combat, 0.33 take-off,
  1 landing), below 3 only up (0) and down (1). Without `CFlap` the flaps stay at 0.
- **Other.**
  - `CDiveBrake`: dive brakes.
  - `CUndercarriage`: retractable gear; without it the gear is fixed down and has no gear drag.
  - `CDifferentialBrakes`, `CLockTailwheel`, `CWheelBrakes` (default 1).
  - `CArrestorHook`, `CWingFold`, `CCockpitDoor`, `CBombBay`, `CStabilizer`.
- **Speed limits.** `CAileronThreshold` (360), `CRudderThreshold` (360), `CElevatorThreshold` (403.2), in
  km/h.

## Per step
`filter(target, x, τ, r)`: `x ← target + (x − target)·e^(−dt/τ)`, then move `x` towards the target by up to
`r·dt`.

- **Speed limits.** With `V` the body-x airspeed (m/s) and thresholds `A`, `R`, `E` (m/s):
  - aileron travel `max(0.2, A³/V³)` above `A`;
  - rudder `max(0.2, R²/V²)` above `R`;
  - elevator `kE = max(0.4, E²/V²)` above `E`.
- **Trim.** The applied trim follows its lever with `filter(…, τ = 999.9, r = 0.25)`. A stick input `s`,
  clamped to the travel limit, gives `(1 ∓ |trim|)·s + trim`, using `−` when trim and `s` have the same sign.
- **Surfaces.**
  - Ailerons: `filter(…, τ = 0.2(1 + 0.3|s|), r = 0.025)`.
  - Elevator: `filter(…, τ = 0.3(1 + 0.3|s|), r = 0.022)`, then `elevator = clamp(ev − 0.25(1 − kE), ±1)`.
  - Rudder: `filter(…, τ = 0.35(1 + 0.3|s|), r = 0.025)`.
- **Throttle.** The player's throttle acts directly.
- **Flaps.** `filter(…, τ = 999, r)`. Raising: `r` goes from 0.15 to 0.25 /s between 150 and 280 km/h.
  Lowering: from 0.15 to 0.02 /s.
- **Gear.** `filter(…, τ = 999.9, r = 0.2)` (5 s travel). **Dive brakes:** `r = 0.5`.
- **Wheel brakes.** Rise at 0.3 /s and release at once.
- **Trim keys.** Each press moves a trim lever by 0.00625.

## Drag terms
Added to each wing half's drag coefficient:
- `0.5·GearCX·gear` per main gear leg, `GearCX` = 0.035 (0 for fixed gear);
- `radiatorCX·(radiator + cockpit door + bomb bay)`, `radiatorCX` = 0.003 (radiators: TODO);
- `AirbrakeCxS·airbrake`.
