//! The pilot's cockpit: mesh, eye point and instruments driven by the cockpit's own code
//! (`docs/behavior/cockpit.md`).

use crate::weapons::Gun;
use glam::{DVec3, Mat4, Vec3};
use il2_data::anim::Instruments;
use il2_fm::Sim;
use il2_render::scene::{Loader, NodeMotion, Rig};
use il2_render::MeshData;
use il2_vfs::Vfs;
use std::collections::HashMap;

pub struct Cockpit {
    /// The mesh with every node, hidden ones included (taken when uploading).
    pub mesh: Option<MeshData>,
    /// The pilot's eye (`CAMERA` hook) in body coordinates.
    pub eye: Vec3,
    rig: Rig,
    /// Node visibility, starting from the hierarchy's `Hidden` flags.
    visible: Vec<bool>,
    instruments: Option<Instruments>,
    acc: f64,
    prev_vel: Option<DVec3>,
    accel: DVec3,
}

impl Cockpit {
    pub fn load(vfs: &Vfs, classes: &il2_data::Classes, class: &str) -> Option<Cockpit> {
        let (cockpit_class, path) = classes.cockpit(class)?;
        let mut loader = Loader::new(vfs);
        let mut mesh = loader.load_hier(&path, true).ok()?;
        mesh.textures = loader.take_images();
        if std::env::var_os("IL2_TRACE").is_some() {
            for w in &loader.warnings {
                eprintln!("cockpit {path}: {w}");
            }
        }
        let eye = loader
            .hier_hooks(&path)
            .ok()?
            .into_iter()
            .find(|(n, _)| n.eq_ignore_ascii_case("CAMERA"))
            .map(|(_, p)| p)?;
        let rig = loader.hier_rig(&path).ok()?;
        let hier = il2_formats::him::Hier::parse(&vfs.read(&path).ok()?).ok()?;
        let visible = hier.nodes.iter().map(|n| !n.hidden).collect();
        let instruments = Instruments::new(classes.cockpit_pool(&cockpit_class));
        if let Some(e) = instruments.as_ref().and_then(|i| i.error.as_ref()) {
            eprintln!("cockpit {cockpit_class}: {e}");
        }
        Some(Cockpit {
            mesh: Some(mesh),
            eye,
            rig,
            visible,
            instruments,
            acc: 0.0,
            prev_vel: None,
            accel: DVec3::ZERO,
        })
    }

    /// Runs the instruments for `dt` seconds of flight and returns the cockpit's node transforms.
    pub fn pose(&mut self, sim: &Sim, guns: &[Gun], time_of_day: f64, dt: f64) -> Vec<Mat4> {
        if let Some(prev) = self.prev_vel {
            if dt > 0.0 {
                self.accel = self.accel * 0.8 + (sim.vel - prev) / dt * 0.2;
            }
        }
        self.prev_vel = Some(sim.vel);
        let mut motion: Vec<Option<NodeMotion>> = vec![None; self.rig.names.len()];
        if let Some(ins) = self.instruments.as_mut() {
            // The cockpit's interpolators step with the simulation tick.
            self.acc += dt;
            let ticks = (self.acc / il2_fm::sim::TICK) as usize;
            self.acc -= ticks as f64 * il2_fm::sim::TICK;
            let table = world_table(sim, guns, time_of_day, self.accel);
            for (name, st) in ins.update(table, ticks.min(10)) {
                let Some(i) = self.rig.find(name) else {
                    continue;
                };
                motion[i] = Some(NodeMotion {
                    angles: st.angles,
                    offset: st.offset,
                });
                if let Some(v) = st.visible {
                    self.visible[i] = v;
                }
            }
            if std::env::var_os("IL2_TRACE").is_some() {
                if let Some(e) = &ins.error {
                    eprintln!("cockpit: {e}");
                }
            }
        }
        let mut nodes = self.rig.pose(&motion);
        for (m, &v) in nodes.iter_mut().zip(&self.visible) {
            if !v {
                *m = Mat4::ZERO;
            }
        }
        nodes
    }

    /// The paths the instrument code read that `world_table` doesn't provide.
    pub fn missing(&self) -> Vec<String> {
        self.instruments
            .as_ref()
            .map(Instruments::missing)
            .unwrap_or_default()
    }
}

/// What the cockpit code reads from the flight model and the world, by access path, in the
/// original's units (`docs/behavior/cockpit.md`, "World values").
fn world_table(s: &Sim, guns: &[Gun], time_of_day: f64, accel: DVec3) -> HashMap<String, f64> {
    let mut t: HashMap<String, f64> = HashMap::new();
    let mut put = |k: &str, v: f64| {
        t.insert(k.to_owned(), v);
    };
    let fwd = s.ori * DVec3::X;
    let body_vel = s.ori.inverse() * s.vel;
    let speed = s.vel.length();
    let yaw = fwd.y.atan2(fwd.x).to_degrees();
    let pitch = fwd.z.clamp(-1.0, 1.0).asin().to_degrees();
    // Gravity minus acceleration in body axes: what a slip ball feels.
    let ball = s.ori.inverse() * (DVec3::new(0.0, 0.0, -9.81) - accel);
    let body_w = s.w;
    put("fm.getAltitude()", s.pos.z);
    put("fm.Loc.x", s.pos.x);
    put("fm.Loc.y", s.pos.y);
    put("fm.Loc.z", s.pos.z);
    put("fm.getSpeedKMH()", body_vel.x * 3.6);
    put("fm.getSpeed()", speed);
    put("fm.getSpeed2KMH()", speed * 3.6);
    put("fm.getVertSpeed()", s.vel.z);
    put("fm.getAOA()", s.aoa.to_degrees());
    put("fm.getOverload()", -ball.z / 9.81);
    put("fm.getBallAccel().x", ball.x);
    put("fm.getBallAccel().y", ball.y);
    put("fm.getBallAccel().z", ball.z);
    put("fm.getW().x", body_w.x);
    put("fm.getW().y", body_w.y);
    put("fm.getW().z", body_w.z);
    put("fm.Or.getKren()", s.bank_deg());
    put("fm.Or.getTangage()", pitch);
    put("fm.Or.getYaw()", yaw);
    put("fm.Or.azimut()", (360.0 - yaw).rem_euclid(360.0));
    put("fm.Or.getAzimut()", (360.0 - yaw).rem_euclid(360.0));
    put("fm.M.fuel", s.fuel);
    put("fm.M.maxFuel", s.ac.fuel_max);
    put("fm.M.nitro", s.nitro);
    let ct = &s.ct;
    put("fm.CT.getRudder()", ct.rudder);
    put("fm.CT.RudderControl", ct.rudder);
    put("fm.CT.getElevator()", ct.elevator);
    put("fm.CT.ElevatorControl", ct.elevator);
    put("fm.CT.getAileron()", ct.aileron);
    put("fm.CT.AileronControl", ct.aileron);
    put("fm.CT.PowerControl", ct.throttle);
    put("fm.CT.getPowerControl()", ct.throttle);
    put("fm.CT.FlapsControl", ct.flaps);
    put("fm.CT.getFlap()", ct.flaps);
    put("fm.CT.GearControl", ct.gear);
    put("fm.CT.getGear()", s.ct.gear);
    for k in ["L", "R", "C"] {
        put(&format!("fm.CT.getGear{k}()"), s.ct.gear);
    }
    put("fm.CT.BrakeControl", ct.brakes);
    put("fm.CT.getBrake()", ct.brakes);
    put("fm.CT.getBrakeL()", ct.brakes);
    put("fm.CT.getBrakeR()", ct.brakes);
    put("fm.CT.AirBrakeControl", ct.airbrake);
    put("fm.CT.getAirBrake()", s.airbrake);
    put("fm.CT.bHasBrakeControl", 1.0);
    put("fm.CT.getTrimElevatorControl()", s.trim[1]);
    put("fm.CT.trimElevator", s.trim[1]);
    put("fm.CT.getTrimAileronControl()", s.trim[0]);
    put("fm.CT.trimAileron", s.trim[0]);
    put("fm.CT.getTrimRudderControl()", s.trim[2]);
    put("fm.CT.trimRudder", s.trim[2]);
    put("fm.Gears.bTailwheelLocked", ct.tail_lock as u8 as f64);
    put("fm.Gears.isHydroOperable()", 1.0);
    put("fm.Gears.bIsHydroOperable", 1.0);
    put("fm.Gears.isOperable()", 1.0);
    for k in ["lgear", "rgear", "cgear"] {
        put(&format!("fm.Gears.{k}"), s.on_ground as u8 as f64);
    }
    put("fm.Gears.onGround()", s.on_ground as u8 as f64);
    put("fm.EI.getNum()", s.engines.len() as f64);
    put("fm.EI.engines.length", s.engines.len() as f64);
    for (i, e) in s.engines.iter().enumerate() {
        let p = format!("fm.EI.engines[{i}]");
        let mut put = |k: &str, v: f64| {
            t.insert(format!("{p}.{k}"), v);
        };
        put("getRPM()", e.rpm());
        put("getw()", e.w);
        put("getManifoldPressure()", e.manifold);
        put("tOilIn", e.t_oil_in);
        put("tOilOut", e.t_oil_out);
        put("tWaterOut", e.t_water);
        put("tWaterCritMax", 120.0);
        put("tOilCritMax", 110.0);
        put("getStage()", e.stage as f64);
        put("getReadyness()", e.readyness);
        put("getControlThrottle()", ct.throttle);
        put("getControlMix()", 1.0);
        put("getControlMagnetos()", 3.0);
        put("getControlCompressor()", 0.0);
        put("getControlRadiator()", e.radiator);
        put("getControlProp()", e.prop_manual.unwrap_or(1.0));
        put("getPropPhi()", e.phi);
        put("getPropPhiMin()", 0.0);
        put("getThrustOutput()", e.thrust);
        put("getPowerOutput()", ct.throttle);
        put("getEngineLoad()", ct.throttle);
        put("getEngineForce().x", e.thrust);
        put("getControlAfterburner()", ct.wep as u8 as f64);
        put("getExtinguishers()", 1.0);
        put("getCylindersOperable()", 1.0);
        put("getCylinders()", 1.0);
        put("isRunning()", e.running() as u8 as f64);
    }
    for g in guns {
        if let Some(h) = g.hook.as_deref() {
            t.insert(
                format!("fm.actor.getGunByHookName({h:?}).countBullets()"),
                g.ammo as f64,
            );
            t.insert(
                format!("fm.actor.getGunByHookName({h:?}).haveBullets()"),
                (g.ammo > 0) as u8 as f64,
            );
            t.insert(
                format!("fm.actor.getBulletEmitterByHookName({h:?}).haveBullets()"),
                (g.ammo > 0) as u8 as f64,
            );
        }
    }
    for i in 0..4 {
        let firing = guns.iter().any(|g| g.firing && g.trigger == i);
        t.insert(format!("fm.CT.saveWeaponControl[{i}]"), firing as u8 as f64);
        t.insert(format!("fm.CT.WeaponControl[{i}]"), firing as u8 as f64);
    }
    t.insert("World.getTimeofDay()".into(), time_of_day);
    t.insert("World.cur().Atm.ro0".into(), 1.225);
    t.insert("World.cur().Atm.T0".into(), 288.15);
    t.insert("World.cur().Atm.P0".into(), 101_325.0);
    t.insert("World.cur().Atm.g".into(), 9.81);
    t.insert(
        "World.cur().diffCur.RealisticNavigationInstruments".into(),
        0.0,
    );
    t.insert("World.rnd().nextFloat()".into(), 0.5);
    t.insert("Geom.DEG2RAD()".into(), std::f64::consts::PI / 180.0);
    t
}
