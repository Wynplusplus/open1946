# Piston engine and propeller

Status: **specified from the runtime's engine model** for running piston engines (`Type Inline/Radial`).
Also specified: jets, rockets, rocket boosters, pulse jets, tow strings and fuel use (sections below).
Emergency boost and engine management (start-up, temperatures, radiator, overheat) are specified below.
Not yet covered: mixture, magnetos, negative-g carburettor cut, engine roughness, combat damage.

## Data (`FlightModels/<family>.emd`)
A submodel `[Name]` is read **on top of** `[Generic]` in the same file: any key the submodel does not give
keeps its `[Generic]` value, then the built-in default below.

| key | meaning (default) |
|---|---|
| `Type` | ends in `Inline`/`Radial` → piston engine |
| `HorsePowers` | rated power (1200) |
| `RPMMin`, `RPMNom`, `RPMMax`, `RPMMaxAllowed` | engine RPM limits (200, 2000, 2200; max allowed ≈ 2387) |
| `Reductor` | propeller/engine speed ratio (1) |
| `Carburetor` | torque curve family 0–4 (0) |
| `BoostFactor` | (1) |
| `EngineAcceleration` | multiplier on positive engine acceleration (1) |
| `EngineI` | engine rotational inertia (1) |
| `PropDiameter`, `PropMass` | metres (3), kg (30) |
| `PropAnglerType` | governor type (0) |
| `PropAnglerSpeed` | pitch change speed, rad/s (0.1) |
| `PropAnglerMinParam`, `PropAnglerMaxParam` | governor range: RPM for types 1, 2, 7, 8, 9; degrees for 5, 6 |
| `PropPhiMin`, `PropPhiMax` | blade pitch limits, degrees (10, 29) |
| `PropAoA0` | degrees (11) |
| `Voptimal` | km/h (324) |
| `CompressorType` | 0 ram air, 1 mechanical stepped, 2 variant, 3 (0) |
| `CompressorSteps` | number of supercharger stages (1) |
| `CompressorAltitude<i>`, `CompressorMultiplier<i>`, `CompressorBaseMultiplier<i>` | per stage (BaseMultiplier default 0.8) |
| `CompressorPMax`, `CompressorRPMP0`, `CompressorMaxATARPM`, `CompressorRPMPMax` | ATA curve anchors |
| `CompressorRPM<k>`/`CompressorATA<k>` | further ATA curve points (k from 0, up to 16 in total) |
| `CompressorSpeedManifold` | ram effect on manifold pressure (0.2) |
| `CompressorPAt0` | (0.3) |

Derived values:
- `ω` values are in rad/s (`ω = RPM·2π/60`).
- `propR = 0.375·PropDiameter` (blade reference radius).
- `propI = 0.083·PropMass·PropDiameter²`.
- `propS = 0.26·propR²` (equivalent blade area).
- `Mmax = 1.2·746·HorsePowers/ωMax` (peak engine torque).
- `pressure(stage i) = p(CompressorAltitude_i)/p0`.

**ATA curve.** The ATA curve is a list of (RPM, ATA) points kept sorted by RPM. It contains
`(CompressorRPMP0, 1)`, `(RPMMax, CompressorMaxATARPM)`, `(CompressorRPMPMax, CompressorPMax)` and the
`CompressorRPM<k>/ATA<k>` pairs. A point whose RPM is already present is not added again.

`ATA(rpm)`:
- 1 if the list has fewer than 2 points;
- ambient pressure ratio for `rpm < 0.1`;
- the last ATA beyond the last point;
- otherwise linear interpolation, using the first segment below the first point.

## Per step (engine running)
Inputs: throttle `T ∈ [0, 1]`, or `[0, 1.1]` when `BoostFactor > 1` (above 100 % is boost); the propeller lever is
automatic. `t = min(T, 1)`. `V` is the airspeed along body x (`Vflow.x`)
and `h` the altitude.

1. **Ambient manifold pressure** in atm: `pe = (p(h) + CompressorSpeedManifold·½ρV²)/101300`.
2. **Propeller forces** with the *current* blade pitch `φ`:
   - `ωp = ω·Reductor` and `φw = atan(V/(ωp·propR))` (π/2 when ω ≈ 0); propeller AoA `α = φ − φw`.
   - `u = ωp·propR`, `W² = V² + u²`.
   - `L = ½·Cy(α°)·ρ·W²·propS` and `D = ½·Cx(α°)·ρ·W²·propS`.
   - If `|W| > 300`, `D *= min(2, 1 + 0.02(|W| − 300))` (tip losses).
   - `k = 1` if `V ≥ Vopt`, else `1 − 5e-5·(Vopt − V)²`.
   - **Thrust** `T = k·(L·u − D·V)/|W|`; **propeller torque** `Q = (D·u + L·V)/|W|·propR`.
   - Blade coefficients (α in degrees, clamped to [−8, 34] for Cy):
     - `Cy = −0.004688α² + 0.15α + 0.4` below 16°, otherwise `9.7222e-4α² − 0.070833α + 2.4844`
       (with α capped at 22 and `+0.01·(α − 22)` added above that).
     - `Cx` mirrors α below −4 (`α ← −8 − α`), caps at 34, `3.5e-4α² + 0.0028α + 0.0256` below 16°, otherwise
       `−0.00555α² + 0.24444α − 2.32888` (α capped at 22, `+0.04·(α − 22)` above).
3. **Governor** (sets a pitch target `φt`):
   - **0 (fixed pitch):** `φt = 0`, i.e. clamped to `PropPhiMin`.
   - **1, 2 (constant speed); also 7, 8 for now:**
     - The lever is `0.75 + 0.25·T`, or `0.75 + 0.227272·T` when `BoostFactor > 1` (so 110 % gives a lever of 1).
     - Target engine speed `ωt = ωmin_p + (ωmax_p − ωmin_p)·lever`, using the governor parameters as rad/s.
     - If `ω < ωt`, then `φt −= min(1, 0.01(ωt − ω) − 0.012·ω̇)·speed·dt`; otherwise
       `φt += min(1, 0.01(ω − ωt) + 0.012·ω̇)·speed·dt`.
     - Also `φt ≥ φw − 0.12`; when that limit applies and `φ < φt`, then `φ += 0.2·dt`.
     - The manifold threshold is `ATA(RPM(ωmin_p + (ωmax_p − ωmin_p)·t))`.
   - **Other types:** threshold `MaxATARPM·(0.55 + 0.45t)`.
   - Then `φ` moves towards `φt` at `min(1, 157.3·|φ − φt|)·speed`, and both are clamped to `[PropPhiMin, PropPhiMax]`.
4. **Supercharger multiplier `c`:**
   - **Type 0:** `c = (p + ½ρV²)/p0`.
   - **Type 2:** with `r(pc) = pe·ω/ωMax·MaxATARPM/pc + 0.001`:
     - **Two stages,** when `r(pressure_0) < thr`: stage 1 is used. If `r(pressure_1) > thr`, then
       `k = f + (1 − pe)/max(1e-4, 1 − pressure_1)·(Mult_1 − f)` (with `f` = `BaseMultiplier_1`, times `Mult_1`
       if that is below 1), then `k += (Mult_0 − k)·(r(pressure_1) − 1)/(pressure_0·thr/pressure_1 − 1)`.
       Otherwise `k = Mult_1`.
     - **Otherwise** stage 0 is used, with `k = 1 + (1 − pe)/max(1e-4, 1 − pressure_0)·(Mult_0 − 1)`. This keeps
       falling linearly past the critical altitude.
     - Then `m` and `c` as in type 1, except the second throttle is applied without lag:
       `s = min(1, MaxATARPM/m)`.
   - **Type 1 (and others):** for each stage `i`:
     - if `pe ≤ pressure_i`: `k_i = Multiplier_i` and `score_i = k_i·pe·thr·(PAt0 + (1 − PAt0)·ω/ωMax)/(pressure_i·thr)`;
     - otherwise `base = (Multiplier_i ≥ 1 ? BaseMultiplier_i : BaseMultiplier_i·Multiplier_i)^i` (the factor
       for stage `i` is applied `i` times), and `k_i = score_i = base + (1 − pe)/max(1e-4, 1 − pressure_i)·(Multiplier_i − base)`.
     - The stage with the highest score is used. With it:
       - `m = (PAt0 + (1 − PAt0)·ω/ωMax)·pe·MaxATARPM/pressure_i`;
       - the second throttle `s` tracks `MaxATARPM/m` with `s −= 3·dt·(s − target)`, capped at 1; `m *= s`;
       - `m *= min(1, thr/MaxATARPM)` and `c = k_i·m/thr`, where `thr` is the governor's manifold threshold.
     - Below `ω = 20` the manifold pressure is limited further (start-up; not used while running).
5. **Engine torque** (`f = 0.05 + 0.95t` for carburettor 0, `f = min(1, 0.1 + 0.9t)` otherwise):
   - **Carburettor 0:** `Mmax·(−x²/f + 2x)`, with `x = ω/ωMax`.
   - **1, 2, 4:** `Mmax·(−0.5f² + f + 0.5)·(2x − x²)`, with `x = ω/(ωNom·f)`.
   - **3:** `Mmax·(−x²/f + 2x)`, with `x = ω/ωNom`.
   - **Boost** (`T > 1`): multiply by `BoostFactor` for carburettors 0 and 3, otherwise by
     `1 + (T − 1)·10·(BoostFactor − 1)`.
   - A negative result is limited to `−0.8·ω/ωMax·Mmax`.
   - The engine moment is `M = torque·c`.
6. **Rotation:**
   - `ω̇ = (M − Q)/(propI + EngineI)`, multiplied by `EngineAcceleration` when positive;
   - `ω += ω̇·dt`, clamped to `[0, 2·ωMaxAllowed]`.
7. **Forces on the airframe:**
   - thrust `T` along body +x;
   - torque reaction `Q + propI·ω̇·Reductor` about body x, positive for `Direction Left` and negative for `Right`;
   - the moment of the thrust about the CG (`propPos × F`; the propeller hub position is from the aircraft).
8. **Slipstream:** `addVflow ← 0.95·addVflow + 0.05·T/(6ρ·max(1, V + addVflow))`, and
   `addVside ← max(0, 0.95·addVside + 0.05·(Q/propR)/(6ρ·max(1, V + addVflow)))`.

**Initial state.** A running engine starts at `ω = ωNom` in the air and `ω = ωMin` on the ground. The pitch
starts at 11°, clamped to the limits.

## Engine types
`Type` is matched by suffix: `Inline` and `Radial` are piston engines, then `Jet`, `RocketBoost`, `Rocket`
(checked after `RocketBoost`), `Tow`, `PVRD`, and `HeloI` (a piston engine whose propeller sees `|V|`, for
tail-sitters). `Thrust` is in kgf (`× 9.81`; default 10.7 N), `Cylinders` defaults to 12, `PropAoA0` to 11°
(stored in radians).

**Jet.**
- Setup: fan area `S = Cylinders²·2·Thrust / (FanCy(PropAoA0)·ρ0·ωMax²·propR²)`. The runtime passes
  `PropAoA0` **in radians** to `FanCy`, which expects degrees; OpenIL2 keeps that. Drag torque at full speed
  `Mmax = ½·FanCx(PropAoA0 in degrees)·ρ0·(ωMax·propR)²·S·propR`.
- Per step:
  - engine torque `M = (PropAnglerMinParam + T·(MaxParam − MinParam))/MaxParam · Mmax` (zero when stopped,
    plus a friction of `−0.002·ω·(propI + EngineI)/dt` then);
  - fan torque `Q = Mmax·(ω/ωMax)²`, so the steady speed is `ωMax·√(M/Mmax)`;
  - `ω̇ = (M − Q)/(propI + EngineI)`, times `EngineAcceleration` when positive; `ω` stays within
    `[−ωMaxAllowed, 2·ωMaxAllowed]`.
- Thrust `Thrust·(ω/ωMax)³·pe·(1 − 0.006(T_air − 290 K))·(1 − 0.0011·V)`, with
  `pe = (p(h) + CompressorSpeedManifold·½ρV²)/101325` (`CompressorSpeedManifold` default 0.2). No torque
  reaction, no slipstream.

**Rocket and RocketBoost.** `ω = ωMin + (ωMax − ωMin)·T`, zero if below `ωMin` or 0, out of fuel or
stopped (a negative `RPMMin` gives a dead band at low throttle). Thrust
`ω/ωMax · Thrust · CompressorPMax/(CompressorPMax + 10⁻⁵·p(h))` (`CompressorPMax` default 1), so it grows with
altitude. *OpenIL2 control mapping:* a `RocketBoost` engine fires while the throttle is above 100 %.

**PVRD** (pulse jet). `ω` as for rockets. Thrust `ω/ωMax · Thrust · √(V/94)`, zero below 94 m/s; above
208.333 m/s it is multiplied by `√((291.666 − V)/83.333)`, and it is zero above 291.666 m/s.

**Tow.** The thrust is set by the towing aircraft; in free flight it is zero (gliders).

## Fuel use
Power fraction `x`: piston engines `x = P/HorsePowers` with `P = M_engine·ω·0.00105` hp (engine torque times
compressor factor, before readiness), floored at 5 % of `HorsePowers` for the amount; other types
`x = T` and the amount `Thrust·x` (newtons), floored at 5 %.

Specific use from `FuelConsumptionP0`, `P05`, `P1`, `PMAX` (defaults 0.4, 0.24, 0.28, 0.3; jets 0.075,
0.075, 0.1, 0.11; pulse jets 0.835 for all):
- for `x ≤ 0.5`: `(x − ½)²·4(P0 − P05) + P05`;
- for `x ≤ 1`: `(x − ½)²·4(P1 − P05) + P05`;
- beyond 1: `P1 + (PMAX − P1)·10·min(x − 1, 0.1)`.

It is per hour, so divide by 3600 and multiply by the amount; piston engines also by
`0.8 + 0.2·ω/ωWEP` (`ωWEP` from `PropAnglerAfterburnerParam`, else `ωMax`).
- Rockets burn `2.5777·T` kg/s (1.8 for BI-1 and BI-6, 0.5 for MXY-7).
- Rocket boosters burn `1.4320556·T` kg/s, and pulse jets their computed rate, from the `[Mass] Nitro`
  tank instead of the fuel tank.
- An engine stops when its tank is empty. Aircraft mass is `empty + fuel + nitro`.

## Emergency boost (`AfterburnerType`)
`AfterburnerType` (0 none) selects the boost system; `WEPBoostFactor` (1) multiplies engine torque while it
is engaged.
- **Boost pressure.** For types 1 and 4–12, the **last point of the ATA curve** (highest RPM) is the boost
  pressure: it is removed from the normal curve, and `wepCompressor = ATA_last / ATA_normal(RPM_last)`
  (1 if fewer than two points remain).
- **While engaged:**
  - the supercharger's allowed manifold pressure and the manifold threshold are multiplied by
    `wepCompressor` (types 7 and 8: only below the top supercharger stage);
  - engine torque is multiplied by `WEPBoostFactor` (types 7 and 8: only below the top stage; type 1: only
    with throttle above 100 % and nitro left);
  - governors 1 and 2 aim at `PropAnglerAfterburnerParam` (default `PropAnglerMaxParam`) instead of the maximum.
- **Engaging.** Types 4 (with nitro), 8 and 10 engage while the throttle is above 100 % and disengage
  below it. The other types use a separate WEP switch; type 1 (MW 50, GM-1) also needs throttle above 100 %
  and nitro.
- **Nitro.** Types 1 (above 100 %), 2, 4, 5 and 9 burn 0.044872 kg/s of `Nitro` while engaged; type 4
  disengages when it runs out.

*OpenIL2 status:* the supercharger stage conditions for types 7 and 8 are not modelled (the factors always
apply).

## Engine management
**Stages.** 0 off, 1–5 starting, 6 running, 7 dead, 8 seized. During stages 1–5 the engine gives no force
and the shaft speed is imposed (RPM): stage 1 `20·f` (0.5–1 s), stage 2 `20 + 15f` (4–8 s cranking), stage
3 `60 + 60f` (0.05–0.1 s), stage 4 120 (0.5–1 s), stage 5 `120 + 120f` (0.5–1 s), where `f` is the fraction
of the stage elapsed and the durations are uniformly random. Rockets and pulse jets jump to stage 5. Engines
with `Autonomous 0` (jets) can start only on the ground within 1200 m of an airfield. Restarting an engine
that has run before:
- stage 2 lasts `0.1 + c·7.9·U(2, 4.2)` s (beyond 9 s: `U(7.8, 9.6)`), with
  `c = (TOILOUTMAXRPM − T_oil)/(TOILOUTMAXRPM − 20)`;
- stage 2 fails with probability 0.5, and stage 3 with 0.12 when `c < 0.75`;
- stage 5 fails with 0.1 for piston engines.

**Stopped engine.** Engine torque is 0. A friction coefficient grows by 0.1/s up to 1, giving the torque
`−fric·(6 + 3.8ω/ωMax)·(propI + EngineI)/dt`, but no more than `−0.99ω(propI + EngineI)/dt`.
Running out of fuel stops the engine.

**Temperatures** (°C). Let:
- `T₀` = ambient temperature, `ρ` = air density;
- `x = ω/ωWEP`, `t` = throttle;
- `r` = radiator opening;
- `s = 0.8 + 60/(50 + V_kmh)`.

The throttle term `t` is reduced for boost engines:
- with `BoostFactor > 1`: `t = (t + max(t − 1, 0)·(BoostFactor − 1.1)·10)/BoostFactor`;
- when emergency boost is off and `WEPBoostFactor > 1` (not types 1, 4, 9): `t /= WEPBoostFactor`.

While running, each value approaches its target at rate `TESPEED` per second (oil in at 0.5/s):
- oil out: `T₀ + TOILOUTMAXRPM·(1 − 0.15r)·(0.65 + 0.45/ρ)·(1 + 0.35·oilDamage)·1.2·x²·(0.7 + 0.3t)·s`;
- oil in: towards `0.8·oil out`;
- water: `T₀ + TWATERMAXRPM·(1 − 0.15r)·(0.65 + k)·(0.6 + 0.4x)·(0.1 + 1.1t)·s·(1.1 − 0.1·mixture)`, with
  `k = 0.45/ρ`, or `0.35 + 0.1/ρ` for jets.

When stopped, oil and water approach `T₀` at `TESPEED·(0.2 + 0.2r)`. Engines spawned running start at
the mid-points of `TOILMIN…TOILOUTMAXRPM` and `TWATERMIN…TWATERMAXRPM`.

**Automatic radiator** (unless the pilot sets it). By aircraft class:
- Gladiator and Fokker D.XXI: closed.
- P-51, P-38, Yak-3 / 3P / 9M / 9U / 9UT, P-63C: opens at 0.1/s while oil out exceeds `TOILOUTMAXRPM`,
  else `1 − V/VmaxH`.
- Spitfire VIII and IX: `(water − TWATERMAX + 20)/20`.
- Otherwise by governor type:
  - 1 and 2: track `1 − power` at 0.15/s;
  - 5 and 6: `1 − V/VmaxH`;
  - 7: as the P-51 rule;
  - 8: the oil rule for inline engines (with a 10 °C band), else as 1/2;
  - the rest: `1 − power`.

Radiator drag: `0.003·r` added to the wing drag coefficient.

**Overheat** (difficulty option). For water above `TWATERMAX` and oil above `TOILMAX`: with
`e = T/T_crit − 1`, a malfunction happens in a step when `1000·e³·dt > U(0, 1)·MAXRPMTIME`. For piston
engines it is drawn from 0–99:
- water: < 50 a cylinder lost, < 65 oil hit, < 94 a cylinder with smoke, < 95 fire, < 97 supercharger
  lost, < 98 engine dies, else seizes;
- oil: < 10 cylinder, < 60 oil hit, < 94 bearing (readiness × 0.95), < 95 fire, < 96 supercharger, < 97
  dies, else seizes.

Other types: 20 % smoke, else 0–3 cylinders' worth of power. Losing cylinders lowers readiness to the
operable fraction; below 12 % the engine dies. Readiness multiplies engine torque.

*OpenIL2 status:*
- An oil hit raises `oilDamage` by 0.5 (the runtime's oil-leak state is part of the damage model).
- Fire and supercharger loss reduce readiness instead of the damage-model effects.

## Propeller governors 3–6 and the propeller lever
`PropAnglerType`:
- **1 and 2:** governed RPM `MinParam + (Max − Min)·lever`; the automatic lever is `0.75 + 0.25·throttle`
  (`0.227272` with boost).
- **3 and 4** (variable pitch): automatic `φ_target = φ_w + PropAoA0` (blades at their design angle of
  attack); manual `φ_target = PropPhiMax − lever·(PropPhiMax − PropPhiMin)`.
- **5** (pitch by airspeed): lever = `−cvt(V, Vmin, Vmax, 0, 1)` when automatic, else `−lever`;
  `φ = MaxParam − lever·(MaxParam − MinParam)` (angles), set at once.
- **6:** a governor like type 1 aiming at `25 + (ωMax − 25)·(0.25 + 0.75·throttle)` rad/s (manual: `lever`
  instead of the throttle term).
- For 3–6 the manifold threshold is `0.5 + (CompressorMaxATARPM − 0.5)·throttle`.

`Vmin` and `Vmax` are `[Params]` values in km/h.

## Mixture, magnetos, negative g, rough running
- **Magnetos** (`cMagneto`, default present): switch 0 off (stops the engine), 1 left, 2 right,
  3 both. Each working magneto gives 87 % of the torque, both together 100 %.
- **Mixture** (`MixerType`, `MixerAltitude`, `cMix`): lever 0…1.2, 1 normal (type 1 can't go
  below 1). With `v = p(MixerAltitude)/p₀ · mixture` and `pe` the outside pressure in
  atmospheres (with the ram term):
  - `v < pe` (lean): full power while `v > 0.25·pe`, else `v/(0.25·pe)`; below 0.03 the engine
    stops;
  - `v > pe` (rich): power `pe/v`, rough running `35·(1 − pe/v)`; type 3 at mixture ≤ 1 is not
    affected.
  Mixture only matters with complex engine management (the realism option, on for the player;
  `--easy-engine` turns it off). AI aircraft always have the right mixture, as in the runtime.
- **Negative g** with float carburetors (`Carburetor` 3 and 4): a counter rises by 0.03 per step
  under negative g and falls by 0.015 (type 4: 0.03) afterwards, while the carburetor floods.
  - Type 3: torque × `max(throttle − 0.1·counter, 0.3)` and rough running `10 + 5·counter`. The
    engine stops when the counter passes `U(5, 8)`.
  - Type 4 at full throttle: × `0.52 − counter` at first; past 0.5 the counter holds at 0.51;
    past 0.8 it falls back. Below full throttle it is like type 3 with 0.2 per count and
    `U(7.5, 9.5)`.
- **Rough running** (`DisP0x/y`, `DisP1x/y`: rpm and percent): the amplitude is
  `a·w² + b·w + c` through the two points, plus `9.25·(lost cylinders share)²` and the rough
  running above (11.25 with two or fewer cylinders left). Torque is multiplied by
  `1 + U(−1, 0.1)·amplitude`, and is 0 when that is negative below mid rpm.
  OpenIL2 feeds the propeller governor the acceleration without this noise, so the governor
  doesn't chase it.

Keys: H richer, J leaner, M next magneto setting. The speed bar shows both. DeviceLink keys 106/108
read the magneto and 107/109 switch it.
