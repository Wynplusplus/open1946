//! Anti-aircraft guns of a mission's `[NStationary]` artillery (`docs/behavior/ground.md`,
//! "Anti-aircraft fire"): they engage enemy aircraft in reach with lead and time-fused shells.

use crate::weapons::{self, Bullet, GunSpec, Rng};
use glam::DVec3;
use il2_formats::map::HeightField;
use il2_formats::mission::Mission;
use il2_formats::SectFile;
use il2_vfs::Vfs;
use std::sync::Arc;

pub struct Battery {
    pub pos: DVec3,
    pub army: u8,
    spec: Arc<GunSpec>,
    reach: f64,
    ceiling: f64,
    /// Seconds of continuous fire per burst (0: single shots) and the pause after it.
    chainfire: f64,
    delay: f64,
    burst_left: f64,
    cooldown: f64,
    next_shot: f64,
    count: u32,
    /// Owner id of its rounds (`hitbox::Target::id` space).
    pub id: usize,
}

impl Battery {
    pub fn spec(&self) -> Arc<GunSpec> {
        self.spec.clone()
    }
}

/// First owner id of anti-aircraft rounds.
pub const ID_BASE: usize = 2_000_000;

pub fn batteries(
    vfs: &Vfs,
    fms: &il2_data::FlightModels,
    m: &Mission,
    field: &HeightField,
) -> Vec<Battery> {
    let technics = vfs
        .read("com/maddox/il2/objects/technics.ini")
        .map(|d| SectFile::parse(&d))
        .unwrap_or_default();
    let classes = il2_data::Classes::new(vfs, &fms.keys);
    let mut armoury = weapons::Armoury::default();
    let mut out = Vec::new();
    for s in &m.stationary {
        let Some(short) = s.class.strip_prefix("vehicles.artillery.Artillery$") else {
            continue;
        };
        let Some(sect) = technics.section(short) else {
            continue;
        };
        let num = |k: &str, d: f64| {
            sect.get(k)
                .and_then(il2_formats::sectfile::atof)
                .map_or(d, f64::from)
        };
        let Some(gun) = sect.get("Gun").and_then(|g| g.split_whitespace().next()) else {
            continue;
        };
        let Some(spec) = armoury.gun(&classes, gun) else {
            continue;
        };
        // Anti-aircraft guns point up at least 45° (field guns and tanks keep to the ground).
        if num("GunMaxPitch", 0.0) < 45.0 {
            continue;
        }
        let z = field.height_at(s.x, s.y) + 1.5;
        out.push(Battery {
            pos: DVec3::new(s.x, s.y, z),
            army: s.flag.clamp(0, 255) as u8,
            spec,
            reach: num("AttackMaxDistance", 2000.0),
            ceiling: num("AttackMaxHeight", 2000.0),
            chainfire: num("ChainfireTime", 0.0),
            delay: num("DelayAfterShoot", 2.0).max(0.2),
            burst_left: 0.0,
            cooldown: 0.0,
            next_shot: 0.0,
            count: 0,
            id: ID_BASE + out.len(),
        });
    }
    eprintln!("flak: {} anti-aircraft guns", out.len());
    out
}

/// One step for every battery: pick the nearest enemy aircraft in reach and fire at its future
/// position. `aircraft` lists (position, velocity, army) of everything flying.
pub fn step(
    batteries: &mut [Battery],
    aircraft: &[(DVec3, DVec3, u8)],
    dt: f64,
    rng: &mut Rng,
    bullets: &mut Vec<Bullet>,
) {
    for b in batteries.iter_mut() {
        b.cooldown -= dt;
        b.next_shot -= dt;
        let target = aircraft
            .iter()
            .filter(|a| a.2 != b.army && a.2 != 0)
            .filter(|a| {
                (a.0 - b.pos).length() < b.reach
                    && a.0.z - b.pos.z < b.ceiling
                    && a.0.z > b.pos.z + 20.0
            })
            .min_by(|x, y| (x.0 - b.pos).length().total_cmp(&(y.0 - b.pos).length()));
        let Some(&(tp, tv, _)) = target else {
            b.burst_left = 0.0;
            continue;
        };
        if b.cooldown > 0.0 {
            continue;
        }
        if b.burst_left <= 0.0 {
            b.burst_left = if b.chainfire > 0.0 { b.chainfire } else { 1e-3 };
        }
        while b.next_shot <= 0.0 && b.burst_left > 0.0 {
            b.count += 1;
            let ty_i = (b.count as usize) % b.spec.bullets.len();
            let ty = &b.spec.bullets[ty_i];
            // Lead: two refinements of the time of flight, then scatter.
            let mut tof = (tp - b.pos).length() / ty.speed.max(1.0);
            for _ in 0..2 {
                tof = (tp + tv * tof - b.pos).length() / ty.speed.max(1.0);
            }
            let aim = tp + tv * tof + DVec3::new(0.0, 0.0, 0.5 * 9.81 * tof * tof);
            let spread = (b.spec.max_delta_angle.max(0.3) * 3.0).to_radians();
            let dir = ((aim - b.pos).normalize()
                + DVec3::new(rng.gaussian(), rng.gaussian(), rng.gaussian()) * spread)
                .normalize();
            let high_explosive = ty.power > 0.0 && ty.power_radius > 0.0;
            bullets.push(Bullet {
                pos: b.pos,
                prev: b.pos,
                vel: dir * ty.speed,
                life: ty.time_life,
                spec: b.spec.clone(),
                kind: ty_i,
                tracer: b.count.is_multiple_of(b.spec.trace_freq.max(1)),
                owner: b.id,
                // Fused to burst where the target will be, give or take 10 %.
                fuse: high_explosive.then(|| ty.time_life - tof * (0.9 + 0.2 * rng.uniform())),
            });
            b.next_shot += b.spec.shot_step.max(0.05);
            if b.chainfire > 0.0 {
                b.burst_left -= b.spec.shot_step.max(0.05);
            } else {
                b.burst_left = 0.0;
            }
        }
        if b.burst_left <= 0.0 {
            b.cooldown = b.delay;
            b.next_shot = b.next_shot.max(0.0);
        }
    }
}
