//! Aircraft parameters from flight-model and engine files (`docs/behavior/flight-model.md`, "Data").

use crate::engine::EngineParams;
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
    /// A seaplane's float or hull point (`[Gear] WaterClipList`): it rides on the water.
    pub float: bool,
}

/// Which controls an aircraft has, from `[Controls]` (`docs/behavior/controls.md`).
#[derive(Clone, Debug)]
pub struct Controls {
    /// Aileron, elevator, rudder trim available.
    pub trim: [bool; 3],
    /// `DefaultAileronTrim`, `DefaultElevatorTrim`, `DefaultRudderTrim`.
    pub default_trim: [f64; 3],
    pub flaps: bool,
    /// `CFlapPos 3`: combat, take-off and landing positions; otherwise only up and down.
    pub flap_steps: bool,
    pub airbrake: bool,
    /// Retractable undercarriage (`CUndercarriage`); fixed gear otherwise.
    pub retractable: bool,
    pub diff_brakes: bool,
    pub tailwheel_lock: bool,
    /// Speeds (m/s) above which aileron, rudder and elevator travel is limited.
    pub thresholds: [f64; 3],
    /// `CArrestorHook`, `CWingFold`.
    pub arrestor: bool,
    pub wing_fold: bool,
    /// Wing folding rate, travel per second (`1 / WingPeriod`, default 0.1).
    pub wing_rate: f64,
}

impl Default for Controls {
    fn default() -> Self {
        Controls {
            trim: [false, true, false],
            default_trim: [0.0; 3],
            flaps: true,
            flap_steps: true,
            airbrake: false,
            retractable: true,
            diff_brakes: true,
            tailwheel_lock: false,
            thresholds: [100.0, 100.0, 112.0],
            arrestor: false,
            wing_fold: false,
            wing_rate: 0.1,
        }
    }
}

impl Controls {
    pub fn load(fmd: &SectFile) -> Self {
        let get = |k: &str, d: f64| {
            fmd.section_ci("Controls")
                .and_then(|s| s.get_ci(k))
                .and_then(il2_formats::sectfile::atof)
                .map_or(d, f64::from)
        };
        let flag = |k: &str, d: bool| get(k, if d { 1.0 } else { 0.0 }) == 1.0;
        Controls {
            trim: [
                flag("CAileronTrim", true),
                flag("CElevatorTrim", true),
                flag("CRudderTrim", true),
            ],
            default_trim: [
                get("DefaultAileronTrim", 0.0),
                get("DefaultElevatorTrim", 0.0),
                get("DefaultRudderTrim", 0.0),
            ],
            flaps: flag("CFlap", false),
            flap_steps: get("CFlapPos", 3.0) >= 3.0,
            airbrake: flag("CDiveBrake", false),
            retractable: flag("CUndercarriage", false),
            diff_brakes: flag("CDifferentialBrakes", false),
            tailwheel_lock: flag("CLockTailwheel", false),
            thresholds: [
                0.27778 * get("CAileronThreshold", 360.0),
                0.27778 * get("CRudderThreshold", 360.0),
                0.27778 * get("CElevatorThreshold", 403.2),
            ],
            arrestor: flag("CArrestorHook", false),
            wing_fold: flag("CWingFold", false),
            wing_rate: fmd
                .section_ci("Controls")
                .and_then(|s| s.get_ci("WingPeriod"))
                .and_then(il2_formats::sectfile::atof)
                .map_or(0.1, |p| 1.0 / f64::from(p).max(0.1)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Aircraft {
    pub length: f64,
    pub wingspan: f64,
    pub mass_empty: f64,
    pub fuel_max: f64,
    /// `[Mass] Nitro`: the separate tank of boost systems, rocket boosters and pulse jets (kg).
    pub nitro_max: f64,
    pub j0: DVec3,
    pub wing_area: f64,
    /// Section areas (in, mid, out) of one wing half.
    pub wing_sections: [f64; 3],
    pub stab_area: f64,
    pub elevator_area: f64,
    /// `[Squares] Aileron`, m².
    pub aileron_area: f64,
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
    /// Wave drag of the wing; `None` when the flight model has no Mach tables.
    pub mach: Option<crate::polar::MachTable>,
    pub engines: Vec<EngineParams>,
    pub gear: Vec<GearPoint>,
    pub controls: Controls,
    /// `[Params] VmaxH`, m/s.
    pub vmax_h: f64,
    /// `[Params] Vmax`, m/s.
    pub vmax: f64,
    /// `[Params] VminFLAPS` × 0.2417 (as the runtime scales it), m/s.
    pub vmin_flaps: f64,
    /// `[Params] G_CLASS`: ultimate load factor at the reference weight.
    pub g_class: f64,
    /// `[Params] ReferenceWeight`: added to the full take-off mass for the load limit, kg.
    pub reference_weight: f64,
    /// `[Params] VmaxAllowed` (default 1.3 `VmaxH`): flutter speed, m/s indicated.
    pub vmax_allowed: f64,
    /// `[Arm] GC_Gear_Shift`: moment arm shift of the gear forces.
    pub gc_gear_shift: f64,
    /// `[Gear] SinkFactor`: buoyancy of the airframe in water.
    pub sink_factor: f64,
    /// `[Aircraft] Seaplane`.
    pub seaplane: bool,
    /// The lowered arrestor hook's contact point (`_ClipAGear`), body coordinates.
    pub arrestor_hook: Option<DVec3>,
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
    /// Placeholder without data, for tooling that needs an `Aircraft` value.
    pub fn empty() -> Self {
        let cfg = PolarCfg {
            cy0: 0.0,
            aoa_crit_h: 16.0,
            aoa_crit_l: -16.0,
            cy_crit_h: 1.0,
            cy_crit_l: -1.0,
            cx_min: 0.02,
            parab_cx: 8e-4,
        };
        Aircraft {
            length: 1.0,
            wingspan: 1.0,
            mass_empty: 1.0,
            fuel_max: 0.0,
            nitro_max: 0.0,
            j0: DVec3::ONE,
            wing_area: 1.0,
            wing_sections: [1.0 / 6.0; 3],
            stab_area: 0.0,
            elevator_area: 0.0,
            aileron_area: 1.0,
            keel_area: 0.0,
            rudder_area: 0.0,
            airbrake_cx: 0.0,
            arm_stab: 0.0,
            arm_keel: 0.0,
            arm_rudder: 0.0,
            arm_wing_mid: 0.0,
            arm_wing_out: 0.0,
            wing_v: 0.0,
            gcenter: 0.0,
            gcenter_z: 0.0,
            gc_aoa_shift: 0.0,
            gc_flaps_shift: 0.0,
            sens_roll: 1.0,
            sens_pitch: 1.0,
            sens_yaw: 1.0,
            spin_cx_loss: 0.0,
            spin_cy_loss: 0.0,
            wing: PolarParams {
                line_cy: 0.09,
                aoa_min_cx_shift: 0.0,
                decline: 0.007,
                max_dist_ang: 30.0,
                parab_angle: 5.0,
                cfg: [cfg, cfg],
            },
            mach: None,
            engines: Vec::new(),
            gear: Vec::new(),
            controls: Controls::default(),
            vmax_h: 150.0,
            vmax: 150.0,
            vmin_flaps: 40.0,
            g_class: 12.0,
            reference_weight: 0.0,
            vmax_allowed: 195.0,
            gc_gear_shift: 0.0,
            sink_factor: 1.0,
            seaplane: false,
            arrestor_hook: None,
        }
    }

    /// `fmd`: the flight model; `emd(family)`: loads an engine family file.
    pub fn load(
        fmd: &SectFile,
        mut emd: impl FnMut(&str) -> Option<SectFile>,
        gear: Vec<GearPoint>,
    ) -> Result<Self, Error> {
        let length = num(fmd, "Aircraft", "Length")?;
        let wingspan = num(fmd, "Aircraft", "Wingspan")?;
        let scheme = num_or(fmd, "Aircraft", "Type", 0.0) as i32;
        let (a, b, c, d) = (
            (0.35 * length).powi(2),
            (0.125 * length).powi(2),
            (0.2 * wingspan).powi(2),
            (0.07 * length).powi(2),
        );
        let j0 = match scheme {
            2 => DVec3::new(
                0.3 * d + 0.7 * c,
                0.2 * a + 0.1 * b + 0.7 * d,
                0.2 * a + 0.1 * b + 0.7 * c,
            ),
            3 => DVec3::new(
                0.2 * d + 0.8 * c,
                0.2 * a + 0.2 * b + 0.6 * d,
                0.2 * a + 0.2 * b + 0.6 * c,
            ),
            4..=7 => DVec3::new(
                0.4 * d + 0.6 * c,
                0.25 * a + 0.15 * b + 0.6 * d,
                0.25 * a + 0.15 * b + 0.6 * c,
            ),
            _ => DVec3::new(
                0.6 * d + 0.4 * c,
                0.2 * a + 0.4 * b + 0.4 * d,
                0.2 * a + 0.4 * b + 0.4 * c,
            ),
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
            return Err(Error::Bad(
                "flight models without explicit polars are not supported yet".into(),
            ));
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
            let mut p = EngineParams::load(&e, sub)
                .ok_or_else(|| Error::Bad(format!("engine {fam}/{sub} not found")))?;
            p.v_schedule = (
                num_or(fmd, "Params", "Vmin", 160.0) / 3.6,
                num_or(fmd, "Params", "Vmax", 500.0) / 3.6,
            );
            engines.push(p);
        }

        Ok(Aircraft {
            length,
            wingspan,
            mass_empty,
            fuel_max: num(fmd, "Mass", "Fuel")?,
            nitro_max: num_or(fmd, "Mass", "Nitro", 0.0),
            j0,
            wing_area,
            wing_sections: sections,
            stab_area: num(fmd, "Squares", "Stabilizer")?,
            elevator_area: num(fmd, "Squares", "Elevator")?,
            aileron_area: num_or(fmd, "Squares", "Aileron", 1.0),
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
            mach: {
                let list = |k: &str| -> Vec<f64> {
                    fmd.section_ci("Polares")
                        .and_then(|s| s.get_ci(k))
                        .map(|v| {
                            v.split(',')
                                .filter_map(|t| il2_formats::sectfile::atof(t.trim()))
                                .map(f64::from)
                                .collect()
                        })
                        .unwrap_or_default()
                };
                crate::polar::MachTable::new(&list("mc3"), &list("mc4"), &list("mm"), &list("mz"))
            },
            wing,
            engines,
            gear,
            controls: Controls::load(fmd),
            vmax_h: num_or(fmd, "Params", "VmaxH", 540.0) / 3.6,
            vmax: num_or(fmd, "Params", "Vmax", 500.0) / 3.6,
            vmin_flaps: num_or(fmd, "Params", "VminFLAPS", 160.0) * 0.241_666_67,
            // Without `G_CLASS` the limit load is 12 g, as if `G_CLASS` were 18.
            g_class: num(fmd, "Params", "G_CLASS").map_or(18.0, |g| g.clamp(0.0, 15.0)),
            reference_weight: num_or(fmd, "Params", "ReferenceWeight", 0.0).clamp(-2000.0, 2000.0),
            vmax_allowed: num_or(
                fmd,
                "Params",
                "VmaxAllowed",
                1.3 * num_or(fmd, "Params", "VmaxH", 540.0),
            ) / 3.6,
            gc_gear_shift: num_or(fmd, "Arm", "GC_Gear_Shift", 0.0),
            sink_factor: num_or(fmd, "Gear", "SinkFactor", 1.0),
            seaplane: num_or(fmd, "Aircraft", "Seaplane", 0.0) != 0.0,
            arrestor_hook: None,
        })
    }

    pub fn wing_polar(&self) -> Polar {
        Polar::new(self.wing.clone())
    }
}
