//! The flight model of AI-flown aircraft (the runtime's `AIFlightModel`;
//! `docs/behavior/ai.md`, "AI flight model"): whole-wing forces, and rotation rates set directly
//! by the controls instead of from control-surface moments.

use crate::aircraft::Aircraft;
use crate::damage::Squares;
use crate::polar::Polar;
use crate::sim::Inputs;
use glam::DVec3;

pub(crate) struct AiAeroIn<'a> {
    pub ac: &'a Aircraft,
    pub sq: &'a Squares,
    pub wing: &'a Polar,
    pub tail: &'a Polar,
    pub ct: &'a Inputs,
    pub vflow: DVec3,
    pub rho: f64,
    pub aoa: f64,
    pub aos: f64,
    pub full_mach: f64,
    pub kq: f64,
    pub height: f64,
    pub gravity: f64,
    pub w: DVec3,
    pub prop_sign: f64,
    pub on_ground: bool,
    pub tail_intact: bool,
    pub wing_mid_l: bool,
    pub wing_mid_r: bool,
    pub radiator: f64,
}

pub(crate) struct AiAeroOut {
    pub af: DVec3,
    pub am: DVec3,
    /// Rotation rates commanded by the controls, rad/s, added to the dynamic ones.
    pub wtrue: DVec3,
    pub excessive_force: bool,
}

pub(crate) fn ai_aero(c: &AiAeroIn) -> AiAeroOut {
    let (ac, sq, ct) = (c.ac, c.sq, c.ct);
    let q = 0.5 * c.rho * c.vflow.length_squared();
    let (cx, cy) = c.wing.coeffs_mach(c.aoa, c.full_mach, ac.mach.as_ref());
    let gear_cx = if ac.controls.retractable {
        0.035 * ct.gear
    } else {
        0.0
    };
    let mut cw = DVec3::new(
        -q * (cx + 2.0 * gear_cx + 2.0 * ac.airbrake_cx * ct.airbrake + 0.003 * c.radiator),
        -q * 0.7 * c.aos.to_radians().sin(),
        q * cy * c.kq,
    );
    if c.height < 0.4 * ac.length {
        let k = 1.0 + 0.3 * (1.0 - c.height / (0.4 * ac.length));
        cw.z *= k;
        cw.x *= k;
    }
    let (wl, wr) = (sq.wing_l.iter().sum::<f64>(), sq.wing_r.iter().sum::<f64>());
    let mut af = cw * (wl + wr);
    af.x -= q * sq.drag_produced;
    af.y -= c.aos * q * 0.1;
    let mut excessive_force = false;
    if !af.is_finite() {
        af = DVec3::ZERO;
        excessive_force = true;
    } else if af.length() > c.gravity * 50.0 {
        af = af.normalize() * c.gravity * 50.0;
        excessive_force = true;
    }
    // The root arm is not in OpenIL2's data; half the middle arm stands in for it.
    let arms = [0.5 * ac.arm_wing_mid, ac.arm_wing_mid, ac.arm_wing_out];
    let mut am = DVec3::ZERO;
    am.x = (0..3)
        .map(|i| (sq.wing_l[i] - sq.wing_r[i]) * arms[i])
        .sum::<f64>()
        * cw.z;
    if c.aoa.abs() > 33.0 {
        am.y = sq.stab * ac.arm_stab * q * c.tail.coeffs(c.aoa).1 * c.kq;
    }
    if c.aos.abs() > 33.0 {
        am.z = (0.2 + sq.keel) * ac.arm_keel * q * c.tail.coeffs(c.aos).1 * c.kq;
    }
    let mut v2 = (c.vflow.length_squared() - 120.0).clamp(0.0, 15_000.0);
    if c.vflow.x < 0.0 {
        v2 = 0.0;
    }
    let area = ac.wing_area;
    let mut wtrue = DVec3::ZERO;
    if !c.tail_intact && c.height > 20.0 && v2 > 10.0 {
        am.y += 5.0 * area * c.vflow.x;
        am.z += 80.0 * area * c.prop_sign;
        if c.aoa.abs() > 20.0 {
            let s = if c.w.z < 0.0 { -1.0 } else { 1.0 };
            am.z += 30.0 * s * area * (3.0 * c.vflow.z + c.vflow.x);
            am.x -= 50.0 * s * area * (c.vflow.z + 3.0 * c.vflow.x);
        }
    } else {
        if !c.on_ground {
            let a = (c.aoa * 3.0).clamp(-25.0, 25.0);
            if !c.wing_mid_l {
                am.x -= a * wr * v2;
            } else if !c.wing_mid_r {
                am.x += a * wl * v2;
            }
        }
        if sq.keel > 0.1 {
            am.z += c.aos * q * 0.5;
        } else {
            am.x += c.aos * q;
        }
        let mut k = 1.0;
        if c.height < 1.5 * ac.length {
            k = (k + (c.height - 1.5 * ac.length) / ac.length).max(0.0);
        }
        if c.vflow.x < 20.0 && c.aos.abs() < 33.0 {
            am.y += k * af.z;
        }
        let vx = c.vflow.x.min(150.0);
        let sens_yaw = (ac.sens_yaw * sq.sens[2]).min(0.2);
        let sens_roll = (ac.sens_roll * sq.sens[0]).min(4.0);
        let elev_k = (20.0 - c.aoa.abs()).max(4.0);
        let mut pitch = c.aoa - ct.elevator * elev_k;
        if pitch > 90.0 {
            pitch -= 180.0;
        }
        if pitch < -90.0 {
            pitch += 180.0;
        }
        pitch *= (0.017 * c.aoa.abs()).max(1.0);
        let rud_k = (12.0 - c.aos.abs()).max(0.0);
        let mut yaw = c.aos - ct.rudder * rud_k;
        if yaw > 90.0 {
            yaw -= 180.0;
        }
        if yaw < -90.0 {
            yaw += 180.0;
        }
        yaw *= (0.01 * c.aos.abs()).max(1.0);
        let inv_area = 1.0 / area.max(1.0);
        wtrue.x = ct.aileron * vx * sens_roll * sq.ailerons * inv_area;
        wtrue.y = pitch * vx * ac.sens_pitch * sq.sens[1] * sq.elevators * inv_area * 0.017;
        wtrue.z = yaw * vx * sens_yaw * sq.rudders * inv_area * 0.15;
    }
    if sq.elevators < 0.1 && c.height > 20.0 && v2 > 10.0 {
        am.y += c.gravity * 0.4;
    }
    AiAeroOut {
        af,
        am,
        wtrue,
        excessive_force,
    }
}
