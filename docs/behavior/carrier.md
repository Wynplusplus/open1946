# Carriers

Ships whose `com/maddox/il2/objects/ships.ini` section has `IsAirport 1` are airports
(**Verified** from the runtime's ship, airport and gear classes unless marked OpenIL2).

## Deck hooks
In the ship's hierarchical mesh, in ship coordinates (x forward, y left, z up from the waterline):
- `_RWY_TO`: take-off frame; its x axis is the take-off direction and its height the deck height.
  It sits at the forward end of the flight deck.
- `_RWY_LDG`: landing frame (stern part, x axis pointing aft).
- `_TOWnnA`, `_TOWnnB` (`nn` = 00, 01, …, lower or upper case): the two ends of arrestor wire `nn`.
  The runtime draws a caught wire with `3DO/Arms/ArrestorCable/mono.sim` (not yet in OpenIL2).
- `_ParkNN`: corners of the take-off parking area (stern part); `_LParkNN`: of the landing parking
  area.

Example, USS Casablanca (CVE-55): deck at 13.4 m, `_RWY_TO` at x 70.6, `_RWY_LDG` at −43, nine wires
from x −50.9 to 8.9 spanning y ±11.1, park area x −67.7…−28.9.

## Deck
The runtime looks along a 20 m line down from an aircraft's origin for a ship; the hit point (+0.5 m
on the ship's collision mesh) becomes the ground under the aircraft, it is not water, and the ship's
velocity is subtracted from the aircraft's for the wheel and skid forces.

OpenIL2: the deck is rasterised once per carrier class from the LOD-0 render mesh: every triangle whose
normal is within `acos 0.7` of vertical adds its height to the 1 m cells it covers. The surface under an
aircraft is the highest such height at most 20 m below its origin and not above it; the ground plane
there is level. An aircraft standing still on the deck (static friction holding it) moves with the ship.

## Take-off
Flights whose first waypoint is `TAKEOFF … <N>_Chief <unit>` start on that ship. The runtime packs
their aircraft into the `_Park` area's cells (`CellAirField`), at the deck height plus the aircraft's
`[Gear] H`, oriented like `_RWY_TO`, moving with the ship, brakes on; without a park area they start
300 m up, 200 m apart behind the ship.

OpenIL2's layout: per carrier, every aircraft of every flight starting there takes the next slot in
mission order (the player's flight included). Slots fill rows across the park area from its forward
edge aft: `span + 1` m apart sideways, `length + 2` m fore and aft. AI aircraft wait 3 + 15 s per slot,
then take off along the deck.

## Arrestor wires
Aircraft with `[Controls] CArrestorHook 1` have a hook (`_ClipAGear`), lowered at 0.2 per second
(key Y in OpenIL2).
- Catching: on a deck, with the hook more than 10 % down and the aircraft faster than √10 m/s over
  the deck, the hook catches the first wire whose plan-view segment its path since the last tick
  crosses while it was lower than the wire's mid height + 0.2 m.
- While caught: with `v0` from the hook to the wire's midpoint in aircraft coordinates, the wire
  pulls `1000·v0` N (a linear spring from the middle of the wire), with moment `hook × 0.3·force`, as
  long as the aircraft still moves forward relative to the deck. If the midpoint gets ahead of the hook
  the wire lets go; once the aircraft is slower than 1 m/s over the deck the wire releases with
  probability 0.008 per tick.
- The hook's own droop is class code (each class pivots it towards the deck). OpenIL2: the lowered
  hook point may hang up to 1.5 m below its lowered position, riding on the deck surface.

Wing folding (`CWingFold`, rate `1 / WingPeriod`, default 0.1 per second) is a control channel
(`moveWingFold`); OpenIL2 folds only on the ground (key T).

`openil2 --mission <carrier mission> --test` runs a deck take-off and, for hooked aircraft, a trap
(approach 15 m behind the stern at 120 km/h airspeed): F4F-3 from the CVE-55 lifts off after a
~250 m run (deck plus ship's way) and traps on wire 7, stopping ~25 m forward of it.

## AI landings on carriers
A `LANDING … <N>_Chief <unit>` waypoint lands on that ship. The runtime lowers the hook with the gear,
aims for at least ship speed + 10 m/s, and below 10 m only softens sinking beyond 5.5 m/s (×0.95 per
tick) and damps climbing (×0.91); after stopping, the aircraft folds its wings, raises the hook and is
removed after 25 s.

OpenIL2: the touchdown point is the deck's `_RWY_LDG` spot, recomputed every tick as the ship moves,
with the ship's heading as the landing direction. The approach (`ai.md`, "Landing") descends at the
closing speed over that moving point, ends with the wheels 1 m over the spot (no flare: it flies onto
the deck and the hook stops it) and holds the centre line tighter close in (`0.004 + 2/distance` per
metre of offset). `--test` on a carrier mission also flies this: the AI F4F-3 and SBD-3 trap on the
CVE-55 and park on the deck.

Not yet in OpenIL2: the landing pattern (`landWay`), deck parking after landing, the drawn wire, ship
rocking and the collision mesh.
