//! Aircraft guns and bullets (`docs/behavior/weapons.md`).

use glam::{DMat3, DQuat, DVec3, Mat4};
use il2_data::anim::Value;
use il2_formats::map::HeightField;
use std::collections::HashMap;
use std::sync::Arc;

/// All of a round's data; not every field is used yet.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct BulletType {
    pub mass: f64,
    /// `kalibr` as stored (an area-like figure used only for drag).
    pub kalibr: f64,
    pub speed: f64,
    /// Explosive charge (kg TNT) and its kind and radius.
    pub power: f64,
    pub power_type: i32,
    pub power_radius: f64,
    pub time_life: f64,
    /// Tracer colour as ARGB.
    pub trace_color: u32,
    pub trace_mesh: Option<String>,
    /// Drag factor: `dv = kv·|v|·v·dt`.
    pub kv: f64,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct GunSpec {
    pub name: String,
    /// Seconds between rounds.
    pub shot_step: f64,
    /// Dispersion, degrees.
    pub max_delta_angle: f64,
    pub cannon: bool,
    pub trace_freq: u32,
    pub bullets: Vec<BulletType>,
    /// Sound preset while firing (`weapon.mgun_13_800`).
    pub sound: Option<String>,
}

impl GunSpec {
    pub fn from_value(name: &str, v: &Value) -> Option<Self> {
        let bullets: Vec<BulletType> = v
            .list("bullet")
            .iter()
            .map(|b| {
                let mass = b.num("massa").unwrap_or(0.01).max(1e-6);
                let kalibr = b.num("kalibr").unwrap_or(0.0);
                BulletType {
                    mass,
                    kalibr,
                    speed: b.num("speed").unwrap_or(800.0),
                    power: b.num("power").unwrap_or(0.0),
                    power_type: b.num("powerType").unwrap_or(0.0) as i32,
                    power_radius: b.num("powerRadius").unwrap_or(0.0),
                    time_life: b.num("timeLife").unwrap_or(2.0),
                    trace_color: b.num("traceColor").unwrap_or(0.0) as i64 as u32,
                    trace_mesh: b.str("traceMesh").map(str::to_owned),
                    kv: -1.346_275_1 * kalibr * kalibr / (8.0 * mass),
                }
            })
            .collect();
        if bullets.is_empty() {
            return None;
        }
        let freq = v.num("shotFreq").unwrap_or(10.0).max(0.001);
        Some(GunSpec {
            name: name.to_owned(),
            shot_step: 1.0 / freq,
            max_delta_angle: v.num("maxDeltaAngle").unwrap_or(0.0),
            cannon: v.num("bCannon").unwrap_or(0.0) != 0.0,
            trace_freq: (v.num("traceFreq").unwrap_or(1.0) as u32).max(1),
            bullets,
            sound: v.str("sound").map(str::to_owned),
        })
    }
}

/// A gun mounted on the aircraft.
#[derive(Clone, Debug)]
pub struct Gun {
    pub spec: Arc<GunSpec>,
    /// The aircraft hook the gun sits on (`_MGUN01`, …).
    pub hook: Option<String>,
    /// Fired during the last step.
    pub firing: bool,
    /// Hook frame in body coordinates (fires along its +x).
    pub frame: Mat4,
    pub trigger: i32,
    pub ammo: i32,
    next_shot: f64,
    index: usize,
    count: u32,
}

#[derive(Clone, Debug)]
pub struct Bullet {
    pub pos: DVec3,
    pub prev: DVec3,
    pub vel: DVec3,
    pub life: f64,
    pub spec: Arc<GunSpec>,
    pub kind: usize,
    pub tracer: bool,
    /// Index of the firing aircraft (to skip self-hits).
    pub owner: usize,
}

impl Bullet {
    pub fn ty(&self) -> &BulletType {
        &self.spec.bullets[self.kind]
    }

    /// Kinetic energy, J.
    pub fn energy(&self) -> f64 {
        0.5 * self.ty().mass * self.vel.length_squared()
    }
}

/// Gun data cache: one evaluation per weapon class.
#[derive(Default)]
pub struct Armoury {
    specs: HashMap<String, Option<Arc<GunSpec>>>,
}

impl Armoury {
    pub fn gun(&mut self, classes: &il2_data::Classes, name: &str) -> Option<Arc<GunSpec>> {
        self.specs
            .entry(name.to_owned())
            .or_insert_with(|| match classes.gun_properties(name) {
                Ok(v) => GunSpec::from_value(name, &v).map(Arc::new),
                Err(e) => {
                    if std::env::var_os("IL2_TRACE").is_some() {
                        eprintln!("weapon {name}: {e}");
                    }
                    None
                }
            })
            .clone()
    }
}

/// Guns of one aircraft with a loadout applied.
pub fn mount(
    armoury: &mut Armoury,
    classes: &il2_data::Classes,
    weapons: &il2_data::AircraftWeapons,
    loadout: &str,
    hooks: &[(String, Mat4)],
) -> Vec<Gun> {
    let slots = weapons
        .loadouts
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(loadout))
        .or_else(|| weapons.loadouts.first())
        .map(|(_, s)| s.clone())
        .unwrap_or_default();
    let mut out = Vec::new();
    for (i, slot) in slots.iter().enumerate() {
        let Some((gun, count)) = slot else { continue };
        // Guns only for now; bombs, rockets and pylons are other weapon kinds.
        if !gun.starts_with("MGun") && !gun.starts_with("Cannon") {
            continue;
        }
        let Some(spec) = armoury.gun(classes, gun) else {
            continue;
        };
        let hook = weapons.hooks.get(i).cloned().unwrap_or_default();
        let Some((_, frame)) = hooks.iter().find(|(n, _)| n.eq_ignore_ascii_case(&hook)) else {
            continue;
        };
        out.push(Gun {
            spec,
            firing: false,
            frame: *frame,
            trigger: weapons.triggers.get(i).copied().unwrap_or(0),
            ammo: *count,
            next_shot: 0.0,
            hook: Some(hook.clone()),
            index: 0,
            count: 0,
        });
    }
    out
}

/// Gaussian random numbers (Box–Muller) from a small xorshift generator.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }
    pub fn uniform(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn gaussian(&mut self) -> f64 {
        let (u, v) = (self.uniform().max(1e-12), self.uniform());
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
    }
}

/// Fires the guns whose trigger groups are pressed; returns the recoil impulse (body frame, N·s).
#[allow(clippy::too_many_arguments)]
pub fn fire(
    guns: &mut [Gun],
    triggers: &[bool; 4],
    pos: DVec3,
    ori: DQuat,
    vel: DVec3,
    dt: f64,
    rng: &mut Rng,
    owner: usize,
    out: &mut Vec<Bullet>,
) -> DVec3 {
    let mut recoil = DVec3::ZERO;
    for g in guns.iter_mut() {
        let pressed = triggers.get(g.trigger as usize).copied().unwrap_or(false);
        g.next_shot = (g.next_shot - dt).max(if pressed { -dt } else { 0.0 });
        g.firing = pressed && g.ammo > 0;
        if !pressed {
            continue;
        }
        while g.next_shot <= 0.0 && g.ammo > 0 {
            // Bullet types cycle; the counter is advanced before use.
            g.index = (g.index + 1) % g.spec.bullets.len();
            let b = &g.spec.bullets[g.index];
            let d = g.spec.max_delta_angle;
            let yaw = (rng.gaussian() * d * 0.37)
                .clamp(-3.0 * d, 3.0 * d)
                .to_radians();
            let pitch = (rng.gaussian() * d * 0.173)
                .clamp(-2.0 * d, 2.0 * d)
                .to_radians();
            let axes = DMat3::from_mat4(g.frame.as_dmat4());
            let dir_body =
                axes * (DQuat::from_rotation_z(yaw) * DQuat::from_rotation_y(-pitch) * DVec3::X);
            let dir = (ori * dir_body).normalize();
            let muzzle = pos + ori * g.frame.w_axis.truncate().as_dvec3();
            // A round fired partway through the step has already flown the rest of it.
            let late = (-g.next_shot).clamp(0.0, dt);
            let v = dir * b.speed + vel;
            let start = muzzle + v * late;
            out.push(Bullet {
                pos: start,
                prev: muzzle,
                vel: v,
                life: b.time_life - late,
                spec: g.spec.clone(),
                kind: g.index,
                tracer: g.count.is_multiple_of(g.spec.trace_freq),
                owner,
            });
            recoil -= dir_body * (b.mass * b.speed);
            g.count += 1;
            g.ammo -= 1;
            g.next_shot += g.spec.shot_step;
        }
    }
    recoil
}

/// What a bullet hit this step.
pub enum Hit {
    Ground(DVec3),
    /// A map object (index) at a point; whether it was destroyed.
    Object(usize, DVec3, bool),
}

impl BulletType {
    /// The runtime's shot type for a hit: 1 shaped charge, 0 high explosive, 3 with filler, 2 plain.
    pub fn shot_type(&self) -> i32 {
        if self.power_radius > 0.0 {
            0
        } else if self.power > 0.0 {
            3
        } else {
            2
        }
    }
}

/// Moves bullets one step (drag and gravity), resolves hits on map objects and the ground, and
/// removes spent rounds.
pub fn advance(
    bullets: &mut Vec<Bullet>,
    dt: f64,
    field: &HeightField,
    objects: &mut crate::objects::MapObjects,
    rng: &mut Rng,
    hits: &mut Vec<(Bullet, Hit)>,
) {
    let mut i = 0;
    while i < bullets.len() {
        let b = &mut bullets[i];
        b.prev = b.pos;
        let kv = b.ty().kv;
        let speed = b.vel.length();
        b.vel += b.vel * (kv * dt * speed);
        b.vel.z -= 9.8 * dt;
        b.pos += b.vel * dt;
        b.life -= dt;
        if let Some((obj, at)) = objects.hit(b.prev, b.pos) {
            let at = DVec3::new(at.x, at.y, at.z.max(field.height_at(at.x, at.y)));
            let ty = b.ty().clone();
            let speed = b.vel.length();
            let mut killed = objects.shot(
                obj,
                0.5 * ty.mass * speed * speed,
                ty.shot_type(),
                speed,
                rng,
            );
            if ty.power > 0.0 && ty.power_radius > 0.0 {
                killed |= objects.explode(at, ty.power + 0.03 * ty.mass, ty.power_radius, rng) > 0;
            }
            let b = bullets.swap_remove(i);
            hits.push((b, Hit::Object(obj, at, killed)));
            continue;
        }
        let ground = field.height_at(b.pos.x, b.pos.y);
        if b.pos.z <= ground {
            // Point of impact along the segment.
            let g0 = field.height_at(b.prev.x, b.prev.y);
            let (h0, h1) = (b.prev.z - g0, b.pos.z - ground);
            let t = if (h0 - h1).abs() > 1e-6 {
                (h0 / (h0 - h1)).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let at = b.prev.lerp(b.pos, t);
            let ty = b.ty().clone();
            // Tracer rounds without a blast may ricochet off a glancing hit.
            if ty.trace_mesh.is_some() && ty.power_radius == 0.0 {
                let n = ground_normal(field, at.x, at.y);
                let speed = b.vel.length();
                let dir = b.vel / speed.max(1e-9);
                let c = n.dot(dir);
                if c >= -0.25 && rng.uniform() - 0.666 >= -c {
                    let mut v = b.vel - n * (1.1 * n.dot(b.vel));
                    let side = dir.cross(n);
                    let r1 = rng.uniform() * 2.4 - 1.2;
                    v += side * (r1 * r1 * r1 * speed);
                    let r2 = rng.uniform() * 1.2;
                    v += n * (r2 * r2 * r2 * speed);
                    let v = v.normalize_or(n);
                    let keep = dir.dot(v);
                    b.vel = v * (keep * keep * speed);
                    b.pos = at + n * 0.05;
                    i += 1;
                    continue;
                }
            }
            if ty.power > 0.0 && ty.power_radius > 0.0 {
                objects.explode(at, ty.power + 0.03 * ty.mass, ty.power_radius, rng);
            }
            let b = bullets.swap_remove(i);
            hits.push((b, Hit::Ground(at)));
            continue;
        }
        if b.life <= 0.0 {
            bullets.swap_remove(i);
            continue;
        }
        i += 1;
    }
}

fn ground_normal(field: &HeightField, x: f64, y: f64) -> DVec3 {
    let e = 1.0;
    DVec3::new(
        field.height_at(x - e, y) - field.height_at(x + e, y),
        field.height_at(x, y - e) - field.height_at(x, y + e),
        2.0 * e,
    )
    .normalize()
}

/// A bomb or rocket type (`docs/behavior/weapons.md`, "Bombs and rockets").
#[derive(Clone, Debug)]
pub struct OrdnanceSpec {
    pub name: String,
    pub rocket: bool,
    pub mesh: Option<String>,
    pub mass: f64,
    pub mass_end: f64,
    pub kalibr: f64,
    /// kg TNT and blast radius (m).
    pub power: f64,
    pub radius: f64,
    /// Rocket thrust (N), burn and life time (s).
    pub force: f64,
    pub time_fire: f64,
    pub time_life: f64,
    /// Seconds between releases from one slot.
    pub shot_step: f64,
}

/// A bomb rack or rocket rail with its remaining rounds.
#[derive(Clone, Debug)]
pub struct Store {
    pub spec: Arc<OrdnanceSpec>,
    pub frame: Mat4,
    pub trigger: i32,
    pub count: i32,
    next: f64,
}

#[derive(Clone, Debug)]
pub struct Projectile {
    pub spec: Arc<OrdnanceSpec>,
    pub pos: DVec3,
    pub prev: DVec3,
    pub vel: DVec3,
    /// Nose direction.
    pub dir: DVec3,
    rot_axis: DVec3,
    mass: f64,
    dm: f64,
    t: f64,
}

fn num(p: &HashMap<String, Value>, k: &str, d: f64) -> f64 {
    match p.get(k) {
        Some(Value::Num(n)) => *n,
        _ => d,
    }
}

/// Data of a bomb or rocket weapon class (`BombGunSC250`, `RocketGunRS82`), with its round class.
pub fn ordnance(classes: &il2_data::Classes, gun: &str) -> Option<OrdnanceSpec> {
    let rocket = gun.starts_with("RocketGun");
    if !rocket && !gun.starts_with("BombGun") {
        return None;
    }
    let gp = classes.class_properties(&format!("com.maddox.il2.objects.weapons.{gun}"));
    let Some(Value::Str(round)) = gp.get("bulletClass") else {
        return None;
    };
    let p = classes.class_properties(round);
    let mass = num(&p, "massa", 6.8);
    Some(OrdnanceSpec {
        name: gun.to_owned(),
        rocket,
        mesh: match p.get("mesh") {
            Some(Value::Str(m)) => Some(m.clone()),
            _ => None,
        },
        mass,
        mass_end: num(&p, "massaEnd", 2.52),
        kalibr: num(&p, "kalibr", 0.082),
        power: num(&p, "power", if rocket { 1000.0 } else { 0.0 }),
        radius: num(&p, "radius", if rocket { 0.0 } else { 7.07 * mass.sqrt() }),
        force: num(&p, "force", 500.0),
        time_fire: num(&p, "timeFire", 4.0),
        time_life: num(&p, "timeLife", if rocket { 45.0 } else { 120.0 }),
        shot_step: 1.0 / num(&gp, "shotFreq", 1.0).max(0.001),
    })
}

/// Bomb racks and rocket rails of a loadout.
pub fn mount_stores(
    classes: &il2_data::Classes,
    weapons: &il2_data::AircraftWeapons,
    loadout: &str,
    hooks: &[(String, Mat4)],
) -> Vec<Store> {
    let slots = weapons
        .loadouts
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(loadout))
        .or_else(|| weapons.loadouts.first())
        .map(|(_, s)| s.clone())
        .unwrap_or_default();
    let mut cache: HashMap<String, Option<Arc<OrdnanceSpec>>> = HashMap::new();
    let mut out = Vec::new();
    for (i, slot) in slots.iter().enumerate() {
        let Some((gun, count)) = slot else { continue };
        let Some(spec) = cache
            .entry(gun.clone())
            .or_insert_with(|| ordnance(classes, gun).map(Arc::new))
            .clone()
        else {
            continue;
        };
        let hook = weapons.hooks.get(i).cloned().unwrap_or_default();
        let frame = hooks
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(&hook))
            .map_or(Mat4::IDENTITY, |(_, f)| *f);
        out.push(Store {
            spec,
            frame,
            trigger: weapons.triggers.get(i).copied().unwrap_or(3),
            count: (*count).max(1),
            next: 0.0,
        });
    }
    out
}

/// Releases bombs (trigger 3) and fires rockets (trigger 2).
#[allow(clippy::too_many_arguments)]
pub fn release(
    stores: &mut [Store],
    triggers: &[bool; 4],
    pos: DVec3,
    ori: DQuat,
    vel: DVec3,
    dt: f64,
    rng: &mut Rng,
    out: &mut Vec<Projectile>,
) {
    for s in stores.iter_mut() {
        s.next = (s.next - dt).max(0.0);
        let pressed = triggers.get(s.trigger as usize).copied().unwrap_or(false);
        if !pressed || s.count <= 0 || s.next > 0.0 {
            continue;
        }
        s.count -= 1;
        s.next = s.spec.shot_step;
        let sp = &s.spec;
        let at = pos + ori * s.frame.w_axis.truncate().as_dvec3();
        let fwd = ori * DVec3::X;
        let sign = |rng: &mut Rng, a: f64, b: f64| {
            let v = a + (b - a) * rng.uniform();
            if rng.uniform() > 0.5 {
                v
            } else {
                -v
            }
        };
        if sp.rocket {
            // Launch along the aircraft axis at 200 m/s more, a third of the vertical speed kept.
            let mut v_body = ori.inverse() * vel;
            v_body.z /= 3.0;
            v_body.x += 200.0;
            let dm = if sp.time_fire > 0.0 {
                (sp.mass - sp.mass_end) / (sp.time_fire / 0.03)
            } else {
                0.0
            };
            out.push(Projectile {
                spec: sp.clone(),
                pos: at,
                prev: at,
                vel: ori * v_body,
                dir: fwd,
                rot_axis: DVec3::ZERO,
                mass: sp.mass,
                dm,
                t: 0.0,
            });
        } else {
            let k = 0.04;
            let dir = (fwd + DVec3::new(sign(rng, 0.0, k), sign(rng, 0.0, k), sign(rng, 0.0, k)))
                .normalize();
            let axis = DVec3::new(
                sign(rng, 0.1, 1.0),
                sign(rng, 0.1, 1.0),
                sign(rng, 0.1, 1.0),
            )
            .normalize();
            let speed = vel.length();
            let mut angle = sign(rng, 2.0, 35.0).to_radians();
            if speed > 60.0 {
                let f = if speed < 350.0 {
                    (1.0 - (speed - 60.0) / 290.0) * 0.95 + 0.05
                } else {
                    0.05
                };
                angle *= f;
            }
            let mass = sp.mass * (1.0 + 0.06 * rng.uniform());
            out.push(Projectile {
                spec: sp.clone(),
                pos: at,
                prev: at,
                vel,
                dir,
                rot_axis: axis * angle,
                mass,
                dm: 0.0,
                t: 0.0,
            });
        }
    }
}

fn kd(z: f64) -> f64 {
    1.0 + z * (-9.59387e-5 + z * (3.53118e-9 + z * -5.83556e-14))
}

fn kf(v: f64) -> f64 {
    (608.5 + (-1.81327 + 0.001_651_14 * v) * v) * v
}

/// Flies bombs and rockets one step; impacts and burnt-out rockets explode against map objects.
/// Returns the explosions: point, kg TNT and objects destroyed.
pub fn fly(
    projectiles: &mut Vec<Projectile>,
    dt: f64,
    field: &HeightField,
    objects: &mut crate::objects::MapObjects,
    rng: &mut Rng,
) -> Vec<(DVec3, f64, usize)> {
    let mut booms = Vec::new();
    let mut i = 0;
    while i < projectiles.len() {
        let p = &mut projectiles[i];
        p.prev = p.pos;
        p.t += dt;
        let sp = p.spec.clone();
        let s = std::f64::consts::PI * sp.kalibr * sp.kalibr / 4.0;
        let speed = p.vel.length();
        if sp.rocket {
            let thrust = if p.t < sp.time_fire {
                p.mass -= p.dm;
                sp.force
            } else {
                0.0
            };
            let drag = if speed > 330.0 {
                kf(speed) * kd(p.pos.z) * s
            } else {
                0.2 * s * 1.225 * kd(p.pos.z) * speed * speed / 2.0
            };
            p.vel += p.dir * ((thrust - drag) / p.mass * dt);
            p.vel.z -= 9.80665 * dt;
            p.pos += p.vel * dt;
            p.dir = p.vel.normalize_or(p.dir);
        } else {
            let dir_w = if speed < 0.001 {
                DVec3::NEG_Z
            } else {
                p.vel / speed
            };
            let c = p.dir.dot(dir_w);
            let drag = if speed > 330.0 {
                kf(speed) * kd(p.pos.z) * s
            } else {
                0.06 * s * 1.225 * kd(p.pos.z) * speed * speed / 2.0
            };
            let a = drag / p.mass;
            p.vel += (p.dir * (6.0 * c) - dir_w * 7.0) * (a * dt);
            p.vel.z -= 9.80665 * dt;
            p.pos += p.vel * dt;
            // The fins turn the nose into the wind.
            if p.t > 0.35 {
                let sin = if c <= -0.996 {
                    0.08
                } else {
                    (1.0 - c * c).max(0.0).sqrt()
                };
                let force = dir_w * -(drag * 0.07 * sin);
                let arm = p.dir * -(0.2 * sp.kalibr);
                let j = p.mass * 0.1 * sp.kalibr.powi(4);
                p.rot_axis += arm.cross(force) / j.max(1e-6) * dt;
                p.rot_axis *= 0.96;
            }
            let angle = p.rot_axis.length() * dt;
            if angle > 1e-9 {
                p.dir = (DQuat::from_axis_angle(p.rot_axis.normalize(), angle) * p.dir).normalize();
            }
        }
        let ground = field.height_at(p.pos.x, p.pos.y);
        let hit_obj = objects.hit(p.prev, p.pos).map(|(_, at)| at);
        let at = if let Some(at) = hit_obj {
            // Object spheres may reach below the surface.
            Some(DVec3::new(at.x, at.y, at.z.max(ground)))
        } else if p.pos.z <= ground {
            Some(DVec3::new(p.pos.x, p.pos.y, ground))
        } else if p.t > sp.time_life {
            Some(p.pos)
        } else {
            None
        };
        if let Some(at) = at {
            if sp.power > 0.0 {
                let killed = objects.explode(at, sp.power, sp.radius.max(1.0), rng);
                booms.push((at, sp.power, killed));
            }
            projectiles.swap_remove(i);
            continue;
        }
        i += 1;
    }
    booms
}
