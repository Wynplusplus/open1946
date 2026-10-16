# Mission objectives and campaigns

## Objectives (`[Target]`)
One objective per line: `type importance timed minutes share x y radius _ object`.

- **type**: 0 destroy (a flight or column), 1 destroy ground (objects in the radius), 2 destroy
  bridge, 3 inspect (fly over the point), 4 and 5 escort, 6 defend ground, 7 defend bridge.
- **importance**: 0 primary, 1 secondary, 2 secret.
- **timed** (1) with **minutes**: a time limit (up to 720).
- **share**: required percentage × 10, plus 1 for "inspect by landing there" (`501` = 50 %, land).
- **radius**: 2–3000 m, default 1000. **object**: the flight (`r0100`), column (`1_Chief`) or
  bridge named.

OpenIL2 judges them when the flight ends (own rules after the runtime's `T*` target classes):
- destroy: at least the share of the named flight or column destroyed;
- destroy ground: at least the share of the destructible objects standing in the radius at the
  start gone;
- inspect: the player came within the radius after having flown, and stopped on the ground there
  if landing is asked;
- escort: at least the share of the named flight still flying;
- defend ground: no more than the remaining share of the objects in the radius gone;
- bridges can't be destroyed yet, so destroy-bridge objectives fail and defend-bridge ones succeed.

A mission succeeds when every primary objective is met and the player's aircraft was not
destroyed. `openil2 --result FILE` writes `result success|failure`, `player alive|lost` and one
`objective <type> <importance> <met>` line per objective when the flight ends.

## Static campaigns (Pilot Career)
`Missions/Campaign/<country>/<campaign>/campaign.ini` names the campaign class
(`CampaignBlue`, …), its awards class and, under `[list]`, the missions. Each line is a stage
listing one or more alternatives, of which one is flown. `info.properties` (localized) gives
`Name` and `Description`. Campaign folders without `[list]` are DGen dynamic campaigns.

OpenIL2's career: one campaign at a time, saved in `$XDG_DATA_HOME/openil2/career.ini` (else
`~/.local/share/openil2/career.ini`) as the campaign, next stage and the counts of missions flown
and won. A stage flies one of its alternatives at random; success moves on to the next stage,
failure repeats it. Ranks, awards and pilot rosters are not kept yet.

## Dynamic campaigns (OpenIL2's own generator on DGen data)
The original generates dynamic campaign missions with the Windows program `DGen.exe` from the
data in `DGen/`. OpenIL2 reads the same data and generates sorties itself.

- `DGen/<Theater>.DB` (ISO-8859-1): first line the map folder, then `[Towns]` (weight, name; the
  positions come from the map's `texts.txt`: `x y … name`), `[Schedule]` (days `yMMDD` with year
  `194y`, and the front's movement), `[OOB]` (squadrons), `[Planes]` (count and class in service),
  `[Aces]`, and `[Operations] <<normal.dat easy.dat hard.dat`.
- Operation files: one operation per line,
  `name date:date LABEL flights… LABEL flights…`, e.g. `oRuRecon 10622:50509 VVS 2Y LW 2F`.
  The first label's flights are red's, the second's blue's. A flight is a count and role letters:
  `Y` the player's flight, `F` fighters, `B` bombers (`dB` dive, `fB` fighter-bombers, `hB`
  heavy, `nB` night), `A` attack aircraft, `T` transports, `R` reconnaissance, `P` paratroopers,
  `x` with bombs. This reading of the letters is OpenIL2's interpretation.
- `DGen/<Theater>Red.mis` and `<Theater>Blue.mis`: each side's ground forces (`[Chiefs]` with
  roads, `[NStationary]`).

A sortie (own design):
1. Pick a random operation where the player's side has a `Y` flight. `oRu…` (and `oUs`, `oGb`)
   operations are started by red, the rest by blue.
2. The target is over the defender's ground for attacks, bombing, reconnaissance, verification,
   free hunts, intercepts and rescues, otherwise over the starter's own ground. The name's ending
   picks it: `AF` the side's airfield (the runway nearest its forces), `Armor`/`Soft`/`Cluster`
   a column (`Armor.` columns for armor) among the five nearest the front, otherwise one of the
   four theater towns on that side nearest the front.
3. Flights take off from their side's airfield, at most four aircraft each. The player's flight
   flies the chosen aircraft; others take aircraft from the theater's `[Planes]` of their side,
   fighters for fighter roles and others otherwise. Bombing roles take the class's first loadout
   with bombs and attack the target (`GATTACK`). The starter's flights fly to it and back, the
   defenders patrol four points 3 km around it, 500 m higher. Altitude 1500–3500 m, hour 7–16.
   The date follows the schedule, one day every two sorties.
4. Both sides' ground forces are included. The player's objective: destroy half the target column
   or the objects around the target when attacking, defend them when defending, otherwise fly
   over it.

The menu's Dynamic Campaign screen lists the theaters, side and aircraft, and counts sorties flown
and won in `$XDG_DATA_HOME/openil2/dynamic.ini`.
