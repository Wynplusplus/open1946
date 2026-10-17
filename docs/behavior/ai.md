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

## Landing
OpenIL2's own landing pilot, until the runway logic of the runtime's `LANDING` maneuver and
`Airport.makeLandWay` is ported. At the `LANDING` waypoint the pilot uses the takeoff spot of the
nearest runway, touching down 300 m beyond it in the takeoff direction.

- **Gate**: he flies to a point 5 km before touchdown, 400 m above it, at 1.6 `v_ref`. He takes
  the final within 1.5 km of the gate, or when 2–6 km out, within a quarter of the distance of the
  centre line and heading within 60° of the runway.
- **Final** (OpenIL2): gear and flaps down. `v_ref` is the faster of 1.25·VminFLAPS and the speed
  for a lift coefficient of 1.1 at sea level. The localizer intercept heading is
  `runway − 0.004·offset` (at most 30°), bank `−2·heading error` (at most 20°). The glide path is
  4°; the vertical speed wanted is `−v·tan 4° + 0.15·(path height − height)`, limited to −6…2 m/s.
  A cascade flies it: the vertical speed error `e` sets the pitch attitude `p_i + 1.2·e` (`p_i`
  integrating `0.4·e` °/s, limited to −8° … touchdown attitude), and the elevator holds it with
  `j + 0.05·(pitch error) + 0.6·pitch rate` (`j` integrating `0.04·pitch error`). The throttle holds
  `v_ref` alone (`k + 0.08·(v_ref − v)`, `k` integrating `0.03·(v_ref − v)`).
- **Flare** (OpenIL2) from 12 m wheel height: the wanted sink is `0.3 + 0.3·(wheel height − 0.5)`
  m/s, `p_i` learns at 1.5 °/s per m/s, the attitude is capped at the touchdown attitude (the
  three-point angle between main and tail wheels for tail-draggers, 6° for nose-wheel types).
- **Touchdown zone** (**Verified**, the runtime's `LANDING` maneuver below 60 m): the attitude is kept
  between −5° and `[Gear] Pitch` + 10°. Below 10 m, while faster than 0.6·VminFLAPS, the runtime acts
  on the velocity itself each 0.03 s tick: a sink beyond 0.6 m/s is multiplied by 0.94 and limited to
  2.5 m/s, a climb is multiplied by 0.91 (this is what keeps AI touchdowns from bouncing). The
  throttle is closed; below 7 m the elevator grows by 1.5/s while the nose is below
  max(Gear Pitch − 2°, 5°), plus `0.05·pitch rate` per tick.
- **On the ground** (runtime): with `[Gear] Pitch` ≤ 5° (nose wheel) the elevator is centred; else it
  grows by 1.5/s while the nose is below the gear pitch, plus `0.3·pitch rate`. Brakes (OpenIL2: 0.3,
  full below 0.75 `v_ref`), rudder on the runway heading; stopped is the end of the flight. After a
  touchdown a bounce does not restart the approach; a float beyond 1.2 km past the touchdown point
  does.
- The runtime flies the final itself above 4.5 m + gear height: it moves the aircraft along the
  landing path kinematically (position, velocity and attitude set directly, the flight model
  skipped) at 1.2·VminFLAPS. OpenIL2 flies it with the controls instead.
- `openil2 --aircraft CLASS --ai-test` ends with a landing from the 5 km gate onto flat ground: all
  55 aircraft of a sample across `air.ini` touch down within 300 m of the aim point at ≤ 3.5 m/s and
  stop within 600 m of it.

## Sides
A wing's name is its regiment followed by two digits. The regiment's army comes from
`PaintSchemes/regiments.ini`: regiments listed under `[Red]` are army 1, under `[Blue]` army 2.

## Air combat
OpenIL2's own interim pilot, until the runtime's `Maneuver` attack and defence maneuvers are
ported:

- Every 0.5 s a pilot looks around. An enemy within 600 m behind (more than 120° off the nose)
  is evaded with a 75° bank hard turn towards its side. Fighters (classes implementing
  `TypeFighter`) attack the nearest enemy in sight: 2.5, 3.5, 4.5 or 5.5 km for rookie to ace.
  They keep a target until it is 1.5 sight ranges away. Other types keep to their route.
- **Attack**: lead pursuit. The aim point is the target's position plus the relative velocity
  times the bullets' time of flight, raised by the drop. The pilot rolls it into the lift plane
  (`aileron = 0.02·roll error − 0.3·roll rate`) and pulls in proportion to its elevation above the
  nose (`elevator = 0.08·elevation° + 0.15`), at full throttle. Below 300 m and descending he
  levels out and climbs. Guns fire with the aim point within 6°/4°/3°/2.5° and range under
  350/450/550/650 m (rookie to ace).

## Bullet hits on aircraft
OpenIL2 tests bullets against a box per visible chunk (from the chunk's geometry), posed with the
aircraft. The runtime uses the meshes' collision shapes and, for most types, detailed internal
"bones" (`xx…`) handled per class. The hit applies the runtime's generic chunk damage
(`docs/behavior/damage.md`):

- Energy as TNT: `½mv²·2.4·10⁻⁷` kg, plus half the charge (`power + 0.03·mass`) for high explosive.
- Hits on an outer wing strike the aileron 20 % of the time, a stabilizer its elevator 45 %, a
  fin its rudder 33 %.
- Damage levels: on an intact `_D0` chunk (not gear) `1 + (TNT − 0.01)/toughness` above 0.01 kg,
  else `TNT/0.01`; on damaged chunks and gear `TNT/toughness`. Fractions carry over per part.
  `[Toughness]` in the `.fmd` gives per-part values (default 100, × 10⁻⁴).

## Formation
OpenIL2's own formation keeping, until the runtime's `FOLLOW`/`STAY_FORMATION` maneuvers and
`AirGroup` formations are ported. A wingman flies station on the lowest-numbered aircraft of
its flight that is still on its route, or on the player when the player leads the flight. The
station is echelon right, 40 m back and 40 m out per place in the flight. The pilot steers with
the waypoint autopilot's laws at a point 300 m ahead of his station, at the station's height.
The throttle is `i + 0.05·e` with `e = 0.02·(distance ahead to the station) + (leader speed −
speed)`, and the integral `i` grows by `0.02·e·dt`.

## Gunners
Guns in trigger groups 10 and up are turrets (group `10 + n` is turret `n`). Each aircraft class
says where its turrets can aim in `turretAngles(turret, float[] {yaw, pitch, roll})`, which
clamps the angles and returns whether they were within reach. OpenIL2 evaluates this method from
the installed classes on a 10° grid at load (yaw −180…180°, pitch −90…90°, in the turret's
frame). It takes the gun hook's frame as the turret's.

OpenIL2's gunner (own design): each step every turret gun picks the nearest enemy aircraft within
500/600/700/800 m (rookie to ace) whose lead point lies inside its arc. The lead point is
`target + relative velocity × time of flight`, raised by the drop. The gun fires at its own rate
with a gaussian scatter of 2/1.4/1/0.7°. The player's aircraft has AI gunners of veteran skill.

## Ground attack
OpenIL2's own level-bombing pilot, until the runtime's `GATTACK` maneuvers (dive bombing,
strafing, rockets, torpedoes) are ported. At a `GATTACK` waypoint the target is the mission
object the waypoint names (a column's lead unit still alive, or a stationary object), else the
waypoint itself, on the ground. The bombs' fall time is `1.05·√(2h/g)`. The predicted impact is
the current position plus the horizontal velocity times the fall time times 0.965 (drag). Bombs
(trigger group 3) release as the predicted impact passes the target, within 200 m to the side.
Once they're gone, the pilot moves on to the next waypoint within 1.5 km of the target.

The waypoint autopilot also has a slow integral of heading errors below 10° (`0.5·error·dt`,
at most 6°) so tracks have no standing offset (OpenIL2).
