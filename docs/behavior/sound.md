# Sound

Status: **data layout specified**; the native sound engine (`mg_snd.dll`) is not studied, so the control
curves are interpreted by their names (marked).

## Files
- Samples: `samples/<file>.wav` (in `fb_sound*.SFS`), PCM, mostly mono 16-bit 44.1 kHz.
- Presets: `presets/sounds/<name>.prs` (section-file syntax); `my_presets/sounds/` may override engine presets.
  - `[common]`: `mode`, `type` (`mixer`), `infinite`, EAX settings.
  - `[spl] <dB>`: loudness.
  - `[samples]`: the sample files.
  - `[sample.<file>]`: `infinite` (loop), `env` (listening environments: 0, 2, 8), `controls <names…>`,
    `period`, `usr <variant>`.
  - `[sample.<file>.<control>]`: `minlo minhi maxlo maxhi value pmin pmax` (bounds controls) or
    `gain0 gain1` (`env`).
  - `[emit]` / `[emit.<name>]`: random one-shot emitters (`speed`, `srand`, `vrand`, `prand`, `spl`, `num`,
    `sample<i>`).
- **Controls** (runtime ids): `speed` 64, `angle` 65, `dist` 66, `env` 67, `rpm` 100, `damage` 101,
  `diving` 102, `flatter` 103, `brake` 104, `gear` 105, `start` 106, `load` 107, `motor_ld` 108, `prop_ld`
  109, `door` 110, `relspeed` 111, `motor_stage` 112, `bomb_speed` 200, `bomb_time` 201, `massa` 202.
  Emitters: 500 run, 501 speed.
- **Which preset.**
  - Engine: `motor.<SoundName>`, where `SoundName` comes from the `.emd`. A custom preset named after the
    sub-model or family wins; the default is `db605`. Start/stop: `motor.<StartStopName>.start.begin/end`
    (default `std_p`).
  - Propeller: `propeller.<PropName>`.
  - Guns: the gun's `sound` property (`weapon.mgun_13_800`, …).
  - Bombs: the round's `sound`.

## Playback (OpenIL2 interpretation)
- **Bounds controls.** Gain is 0 below `minlo`, rises to 1 at `minhi`, holds until `maxlo`, and falls to 0
  at `maxhi`. It is multiplied by `value/10`. Pitch goes linearly from `pmin` at `minlo` to `pmax` at
  `maxhi`. The `rpm` control is the engine RPM.
- **What plays.** Looping samples play while their source runs. One-shot presets (explosions) play one
  random sample at `10^((spl − 130)/40)`.
- **Mixing.** Software mixing at 44.1 kHz. Distance attenuates as `1/(1 + d/40)` for the player's aircraft
  and `1/(1 + d/300)` for explosions. Output is ALSA's default device, with `libasound.so.2` loaded at run
  time.
