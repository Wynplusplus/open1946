//! One aircraft's rigid-body simulation: `docs/behavior/flight-model.md`.

use crate::aircraft::Aircraft;
use crate::atmosphere::{self, G};
use crate::polar::Polar;
use glam::{DQuat, DVec3};

/// Pilot inputs, all in [-1, 1] except throttle, flaps and brakes in [0, 1].
#[derive(Clone, Copy, Debug, Default)]
pub struct Inputs {
    pub aileron: f64,
    pub elevator: f64,
    pub rudder: f64,
    pub throttle: f64,
    pub flaps: f64,
    pub brakes: f64,
}

/// Ground height query in world coordinates.
pub trait Ground {
    fn height(&self, x: f64, y: f64) -> f64;
}

impl<F: Fn(f64, f64) -> f64> Ground for F {
    fn height(&self, x: f64, y: f64) -> f64 {
        self(x, y)
    }
}

pub const TICK: f64 = 0.030;

/// Spring compression of each wheel under an even share of the aircraft's weight, metres.
pub const GEAR_STATIC_SINK: f64 = 0.08;

pub struct Sim {
    pub ac: Aircraft,
    pub pos: DVec3,
    pub vel: DVec3,
    /// Body to world.
    pub ori: DQuat,
    /// Angular velocity, body frame.
    pub w: DVec3,
    pub fuel: f64,
    /// Control surface positions (lagged inputs).
    pub ct: Inputs,
    wing: Polar,
    fusel: Polar,
    tail: Polar,
    spin_coeff: f64,
    add_vflow: f64,
    add_vside: f64,
    pub on_ground: bool,
    /// Diagnostics from the last step.
    pub aoa: f64,
    pub ias: f64,
    pub thrust: f64,
}

/// First-order approach of `x` to `target` at `rate` per second.
fn approach(x: f64, target: f64, rate: f64, dt: f64) -> f64 {
    x + (target - x).clamp(-rate * dt, rate * dt)
}

fn deg(r: f64) -> f64 {
    r.to_degrees()
}

impl Sim {
    pub fn new(ac: Aircraft, pos: DVec3, ori: DQuat, speed: f64) -> Self {
        let wing = ac.wing_polar();
        let fuel = ac.fuel_max;
        Sim {
            vel: ori * DVec3::new(speed, 0.0, 0.0),
            ac,
            pos,
            ori,
            w: DVec3::ZERO,
            fuel,
            ct: Inputs::default(),
            wing,
            fusel: Polar::fixed(0.02, 0.2, 17.0, 0.0),
            tail: Polar::fixed(0.085, 1.1, 17.0, 0.02),
            spin_coeff: 0.5,
            add_vflow: 0.0,
            add_vside: 0.0,
            on_ground: false,
            aoa: 0.0,
            ias: 0.0,
            thrust: 0.0,
        }
    }

    pub fn mass(&self) -> f64 {
        self.ac.mass_empty + self.fuel
    }

    /// Advances one game tick, splitting it like the original when the aircraft is fast.
    pub fn tick(&mut self, input: &Inputs, ground: &impl Ground) {
        self.step(TICK, input, ground);
    }

    fn step(&mut self, dt: f64, input: &Inputs, ground: &impl Ground) {
        if self.vel.length() * dt > 5.0 {
            self.step(dt * 0.5, input, ground);
            self.step(dt * 0.5, input, ground);
            return;
        }
        // OpenIL2 simplification: surfaces follow the stick with a fixed rate.
        self.ct.aileron = approach(self.ct.aileron, input.aileron, 4.0, dt);
        self.ct.elevator = approach(self.ct.elevator, input.elevator, 4.0, dt);
        self.ct.rudder = approach(self.ct.rudder, input.rudder, 4.0, dt);
        self.ct.throttle = approach(self.ct.throttle, input.throttle, 1.0, dt);
        self.ct.flaps = approach(self.ct.flaps, input.flaps, 0.2, dt);
        self.ct.brakes = input.brakes;
        self.wing.set_flaps(self.ct.flaps);

        let ac = &self.ac;
        let mass = self.mass();
        let gravity = mass * G;
        let inv = self.ori.inverse();
        let vflow = inv * self.vel; // no wind yet
        let v2 = vflow.length_squared();
        let v = v2.sqrt();
        let rho = atmosphere::density(self.pos.z);
        let aoa = -deg(vflow.z.atan2(vflow.x));
        let aos = deg(vflow.y.atan2(vflow.x));
        let ias = v * (rho / atmosphere::RHO0).sqrt();
        let mach = (v / atmosphere::sonic_speed(self.pos.z)).min(0.8);
        let kq = 1.0 / (1.0 - mach * mach).sqrt();
        let q = |vv: DVec3| 0.5 * rho * vv.length_squared();
        let w = self.w;
        let flap = self.ct.flaps;
        let ground_z = ground.height(self.pos.x, self.pos.y);
        let height = (self.pos.z - ground_z).max(0.0);

        // Engine (simplified, see spec).
        let mut thrust = 0.0;
        let mut torque = 0.0;
        let mut power_out = 0.0;
        for e in &ac.engines {
            let alt_factor = if self.pos.z <= e.compressor_alt {
                1.0
            } else {
                rho / atmosphere::density(e.compressor_alt)
            };
            let p = e.power_w * self.ct.throttle * alt_factor * if self.fuel > 0.0 { 1.0 } else { 0.0 };
            let t = (0.8 * p / v.max(1.0)).min(0.0164 * p);
            thrust += t;
            torque += e.prop_sign * p / e.prop_omega;
            power_out += self.ct.throttle;
            let denom = 6.0 * rho * (vflow.x + self.add_vflow).max(1.0);
            self.add_vflow = 0.95 * self.add_vflow + 0.05 * t / denom;
            self.add_vside = (0.95 * self.add_vside + 0.05 * (p / e.prop_omega / e.prop_radius) / denom).max(0.0);
        }
        if !ac.engines.is_empty() {
            power_out /= ac.engines.len() as f64;
        }
        let prop_sign = ac.engines.first().map_or(1.0, |e| e.prop_sign);
        let single_prop = ac.engines.len() == 1;

        // Wing halves.
        let delta = ac.wing_v * aos.to_radians().sin() + ac.sens_roll * (1.0 - 0.1 * flap) * 14.0 * self.ct.aileron;
        let half = |side: f64| {
            let p = DVec3::new(-ac.gcenter, side * 0.85 * ac.arm_wing_out, -0.5);
            let mut vn = w.cross(p) + vflow;
            let a_geo = side * delta - deg(vn.z.atan2(vn.x));
            vn.x += 0.07 * self.add_vflow;
            let qn = q(vn);
            let a = side * delta - deg((vn.z + side * 0.07 * self.add_vside * prop_sign).atan2(vn.x));
            let mut ail_drag = 0.015 * side * delta;
            if ail_drag < 0.0 {
                ail_drag *= 0.18;
            }
            let (cx, cy) = self.wing.coeffs(a);
            let (fcx, fcy) = self.fusel.coeffs(aos);
            let cw = DVec3::new(-qn * (cx + ail_drag + ac.airbrake_cx * 0.0) - qn * fcx, -qn * fcy, qn * cy * kq);
            (cw, a_geo, qn)
        };
        let (mut cwl, al, ql) = half(1.0);
        let (mut cwr, ar, _qr) = half(-1.0);

        // Stall and spin.
        let crit = self.wing.aoa_crit_h;
        let mut rudder_infl = 1.0;
        if al > crit || ar > crit {
            self.spin_coeff += 0.2 * dt;
            if ac.rudder_area > 0.0 && self.ct.rudder.abs() > 0.5 && self.ct.rudder * w.z > 0.0 {
                self.spin_coeff -= 0.3 * dt;
            }
            let amax = al.max(ar);
            let turb = (0.8 * (amax - crit)).clamp(1.0, 15.0);
            let mut s = ql * turb * self.spin_coeff;
            if height < 10.0 {
                s *= 0.1 * height;
            }
            let (stalled, other) = if al > ar { (&mut cwl, &mut cwr) } else { (&mut cwr, &mut cwl) };
            other.x += 0.02 * s * ac.spin_cx_loss;
            stalled.x -= 0.25 * s * ac.spin_cx_loss;
            other.z += 0.02 * s * ac.spin_cy_loss;
            stalled.z -= 0.1 * s * ac.spin_cy_loss;
            rudder_infl = 1.0 + 0.035 * turb;
        } else {
            self.spin_coeff -= 0.2 * dt;
        }
        self.spin_coeff = self.spin_coeff.clamp(0.5 + 0.4 * power_out, 1.2 + 0.4 * power_out);

        // Ground effect.
        if height < 0.4 * ac.length {
            let k = 1.0 + 0.2 * (1.0 - height / (0.4 * ac.length));
            cwl *= DVec3::new(k, 1.0, k);
            cwr *= DVec3::new(k, 1.0, k);
        }

        // Horizontal tail and fin.
        let mut vn = w.cross(DVec3::new(-ac.arm_keel, 0.0, 0.0)) + vflow;
        let yz = (vn.y * vn.y + vn.z * vn.z).sqrt();
        let mut side = 0.0;
        if !ac.engines.is_empty() {
            let r = 1.0 / (1.0 + 0.04 * ac.arm_rudder).powi(2);
            let base = (vn.x + r * self.add_vflow).max(0.2);
            let s = (1.0 - 1.5 * yz / base).max(0.0);
            vn.x += s * r * self.add_vflow;
            let k = if vn.x < 0.0 { 0.0 } else { (0.0011 * vn.x * vn.x).min(1.0) };
            side = s * k * self.add_vside;
        }
        let qt = q(vn);
        let defl_e = self.ct.elevator * if self.ct.elevator > 0.0 { 28.0 } else { 20.0 };
        let tail_a = |dz: f64| -deg((vn.z + dz).atan2(vn.x)) - 2.0 - 0.002 * v - ac.sens_pitch * defl_e;
        let (al_t, ar_t) = if single_prop {
            (tail_a(-0.36 * side * prop_sign), tail_a(0.36 * side * prop_sign))
        } else {
            (tail_a(0.0), tail_a(0.0))
        };
        let tail_force = |a: f64| {
            let (cx, cy) = self.tail.coeffs(a);
            DVec3::new(-qt * cx, 0.0, qt * cy) * ((ac.stab_area + ac.elevator_area) * 0.5)
        };
        let fhl = tail_force(al_t);
        let fhr = tail_force(ar_t);
        let defl_r = if ac.rudder_area < 0.05 { 0.0 } else { 28.0 * self.ct.rudder };
        let fin_a = -deg((vn.y - 0.5 * side * prop_sign).atan2(vn.x)) + ac.sens_yaw * rudder_infl * defl_r;
        let (cx, cy) = self.tail.coeffs(fin_a);
        let fv = DVec3::new(-qt * cx, qt * cy, 0.0) * (0.2 + 1.5 * ac.keel_area + ac.rudder_area);

        // Wing forces and drag.
        let half_area: f64 = ac.wing_sections.iter().sum();
        let mut fwl = cwl * half_area;
        let mut fwr = cwr * half_area;
        let parasite = 0.0;
        fwl.x -= q(vflow) * parasite * 0.5;
        fwr.x -= q(vflow) * parasite * 0.5;
        let mut af = fwl + fwr;
        if !af.is_finite() {
            af = DVec3::ZERO;
        } else if af.length() > gravity * 50.0 {
            af = af.normalize() * gravity * 50.0;
        }

        // Moments.
        let u = {
            let mut u = 0.0111 * aoa.abs();
            if aoa > self.wing.aoa_crit_l && aoa < self.wing.aoa_crit_h {
                u = 0.0;
            } else if aoa >= self.wing.aoa_crit_h {
                u = u.min(0.3 * (aoa - self.wing.aoa_crit_h));
            } else {
                u = u.min(0.3 * (self.wing.aoa_crit_l - aoa));
            }
            u
        };
        let xcg = ac.gcenter + ac.gc_flaps_shift * (3.0 * flap).min(1.0) * (1.0 - u) + ac.gc_aoa_shift * u;
        let saos = aos.to_radians().sin();
        let mut am = DVec3::ZERO;
        if aoa.abs() < 12.0 {
            let bank = self.bank_deg().clamp(-30.0, 30.0) * ((vflow.x - 50.0).min(50.0) * 0.003);
            am.x -= bank * 0.01 * gravity;
        }
        am += DVec3::new(-xcg, ac.arm_wing_mid * (1.3 + saos), -ac.gcenter_z).cross(fwl);
        am += DVec3::new(-xcg, -ac.arm_wing_mid * (1.3 - saos), -ac.gcenter_z).cross(fwr);
        am += DVec3::new(-ac.arm_stab, 1.0, 0.0).cross(fhl);
        am += DVec3::new(-ac.arm_stab, -1.0, 0.0).cross(fhr);
        am += DVec3::new(-ac.arm_keel, 0.0, 1.0).cross(fv);
        if !am.is_finite() {
            am = DVec3::ZERO;
        } else if am.length() > gravity * 150.0 {
            am = am.normalize() * gravity * 150.0;
        }

        // Engine force and torque.
        af.x += thrust;
        am.x += torque;

        // Per-step damping and rate limit.
        self.w *= (1.0 - 1e-5 * ias).max(0.8);
        if self.w.length_squared() > 25.0 {
            self.w *= 5.0 / self.w.length();
        }

        let j = ac.j0 * ac.mass_empty;
        let mut gear_f = self.gear_forces(ground, mass);
        let n = if gear_f.2 { 5 } else { 1 };
        let sub = dt / n as f64;
        for k in 0..n {
            let f = af + gear_f.0 + self.ori.inverse() * DVec3::new(0.0, 0.0, -gravity);
            let m = am + gear_f.1;
            let w = self.w;
            let aw = DVec3::new(
                ((j.y - j.z) * w.y * w.z + m.x) / j.x,
                ((j.z - j.x) * w.z * w.x + m.y) / j.y,
                ((j.x - j.y) * w.x * w.y + m.z) / j.z,
            );
            self.w += aw * sub;
            self.ori = (self.ori * DQuat::from_scaled_axis(self.w * sub)).normalize();
            self.vel += self.ori * (f / mass) * sub;
            self.pos += self.vel * sub;
            if k + 1 < n {
                gear_f = self.gear_forces(ground, mass);
            }
        }
        self.on_ground = gear_f.2;
        self.aoa = aoa;
        self.ias = ias;
        self.thrust = thrust;
        let _ = (self.ct.brakes, self.add_vside);
    }

    /// Bank angle in degrees, positive when banked right.
    pub fn bank_deg(&self) -> f64 {
        let y = self.ori * DVec3::Y;
        let z = self.ori * DVec3::Z;
        deg(y.z.atan2(z.z))
    }

    /// Gear forces and moments in the body frame, and whether any wheel touches the ground.
    /// OpenIL2 simplification (see spec, "Landing gear").
    fn gear_forces(&self, ground: &impl Ground, mass: f64) -> (DVec3, DVec3, bool) {
        let n = self.ac.gear.len().max(1) as f64;
        let k = mass * G / (n * GEAR_STATIC_SINK);
        let c = 2.0 * 0.6 * (k * mass / n).sqrt();
        let (mut f_body, mut m_body, mut contact) = (DVec3::ZERO, DVec3::ZERO, false);
        let fwd_w = self.ori * DVec3::X;
        for g in &self.ac.gear {
            let pw = self.pos + self.ori * g.pos;
            let gz = ground.height(pw.x, pw.y);
            let depth = gz - pw.z;
            if depth <= 0.0 {
                continue;
            }
            contact = true;
            let vp = self.vel + self.ori * self.w.cross(g.pos);
            let fn_ = (k * depth - c * vp.z).max(0.0);
            let fwd = DVec3::new(fwd_w.x, fwd_w.y, 0.0).normalize_or_zero();
            let lat = DVec3::Z.cross(fwd);
            let (vf, vl) = (vp.dot(fwd), vp.dot(lat));
            let mu_f = 0.03 + 0.6 * self.ct.brakes;
            let mu_l = if g.castor { 0.1 } else { 0.8 };
            let smooth = |v: f64| (v / 0.5).clamp(-1.0, 1.0);
            let fw = DVec3::Z * fn_ - fwd * (mu_f * fn_ * smooth(vf)) - lat * (mu_l * fn_ * smooth(vl));
            let fb = self.ori.inverse() * fw;
            f_body += fb;
            m_body += g.pos.cross(fb);
        }
        (f_body, m_body, contact)
    }
}
