//! Running an aircraft class's own `hitBone` damage code (`docs/behavior/damage.md`, "Bullet
//! hits"): the interpreter calls back here for everything the code does to the aircraft.

use crate::airframe::{Airframe, Shot};
use crate::world::AircraftAssets;
use il2_data::anim::{Host, HostVal, ShotState};
use il2_fm::Sim;
use std::collections::BTreeSet;

pub struct HitHost<'a> {
    pub sim: &'a mut Sim,
    pub af: &'a mut Airframe,
    pub toughness: &'a [f64; 44],
    /// Guns whose ammunition the hit destroyed: (trigger, index in that trigger's guns).
    pub jams: Vec<(i32, i32)>,
    /// World paths the code read that we don't model (diagnostics).
    pub missing: BTreeSet<String>,
    rng: u64,
}

impl<'a> HitHost<'a> {
    pub fn new(
        sim: &'a mut Sim,
        af: &'a mut Airframe,
        toughness: &'a [f64; 44],
        seed: u64,
    ) -> Self {
        HitHost {
            sim,
            af,
            toughness,
            jams: Vec::new(),
            missing: BTreeSet::new(),
            rng: seed | 1,
        }
    }

    fn uniform(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }

    /// The `[n]` of a path such as `FM.EI.engines[1]`.
    fn index(path: &str) -> usize {
        path.rfind('[')
            .and_then(|i| path[i + 1..].split(']').next()?.parse().ok())
            .unwrap_or(0)
    }

    fn engine_call(&mut self, e: usize, name: &str, args: &[HostVal]) -> Option<HostVal> {
        let n = |i: usize| args.get(i).map_or(0.0, HostVal::num);
        let p = self.sim.ac.engines.get(e)?.clone();
        let st = self.sim.engines.get_mut(e)?;
        let r = match name {
            "getReadyness" => HostVal::Num(st.readyness),
            "setReadyness" => {
                st.set_readyness(n(1));
                HostVal::Null
            }
            "setCyliderKnockOut" => {
                st.knock_out_cylinders(&p, n(1).max(0.0));
                HostVal::Null
            }
            "getCylinders" => HostVal::Num(p.cylinders),
            "getCylindersOperable" => HostVal::Num((p.cylinders - st.cylinders_out).max(0.0)),
            "getCylindersRatio" => HostVal::Num(1.0 - st.cylinders_out / p.cylinders.max(1.0)),
            "setMagnetoKnockOut" => {
                st.knock_out_magneto(n(1) as usize);
                HostVal::Null
            }
            "setEngineStuck" => {
                st.seize();
                HostVal::Null
            }
            "setEngineDies" => {
                st.die();
                HostVal::Null
            }
            "setEngineStops" => {
                st.stop();
                HostVal::Null
            }
            "getStage" => HostVal::Num(st.stage as f64),
            "getType" => HostVal::Num(match p.kind {
                il2_fm::engine::EngineKind::Piston | il2_fm::engine::EngineKind::PistonHelo => {
                    if p.radial {
                        1.0
                    } else {
                        0.0
                    }
                }
                il2_fm::engine::EngineKind::Jet => 2.0,
                il2_fm::engine::EngineKind::Rocket => 3.0,
                il2_fm::engine::EngineKind::RocketBoost => 4.0,
                il2_fm::engine::EngineKind::Tow => 5.0,
                il2_fm::engine::EngineKind::Pvrd => 6.0,
            }),
            "getPowerOutput" => HostVal::Num(if st.running() {
                self.sim.ct.throttle.min(1.1) * st.readyness
            } else {
                0.0
            }),
            "setKillCompressor" => {
                self.sim.engine_specific(e, 0);
                HostVal::Null
            }
            "setKillPropAngleDevice" => {
                self.sim.engine_specific(e, 3);
                HostVal::Null
            }
            "setKillPropAngleDeviceSpeeds" => {
                self.sim.engine_specific(e, 4);
                HostVal::Null
            }
            _ => return None,
        };
        Some(r)
    }

    /// Calls on `this.FM.AS` (`AircraftState`).
    fn state_call(&mut self, name: &str, args: &[HostVal]) -> Option<HostVal> {
        let n = |i: usize| args.get(i).map_or(0.0, HostVal::num);
        let s = &mut *self.sim;
        match name {
            "setCockpitState" => s.systems.cockpit = n(1) as u32,
            "setJamBullets" => self.jams.push((n(0) as i32, n(1) as i32)),
            "setControlsDamage" => s.cut_controls(n(1) as usize),
            "hitTank" => s.hit_tank(n(1) as usize, n(2).max(0.0) as u8),
            "doSetTankState" | "setTankState" => {
                s.set_tank(n(1) as usize, n(2).clamp(0.0, 6.0) as u8);
                return Some(HostVal::Num(1.0));
            }
            "explodeTank" => s.explode_tank(n(1) as usize),
            "setEngineSpecificDamage" => s.engine_specific(n(1) as usize, n(2) as i32),
            "hitEngine" => s.hit_engine(n(1) as usize, n(2).max(0.0) as u8),
            "doSetEngineState" | "setEngineState" => {
                s.set_engine(n(1) as usize, n(2).clamp(0.0, 4.0) as u8);
                return Some(HostVal::Num(1.0));
            }
            "hitOil" => s.hit_oil(n(1) as usize),
            "setOilState" => {
                if n(2) > 0.0 {
                    s.hit_oil(n(1) as usize);
                }
            }
            "setInternalDamage" => s.internal_damage(n(1) as i32),
            "setEngineStuck" => {
                if let Some(e) = s.engines.get_mut(n(1) as usize) {
                    e.seize();
                }
            }
            "setEngineDies" => {
                if let Some(e) = s.engines.get_mut(n(1) as usize) {
                    e.die();
                }
            }
            "hitPilot" => s.hit_crew(n(1) as usize, n(2) as i32),
            "setPilotState" => {
                let i = n(1) as usize;
                let have = s.systems.crew.get(i).copied().unwrap_or(0) as i32;
                s.hit_crew(i, n(2) as i32 - have);
            }
            "setBleedingPilot" => s.bleed(n(1) as usize, n(2) as i32),
            "setPilotWound" => s.wound(n(2) as i32),
            "woundedPilot" => {
                let (kind, amount) = (n(1) as i32, n(2));
                if amount > 4.0 && (kind == 2 || self.uniform() < 0.18) {
                    self.sim.wound(kind);
                }
            }
            "isMaster" => return Some(HostVal::Num(1.0)),
            _ => return None,
        }
        Some(HostVal::Null)
    }

    /// Calls on the aircraft itself that touch the engine's world (others run as class code).
    fn this_call(&mut self, name: &str, args: &[HostVal], shot: &mut ShotState) -> Option<HostVal> {
        let n = |i: usize| args.get(i).map_or(0.0, HostVal::num);
        let s = |i: usize| args.get(i).map_or("", HostVal::str).to_owned();
        match name {
            "debuggunnery" | "debugprintln" | "msgSndShot" | "sfxHit" | "sfxCrash" => {}
            "hitChunk" => {
                let base = s(0);
                let chunk = if base.contains('_') {
                    base
                } else {
                    format!("{base}_D{}", self.af.visible_level(&base).unwrap_or(0))
                };
                self.af
                    .hit_chunk(self.sim, &chunk, shot.power * 2.4e-7, self.toughness);
            }
            "chunkDamageVisible" => {
                return Some(HostVal::Num(
                    self.af.visible_level(&s(0)).unwrap_or(0) as f64
                ))
            }
            "isChunkAnyDamageVisible" => {
                return Some(HostVal::Num(
                    self.af.visible_level(&s(0)).is_some() as i32 as f64
                ))
            }
            "nextDMGLevels" => {
                let k = n(0).max(0.0) as usize;
                self.af.next_dmg_levels(self.sim, k, &s(2));
            }
            "msgCollision" => self.af.collision(self.sim, &s(1)),
            "doRicochet" => shot.power = 0.0,
            "doRicochetBack" => {}
            "killPilot" => self.sim.hit_crew(n(1) as usize, 100),
            "hitProp" => {
                // The propeller is shot away: no more thrust from that engine.
                if let Some(e) = self.sim.engines.get_mut(n(0) as usize) {
                    e.seize();
                }
            }
            _ => return None,
        }
        Some(HostVal::Null)
    }
}

impl Host for HitHost<'_> {
    fn call(
        &mut self,
        recv: &str,
        name: &str,
        args: &[HostVal],
        shot: &mut ShotState,
    ) -> Option<HostVal> {
        let n = |i: usize| args.get(i).map_or(0.0, HostVal::num);
        match recv {
            "World.Rnd()" | "World.Rnd" => {
                let u = self.uniform();
                Some(HostVal::Num(match (name, args.len()) {
                    ("nextInt", 2) => {
                        let (a, b) = (n(0), n(1));
                        (a + (u * (b - a + 1.0)).floor()).min(b)
                    }
                    ("nextInt", 1) => (u * n(0)).floor(),
                    ("nextBoolean", _) => (u < 0.5) as i32 as f64,
                    (_, 2) => n(0) + (n(1) - n(0)) * u,
                    _ => u,
                }))
            }
            "World" => Some(match name {
                "Rnd" => HostVal::Path("World.Rnd()".into()),
                "getPlayerAircraft" => HostVal::Path("player".into()),
                _ => HostVal::Path(format!("World.{name}()")),
            }),
            "Aircraft" if name == "debugprintln" => Some(HostVal::Null),
            "HUD" | "Voice" | "Explosions" | "Eff3DActor" | "Chat" | "EventLog" | "Config" => {
                Some(HostVal::Null)
            }
            "this" => self.this_call(name, args, shot),
            "shot" => match name {
                "powerToTNT" => Some(HostVal::Num(shot.power * 2.4e-7)),
                _ => None,
            },
            "FM.AS" => self.state_call(name, args),
            "FM.EI" if name == "getNum" => Some(HostVal::Num(self.sim.engines.len() as f64)),
            "FM.Gears" => {
                if matches!(name, "setHydroOperable" | "setOperable") && n(0) == 0.0 {
                    self.sim
                        .internal_damage(if name == "setOperable" { 3 } else { 0 });
                }
                Some(HostVal::Null)
            }
            "FM" => match name {
                "setTakenMortalDamage" if n(0) != 0.0 => {
                    self.sim.systems.mortal = true;
                    Some(HostVal::Null)
                }
                "getSpeed" | "getSpeedKMH" => Some(HostVal::Num(
                    self.sim.vel.length() * if name == "getSpeedKMH" { 3.6 } else { 1.0 },
                )),
                _ => Some(HostVal::Null),
            },
            _ if recv.starts_with("FM.EI.engines[") => {
                let e = Self::index(recv);
                self.engine_call(e, name, args)
            }
            _ => None,
        }
    }

    fn get(&mut self, path: &str) -> f64 {
        let s = &self.sim.systems;
        let i = Self::index(path);
        let field = path.rsplit('.').next().unwrap_or(path);
        let base = field.split('[').next().unwrap_or(field);
        match base {
            "astateTankStates" => s.tanks.get(i).copied().unwrap_or(0) as f64,
            "astateEngineStates" => s.engines.get(i).copied().unwrap_or(0) as f64,
            "astateOilStates" => s.oil.get(i).copied().unwrap_or(0) as f64,
            "astatePilotStates" => s.crew.get(i).copied().unwrap_or(0) as f64,
            "astateCockpitState" => s.cockpit as f64,
            "Skill" => 2.0,
            "fuel" => self.sim.fuel,
            "z" if path.contains("Vwld") => self.sim.vel.z,
            _ => {
                self.missing.insert(path.to_owned());
                0.0
            }
        }
    }
}

/// Runs `hitBone` for a struck part; `false` if the class code failed (the caller falls back to
/// the generic damage).
#[allow(clippy::too_many_arguments)]
pub fn run(
    assets: &AircraftAssets,
    sim: &mut Sim,
    af: &mut Airframe,
    bone: &str,
    shot: &mut Shot,
    point: glam::DVec3,
    seed: u64,
    jams: &mut Vec<(i32, i32)>,
) -> bool {
    let Some(chain) = assets.hit_chain.as_deref() else {
        return false;
    };
    let mut st = ShotState {
        power: shot.power,
        mass: shot.mass,
        power_type: shot.power_type,
    };
    let mut host = HitHost::new(sim, af, &assets.toughness, seed);
    let r = il2_data::anim::run_hit_bone(
        chain,
        &mut host,
        bone,
        &mut st,
        point.to_array(),
        shot.dir.to_array(),
    );
    jams.append(&mut host.jams);
    if std::env::var_os("IL2_TRACE").is_some() {
        let sys = &host.sim.systems;
        eprintln!(
            "  hitBone {bone}: {:.0} J -> {:.0} J (tanks {:?} engines {:?} oil {:?} crew {:?} controls {:?}){}{}",
            shot.power,
            st.power,
            sys.tanks,
            sys.engines,
            sys.oil,
            sys.crew,
            sys.controls_cut,
            r.as_ref().err().map_or(String::new(), |e| format!(" (failed: {e})")),
            if host.missing.is_empty() {
                String::new()
            } else {
                format!(" missing {:?}", host.missing)
            }
        );
    }
    shot.power = st.power;
    shot.power_type = st.power_type;
    r.is_ok()
}

/// The runtime's `getEnergyPastArmor`: armour `thickness` (1700 J each, twice that against high
/// explosive) takes energy from the round; returns what is left.
pub fn armour(thickness: f64, shot: &mut Shot) -> f64 {
    let k = if shot.power_type == 0 { 2.0 } else { 1.0 };
    shot.power -= k * thickness * 1700.0;
    shot.power
}

/// A round passing through the collision `parts` (in order) of an aircraft
/// (`docs/behavior/damage.md`, "Bullet hits"). Guns it put out of action are added to `jams`.
pub fn bullet(
    af: &mut Airframe,
    sim: &mut Sim,
    assets: &AircraftAssets,
    parts: &[crate::hitbox::PartHit],
    shot: &mut Shot,
    jams: &mut Vec<(i32, i32)>,
) {
    // The impact jolts the aircraft.
    let k = 2000.0 * shot.mass / sim.mass().max(1.0);
    let kick = glam::DVec3::new(
        (af.random() * 2.0 - 1.0) * k,
        (af.random() * 2.0 - 1.0) * k,
        (af.random() * 2.0 - 1.0) * k,
    );
    sim.w += kick;
    let n = if shot.power_type == 1 {
        parts.len().min(2)
    } else {
        parts.len()
    };
    let mut generic = false;
    for i in 0..n {
        let p = &parts[i];
        let Some(name) = p
            .hull
            .and_then(|k| assets.hulls.get(p.node)?.get(k))
            .map(|h| h.name.clone())
        else {
            generic = true;
            continue;
        };
        if shot.power <= 0.0 {
            continue;
        }
        // The hit point in aircraft coordinates (the trace is 50 m long).
        let point = p.point;
        let seed = (af.random() * u64::MAX as f64) as u64;
        let ran = run(assets, sim, af, &name, shot, point, seed, jams);
        if !ran && !name.starts_with("xx") {
            let chunk = af.name(p.node).to_owned();
            af.hit_chunk(sim, &chunk, shot.tnt(), &assets.toughness);
        }
        if !name.starts_with("xx") {
            // Passing through a chunk costs energy as armour of 33.3 × the fraction of the
            // 50 m trace to the next part.
            let span = if i + 1 == n {
                0.02
            } else {
                parts[i + 1].t - p.t
            };
            armour(33.333 * span, shot);
            if af.random() < 0.05 {
                shot.power = 0.0;
            }
        }
    }
    // Models without collision hulls: the old generic damage on the first chunk struck.
    if generic {
        if let Some(p) = parts.first() {
            af.shot(sim, p.node, shot.tnt(), &assets.toughness);
        }
    }
}
