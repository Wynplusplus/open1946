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
