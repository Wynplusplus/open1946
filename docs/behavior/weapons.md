# Weapons

Status: **specified from the runtime** for guns; bombs, rockets, turrets: TODO.

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
