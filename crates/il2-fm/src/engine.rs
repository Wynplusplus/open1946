//! Piston engine and propeller: `docs/behavior/engine.md`.

use crate::atmosphere;
use il2_formats::sectfile::atof;
use il2_formats::SectFile;
use std::f64::consts::PI;

fn rad_s(rpm: f64) -> f64 {
    rpm * 2.0 * PI / 60.0
}

fn rpm(w: f64) -> f64 {
    w * 60.0 / (2.0 * PI)
}

#[derive(Clone, Debug)]
pub struct EngineParams {
    pub piston: bool,
    pub w_min: f64,
    pub w_nom: f64,
    pub w_max: f64,
    pub w_max_allowed: f64,
    pub reductor: f64,
    pub carburetor: i32,
    pub boost_factor: f64,
    pub acceleration: f64,
    pub engine_i: f64,
    pub prop_r: f64,
    pub prop_i: f64,
    pub prop_s: f64,
    pub m_max: f64,
    pub governor: i32,
    pub governor_speed: f64,
    pub gov_min: f64,
    pub gov_max: f64,
    pub phi_min: f64,
    pub phi_max: f64,
    pub v_opt: f64,
    pub compressor: i32,
    pub stage_pressure: Vec<f64>,
    pub stage_mult: Vec<f64>,
    pub stage_base: Vec<f64>,
    pub max_ata_rpm: f64,
    pub speed_manifold: f64,
    pub p_at0: f64,
    /// (RPM, ATA) points, sorted by RPM.
    pub ata: Vec<(f64, f64)>,
    /// +1 for `Direction Left`, −1 for `Right`.
    pub direction: f64,
}

impl EngineParams {
    /// Reads submodel `sub` of an engine file on top of its `[Generic]` section.
    pub fn load(emd: &SectFile, sub: &str) -> Option<Self> {
        let s = emd.section_ci(sub)?;
        let g = emd.section_ci("Generic");
        let raw = |k: &str| s.get_ci(k).or_else(|| g.and_then(|g| g.get_ci(k))).map(str::trim);
        let num = |k: &str, d: f64| raw(k).and_then(atof).map_or(d, f64::from);
        let ty = raw("Type").unwrap_or("Inline");
        let piston = ty.ends_with("Inline") || ty.ends_with("Radial");
        let rpm_max = num("RPMMax", 2200.0);
        let w_max = rad_s(rpm_max);
        let diameter = num("PropDiameter", 3.0);
        let prop_r = 0.5 * diameter * 0.75;
        let mut governor = num("PropAnglerType", 0.0) as i32;
        if governor > 255 {
            governor -= 256;
        }
        let gov_param = |k: &str| {
            let v = num(k, 0.0);
            match governor {
                5 | 6 => v.to_radians(),
                1 | 2 | 7 | 8 | 9 => rad_s(v),
                _ => v,
            }
        };
        let steps = (num("CompressorSteps", 1.0) as i32 - 1).max(0) as usize + 1;
        let p_ratio = |alt: f64| atmosphere::pressure(alt) / atmosphere::P0;
        let stage_pressure = (0..steps).map(|i| p_ratio(num(&format!("CompressorAltitude{i}"), 0.0))).collect();
        let stage_mult = (0..steps).map(|i| num(&format!("CompressorMultiplier{i}"), 0.0)).collect();
        let stage_base = (0..steps).map(|i| num(&format!("CompressorBaseMultiplier{i}"), 0.8)).collect();
        let max_ata_rpm = num("CompressorMaxATARPM", 1.45);
        let mut ata: Vec<(f64, f64)> = Vec::new();
        let mut insert = |r: f64, a: f64| {
            if ata.len() < 16 && !ata.iter().any(|p| p.0 == r) {
                let i = ata.partition_point(|p| p.0 < r);
                ata.insert(i, (r, a));
            }
        };
        if raw("CompressorRPMP0").is_some() {
            insert(num("CompressorRPMP0", 1500.0), 1.0);
        }
        if raw("CompressorMaxATARPM").is_some() {
            insert(rpm_max, max_ata_rpm);
        }
        if raw("CompressorRPMPMax").is_some() {
            insert(num("CompressorRPMPMax", 2600.0), num("CompressorPMax", 1.0));
        }
        for k in 0..16 {
            let (Some(r), Some(a)) = (raw(&format!("CompressorRPM{k}")), raw(&format!("CompressorATA{k}"))) else { break };
            insert(atof(r).unwrap_or(2000.0) as f64, atof(a).unwrap_or(1.0) as f64);
        }
        let hp = num("HorsePowers", 1200.0);
        Some(EngineParams {
            piston,
            w_min: rad_s(num("RPMMin", 200.0)),
            w_nom: rad_s(num("RPMNom", 2000.0)),
            w_max,
            w_max_allowed: raw("RPMMaxAllowed").and_then(atof).map_or(250.0, |r| rad_s(r as f64)),
            reductor: num("Reductor", 1.0),
            carburetor: num("Carburetor", 0.0) as i32,
            boost_factor: num("BoostFactor", 1.0),
            acceleration: num("EngineAcceleration", 1.0),
            engine_i: num("EngineI", 1.0),
            prop_r,
            prop_i: num("PropMass", 30.0) * diameter * diameter * 0.083,
            prop_s: 0.26 * prop_r * prop_r,
            m_max: hp * 746.0 * 1.2 / w_max,
            governor,
            governor_speed: num("PropAnglerSpeed", 0.1),
            gov_min: gov_param("PropAnglerMinParam"),
            gov_max: gov_param("PropAnglerMaxParam"),
            phi_min: num("PropPhiMin", 10.0).to_radians(),
            phi_max: num("PropPhiMax", 29.0).to_radians(),
            v_opt: num("Voptimal", 324.0) / 3.6,
            compressor: num("CompressorType", 0.0) as i32,
            stage_pressure,
            stage_mult,
            stage_base,
            max_ata_rpm,
            speed_manifold: num("CompressorSpeedManifold", 0.2),
            p_at0: num("CompressorPAt0", 0.3),
            ata,
            direction: if raw("Direction").is_some_and(|d| d.ends_with("Right")) { -1.0 } else { 1.0 },
        })
    }

    /// Highest throttle setting: 1.1 (110 %) with boost, else 1.
    pub fn max_throttle(&self) -> f64 {
        if self.boost_factor > 1.0 {
            1.1
        } else {
            1.0
        }
    }

    /// ATA curve value at `r` RPM.
    fn ata(&self, r: f64, h: f64) -> f64 {
        let p = &self.ata;
        if p.len() < 2 {
            return 1.0;
        }
        if r < 0.1 {
            return atmosphere::pressure(h) / atmosphere::P0;
        }
        if r >= p[p.len() - 1].0 {
            return p[p.len() - 1].1;
        }
        let i = if r < p[0].0 { 0 } else { (0..p.len() - 1).find(|&i| p[i].0 <= r && r < p[i + 1].0).unwrap_or(0) };
        let span = (p[i + 1].0 - p[i].0).max(0.001);
        p[i].1 + (r - p[i].0) * (p[i + 1].1 - p[i].1) / span
    }
}

fn fan_cy(a: f64) -> f64 {
    let mut a = a.clamp(-8.0, 34.0);
    if a < 16.0 {
        -0.004688 * a * a + 0.15 * a + 0.4
    } else {
        let extra = if a > 22.0 { 0.01 * (a - 22.0) } else { 0.0 };
        a = a.min(22.0);
        9.7222e-4 * a * a - 0.070833 * a + 2.4844 + extra
    }
}

fn fan_cx(a: f64) -> f64 {
    let mut a = if a < -4.0 { -8.0 - a } else { a };
    a = a.min(34.0);
    if a < 16.0 {
        3.5e-4 * a * a + 0.0028 * a + 0.0256
    } else {
        let extra = if a > 22.0 { 0.04 * (a - 22.0) } else { 0.0 };
        a = a.min(22.0);
        -0.00555 * a * a + 0.24444 * a - 2.32888 + extra
    }
}

/// Dynamic state of one engine.
#[derive(Clone, Debug)]
pub struct EngineState {
    pub w: f64,
    pub aw: f64,
    pub phi: f64,
    pub phi_target: f64,
    pub second_throttle: f64,
    pub thrust: f64,
    /// Torque reaction about body x (N·m).
    pub torque: f64,
    pub prop_torque: f64,
    pub manifold: f64,
}

impl EngineState {
    pub fn new(p: &EngineParams, running_at: f64) -> Self {
        let phi = 11f64.to_radians().clamp(p.phi_min, p.phi_max);
        EngineState { w: running_at, aw: 0.0, phi, phi_target: phi, second_throttle: 1.0, thrust: 0.0, torque: 0.0, prop_torque: 0.0, manifold: 1.0 }
    }

    /// Advances the engine by `dt`. `v`: airspeed along body x; `h`: altitude; `speed`: total airspeed.
    pub fn step(&mut self, p: &EngineParams, throttle: f64, v: f64, speed: f64, h: f64, dt: f64) {
        // Throttle above 1 (up to 1.1) is boost, only for engines with BoostFactor > 1.
        let throttle = throttle.clamp(0.0, p.max_throttle());
        let t = throttle.min(1.0);
        let rho = atmosphere::density(h);
        let pe = (atmosphere::pressure(h) + p.speed_manifold * 0.5 * rho * speed * speed) / atmosphere::P0;

        // 2. Propeller forces with the current pitch.
        let wp = self.w * p.reductor;
        let phi_w = if self.w.abs() < 1e-5 { PI / 2.0 } else { (v / (wp * p.prop_r)).atan() };
        let alpha = (self.phi - phi_w).to_degrees();
        let u = wp * p.prop_r;
        let w2 = v * v + u * u;
        let wl = w2.sqrt().max(0.001);
        let lift = 0.5 * fan_cy(alpha) * rho * w2 * p.prop_s;
        let mut drag = 0.5 * fan_cx(alpha) * rho * w2 * p.prop_s;
        if wl > 300.0 {
            drag *= (1.0 + 0.02 * (wl - 300.0)).min(2.0);
        }
        let k = if v < p.v_opt { 1.0 - 5e-5 * (p.v_opt - v).powi(2) } else { 1.0 };
        let thrust = k * (lift * u - drag * v) / wl;
        let q = (drag * u + lift * v) / wl * p.prop_r;

        // 3. Governor.
        let threshold = match p.governor {
            1 | 2 | 7 | 8 => {
                let lever = if p.boost_factor > 1.0 { 0.75 + 0.227272 * throttle } else { 0.75 + 0.25 * throttle };
                let wt = p.gov_min + (p.gov_max - p.gov_min) * lever;
                let rate = p.governor_speed * dt;
                if self.w < wt {
                    self.phi_target -= (0.01 * (wt - self.w) - 0.012 * self.aw).min(1.0) * rate;
                } else {
                    self.phi_target += (0.01 * (self.w - wt) + 0.012 * self.aw).min(1.0) * rate;
                }
                if self.phi_target < phi_w - 0.12 {
                    self.phi_target = phi_w - 0.12;
                    if self.phi < self.phi_target {
                        self.phi += 0.2 * dt;
                    }
                }
                p.ata(rpm(p.gov_min + (p.gov_max - p.gov_min) * t), h)
            }
            0 => {
                self.phi_target = 0.0;
                p.max_ata_rpm * (0.55 + 0.45 * t)
            }
            _ => p.max_ata_rpm * (0.55 + 0.45 * t),
        };
        let step = p.governor_speed * dt;
        let diff = self.phi - self.phi_target;
        self.phi -= diff.signum() * (157.3 * diff.abs()).min(1.0) * step;
        self.phi_target = self.phi_target.clamp(p.phi_min, p.phi_max);
        self.phi = self.phi.clamp(p.phi_min, p.phi_max);

        // 4. Supercharger.
        let rpm_factor = p.p_at0 + (1.0 - p.p_at0) * self.w / p.w_max;
        let c = match p.compressor {
            0 => (atmosphere::pressure(h) + 0.5 * rho * speed * speed) / atmosphere::P0,
            2 => {
                let pr = &p.stage_pressure;
                let rho_term = |pc: f64| pe * (p.p_at0 + (1.0 - p.p_at0)) * self.w / p.w_max * p.max_ata_rpm / pc + 0.001;
                let (stage, k) = if p.stage_pressure.len() == 2 && rho_term(pr[0]) < threshold {
                    let pc = pr[1];
                    if rho_term(pc) > threshold {
                        let f = if p.stage_mult[1] >= 1.0 { p.stage_base[1] } else { p.stage_base[1] * p.stage_mult[1] };
                        let mut k = f + (1.0 - pe) / (1.0 - pc).max(1e-4) * (p.stage_mult[1] - f);
                        k += (p.stage_mult[0] - k) * (rho_term(pc) - 1.0) / (pr[0] * threshold / pc - 1.0);
                        (1, k)
                    } else {
                        (1, p.stage_mult[1])
                    }
                } else {
                    (0, 1.0 + (1.0 - pe) / (1.0 - pr[0]).max(1e-4) * (p.stage_mult[0] - 1.0))
                };
                let mut m = rpm_factor * pe * p.max_ata_rpm / pr[stage];
                self.second_throttle = (p.max_ata_rpm / m.max(1e-6)).min(1.0);
                m *= self.second_throttle;
                m *= (threshold / p.max_ata_rpm).min(1.0);
                self.manifold = m;
                k * m / threshold.max(1e-6)
            }
            _ => {
                let mut best = (-1.0, 0.0, 0usize);
                for i in 0..p.stage_pressure.len() {
                    let pc = p.stage_pressure[i];
                    let (ki, score) = if pe <= pc {
                        let ki = p.stage_mult[i];
                        (ki, ki * pe * rpm_factor / pc)
                    } else {
                        let f = if p.stage_mult[i] >= 1.0 { p.stage_base[i] } else { p.stage_base[i] * p.stage_mult[i] };
                        let base = f.powi(i as i32);
                        let ki = base + (1.0 - pe) / (1.0 - pc).max(1e-4) * (p.stage_mult[i] - base);
                        (ki, ki)
                    };
                    if score > best.0 {
                        best = (score, ki, i);
                    }
                }
                let (_, ki, i) = best;
                let mut m = rpm_factor * pe * p.max_ata_rpm / p.stage_pressure[i];
                let target = p.max_ata_rpm / m.max(1e-6);
                self.second_throttle = (self.second_throttle - 3.0 * dt * (self.second_throttle - target)).min(1.0);
                m *= self.second_throttle;
                m *= (threshold / p.max_ata_rpm).min(1.0);
                self.manifold = m;
                ki * m / threshold.max(1e-6)
            }
        };

        // 5. Engine torque.
        let torque_curve = match p.carburetor {
            0 => {
                let f = 0.05 + 0.95 * t;
                let x = self.w / p.w_max;
                p.m_max * (-x * x / f + 2.0 * x)
            }
            3 => {
                let f = (0.1 + 0.9 * t).min(1.0);
                let x = self.w / p.w_nom;
                p.m_max * (-x * x / f + 2.0 * x)
            }
            _ => {
                let f = (0.1 + 0.9 * t).min(1.0);
                let x = self.w / (p.w_nom * f);
                p.m_max * (-0.5 * f * f + f + 0.5) * (2.0 * x - x * x)
            }
        };
        let boost = if throttle > 1.0 {
            match p.carburetor {
                0 | 3 => p.boost_factor,
                _ => 1.0 + (throttle - 1.0) * 10.0 * (p.boost_factor - 1.0),
            }
        } else {
            1.0
        };
        let torque_curve = (torque_curve * boost).max(-0.8 * self.w / p.w_max * p.m_max);
        let m = torque_curve * c;

        // 6. Rotation.
        let mut aw = (m - q) / (p.prop_i + p.engine_i);
        if aw > 0.0 {
            aw *= p.acceleration;
        }
        self.aw = aw;
        self.w = (self.w + aw * dt).clamp(0.0, 2.0 * p.w_max_allowed);

        // 7. Forces on the airframe.
        self.thrust = thrust;
        self.prop_torque = q;
        self.torque = p.direction * (q + p.prop_i * aw * p.reductor);
    }

    pub fn rpm(&self) -> f64 {
        rpm(self.w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blade_coefficients_are_continuous() {
        for a in [16.0, 22.0] {
            assert!((fan_cy(a - 1e-6) - fan_cy(a + 1e-6)).abs() < 0.02, "Cy jump at {a}");
            assert!((fan_cx(a - 1e-6) - fan_cx(a + 1e-6)).abs() < 0.02, "Cx jump at {a}");
        }
    }

    #[test]
    fn engine_reaches_steady_rpm() {
        let emd = SectFile::parse(
            b"[Generic]\nType Inline\n[E]\nHorsePowers 1400\nRPMMin 500\nRPMNom 2800\nRPMMax 2800\nReductor 0.72\n\
              PropDiameter 3.12\nPropAnglerType 2\nPropAnglerSpeed 0.08\nPropAnglerMinParam 1650\nPropAnglerMaxParam 2800\n\
              PropPhiMin 16\nPropPhiMax 55\nCompressorType 1\nCompressorSteps 1\nCompressorPMax 1.42\nCompressorAltitude0 6200\n\
              CompressorMultiplier0 0.86\nCompressorRPMP0 2000\nCompressorRPMPMax 2800\nCompressorMaxATARPM 1.42\n",
        );
        let p = EngineParams::load(&emd, "E").unwrap();
        let mut s = EngineState::new(&p, p.w_nom);
        for _ in 0..2000 {
            s.step(&p, 1.0, 100.0, 100.0, 500.0, 0.03);
        }
        assert!((s.rpm() - 2800.0).abs() < 150.0, "rpm {}", s.rpm());
        assert!(s.thrust > 5000.0, "thrust {}", s.thrust);
    }
}
