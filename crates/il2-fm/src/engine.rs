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

/// `Type` of an engine (`docs/behavior/engine.md`, "Engine types").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineKind {
    /// `Inline`, `Radial`.
    Piston,
    /// `HeloI`: piston, propeller sees the airspeed's magnitude (tail-sitters).
    PistonHelo,
    Jet,
    Rocket,
    /// A rocket booster burning the separate `[Mass] Nitro` tank.
    RocketBoost,
    /// Tow string of a glider: thrust only while towed.
    Tow,
    /// `PVRD`: pulse jet / ramjet, also burning the `Nitro` tank.
    Pvrd,
}

#[derive(Clone, Debug)]
pub struct EngineParams {
    pub kind: EngineKind,
    pub piston: bool,
    /// `Thrust` (kgf) in newtons.
    pub thrust_max: f64,
    pub horse_powers: f64,
    pub cylinders: f64,
    /// Radians.
    pub prop_aoa0: f64,
    /// Jets: the fan's equivalent area and the drag torque at full RPM.
    pub jet_s: f64,
    pub jet_m_max: f64,
    /// Specific fuel use at power 0, 0.5, 1 and maximum (kg per hp·h, or per N·h for jets and pulse jets).
    pub fuel_curve: [f64; 4],
    /// Shaft speed for boost (`PropAnglerAfterburnerParam`), rad/s; `w_max` when not given.
    pub w_wep: f64,
    /// `AfterburnerType`: emergency boost system (0 none).
    pub afterburner: i32,
    /// Temperature model (`TESPEED`, `TWATERMAXRPM`, `TOILINMAXRPM`, `TOILOUTMAXRPM`, `MAXRPMTIME`,
    /// `TWATERMAX`, `TOILMAX`, `TWATERMIN`, `TOILMIN`), °C and seconds.
    pub t_speed: f64,
    pub t_water_max_rpm: f64,
    pub t_oil_in_max_rpm: f64,
    pub t_oil_out_max_rpm: f64,
    pub time_overheat: f64,
    pub t_water_crit: f64,
    pub t_oil_crit: f64,
    pub t_water_min: f64,
    pub t_oil_min: f64,
    /// `Autonomous 0`: needs ground equipment (near an airfield, on the ground) to start.
    pub autonomous: bool,
    pub radiator_mode: RadiatorMode,
    /// `WEPBoostFactor`: torque factor with boost engaged (1).
    pub wep_boost: f64,
    /// Boost ATA over normal ATA at the boost point of the ATA curve.
    pub wep_compressor: f64,
    /// Governor target at full lever with boost (`PropAnglerAfterburnerParam`).
    pub gov_max_wep: f64,
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
    /// `CompressorPMax` (rockets use it for their altitude term).
    pub compressor_p_max: f64,
    /// Rockets: kg/s at full throttle (2.5777; the runtime uses 1.8 for BI-1/BI-6 and 0.5 for MXY-7).
    pub rocket_burn: f64,
}

impl EngineParams {
    /// Reads submodel `sub` of an engine file on top of its `[Generic]` section.
    pub fn load(emd: &SectFile, sub: &str) -> Option<Self> {
        let s = emd.section_ci(sub)?;
        let g = emd.section_ci("Generic");
        let raw = |k: &str| s.get_ci(k).or_else(|| g.and_then(|g| g.get_ci(k))).map(str::trim);
        let num = |k: &str, d: f64| raw(k).and_then(atof).map_or(d, f64::from);
        let ty = raw("Type").unwrap_or("Inline");
        let kind = if ty.ends_with("Inline") || ty.ends_with("Radial") {
            EngineKind::Piston
        } else if ty.ends_with("Jet") {
            EngineKind::Jet
        } else if ty.ends_with("RocketBoost") {
            EngineKind::RocketBoost
        } else if ty.ends_with("Rocket") {
            EngineKind::Rocket
        } else if ty.ends_with("Tow") {
            EngineKind::Tow
        } else if ty.ends_with("PVRD") {
            EngineKind::Pvrd
        } else if ty.ends_with("HeloI") {
            EngineKind::PistonHelo
        } else {
            return None;
        };
        let piston = matches!(kind, EngineKind::Piston | EngineKind::PistonHelo);
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
        // Boost types keep their boost pressure as the last ATA point; it is taken out of the normal curve.
        let afterburner = num("AfterburnerType", 0.0) as i32;
        let mut wep_compressor = 1.0;
        if matches!(afterburner, 1 | 4..=12) && !ata.is_empty() {
            let (r, a) = ata.pop().unwrap();
            if ata.len() >= 2 {
                let normal = ata_at(&ata, r, 0.0);
                wep_compressor = a / normal;
            }
        }
        let hp = num("HorsePowers", 1200.0);
        let thrust_max = num("Thrust", 10.7 / 9.81) * 9.81;
        let cylinders = num("Cylinders", 12.0);
        let prop_aoa0 = num("PropAoA0", 11.0).to_radians();
        // Jets: the fan is sized so it would give twice the rated thrust at full RPM; the runtime passes
        // the angle in radians to a function that expects degrees, which is kept here.
        let jet_s = cylinders * cylinders * 2.0 * thrust_max / (fan_cy(prop_aoa0) * atmosphere::RHO0 * w_max * w_max * prop_r * prop_r);
        let jet_m_max = 0.5 * fan_cx(prop_aoa0.to_degrees()) * atmosphere::RHO0 * (w_max * prop_r).powi(2) * jet_s * prop_r;
        let default_fuel = match kind {
            EngineKind::Jet => [0.075, 0.075, 0.1, 0.11],
            EngineKind::Pvrd => [0.835; 4],
            _ => [0.4, 0.24, 0.28, 0.3],
        };
        let fuel_curve = [
            num("FuelConsumptionP0", default_fuel[0]),
            num("FuelConsumptionP05", default_fuel[1]),
            num("FuelConsumptionP1", default_fuel[2]),
            num("FuelConsumptionPMAX", default_fuel[3]),
        ];
        Some(EngineParams {
            kind,
            piston,
            thrust_max,
            horse_powers: hp,
            cylinders,
            prop_aoa0,
            jet_s,
            jet_m_max,
            fuel_curve,
            afterburner,
            t_speed: num("TESPEED", 0.0),
            t_water_max_rpm: num("TWATERMAXRPM", 0.0),
            t_oil_in_max_rpm: num("TOILINMAXRPM", 0.0),
            t_oil_out_max_rpm: num("TOILOUTMAXRPM", 0.0),
            time_overheat: num("MAXRPMTIME", 0.0),
            t_water_crit: num("TWATERMAX", 0.0),
            t_oil_crit: num("TOILMAX", 0.0),
            t_water_min: num("TWATERMIN", 0.0),
            t_oil_min: num("TOILMIN", 0.0),
            autonomous: num("Autonomous", 1.0) != 0.0,
            radiator_mode: RadiatorMode::Governor,
            wep_boost: num("WEPBoostFactor", 1.0),
            wep_compressor,
            gov_max_wep: if raw("PropAnglerAfterburnerParam").is_some() { gov_param("PropAnglerAfterburnerParam") } else { gov_param("PropAnglerMaxParam") },
            w_wep: raw("PropAnglerAfterburnerParam").and_then(atof).map_or(w_max, |r| rad_s(r as f64)),
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
            compressor_p_max: num("CompressorPMax", 1.0),
            rocket_burn: 2.5777,
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
        ata_at(&self.ata, r, h)
    }

    /// Whether boost is engaged for these controls (`docs/behavior/engine.md`, "Emergency boost").
    pub fn wep_active(&self, throttle: f64, wep: bool, nitro_left: bool) -> bool {
        match self.afterburner {
            0 => false,
            4 => throttle > 1.0 && nitro_left,
            8 | 10 => throttle > 1.0,
            1 => wep && throttle > 1.0 && nitro_left,
            _ => wep,
        }
    }

    /// Nitro use while boost is engaged, kg/s.
    pub fn wep_nitro_rate(&self) -> f64 {
        if matches!(self.afterburner, 1 | 2 | 4 | 5 | 9) {
            0.044872
        } else {
            0.0
        }
    }
}

fn ata_at(p: &[(f64, f64)], r: f64, h: f64) -> f64 {
    {
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
    /// Fuel use, kg/s, from the main tank and from the `Nitro` tank.
    pub fuel_rate: f64,
    pub nitro_rate: f64,
    /// 0 off, 1–5 starting, 6 running, 7 dead, 8 seized (`docs/behavior/engine.md`, "Engine management").
    pub stage: u8,
    stage_time: f64,
    stage_len: f64,
    /// Has run before (a warm restart may fail).
    ran: bool,
    friction: f64,
    pub t_oil_in: f64,
    pub t_oil_out: f64,
    pub t_water: f64,
    /// Radiator flap, 0 closed .. 1 open; automatic unless the pilot sets it.
    pub radiator: f64,
    pub radiator_manual: Option<f64>,
    /// Engine condition 0..1 (multiplies torque).
    pub readyness: f64,
    pub cylinders_out: f64,
    /// Oil system damage 0..1 (raises the oil temperature target by up to 35 %).
    pub oil_damage: f64,
    rng: u64,
    /// Malfunctions that happened, for messages (`overheat`, `cylinder`, `oil`, `fire`, …).
    pub events: Vec<&'static str>,
}

/// How the automatic radiator works; the runtime picks it by aircraft class.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RadiatorMode {
    /// By propeller governor type.
    #[default]
    Governor,
    /// No radiator control (Gladiator, Fokker D.XXI).
    Closed,
    /// Opens with oil temperature, else closes with speed (P-51, P-38, Yak-3/9 late, P-63).
    OilThermostat,
    /// Follows coolant temperature (Spitfire VIII and IX).
    WaterThermostat,
}

/// Engine conditions the management needs each step.
pub struct EngineEnv {
    pub dt: f64,
    pub speed: f64,
    pub altitude: f64,
    /// On the ground within 1200 m of an airfield (for non-autonomous starts).
    pub at_airfield: bool,
    /// The aircraft's `VmaxH` reference, m/s (radiator schedules).
    pub vmax_h: f64,
    /// Difficulty option "engine overheat": malfunctions from excess temperature.
    pub overheat: bool,
}

impl EngineState {
    pub fn new(p: &EngineParams, running_at: f64) -> Self {
        let phi = 11f64.to_radians().clamp(p.phi_min, p.phi_max);
        let w = match p.kind {
            // Jets idle at the torque balance of the minimum throttle setting.
            EngineKind::Jet => running_at.min(p.w_max),
            _ => running_at,
        };
        EngineState {
            w,
            aw: 0.0,
            phi,
            phi_target: phi,
            second_throttle: 1.0,
            thrust: 0.0,
            torque: 0.0,
            prop_torque: 0.0,
            manifold: 1.0,
            fuel_rate: 0.0,
            nitro_rate: 0.0,
            stage: 6,
            stage_time: 0.0,
            stage_len: 0.0,
            ran: true,
            friction: 0.0,
            t_oil_in: 0.5 * (p.t_oil_min + p.t_oil_in_max_rpm),
            t_oil_out: 0.5 * (p.t_oil_min + p.t_oil_out_max_rpm),
            t_water: 0.5 * (p.t_water_min + p.t_water_max_rpm),
            radiator: 0.0,
            radiator_manual: None,
            readyness: 1.0,
            cylinders_out: 0.0,
            oil_damage: 0.0,
            rng: 0x9E37_79B9_7F4A_7C15 ^ (p.horse_powers.to_bits() ^ p.thrust_max.to_bits()),
            events: Vec::new(),
        }
    }

    fn rand(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }

    fn rand_range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.rand()
    }

    /// A cold, stopped engine (at ambient temperature `ambient` °C).
    pub fn cold(p: &EngineParams, ambient: f64) -> Self {
        let mut s = Self::new(p, 0.0);
        s.stage = 0;
        s.ran = false;
        s.t_oil_in = ambient;
        s.t_oil_out = ambient;
        s.t_water = ambient;
        s
    }

    pub fn running(&self) -> bool {
        self.stage == 6
    }

    /// Starts a stopped engine, or stops a running one.
    pub fn toggle(&mut self, p: &EngineParams, env: &EngineEnv) {
        match self.stage {
            0 => {
                if !p.autonomous && !env.at_airfield && p.piston {
                    return;
                }
                // Rockets and pulse jets light at once.
                self.stage = if matches!(p.kind, EngineKind::Rocket | EngineKind::RocketBoost | EngineKind::Pvrd) { 5 } else { 1 };
                self.stage_time = 0.0;
                self.stage_len = 0.0;
            }
            1..=6 => self.stage = 0,
            _ => {}
        }
    }

    fn set_stage(&mut self, s: u8) {
        self.stage = s;
        self.stage_time = 0.0;
        self.stage_len = 0.0;
    }

    /// Start-up sequence and the shaft speed it imposes; `None` once running or stopped.
    fn start_sequence(&mut self, p: &EngineParams, dt: f64) -> Option<f64> {
        if !(1..=5).contains(&self.stage) {
            return None;
        }
        let first = self.stage_len == 0.0;
        let cold = (p.t_oil_out_max_rpm - self.t_oil_out) / (p.t_oil_out_max_rpm - 20.0).max(1.0);
        if first {
            self.stage_len = match self.stage {
                1 => 0.5 * self.rand_range(1.0, 2.0),
                2 if self.ran => {
                    let l = 0.1 + cold * 7.9 * self.rand_range(2.0, 4.2);
                    if l > 9.0 { self.rand_range(7.8, 9.6) } else { l }
                }
                2 => 4.0 * self.rand_range(1.0, 2.0),
                3 => 0.05 * self.rand_range(1.0, 2.0),
                _ => 0.5 * self.rand_range(1.0, 2.0),
            };
            if self.stage == 2 && self.ran && self.rand() < 0.5 {
                self.set_stage(0);
                return None;
            }
            if self.stage == 3 && self.rand() < 0.12 && cold < 0.75 {
                self.set_stage(0);
                return None;
            }
            if self.stage == 5 && self.ran && p.piston && self.rand() < 0.1 {
                self.set_stage(0);
                return None;
            }
            if self.stage == 5 {
                self.ran = true;
            }
        }
        let f = (self.stage_time / self.stage_len).min(1.0);
        let w = 0.1047 * match self.stage {
            1 => 20.0 * f,
            2 => 20.0 + 15.0 * f,
            3 => 60.0 + 60.0 * f,
            4 => 120.0,
            _ => 120.0 + 120.0 * f,
        };
        self.stage_time += dt;
        if self.stage_time > self.stage_len {
            let next = self.stage + 1;
            self.set_stage(next);
        }
        Some(w)
    }

    /// Temperatures, radiator, overheat damage; call once per step after the forces.
    pub fn manage(&mut self, p: &EngineParams, throttle: f64, wep: bool, env: &EngineEnv) {
        if !p.piston && p.kind != EngineKind::Jet {
            return;
        }
        let dt = env.dt;
        let kmh = env.speed.max(0.0) * 3.6;
        let ambient = atmosphere::temperature(env.altitude) - 273.15;
        let rho = atmosphere::density(env.altitude);
        // Radiator (automatic schedules by governor type).
        let by_speed = (1.0 - env.speed / env.vmax_h.max(1.0)).max(0.0);
        self.radiator = match (self.radiator_manual, p.radiator_mode) {
            (Some(r), _) => r.clamp(0.0, 1.0),
            (None, RadiatorMode::Closed) => 0.0,
            (None, RadiatorMode::OilThermostat) => {
                if self.t_oil_out > p.t_oil_out_max_rpm {
                    (self.radiator + 0.1 * dt).min(1.0)
                } else {
                    by_speed
                }
            }
            (None, RadiatorMode::WaterThermostat) => ((self.t_water - p.t_water_crit + 20.0) / 20.0).clamp(0.0, 1.0),
            (None, RadiatorMode::Governor) => {
                let power = if self.running() { throttle.min(1.1) * self.readyness } else { 0.0 };
                match p.governor {
                    1 | 2 => {
                        if self.radiator > 1.0 - power {
                            (self.radiator - 0.15 * dt).max(0.0)
                        } else {
                            self.radiator + 0.15 * dt
                        }
                    }
                    5 | 6 => (1.0 - env.speed / env.vmax_h.max(1.0)).max(0.0),
                    7 | 8 => {
                        if self.t_oil_out > p.t_oil_out_max_rpm {
                            (self.radiator + 0.1 * dt).min(1.0)
                        } else {
                            (1.0 - env.speed / env.vmax_h.max(1.0)).max(0.0)
                        }
                    }
                    _ => 1.0 - power,
                }
                .clamp(0.0, 1.0)
            }
        };
        let r = self.radiator;
        let mut t = throttle;
        if p.boost_factor > 1.0 {
            t = (t + (t - 1.0).max(0.0) * (p.boost_factor - 1.1) * 10.0) / p.boost_factor;
        }
        if !wep && p.wep_boost > 1.0 && !matches!(p.afterburner, 1 | 4 | 9) {
            t /= p.wep_boost;
        }
        let x = self.w / p.w_wep.max(1e-6);
        let speed_cool = 0.8 + 60.0 / (50.0 + kmh);
        if self.running() {
            let target = ambient
                + p.t_oil_out_max_rpm * (1.0 - 0.15 * r) * (0.65 + 0.45 / rho) * (1.0 + 0.35 * self.oil_damage) * 1.2 * x * x * (0.7 + 0.3 * t) * speed_cool;
            self.t_oil_out += (target - self.t_oil_out) * dt * p.t_speed;
            let oil_in = self.t_oil_out * 0.8;
            self.t_oil_in += (oil_in - self.t_oil_in) * dt * 0.5;
            let k7 = if p.kind == EngineKind::Jet { 0.35 + 0.1 / rho } else { 0.45 / rho };
            let target = ambient + p.t_water_max_rpm * (1.0 - 0.15 * r) * (0.65 + k7) * (0.6 + 0.4 * x) * (0.1 + 1.1 * t) * speed_cool;
            self.t_water += (target - self.t_water) * dt * p.t_speed;
        } else {
            let k = dt * p.t_speed * (0.2 + 0.2 * r);
            self.t_oil_out += (ambient - self.t_oil_out) * k;
            self.t_oil_in += (self.t_oil_out - self.t_oil_in) * dt * 0.5;
            self.t_water += (ambient - self.t_water) * k;
        }
        // Overheat: random malfunctions growing with the cube of the excess.
        if env.overheat && p.time_overheat > 0.0 && self.stage < 7 {
            for (temp, crit, water) in [(self.t_water, p.t_water_crit, true), (self.t_oil_out, p.t_oil_crit, false)] {
                if crit <= 0.0 || temp <= crit {
                    continue;
                }
                let e = temp / crit - 1.0;
                let risk = e * e * e * dt * 1000.0;
                if risk > self.rand() * p.time_overheat && self.stage < 7 {
                    self.malfunction(p, water);
                }
            }
        }
    }

    fn knock_out(&mut self, p: &EngineParams, n: f64) {
        self.cylinders_out = (self.cylinders_out + n).min(p.cylinders);
        let ratio = 1.0 - self.cylinders_out / p.cylinders.max(1.0);
        if ratio < 0.12 {
            self.set_stage(7);
            self.events.push("dead");
        } else if ratio < self.readyness {
            self.readyness = ratio;
        }
    }

    fn malfunction(&mut self, p: &EngineParams, water: bool) {
        self.events.push("overheat");
        if !p.piston {
            if self.rand() < 0.2 {
                self.events.push("smoke");
                self.readyness *= 0.9;
            } else {
                let n = (self.rand() * 4.0).floor();
                self.knock_out(p, n);
            }
            return;
        }
        let roll = (self.rand() * 100.0) as u32;
        // Water overheat: mostly cylinders; oil overheat: mostly oil and bearings.
        let (cyl, oil, bearing, fire, charger, dead) = if water { (50, 65, 94, 95, 97, 98) } else { (10, 60, 94, 95, 96, 97) };
        if roll < cyl {
            self.knock_out(p, 1.0);
            self.events.push("cylinder");
        } else if roll < oil {
            self.events.push("oil");
            self.oil_damage = (self.oil_damage + 0.5).min(1.0);
        } else if roll < bearing {
            if water {
                self.knock_out(p, 1.0);
                self.events.push("smoke");
            } else {
                self.readyness *= 0.95;
                self.events.push("bearing");
            }
        } else if roll < fire {
            self.knock_out(p, 1.0);
            self.events.push("fire");
        } else if roll < charger {
            self.events.push("supercharger");
            self.readyness *= 0.7;
        } else if roll < dead {
            self.set_stage(7);
            self.events.push("dead");
        } else {
            self.set_stage(8);
            self.w = 0.0;
            self.events.push("seized");
        }
    }

    /// Specific consumption at power fraction `x` (kg per unit per hour).
    fn specific_fuel(p: &EngineParams, x: f64) -> f64 {
        let [p0, p05, p1, pmax] = p.fuel_curve;
        let x = x.max(0.0);
        if x <= 0.5 {
            (x - 0.5).powi(2) * (p0 - p05) * 4.0 + p05
        } else if x <= 1.0 {
            (x - 0.5).powi(2) * (p1 - p05) * 4.0 + p05
        } else {
            p1 + (pmax - p1) * ((x - 1.0).min(0.1) * 10.0)
        }
    }

    /// Advances any engine type; `fuel_left`: whether the tank it burns from has fuel.
    #[allow(clippy::too_many_arguments)]
    pub fn step_any(&mut self, p: &EngineParams, throttle: f64, wep: bool, v: f64, speed: f64, h: f64, dt: f64, fuel_left: bool) {
        self.fuel_rate = 0.0;
        self.nitro_rate = 0.0;
        if fuel_left && self.stage == 6 && throttle <= 0.0 && p.piston {
            // Throttle closed: the engine idles (it does not stop).
        }
        if let Some(w) = self.start_sequence(p, dt) {
            self.w = w;
            self.thrust = 0.0;
            self.torque = 0.0;
            self.prop_torque = 0.0;
            return;
        }
        if self.stage == 8 {
            self.w = 0.0;
            self.thrust = 0.0;
            self.torque = 0.0;
            return;
        }
        let running = self.stage == 6;
        if !fuel_left && running {
            self.set_stage(0);
            self.events.push("no fuel");
        }
        let fuel_left = fuel_left && self.stage == 6;
        match p.kind {
            EngineKind::Piston | EngineKind::PistonHelo => {
                let v = if p.kind == EngineKind::PistonHelo { v.abs() } else { v };
                self.step_wep(p, if fuel_left { throttle } else { 0.0 }, wep && fuel_left, v, speed, h, dt);
                if wep && fuel_left {
                    self.nitro_rate = p.wep_nitro_rate();
                }
            }
            EngineKind::Jet => {
                let t = if fuel_left { throttle.clamp(0.0, 1.0) } else { 0.0 };
                let running = fuel_left;
                let x = self.w / p.w_max;
                let mut m = if running { (p.gov_min + t * (p.gov_max - p.gov_min)) / p.gov_max.max(1e-6) * p.jet_m_max } else { 0.0 };
                if !running {
                    m -= 0.002 * self.w * (p.prop_i + p.engine_i) / dt;
                }
                let q = p.jet_m_max * x * x;
                let mut aw = (m - q) / (p.prop_i + p.engine_i);
                if aw > 0.0 {
                    aw *= p.acceleration;
                }
                self.aw = aw;
                self.w = (self.w + aw * dt).clamp(-p.w_max_allowed, 2.0 * p.w_max_allowed);
                let x = self.w / p.w_max;
                let pe = (atmosphere::pressure(h) + p.speed_manifold * 0.5 * atmosphere::density(h) * speed * speed) * 9.871_668_3e-6;
                let temp = 1.0 - 0.006 * (atmosphere::temperature(h) - 290.0);
                let ram = 1.0 - 0.0011 * speed;
                self.thrust = if running { p.thrust_max * x * pe * x * x * temp * ram } else { 0.0 };
                self.prop_torque = 0.0;
                self.torque = 0.0;
                if running {
                    let power = t;
                    let n = (p.thrust_max * power).max(p.thrust_max * 0.05);
                    self.fuel_rate = Self::specific_fuel(p, power) / 3600.0 * n;
                }
            }
            EngineKind::Rocket | EngineKind::RocketBoost => {
                let t = throttle.clamp(0.0, 1.0);
                let mut w = p.w_min + (p.w_max - p.w_min) * t;
                if w < p.w_min || w < 0.0 || !fuel_left {
                    w = 0.0;
                }
                self.w = w;
                self.thrust = w / p.w_max * p.thrust_max * (p.compressor_p_max / (p.compressor_p_max + 1e-5 * atmosphere::pressure(h)));
                self.prop_torque = 0.0;
                self.torque = 0.0;
                if w > 0.0 {
                    if p.kind == EngineKind::Rocket {
                        self.fuel_rate = p.rocket_burn * t;
                    } else {
                        self.nitro_rate = 1.432_055_6 * t;
                    }
                }
            }
            EngineKind::Tow => {
                // Free flight: the tow string pulls only while a tug tows the glider.
                self.thrust = 0.0;
                self.torque = 0.0;
                self.w = 0.0;
            }
            EngineKind::Pvrd => {
                let t = throttle.clamp(0.0, 1.0);
                let mut w = p.w_min + (p.w_max - p.w_min) * t;
                if w < p.w_min || w < 0.0 || !fuel_left {
                    w = 0.0;
                }
                let r = speed / 94.0;
                let k = if r < 1.0 {
                    w = 0.0;
                    0.0
                } else {
                    r.sqrt()
                };
                let mut thrust = w / p.w_max * p.thrust_max * k;
                if speed > 208.333 {
                    thrust = if speed > 291.666 { 0.0 } else { thrust * ((291.666 - speed) / 83.333).sqrt() };
                }
                self.w = w;
                self.thrust = thrust;
                self.torque = 0.0;
                self.prop_torque = 0.0;
                if w > 0.0 {
                    let n = (p.thrust_max * t).max(p.thrust_max * 0.05);
                    self.nitro_rate = Self::specific_fuel(p, t) / 3600.0 * n;
                }
            }
        }
    }

    /// Advances the engine by `dt`. `v`: airspeed along body x; `h`: altitude; `speed`: total airspeed.
    pub fn step(&mut self, p: &EngineParams, throttle: f64, v: f64, speed: f64, h: f64, dt: f64) {
        self.step_wep(p, throttle, false, v, speed, h, dt);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn step_wep(&mut self, p: &EngineParams, throttle: f64, wep: bool, v: f64, speed: f64, h: f64, dt: f64) {
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
                let top = if wep { p.gov_max_wep } else { p.gov_max };
                let wt = p.gov_min + (top - p.gov_min) * lever;
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
        // Boost raises the allowed manifold pressure.
        let wf = if wep { p.wep_compressor } else { 1.0 };
        let threshold = threshold * wf;
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
                self.second_throttle = (wf * p.max_ata_rpm / m.max(1e-6)).min(1.0);
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
                let target = wf * p.max_ata_rpm / m.max(1e-6);
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
        let boost = if wep { boost * p.wep_boost } else { boost };
        let torque_curve = (torque_curve * boost).max(-0.8 * self.w / p.w_max * p.m_max);
        let running = self.stage == 6;
        let mut m = if running { torque_curve * c * self.readyness } else { 0.0 };
        if !running {
            // Friction of a stopped engine (dt-scaled like the runtime).
            self.friction = (self.friction + 0.1 * dt).min(1.0);
            let x = self.w / p.w_max;
            let inertia = p.prop_i + p.engine_i;
            m += (-self.friction * (6.0 + 3.8 * x) * inertia / dt).max(-0.99 * self.w * inertia / dt);
        } else {
            self.friction = 0.0;
        }

        // 6. Rotation.
        let mut aw = (m - q) / (p.prop_i + p.engine_i);
        if aw > 0.0 {
            aw *= p.acceleration;
        }
        self.aw = aw;
        self.w = (self.w + aw * dt).clamp(0.0, 2.0 * p.w_max_allowed);

        // Fuel: specific use at the power produced (torque × compressor), per hour.
        let hp_out = torque_curve * c * self.w * 0.00105;
        if running {
            let x = hp_out / p.horse_powers;
            let hp_for = hp_out.max(p.horse_powers * 0.05);
            self.fuel_rate = Self::specific_fuel(p, x) / 3600.0 * (0.8 + 0.2 * self.w / p.w_wep) * hp_for;
        }

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
