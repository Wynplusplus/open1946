# Validation against the original's reference figures

`openil2 --test` flies level at full power (110 % where boost exists) with a simple autopilot and compares
the result with the `[Params] Vmax`/`VmaxH` figures in each flight model. Those figures are the original
designers' targets, not measurements of the original runtime. Measured 2026-10-15:

| aircraft | low altitude (OpenIL2 / reference) | high altitude (OpenIL2 / reference) | notes |
|---|---|---|---|
| Bf 109 G-6 (early) | 530 / 525 km/h | 624 / 643 km/h at 6700 m | |
| Bf 109 F-4 | 521 / 520 | 618 / 628 at 7000 m | |
| Fw 190 A-4 | 568 / 547 | 654 / 640 at 6300 m | |
| P-51D-20 | 586 / 595 | 700 / 703 at 7620 m | two-stage supercharger |
| La-5FN | 587 / 583 | 610 / 648 at 6500 m | with boost |
| Spitfire IXc | 540 / 543 | 646 / 655 at 6600 m | with boost |

Low altitude runs at 500 m and the references are at sea level, so OpenIL2 should read slightly high there.

Ground handling (Bf 109 F-4, Kuban mission start, full power, simple pilot): lift-off at about 207 km/h after
about 480 m. Parked attitude for all of the above is within 0.7° and 2 cm of `[Gear] Pitch` and `H`.

## All aircraft (`openil2 --survey`)
2026-10-16, every class in `air.ini` (404): all load and fly, including jets, rockets, pulse jets and
gliders. The checks fly at full throttle with emergency boost. Level speed at 500 m against `Vmax` has a
median of 0.0 %, and at `HofVmax` against `VmaxH` −1.8 %. Within ±5 % at both altitudes: 245 aircraft;
±10 %: 330; ±15 %: 370. (Before boost was modelled: 189 / 321 / 365.) Some references are clearly
not measurements; for example, Yak-15 `Vmax 567` km/h is below the real aircraft's ~786.
