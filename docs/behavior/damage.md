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

## Bridges
**Verified** from the runtime's bridge classes. Each segment (`formats/static.md`, "Bridges") has two
halves, 0 towards the bridge's begin and 1 towards its end, each with its own life: 240 with an ignore
threshold of 100 kg TNT on highway and railway bridges, 120 and 10 kg on country-road ones.

- A segment's deck runs from `a = p − ½·step` to `c = p + ½·step` (`p` its centre, `step` from one segment
  centre to the next) with a middle point `b`. Middle segments: `a`, `b = p`, `c` at the deck height. First
  segment: `b = a + l·step̂`, heights `a` 0, `b` the ramp height, `c` the deck height; the last segment
  mirrors it. Deck height and width are the intact end mesh's `Top` hook (`z`, `2·|y|`); the ramp height is
  its `Mid` hook `z`, `l = −Mid.x + ½·segment length` (100 m axis-aligned, 141.42 m diagonal; half each).
- **TNT on a segment** `t` from point `q`: nothing if `t ≤ ignore`; else damage `U(0,1)²·2t + 0.8t −
  ignore/2` to half `dir·(q − p) < 0 ? 0 : 1` (`dir` the bridge's unit-step direction), if that half
  still stands.
- Explosions reach every segment that is not wholly down: `d` = distance from the blast to the deck line
  `a–c` minus the bridge width (at least 0); TNT `power` below 1 m, `power/d²` below the blast radius,
  else 0.
- Shots (at segments not wholly down) give their kinetic energy × 2.4·10⁻⁷ kg TNT, except armour-piercing
  rounds; in practice only blasts matter.
- A half reaching 0 shows the collapse (smoke columns along that half of the deck, at least two and two
  per bridge width, 3 s) and the segment its damage state `s = (half 1 down ? 2) + (half 0 down ? 1)`:
  mesh `mono1` for 0, `mono3` for 3, else `mono2`. Middle segments with `s = 2` take the last segment's
  orientation (no 180° turn), so the broken end faces the gap.
- Destruction spreads: half 0 down brings down half 1 of the previous segment, half 1 down half 0 of the
  next; the first segment loses both halves when its half 0 goes, the last when its half 1 goes.
- The bridge counts as destroyed (objectives, `campaign.md`) once any segment is damaged.
- Not yet in OpenIL2: ships ramming segments (TNT `life + 1`), columns and trains stopping or falling at
  broken segments, damaged bridges in DGen's saved state.

## Aircraft

### Parts and damage levels
An aircraft mesh splits into chunks named after parts with a damage level: `WingLIn_D0`,
`WingLIn_D1`, … (the `_D0` chunk is intact, higher levels are hidden at the start). A chunk
belongs to the first entry of the runtime's part list its name starts with: `AroneL`, `AroneR`,
`CF`, `Engine1`–`Engine4`, `GearC2`, `FlapR`, `GearL2`, `GearR2`, `Keel1`, `Keel2`, `Nose`, `Oil`,
`Rudder1`, `Rudder2`, `StabL`, `StabR`, `Tail1`, `Tail2`, `Tank1`–`Tank4`, `Turret1B`–`Turret6B`,
`VatorL`, `VatorR`, `WingLIn`, `WingLMid`, `WingLOut`, `WingRIn`, `WingRMid`, `WingROut`,
`Flap01`–`Flap04`, `NullPart` (indices 0–43 in this order).

- **Next damage level.** The visible `_Dn` chunk is hidden and `_Dn+1` shown, and the flight model
  takes a hit to that part (below). Without a further level, the part is cut off.
- **Cut.** All `Name_D*` chunks with their whole subtrees leave the aircraft as wreckage carrying
  the aircraft's velocity. An outer wing section and its aileron go with the middle section, and
  `Name_CAP` (the broken stub), if present, is shown. Then the flight model loses the part.
- A hard **collision** (`msgCollision`) applies four damage levels at once. Structural failure
  and ground strikes use it.

### Flight-model effects
The runtime keeps per-part areas: six wing sections (the `.fmd` sections, mirrored), stabilizer,
elevators, keel, rudders. It also keeps roll/pitch/yaw sensitivity factors and an added drag
coefficient `ΔCx` that applies at the wing halves like parasite drag. Each half wing's force is
its coefficient times the sum of its three sections.

A **hit** (one damage level):

| part | effect |
|---|---|
| aileron | outer section of that side × 0.95, roll sensitivity × 0.68 |
| keel | keel × 0.68, ΔCx + 0.12 |
| rudder | rudders × 0.5, yaw sensitivity × 0.68 |
| stabilizer | stabilizer × 0.68, ΔCx + 0.12 |
| elevator | elevators × 0.68, pitch sensitivity × 0.68 |
| tail | load limit damage (slot 6) |
| wing section | section − 0.8 m² (OpenIL2 stops at 0), ΔCx + 0.12, load limit damage (slots: L out 0, L mid 1, L in 2, R in 3, R mid 4, R out 5) |

A **cut**:

| part | effect |
|---|---|
| fuselage (`CF`) | aircraft destroyed, engines stop, then as tail |
| tail | centre of gravity moves to 0, random ±0.1 rad/s pitch and yaw rate, pitch inertia halved, both stabilizers and keels cut; from then on extra moments `(0, 8·S·Vx, 200·S·prop sign)` tumble the aircraft |
| engine | engine dead |
| gear leg | that wheel no longer rolls (its contact point scrapes like the airframe) |
| keel | keel area 0, rudder cut (twin-fin layouts, `Scheme2a/5/7`: keel × 0.25 if the other fin remains) |
| rudder | rudders 0 and yaw sensitivity 0 (twin fin: × 0.5 and × 0.25) |
| stabilizer | stabilizer × (0.5·other + 0.1), ΔCx + 0.12, that side's mid and outer wing × 0.9, the other's × 1.1; with both gone, aileron trim ∓0.25 and one half × 0.95, the other × 0.75 at random; then the elevator on that side is cut |
| elevator | elevators and pitch sensitivity × 0.5·(other elevator intact) |
| wing root | root × 0.25, gear leg on that side cut, then as middle |
| wing middle / outer | the sections outboard are cut too; the whole half loses all lift (as the runtime does), ailerons stop working, then the aileron is cut |
| aileron | aileron area × 0.5, roll sensitivity × 0.5·(other aileron intact) |

### Structural limits
`LimitLoad = G_CLASS / 1.5` (12 without `G_CLASS`). The reference force is
`LimitLoad·(empty + fuel + nitro + ReferenceWeight)`. Every step the current limit is
`reference force / mass`, the negative limit is half of it, negated, and the ultimate load is
`limit × SafetyFactor` (starting at 1.5). With the load factor `n` (aerodynamic and gear forces
along body z over weight):

- `n > ultimate + U(2, 4)` or `n < negative limit − U(1, 2.5)`, airborne, with a 2/101 chance
  per step: a wing or the tail breaks. It is the most recently damaged section if any (slots
  above, slot 6 the tail); otherwise `Tail1` 1/9, right mid 2/9, left mid 2/9, left root 2/9,
  right root 2/9.
- Beyond the limits for more than 0.85 s at a time (the second such cycle onward), the safety
  factor drops by `((n_max − limit)/limit)²`. The flutter speed becomes
  `VmaxAllowed·(0.3·SafetyFactor + 0.55)`, and a random draw may weaken a control
  sensitivity, fail the flaps or reduce an outer wing section.
- Flaps fail (retract and stop working) above 81 m/s IAS when `(IAS − 81)·flaps > 8` with the
  lever beyond 0.21.
- Above `VmaxAllowed` (default 1.3 `VmaxH`), with a 2 % chance per step when
  `U(0, 16) < IAS − VmaxAllowed`, flutter tears off a part: an aileron 5/30 each, an elevator
  4/30 each, one wing section 1/30 each, a rudder 3/30 each.
- Above 310 m/s IAS, or when the aerodynamic force exceeds 50 g, the fuselage fails.

### Ground strikes
Contact points report strikes (`docs/behavior/gear.md`). A broken main or third wheel strikes
its gear leg (`GearL2_D0`, `GearR2_D0`, `GearC2_D0`). An airframe point (`_Clip00`…, on the chunk
whose mesh carries the hook) strikes its chunk when the aircraft moves at 13 m/s or more. The
fuselage needs a severity above 2 and `Tail1` above 1.3.

OpenIL2 models the wreckage as a tumbling body with quadratic drag (`0.004·v²` per unit mass) that
rests where it lands. The design is its own.
