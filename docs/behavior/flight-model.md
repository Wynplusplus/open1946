# Flight model (aircraft dynamics)

Status: **specified from the runtime's flight model** for the parts listed here. Sections marked
*OpenIL2 simplification* are knowingly different from the original until the corresponding part is specified.

## Frames and units
- SI units throughout: metres, kg, s, N. Angles inside aerodynamic formulas are in **degrees**.
- World frame: x east, y north, z up (see `docs/formats/map.md`).
- Body frame: **x forward, y left, z up** (right-handed). Moments and angular velocity `W = (p, q, r)` are
  right-handed about these axes, so positive `p` rolls right (left wing up), positive `q` pitches nose down,
  and positive `r` yaws left.
- `Vflow`: the air velocity relative to the aircraft, expressed in the body frame (`Vworld − wind`, rotated into the body).
- `AOA = −atan2(Vflow.z, Vflow.x)`, `AOS = atan2(Vflow.y, Vflow.x)`, both in degrees.

## Time step
The simulation tick is **30 ms**. A step whose travel `V·dt` exceeds 5 m is split into two half steps,
recursively. Some damping below is applied **per step**, so the step length is part of the behavior.

## Data
From the `.fmd` file (`docs/formats/krypto.md`):
- `[Aircraft] Type` (airframe scheme 0–7), `Length`, `Wingspan`, `Crew`.
- `[Mass]`:
  - empty mass = `Empty + Oil + 90·Crew`,
  - `TakeOff` is the max weight,
  - `Fuel` is the max fuel (the tank starts full),
  - current mass = empty + fuel (+ nitro + stores).
- `[Squares]`:
  - `Wing` is the total wing area,
  - `Wing_In/Mid/Out` are the areas per wing half (each applies to both halves),
  - `Stabilizer`, `Elevator`, `Keel`, `Rudder`, `Aileron`, `Flap`,
  - `AirbrakeCxS`.
  - Check: if `2·(In+Mid+Out)` differs from `Wing` by more than 10%, either `Wing` is replaced by twice
    the sum (when the sum is larger) or each section is set to `Wing/6`.
- `[Arm]` (metres from the CG reference):
  - `Wing_In/Mid/Out` (spanwise),
  - `Stabilizer` and `Keel` (aft distance of the tail surfaces),
  - `Wing_V` (dihedral effect),
  - `GCenter`, `GCenterZ`,
  - `GC_AOA_Shift`, `GC_Flaps_Shift`, `GC_Gear_Shift`.
- `[Params]`: `SensRoll`, `SensPitch`, `SensYaw` (control effectiveness), `SpinCxLoss`, `SpinCyLoss`, speed limits.
- `[Polares]`: the wing polar (below) and the Mach table `mc3`, `mc4`, `mm`, `mz`.
- `[Gear]`: `H`, `Pitch`, `SinkFactor` (plus optional `SpringsStiffness`, `TailStiffness`).
- `[Engine] Engine<i>Family/SubModel`: the section `[SubModel]` of `FlightModels/<Family>.emd`.

## Inertia
Let `a = (0.35 L)²`, `b = (0.125 L)²`, `c = (0.2 span)²`, `d = (0.07 L)²` (L = length). The unit inertia `J0` depends on the scheme:

| scheme | Jz (yaw) | Jy (pitch) | Jx (roll) |
|---|---|---|---|
| 0, 1 | 0.2a + 0.4b + 0.4c | 0.2a + 0.4b + 0.4d | 0.6d + 0.4c |
| 2 | 0.2a + 0.1b + 0.7c | 0.2a + 0.1b + 0.7d | 0.3d + 0.7c |
| 3 | 0.2a + 0.2b + 0.6c | 0.2a + 0.2b + 0.6d | 0.2d + 0.8c |
| 4–7 | 0.25a + 0.15b + 0.6c | 0.25a + 0.15b + 0.6d | 0.4d + 0.6c |

The full inertia is `J = J0 · empty mass` (plus stores, later).

## Polars
A polar maps an angle of attack α (degrees) to lift and drag coefficients. It has two configurations, clean
(`_0`) and flapped (`_1`); flap position `f ∈ [0,1]` blends them with weight `k = 0.25f + 0.75√f` for the
lift parameters and `f` for the drag parameters:
- `Cy0 = Cy0_0 + (Cy0_1 − Cy0_0)k`, and likewise `CyCritH`, `CyCritL`, `AOACritH`, `AOACritL`.
- `CxMin` and `parabCxCoeff` blend linearly in `f`.

Derived values:
- `AOALineH = 2(CyCritH − Cy0)/lineCyCoeff − AOACritH` and `parabH = 0.5·lineCyCoeff/(AOACritH − AOALineH)`;
  likewise `AOALineL` and `parabL = 0.5·lineCyCoeff/(AOALineL − AOACritL)`.
- `AOAMinCx = AOAMinCx_Shift − Cy0/lineCyCoeff`.

**Lift coefficient along the wind, `Cya(α)`:**
- linear `Cy0 + lineCyCoeff·α` between `AOALineL` and `AOALineH`;
- above that, up to `AOACritH`: `CyCritH − parabH·(AOACritH − α)²`;
- post-stall, up to `maxDistAng`:
  - `CyCritH − Decline·(α − AOACritH)²` while `α − AOACritH < parabAngle`,
  - then a parabola that meets `0.9·sin(0.0392699·maxDistAng)` at `maxDistAng`;
- `0.9·sin(0.0392699·α)` up to 40°;
- 40°–140°: `±0.9·sin(π/2 + 0.0314159·(α' − 40))`, where α' is mirrored about 90° (`α' = 180 − α` for α > 90) and the sign flips beyond 90°;
- beyond 140°: `−0.9·sin(0.0392699·(180 − α))`.

Negative α is the mirror image with the `L` parameters: a parabola `CyCritL + parabL(α − AOACritL)²` down to
`AOACritL`; then `min(CyCritL + 0.007(AOACritL − α)², −0.9 sin(0.0392699·|α|))` down to −40°; and the same
large-angle shape with signs swapped.

**Drag coefficient along the wind, `Cxa(α)`:** `CxMin + parabCxCoeff(α − AOAMinCx)²`.
Outside ±6° it is capped at `0.2 + 1.2|sin α|`, and beyond the critical angles `0.03` per degree past
critical is added before the cap.

**Body-axis coefficients:** `Cx = Cxa cos α − Cya sin α` and `Cy = Cya cos α + Cxa sin α`.

**Wave drag** (wing only; Mach is capped at 0.8 for this use): from the 8-entry `mc3/mc4/mm/mz` tables,
indexed by lift coefficient `(Cya + 0.9)·3.333`. *OpenIL2 status:* specified separately, not used yet.

Fixed polars (no flap dependence):

| | lineCyCoeff | CyCritH/L | AOACritH/L | CxMin | parabCxCoeff | Decline | maxDistAng | parabAngle |
|---|---|---|---|---|---|---|---|---|
| fuselage | 0.02 | ±0.2 | ±17 | 0 | 6e-4 | 0.007 | 30 | 5 |
| tail | 0.085 | ±1.1 | ±17 | 0.02 | 6e-4 | 0.007 | 30 | 5 |

## Per-step aerodynamics
`q(v) = ½ρ|v|²`. `Kq = 1/√(1 − M²)` with Mach capped at 0.8. `addVflow`/`addVside` are the propeller slipstream
terms (see Engine).

**Wing halves.** The left half uses the point `P = (−GCenter, 0.85·Wing_Out, −0.5)`; the right half mirrors y.
- Local flow `Vn = W × P + Vflow`, then `Vn.x += 0.07·addVflow`.
- Aileron incidence `δ = WING_V·sin(AOS) + SensRoll·ailerInfluence·(1 − 0.1f)·14·aileron`. It is `+δ` on the left half and `−δ` on the right.
- Half α = `±δ − atan2(Vn.z ± 0.07·addVside·propSign, Vn.x)`.
- Aileron drag adds `0.015·(±δ)` to Cx, multiplied by 0.18 when negative.
- Force coefficients per unit area: `Cw.x = −q(Vn)(Cx_M + aileron drag + gear and radiator terms)` and `Cw.z = q(Vn)·Cy·Kq`.
  The fuselage adds `Cw.y = −q·Cy_fus(AOS)` and subtracts `q·Cx_fus(AOS)` from `Cw.x`.

**Stall and spin** (realistic setting).
- If either half's α exceeds `AOACritH`:
  - `spinCoeff` grows by 0.2/s (by 0.1/s when opposite rudder is held against the yaw rate),
  - turbulence = `clamp(0.8·(α_max − AOACritH), 1, 15)`,
  - the stalled side gets `Cx += 0.25·s·SpinCxLoss` and `Cy −= 0.1·s·SpinCyLoss`, with `s = q·turbulence·spinCoeff`
    (scaled by `0.1·height` below 10 m above ground),
  - the other side gets small opposite terms,
  - rudder effectiveness becomes `1 + 0.035·turbulence`.
- Otherwise `spinCoeff` decays by 0.2/s.
- `spinCoeff` is clamped to `[0.5 + 0.4·power, 1.2 + 0.4·power]`.

**Ground effect.** Below `0.4·Length` above ground, wing lift and drag are multiplied by `1 + 0.2·(1 − h/(0.4L))`.

**Horizontal tail.**
- Local flow `Vn = W × (−Stabilizer arm, 0, 0) + Vflow`, plus slipstream for propeller aircraft: `Vn.x += s·r·addVflow`,
  with `r = 1/(1 + 0.04·Rudder arm)²` and `s = max(0, 1 − 1.5·|Vn.yz|/max(0.2, Vn.x + r·addVflow))`.
- Elevator input `e` gives a deflection of `28e` when positive and `20e` when negative.
- Tail α = `−atan2(Vn.z ∓ 0.36·side, Vn.x) − 2 − 0.002·V − SensPitch·deflection`, where `side` is the slipstream side term
  for single-engine propeller aircraft.
- Force per half = `(Stabilizer + Elevator)/2 · q(Vn) · (−Cx_tail, 0, Cy_tail)`.

**Fin.** Rudder deflection `28·rudder` (0 if the rudder area is under 0.05).
- Fin α = `−atan2(Vn.y − 0.5·side, Vn.x) + SensYaw·rudderInfluence·deflection`.
- Force = `(0.2 + 1.5·Keel + Rudder) · q(Vn) · (−Cx_tail, Cy_tail, 0)`.

**Forces.**
- Wing half force = (sum of that half's section areas) · `Cw`.
- Each half also gets `−½·q(Vflow)·(parasite + produced drag)` in x.
- The aerodynamic force is the sum of the two wing halves only. Tail and fin produce moments only.

**Moments.**
- CG shift `x_cg = GCenter + GC_Flaps_Shift·min(3f, 1)·(1 − u) + GC_AOA_Shift·u`, where `u = 0` inside the critical
  α range and grows by 0.0111 per degree outside it (capped by `0.3·excess`).
- Left wing arm `(−x_cg, Wing_Mid·(1.3 + sin AOS), −GCenterZ)`, right wing `(−x_cg, −Wing_Mid·(1.3 − sin AOS), −GCenterZ)`.
- Tail halves `(−Stabilizer, ±1, 0)`, fin `(−Keel, 0, 1)`.
- Moment = Σ arm × force.
- Bank leveling (|α| < 12°): `Mx −= 0.01·g·m·clamp(bank, ±30°)·min(Vflow.x − 50, 50)·0.003`.

**Damping and limits.**
- Per step, `W *= max(0.8, 1 − 1e-5·IAS)`.
- `|W|` is capped at 5 rad/s.
- `|AF| ≤ 50 g·m` and `|AM| ≤ 150 g·m` (exceeding these is structural failure; OpenIL2 clamps for now).

## Integration
Body forces `F = AF + engine + gear + gravity` and moments `M = AM + engine + gear`, using `n = 5` sub-steps when
any wheel touches the ground, otherwise 1. Each sub-step:
1. `Ẇ = ((Jy−Jz)·Wy·Wz + Mx)/Jx`, and cyclically for y and z; then `W += Ẇ·dt/n`.
2. Rotate the orientation by `W·dt/n`.
3. Velocity `+=` `R·F/m · dt/n`, then position `+=` velocity · `dt/n`.
4. Gear forces are recomputed between sub-steps.

## Engine and propeller
See `engine.md`. Thrust acts along body +x. The torque reaction acts about body x. Slipstream terms feed the
wing and tail flow as described above.

## Landing gear — *OpenIL2 simplification*
Contact points are the hooks `_ClipLGear`, `_ClipRGear`, `_ClipCGear` of the aircraft's hierarchy, taken with
the gear animation at "down" (`animation.md`), because the meshes are modelled with the gear retracted. Only
the hooks' x and y are used. Each wheel's z is solved
so the aircraft rests at `[Gear] H` (origin height) and `Pitch` (nose-up angle):
`z = −(H + x·sin Pitch)/cos Pitch − 0.08`, where 0.08 m is the springs' static compression.
Each point is a spring-damper:
- stiffness `k = m·g/(n·0.08 m)` and damping `c = 2·0.6·√(k·m/n)`,
- friction: rolling 0.03 along the wheel, 0.8 sideways (0.1 for the tail or nose wheel unless locked),
- brakes add up to 0.6 along the wheel.
- Airframe points (hooks `_Clip<digits>`: nose, wingtips, tail) also collide with the ground: 10× stiffer,
  3× damping, friction 0.6 in all directions.
- Differential braking: the left wheel gets `brakes·min(1, 1 − rudder)`
  and the right wheel `brakes·min(1, 1 + rudder)`; the castoring wheel is unbraked.

The original `Gear` model (sink factor, roughness, water, arrestor hooks) is to be specified.
