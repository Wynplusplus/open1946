//! The mission's other aircraft: AI flights spawned from `[Wing]` sections, flown by `ai::AiPilot`
//! with the AI flight model, drawn with shared meshes.

use crate::ai::{AiPilot, Contact, Phase};
use crate::airframe::Airframe;
use crate::weapons::{self, Bullet, Gun, Projectile, Rng, Store};
use crate::world::{self, AircraftAssets};
use glam::{DMat4, DVec3};
use il2_fm::Sim;
use il2_formats::map::HeightField;
use il2_formats::mission::{Mission, Wing};
use il2_render::{GpuMesh, MeshData, Renderer};
use il2_vfs::Vfs;
use std::collections::HashMap;

struct ClassAssets {
    assets: AircraftAssets,
    mesh: Option<MeshData>,
    gpu: Option<GpuMesh>,
}

pub struct AiPlane {
    pub name: String,
    pub army: u8,
    class: usize,
    pub sim: Sim,
    pub pilot: AiPilot,
    pub airframe: Airframe,
    gpu: Option<GpuMesh>,
    trace_t: f64,
    pub guns: Vec<Gun>,
    /// Bombs and rockets.
    pub stores: Vec<Store>,
    /// Its flight and whether the player leads that flight.
    wing: String,
    player_leads: bool,
    /// Who it is fighting: contact id (0 player, 1 + n AI) and whether it attacks or evades.
    engaged: Option<(usize, bool)>,
    think: f64,
}

pub struct Traffic {
    classes: Vec<ClassAssets>,
    pub planes: Vec<AiPlane>,
    /// The player's side.
    pub player_army: u8,
}

/// Regiment name -> army (1 red, 2 blue) from `PaintSchemes/regiments.ini`.
pub fn armies(vfs: &Vfs) -> HashMap<String, u8> {
    let mut out = HashMap::new();
    let Ok(d) = vfs.read("PaintSchemes/regiments.ini") else {
        return out;
    };
    let f = il2_formats::SectFile::parse(&d);
    for (sect, army) in [("Red", 1u8), ("Blue", 2u8)] {
        if let Some(s) = f.section_ci(sect) {
            for (k, _) in &s.entries {
                out.insert(k.to_ascii_lowercase(), army);
            }
        }
    }
    out
}

/// A wing's army: its name without the squadron and flight digits names the regiment.
pub fn wing_army(armies: &HashMap<String, u8>, wing: &str) -> u8 {
    let reg = &wing[..wing.len().saturating_sub(2)];
    armies.get(&reg.to_ascii_lowercase()).copied().unwrap_or(1)
}

impl Traffic {
    /// Spawns every AI aircraft of the mission (the player's wingmen included).
    pub fn from_mission(
        vfs: &Vfs,
        fms: &il2_data::FlightModels,
        m: &Mission,
        field: &HeightField,
        runways: &[il2_formats::statics::Placed],
    ) -> Traffic {
        let armies = armies(vfs);
        let classes_db = il2_data::Classes::new(vfs, &fms.keys);
        let mut armoury = weapons::Armoury::default();
        let player_army = m
            .player_wing
            .as_deref()
            .map_or(1, |w| wing_army(&armies, w));
        let mut t = Traffic {
            classes: Vec::new(),
            planes: Vec::new(),
            player_army,
        };
        let mut by_class: HashMap<String, usize> = HashMap::new();
        for wing in &m.wings {
            let is_player = m.player_wing.as_deref() == Some(&wing.name);
            let class = match by_class.get(&wing.class) {
                Some(&c) => c,
                None => match world::load_aircraft_with(vfs, fms, &wing.class, false) {
                    Ok(mut assets) => {
                        let mesh = std::mem::take(&mut assets.mesh);
                        t.classes.push(ClassAssets {
                            assets,
                            mesh: Some(mesh),
                            gpu: None,
                        });
                        by_class.insert(wing.class.clone(), t.classes.len() - 1);
                        t.classes.len() - 1
                    }
                    Err(e) => {
                        eprintln!("AI flight {}: {e}", wing.name);
                        continue;
                    }
                },
            };
            for i in 0..wing.planes as usize {
                if is_player && i == m.player_num as usize {
                    continue;
                }
                let a = &t.classes[class].assets;
                let (sim, mut pilot) = spawn(a, wing, i, field, runways);
                if let Some(l) = wing
                    .waypoints
                    .iter()
                    .find(|w| w.kind.starts_with("LANDING"))
                {
                    if let Some((x, y, heading)) = world::takeoff_spot(runways, l.x, l.y) {
                        let (fx, fy) = (heading.cos(), heading.sin());
                        let (tx, ty) = (x + fx * 300.0, y + fy * 300.0);
                        pilot.landing_runway =
                            Some((DVec3::new(tx, ty, field.height_at(tx, ty)), heading));
                    }
                }
                let mut sim = sim;
                sim.ai = true;
                sim.structure.twin_fin = a.twin_fin;
                sim.fuel *= (wing.fuel_percent / 100.0).clamp(0.0, 1.0);
                let airframe = Airframe::new(
                    a.rig.names.clone(),
                    a.rig.parents.clone(),
                    &a.hidden,
                    a.gear_chunks.clone(),
                );
                t.planes.push(AiPlane {
                    name: format!("{}{}", wing.name, i),
                    army: wing_army(&armies, &wing.name),
                    class,
                    sim,
                    pilot,
                    airframe,
                    gpu: None,
                    trace_t: 0.0,
                    guns: Self::guns(&mut armoury, &classes_db, &wing.class, &wing.weapons, a),
                    stores: {
                        let w = classes_db.aircraft_weapons(&wing.class).unwrap_or_default();
                        weapons::mount_stores(&classes_db, &w, &wing.weapons, &a.hook_frames)
                    },
                    wing: wing.name.clone(),
                    player_leads: is_player && m.player_num == 0,
                    engaged: None,
                    think: 0.0,
                });
            }
        }
        eprintln!(
            "AI: {} aircraft in {} classes",
            t.planes.len(),
            t.classes.len()
        );
        t
    }

    /// One AI aircraft flying straight from `pos` towards `toward` (tests and gunnery practice).
    pub fn single(
        vfs: &Vfs,
        fms: &il2_data::FlightModels,
        class: &str,
        pos: DVec3,
        toward: DVec3,
        speed: f64,
    ) -> Option<Traffic> {
        let mut assets = world::load_aircraft_with(vfs, fms, class, false)
            .map_err(|e| eprintln!("target {class}: {e}"))
            .ok()?;
        let mesh = std::mem::take(&mut assets.mesh);
        let mut sim = world::spawn_at(assets.fm.clone(), pos, toward, speed);
        sim.ai = true;
        sim.structure.twin_fin = assets.twin_fin;
        let d = toward - pos;
        let far = pos + d.normalize() * 200_000.0;
        let wp = il2_formats::mission::Waypoint {
            kind: "NORMFLY".into(),
            x: far.x,
            y: far.y,
            z: pos.z,
            speed: speed * 3.6,
            target: None,
        };
        let mut pilot = AiPilot::new(vec![wp], 0, 0, 2);
        let classes_db = il2_data::Classes::new(vfs, &fms.keys);
        let guns = Self::guns(
            &mut weapons::Armoury::default(),
            &classes_db,
            class,
            "default",
            &assets,
        );
        pilot.ct.gear = 0.0;
        let airframe = Airframe::new(
            assets.rig.names.clone(),
            assets.rig.parents.clone(),
            &assets.hidden,
            assets.gear_chunks.clone(),
        );
        Some(Traffic {
            classes: vec![ClassAssets {
                assets,
                mesh: Some(mesh),
                gpu: None,
            }],
            player_army: 2,
            planes: vec![AiPlane {
                name: "target".into(),
                army: 1,
                class: 0,
                sim,
                pilot,
                airframe,
                gpu: None,
                trace_t: 0.0,
                guns,
                stores: Vec::new(),
                wing: "target".into(),
                player_leads: false,
                engaged: None,
                think: 0.0,
            }],
        })
    }

    fn guns(
        armoury: &mut weapons::Armoury,
        classes: &il2_data::Classes,
        class: &str,
        loadout: &str,
        a: &AircraftAssets,
    ) -> Vec<Gun> {
        let w = classes.aircraft_weapons(class).unwrap_or_default();
        weapons::mount(armoury, classes, &w, loadout, &a.hook_frames)
    }

    /// The tracer and bullet types the AI guns use, for `effects::Tracers`.
    pub fn gun_specs(&self) -> Vec<std::sync::Arc<weapons::GunSpec>> {
        self.planes
            .iter()
            .flat_map(|p| p.guns.iter().map(|g| g.spec.clone()))
            .collect()
    }

    /// One AI aircraft with a loadout flying a route from `pos` (tests).
    pub fn with_route(
        vfs: &Vfs,
        fms: &il2_data::FlightModels,
        class: &str,
        loadout: &str,
        pos: DVec3,
        speed: f64,
        way: Vec<il2_formats::mission::Waypoint>,
    ) -> Option<Traffic> {
        let toward = way
            .first()
            .map_or(pos + DVec3::X, |w| DVec3::new(w.x, w.y, pos.z));
        let mut t = Self::single(vfs, fms, class, pos, toward, speed)?;
        let classes_db = il2_data::Classes::new(vfs, &fms.keys);
        let w = classes_db.aircraft_weapons(class).unwrap_or_default();
        let a = &t.classes[0].assets;
        let p = &mut t.planes[0];
        p.stores = weapons::mount_stores(&classes_db, &w, loadout, &a.hook_frames);
        p.pilot.way = way;
        p.pilot.cur = 0;
        p.name = "test".into();
        Some(t)
    }

    /// One line per aircraft: where it is and what it does (diagnostics).
    pub fn report(&self, field: &HeightField) -> String {
        let mut out = String::new();
        for p in &self.planes {
            let s = &p.sim;
            out += &format!(
                "{:10} army {} {:?} wp {} pos {:.0} {:.0} agl {:.0} {:.0} km/h{}\n",
                p.name,
                p.army,
                p.pilot.phase,
                p.pilot.cur,
                s.pos.x,
                s.pos.y,
                s.pos.z - field.height_at(s.pos.x, s.pos.y),
                s.vel.length() * 3.6,
                if s.structure.destroyed {
                    " DESTROYED"
                } else {
                    ""
                }
            );
        }
        out
    }

    /// Position, velocity and army of every AI aircraft still flying.
    pub fn contacts(&self) -> Vec<(DVec3, DVec3, u8)> {
        self.planes
            .iter()
            .filter(|p| !p.sim.structure.destroyed)
            .map(|p| (p.sim.pos, p.sim.vel, p.army))
            .collect()
    }

    /// Hit targets for the AI aircraft (ids 1 + index).
    pub fn targets(&self, out: &mut Vec<crate::hitbox::Target>) {
        for (n, p) in self.planes.iter().enumerate() {
            let c = &self.classes[p.class].assets;
            let mut nodes = world::pose(c, &p.sim.ct);
            p.airframe.apply(&mut nodes, &p.sim);
            out.push(crate::hitbox::Target::new(
                1 + n,
                p.sim.pos,
                p.sim.ori,
                &nodes,
                &c.boxes,
            ));
        }
    }

    /// Damage to AI aircraft `n` (its index) in chunk `node`.
    pub fn shot(&mut self, n: usize, node: usize, tnt: f64) {
        let Some(p) = self.planes.get_mut(n) else {
            return;
        };
        let toughness = self.classes[p.class].assets.toughness;
        p.airframe.shot(&mut p.sim, node, tnt, &toughness);
    }

    /// One simulation tick for every AI aircraft: choose whom to fight, fly, shoot.
    /// `player` is the player's aircraft (position, velocity, army, still flying).
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        field: &HeightField,
        dt: f64,
        player: (DVec3, DVec3, u8, bool),
        rng: &mut Rng,
        bullets: &mut Vec<Bullet>,
        projectiles: &mut Vec<Projectile>,
        target_of: &dyn Fn(&str) -> Option<DVec3>,
    ) {
        let ground = |x: f64, y: f64| field.height_at(x, y);
        // Everyone the pilots can see: id, position, velocity, army.
        let mut contacts: Vec<(usize, DVec3, DVec3, u8)> = Vec::new();
        if player.3 {
            contacts.push((0, player.0, player.1, player.2));
        }
        for (n, p) in self.planes.iter().enumerate() {
            if !p.sim.structure.destroyed && !matches!(p.pilot.phase, Phase::Lineup(_)) {
                contacts.push((1 + n, p.sim.pos, p.sim.vel, p.army));
            }
        }
        // Flight leaders: the lowest-numbered aircraft of each flight still flying its route.
        let leaders: Vec<Option<(DVec3, DVec3)>> = self
            .planes
            .iter()
            .map(|p| {
                if p.pilot.index == 0 {
                    return None;
                }
                if p.player_leads {
                    return player.3.then_some((player.0, player.1));
                }
                self.planes
                    .iter()
                    .filter(|q| {
                        q.wing == p.wing
                            && q.pilot.index < p.pilot.index
                            && !q.sim.structure.destroyed
                            && q.pilot.phase == Phase::Cruise
                    })
                    .min_by_key(|q| q.pilot.index)
                    .map(|q| (q.sim.pos, q.sim.vel))
            })
            .collect();
        for (n, p) in self.planes.iter_mut().enumerate() {
            let g = field.height_at(p.sim.pos.x, p.sim.pos.y);
            let fighter = self.classes[p.class].assets.fighter;
            let mut fire = false;
            let alive = !p.sim.structure.destroyed;
            p.think -= dt;
            if alive && p.pilot.phase == Phase::Cruise && p.think <= 0.0 {
                p.think = 0.5;
                p.engaged = engage(p, n, fighter, &contacts);
            }
            let contact = p.engaged.and_then(|(id, attack)| {
                contacts
                    .iter()
                    .find(|c| c.0 == id)
                    .map(|c| (Contact { pos: c.1, vel: c.2 }, attack))
            });
            let inputs = match (&contact, alive) {
                (Some((c, true)), true) => {
                    let speed = p
                        .guns
                        .first()
                        .and_then(|g| g.spec.bullets.first())
                        .map_or(800.0, |b| b.speed);
                    let (i, f) = p.pilot.attack(&p.sim, c, g, speed);
                    fire = f;
                    i
                }
                (Some((c, false)), true) => p.pilot.evade(&p.sim, c, g),
                _ => {
                    p.engaged = None;
                    match leaders[n].filter(|_| p.pilot.phase == Phase::Cruise) {
                        // Echelon right, 40 m back and 40 m out per place.
                        Some((lp, lv)) => {
                            let k = p.pilot.index as f64;
                            p.pilot.follow(
                                &p.sim,
                                lp,
                                lv,
                                DVec3::new(-40.0 * k, -40.0 * k, 0.0),
                                g,
                                &ground,
                                dt,
                            )
                        }
                        None => p.pilot.update(&p.sim, g, &ground, dt),
                    }
                }
            };
            p.sim.tick(&inputs, &ground);
            if alive {
                let enemies: Vec<(DVec3, DVec3)> = contacts
                    .iter()
                    .filter(|c| c.0 != 1 + n && c.3 != p.army)
                    .map(|c| (c.1, c.2))
                    .collect();
                let arcs = self.classes[p.class].assets.turret_arcs.as_deref();
                crate::gunners::fire(
                    &mut p.guns,
                    arcs,
                    p.sim.pos,
                    p.sim.ori,
                    p.sim.vel,
                    &enemies,
                    p.pilot.skill,
                    dt,
                    rng,
                    1 + n,
                    bullets,
                );
                // Level bombing at a ground-attack waypoint: release when the bombs' fall carries them onto
                // the target.
                let mut bomb = false;
                if p.engaged.is_none() && p.pilot.phase == Phase::Cruise {
                    if let Some(w) = p
                        .pilot
                        .way
                        .get(p.pilot.cur)
                        .filter(|w| w.kind.starts_with("GATTACK"))
                    {
                        let t = w
                            .target
                            .as_ref()
                            .and_then(|(name, _)| target_of(name))
                            .unwrap_or_else(|| DVec3::new(w.x, w.y, 0.0));
                        let t = DVec3::new(t.x, t.y, field.height_at(t.x, t.y));
                        let h = (p.sim.pos.z - t.z).max(0.0);
                        let fall = (2.0 * h / 9.81).sqrt() * 1.05;
                        // Drag shortens the bombs' travel by about 3.5 %.
                        let impact = p.sim.pos.truncate() + p.sim.vel.truncate() * fall * 0.965;
                        let toward =
                            (t.truncate() - p.sim.pos.truncate()).dot(p.sim.vel.truncate()) > 0.0;
                        let has_bombs = p.stores.iter().any(|s| s.trigger == 3 && s.count > 0);
                        // Release as the predicted impact passes the target, if not too far to the side.
                        let vh = p.sim.vel.truncate().normalize_or_zero();
                        let rel = t.truncate() - impact;
                        let (along, cross) = (rel.dot(vh), rel.perp_dot(vh).abs());
                        if has_bombs && toward && along < 15.0 && along > -80.0 && cross < 200.0 {
                            bomb = true;
                        }
                        if std::env::var("IL2_TRACE_AI").is_ok_and(|n| n == p.name) && bomb {
                            eprintln!("{}: bombs away, predicted miss {along:.0} m along, {cross:.0} m across", p.name);
                        }
                        if !has_bombs
                            && p.pilot.cur + 1 < p.pilot.way.len()
                            && (p.sim.pos.truncate() - t.truncate()).length() < 1500.0
                        {
                            p.pilot.cur += 1;
                        }
                    }
                }
                weapons::release(
                    &mut p.stores,
                    &[false, false, false, bomb],
                    p.sim.pos,
                    p.sim.ori,
                    p.sim.vel,
                    dt,
                    rng,
                    projectiles,
                );
                let t = [fire, fire, false, false];
                let recoil = weapons::fire(
                    &mut p.guns,
                    &t,
                    p.sim.pos,
                    p.sim.ori,
                    p.sim.vel,
                    dt,
                    rng,
                    1 + n,
                    bullets,
                );
                if recoil != DVec3::ZERO {
                    let m = p.sim.mass();
                    p.sim.vel += p.sim.ori * recoil / m;
                }
            }
            p.airframe.update(&mut p.sim, field, dt);
            if !std::env::var("IL2_TRACE_AI").is_ok_and(|n| n == p.name) {
                p.airframe.messages.clear();
            }
            if std::env::var("IL2_TRACE_AI").is_ok_and(|n| n == p.name) {
                p.trace_t += dt;
                if p.trace_t >= 2.0 {
                    p.trace_t = 0.0;
                    let s = &p.sim;
                    eprintln!(
                        "{} {:?} {:?} wp {} pos {:.0} {:.0} {:.0} agl {:.0} {:.0} km/h pitch {:+.1} bank {:+.0} aoa {:+.1} ct a{:+.2} e{:+.2} r{:+.2} t{:.2} fire {fire} {}",
                        p.name, p.pilot.phase, p.engaged, p.pilot.cur, s.pos.x, s.pos.y, s.pos.z, s.pos.z - g, s.vel.length() * 3.6,
                        (s.ori * DVec3::X).z.asin().to_degrees(), s.bank_deg(), s.aoa,
                        s.ct.aileron, s.ct.elevator, s.ct.rudder, s.ct.throttle,
                        p.airframe.messages.join(", ")
                    );
                }
            }
        }
    }

    /// Places every AI aircraft relative to the camera `eye` and returns the meshes to draw.
    pub fn prepare(&mut self, r: &Renderer, eye: DVec3) -> Vec<&GpuMesh> {
        for c in &mut self.classes {
            if c.gpu.is_none() {
                if let Some(m) = c.mesh.take() {
                    c.gpu = Some(r.upload(&m));
                }
            }
        }
        for p in &mut self.planes {
            let c = &self.classes[p.class];
            let Some(base) = &c.gpu else { continue };
            let g = p.gpu.get_or_insert_with(|| r.instance_of(base));
            r.set_transform(
                g,
                DMat4::from_rotation_translation(p.sim.ori, p.sim.pos - eye).as_mat4(),
            );
            let mut nodes = world::pose(&c.assets, &p.sim.ct);
            p.airframe.apply(&mut nodes, &p.sim);
            r.set_nodes(g, &nodes);
        }
        self.planes.iter().filter_map(|p| p.gpu.as_ref()).collect()
    }
}

/// Whom to fight (OpenIL2's own rules, `docs/behavior/ai.md`, "Air combat"): an enemy close
/// behind is evaded; fighters attack the nearest enemy they see; others keep to their route.
fn engage(
    p: &AiPlane,
    n: usize,
    fighter: bool,
    contacts: &[(usize, DVec3, DVec3, u8)],
) -> Option<(usize, bool)> {
    let me = 1 + n;
    let sight = [2500.0, 3500.0, 4500.0, 5500.0][p.pilot.skill.min(3) as usize];
    let fwd = p.sim.ori * DVec3::X;
    let mut best: Option<(usize, f64)> = None;
    for &(id, pos, _, army) in contacts {
        if id == me || army == p.army {
            continue;
        }
        let rel = pos - p.sim.pos;
        let d = rel.length();
        // Someone close on our tail.
        if d < 600.0 && rel.normalize_or_zero().dot(fwd) < -0.5 {
            return Some((id, false));
        }
        if d < sight && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((id, d));
        }
    }
    if !fighter {
        return None;
    }
    // Keep the current target unless it got away.
    if let Some((id, true)) = p.engaged {
        if contacts
            .iter()
            .any(|c| c.0 == id && (c.1 - p.sim.pos).length() < sight * 1.5)
        {
            return Some((id, true));
        }
    }
    best.map(|(id, _)| (id, true))
}

/// Where and how a wing's `i`-th aircraft starts: lined up on the runway for a `TAKEOFF` waypoint
/// (each one 40 m further back, 15 s later), else in the air at the first waypoint in echelon.
fn spawn(
    a: &AircraftAssets,
    wing: &Wing,
    i: usize,
    field: &HeightField,
    runways: &[il2_formats::statics::Placed],
) -> (Sim, AiPilot) {
    let skill = wing.skills.get(i).copied().unwrap_or(1);
    let way = wing.waypoints.clone();
    let Some(w0) = way.first() else {
        let sim = world::spawn_airborne(a.fm.clone(), field, 1000.0, 100.0);
        return (sim, AiPilot::new(way, 0, i, skill));
    };
    if w0.kind.starts_with("TAKEOFF") {
        if let Some((x, y, heading)) = world::takeoff_spot(runways, w0.x, w0.y) {
            let back = 40.0 * i as f64;
            let side = if i % 2 == 1 { 12.0 } else { 0.0 };
            let (fx, fy) = (heading.cos(), heading.sin());
            let (h, pitch) = a.parked;
            let sim = world::spawn_on_ground(
                a.fm.clone(),
                field,
                x - fx * back - fy * side,
                y - fy * back + fx * side,
                heading,
                h,
                pitch,
            );
            let mut p = AiPilot::new(way, 0, i, skill);
            p.phase = Phase::Lineup(3.0 + 15.0 * i as f64);
            return (sim, p);
        }
    }
    let next = way.get(1).unwrap_or(w0);
    let start = DVec3::new(w0.x, w0.y, w0.z.max(field.height_at(w0.x, w0.y) + 300.0));
    let dir = (DVec3::new(next.x, next.y, start.z) - start).normalize_or(DVec3::Y);
    let right = DVec3::new(dir.y, -dir.x, 0.0);
    let pos = start - dir * (40.0 * i as f64) + right * (30.0 * i as f64);
    let sim = world::spawn_at(a.fm.clone(), pos, pos + dir, (w0.speed / 3.6).max(70.0));
    let first = if w0.kind.starts_with("TAKEOFF") { 1 } else { 0 };
    let mut p = AiPilot::new(way.clone(), first, i, skill);
    p.ct.gear = 0.0;
    (sim, p)
}
