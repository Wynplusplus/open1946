//! One aircraft's rigid-body simulation: `docs/behavior/flight-model.md`.

use crate::aircraft::Aircraft;
use crate::atmosphere::{self, G};
use crate::engine::{EngineEnv, EngineKind, EngineState};
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
    /// Gear lever: 1 = down, 0 = up.
    pub gear: f64,
    /// Emergency boost (WEP) switch, for boost systems not engaged by the throttle alone.
    pub wep: bool,
    /// Dive brake lever, 0..1.
    pub airbrake: f64,
    /// Tail wheel lock (aircraft with `CLockTailwheel`).
    pub tail_lock: bool,
}

/// The runtime's control filter: exponential approach with time constant `tau`, plus a linear
/// approach at `rate` per second.
fn filter(dt: f64, target: f64, cur: f64, tau: f64, rate: f64) -> f64 {
    let mut x = target + (cur - target) * (-dt / tau).exp();
    if x < target {
        x = (x + rate * dt).min(target);
    } else if x > target {
        x = (x - rate * dt).max(target);
    }
    x
}

/// Stick input to surface position with trim (`docs/behavior/controls.md`).
fn trimmed(input: f64, trim: f64, limit: f64) -> f64 {
    let x = input.clamp(-1.0, 1.0).clamp(-limit, limit);
    let k = if trim * x > 0.0 { -1.0 } else { 1.0 };
    (1.0 + k * trim.abs()) * x + trim
}

/// Flap lever positions.
pub const FLAP_STEPS: [f64; 4] = [0.0, 0.2, 0.33, 1.0];

pub use crate::gear::Ground;
use crate::gear::{GearInputs, GearState};

pub const TICK: f64 = 0.030;

/// Spring compression of each wheel under an even share of the aircraft's weight, metres.
/// Static compression of the gear legs used to place wheels when the gear animation is unknown.
pub const GEAR_STATIC_SINK: f64 = 0.02;

pub struct Sim {
    pub ac: Aircraft,
    pub pos: DVec3,
    pub vel: DVec3,
    /// Body to world.
    pub ori: DQuat,
    /// Angular velocity, body frame.
    pub w: DVec3,
    pub fuel: f64,
    pub nitro: f64,
    /// Trim lever targets (aileron, elevator, rudder) and the trim actually applied.
    pub trim_target: [f64; 3],
    pub trim: [f64; 3],
    /// Dive brake position.
    pub airbrake: f64,
    /// Difficulty: engines suffer from overheating.
    pub engine_overheat: bool,
    pub gear: GearState,
    dry_friction: f64,
    ticks: u64,
    /// Body-x airspeed of the last step, for the controls' speed limits.
    last_vx: f64,
    /// Elevator before the high-speed neutral shift.
    ev: f64,
    pub engines: Vec<EngineState>,
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
    /// Lifting and control areas left by damage.
    pub sq: crate::damage::Squares,
    /// Load limits, fatigue and the parts still attached.
    pub structure: crate::damage::Structure,
    /// Aerodynamic and thrust load factor along body z of the last step.
    pub overload: f64,
    /// Flown by the AI: the simplified flight model (`ai_fm`).
    pub ai: bool,
    /// Realism option "complex engine management": mixture and supercharger by hand.
    pub complex_engine: bool,
    /// Rotation rates the AI's controls command, body axes (the runtime's `Wtrue`).
    pub wtrue: DVec3,
}

fn deg(r: f64) -> f64 {
    r.to_degrees()
}

impl Sim {
    /// Starts stopped engines or stops running ones (`at_airfield`: for engines that need ground support).
    pub fn toggle_engines(&mut self, at_airfield: bool) {
        let env = EngineEnv {
            dt: TICK,
            speed: self.vel.length(),
            altitude: self.pos.z,
            at_airfield,
            vmax_h: self.ac.vmax_h,
            overheat: false,
        };
        for (p, st) in self.ac.engines.iter().zip(&mut self.engines) {
            st.toggle(p, &env);
        }
    }

    /// Sets the propeller lever (0..1), or `None` for automatic.
    pub fn set_prop(&mut self, l: Option<f64>) {
        for st in &mut self.engines {
            st.prop_manual = l;
        }
    }

    /// Moves every engine's mixture lever by `d` (0..1.2; 1 is normal).
    pub fn change_mix(&mut self, d: f64) {
        for (p, st) in self.ac.engines.iter().zip(&mut self.engines) {
            let v = st.mix + d;
            st.set_mix(p, v);
        }
    }

    /// Next magneto setting (both → left → right → off → both).
    pub fn cycle_magneto(&mut self) {
        for (p, st) in self.ac.engines.iter().zip(&mut self.engines) {
            let next = match st.magneto {
                3 => 1,
                1 => 2,
                2 => 0,
                _ => 3,
            };
            st.set_magneto(p, next);
        }
    }

    /// Sets the radiator flaps (0..1), or `None` for automatic.
    pub fn set_radiator(&mut self, r: Option<f64>) {
        for st in &mut self.engines {
            st.radiator_manual = r;
        }
    }

    /// Engines cold and stopped (a cold start on the ground).
    pub fn set_engines_cold(&mut self) {
        let ambient = atmosphere::temperature(self.pos.z) - 273.15;
        for (p, st) in self.ac.engines.iter().zip(&mut self.engines) {
            *st = EngineState::cold(p, ambient);
        }
    }

    /// Moves a trim lever by `steps` notches (0.00625 each) if the aircraft has that trim.
    pub fn trim(&mut self, axis: usize, steps: f64) {
        if self.ac.controls.trim[axis] {
            self.trim_target[axis] = (self.trim_target[axis] + 0.00625 * steps).clamp(-0.5, 0.5);
        }
    }

    /// Next flap lever position up or down from `cur` (`FLAP_STEPS`, or up/down only).
    pub fn flap_step(&self, cur: f64, down: bool) -> f64 {
        if !self.ac.controls.flaps {
            return 0.0;
        }
        if !self.ac.controls.flap_steps {
            return if down { 1.0 } else { 0.0 };
        }
        if down {
            FLAP_STEPS
                .iter()
                .copied()
                .find(|&s| s > cur + 1e-6)
                .unwrap_or(1.0)
        } else {
            FLAP_STEPS
                .iter()
                .rev()
                .copied()
                .find(|&s| s < cur - 1e-6)
                .unwrap_or(0.0)
        }
    }

    pub fn new(ac: Aircraft, pos: DVec3, ori: DQuat, speed: f64) -> Self {
        let wing = ac.wing_polar();
        let fuel = ac.fuel_max;
        let nitro = ac.nitro_max;
        let ac_trim = ac.controls.default_trim;
        let sq = crate::damage::Squares::new(&ac);
        let structure = crate::damage::Structure::new(&ac);
        // In flight, blades start at their working pitch for the airspeed (no windmilling overspeed).
        let engines = ac
            .engines
            .iter()
            .map(|p| {
                let mut e = EngineState::new(p, p.w_nom);
                if speed > 1.0 && p.piston {
                    let phi_w = (speed / (p.w_nom * p.reductor * p.prop_r).max(1.0)).atan();
                    e.phi = (phi_w + p.prop_aoa0).clamp(p.phi_min, p.phi_max);
                    e.phi_target = e.phi;
                }
                e
            })
            .collect();
        Sim {
            engines,
            vel: ori * DVec3::new(speed, 0.0, 0.0),
            ac,
            pos,
            ori,
            w: DVec3::ZERO,
            fuel,
            nitro,
            trim_target: ac_trim,
            trim: ac_trim,
            airbrake: 0.0,
            engine_overheat: true,
            gear: GearState::default(),
            dry_friction: 1.0,
            ticks: 0,
            last_vx: speed,
            ev: 0.0,
            ct: Inputs {
                gear: 1.0,
                ..Default::default()
            },
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
            sq,
            structure,
            overload: 1.0,
            ai: false,
            complex_engine: false,
            wtrue: DVec3::ZERO,
        }
    }

    /// Running engines at idle (ground start) or nominal RPM (air start).
    pub fn set_engines_idle(&mut self, idle: bool) {
        for (p, st) in self.ac.engines.iter().zip(&mut self.engines) {
            *st = EngineState::new(p, if idle { p.w_min } else { p.w_nom });
        }
    }

    pub fn mass(&self) -> f64 {
        self.ac.mass_empty + self.fuel + self.nitro
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
        // Controls (docs/behavior/controls.md).
        let c = &self.ac.controls;
        let vx = self.last_vx;
        let lim = |thr: f64, p: i32, floor: f64| {
            if vx > thr {
                (thr.powi(p) / vx.powi(p)).max(floor)
            } else {
                1.0
            }
        };
        let (ail_lim, rud_lim, elev_lim) = (
            lim(c.thresholds[0], 3, 0.2),
            lim(c.thresholds[1], 2, 0.2),
            lim(c.thresholds[2], 2, 0.4),
        );
        for i in 0..3 {
            self.trim[i] = filter(dt, self.trim_target[i], self.trim[i], 999.9, 0.25);
        }
        let ail_in = if self.structure.ailerons_failed {
            0.0
        } else {
            input.aileron
        };
        let ail = trimmed(ail_in, self.trim[0], ail_lim);
        self.ct.aileron = filter(
            dt,
            ail,
            self.ct.aileron,
            0.2 * (1.0 + 0.3 * input.aileron.abs()),
            0.025,
        );
        let ev = trimmed(input.elevator, self.trim[1], elev_lim);
        // The elevator surface lags in `ev`; at high speed the trimmed neutral moves up.
        self.ev = filter(
            dt,
            ev,
            self.ev,
            0.3 * (1.0 + 0.3 * input.elevator.abs()),
            0.022,
        );
        self.ct.elevator = (self.ev - 0.25 * (1.0 - elev_lim)).clamp(-1.0, 1.0);
        let rud = trimmed(input.rudder, self.trim[2], rud_lim);
        self.ct.rudder = filter(
            dt,
            rud,
            self.ct.rudder,
            0.35 * (1.0 + 0.3 * input.rudder.abs()),
            0.025,
        );
        let max_throttle = self
            .ac
            .engines
            .iter()
            .map(|e| e.max_throttle())
            .fold(1.0, f64::max);
        // The player's throttle acts directly.
        self.ct.throttle = input.throttle.clamp(0.0, max_throttle);
        let flaps_target = if c.flaps && !self.structure.flaps_failed {
            input.flaps.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let kmh = vx * 3.6;
        let cvt = |lo: f64, hi: f64| lo + (hi - lo) * ((kmh - 150.0) / 130.0).clamp(0.0, 1.0);
        let flap_rate = if self.ct.flaps > flaps_target {
            cvt(0.15, 0.25)
        } else {
            cvt(0.15, 0.02)
        };
        self.ct.flaps = filter(dt, flaps_target, self.ct.flaps, 999.0, flap_rate);
        self.ct.brakes = if input.brakes > self.ct.brakes {
            (self.ct.brakes + 0.3 * dt).min(input.brakes)
        } else {
            input.brakes
        };
        self.ct.wep = input.wep;
        self.ct.tail_lock = input.tail_lock && c.tailwheel_lock;
        self.ct.gear = if c.retractable {
            filter(dt, input.gear.clamp(0.0, 1.0), self.ct.gear, 999.9, 0.2)
        } else {
            1.0
        };
        if c.airbrake {
            self.airbrake = filter(
                dt,
                input.airbrake.clamp(0.0, 1.0),
                self.airbrake,
                999.9,
                0.5,
            );
        }
        self.ct.airbrake = self.airbrake;
        self.wing.set_flaps(self.ct.flaps);

        let ac = &self.ac;
        let mass = self.mass();
        let gravity = mass * G;
        let inv = self.ori.inverse();
        let vflow = inv * self.vel; // no wind yet
        self.last_vx = vflow.x;
        let v2 = vflow.length_squared();
        let v = v2.sqrt();
        let rho = atmosphere::density(self.pos.z);
        let aoa = -deg(vflow.z.atan2(vflow.x));
        let aos = deg(vflow.y.atan2(vflow.x));
        let ias = v * (rho / atmosphere::RHO0).sqrt();
        let full_mach = v / atmosphere::sonic_speed(self.pos.z);
        let mach = full_mach.min(0.8);
        let kq = 1.0 / (1.0 - mach * mach).sqrt();
        let q = |vv: DVec3| 0.5 * rho * vv.length_squared();
        let w = self.w;
        let flap = self.ct.flaps;
        let ground_z = ground.height(self.pos.x, self.pos.y);
        let height = (self.pos.z - ground_z).max(0.0);

        // Engines (docs/behavior/engine.md).
        let mut thrust = 0.0;
        let mut torque = 0.0;
        let throttle = self.ct.throttle;
        let (mut burn, mut burn_nitro) = (0.0, 0.0);
        for (p, st) in ac.engines.iter().zip(&mut self.engines) {
            // OpenIL2 control mapping: a rocket booster fires while the throttle is in the boost range.
            let (t, fuel_left) = match p.kind {
                EngineKind::RocketBoost => {
                    (if throttle > 1.0 { 1.0 } else { 0.0 }, self.nitro > 0.0)
                }
                EngineKind::Pvrd => (throttle, self.nitro > 0.0),
                _ => (throttle, self.fuel > 0.0),
            };
            let wep = p.wep_active(t, self.ct.wep, self.nitro > 0.0);
            st.overload = self.overload;
            st.complex = self.complex_engine && !self.ai;
            st.step_any(p, t, wep, vflow.x, v, self.pos.z, dt, fuel_left);
            let env = EngineEnv {
                dt,
                speed: v,
                altitude: self.pos.z,
                at_airfield: false,
                vmax_h: ac.vmax_h,
                overheat: self.engine_overheat,
            };
            st.manage(p, t, wep, &env);
            thrust += st.thrust;
            torque += st.torque;
            burn += st.fuel_rate * dt;
            burn_nitro += st.nitro_rate * dt;
            // Slipstream comes from propellers only.
            if p.piston {
                let denom = 6.0 * rho * (vflow.x + self.add_vflow).max(1.0);
                self.add_vflow = 0.95 * self.add_vflow + 0.05 * st.thrust / denom;
                self.add_vside =
                    (0.95 * self.add_vside + 0.05 * (st.prop_torque / p.prop_r) / denom).max(0.0);
            }
        }
        self.fuel = (self.fuel - burn).max(0.0);
        self.nitro = (self.nitro - burn_nitro).max(0.0);
        let power_out = throttle.min(1.0);
        let prop_sign = ac.engines.first().map_or(1.0, |e| e.direction);
        let single_prop = ac.engines.len() == 1;

        let radiator = if self.engines.is_empty() {
            0.0
        } else {
            self.engines.iter().map(|e| e.radiator).sum::<f64>() / self.engines.len() as f64
        };

        let (mut af, mut am, excessive_force, wtrue) = 'aero: {
            if self.ai {
                let out = crate::ai_fm::ai_aero(&crate::ai_fm::AiAeroIn {
                    ac,
                    sq: &self.sq,
                    wing: &self.wing,
                    tail: &self.tail,
                    ct: &self.ct,
                    vflow,
                    rho,
                    aoa,
                    aos,
                    full_mach,
                    kq,
                    height,
                    gravity,
                    w,
                    prop_sign,
                    on_ground: self.on_ground,
                    tail_intact: self.structure.intact(crate::damage::TAIL1),
                    wing_mid_l: self.structure.intact(crate::damage::WING_L_IN + 1),
                    wing_mid_r: self.structure.intact(crate::damage::WING_R_IN + 1),
                    radiator,
                });
                break 'aero (out.af, out.am, out.excessive_force, out.wtrue);
            }
            // Wing halves.
            let delta = ac.wing_v * aos.to_radians().sin()
                + ac.sens_roll * self.sq.sens[0] * (1.0 - 0.1 * flap) * 14.0 * self.ct.aileron;
            let half = |side: f64| {
                let p = DVec3::new(-ac.gcenter, side * 0.85 * ac.arm_wing_out, -0.5);
                let mut vn = w.cross(p) + vflow;
                let a_geo = side * delta - deg(vn.z.atan2(vn.x));
                vn.x += 0.07 * self.add_vflow;
                let qn = q(vn);
                let a = side * delta
                    - deg((vn.z + side * 0.07 * self.add_vside * prop_sign).atan2(vn.x));
                let mut ail_drag = 0.015 * side * delta;
                if ail_drag < 0.0 {
                    ail_drag *= 0.18;
                }
                let (cx, cy) = self.wing.coeffs_mach(a, full_mach, ac.mach.as_ref());
                let (fcx, fcy) = self.fusel.coeffs(aos);
                // Gear (0.035 when down, none for fixed gear) and dive brake drag.
                let gear_cx = if ac.controls.retractable {
                    0.035 * self.ct.gear
                } else {
                    0.0
                };
                let radiator_cx = 0.003 * radiator;
                let cw = DVec3::new(
                    -qn * (cx + ail_drag + gear_cx + radiator_cx + ac.airbrake_cx * self.airbrake)
                        - qn * fcx,
                    -qn * fcy,
                    qn * cy * kq,
                );
                (cw, a_geo, qn)
            };
            let (mut cwl, al, ql) = half(1.0);
            let (mut cwr, ar, _qr) = half(-1.0);

            // Stall and spin.
            let crit = self.wing.aoa_crit_h;
            let mut rudder_infl = 1.0;
            if al > crit || ar > crit {
                self.spin_coeff += 0.2 * dt;
                if self.sq.rudders > 0.0 && self.ct.rudder.abs() > 0.5 && self.ct.rudder * w.z > 0.0
                {
                    self.spin_coeff -= 0.3 * dt;
                }
                let amax = al.max(ar);
                let turb = (0.8 * (amax - crit)).clamp(1.0, 15.0);
                let mut s = ql * turb * self.spin_coeff;
                if height < 10.0 {
                    s *= 0.1 * height;
                }
                let (stalled, other) = if al > ar {
                    (&mut cwl, &mut cwr)
                } else {
                    (&mut cwr, &mut cwl)
                };
                other.x += 0.02 * s * ac.spin_cx_loss;
                stalled.x -= 0.25 * s * ac.spin_cx_loss;
                other.z += 0.02 * s * ac.spin_cy_loss;
                stalled.z -= 0.1 * s * ac.spin_cy_loss;
                rudder_infl = 1.0 + 0.035 * turb;
            } else {
                self.spin_coeff -= 0.2 * dt;
            }
            self.spin_coeff = self
                .spin_coeff
                .clamp(0.5 + 0.4 * power_out, 1.2 + 0.4 * power_out);

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
                let k = if vn.x < 0.0 {
                    0.0
                } else {
                    (0.0011 * vn.x * vn.x).min(1.0)
                };
                side = s * k * self.add_vside;
            }
            let qt = q(vn);
            let defl_e = self.ct.elevator * if self.ct.elevator > 0.0 { 28.0 } else { 20.0 };
            let tail_a = |dz: f64| {
                -deg((vn.z + dz).atan2(vn.x))
                    - 2.0
                    - 0.002 * v
                    - ac.sens_pitch * self.sq.sens[1] * defl_e
            };
            let (al_t, ar_t) = if single_prop {
                (
                    tail_a(-0.36 * side * prop_sign),
                    tail_a(0.36 * side * prop_sign),
                )
            } else {
                (tail_a(0.0), tail_a(0.0))
            };
            let tail_force = |a: f64| {
                let (cx, cy) = self.tail.coeffs(a);
                DVec3::new(-qt * cx, 0.0, qt * cy) * ((self.sq.stab + self.sq.elevators) * 0.5)
            };
            let fhl = tail_force(al_t);
            let fhr = tail_force(ar_t);
            let defl_r = if self.sq.rudders < 0.05 {
                0.0
            } else {
                28.0 * self.ct.rudder
            };
            let fin_a = -deg((vn.y - 0.5 * side * prop_sign).atan2(vn.x))
                + ac.sens_yaw * self.sq.sens[2] * rudder_infl * defl_r;
            let (cx, cy) = self.tail.coeffs(fin_a);
            let fv =
                DVec3::new(-qt * cx, qt * cy, 0.0) * (0.2 + 1.5 * self.sq.keel + self.sq.rudders);

            // Wing forces and drag.
            let mut fwl = cwl * self.sq.wing_l.iter().sum::<f64>();
            let mut fwr = cwr * self.sq.wing_r.iter().sum::<f64>();
            let parasite = self.sq.drag_produced;
            fwl.x -= q(vflow) * parasite * 0.5;
            fwr.x -= q(vflow) * parasite * 0.5;
            let mut af = fwl + fwr;
            let mut excessive_force = false;
            if !af.is_finite() {
                af = DVec3::ZERO;
            } else if af.length() > gravity * 50.0 {
                af = af.normalize() * gravity * 50.0;
                excessive_force = true;
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
            let xcg = ac.gcenter
                + ac.gc_flaps_shift * (3.0 * flap).min(1.0) * (1.0 - u)
                + ac.gc_aoa_shift * u;
            let saos = aos.to_radians().sin();
            let mut am = DVec3::ZERO;
            if aoa.abs() < 12.0 {
                let bank =
                    self.bank_deg().clamp(-30.0, 30.0) * ((vflow.x - 50.0).min(50.0) * 0.003);
                am.x -= bank * 0.01 * gravity;
            }
            am += DVec3::new(-xcg, ac.arm_wing_mid * (1.3 + saos), -ac.gcenter_z).cross(fwl);
            am += DVec3::new(-xcg, -ac.arm_wing_mid * (1.3 - saos), -ac.gcenter_z).cross(fwr);
            am += DVec3::new(-ac.arm_stab, 1.0, 0.0).cross(fhl);
            am += DVec3::new(-ac.arm_stab, -1.0, 0.0).cross(fhr);
            am += DVec3::new(-ac.arm_keel, 0.0, 1.0).cross(fv);
            am += self.tail_moments(vflow.x, prop_sign);
            if !am.is_finite() {
                am = DVec3::ZERO;
            } else if am.length() > gravity * 150.0 {
                am = am.normalize() * gravity * 150.0;
            }

            (af, am, excessive_force, DVec3::ZERO)
        };
        // Engine force and torque.
        af.x += thrust;
        am.x += torque;

        self.wtrue = wtrue;
        // Per-step damping and rate limit.
        if self.ai {
            if self.w.length_squared() > 36.0 {
                self.w *= 6.0 / self.w.length();
            }
            let wl: f64 = self.sq.wing_l.iter().sum();
            let wr: f64 = self.sq.wing_r.iter().sum();
            let k = 1.0 / (0.1 + ac.wing_area).max(1.0);
            self.w.x *= 1.0 - 0.12 * (0.2 + wl + wr) * k;
            self.w.y *= 1.0 - 0.5 * (0.2 + self.sq.stab) * k;
            self.w.z *= 1.0 - 0.5 * (0.2 + self.sq.keel) * k;
        } else {
            self.w *= (1.0 - 1e-5 * ias).max(0.8);
            if self.w.length_squared() > 25.0 {
                self.w *= 5.0 / self.w.length();
            }
        }

        let j = ac.j0 * ac.mass_empty;
        self.ticks += 1;
        if self.ticks % 2 == 1 {
            self.gear.read_surface(ground, self.pos.x, self.pos.y);
        }
        let mut gear_f = self.gear_forces(ground, mass);
        let n = if gear_f.0 == DVec3::ZERO && gear_f.1 == DVec3::ZERO {
            1
        } else {
            5
        };
        // Static friction: at low power, small forces and motion leave a grounded aircraft standing.
        self.dry_friction -= 0.01;
        if self.gear.changed {
            self.dry_friction = 1.0;
        }
        if self.gear.points_on_ground > 0 {
            self.dry_friction += 0.02;
        }
        self.dry_friction = self.dry_friction.clamp(1.0, 32.0);
        let power = (self
            .engines
            .iter()
            .map(|e| {
                if e.running() {
                    self.ct.throttle.min(1.1) * e.readyness
                } else {
                    0.0
                }
            })
            .sum::<f64>()
            / self.engines.len().max(1) as f64)
            .min(1.1);
        let k = (4.0 * (0.25 - power)).max(0.0).powi(2) * self.dry_friction;
        let km = k * mass * mass;
        let sum_f = af + gear_f.0 + self.ori.inverse() * DVec3::new(0.0, 0.0, -gravity);
        let sum_m = am + gear_f.1;
        let stuck = !((self.gear.points_on_ground == 0 && self.gear.wheels_on_ground < 3)
            || k == 0.0
            || sum_m.length_squared() > 2.0 * km
            || sum_f.length_squared() > 80.0 * km
            || self.w.length_squared() > 1.4e-4 * k
            || self.vel.length_squared() > 0.09 * k);
        let n = if stuck { 0 } else { n };
        let sub = dt / n.max(1) as f64;
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
            self.ori = (self.ori * DQuat::from_scaled_axis((self.w + wtrue) * sub)).normalize();
            self.vel += self.ori * (f / mass) * sub;
            self.pos += self.vel * sub;
            if k + 1 < n {
                gear_f = self.gear_forces(ground, mass);
            }
        }
        self.on_ground = self.gear.on_ground();
        self.aoa = aoa;
        self.ias = ias;
        self.thrust = thrust;
        self.overload = (af.z + gear_f.0.z) / gravity;
        let checks = crate::damage::StructureInputs {
            dt,
            mass,
            overload: self.overload,
            excessive_force,
            ias,
            on_ground: self.on_ground,
            flaps_lever: input.flaps,
            flaps: self.ct.flaps,
        };
        self.check_structure(&checks);
    }

    fn gear_forces(&mut self, ground: &impl Ground, mass: f64) -> (DVec3, DVec3) {
        let c = GearInputs {
            pos: self.pos,
            ori: self.ori,
            vel: self.vel,
            w: self.w,
            mass,
            brake: self.ct.brakes,
            rudder: self.ct.rudder,
            gear: self.ct.gear,
            tail_locked: self.ct.tail_lock,
            gc_gear_shift: self.ac.gc_gear_shift,
            wing_end: self.ac.wingspan * 0.5,
        };
        self.gear.forces(&self.ac.gear, ground, &c)
    }

    /// Bank angle in degrees, positive when banked right.
    pub fn bank_deg(&self) -> f64 {
        let y = self.ori * DVec3::Y;
        let z = self.ori * DVec3::Z;
        deg(y.z.atan2(z.z))
    }
}
