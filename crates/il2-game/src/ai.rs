//! AI pilots (`docs/behavior/ai.md`): the waypoint autopilot of the runtime's `AutopilotAI`,
//! flying the simplified AI flight model (`il2_fm::ai_fm`).

use glam::{DQuat, DVec3};
use il2_fm::{Inputs, Sim};
use il2_formats::mission::Waypoint;

/// Heading in the runtime's `azimut` sense: 360° minus the angle counter-clockwise from east.
fn azimut(s: &Sim) -> f64 {
    let fwd = s.ori * DVec3::X;
    (360.0 - fwd.y.atan2(fwd.x).to_degrees()).rem_euclid(360.0)
}

fn pitch(s: &Sim) -> f64 {
    (s.ori * DVec3::X).z.clamp(-1.0, 1.0).asin().to_degrees()
}

/// Wraps degrees into −180..180.
fn wrap(a: f64) -> f64 {
    let a = (a + 3600.0).rem_euclid(360.0);
    if a > 180.0 {
        a - 360.0
    } else {
        a
    }
}

/// What the pilot is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// On the runway, waiting `f64` seconds for the aircraft ahead.
    Lineup(f64),
    /// Rolling: straight down the runway on `heading` (radians, counter-clockwise from east).
    Takeoff { heading: f64 },
    /// Following waypoints.
    Cruise,
    /// Approach and landing on a runway: touchdown point and landing direction (radians).
    Landing { at: DVec3, heading: f64 },
    /// Stopped on the ground at the end of the flight.
    Parked,
}

pub struct AiPilot {
    pub phase: Phase,
    pub way: Vec<Waypoint>,
    pub cur: usize,
    /// Position in its flight (0 leads).
    pub index: usize,
    pub skill: u8,
    /// The controls as the pilot holds them.
    pub ct: Inputs,
    stab_speed: f64,
    stab_alt: f64,
    stab_dir: f64,
    prev_dist2: f64,
    prev_w: DVec3,
    prev_vx: f64,
    /// Integral part of the formation throttle.
    thr_i: f64,
    /// Touchdown point and direction of the runway at the landing waypoint.
    pub landing_runway: Option<(DVec3, f64)>,
    on_final: bool,
    ev_i: f64,
    hdg_i: f64,
    /// Final approach: pitch trim (degrees) and throttle integrators.
    pitch_i: f64,
    land_thr_i: f64,
    /// Touched down: rolling out (a bounce doesn't restart the approach).
    rollout: bool,
    /// Landing on a carrier (column and unit of the `LANDING` waypoint): the hook goes down and
    /// the touchdown point moves with the ship.
    pub landing_ship: Option<(String, usize)>,
    /// The touchdown point at the last step and its velocity (a moving deck).
    prev_at: Option<DVec3>,
    at_vel: DVec3,
}

impl AiPilot {
    pub fn new(way: Vec<Waypoint>, cur: usize, index: usize, skill: u8) -> Self {
        AiPilot {
            phase: Phase::Cruise,
            way,
            cur,
            index,
            skill,
            ct: Inputs {
                throttle: 0.8,
                gear: 0.0,
                ..Default::default()
            },
            stab_speed: 80.0,
            stab_alt: 1000.0,
            stab_dir: 0.0,
            prev_dist2: 1e8,
            prev_w: DVec3::ZERO,
            prev_vx: 0.0,
            thr_i: 0.6,
            landing_runway: None,
            on_final: false,
            ev_i: 0.0,
            hdg_i: 0.0,
            pitch_i: 3.0,
            land_thr_i: 0.4,
            rollout: false,
            landing_ship: None,
            prev_at: None,
            at_vel: DVec3::ZERO,
        }
    }

    /// The point the pilot steers for: the current waypoint, offset to its place in the flight.
    fn target(&self) -> Option<DVec3> {
        let w = self.way.get(self.cur)?;
        Some(DVec3::new(w.x, w.y, w.z))
    }

    /// Full power down the runway heading; the tail comes up, the aircraft lifts off at 1.2 times its
    /// flaps-down minimum speed and climbs out. OpenIL2's own simple takeoff pilot.
    fn takeoff(&mut self, s: &Sim, ground: f64, heading: f64) -> Inputs {
        let fwd = s.ori * DVec3::X;
        let (h0, f) = (
            glam::DVec2::new(heading.cos(), heading.sin()),
            fwd.truncate().normalize_or_zero(),
        );
        let yaw_err = h0.perp_dot(f);
        let v = s.vel.length();
        let vr = 1.2 * s.ac.vmin_flaps;
        let pitch = pitch(s);
        let target = if v < 0.6 * vr {
            99.0
        } else if v < vr {
            1.5
        } else {
            8.0
        };
        self.ct.throttle = 1.0;
        self.ct.brakes = 0.0;
        self.ct.tail_lock = v < 0.6 * vr;
        self.ct.flaps = 0.2;
        self.ct.rudder = (2.0 * yaw_err + 0.8 * s.w.z).clamp(-1.0, 1.0);
        // With the AI flight model the elevator sets the angle of attack it settles at.
        self.ct.elevator = if target > 90.0 {
            0.0
        } else if v < vr {
            (0.08 * (target - pitch) + 0.3 * s.w.y).clamp(-1.0, 1.0)
        } else {
            (0.5 + 0.08 * (target - pitch)).clamp(0.0, 1.0)
        };
        self.ct.aileron = (-0.05 * s.bank_deg() - 0.3 * s.w.x).clamp(-1.0, 1.0);
        let agl = s.pos.z - ground;
        if !s.on_ground && agl > 15.0 {
            self.ct.gear = 0.0;
        }
        if agl > 200.0 && !s.on_ground {
            self.ct.flaps = 0.0;
            self.ct.tail_lock = false;
            self.phase = Phase::Cruise;
            // Takeoff waypoints are behind once airborne.
            while self.cur + 1 < self.way.len() && self.way[self.cur].kind.starts_with("TAKEOFF") {
                self.cur += 1;
            }
        }
        self.ct
    }

    /// Waypoint reached: within 1 km and no longer closing (the runtime's `Way.isReached`).
    fn reached(&mut self, pos: DVec3) -> bool {
        let Some(t) = self.target() else { return false };
        let d2 = (pos.x - t.x).powi(2) + (pos.y - t.y).powi(2);
        if d2 < 1e6 && d2 > self.prev_dist2 {
            self.prev_dist2 = 1e8;
            return true;
        }
        self.prev_dist2 = d2;
        false
    }

    /// One step: the takeoff run, then the waypoint autopilot. Returns the controls to fly with.
    pub fn update(
        &mut self,
        s: &Sim,
        ground: f64,
        terrain: &dyn Fn(f64, f64) -> f64,
        dt: f64,
    ) -> Inputs {
        match self.phase {
            Phase::Lineup(t) => {
                self.ct = Inputs {
                    throttle: 0.0,
                    gear: 1.0,
                    brakes: 1.0,
                    tail_lock: true,
                    ..Default::default()
                };
                self.phase = if t > dt {
                    Phase::Lineup(t - dt)
                } else {
                    Phase::Takeoff {
                        heading: (s.ori * DVec3::X).y.atan2((s.ori * DVec3::X).x),
                    }
                };
                return self.ct;
            }
            Phase::Takeoff { heading } => return self.takeoff(s, ground, heading),
            Phase::Landing { at, heading } => {
                return self.land(s, ground, terrain, at, heading, dt)
            }
            Phase::Parked => {
                self.ct = Inputs {
                    throttle: 0.0,
                    gear: 1.0,
                    brakes: 1.0,
                    flaps: self.ct.flaps,
                    ..Default::default()
                };
                return self.ct;
            }
            Phase::Cruise => {}
        }
        // The landing waypoint: the caller turns it into a runway (`start_landing`).
        if self
            .way
            .get(self.cur)
            .is_some_and(|w| w.kind.starts_with("LANDING"))
            && self.landing_runway.is_some()
        {
            let (at, heading) = self.landing_runway.unwrap();
            self.phase = Phase::Landing { at, heading };
        }
        if self.reached(s.pos) && self.cur + 1 < self.way.len() {
            self.cur += 1;
        }
        let Some(wp) = self.target() else {
            return self.ct;
        };
        let w = &self.way[self.cur];
        if w.speed > 0.0 {
            // Wingmen fly a little slower so they can hold position.
            self.stab_speed = w.speed / 3.6 - 2.0 * self.index as f64;
        }
        self.stab_alt = wp.z;
        self.steer(s, wp, ground, terrain, dt)
    }

    /// Approach and landing (OpenIL2's own): fly to a gate 5 km out at 400 m above the runway,
    /// then down a 4° glide path with gear and flaps down at the faster of 1.5·VminFLAPS and the speed for a lift coefficient of 0.8; flare, cut the throttle
    /// and brake to a stop.
    fn land(
        &mut self,
        s: &Sim,
        ground: f64,
        terrain: &dyn Fn(f64, f64) -> f64,
        at: DVec3,
        heading: f64,
        dt: f64,
    ) -> Inputs {
        let dir = DVec3::new(heading.cos(), heading.sin(), 0.0);
        if let Some(p) = self.prev_at.replace(at) {
            if dt > 0.0 {
                self.at_vel = (at - p) / dt;
            }
        }
        let to = at - s.pos;
        let along = -to.truncate().dot(dir.truncate()); // < 0 before the threshold
        let dist = -along;
        let side = to.truncate().perp_dot(dir.truncate());
        // Near the touchdown point heights count from it (a carrier's deck stands above the sea).
        let ground = if (-3000.0..1500.0).contains(&along) {
            ground.max(at.z)
        } else {
            ground
        };
        let agl = s.pos.z - ground;
        // 1.25 × VminFLAPS (`vmin_flaps` holds 0.87 of it in m/s), and fast enough for a lift
        // coefficient of 1.1.
        let vlift = (2.0 * s.mass() * 9.81 / (1.225 * s.ac.wing_area.max(1.0) * 1.1)).sqrt();
        let vref = (1.25 / 0.87 * s.ac.vmin_flaps).max(vlift);
        if s.on_ground {
            // Roll-out: tail-draggers bring the stick back as they slow, nose-wheel types keep the
            // nose down.
            self.rollout = true;
            let nose_wheel =
                s.ac.gear
                    .iter()
                    .any(|g| g.kind == il2_fm::ContactKind::Castor && g.pos.x > 0.0);
            let v = (s.vel - s.gear.surface_vel).length();
            self.ct.throttle = 0.0;
            self.ct.brakes = if !s.on_ground {
                0.0
            } else if v < 0.75 * vref {
                1.0
            } else {
                0.3
            };
            let wv = s.wtrue + s.w;
            // The runtime's roll-out: nose-wheel types (gear pitch up to 5°) centre the stick;
            // tail-draggers keep pulling until they sit at their ground attitude.
            let pitch = (s.ori * DVec3::X).z.clamp(-1.0, 1.0).asin().to_degrees();
            self.ct.elevator = if s.ac.gear_pitch <= 5.0 {
                0.0
            } else {
                let mut e = self.ct.elevator;
                if pitch < s.ac.gear_pitch {
                    e += 1.5 * dt;
                }
                (e + 0.3 * wv.y).clamp(-1.0, 1.0)
            };
            let _ = nose_wheel;
            self.ct.aileron = (-0.03 * s.bank_deg() - 0.3 * wv.x).clamp(-1.0, 1.0);
            let f = (s.ori * DVec3::X).truncate().normalize_or_zero();
            self.ct.rudder = (2.0 * dir.truncate().perp_dot(f) + 0.8 * s.w.z).clamp(-1.0, 1.0);
            if v < 2.0 {
                self.phase = Phase::Parked;
            }
            return self.ct;
        }
        // Too far out, or floated far down the runway: start over.
        if (dist > 6000.0 || dist < -1200.0) && !(self.rollout && agl < 15.0) {
            self.on_final = false;
            self.rollout = false;
        }
        let f = (s.ori * DVec3::X).truncate().normalize_or_zero();
        let hdg = f.y.atan2(f.x);
        let wrap_rad = |a: f64| {
            (a + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
        };
        // Not lined up yet: go to the gate 5 km out; take the final once roughly lined up.
        if !self.on_final {
            let gate = at - dir * 5000.0 + DVec3::new(0.0, 0.0, 400.0);
            let near_gate = (gate - s.pos).truncate().length() < 1500.0;
            let lined_up = (2000.0..6000.0).contains(&dist)
                && side.abs() < 0.25 * dist
                && wrap_rad(heading - hdg).abs() < 60f64.to_radians();
            if !near_gate && !lined_up {
                self.stab_alt = gate.z;
                self.stab_speed = 1.6 * vref;
                return self.steer(s, gate, ground, terrain, dt);
            }
        }
        // Final: glide path, gear and flaps (and the hook for a carrier).
        self.on_final = true;
        self.ct.gear = 1.0;
        self.ct.flaps = 1.0;
        if self.landing_ship.is_some() {
            self.ct.arrestor = 1.0;
        }
        self.stab_speed = vref;
        // On a carrier the path ends with the wheels 1 m over the landing spot (no flare: the
        // aircraft flies onto the deck and the hook stops it).
        let ship = self.landing_ship.is_some();
        let wheel_h = -s
            .ac
            .gear
            .iter()
            .filter(|g| g.kind != il2_fm::ContactKind::Body)
            .map(|g| g.pos.z)
            .fold(0.0f64, f64::min);
        let glide = at.z
            + if ship { wheel_h + 1.0 } else { 0.0 }
            + (dist.max(0.0) * 4f64.to_radians().tan());
        self.stab_alt = glide;
        // Localizer: intercept the centre line at up to 30°, banking at most 20°.
        let mut ct = self.ct;
        let wv = s.wtrue + s.w;
        let bank = s.bank_deg();
        // Closer in the centre line is held tighter (a deck is narrow).
        let k_side = if ship {
            0.004 + 2.0 / dist.max(100.0)
        } else {
            0.004
        };
        let cmd = heading + (-side * k_side).clamp(-0.52, 0.52);
        let herr = wrap_rad(cmd - hdg).to_degrees();
        let bank_cmd = (-2.0 * herr).clamp(-20.0, 20.0);
        ct.aileron = (0.03 * (bank_cmd - bank) - 0.3 * wv.x).clamp(-1.0, 1.0);
        let aos = {
            let v = s.ori.inverse() * s.vel;
            v.y.atan2(v.x).to_degrees()
        };
        ct.rudder = (ct.rudder - aos * 0.04 * dt).clamp(-1.0, 1.0);
        let _ = terrain;
        // Vertical speed for the glide path, corrected towards it; in the flare, a sink rate easing
        // off with height.
        let v = s.vel.length();
        // Wheel height: the origin's height above the lowest wheel at level attitude.
        let wheels = -s
            .ac
            .gear
            .iter()
            .filter(|g| g.kind != il2_fm::ContactKind::Body)
            .map(|g| g.pos.z)
            .fold(0.0f64, f64::min);
        let agl_w = agl - wheels;
        let nose_wheel =
            s.ac.gear
                .iter()
                .any(|g| g.kind == il2_fm::ContactKind::Castor && g.pos.x > 0.0);
        // Touchdown attitude (the most the flare pitches up): three points for tail-draggers (main and tail wheel together), 6°
        // for nose-wheel types.
        let three_point = {
            let main =
                s.ac.gear
                    .iter()
                    .find(|g| g.kind == il2_fm::ContactKind::Wheel);
            let tail =
                s.ac.gear
                    .iter()
                    .find(|g| g.kind == il2_fm::ContactKind::Castor && g.pos.x < 0.0);
            match (main, tail) {
                (Some(m), Some(t)) if m.pos.x > t.pos.x => ((t.pos.z - m.pos.z)
                    / (m.pos.x - t.pos.x))
                    .atan()
                    .to_degrees(),
                _ => 10.0,
            }
        };
        let max_pitch = if nose_wheel {
            6.0
        } else {
            three_point.clamp(5.0, 16.0)
        };
        // Flare: level off about 1 m over the runway and hold off while the speed bleeds away,
        // the attitude rising to the touchdown attitude.
        let flare = !ship && (agl_w < 12.0 || self.rollout);
        let vz_t = if flare {
            -(0.3 + 0.3 * (agl_w - 0.5).max(0.0)).min(6.0)
        } else {
            {
                // Down the path at the closing speed over the (possibly moving) touchdown point.
                let closing = (s.vel - self.at_vel).dot(dir).max(0.0);
                (-closing * 4f64.to_radians().tan() + 0.15 * (glide - s.pos.z)).clamp(-6.0, 2.0)
            }
        };
        let e = vz_t - s.vel.z;
        // Cascade: the vertical speed error sets a pitch attitude (about 1° per m/s plus a learnt
        // trim attitude, learnt faster in the flare), which the elevator holds with rate damping.
        // Bouncing after touchdown, the pilot holds the touchdown attitude.
        let learn = if flare { 1.5 } else { 0.4 };
        self.pitch_i = (self.pitch_i + learn * e * dt).clamp(-8.0, max_pitch);
        let pitch_cmd = (self.pitch_i + 1.2 * e).clamp(-10.0, max_pitch);
        if agl < 10.0 && !ship {
            // The runtime's touchdown zone (with `constrain`): throttle closed, the stick eased
            // back while the nose is below max(gear pitch − 2°, 5°) under 7 m, with rate damping.
            let pitch = (s.ori * DVec3::X).z.clamp(-1.0, 1.0).asin().to_degrees();
            let mut el = self.ct.elevator;
            if agl < 7.0 && pitch < (s.ac.gear_pitch - 2.0).max(5.0) {
                el += 1.5 * dt;
            }
            el = (el + 0.05 * wv.y).clamp(-1.0, 1.0);
            ct.elevator = el;
            ct.throttle = 0.0;
            self.ct = ct;
            return ct;
        }
        let pitch = (s.ori * DVec3::X).z.clamp(-1.0, 1.0).asin().to_degrees();
        let pe = pitch_cmd - pitch;
        self.ev_i = (self.ev_i + 0.04 * pe * dt).clamp(-0.5, 0.85);
        ct.elevator = (self.ev_i + 0.05 * pe + 0.6 * wv.y).clamp(-0.6, 0.9);
        // Speed by throttle alone.
        let ve = vref - v;
        self.land_thr_i = (self.land_thr_i + 0.03 * ve * dt).clamp(0.0, 1.0);
        ct.throttle = (self.land_thr_i + 0.08 * ve).clamp(0.0, 1.0);
        if flare {
            ct.throttle = 0.0;
        }
        self.ct = ct;
        ct
    }

    /// The runtime's hand on a landing aircraft below 60 m (`docs/behavior/ai.md`, "Landing"):
    /// the attitude is kept between −5° and the gear pitch + 10°, and below 10 m, while faster than
    /// 0.6·VminFLAPS, sinking is damped (×0.94 per tick beyond 0.6 m/s, at most 2.5 m/s) and
    /// climbing too (×0.91 per tick). Call after each tick.
    pub fn constrain(&self, s: &mut Sim, ground: f64) {
        let Phase::Landing { at, heading } = self.phase else {
            return;
        };
        if !self.on_final {
            return;
        }
        let along = (s.pos - at)
            .truncate()
            .dot(glam::DVec2::new(heading.cos(), heading.sin()));
        let ground = if (-3000.0..1500.0).contains(&along) {
            ground.max(at.z)
        } else {
            ground
        };
        let agl = s.pos.z - ground;
        if agl > 60.0 {
            return;
        }
        let pitch = (s.ori * DVec3::X).z.clamp(-1.0, 1.0).asin().to_degrees();
        let max = s.ac.gear_pitch + 10.0;
        let fix = if pitch < -5.0 {
            -5.0 - pitch
        } else if pitch > max {
            max - pitch
        } else {
            0.0
        };
        if fix != 0.0 {
            s.ori = (s.ori * DQuat::from_rotation_y(-fix.to_radians())).normalize();
        }
        if s.vel.length() > 0.6 * s.ac.vmin_flaps && agl < 10.0 {
            let mut vz = s.vel.z;
            if self.landing_ship.is_some() {
                // On a carrier only hard sinking is softened.
                if vz < -5.5 {
                    vz *= 0.95;
                }
            } else {
                if vz < -0.6 {
                    vz *= 0.94;
                }
                if vz < -2.5 {
                    vz = -2.5;
                }
            }
            if vz > 0.0 {
                vz *= 0.91;
            }
            s.vel.z = vz;
        }
    }

    /// Station on a leader (OpenIL2's own formation keeping): aim at a point 300 m ahead of the
    /// slot, and match the leader's speed corrected by how far ahead or behind the slot we are.
    /// `slot` is the offset in the leader's heading frame (x forward, y left, z up).
    #[allow(clippy::too_many_arguments)]
    pub fn follow(
        &mut self,
        s: &Sim,
        lead_pos: DVec3,
        lead_vel: DVec3,
        slot: DVec3,
        ground: f64,
        terrain: &dyn Fn(f64, f64) -> f64,
        dt: f64,
    ) -> Inputs {
        let fwd = lead_vel.truncate().normalize_or(glam::DVec2::X);
        let left = glam::DVec2::new(-fwd.y, fwd.x);
        let station = lead_pos + (fwd * slot.x + left * slot.y).extend(slot.z);
        let rel = station - s.pos;
        let along = rel.truncate().dot(fwd);
        self.stab_speed =
            (lead_vel.length() + (0.05 * along).clamp(-15.0, 15.0)).max(s.ac.vmin_flaps * 1.1);
        self.stab_alt = station.z;
        let aim = station + (fwd * 300.0).extend(0.0);
        let mut ct = self.steer(s, aim, ground, terrain, dt);
        // Throttle with an integral term, so the slot is held without a standing error.
        let err = 0.02 * along + (lead_vel.length() - s.vel.length());
        self.thr_i = (self.thr_i + 0.02 * err * dt).clamp(0.0, 1.0);
        ct.throttle = (self.thr_i + 0.05 * err).clamp(0.0, 1.0);
        self.ct.throttle = ct.throttle;
        ct
    }

    /// Flies towards `wp` at `stab_alt` and `stab_speed` (the autopilot's control laws).
    fn steer(
        &mut self,
        s: &Sim,
        wp: DVec3,
        ground: f64,
        terrain: &dyn Fn(f64, f64) -> f64,
        dt: f64,
    ) -> Inputs {
        // OpenIL2's terrain floor: 150 m over the highest ground of the next 3 km of the track.
        let ahead = (s.vel.truncate().normalize_or_zero(), s.pos.truncate());
        let high = (0..=6)
            .map(|k| {
                let p = ahead.1 + ahead.0 * (500.0 * k as f64);
                terrain(p.x, p.y)
            })
            .fold(ground, f64::max);
        let alt_floor = if self.on_final {
            self.stab_alt.max(ground)
        } else {
            self.stab_alt.max(high + 150.0).max(ground + 5.0)
        };
        let speed = s.vel.length();
        let alt = s.pos.z;
        let tangage = pitch(s);
        let kren = s.bank_deg();
        let mass = s.mass();
        let ac = &s.ac;
        let vmin = ac.vmin_flaps;
        // Rates as the runtime's AI reads them, and their change.
        let wv = s.wtrue + s.w;
        let aw = (wv - self.prev_w) / dt.max(1e-3);
        self.prev_w = wv;
        let vx = (s.ori.inverse() * s.vel).x;
        // The runtime's forward accelerometer: aerodynamic and thrust force along x over weight,
        // that is the measured acceleration with gravity's share added back, in g.
        let forw_accel = (vx - self.prev_vx) / dt.max(1e-3) / 9.81 + pitch(s).to_radians().sin();
        self.prev_vx = vx;

        self.stab_dir = -(wp.y - s.pos.y).atan2(wp.x - s.pos.x).to_degrees();

        // Throttle holds the speed.
        self.ct.throttle = (0.3 - 0.04 * (speed - self.stab_speed)).clamp(0.0, 1.0);

        // OpenIL2: climbing far below the wanted height takes climb power.
        if alt_floor - alt > 100.0 {
            self.ct.throttle = self.ct.throttle.max(0.9);
        }
        // Elevator holds the altitude.
        let mut ev = self.ct.elevator;
        let dh = alt_floor - alt;
        let mut climb = 0.0;
        let mut descend = 0.0;
        if dh > -50.0 {
            let max_aoa = (5.0 + 2.5e-4 * alt + 0.02 * (250.0 - ac.vmax)).min(14.0);
            climb = (s.aoa - max_aoa).min(tangage - 1.0) * dt + 0.5 * forw_accel;
        }
        let min_pitch = (-15.0 + mass * 3.3e-4).min(-4.0);
        if dh < 50.0 {
            descend = (tangage - min_pitch) * 0.8 * dt;
        }
        let k = (0.01 * (dh + 50.0)).clamp(0.0, 1.0);
        ev -= k * climb + (1.0 - k) * descend;
        ev += wv.y + 0.5 * aw.y;
        if speed < 1.3 * vmin {
            ev -= 0.004 * dt;
        }
        let max_pitch = if vmin < 28.0 {
            10.0
        } else {
            (9.0 * speed / vmin).min(25.0)
        };
        ev = ev
            .min((max_pitch - tangage) * 0.1)
            .max((min_pitch - tangage) * 0.2);
        self.ct.elevator = 0.8 * self.ct.elevator + 0.2 * ev;

        // Ailerons turn towards the waypoint.
        let max_bank = (((speed - vmin) * 3.6 + s.vel.z * 40.0) * 0.25).clamp(15.0, 65.0);
        let err = wrap(azimut(s) - self.stab_dir).clamp(-max_bank, max_bank);
        // OpenIL2: a slow integral of small heading errors removes the standing offset the bank
        // needed against engine torque would leave.
        if err.abs() < 10.0 {
            self.hdg_i = (self.hdg_i + 0.5 * err * dt).clamp(-6.0, 6.0);
        }
        let ail =
            (-0.01 * (err + self.hdg_i + wrap(kren) + 3.0 * wv.x + 0.5 * aw.x)).clamp(-1.0, 1.0);
        let local = s.ori.inverse() * (wp - s.pos);
        self.ct.aileron = if local.y.abs() < 25.0 && local.x.abs() < 150.0 {
            -0.01 * kren
        } else {
            ail
        };
        self.ct.elevator += kren.abs() * 0.004 * dt;
        let aos = {
            let v = s.ori.inverse() * s.vel;
            v.y.atan2(v.x).to_degrees()
        };
        self.ct.rudder -= aos * 0.04 * dt;
        self.ct.elevator = self.ct.elevator.clamp(-1.0, 1.0);
        self.ct.rudder = self.ct.rudder.clamp(-1.0, 1.0);
        self.ct
    }
}

/// What an attacking pilot sees of its target.
pub struct Contact {
    pub pos: DVec3,
    pub vel: DVec3,
}

impl AiPilot {
    /// Lead pursuit of `t` (OpenIL2's own air-combat pilot, see `docs/behavior/ai.md`): roll the
    /// aim point into the lift plane and pull towards it. Returns the controls and whether the guns
    /// should fire.
    pub fn attack(
        &mut self,
        s: &Sim,
        t: &Contact,
        ground: f64,
        bullet_speed: f64,
    ) -> (Inputs, bool) {
        let rel = t.pos - s.pos;
        let range = rel.length();
        let tof = range / bullet_speed.max(100.0);
        // Where to point the guns: the target's future position, less our own motion the rounds
        // inherit, raised for the drop.
        let aim = rel + (t.vel - s.vel) * tof + DVec3::new(0.0, 0.0, 0.5 * 9.81 * tof * tof);
        let local = s.ori.inverse() * aim;
        let off = local.y.hypot(local.z).atan2(local.x).to_degrees();
        let alt = s.pos.z - ground;
        // Roll so the aim point lies above the nose (towards the lift).
        let roll_err = (-local.y)
            .atan2(local.z.max(1e-3) + local.x.max(0.0) * 0.05)
            .to_degrees();
        let roll_err = if local.x < 0.0 {
            roll_err.clamp(-90.0, 90.0)
        } else {
            roll_err
        };
        let wv = s.wtrue + s.w;
        let mut ail = 0.02 * roll_err - 0.3 * wv.x;
        // Pull in proportion to how far above the nose the aim point is.
        let up = local.z.atan2(local.x.abs().max(1.0)).to_degrees();
        let mut elev = (0.08 * up + 0.15).clamp(-0.3, 1.0);
        if local.x < 0.0 {
            elev = 1.0;
        }
        // Don't fly into the ground: level out and climb when low and descending.
        if alt < 300.0 && s.vel.z < 0.0 {
            ail = -0.03 * s.bank_deg();
            elev = 0.6;
        }
        self.ct.aileron = ail.clamp(-1.0, 1.0);
        self.ct.elevator = 0.6 * self.ct.elevator + 0.4 * elev;
        self.ct.rudder = (-0.05 * local.y.atan2(local.x.max(1.0)).to_degrees()).clamp(-0.5, 0.5);
        self.ct.throttle = 1.0;
        self.ct.gear = 0.0;
        // Better pilots fire from further and only with a good solution.
        let (cone, reach) = match self.skill {
            0 => (6.0, 350.0),
            1 => (4.0, 450.0),
            2 => (3.0, 550.0),
            _ => (2.5, 650.0),
        };
        (self.ct, off < cone && range < reach)
    }

    /// A hard turn away from an attacker behind.
    pub fn evade(&mut self, s: &Sim, threat: &Contact, ground: f64) -> Inputs {
        let local = s.ori.inverse() * (threat.pos - s.pos);
        // Turn towards the side the attacker is on, to spoil his deflection.
        let want_bank = if local.y > 0.0 { -75.0 } else { 75.0 };
        let wv = s.wtrue + s.w;
        self.ct.aileron = (0.03 * (want_bank - s.bank_deg()) - 0.3 * wv.x).clamp(-1.0, 1.0);
        self.ct.elevator = 0.8;
        if s.pos.z - ground < 300.0 && s.vel.z < 0.0 {
            self.ct.aileron = (-0.03 * s.bank_deg()).clamp(-1.0, 1.0);
            self.ct.elevator = 0.6;
        }
        self.ct.throttle = 1.0;
        self.ct.rudder = 0.0;
        self.ct
    }
}
