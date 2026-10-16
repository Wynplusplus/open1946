//! Air gunners (`docs/behavior/ai.md`, "Gunners"): turret guns (trigger groups 10 and up) fire at
//! enemy aircraft inside their arcs.

use crate::weapons::{Bullet, Gun, Rng};
use glam::{DMat3, DQuat, DVec3};

/// A turret's arc on a 10° grid: `[pitch row][yaw column]`, as `Classes::turret_limits` gives.
pub type Arc = Vec<Vec<bool>>;

fn inside(arcs: Option<&[Arc]>, turret: usize, yaw: f64, pitch: f64) -> bool {
    match arcs.and_then(|a| a.get(turret)) {
        Some(g) => {
            let r = (((pitch + 90.0) / 10.0).round() as usize).min(18);
            let c = (((yaw + 180.0) / 10.0).round() as usize).min(36);
            g[r][c]
        }
        // Without arcs: anything within 100° of the gun's rest direction.
        None => {
            yaw.abs().max(pitch.abs()) < 100.0
                && (yaw.to_radians().cos() * pitch.to_radians().cos()) > -0.17
        }
    }
}

/// Fires the turret guns of one aircraft at the nearest enemy each can reach.
#[allow(clippy::too_many_arguments)]
pub fn fire(
    guns: &mut [Gun],
    arcs: Option<&[Arc]>,
    pos: DVec3,
    ori: DQuat,
    vel: DVec3,
    enemies: &[(DVec3, DVec3)],
    skill: u8,
    dt: f64,
    rng: &mut Rng,
    owner: usize,
    bullets: &mut Vec<Bullet>,
) {
    let reach: f64 = [500.0, 600.0, 700.0, 800.0][skill.min(3) as usize];
    let scatter: f64 = [2.0, 1.4, 1.0, 0.7][skill.min(3) as usize];
    for g in guns.iter_mut() {
        if g.trigger < 10 || g.ammo <= 0 {
            g.firing = false;
            continue;
        }
        g.next_shot = (g.next_shot - dt).max(-dt);
        let turret = (g.trigger - 10) as usize;
        let axes = ori * DQuat::from_mat3(&DMat3::from_mat4(g.frame.as_dmat4()));
        let muzzle = pos + ori * g.frame.w_axis.truncate().as_dvec3();
        let speed = g.spec.bullets.first().map_or(800.0, |b| b.speed);
        let mut best: Option<(f64, DVec3)> = None;
        for &(ep, ev) in enemies {
            let d = (ep - muzzle).length();
            if d > reach {
                continue;
            }
            let tof = d / speed;
            let aim = ep + (ev - vel) * tof + DVec3::new(0.0, 0.0, 0.5 * 9.81 * tof * tof) - muzzle;
            let local = axes.inverse() * aim;
            let yaw = local.y.atan2(local.x).to_degrees();
            let pitch = (local.z / local.length().max(1e-6))
                .clamp(-1.0, 1.0)
                .asin()
                .to_degrees();
            if inside(arcs, turret, yaw, pitch) && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, aim));
            }
        }
        g.firing = best.is_some();
        let Some((_, aim)) = best else { continue };
        while g.next_shot <= 0.0 && g.ammo > 0 {
            g.index = (g.index + 1) % g.spec.bullets.len();
            let b = &g.spec.bullets[g.index];
            let s = scatter.to_radians();
            let dir = (aim.normalize()
                + DVec3::new(rng.gaussian(), rng.gaussian(), rng.gaussian()) * s)
                .normalize();
            bullets.push(Bullet {
                pos: muzzle,
                prev: muzzle,
                vel: dir * b.speed + vel,
                life: b.time_life,
                spec: g.spec.clone(),
                kind: g.index,
                tracer: g.count.is_multiple_of(g.spec.trace_freq.max(1)),
                owner,
                fuse: None,
                id: crate::weapons::bullet_id(),
            });
            g.count += 1;
            g.ammo -= 1;
            g.next_shot += g.spec.shot_step;
        }
    }
}
