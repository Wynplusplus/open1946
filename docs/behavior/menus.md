# Menus

`openil2` without options opens the menus (also `--menu`). Labels come from the install's
`i18n/gui.properties`, or `gui_<lang>.properties` for the language in `IL2_LANG` or `LANG`
(`de`, `fr`, `pl`, …). Text uses `lib/fonts/LucidaSansRegular.ttf` with Latin-1 and Latin
Extended-A. A chosen flight runs as a child process (`openil2 --mission …`); the menu window hides
until it ends. The screens are OpenIL2's own layout; the original's GUI screens and their
textures are not reproduced.

## Single missions
`Missions/Single/<country>/<aircraft>/*.mis`, three lists (country, aircraft, mission). Each
mission's `<name>.properties` (or `<name>_<lang>.properties`, Java properties in ISO-8859-1)
gives `Name`, `Short` and `Description`. The description is shown as the briefing.

## Quick Mission Builder
Templates are `Missions/Quick/<Map>/<Map><Red|Blue><Situation><nn>.mis` (for example
`KubanBlueAirbase00.mis`). The runtime's QMB keeps the template's ground objects and routes and
rewrites the flights:

- flights `r010n`/`r011n` (red) and `g010n`/`g011n` (blue): up to four own and four additional
  flights per side. Unused ones are removed from `[Wing]` together with their sections;
- a used flight's section becomes `Planes`, `Skill`, `Class`, `Fuel`, `weapons` (plus skins);
- waypoint heights become the chosen altitude, except `TAKEOFF` and `LANDING` (0). In the
  "situation" variants one side gets the position difference added.

OpenIL2's QMB: side, map, situation (the templates for that map and side), the player's aircraft,
count (1–4) and skill, one hostile flight's aircraft, count (0–4) and skill, altitude and time of
day. The player flies `r0100` or `g0100`, the hostiles are `g0100` or `r0100`; all other flights
are removed. The mission is written to the system's temporary directory as `openil2_quick.mis`.
Aircraft lists come from `air.ini` (the number after the class is the side) with names from
`i18n/plane.properties`.
