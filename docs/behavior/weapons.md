# Weapons

Status: **specified from the runtime** for guns, bombs and rockets; turrets, torpedoes, fuzes: TODO.

## Data
- **Loadouts.** An aircraft class's static initializer calls `Aircraft.weaponTriggersRegister(cls, int[])`
  (trigger group per slot: 0 machine guns, 1 cannons, 2 rockets, 3 bombs, 9 other),
  `weaponHooksRegister(cls, String[])` (hook per slot, e.g. `_MGUN01`, `_CANNON01`, `_ExternalBomb01`) and,
  per loadout, `weaponsRegister(cls, name, String[])`, where each slot is `null` or `"<WeaponClass> <count>"`.
  The mission's `weapons` key picks the loadout (`default` otherwise). OpenIL2 runs the initializer in its
  restricted interpreter and records these calls.
- **Guns** (`com.maddox.il2.objects.weapons.<Class>`). `createProperties()` builds a `GunProperties`:
  - `shotFreq` (rounds/s), `shotFreqDeviation`, `maxDeltaAngle` (dispersion, degrees);
  - `traceFreq`, `bCannon`, `bullets` (default count);
  - `bullet[]` of `BulletProperties`: `massa` (kg), `kalibr`, `speed` (m/s), `power` (kg TNT),
    `powerType`, `powerRadius`, `timeLife` (s), `traceMesh`, `traceColor` (ARGB).

  After construction `bulletsCluster` is forced to 1.

## Firing
- **Rate.** One round every `1/shotFreq` s while the trigger is held, as long as ammunition remains.
- **Bullet type.** The type index advances before each shot and wraps, so the first round uses type 1.
- **Tracers.** Every `traceFreq`-th round (`round number % traceFreq == 0`) is one.
- **Direction.** The hook's +x, deviated by a yaw of `N(0,1)·0.37·maxDeltaAngle` (clamped ±3·max) and a
  pitch of `N(0,1)·0.173·maxDeltaAngle` (clamped ±2·max). Anti-aircraft guns use a quarter of that.
- **Velocity.** `direction·speed + aircraft velocity`.
- **Recoil.** An impulse `massa·speed` per round against the aircraft.

## Flight
Per step: `v += KV·|v|·v·dt`, `v_z −= 9.8·dt`, with `KV = −1.3462751·kalibr²/(8·massa)` ("realistic
gunnery"; without it there is neither drag nor gravity). A round lives `timeLife` seconds.

## Tracers
- **Near.** Within 500 m of the camera, the tracer mesh (a `.sim`, glowing, additive) is drawn along the
  velocity.
- **Far.** Beyond that, a line of `traceColor` the mesh's length. Rounds are not drawn in their first tick.
- *OpenIL2:* tracer meshes are widened by `max(1, distance/50)` across their axis instead of drawing lines.

## Bombs and rockets
- **Data.** Bomb and rocket weapons (`BombGun*`, `RocketGun*`) set class properties in their static
  initializers (`Property.set(cls, key, value)`): `bulletClass` (the round's class), `bullets`, `shotFreq`,
  `external`, `cassette`. The round class sets:
  - `mesh`, `massa`, `kalibr` (m), `power` (kg TNT), `powerType`;
  - `radius` (bombs default `7.07·√massa`, rockets 0);
  - rockets also `force` (N, default 500), `timeFire` (4 s), `massaEnd` (2.52 kg), `timeLife` (45 s),
    `maxDeltaAngle`.
- **Release.** One round per press per slot, at most every `1/shotFreq` s.
- **Bombs.**
  - Start with the aircraft's velocity and its axis plus `U(±0.04)` per component, with mass
    `massa·U(1, 1.06)`.
  - The initial spin axis is random, with an angle of `±U(2°, 35°)`, scaled above 60 m/s by
    `(1 − (V − 60)/290)·0.95 + 0.05` (0.05 from 350 m/s).
- **Rockets.** Launch along the aircraft axis at the aircraft's velocity plus 200 m/s along it, keeping a
  third of the vertical body speed. Thrust `force` acts for `timeFire` while the mass falls linearly to
  `massaEnd`. A rocket explodes after `timeLife`.
- **Drag.** With `S = π·kalibr²/4` and `KD(z) = 1 − 9.59387·10⁻⁵z + 3.53118·10⁻⁹z² − 5.83556·10⁻¹⁴z³`: above
  330 m/s `D = KF(v)·KD·S` with `KF(v) = (608.5 − 1.81327v + 0.00165114v²)·v`; below it `D = c·S·1.225·KD·v²/2`
  with `c = 0.06` for bombs and 0.2 for rockets.
  - Bombs: `v += (6·cos α·nose − 7·wind)·D/m·dt`, where `cos α = nose·wind` and `wind` is the airflow
    direction.
  - Rockets: `v += nose·(thrust − D)/m·dt`.
  - Both: `v_z −= g·dt`.
- **Bomb stability.** From 0.35 s: `F = −0.07·D·sin α·wind` (`sin α` taken as 0.08 at `cos α ≤ −0.996`) acts
  at `−0.2·kalibr` along the nose. The spin axis gains `(arm × F)/J·dt` with `J = 0.1·m·kalibr⁴`, is
  multiplied by 0.96 per step, and turns the nose.
- **Impact.** Rounds explode on the ground, on an object, or (rockets) at the end of their life; fuze delays:
  TODO.
