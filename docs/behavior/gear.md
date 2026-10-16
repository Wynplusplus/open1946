# Landing gear and ground contact

Status: **specified from the runtime's gear code**, except moving decks, water, plates' damage effects and
arrestor hooks.

## Contact points
- **Points.** Wheels: hooks `_ClipLGear`, `_ClipRGear`, `_ClipCGear` (the third wheel is a nose wheel when its
  hook has `x > −1`). Airframe: hooks `_Clip00`, `_Clip01`, …, in the **current pose** (gear animation
  applied). OpenIL2 uses the gear-down pose. When the gear animation of a class is not decoded, the wheel
  heights are solved from `[Gear] H/Pitch` instead (`flight-model.md`).
- **Parked pose.** Unless `[Gear] FromIni 1` (then `H`, `Pitch`): `Pitch` levels the line through the main
  wheels' mid-point and the third wheel, `H` is the origin's distance from that line minus
  `(empty + fuel + nitro mass)·g/2 700 000`.
- **Ground plane.** Taken at the point below the aircraft (normal from the terrain). `d` = a point's height
  above it (negative when touching). Nothing is computed above 50 m.
- **Velocities.** `Vs` = the point's velocity in the body frame (`V + W × P`). With the plane basis
  `Forward = Y × Normal` and `Right = Normal × Forward`, the components are `Fv`, `Rv`, `Nv` (`Nv` clamped
  to ≤ 0 for wheels).
- **Mass coefficient** `k = min(1, 4·10⁻⁴·mass)` scales the wheel forces and the airframe springs.
- **Moments** use the point shifted by `[Arm] GC_Gear_Shift` in x.
- **Retracted gear.** Wheels count only with the gear lever above 0.01.
- *OpenIL2:* the hooks are used only when they give a plausible stance (nose-wheel aircraft within ±6°;
  tail-draggers 3–25° and within 4° of `[Gear] Pitch`); otherwise the `H/Pitch` solution applies. This
  guards against partly imported gear animations. With it, 382 of 404 aircraft park within 3° of
  `[Gear] Pitch` or plausibly level (nose-wheel aircraft).

## Wheels
- **Roughness.** It is reset from the surface every other tick (0 on concrete plates, 0.8 on "ground"
  plates and roads, 0 on highways, 5 on railways, else 3.8 (fields)). For each wheel contact:
  - `b = clamp(0.01·(Vx² + Vy² − 2), 0, 4.5)`, limited to `0.4·max(r², r)`;
  - the stored roughness becomes `max(min(r, b), 0.2)`, so it collapses within a step;
  - `d += U(−2, 1)·0.04·b·k`.
- **Spring.** `N = max(−9500(d − 0.1), −950 000·d)`, times `SpringsStiffness` (1) for main wheels. The third
  wheel uses 0.45 for a nose wheel moving forward, else `TailStiffness` (0.6). Then `N −= 40 000·Nv`.
  Friction limits use `n = 10⁻⁴·N`.
- **Main wheels.**
  - Braking: `brake = Brake`; with the rudder past ±0.1 the left wheel gets `brake − 1.2·brake·(rudder ∓ 0.1)`
    and the right `+1.2…`, clamped 0..1.
  - With `s = |(Fv, Rv)|` (min 0.01), `cf = |Fv|/s`, `cr = |Rv|/s`: the side factor `K = 5`, raised by
    `7·|Rv|` (max 20) when the wheel is on the outside of the skid. The cap `C = 15 000`, plus
    `3000·((3 − s)/3)²` below 3 m/s.
  - Side force `−K·10⁵·Rv·n`, limited to `C·n·cr`.
  - Rolling force `−K·600·Fv·n`, limited to `K·max(200(1 − 0.04s), 5)·n·cf`.
  - Brake: `c = 0.03` (0.06 with a nose wheel), plus `3(1 − |Fv|)` below 1 m/s, times `0.03·brake`; it adds
    `−300 000·c·Fv·n` to the rolling force, limited to `C·n·cf`.
  - The combined force is limited to `C·n·cf`.
  - A wheel breaks when `Fv²·8·10⁻⁵ + Rv²·0.0068 + Nv²·0.07 > 1` (damage: TODO).
- **Locked tail wheel** (lock on, steering within ±5°): forward `−400·Fv` (±400), side `−10 000·Rv` (±40 000).
- **Nose wheel.** Steering angle → `atan2(−0.5·rudder, 1)`, 11.5 % per step. Along the wheel `−100·Fv`
  (±4000), across `−500·Rv` (±4000), combined at most `5000·max(0.1, 1 − 0.02s)`.
- **Castering tail wheel.** `−1000·(Fv, Rv)`, combined at most 1500.
- The third wheel breaks when `(Fv² + Rv²)·10⁻⁴ + Nv²·0.02 > 1`.
- All wheel forces × `k`.

## Airframe points
- `Nv` above 0 is reduced by 3 (floor 0). Spring `max(−120 000·clamp(d − 0.06, −2, 0), −360 000·clamp(d + 0.04, −0.25, 0))·k`,
  plus `clamp(−40 000·Nv, ±100 000)`.
- Friction `(−8000·Fv, −50 000·Rv)`, combined at most `20 000·(1 − 0.5|y|/WingEnd)`, times 0.8.
- They also count within 0.1 m above the ground. A hit breaks when `(Fv² + Rv²)·2.367·10⁻⁴ + Nv²·0.0204 > 1`.

## Integration and static friction
- **Sub-steps.** With any ground force, a step uses 5 sub-steps, recomputing the gear between them.
- **Static friction.** `dry` decays by 0.01 per step, resets to 1 when the number of contacts changes, grows
  by 0.02 per step with airframe contact, and stays within 1..32. With `k = (4·(0.25 − power))²·dry`
  (power = throttle × readiness, 0 when ≤ 0) and `m2 = k·mass²`, the step is **skipped** (the aircraft does
  not move) unless one of these holds:
  - no airframe contact and fewer than 3 wheels down;
  - `k = 0`;
  - `|M|² > 2·m2` or `|F|² > 80·m2`;
  - `|W|² > 1.4·10⁻⁴·k` or `|V|² > 0.09·k`.
