//! Aircraft parameters from flight-model and engine files (`docs/behavior/flight-model.md`, "Data").

use crate::polar::{Polar, PolarCfg, PolarParams};
use glam::DVec3;
use il2_formats::sectfile::atof;
use il2_formats::SectFile;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("flight model: missing [{0}] {1}")]
    Missing(&'static str, &'static str),
    #[error("flight model: {0}")]
    Bad(String),
}

#[derive(Clone, Debug)]
pub struct Engine {
    pub power_w: f64,
    pub compressor_alt: f64,
    pub prop_omega: f64,
    pub prop_radius: f64,
    /// +1 or −1, sign of the torque reaction about body x.
    pub prop_sign: f64,
    /// Position of the propeller hub in body coordinates (for torque arms; currently on the x axis).
    pub pos: DVec3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ContactKind {
    Wheel,
    /// Tail or nose wheel: low side friction, no brake.
    Castor,
    /// Airframe point (nose, wingtips, tail): stiff, scrapes with high friction.
    Body,
}

#[derive(Clone, Debug)]
pub struct GearPoint {
    pub pos: DVec3,
    pub kind: ContactKind,
}

#[derive(Clone, Debug)]
pub struct Aircraft {
    pub length: f64,
    pub wingspan: f64,
    pub mass_empty: f64,
    pub fuel_max: f64,
    pub j0: DVec3,
    pub wing_area: f64,
    /// Section areas (in, mid, out) of one wing half.
    pub wing_sections: [f64; 3],
    pub stab_area: f64,
    pub elevator_area: f64,
    pub keel_area: f64,
    pub rudder_area: f64,
    pub airbrake_cx: f64,
    pub arm_stab: f64,
    pub arm_keel: f64,
    pub arm_rudder: f64,
    pub arm_wing_mid: f64,
    pub arm_wing_out: f64,
    pub wing_v: f64,
    pub gcenter: f64,
    pub gcenter_z: f64,
    pub gc_aoa_shift: f64,
    pub gc_flaps_shift: f64,
    pub sens_roll: f64,
    pub sens_pitch: f64,
    pub sens_yaw: f64,
    pub spin_cx_loss: f64,
    pub spin_cy_loss: f64,
    pub wing: PolarParams,
    pub engines: Vec<Engine>,
    pub gear: Vec<GearPoint>,
}

fn num(f: &SectFile, s: &'static str, k: &'static str) -> Result<f64, Error> {
    f.section_ci(s)
        .and_then(|sec| sec.get_ci(k))
        .and_then(|v| atof(v.split_whitespace().next()?))
        .map(f64::from)
        .ok_or(Error::Missing(s, k))
}

fn num_or(f: &SectFile, s: &'static str, k: &'static str, d: f64) -> f64 {
    num(f, s, k).unwrap_or(d)
}

impl Aircraft {
    /// `fmd`: the flight model; `emd(family)`: loads an engine family file.
    pub fn load(
        fmd: &SectFile,
        mut emd: impl FnMut(&str) -> Option<SectFile>,
        gear: Vec<GearPoint>,
    ) -> Result<Self, Error> {
        let length = num(fmd, "Aircraft", "Length")?;
        let wingspan = num(fmd, "Aircraft", "Wingspan")?;
        let scheme = num_or(fmd, "Aircraft", "Type", 0.0) as i32;
        let (a, b, c, d) = ((0.35 * length).powi(2), (0.125 * length).powi(2), (0.2 * wingspan).powi(2), (0.07 * length).powi(2));
        let j0 = match scheme {
            2 => DVec3::new(0.3 * d + 0.7 * c, 0.2 * a + 0.1 * b + 0.7 * d, 0.2 * a + 0.1 * b + 0.7 * c),
            3 => DVec3::new(0.2 * d + 0.8 * c, 0.2 * a + 0.2 * b + 0.6 * d, 0.2 * a + 0.2 * b + 0.6 * c),
            4..=7 => DVec3::new(0.4 * d + 0.6 * c, 0.25 * a + 0.15 * b + 0.6 * d, 0.25 * a + 0.15 * b + 0.6 * c),
            _ => DVec3::new(0.6 * d + 0.4 * c, 0.2 * a + 0.4 * b + 0.4 * d, 0.2 * a + 0.4 * b + 0.4 * c),
        };
        let crew = num_or(fmd, "Aircraft", "Crew", 1.0);
        let mass_empty = num(fmd, "Mass", "Empty")? + num_or(fmd, "Mass", "Oil", 0.0) + 90.0 * crew;

        let mut wing_area = num(fmd, "Squares", "Wing")?;
        let mut sections = [
            num(fmd, "Squares", "Wing_In")?,
            num(fmd, "Squares", "Wing_Mid")?,
            num(fmd, "Squares", "Wing_Out")?,
        ];
        let ratio = 2.0 * sections.iter().sum::<f64>() / (wing_area + 0.01);
        if !(0.9..=1.1).contains(&ratio) {
            if ratio > 1.0 {
                wing_area = 2.0 * sections.iter().sum::<f64>();
            } else {
                sections = [wing_area / 6.0; 3];
            }
        }

        let p = |k: &'static str, d: f64| num_or(fmd, "Polares", k, d);
        if num(fmd, "Polares", "lineCyCoeff").is_err() {
            return Err(Error::Bad("flight models without explicit polars are not supported yet".into()));
        }
        let wing = PolarParams {
            line_cy: p("lineCyCoeff", 0.09),
            aoa_min_cx_shift: p("AOAMinCx_Shift", 0.0),
            decline: p("Decline", 0.007),
            max_dist_ang: p("maxDistAng", 30.0),
            parab_angle: p("parabAngle", 5.0),
            cfg: [
                PolarCfg {
                    cy0: p("Cy0_0", 0.15),
                    aoa_crit_h: p("AOACritH_0", 16.0),
                    aoa_crit_l: p("AOACritL_0", -16.0),
                    cy_crit_h: p("CyCritH_0", 1.1),
                    cy_crit_l: p("CyCritL_0", -0.8),
                    cx_min: p("CxMin_0", 0.026),
                    parab_cx: p("parabCxCoeff_0", 8e-4),
                },
                PolarCfg {
                    cy0: p("Cy0_1", 0.65),
                    aoa_crit_h: p("AOACritH_1", 15.0),
                    aoa_crit_l: p("AOACritL_1", -18.0),
                    cy_crit_h: p("CyCritH_1", 1.6),
                    cy_crit_l: p("CyCritL_1", -0.75),
                    cx_min: p("CxMin_1", 0.09),
                    parab_cx: p("parabCxCoeff_1", 0.0025),
                },
            ],
        };

        let mut engines = Vec::new();
        for i in 0..8 {
            let fam_key = format!("Engine{i}Family");
            let sub_key = format!("Engine{i}SubModel");
            let sect = fmd.section_ci("Engine");
            let (Some(fam), Some(sub)) = (
                sect.and_then(|s| s.get_ci(&fam_key)).map(str::trim),
                sect.and_then(|s| s.get_ci(&sub_key)).map(str::trim),
            ) else {
                break;
            };
            let e = emd(fam).ok_or_else(|| Error::Bad(format!("engine family {fam} not found")))?;
            let es = e.section_ci(sub).ok_or_else(|| Error::Bad(format!("engine {fam}/{sub} not found")))?;
            let g = |k: &str, d: f64| es.get_ci(k).and_then(atof).map(f64::from).unwrap_or(d);
            let hp = g("HorsePowers", 1000.0);
            let rpm = g("RPMNom", 2500.0) * g("Reductor", 1.0);
            engines.push(Engine {
                power_w: hp * 745.7,
                compressor_alt: g("CompressorAltitude0", 0.0),
                prop_omega: (rpm * std::f64::consts::TAU / 60.0).max(1.0),
                prop_radius: 0.5 * g("PropDiameter", 3.0),
                prop_sign: if g("Direction", 0.0) == 1.0 { -1.0 } else { 1.0 },
                pos: DVec3::ZERO,
            });
        }

        Ok(Aircraft {
            length,
            wingspan,
            mass_empty,
            fuel_max: num(fmd, "Mass", "Fuel")?,
            j0,
            wing_area,
            wing_sections: sections,
            stab_area: num(fmd, "Squares", "Stabilizer")?,
            elevator_area: num(fmd, "Squares", "Elevator")?,
            keel_area: num(fmd, "Squares", "Keel")?,
            rudder_area: num(fmd, "Squares", "Rudder")?,
            airbrake_cx: num_or(fmd, "Squares", "AirbrakeCxS", 0.0),
            arm_stab: num(fmd, "Arm", "Stabilizer")?,
            arm_keel: num(fmd, "Arm", "Keel")?,
            arm_rudder: num_or(fmd, "Arm", "Rudder", 0.0),
            arm_wing_mid: num(fmd, "Arm", "Wing_Mid")?,
            arm_wing_out: num(fmd, "Arm", "Wing_Out")?,
            wing_v: num_or(fmd, "Arm", "Wing_V", 0.0),
            gcenter: num_or(fmd, "Arm", "GCenter", 0.0),
            gcenter_z: num_or(fmd, "Arm", "GCenterZ", 0.0),
            gc_aoa_shift: num_or(fmd, "Arm", "GC_AOA_Shift", 0.0),
            gc_flaps_shift: num_or(fmd, "Arm", "GC_Flaps_Shift", 0.0),
            sens_roll: num_or(fmd, "Params", "SensRoll", 1.0),
            sens_pitch: num_or(fmd, "Params", "SensPitch", 1.0),
            sens_yaw: num_or(fmd, "Params", "SensYaw", 1.0),
            spin_cx_loss: num_or(fmd, "Params", "SpinCxLoss", 0.0),
            spin_cy_loss: num_or(fmd, "Params", "SpinCyLoss", 0.0),
            wing,
            engines,
            gear,
        })
    }

    pub fn wing_polar(&self) -> Polar {
        Polar::new(self.wing.clone())
    }
}
