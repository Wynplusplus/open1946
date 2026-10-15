# Piston engine and propeller

Status: **specified from the runtime's engine model** for running piston engines (`Type Inline/Radial`).
Not yet covered: start-up and shut-down stages, mixture, magnetos, failures, temperatures, WEP
(afterburner types), negative-g carburettor cut, engine roughness, jets and rockets.

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
