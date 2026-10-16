# Ground and naval columns

## Mission data
`[Chiefs]` lists moving columns, one per line: `name Category.Type army [delay skill slowfire]`,
e.g. `0_Chief Armor.3-BT5 1`. The route is `[name_Road]`, one point per line:
`x y width [wait …]`. A negative width marks a bridge segment (`−index − 1`), and widths above 100
count as 60. The fourth number is the time to wait at the point in minutes. Lines written by the
mission builder carry two more numbers, the last being the leg's speed in m/s.

`com/maddox/il2/objects/chief.ini` lists the types per category (`[Armor]`, `[Vehicles]`,
`[Trains]`, `[Ships]`) with the chief class and map icon. A section `[Category.Type]` lists the
units of such a column by class, one per line (`com.maddox.il2.objects.vehicles.tanks.Tank$BT5`
three times for `Armor.3-BT5`).

Unit models:
- vehicles: `technics.ini` section named after the class's short name (`[BT5]`): `MeshSummer`,
  `MeshWinter`, `MeshDesert`, armour `PanzerBody*`, `SpeedAverage` (km/h), `BestSpace` (m, the
  gap in a column);
- ships: `ships.ini` (`[Tramp]`): `Mesh`, `Speed`;
- trains: the wagon and locomotive classes name their models in their code (the first `3do/…`
  `.him`/`.sim` string constant).

## Movement and damage (OpenIL2's own design)
The runtime's `ChiefGround` with road formations, bridges and collision avoidance is not ported
yet. OpenIL2's columns:
- The head moves along the route at the leg's speed (else the lead unit's `SpeedAverage`, ships
  `2·Speed/3.6`), waiting at points with a wait time. Units follow at `1.6·BestSpace` (at least
  12.8 m; ships 250 m; trains 18 m), on the terrain (ships at sea level), facing along the road.
- Gunfire: a round with kinetic energy above `1.5·10⁸·PanzerBodyTop²` J destroys a vehicle one
  time in four. Ships ignore gunfire.
- Blasts: TNT received (the realistic bomb-sight falloff, `damage.md`, radius 3 m) above
  `40·PanzerBodyTop` kg (at least 0.08 kg) destroys a vehicle; ships need 30 kg.
- Wrecks stay, sunk a little and tilted. The rest of the column drives on.
