//! Particle effects from the game's `.eff` files (`docs/behavior/effects.md`). The native
//! particle code has not been studied; the parameters are interpreted by their names.

use crate::weapons::Rng;
use glam::{DQuat, DVec3, Mat4, Vec3};
use il2_formats::sectfile::atof;
use il2_formats::SectFile;
use il2_render::scene::Loader;
use il2_render::{GpuMesh, InstanceRaw, InstancedDraw, MeshData};
use il2_vfs::Vfs;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct EffectSpec {
    pub sprite: bool,
    /// Index of the billboard batch for this effect's material.
    pub batch: usize,
    pub max_particles: usize,
    pub finish: f64,
    pub live: f64,
    pub emit_frq: f64,
    pub velocity: (f64, f64),
    pub theta: (f64, f64),
    pub gas_resist: f64,
    pub vert_accel: f64,
    pub size: (f64, f64),
    pub color: ([f32; 4], [f32; 4]),
    pub max_r: f64,
    /// Sprites: radius range and period, spin.
    pub radius_period: f64,
    pub angle_speed: f64,
}

/// Normalises a game path: backslashes, `..` and `.` components.
fn join(dir: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    for p in rel.split(['/', '\\']) {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

fn parse(text: &[u8], path: &str, loader: &mut Loader, mesh: &mut MeshData) -> Option<EffectSpec> {
    let f = SectFile::parse(text);
    let class = f
        .section_ci("ClassInfo")?
        .get_ci("ClassName")?
        .trim()
        .to_owned();
    let g = f.section_ci("General")?;
    let nums = |k: &str| -> Vec<f64> {
        g.get_ci(k)
            .map(|v| {
                v.split_whitespace()
                    .filter_map(atof)
                    .map(f64::from)
                    .collect()
            })
            .unwrap_or_default()
    };
    let n = |k: &str, d: f64| nums(k).first().copied().unwrap_or(d);
    let pair = |k: &str, d: (f64, f64)| {
        let v = nums(k);
        (
            v.first().copied().unwrap_or(d.0),
            v.get(1).copied().unwrap_or(d.1),
        )
    };
    let color = |k: &str| {
        let v = nums(k);
        let c = |i: usize| v.get(i).copied().unwrap_or(1.0) as f32;
        [c(0), c(1), c(2), c(3)]
    };
    let dir = &path[..path.rfind('/').map_or(0, |i| i)];
    let mat = join(dir, g.get_ci("MatName")?.trim());
    let batch = loader.append_quad(mesh, &mat)?;
    match class.as_str() {
        "TParticlesSystemParams" => Some(EffectSpec {
            sprite: false,
            batch,
            max_particles: n("nParticles", 10.0).max(1.0) as usize,
            finish: n("FinishTime", 1.0),
            live: n("LiveTime", 1.0),
            emit_frq: n("EmitFrq", 10.0),
            velocity: pair("EmitVelocity", (1.0, 0.0)),
            theta: pair("EmitTheta", (0.0, 0.0)),
            gas_resist: n("GasResist", 0.0),
            vert_accel: n("VertAccel", 0.0),
            size: pair("Size", (1.0, 1.0)),
            color: (color("Color0"), color("Color1")),
            max_r: n("MaxR", 0.0),
            radius_period: 0.0,
            angle_speed: 0.0,
        }),
        "TAnimatedSpriteParams" => Some(EffectSpec {
            sprite: true,
            batch,
            max_particles: 1,
            finish: n("RadiusPeriod", 1.0),
            live: n("ColorPeriod", 1.0).max(n("RadiusPeriod", 1.0)),
            emit_frq: 0.0,
            velocity: (0.0, 0.0),
            theta: (0.0, 0.0),
            gas_resist: 0.0,
            vert_accel: 0.0,
            size: (n("Radius0", 1.0), n("Radius1", 1.0)),
            color: (color("Color0"), color("Color1")),
            max_r: 0.0,
            radius_period: n("RadiusPeriod", 1.0),
            angle_speed: n("AngleSpeed", 0.0),
        }),
        _ => None,
    }
}

struct Particle {
    pos: DVec3,
    vel: DVec3,
    age: f64,
}

struct Emitter {
    spec: Arc<EffectSpec>,
    /// Set for emitters that follow something (`attach`).
    id: u64,
    /// Where the emitter was at the last update (particles are spread along the way).
    prev: DVec3,
    pos: DVec3,
    axis: DVec3,
    scale: f64,
    age: f64,
    /// Emission stops after this many seconds.
    life: f64,
    debt: f64,
    particles: Vec<Particle>,
}

/// All live effects and the billboard materials they use.
pub struct Effects {
    specs: HashMap<String, Option<Arc<EffectSpec>>>,
    pub data: Option<MeshData>,
    emitters: Vec<Emitter>,
    rng: Rng,
    next_id: u64,
}

impl Effects {
    /// Loads the given effect files up front (their materials go into one mesh).
    pub fn load<'a>(vfs: &Vfs, names: impl Iterator<Item = &'a str>) -> Self {
        let mut loader = Loader::new(vfs);
        let mut mesh = MeshData::default();
        let mut specs = HashMap::new();
        for name in names {
            let key = name.to_ascii_lowercase();
            if specs.contains_key(&key) {
                continue;
            }
            let spec = vfs
                .read(name)
                .ok()
                .and_then(|d| parse(&d, name, &mut loader, &mut mesh))
                .map(Arc::new);
            specs.insert(key, spec);
        }
        let loaded = specs.values().filter(|s| s.is_some()).count();
        eprintln!("effects: {loaded} of {} loaded", specs.len());
        if std::env::var_os("IL2_TRACE").is_some() {
            for (k, v) in &specs {
                if v.is_none() {
                    eprintln!("  effect not loaded: {k}");
                }
            }
            for w in &loader.warnings {
                eprintln!("  {w}");
            }
        }
        mesh.textures = loader.take_images();
        mesh.node_count = mesh.node_count.max(1);
        Effects {
            specs,
            data: Some(mesh),
            emitters: Vec::new(),
            rng: Rng::new(0xE77EC7),
            next_id: 1,
        }
    }

    /// Starts an effect at `pos` pointing along `axis`; `life` < 0 uses the file's own duration.
    pub fn spawn(&mut self, name: &str, pos: DVec3, axis: DVec3, scale: f64, life: f64) {
        let Some(Some(spec)) = self.specs.get(&name.to_ascii_lowercase()) else {
            return;
        };
        let life = if life > 0.0 {
            life.min(spec.finish.max(life))
        } else {
            spec.finish
        };
        let mut e = Emitter {
            spec: spec.clone(),
            id: 0,
            prev: pos,
            pos,
            axis: axis.normalize_or(DVec3::Z),
            scale,
            age: 0.0,
            life,
            debt: 0.0,
            particles: Vec::new(),
        };
        if spec.sprite {
            e.particles.push(Particle {
                pos,
                vel: DVec3::ZERO,
                age: 0.0,
            });
        }
        self.emitters.push(e);
    }

    /// Starts an effect that keeps emitting until `detach`, moved along with `follow`.
    pub fn attach(&mut self, name: &str, pos: DVec3, axis: DVec3, scale: f64) -> Option<u64> {
        let n = self.emitters.len();
        self.spawn(name, pos, axis, scale, -1.0);
        let e = self.emitters.get_mut(n)?;
        e.life = f64::INFINITY;
        e.id = self.next_id;
        self.next_id += 1;
        Some(e.id)
    }

    /// Moves an attached emitter.
    pub fn follow(&mut self, id: u64, pos: DVec3, axis: DVec3) {
        if let Some(e) = self.emitters.iter_mut().find(|e| e.id == id) {
            e.pos = pos;
            e.axis = axis.normalize_or(DVec3::X);
        }
    }

    /// Stops an attached emitter (its particles live out their time).
    pub fn detach(&mut self, id: u64) {
        if let Some(e) = self.emitters.iter_mut().find(|e| e.id == id) {
            e.life = e.age;
            e.id = 0;
        }
    }

    pub fn update(&mut self, dt: f64) {
        let rng = &mut self.rng;
        for e in &mut self.emitters {
            e.age += dt;
            let s = e.spec.clone();
            if !s.sprite && e.age <= e.life {
                e.debt += s.emit_frq * dt;
                let total = e.debt.floor().max(1.0);
                let mut k = 0.0;
                while e.debt >= 1.0 && e.particles.len() < s.max_particles {
                    e.debt -= 1.0;
                    k += 1.0;
                    let origin = e.prev.lerp(e.pos, k / total);
                    // Inside a cone of half-angle theta around the axis.
                    let theta = (s.theta.0 + (s.theta.1 - s.theta.0) * rng.uniform())
                        .to_radians()
                        .abs();
                    let cos_t = 1.0 - rng.uniform() * (1.0 - theta.min(std::f64::consts::PI).cos());
                    let sin_t = (1.0 - cos_t * cos_t).max(0.0).sqrt();
                    let phi = rng.uniform() * std::f64::consts::TAU;
                    let local = DVec3::new(cos_t, sin_t * phi.cos(), sin_t * phi.sin());
                    let dir = DQuat::from_rotation_arc(DVec3::X, e.axis) * local;
                    let speed =
                        (s.velocity.0 + s.velocity.1 * (rng.uniform() * 2.0 - 1.0)) * e.scale;
                    let offset = dir * (s.max_r * rng.uniform() * e.scale);
                    e.particles.push(Particle {
                        pos: origin + offset,
                        vel: dir * speed,
                        age: 0.0,
                    });
                }
                if e.particles.len() >= s.max_particles {
                    e.debt = e.debt.min(1.0);
                }
            }
            e.prev = e.pos;
            e.particles.retain_mut(|p| {
                p.age += dt;
                // Quadratic air resistance, dv/dt = −k·v².
                let v = p.vel.length();
                p.vel /= 1.0 + s.gas_resist * v * dt;
                p.vel.z += s.vert_accel * dt;
                p.pos += p.vel * dt;
                p.age < if s.sprite { e.life.max(s.live) } else { s.live }
            });
        }
        self.emitters
            .retain(|e| !e.particles.is_empty() || (!e.spec.sprite && e.age <= e.life));
    }

    pub fn count(&self) -> usize {
        self.emitters.iter().map(|e| e.particles.len()).sum()
    }

    /// Camera-facing billboards of all particles.
    pub fn gather<'a>(
        &self,
        mesh: &'a GpuMesh,
        eye: DVec3,
        out: &mut Vec<InstanceRaw>,
    ) -> Vec<InstancedDraw<'a>> {
        let mut by_batch: HashMap<usize, Vec<InstanceRaw>> = HashMap::new();
        for e in &self.emitters {
            let s = &e.spec;
            for p in &e.particles {
                let t = if s.sprite {
                    (p.age / s.radius_period.max(1e-3)).min(1.0)
                } else {
                    (p.age / s.live.max(1e-3)).min(1.0)
                };
                let size = (s.size.0 + (s.size.1 - s.size.0) * t) * e.scale;
                let c = |i: usize| s.color.0[i] + (s.color.1[i] - s.color.0[i]) * t as f32;
                let rel = (p.pos - eye).as_vec3();
                let to_cam = (-rel).normalize_or(Vec3::Z);
                let spin = (s.angle_speed * p.age).to_radians() as f32;
                let rot = glam::Quat::from_rotation_arc(Vec3::X, to_cam)
                    * glam::Quat::from_rotation_x(spin);
                let m = Mat4::from_scale_rotation_translation(Vec3::splat(size as f32), rot, rel);
                by_batch
                    .entry(s.batch)
                    .or_default()
                    .push(InstanceRaw::colored(m, [c(0), c(1), c(2), c(3)]));
            }
        }
        let mut draws = Vec::new();
        for (batch, inst) in by_batch {
            let start = out.len() as u32;
            out.extend(inst);
            draws.push(InstancedDraw {
                mesh,
                batches: batch..batch + 1,
                level: 0,
                instances: start..out.len() as u32,
            });
        }
        draws
    }
}

/// Every effect file the explosion rules below can start.
pub fn known_effects() -> Vec<String> {
    let mut v: Vec<String> = crate::smoke::effect_files();
    for (dir, files) in [
        (
            "effects/Explodes/Bullet/Land",
            &[
                "BaseSpray",
                "Fontain",
                "Spray",
                "Flash",
                "Exp_BaseSpray",
                "Exp_Flash",
                "Exp_Spray",
            ][..],
        ),
        (
            "effects/Explodes/Bullet/Water",
            &[
                "BaseSpray",
                "Fontain",
                "Spray",
                "Exp_BaseSpray",
                "Exp_Spray",
            ][..],
        ),
        ("effects/Explodes/Bullet/Object", &["Spray"][..]),
        (
            "effects/Explodes/Cannon/Land",
            &[
                "BaseSpray",
                "Fontain",
                "Spray",
                "Flash",
                "20mm_Exp_BaseSpray",
                "20mm_Exp_Flame",
                "20mm_Exp_Flash",
                "20mm_Exp_Fontain",
                "20mm_Exp_Spray",
            ][..],
        ),
        (
            "effects/Explodes/Cannon/Water",
            &[
                "BaseSpray",
                "Fontain",
                "Spray",
                "20mm_Exp_BaseSpray",
                "20mm_Exp_Fontain",
                "20mm_Exp_Spray",
            ][..],
        ),
        ("effects/Explodes/Cannon/Object", &["Sparks"][..]),
        (
            "effects/Explodes/Explode10Kg/Land",
            &["BaseSpray", "Flame", "Flash", "Dirt", "Spray"][..],
        ),
        (
            "effects/Explodes/Explode10Kg/Water",
            &["BaseSpray", "Fontain", "FontainSoot", "Spray"][..],
        ),
        (
            "effects/Explodes/Explode10Kg/Object",
            &["Sparks", "Flash", "Spray"][..],
        ),
        (
            "effects/Explodes/Objects/Bridges",
            &["SmokeBoiling", "SmokeBoiling2"][..],
        ),
        (
            "effects/Smokes",
            &["SmokeAirSplat", "SmokeAirTouch", "SmokeAirTouchW"][..],
        ),
    ] {
        for f in files {
            v.push(format!("{dir}/{f}.eff"));
        }
    }
    for b in ["Bomb250", "Bomb1000", "RS82"] {
        for land in ["Land", "Land_2", "Land_dig_in", "Land_dig_in_2"] {
            for f in ["Fontain", "Peaces", "Burn", "Burn_Fog", "Shock", "Base"] {
                v.push(format!("effects/Explodes/{b}/{land}/{f}.eff"));
            }
        }
    }
    v
}

/// What hit the ground or an object, for choosing effects.
pub enum Impact {
    /// A non-explosive round of this mass (kg).
    Shot { mass: f64 },
    /// An explosion of this many kg TNT.
    Blast { power: f64 },
}

/// Starts the runtime's effects for an impact on land, water or an object.
pub fn impact(fx: &mut Effects, at: DVec3, what: Impact, water: bool, object: bool, rng: &mut Rng) {
    let up = DVec3::Z;
    let run = |fx: &mut Effects, dir: &str, files: &[&str], life: f64| {
        for f in files {
            fx.spawn(&format!("{dir}/{f}.eff"), at, up, 1.0, life);
        }
    };
    match what {
        Impact::Shot { mass } => {
            let surface = if object {
                "Object"
            } else if water {
                "Water"
            } else {
                "Land"
            };
            if mass < 0.05 {
                let files: &[&str] = match surface {
                    "Object" => &["Spray"],
                    "Water" => &["BaseSpray", "Fontain", "Spray"],
                    _ => &["BaseSpray", "Fontain", "Spray"],
                };
                run(
                    fx,
                    &format!("effects/Explodes/Bullet/{surface}"),
                    files,
                    0.5,
                );
                if surface == "Land" && rng.uniform() < 0.2 {
                    run(fx, "effects/Explodes/Bullet/Land", &["Flash"], 0.5);
                }
            } else if mass < 0.701 {
                let files: &[&str] = if object {
                    &["Sparks"]
                } else {
                    &["BaseSpray", "Fontain", "Spray"]
                };
                run(
                    fx,
                    &format!("effects/Explodes/Cannon/{surface}"),
                    files,
                    4.0,
                );
                if surface == "Land" && rng.uniform() < 0.3 {
                    run(fx, "effects/Explodes/Cannon/Land", &["Flash"], 4.0);
                }
            } else {
                blast(fx, at, 10.0, water, rng);
            }
        }
        Impact::Blast { power } => blast(fx, at, power, water, rng),
    }
}

fn blast(fx: &mut Effects, at: DVec3, power: f64, water: bool, rng: &mut Rng) {
    let up = DVec3::Z;
    let run = |fx: &mut Effects, dir: &str, files: &[&str], life: f64| {
        for f in files {
            fx.spawn(&format!("{dir}/{f}.eff"), at, up, 1.0, life);
        }
    };
    let surface = if water { "Water" } else { "Land" };
    if power < 0.0019 {
        if rng.uniform() < 0.3 {
            run(
                fx,
                &format!("effects/Explodes/Bullet/{surface}"),
                &["Exp_BaseSpray", "Exp_Spray"],
                4.0,
            );
        }
    } else if power < 0.05 {
        if rng.uniform() < 0.4 {
            let files: &[&str] = if water {
                &["20mm_Exp_BaseSpray", "20mm_Exp_Fontain", "20mm_Exp_Spray"]
            } else {
                &[
                    "20mm_Exp_BaseSpray",
                    "20mm_Exp_Flame",
                    "20mm_Exp_Flash",
                    "20mm_Exp_Fontain",
                    "20mm_Exp_Spray",
                ]
            };
            run(
                fx,
                &format!("effects/Explodes/Cannon/{surface}"),
                files,
                4.0,
            );
        }
    } else if power < 15.0 {
        let files: &[&str] = if water {
            &["BaseSpray", "Fontain", "FontainSoot", "Spray"]
        } else {
            &["BaseSpray", "Flame", "Flash", "Dirt", "Spray"]
        };
        run(
            fx,
            &format!("effects/Explodes/Explode10Kg/{surface}"),
            files,
            4.0,
        );
    } else {
        // Bomb and rocket fountains: several fountains thrown at random headings around the vertical.
        let (name, count) = if power >= 300.0 {
            ("Bomb1000", 3 + (rng.uniform() * 3.0) as usize)
        } else if power >= 50.0 {
            ("Bomb250", 4 + (rng.uniform() * 2.0) as usize)
        } else {
            ("RS82", 2 + (rng.uniform() * 2.0) as usize)
        };
        let land = if rng.uniform() < 0.5 {
            "Land_dig_in_2"
        } else {
            "Land_dig_in"
        };
        let dir = format!("effects/Explodes/{name}/{land}");
        fx.spawn(&format!("{dir}/Peaces.eff"), at, up, 1.0, 3.5);
        for _ in 0..count {
            let yaw = rng.uniform() * std::f64::consts::TAU;
            let tilt = (rng.uniform() * 2.0 - 1.0) * 30f64.to_radians();
            let axis = DVec3::new(tilt.sin() * yaw.cos(), tilt.sin() * yaw.sin(), tilt.cos());
            fx.spawn(&format!("{dir}/Fontain.eff"), at, axis, 1.0, 10.0);
            for f in ["Burn_Fog", "Shock", "Base"] {
                fx.spawn(&format!("{dir}/{f}.eff"), at, axis, 1.0, -1.0);
            }
        }
    }
}

/// A bridge part collapsing between `a` and `b` (`width` m wide): smoke columns at random points
/// along it, at least two and two per `width` of length.
pub fn bridge_collapse(fx: &mut Effects, a: DVec3, b: DVec3, width: f64, rng: &mut Rng) {
    let n = ((2.0 * a.distance(b) / width.max(0.1)) as usize).max(2);
    for _ in 0..n {
        let at = a.lerp(b, rng.uniform());
        let f = if rng.uniform() < 0.5 {
            "SmokeBoiling"
        } else {
            "SmokeBoiling2"
        };
        fx.spawn(
            &format!("effects/Explodes/Objects/Bridges/{f}.eff"),
            at,
            DVec3::Z,
            1.0,
            3.0,
        );
    }
}

/// Spray or dust where an aircraft skims the surface faster than 60 km/h: one puff per call (the
/// runtime keeps one going per touching point).
pub fn surface_touch(fx: &mut Effects, at: DVec3, water: bool, winter: bool) {
    let f = if water {
        "SmokeAirSplat"
    } else if winter {
        "SmokeAirTouchW"
    } else {
        "SmokeAirTouch"
    };
    fx.spawn(&format!("effects/Smokes/{f}.eff"), at, DVec3::Z, 1.0, 0.5);
}
