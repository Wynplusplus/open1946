//! Landing gear and airframe contact with the ground: `docs/behavior/gear.md`.

use crate::aircraft::{ContactKind, GearPoint};
use glam::{DQuat, DVec3};

/// Terrain under the aircraft.
pub trait Ground {
    fn height(&self, x: f64, y: f64) -> f64;
    /// Surface roughness for the wheels: 0 concrete, 0.8 packed ground, 3.8 grass (the default).
    fn roughness(&self, _x: f64, _y: f64) -> f64 {
        3.8
    }
    /// Open water (land-type water) at this point.
    fn is_water(&self, _x: f64, _y: f64) -> bool {
        false
    }
}

impl<F: Fn(f64, f64) -> f64> Ground for F {
    fn height(&self, x: f64, y: f64) -> f64 {
        self(x, y)
    }
}

/// Controls and state the gear needs from the aircraft.
pub struct GearInputs {
    pub pos: DVec3,
    pub ori: DQuat,
    pub vel: DVec3,
    pub w: DVec3,
    pub mass: f64,
    pub brake: f64,
    pub rudder: f64,
    /// Gear extension, 0 up .. 1 down.
    pub gear: f64,
    pub tail_locked: bool,
    pub gc_gear_shift: f64,
    pub wing_end: f64,
    /// `[Gear] SinkFactor` and `[Aircraft] Seaplane`.
    pub sink_factor: f64,
    pub seaplane: bool,
    /// Simulation time (s), for the waves.
    pub time: f64,
    pub dt: f64,
}

#[derive(Clone, Debug)]
pub struct GearState {
    roughness: f64,
    steer_angle: f64,
    rng: u64,
    pub wheels_on_ground: usize,
    pub points_on_ground: usize,
    prev: (usize, usize),
    pub changed: bool,
    /// Wheel compression of the left, right and third wheel (m), for animation.
    pub sinking: [f64; 3],
    /// Hard contacts this step: (point index, severity > 1 breaks).
    pub hits: Vec<(usize, f64)>,
    /// Points in the water this step.
    pub water_points: usize,
    /// Current buoyancy (`SinkFactor`, slowly lost by landplanes); `None` until first in water.
    sink: Option<f64>,
}

impl Default for GearState {
    fn default() -> Self {
        GearState {
            roughness: 3.8,
            steer_angle: 0.0,
            rng: 0x2545_F491_4F6C_DD1D,
            wheels_on_ground: 0,
            points_on_ground: 0,
            prev: (0, 0),
            changed: false,
            sinking: [0.0; 3],
            hits: Vec::new(),
            water_points: 0,
            sink: None,
        }
    }
}

/// The runtime's two-step limit `if x > m { m }; if x < -m { -m }` (also defined for negative `m`).
fn lim(x: f64, m: f64) -> f64 {
    let x = if x > m { m } else { x };
    if x < -m {
        -m
    } else {
        x
    }
}

impl GearState {
    fn rand(&mut self, a: f64, b: f64) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        a + (b - a) * ((self.rng >> 11) as f64 / (1u64 << 53) as f64)
    }

    /// Resets the roughness from the surface; the runtime does this every other tick.
    pub fn read_surface(&mut self, ground: &impl Ground, x: f64, y: f64) {
        self.roughness = ground.roughness(x, y);
    }

    /// Ground force and moment in the body frame.
    pub fn forces(
        &mut self,
        points: &[GearPoint],
        ground: &impl Ground,
        c: &GearInputs,
    ) -> (DVec3, DVec3) {
        let (mut f, mut m) = (DVec3::ZERO, DVec3::ZERO);
        self.wheels_on_ground = 0;
        self.points_on_ground = 0;
        self.sinking = [0.0; 3];
        self.hits.clear();
        self.water_points = 0;
        let h0 = ground.height(c.pos.x, c.pos.y);
        if c.pos.z - h0 > 50.0 {
            return (f, m);
        }
        // Ground plane at the point below the aircraft.
        let e = 1.0;
        let nw = DVec3::new(
            ground.height(c.pos.x - e, c.pos.y) - ground.height(c.pos.x + e, c.pos.y),
            ground.height(c.pos.x, c.pos.y - e) - ground.height(c.pos.x, c.pos.y + e),
            2.0 * e,
        )
        .normalize();
        let inv = c.ori.inverse();
        let normal = inv * nw;
        let pn = inv * DVec3::new(0.0, 0.0, h0 - c.pos.z);
        let d0 = -normal.dot(pn);
        if d0 > 50.0 {
            return (f, m);
        }
        let forward = DVec3::Y.cross(normal).normalize();
        let right = normal.cross(forward);
        let vrel = c.vel; // no moving decks
        let fd = (inv * -vrel).normalize_or_zero();
        let mass_coeff = (4e-4 * c.mass).min(1.0);
        let water = ground.is_water(c.pos.x, c.pos.y);
        if water {
            self.roughness = 0.5;
        }
        for (i, p) in points.iter().enumerate() {
            let is_gear = p.kind != ContactKind::Body;
            if is_gear && c.gear < 0.01 {
                continue;
            }
            let mut d = normal.dot(p.pos) + d0;
            if water {
                if d < 0.0 {
                    let vs = inv * vrel + c.w.cross(p.pos);
                    let tn = self.water_contact(i, p, d, vs, normal, forward, right, vrel, c);
                    let arm = p.pos + DVec3::new(c.gc_gear_shift, 0.0, 0.0);
                    f += tn;
                    m += arm.cross(tn);
                }
                continue;
            }
            if d >= 0.0 && (is_gear || d >= 0.1) {
                continue;
            }
            let vs = inv * vrel + c.w.cross(p.pos);
            let (fvp, rvp) = (forward.dot(vs), right.dot(vs));
            let mut tn;
            if is_gear {
                let slot = match p.kind {
                    ContactKind::Wheel if p.pos.y >= 0.0 => 0,
                    ContactKind::Wheel => 1,
                    _ => 2,
                };
                self.sinking[slot] = -d;
                let nvp = normal.dot(vs).min(0.0);
                // Surface bumps grow with speed, limited by the roughness.
                // Each contact lowers the stored roughness, which the surface resets every other tick.
                let mut bump = (0.01 * (vrel.x * vrel.x + vrel.y * vrel.y - 2.0)).clamp(0.0, 4.5);
                bump = bump.min(0.4 * (self.roughness * self.roughness).max(self.roughness));
                self.roughness = self.roughness.min(bump).max(0.2);
                d += self.rand(-2.0, 1.0) * 0.04 * bump * mass_coeff;
                let mut spring = (-9500.0 * (d - 0.1)).max(-950_000.0 * d);
                if slot == 2 {
                    spring *= if p.pos.x > 0.0 && fd.dot(normal) >= 0.0 {
                        0.45
                    } else {
                        0.6
                    };
                }
                spring -= 40_000.0 * nvp;
                tn = normal * spring;
                let n = 1e-4 * spring;
                self.wheels_on_ground += 1;
                match slot {
                    0 | 1 => {
                        // Differential braking from the rudder.
                        let side = if slot == 0 { -1.2 } else { 1.2 };
                        let mut brake = c.brake;
                        if brake > 0.1 {
                            if c.rudder > 0.1 {
                                brake += side * brake * (c.rudder - 0.1);
                            }
                            if c.rudder < -0.1 {
                                brake += side * brake * (c.rudder + 0.1);
                            }
                            brake = brake.clamp(0.0, 1.0);
                        }
                        let speed = fvp.hypot(rvp).max(0.01);
                        let (cf, cr) = ((fvp / speed).abs(), (rvp / speed).abs());
                        let mut k = 5.0;
                        if p.pos.y * rvp > 0.0 {
                            k = if p.pos.y > 0.0 {
                                k + 7.0 * rvp
                            } else {
                                k - 7.0 * rvp
                            };
                            k = k.min(20.0);
                        }
                        let mut cap = 15_000.0;
                        if speed < 3.0 {
                            let x = -(speed - 3.0) / 3.0;
                            cap += 3000.0 * x * x;
                        }
                        let fric_r = lim(-k * 100_000.0 * rvp * n, cap * n * cr);
                        let roll_cap = k * (200.0 * (1.0 - 0.04 * speed)).max(5.0) * n * cf;
                        let mut fric_f = lim(-k * 600.0 * fvp * n, roll_cap);
                        let nose = points
                            .iter()
                            .any(|q| q.kind == ContactKind::Castor && q.pos.x > 0.0);
                        let mut b = if nose { 0.06 } else { 0.03 };
                        if fvp.abs() < 1.0 {
                            b += 3.0 * (1.0 - fvp.abs());
                        }
                        b *= 0.03 * brake;
                        let max = cap * n * cf;
                        fric_f = lim(fric_f - 300_000.0 * b * fvp * n, max);
                        let (mut ff, mut fr) = (fric_f, fric_r);
                        let fric = ff.hypot(fr);
                        if fric > max && fric > 0.0 {
                            ff *= max / fric;
                            fr *= max / fric;
                        }
                        tn += forward * ff + right * fr;
                        // Hard landings and side loads break the gear.
                        if nvp < 0.0 {
                            let severity = fvp * fvp * 8e-5 + rvp * rvp * 0.0068 + nvp * nvp * 0.07;
                            if severity > 1.0 {
                                self.hits.push((i, severity));
                            }
                        }
                    }
                    _ => {
                        if c.tail_locked && self.steer_angle.abs() < 5.0 {
                            self.steer_angle = 0.0;
                            tn += forward * (-400.0 * fvp).clamp(-400.0, 400.0)
                                + right * (-10_000.0 * rvp).clamp(-40_000.0, 40_000.0);
                        } else if p.pos.x > -1.0 {
                            // Steerable nose wheel.
                            let dir = DVec3::new(1.0, -0.5 * c.rudder, 0.0);
                            let target = dir.y.atan2(dir.x).to_degrees();
                            self.steer_angle += (target - self.steer_angle) * 0.115;
                            let nr = normal.cross(dir).normalize();
                            let nf = nr.cross(normal);
                            let (fv, rv) = (nf.dot(vs), nr.dot(vs));
                            let s = fv.hypot(rv).max(0.01);
                            let (mut ff, mut fr) = (
                                (-100.0 * fv).clamp(-4000.0, 4000.0),
                                (-500.0 * rv).clamp(-4000.0, 4000.0),
                            );
                            let max = 5000.0 * (1.0 - 0.02 * s).max(0.1);
                            let fric = ff.hypot(fr);
                            if fric > max {
                                ff *= max / fric;
                                fr *= max / fric;
                            }
                            tn += forward * ff + right * fr;
                        } else {
                            // Castering tail wheel.
                            if vs.length_squared() > 0.04 {
                                let target = vs.y.atan2(vs.x).to_degrees();
                                self.steer_angle += (target - self.steer_angle) * 0.115;
                            }
                            let (mut ff, mut fr) = (-1000.0 * fvp, -1000.0 * rvp);
                            let fric = ff.hypot(fr);
                            if fric > 1500.0 {
                                ff *= 1500.0 / fric;
                                fr *= 1500.0 / fric;
                            }
                            tn += forward * ff + right * fr;
                        }
                        if nvp < 0.0 {
                            let severity = (fvp * fvp + rvp * rvp) * 1e-4 + nvp * nvp * 0.02;
                            if severity > 1.0 {
                                self.hits.push((i, severity));
                            }
                        }
                    }
                }
                tn *= mass_coeff;
            } else {
                // Airframe points scrape and skid.
                self.points_on_ground += 1;
                let mut nvp = normal.dot(vs);
                if nvp > 0.0 {
                    nvp = (nvp - 3.0).max(0.0);
                }
                let a = (d - 0.06).clamp(-2.0, 0.0);
                let b = (d + 0.04).clamp(-0.25, 0.0);
                let spring = (-120_000.0 * a).max(-360_000.0 * b) * mass_coeff;
                tn = normal * (spring + (-40_000.0 * nvp).clamp(-100_000.0, 100_000.0));
                let span = (1.0 - 0.5 * p.pos.y.abs() / c.wing_end.max(1.0)).max(0.0);
                let (mut ff, mut fr) = (-8000.0 * fvp, -50_000.0 * rvp);
                let fric = ff.hypot(fr);
                if fric > 20_000.0 * span && fric > 0.0 {
                    ff *= 20_000.0 * span / fric;
                    fr *= 20_000.0 * span / fric;
                }
                tn += forward * (ff * 0.8) + right * (fr * 0.8);
                if nvp < 0.0 {
                    let severity =
                        (fvp * fvp + rvp * rvp) * 2.366_863_9e-4 + nvp * nvp * 0.020_408_16;
                    if severity > 1.0 {
                        self.hits.push((i, severity));
                    }
                }
            }
            let arm = p.pos + DVec3::new(c.gc_gear_shift, 0.0, 0.0);
            f += tn;
            m += arm.cross(tn);
        }
        let counts = (self.wheels_on_ground, self.points_on_ground);
        self.changed = counts != self.prev;
        self.prev = counts;
        (f, m)
    }

    pub fn on_ground(&self) -> bool {
        self.wheels_on_ground + self.points_on_ground + self.water_points > 0
    }

    /// Force of the water on a point `d` below its surface (`docs/behavior/gear.md`, "Water"): buoyancy
    /// growing with depth and forward speed, heave damping and drag; hard enough impacts are hits.
    #[allow(clippy::too_many_arguments)]
    fn water_contact(
        &mut self,
        i: usize,
        p: &GearPoint,
        mut d: f64,
        vs: DVec3,
        normal: DVec3,
        forward: DVec3,
        right: DVec3,
        vrel: DVec3,
        c: &GearInputs,
    ) -> DVec3 {
        self.water_points += 1;
        let sink = *self.sink.get_or_insert(c.sink_factor);
        let float = c.seaplane && p.float;
        let (fvp, mut nvp, rvp) = (forward.dot(vs), normal.dot(vs), right.dot(vs));
        if !float && d < -2.0 {
            d = -2.0;
        }
        let wave = 1.0 + 0.3 * ((1 + i % 3) as f64 * c.time).sin();
        let lift = -(1.0 + 0.3 * fvp.max(0.0)) * sink * d * d.abs() * wave;
        if float {
            nvp = nvp.min(0.0);
        }
        let mut tn = normal * (lift + (-1000.0 * nvp).clamp(-4000.0, 4000.0));
        let (mut ff, mut fr) = if float {
            (-40.0 * fvp, -300.0 * rvp)
        } else {
            (-500.0 * fvp, -800.0 * rvp)
        };
        let fric = ff.hypot(fr);
        if fric > 50_000.0 {
            ff *= 50_000.0 / fric;
            fr *= 50_000.0 / fric;
        }
        tn += forward * ff + right * fr;
        if !float && sink > 1.0 {
            self.sink = Some((sink - 0.4 * c.dt).max(1.0));
        }
        if nvp < 0.0 && vrel.length() >= 13.0 {
            let severity = (fvp * fvp + rvp * rvp) * 4e-4 + nvp * nvp / 900.0;
            if severity > 1.0 {
                self.hits.push((i, severity));
            }
        }
        tn
    }
}
