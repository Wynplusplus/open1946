# Damage

Status: **specified from the runtime** for houses; aircraft and vehicles: TODO.

## Shots and explosions
- **Shots.** A bullet hit delivers a *shot* of energy `½·massa·v²` (J), with a type:
  - 1 shaped charge (`cumulativePower > 0`, which replaces the energy);
  - 0 high explosive (`powerRadius > 0`);
  - 3 explosive filler without radius (`power > 0`);
  - 2 plain.
- **Explosions.** High-explosive rounds also explode with `power + 0.03·massa` kg TNT over `powerRadius`.
  An object of collision radius `r` at distance `d` receives `(1 − (d − r)/radius)²·power` (all of it when
  that factor is ≥ 1, none beyond the radius). With "realistic bomb sights" (the full-realism setting,
  which OpenIL2 uses) it is `power/((d − 0.7r)/2)^1.5` instead, all of it when `(d − 0.7r)/2 < 1`.

## Houses (`static.ini` `Body`, `Panzer`)
| Body | material | MIN_TNT | MAX_TNT | death probability at MIN |
|---|---|---|---|---|
| WoodSmall | 0 wood | `0.25·P/0.025` | ×2 | 0.01 |
| WoodMiddle | 0 | `5·P/0.15` | ×1.7 | 0.01 |
| RockMiddle | 1 stone | `6·P/0.12` | ×1.7 | 0.01 |
| RockBig | 1 | `12·P/0.24` | ×1.7 | 0.05 |
| RockHuge | 1 | `30·P/0.48` | ×1.7 | 0.05 |
| FuelSmall | 2 fuel | `0.2·P/0.002` | ×1.7 | 0.01 |
| FuelBig | 2 | `0.8·P/0.008` | ×1.7 | 0.01 |
| Flesh | 3 | `0.01·P/0.002` | ×1.7 | 0.1 |

- **Shots.**
  - Flesh dies from shaped charges or any hit faster than 20 m/s.
  - Fuel dies from a shaped charge when `Panzer ≤ 0.256·(power·U(0.75, 1.15))^¼`.
  - Other shaped charges have no effect.
  - Otherwise the kill probability comes from the penetration tables (below).
- **Explosions** (high explosive): with `R` received kg TNT, no kill at or below `MIN_TNT`, certain at or
  above `MAX_TNT`, else probability `p + (1 − p)·(R − MIN)/(MAX − MIN)`.
- A killed house shows its `MeshDead` (or disappears without one), standing on that mesh's own ground
  offset and `AddHeightDead`, and no longer collides.

**Penetration tables.** Per material, two thickness anchors `T0, T1` and per anchor three energies
`E0 < E1 < E2` (10 %, 30 %, 100 % kill):

| material | T0, T1 (m) | energies at T0 (J) | energies at T1 (J) |
|---|---|---|---|
| wood, flesh | 0.025, 0.15 | 23 400, 131 400, 252 000 | 131 400, 369 000, 1 224 000 |
| stone | 0.12, 0.24 | 131 400, 252 000, 3 295 500 | 369 000, 3 295 500, 5 120 000 |
| fuel | 0.002, 0.008 | 511.225, 2453.88, 23 400 | 2453.88, 10 140, 23 400 |

At each anchor, `p(E)` is 0 below `E0`, rises linearly from 0.1 at `E0` to 0.3 at `E1`, then to 1 at `E2`.
The two anchors' values are extrapolated linearly to `Panzer`. The result is 0 below 0.1 and clamped at 1;
for `n` hits it becomes `1 − (1 − p)ⁿ`.

## Ground hits
- Tracer rounds without a blast radius may ricochet. When the impact cosine `c = n·dir ≥ −0.25` and
  `U(0, 1) − 0.666 ≥ −c`, the velocity is reflected (`v − 1.1(n·v)n`), deflected sideways by `U(−1.2, 1.2)³·|v|`
  and upwards by `U(0, 1.2)³·|v|`, then scaled to `(dir·v̂)²·|v|`.
- High-explosive rounds explode at the impact point.
- OpenIL2 hit-tests objects against their LOD-0 bounding spheres.
