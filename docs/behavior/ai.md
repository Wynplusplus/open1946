# AI aircraft

## AI flight model
AI aircraft fly a simplified flight model (the runtime's `AIFlightModel`; the player's aircraft
uses the full one). Engines, gear and damage are shared with the full model. The aerodynamics
differ:

- One force for the whole wing: `Cx = q·(Cx(α) + 2·gear Cx + 2·air brake Cx + radiator Cx)`,
  `Cy = q·Cy(α)·Kq`, side force `−q·0.7·sin β`. Ground effect below `0.4·length` scales lift and drag
  by `1 + 0.3·(1 − h/0.4L)`. The force is the coefficient times the sum of all six wing sections.
  Damage drag `ΔCx·q` and `−β·q·0.1` sideways are added. Above 50 g the force is limited and the
  fuselage fails.
- Roll moment from the difference of the left and right sections times their arms
  (`Wing_In`, `Wing_Mid`, `Wing_Out`). OpenIL2 lacks the root arm and uses half the middle one.
  Pitch and yaw moments come from the tail only beyond 33° of α or β.
- **The controls command rotation rates directly** (added to the dynamic rates):
  - roll `= aileron·min(Vx, 150)·min(SensRoll, 4)·S_aileron / max(S, 1)`;
  - pitch `= (α − elevator·max(20 − |α|, 4))·max(0.017|α|, 1)·min(Vx,150)·SensPitch·S_elevator/max(S,1)·0.017`;
  - yaw `= (β − rudder·max(12 − |β|, 0))·max(0.01|β|, 1)·min(Vx,150)·min(SensYaw, 0.2)·S_rudder/max(S,1)·0.15`.
- Weathervaning: `M_z += β·q·0.5` with a keel (else `M_x += β·q`). Below 20 m/s
  the lift also pitches (`M_y += k·F_z`, `k` falling to 0 within 1.5 lengths of the ground).
- With the tail gone, or one wing middle section gone, extra moments make the aircraft spin
  (as in the full model).
- Damping per step: roll rate × `1 − 0.12(0.2 + ΣS_wing)/max(0.1 + S, 1)`, pitch rate ×
  `1 − 0.5(0.2 + S_stab)/…`, yaw rate × `1 − 0.5(0.2 + S_keel)/…`; at most 6 rad/s.

## Waypoint autopilot
(The runtime's `AutopilotAI` in waypoint mode, run every step.)

- A waypoint is **reached** within 1 km horizontally once the distance starts to grow again.
- Target speed: the waypoint's speed, less 2 m/s per position in the flight. **Throttle**
  `= clamp(0.3 − 0.04·(v − v_target), 0, 1)`.
- **Elevator**, with `Δh = target altitude − altitude`. The target is at least 5 m over the ground.
  - Climb term (when `Δh > −50`): `min(α − α_max, pitch − 1)·dt + 0.5·a_x`, with
    `α_max = min(5 + 0.00025·h + 0.02·(250 − Vmax), 14)` and `a_x` the forward aerodynamic and
    thrust acceleration in g.
  - Descent term (when `Δh < 50`): `(pitch − p_min)·0.8·dt`, with `p_min = min(−15 + 0.00033·mass, −4)`.
  - Blend with `k = clamp(0.01·(Δh + 50), 0, 1)`: `e −= k·climb + (1 − k)·descent`, then
    `e += ω_y + 0.5·ω̇_y`. Below `1.3·VminFLAPS`, `e −= 0.004·dt`.
  - The command is limited to `(p_max − pitch)·0.1` and `(p_min − pitch)·0.2`, with
    `p_max = min(9·v/VminFLAPS, 25)` (10 for very slow types). The surface follows `0.8·old + 0.2·e`.
  - Banking adds `|bank|·0.004·dt`.
- **Ailerons**: heading error `azimut − bearing` (degrees, wrapped), limited to the bank limit
  `clamp(((v − VminFLAPS)·3.6 + climb rate·40)·0.25, 15, 65)`;
  `aileron = clamp(−0.01·(error + bank + 3ω_x + 0.5ω̇_x), −1, 1)`. Within 25 m sideways and 150 m
  ahead of the waypoint it only levels the wings (`−0.01·bank`).
- **Rudder** `−= β·0.04·dt`.

`VminFLAPS` is scaled by 0.2417 (km/h to m/s with a 0.87 factor), as the runtime loads it.

OpenIL2 additions (own design):
- The altitude target is at least 150 m over the highest ground of the next 3 km of the track.
  The runtime's waypoint flight trusts the mission's altitudes.
- More than 100 m below the target, the throttle is at least 0.9.

## Takeoff
OpenIL2's own takeoff pilot, until the runtime's `TAKEOFF` maneuver is ported. Aircraft of a
flight whose first waypoint is `TAKEOFF` line up at the airfield's takeoff spot, each 40 m behind
the one before and staggered sideways. They start rolling 15 s apart: full power, flaps 0.2,
rudder holding the runway heading, tail up from 0.6·`v_r`, lift-off at `v_r = 1.2·VminFLAPS`
(elevator `0.5 + 0.08·(8° − pitch)`). They climb straight out to 200 m, then follow the waypoints
after the takeoff ones. Flights starting in the air begin at their first waypoint in echelon
(40 m back, 30 m to the side per aircraft).

## Sides
A wing's name is its regiment followed by two digits. The regiment's army comes from
`PaintSchemes/regiments.ini`: regiments listed under `[Red]` are army 1, under `[Blue]` army 2.
