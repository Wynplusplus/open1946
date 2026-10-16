# Visual effects

Status: the **choice of effects** is specified from the runtime (`Explosions`); the particle system itself
is native and **not studied** — OpenIL2 interprets the `.eff` parameters by their names (marked below).

## Effect files (`.eff`, section-file syntax)
`[ClassInfo] ClassName` is one of `TParticlesSystemParams` (most), `TAnimatedSpriteParams`,
`TSmokeTrailParams`, `TSmokeSpiralParams`. `[General]` holds `MatName` (a material path relative to the
`.eff`, with `\` separators and `..`) and, for particle systems:
`nParticles`, `FinishTime`, `LiveTime`, `EmitFrq`, `EmitVelocity min max`, `EmitTheta min max`, `GasResist`,
`VertAccel`, `Wind`, `Size min max`, `Color0`/`Color1` (RGBA), `MaxR`, `PhiN`, `PsiN`, `Rnd`,
`TranspTransitionTime`, `FullRegenAtTimeout`. Sprites: `Radius0/1`, `RadiusPeriod`, `Angle`, `AngleSpeed`,
`Color0/1`, `ColorPeriod`.

*OpenIL2 interpretation:*
- **Emission.** An emitter emits `EmitFrq` particles/s for `FinishTime` s (or the duration the runtime passes
  when starting the effect), keeping at most `nParticles` alive.
- **Direction and speed.** Particles leave within a cone of half-angle `U(EmitTheta)` degrees around the
  emitter axis, at `EmitVelocity.0 ± EmitVelocity.1` m/s, from up to `MaxR` m off the centre.
- **Motion.** Particles slow by quadratic drag `dv/dt = −GasResist·|v|·v` and rise by `VertAccel`.
- **Appearance.** Over `LiveTime` the size (billboard half-width) goes `Size.0 → Size.1` and the colour
  `Color0 → Color1` (alpha fades). The material's alpha test applies to the texture before the colour.

## When effects start (runtime)
- **Shots** (non-explosive), by round mass:
  - below 0.05 kg: `effects/Explodes/Bullet/<Land|Water|Object>/`
    (`BaseSpray`, `Fontain`, `Spray`, on land 20 % `Flash`);
  - below 0.701 kg: `Cannon/…` (objects `Sparks`, land 30 % `Flash`);
  - heavier: the 10 kg explosion.
- **High explosive on land or water**, by kg TNT:
  - below 0.0019: 30 % `Bullet/…/Exp_*`;
  - below 0.005: `Cannon/…/12mm_Exp_*`;
  - below 0.05: 40 % `Cannon/…/20mm_Exp_*`;
  - below 1: `CannonBig/…`;
  - below 15: `Explode10Kg/…` (`BaseSpray`, `Flame`, `Flash`, `Dirt`, `Spray`);
  - below 50: the RS-82 fountain;
  - else the Bomb250 or Bomb1000 fountain.
- **Fountains** (`effects/Explodes/<Bomb250|Bomb1000|RS82>/<Land|Land_dig_in>[_2]/`): `Peaces` once
  (3.5 s), then 4–5 times (Bomb1000 3–5, RS-82 2–3) at a random heading and 90° ± 30° elevation: `Fontain`
  (10 s), `Burn_Fog`, `Shock`, `Base`. Craters, surface waves and lights: TODO.
- **Effect size.** The runtime passes a size factor (1.0 or 0.75) when it starts an effect.
