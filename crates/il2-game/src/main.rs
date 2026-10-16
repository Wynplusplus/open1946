mod ai;
mod airframe;
mod audio;
mod cockpit;
mod damage;
mod effects;
mod forest;
mod hitbox;
mod hud;
mod joystick;
mod mission_objects;
mod objects;
mod particles;
mod roads;
mod sky;
mod traffic;
mod view;
mod weapons;
mod world;

use clap::Parser;
#[cfg(feature = "gamepad")]
use gilrs::{Axis, Gilrs};
use glam::{DMat4, DQuat, DVec3, Mat4};
use il2_fm::{Inputs, Sim, TICK};
use il2_formats::map::HeightField;
use il2_render::{GpuMesh, Renderer};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::WindowId;

#[derive(Parser)]
#[command(about = "OpenIL2: fly an aircraft over a map")]
struct Args {
    #[arg(long, env = "IL2_ROOT")]
    root: PathBuf,
    #[arg(long, default_value = "maps/Slovakia/load.ini")]
    map: String,
    /// Aircraft class, as in missions.
    #[arg(long, default_value = "air.BF_109G6")]
    aircraft: String,
    /// Mission file (e.g. Missions/Single/SK/Bf-109F-4/Bf-109F-4_1.mis); sets map, aircraft and start.
    #[arg(long)]
    mission: Option<String>,
    /// Start position "x,y" in world metres (without a mission; default: map centre).
    #[arg(long, value_delimiter = ',')]
    at: Option<Vec<f64>>,
    /// Start altitude above ground, metres (without a mission).
    #[arg(long, default_value_t = 1500.0)]
    agl: f64,
    /// Joystick axes, e.g. "aileron=0,elevator=1,rudder=3,throttle=-2" (minus inverts).
    #[arg(long)]
    joy_map: Option<String>,
    /// Time of day, hours (overrides the mission's).
    #[arg(long)]
    time: Option<f64>,
    /// Fly the level-speed check for every aircraft in air.ini and print a table.
    #[arg(long)]
    survey: bool,
    /// An AI aircraft of this class 150 m ahead, flying straight (gunnery practice, tests).
    #[arg(long)]
    enemy: Option<String>,
    /// Leave out the mission's AI aircraft.
    #[arg(long)]
    no_ai: bool,
    /// Fly the aircraft by the AI around a square of waypoints and print its track.
    #[arg(long)]
    ai_test: bool,
    /// Fly headless level-flight tests instead of opening a window.
    #[arg(long)]
    test: bool,
    /// Fly hands-off for `--seconds`, render one chase-camera frame to this PNG and exit.
    #[arg(long)]
    screenshot: Option<PathBuf>,
    #[arg(long, default_value_t = 3.0)]
    seconds: f64,
    /// Screenshot: orbit camera "yaw,pitch,distance" (degrees relative to the aircraft heading, metres).
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    orbit: Option<Vec<f64>>,
    /// Weapon loadout (overrides the mission's), e.g. R1-SC500.
    #[arg(long)]
    loadout: Option<String>,
    /// Start pitched nose-down by this many degrees (without a mission).
    #[arg(long)]
    dive: Option<f64>,
    /// Screenshot: camera position "x,y,z" (world metres), looking at `--target`.
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    eye: Option<Vec<f64>>,
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    target: Option<Vec<f64>>,
    /// Screenshot: hold the triggers (all guns) during the simulated seconds.
    #[arg(long)]
    fire: bool,
    /// Screenshot: hold these inputs (aileron, elevator, rudder, flaps, gear).
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    hold: Option<Vec<f64>>,
    /// Screenshot: from the cockpit, looking "yaw,pitch" degrees off the nose.
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true, num_args = 0..)]
    cockpit: Option<Vec<f64>>,
    /// Screenshot: strike these chunks at the start (e.g. `WingLOut_D0,StabL_D0`).
    #[arg(long, value_delimiter = ',')]
    damage: Option<Vec<String>>,
}

/// Chase-camera view-projection and the eye position (scene is drawn relative to the eye).
fn chase_camera(s: &Sim, field: &HeightField, aspect: f32) -> (Mat4, DVec3) {
    let fwd = s.ori * DVec3::X;
    let flat = DVec3::new(fwd.x, fwd.y, 0.0).normalize_or(DVec3::Y);
    let mut eye = s.pos - flat * 22.0 + DVec3::Z * 5.0;
    eye.z = eye.z.max(field.height_at(eye.x, eye.y) + 2.0);
    let target = s.pos + DVec3::Z * 1.0;
    let view = DMat4::look_at_rh(DVec3::ZERO, target - eye, DVec3::Z);
    let proj = Mat4::perspective_rh(65f32.to_radians(), aspect, 0.5, 150_000.0);
    (proj * view.as_mat4(), eye)
}

/// The pilot's view: the eye at the cockpit's `CAMERA` hook, the head turned by (yaw, pitch).
/// Returns the world view-projection, the cockpit pass's (closer near plane) and the eye.
fn cockpit_camera(
    s: &Sim,
    cam: glam::Vec3,
    (yaw, pitch): (f64, f64),
    aspect: f32,
) -> (Mat4, Mat4, DVec3) {
    let eye = s.pos + s.ori * cam.as_dvec3();
    let head = s.ori * DQuat::from_rotation_z(yaw) * DQuat::from_rotation_y(-pitch);
    let view = DMat4::look_at_rh(DVec3::ZERO, head * DVec3::X, head * DVec3::Z).as_mat4();
    // 90° horizontal field of view, as the original's default.
    let fovy = 2.0 * (1.0f32 / aspect).atan();
    let world = Mat4::perspective_rh(fovy, aspect, 0.5, 150_000.0) * view;
    let near = Mat4::perspective_rh(fovy, aspect, 0.02, 20.0) * view;
    (world, near, eye)
}

fn gun_readings(guns: &[weapons::Gun]) -> Vec<cockpit::GunReading<'_>> {
    guns.iter()
        .map(|g| cockpit::GunReading {
            hook: g.hook.as_deref().unwrap_or(""),
            ammo: g.ammo,
            firing: g.firing,
            trigger: g.trigger,
        })
        .collect()
}

/// Structural failures, ground strikes and wreckage for one tick, with their dust and fire.
fn airframe_step(
    af: &mut airframe::Airframe,
    sim: &mut Sim,
    field: &HeightField,
    combat: &mut Combat,
    view: &mut view::WorldView,
) {
    af.update(sim, field, TICK);
    for (at, power) in std::mem::take(&mut af.crashes) {
        let what = if power > 0.0 {
            particles::Impact::Blast { power }
        } else {
            particles::Impact::Shot { mass: 1.0 }
        };
        particles::impact(&mut view.effects, at, what, false, true, &mut combat.rng);
        if power > 0.0 {
            combat.booms.push((at, power));
        }
    }
}

/// Everything bullets can hit this tick: the player (id 0) and the AI aircraft.
fn build_targets(
    sim: &Sim,
    assets: &world::AircraftAssets,
    af: &airframe::Airframe,
    traffic: Option<&traffic::Traffic>,
) -> Vec<hitbox::Target> {
    let mut nodes = world::pose(assets, &sim.ct);
    af.apply(&mut nodes, sim);
    let mut out = vec![hitbox::Target::new(
        0,
        sim.pos,
        sim.ori,
        &nodes,
        &assets.boxes,
    )];
    if let Some(t) = traffic {
        t.targets(&mut out);
    }
    out
}

/// Bullet hits on aircraft from the last step: kinetic energy as TNT (2.4·10⁻⁷ kg/J) plus half the
/// charge of high-explosive rounds.
fn apply_aircraft_hits(
    combat: &Combat,
    sim: &mut Sim,
    af: &mut airframe::Airframe,
    assets: &world::AircraftAssets,
    mut traffic: Option<&mut traffic::Traffic>,
) {
    for (b, h) in &combat.hits {
        let weapons::Hit::Aircraft(id, node, _) = h else {
            continue;
        };
        let ty = b.ty();
        let speed = b.vel.length();
        let mut tnt = 0.5 * ty.mass * speed * speed * 2.4e-7;
        if ty.power > 0.0 && ty.power_radius > 0.0 {
            tnt += 0.5 * (ty.power + 0.03 * ty.mass);
        }
        if std::env::var_os("IL2_TRACE").is_some() {
            eprintln!(
                "{} hit aircraft {id} in node {node} ({tnt:.4} kg TNT)",
                b.spec.name
            );
        }
        if *id == 0 {
            af.shot(sim, *node, tnt, &assets.toughness);
        } else if let Some(t) = traffic.as_deref_mut() {
            t.shot(id - 1, *node, tnt);
        }
    }
}

fn new_airframe(a: &world::AircraftAssets) -> airframe::Airframe {
    airframe::Airframe::new(
        a.rig.names.clone(),
        a.rig.parents.clone(),
        &a.hidden,
        a.gear_chunks.clone(),
    )
}

fn place(r: &Renderer, terrain: &il2_render::GpuTerrain, plane: &GpuMesh, s: &Sim, eye: DVec3) {
    r.set_transform(&terrain.mesh, Mat4::from_translation((-eye).as_vec3()));
    r.set_transform(
        plane,
        DMat4::from_rotation_translation(s.ori, s.pos - eye).as_mat4(),
    );
}

/// The player's guns and the bullets in flight.
struct Combat {
    guns: Vec<weapons::Gun>,
    stores: Vec<weapons::Store>,
    projectiles: Vec<weapons::Projectile>,
    bullets: Vec<weapons::Bullet>,
    rng: weapons::Rng,
    hits: Vec<(weapons::Bullet, weapons::Hit)>,
    /// Explosions of the last steps (point, kg TNT), for sound.
    booms: Vec<(DVec3, f64)>,
}

impl Combat {
    fn new(guns: Vec<weapons::Gun>, stores: Vec<weapons::Store>) -> Self {
        Combat {
            guns,
            stores,
            projectiles: Vec::new(),
            bullets: Vec::new(),
            rng: weapons::Rng::new(0x5EED),
            hits: Vec::new(),
            booms: Vec::new(),
        }
    }

    fn step(
        &mut self,
        sim: &mut Sim,
        triggers: &[bool; 4],
        field: &HeightField,
        view: &mut view::WorldView,
        targets: &[hitbox::Target],
    ) {
        let objects = &mut view.objects;
        let recoil = weapons::fire(
            &mut self.guns,
            triggers,
            sim.pos,
            sim.ori,
            sim.vel,
            TICK,
            &mut self.rng,
            0,
            &mut self.bullets,
        );
        if recoil != DVec3::ZERO {
            // Gun recoil acts on the aircraft.
            sim.vel += sim.ori * recoil / sim.mass();
        }
        weapons::release(
            &mut self.stores,
            triggers,
            sim.pos,
            sim.ori,
            sim.vel,
            TICK,
            &mut self.rng,
            &mut self.projectiles,
        );
        let booms = weapons::fly(&mut self.projectiles, TICK, field, objects, &mut self.rng);
        for &(at, power, killed) in &booms {
            eprintln!(
                "explosion of {power:.1} kg TNT at {:.0?}: {killed} objects destroyed",
                at
            );
        }
        self.hits.clear();
        weapons::advance(
            &mut self.bullets,
            TICK,
            field,
            objects,
            &mut self.rng,
            targets,
            &mut self.hits,
        );
        let fx = &mut view.effects;
        for &(at, power, _) in &booms {
            self.booms.push((at, power));
            particles::impact(
                fx,
                at,
                particles::Impact::Blast { power },
                at.z < 0.5,
                false,
                &mut self.rng,
            );
        }
        for (b, h) in &self.hits {
            let ty = b.ty();
            let (at, object) = match h {
                weapons::Hit::Ground(p) => (*p, false),
                weapons::Hit::Object(_, p, _) | weapons::Hit::Aircraft(_, _, p) => (*p, true),
            };
            let what = if ty.power > 0.0 && ty.power_radius > 0.0 {
                particles::Impact::Blast { power: ty.power }
            } else {
                particles::Impact::Shot { mass: ty.mass }
            };
            particles::impact(fx, at, what, at.z < 0.5 && !object, object, &mut self.rng);
            match h {
                weapons::Hit::Object(_, _, true) => {
                    eprintln!("{} destroyed a building", b.spec.name)
                }
                weapons::Hit::Object(_, _, false) if std::env::var_os("IL2_TRACE").is_some() => {
                    eprintln!("hit object")
                }
                weapons::Hit::Ground(p) if std::env::var_os("IL2_TRACE").is_some() => {
                    eprintln!("hit ground at {:.0?}", p)
                }
                _ => {}
            }
        }
        fx.update(TICK);
    }
}

struct Game {
    sim: Sim,
    airframe: airframe::Airframe,
    traffic: Option<traffic::Traffic>,
    assets: world::AircraftAssets,
    field: HeightField,
    terrain_data: Option<il2_render::TerrainData>,
    plane_data: Option<il2_render::MeshData>,
    renderer: Option<Renderer>,
    terrain: Option<il2_render::GpuTerrain>,
    plane: Option<GpuMesh>,
    cockpit_state: Option<cockpit::Cockpit>,
    cockpit: Option<GpuMesh>,
    /// Hours since midnight.
    time_of_day: f64,
    /// In the cockpit (F1), head turned by (yaw, pitch).
    in_cockpit: bool,
    head: (f64, f64),
    view: view::WorldView,
    env: il2_render::Environment,
    combat: Combat,
    vfs: Arc<il2_vfs::Vfs>,
    audio: audio::Audio,
    hud: hud::Hud,
    keys: HashSet<KeyCode>,
    #[cfg(feature = "gamepad")]
    gilrs: Option<Gilrs>,
    joystick: Option<(joystick::Joystick, joystick::AxisMap)>,
    inputs: Inputs,
    last: Instant,
    acc: f64,
    /// Free orbit camera around the aircraft: (yaw, pitch, distance), relative to the heading.
    orbit: Option<(f64, f64, f64)>,
    dragging: bool,
    cursor: Option<(f64, f64)>,
}

/// The speed bar: flight and engine readouts.
fn hud_lines(s: &Sim, field: &HeightField) -> Vec<String> {
    let fwd = s.ori * DVec3::X;
    let heading = (90.0 - fwd.y.atan2(fwd.x).to_degrees()).rem_euclid(360.0);
    let agl = s.pos.z - field.height_at(s.pos.x, s.pos.y);
    let mut l = vec![format!(
        "Speed {:.0} km/h   Alt {:.0} m ({:.0} AGL)   Hdg {:03.0}°   Thr {:.0}%{}",
        s.ias * 3.6,
        s.pos.z,
        agl,
        heading,
        s.ct.throttle * 100.0,
        if s.ct.wep { " WEP" } else { "" }
    )];
    if let Some(e) = s.engines.first() {
        l.push(format!(
            "Engine {}  {:.0} rpm   Oil {:.0}°   Water {:.0}°   Rad {}   Prop {}   Fuel {:.0} kg",
            match e.stage {
                0 => "off",
                1..=5 => "starting",
                6 => "on",
                7 => "dead",
                _ => "seized",
            },
            e.rpm(),
            e.t_oil_out,
            e.t_water,
            e.radiator_manual
                .map_or("auto".to_owned(), |r| format!("{:.0}%", r * 100.0)),
            e.prop_manual
                .map_or("auto".to_owned(), |p| format!("{:.0}%", p * 100.0)),
            s.fuel
        ));
    }
    l.push(format!(
        "Flaps {:.0}%   Gear {}   Trim {:+.2}{}{}",
        s.ct.flaps * 100.0,
        if s.ct.gear > 0.98 {
            "down"
        } else if s.ct.gear < 0.02 {
            "up"
        } else {
            "moving"
        },
        s.trim[1] * 100.0,
        if s.airbrake > 0.5 { "   Air brake" } else { "" },
        if s.on_ground { "   On ground" } else { "" }
    ));
    l
}

/// Orbit camera "yaw, pitch, distance" (radians relative to the aircraft heading, metres).
fn orbit_camera(
    s: &Sim,
    field: &HeightField,
    aspect: f32,
    (yaw, pitch, dist): (f64, f64, f64),
) -> (Mat4, DVec3) {
    let fwd = s.ori * DVec3::X;
    let heading = fwd.y.atan2(fwd.x);
    let a = heading + yaw;
    let mut eye =
        s.pos + DVec3::new(a.cos() * pitch.cos(), a.sin() * pitch.cos(), pitch.sin()) * dist;
    eye.z = eye.z.max(field.height_at(eye.x, eye.y) + 1.0);
    let view = DMat4::look_at_rh(DVec3::ZERO, s.pos - eye, DVec3::Z);
    let proj = Mat4::perspective_rh(55f32.to_radians(), aspect, 0.3, 150_000.0);
    (proj * view.as_mat4(), eye)
}

/// Keyboard stick: holds deflect at a fixed rate, release recenters.
fn key_axis(cur: f64, neg: bool, pos: bool, dt: f64) -> f64 {
    let target = match (neg, pos) {
        (true, false) => -1.0,
        (false, true) => 1.0,
        _ => 0.0,
    };
    let rate = if target == 0.0 { 4.0 } else { 1.5 };
    cur + (target - cur).clamp(-rate * dt, rate * dt)
}

impl Game {
    fn update_inputs(&mut self, dt: f64) {
        let k = |c: KeyCode| self.keys.contains(&c);
        let i = &mut self.inputs;
        i.elevator = key_axis(i.elevator, k(KeyCode::ArrowUp), k(KeyCode::ArrowDown), dt);
        i.aileron = key_axis(i.aileron, k(KeyCode::ArrowLeft), k(KeyCode::ArrowRight), dt);
        i.rudder = key_axis(i.rudder, k(KeyCode::KeyZ), k(KeyCode::KeyX), dt);
        if k(KeyCode::PageUp) || k(KeyCode::Equal) {
            i.throttle = (i.throttle + 0.5 * dt).min(1.1);
        }
        if k(KeyCode::PageDown) || k(KeyCode::Minus) {
            i.throttle = (i.throttle - 0.5 * dt).max(0.0);
        }
        i.brakes = if k(KeyCode::KeyB) { 1.0 } else { 0.0 };
        if let Some((j, map)) = self.joystick.as_mut() {
            j.poll();
            let dz = |v: f64| if v.abs() < 0.03 { 0.0 } else { v };
            if let Some(v) = j.axis(map.aileron) {
                i.aileron = dz(v);
            }
            if let Some(v) = j.axis(map.elevator) {
                i.elevator = dz(v);
            }
            if let Some(v) = j.axis(map.rudder) {
                i.rudder = dz(v);
            }
            if let Some(v) = j.axis(map.throttle) {
                // Full travel maps to 0..100 %; boost stays on the keyboard.
                if !i.wep {
                    i.throttle = (v + 1.0) * 0.5;
                }
            }
            i.brakes = i.brakes.max(if j.buttons[1] { 1.0 } else { 0.0 });
        }
        #[cfg(feature = "gamepad")]
        if let Some(g) = self.gilrs.as_mut() {
            while g.next_event().is_some() {}
            if let Some((_, pad)) = g.gamepads().next() {
                let dz = |v: f32| if v.abs() < 0.08 { 0.0 } else { v as f64 };
                let (x, y) = (
                    dz(pad.value(Axis::LeftStickX)),
                    dz(pad.value(Axis::LeftStickY)),
                );
                if x != 0.0 {
                    i.aileron = x;
                }
                if y != 0.0 {
                    i.elevator = y;
                }
                let r = dz(pad.value(Axis::RightStickX));
                if r != 0.0 {
                    i.rudder = r;
                }
            }
        }
    }

    fn frame(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f64().min(0.25);
        self.last = now;
        self.update_inputs(dt);
        self.acc += dt;
        let field = &self.field;
        let ground = |x: f64, y: f64| field.height_at(x, y);
        let k = |c| self.keys.contains(&c);
        let triggers = [
            k(KeyCode::Space),
            k(KeyCode::Space) || k(KeyCode::Enter),
            k(KeyCode::KeyU),
            k(KeyCode::KeyK),
        ];
        while self.acc >= TICK {
            self.sim.tick(&self.inputs, &ground);
            let targets = build_targets(
                &self.sim,
                &self.assets,
                &self.airframe,
                self.traffic.as_ref(),
            );
            self.combat
                .step(&mut self.sim, &triggers, field, &mut self.view, &targets);
            apply_aircraft_hits(
                &self.combat,
                &mut self.sim,
                &mut self.airframe,
                &self.assets,
                self.traffic.as_mut(),
            );
            airframe_step(
                &mut self.airframe,
                &mut self.sim,
                field,
                &mut self.combat,
                &mut self.view,
            );
            if let Some(t) = self.traffic.as_mut() {
                let pl = (
                    self.sim.pos,
                    self.sim.vel,
                    t.player_army,
                    !self.sim.structure.destroyed,
                );
                t.step(
                    field,
                    TICK,
                    pl,
                    &mut self.combat.rng,
                    &mut self.combat.bullets,
                );
            }
            self.acc -= TICK;
        }
        let (Some(r), Some(terrain), Some(plane)) =
            (self.renderer.as_mut(), &self.terrain, &self.plane)
        else {
            return;
        };

        // Chase camera, rendered relative to the eye to keep precision far from the origin.
        let s = &self.sim;
        let cam = self
            .cockpit_state
            .as_ref()
            .map(|c| c.eye)
            .filter(|_| self.in_cockpit && self.orbit.is_none() && self.cockpit.is_some());
        let (vp, eye, fg_vp) = match (self.orbit, cam) {
            (Some(o), _) => {
                let (vp, eye) = orbit_camera(s, field, r.aspect(), o);
                (vp, eye, None)
            }
            (None, Some(c)) => {
                let (vp, near, eye) = cockpit_camera(s, c, self.head, r.aspect());
                (vp, eye, Some(near))
            }
            (None, None) => {
                let (vp, eye) = chase_camera(s, field, r.aspect());
                (vp, eye, None)
            }
        };
        place(r, terrain, plane, s, eye);
        self.time_of_day = (self.time_of_day + dt / 3600.0).rem_euclid(24.0);
        if let (Some(c), Some(cs), Some(_)) = (&self.cockpit, self.cockpit_state.as_mut(), fg_vp) {
            r.set_transform(
                c,
                DMat4::from_rotation_translation(s.ori, s.pos - eye).as_mat4(),
            );
            r.set_nodes(
                c,
                &cs.pose(s, &gun_readings(&self.combat.guns), self.time_of_day, dt),
            );
        }
        let mut nodes = world::pose(&self.assets, &s.ct);
        self.airframe.apply(&mut nodes, s);
        r.set_nodes(plane, &nodes);
        // Sound: engines, guns, explosions (heard from the camera).
        let level = (1.0 / (1.0 + (s.pos - eye).length() / 40.0)) as f32;
        for (i, (p, e)) in s.ac.engines.iter().zip(&s.engines).enumerate() {
            let name = format!("motor.{}", p.sound_name.as_deref().unwrap_or("db605"));
            let preset = self
                .audio
                .preset(&self.vfs, &name)
                .or_else(|| self.audio.preset(&self.vfs, "motor.db605"));
            if let Some(pr) = preset {
                let rpm = if e.stage == 0 || e.stage >= 7 {
                    0.0
                } else {
                    e.rpm() as f32
                };
                self.audio.drive(
                    &self.vfs,
                    1 + i as u64,
                    &pr,
                    &[("rpm", rpm)],
                    audio::ENV_OUTSIDE,
                    level,
                );
            }
        }
        for (i, g) in self.combat.guns.iter().enumerate() {
            let key = 100 + i as u64;
            match (g.firing, g.spec.sound.as_deref()) {
                (true, Some(snd)) => {
                    if let Some(pr) = self.audio.preset(&self.vfs, snd) {
                        self.audio
                            .drive(&self.vfs, key, &pr, &[], audio::ENV_OUTSIDE, level * 0.5);
                    }
                }
                _ => self.audio.stop(key),
            }
        }
        for (at, power) in std::mem::take(&mut self.combat.booms) {
            let name = if power >= 50.0 {
                "explode.big"
            } else if power >= 1.0 {
                "explode.middle"
            } else {
                "explode.bullet"
            };
            if let Some(pr) = self.audio.preset(&self.vfs, name) {
                let d = (at - eye).length() as f32;
                self.audio.play(&self.vfs, &pr, 1.0 / (1.0 + d / 300.0));
            }
        }
        let (w, h) = (r.size().0 as f32, r.size().1 as f32);
        r.set_overlay(self.hud.build(
            w,
            h,
            &hud_lines(s, field),
            self.orbit.is_none() && fg_vp.is_none(),
            dt,
        ));
        let cockpit: Vec<&GpuMesh> = self.cockpit.iter().collect();
        let mut outside: Vec<&GpuMesh> = self
            .traffic
            .as_mut()
            .map(|t| t.prepare(r, eye))
            .unwrap_or_default();
        let fg = match fg_vp {
            Some(m) => Some((m, cockpit.as_slice())),
            None => {
                outside.push(plane);
                None
            }
        };
        self.view.draw(
            r,
            field,
            terrain,
            vp,
            eye,
            &outside,
            &self.combat.bullets,
            &self.combat.projectiles,
            fg,
            false,
        );

        for (i, e) in self.sim.engines.iter_mut().enumerate() {
            for ev in e.events.drain(..) {
                self.hud.message(format!("Engine {}: {ev}", i + 1));
            }
        }
        for m in self.airframe.messages.drain(..) {
            self.hud.message(m);
        }
        r.window.as_ref().unwrap().request_redraw();
    }
}

impl ApplicationHandler for Game {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.renderer.is_some() {
            return;
        }
        let window = Arc::new(
            el.create_window(winit::window::Window::default_attributes().with_title("OpenIL2"))
                .expect("window"),
        );
        let mut r = Renderer::new(window);
        r.env = self.env;
        self.terrain = self.terrain_data.take().map(|d| r.upload_terrain(&d));
        self.plane = self.plane_data.take().map(|d| r.upload(&d));
        self.cockpit = self
            .cockpit_state
            .as_mut()
            .and_then(|c| c.mesh.take())
            .map(|d| r.upload(&d));
        self.view.upload(&r);
        if let Some(a) = &self.hud.atlas {
            r.set_overlay_atlas(a);
        }
        self.renderer = Some(r);
        self.last = Instant::now();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => {
                if let Some(r) = self.renderer.as_mut() {
                    r.resize(s.width, s.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    if event.state == ElementState::Pressed {
                        self.keys.insert(code);
                        match code {
                            KeyCode::Escape => el.exit(),
                            KeyCode::F1 => {
                                self.orbit = None;
                                self.in_cockpit = true;
                                self.head = (0.0, 0.0);
                            }
                            KeyCode::F2 => {
                                self.orbit =
                                    Some(self.orbit.unwrap_or((std::f64::consts::PI, 0.25, 25.0)))
                            }
                            KeyCode::F3 => {
                                self.orbit = None;
                                self.in_cockpit = false;
                            }
                            KeyCode::KeyF => {
                                self.inputs.flaps = self.sim.flap_step(self.inputs.flaps, true)
                            }
                            KeyCode::KeyV => {
                                self.inputs.flaps = self.sim.flap_step(self.inputs.flaps, false)
                            }
                            KeyCode::Comma => self.sim.trim(1, -1.0),
                            KeyCode::Period => self.sim.trim(1, 1.0),
                            KeyCode::BracketLeft => self.sim.trim(0, -1.0),
                            KeyCode::BracketRight => self.sim.trim(0, 1.0),
                            KeyCode::Semicolon => self.sim.trim(2, -1.0),
                            KeyCode::Quote => self.sim.trim(2, 1.0),
                            KeyCode::KeyI => {
                                let s = &self.sim;
                                let at_airfield = s.on_ground && s.vel.length() < 1.0;
                                self.sim.toggle_engines(at_airfield);
                            }
                            KeyCode::KeyR => {
                                // Radiator: automatic, then 1/3, 2/3, fully open, then automatic again.
                                let next = match self
                                    .sim
                                    .engines
                                    .first()
                                    .and_then(|e| e.radiator_manual)
                                {
                                    None => Some(1.0 / 3.0),
                                    Some(r) if r < 0.9 => Some((r + 1.0 / 3.0).min(1.0)),
                                    Some(_) => None,
                                };
                                self.sim.set_radiator(next);
                            }
                            KeyCode::KeyO => {
                                let auto = self
                                    .sim
                                    .engines
                                    .first()
                                    .is_none_or(|e| e.prop_manual.is_none());
                                self.sim.set_prop(if auto { Some(1.0) } else { None });
                            }
                            KeyCode::KeyQ | KeyCode::KeyE => {
                                let cur = self
                                    .sim
                                    .engines
                                    .first()
                                    .and_then(|e| e.prop_manual)
                                    .unwrap_or(1.0);
                                let d = if code == KeyCode::KeyQ { -0.05 } else { 0.05 };
                                self.sim.set_prop(Some((cur + d).clamp(0.0, 1.0)));
                            }
                            KeyCode::KeyL => self.inputs.tail_lock = !self.inputs.tail_lock,
                            KeyCode::KeyN => {
                                self.inputs.airbrake =
                                    if self.inputs.airbrake > 0.5 { 0.0 } else { 1.0 }
                            }
                            KeyCode::KeyG => {
                                self.inputs.gear = if self.inputs.gear > 0.5 { 0.0 } else { 1.0 }
                            }
                            KeyCode::Digit0 => self.inputs.throttle = 1.0,
                            // Boost: throttle to 110 % where allowed, and the WEP switch.
                            KeyCode::KeyW => {
                                self.inputs.wep = !self.inputs.wep;
                                if self.inputs.wep {
                                    self.inputs.throttle = 1.1;
                                }
                            }
                            KeyCode::Digit1 => self.inputs.throttle = 0.0,
                            c @ (KeyCode::Digit2
                            | KeyCode::Digit3
                            | KeyCode::Digit4
                            | KeyCode::Digit5
                            | KeyCode::Digit6
                            | KeyCode::Digit7
                            | KeyCode::Digit8
                            | KeyCode::Digit9) => {
                                let n = c as u32 - KeyCode::Digit0 as u32;
                                self.inputs.throttle = (n as f64 - 1.0) / 9.0;
                            }
                            _ => {}
                        }
                    } else {
                        self.keys.remove(&code);
                    }
                }
            }
            WindowEvent::RedrawRequested => self.frame(),
            WindowEvent::MouseInput {
                state,
                button: winit::event::MouseButton::Left,
                ..
            } => {
                self.dragging = state == ElementState::Pressed;
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let (Some((x, y)), true) = (self.cursor, self.dragging) {
                    let (dx, dy) = ((position.x - x) * 0.008, (position.y - y) * 0.008);
                    if let Some(o) = self.orbit.as_mut() {
                        o.0 -= dx;
                        o.1 = (o.1 + dy).clamp(-1.5, 1.5);
                    } else if self.in_cockpit {
                        // Looking around: the head turns up to 150° either side.
                        self.head.0 = (self.head.0 - dx).clamp(-2.6, 2.6);
                        self.head.1 = (self.head.1 - dy).clamp(-0.6, 1.4);
                    }
                }
                self.cursor = Some((position.x, position.y));
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if let Some(o) = self.orbit.as_mut() {
                    let steps = match delta {
                        winit::event::MouseScrollDelta::LineDelta(_, y) => y as f64,
                        winit::event::MouseScrollDelta::PixelDelta(p) => p.y / 40.0,
                    };
                    o.2 = (o.2 * 0.9f64.powf(steps)).clamp(3.0, 5000.0);
                }
            }
            _ => {}
        }
    }
}

/// Headless check: hold level flight at full throttle with a simple autopilot and report the
/// stabilised speed, for comparison with the flight model's own Vmax figures.
fn level_flight_test(mut sim: Sim, alt: f64) -> (f64, f64) {
    let ground = |_: f64, _: f64| 0.0;
    // Like the reference figures: healthy engine, radiator closed.
    sim.engine_overheat = false;
    sim.set_radiator(Some(0.0));
    sim.pos.z = alt;
    let mut inputs = Inputs {
        throttle: 1.1,
        wep: true,
        ..Default::default()
    };
    let mut last = Vec::new();
    for step in 0..(240.0 / TICK) as usize {
        let vz = sim.vel.z;
        let alt_err = alt - sim.pos.z;
        let bank = sim.bank_deg();
        inputs.elevator = (0.01 * alt_err - 0.03 * vz + 0.3 * sim.w.y).clamp(-1.0, 1.0);
        inputs.aileron = (-0.03 * bank - 0.5 * sim.w.x).clamp(-1.0, 1.0);
        inputs.rudder = (0.5 * sim.w.z).clamp(-1.0, 1.0);
        sim.tick(&inputs, &ground);
        if std::env::var_os("IL2_TRACE").is_some() && step % 33 == 0 && step < 33 * 25 {
            let pitch = (sim.ori * DVec3::X).z.asin().to_degrees();
            eprintln!(
                "t={:5.1} z={:7.1} V={:6.1} vz={:6.1} pitch={:6.1} bank={:6.1} aoa={:6.1} w={:?} elev={:5.2} thrust={:6.0}",
                step as f64 * TICK, sim.pos.z, sim.vel.length() * 3.6, sim.vel.z, pitch, sim.bank_deg(), sim.aoa,
                (sim.w * 100.0).round() / 100.0, inputs.elevator, sim.thrust
            );
        }
        if !sim.pos.is_finite() {
            return (f64::NAN, f64::NAN);
        }
        if step as f64 * TICK > 200.0 {
            last.push((sim.vel.length(), sim.pos.z));
        }
    }
    if let Some(e) = sim
        .engines
        .first()
        .filter(|_| std::env::var_os("IL2_QUIET").is_none())
    {
        eprintln!(
            "  engine at {alt:.0} m: {:.0} rpm, manifold {:.2} ata, pitch {:.1} deg, thrust {:.0} N, prop torque {:.0} N·m",
            e.rpm(), e.manifold, e.phi.to_degrees(), e.thrust, e.prop_torque
        );
    }
    let n = last.len() as f64;
    (
        last.iter().map(|v| v.0).sum::<f64>() / n * 3.6,
        last.iter().map(|v| v.1).sum::<f64>() / n,
    )
}

/// Full-throttle takeoff with a simple pilot: keep straight with rudder, raise the tail at 100 km/h,
/// rotate at 170 km/h. Reports lift-off speed and ground roll.
fn takeoff_test(mut sim: Sim, field: &HeightField) -> String {
    let start = sim.pos;
    let heading0 = (sim.ori * DVec3::X).truncate().normalize();
    let mut inputs = Inputs {
        throttle: 1.1,
        gear: 1.0,
        tail_lock: true,
        ..Default::default()
    };
    let ground = |x: f64, y: f64| field.height_at(x, y);
    let mut t = 0.0;
    let mut last_contact = (0.0, sim.pos, 0.0, heading0);
    for _ in 0..(90.0 / TICK) as usize {
        let fwd = (sim.ori * DVec3::X).truncate().normalize();
        let yaw_err = heading0.perp_dot(fwd); // >0: nose left of the runway heading
        let kmh = sim.vel.length() * 3.6;
        inputs.rudder = (2.0 * yaw_err + 0.8 * sim.w.z).clamp(-1.0, 1.0);
        let pitch = (sim.ori * DVec3::X).z.asin().to_degrees();
        let target = if kmh < 100.0 {
            99.0
        } else if kmh < 170.0 {
            1.5
        } else {
            8.0
        };
        inputs.elevator = if target > 90.0 {
            0.0
        } else {
            (0.08 * (target - pitch) + 0.3 * sim.w.y).clamp(-1.0, 1.0)
        };
        inputs.aileron = (-0.05 * sim.bank_deg() - 0.3 * sim.w.x).clamp(-1.0, 1.0);
        sim.tick(&inputs, &ground);
        t += TICK;
        if sim.on_ground {
            last_contact = (t, sim.pos, sim.ias, fwd);
        }
        if std::env::var_os("IL2_TRACE").is_some()
            && ((t / TICK).round() as usize).is_multiple_of(33)
        {
            eprintln!(
                "t={t:5.1} v={kmh:5.0} yaw_err={:6.1} wz={:6.2} rud={:5.2} pitch={:5.1} gnd={} agl={:5.2}",
                yaw_err.asin().to_degrees(), sim.w.z, inputs.rudder,
                (sim.ori * DVec3::X).z.asin().to_degrees(), sim.on_ground, sim.pos.z - field.height_at(sim.pos.x, sim.pos.y)
            );
        }
        // Lift-off: a full second without ground contact (wheels may skip over bumps before).
        if t > 2.0 && !sim.on_ground && t - last_contact.0 > 1.0 {
            let (tl, pl, ias, fl) = last_contact;
            let roll = (pl - start).truncate().length();
            return format!(
                "takeoff: lift-off at {:.0} km/h IAS after {:.0} m ground roll, {:.1} s, heading deviation {:.1} deg",
                ias * 3.6, roll, tl, heading0.angle_to(fl).to_degrees()
            );
        }
    }
    format!(
        "takeoff: no lift-off within 90 s (speed {:.0} km/h, on ground {})",
        sim.vel.length() * 3.6,
        sim.on_ground
    )
}

/// Drops the aircraft from 0.5 m onto flat ground at its parked attitude, brakes on, engine off.
fn parked_test(mut sim: Sim, (h, pitch_deg): (f64, f64)) -> (f64, f64, f64) {
    let ground = |_: f64, _: f64| 0.0;
    // Dropped from 10 cm above its parked stance.
    sim.ori = glam::DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2)
        * glam::DQuat::from_rotation_y(-pitch_deg.to_radians());
    sim.pos.z = h + 0.1;
    sim.vel = DVec3::ZERO;
    sim.set_engines_idle(true);
    let inputs = Inputs {
        brakes: 1.0,
        gear: 1.0,
        ..Default::default()
    };
    for _ in 0..(10.0 / TICK) as usize {
        sim.tick(&inputs, &ground);
    }
    let pitch = (sim.ori * DVec3::X).z.asin().to_degrees();
    if let Some(e) = sim
        .engines
        .first()
        .filter(|_| std::env::var_os("IL2_QUIET").is_none())
    {
        eprintln!(
            "  parked: engine {:.0} rpm, thrust {:.0} N",
            e.rpm(),
            e.thrust
        );
    }
    (pitch, sim.pos.z, sim.vel.length())
}

/// How the player starts: from a mission, or airborne over the map centre.
enum Start {
    Takeoff {
        x: f64,
        y: f64,
        heading: f64,
    },
    Air {
        pos: DVec3,
        toward: DVec3,
        speed: f64,
    },
    Default,
}

fn main() {
    let mut args = Args::parse();
    let vfs = il2_vfs::Vfs::open_install(&args.root).unwrap_or_else(|e| fail(e));
    let fms = il2_data::FlightModels::open(&vfs).unwrap_or_else(|e| fail(e));
    if args.survey {
        survey(&vfs, &fms);
        return;
    }
    if args.ai_test {
        let assets = world::load_aircraft_with(&vfs, &fms, &args.aircraft, false)
            .unwrap_or_else(|e| fail(e));
        print!("{}", ai_landing_test(&assets));
        print!("{}", ai_takeoff_test(&assets));
        print!("{}", ai_test(assets.fm));
        return;
    }
    let mut start = Start::Default;
    let mut fuel = 1.0;
    let mission = args.mission.as_ref().map(|path| {
        il2_formats::mission::Mission::parse(&vfs.read(path).unwrap_or_else(|e| fail(e)))
    });
    if let (Some(path), Some(m)) = (&args.mission, &mission) {
        let wing = m
            .player()
            .unwrap_or_else(|| fail("mission has no player flight"));
        args.map = format!("maps/{}", m.map);
        args.aircraft = wing.class.clone();
        fuel = wing.fuel_percent / 100.0;
        eprintln!(
            "mission: {} on {}, flight {} ({} x {})",
            path, m.map, wing.name, wing.planes, wing.class
        );
        let ends = world::load_runways(&vfs, &args.map);
        if let Some(w0) = wing.waypoints.first() {
            start = if w0.kind.starts_with("TAKEOFF") {
                match world::takeoff_spot(&ends, w0.x, w0.y) {
                    Some((x, y, heading)) => Start::Takeoff { x, y, heading },
                    None => Start::Air {
                        pos: DVec3::new(w0.x, w0.y, 500.0),
                        toward: DVec3::new(w0.x, w0.y + 1.0, 500.0),
                        speed: 100.0,
                    },
                }
            } else {
                let next = wing.waypoints.get(1).unwrap_or(w0);
                Start::Air {
                    pos: DVec3::new(w0.x, w0.y, w0.z.max(100.0)),
                    toward: DVec3::new(next.x, next.y, next.z) + DVec3::new(0.0, 1e-3, 0.0),
                    speed: (w0.speed / 3.6).max(60.0),
                }
            };
        }
    }
    if args.at.is_some() {
        start = Start::Default;
    }
    let assets = world::load_aircraft(&vfs, &fms, &args.aircraft).unwrap_or_else(|e| fail(e));
    let fmd_name = il2_data::Classes::new(&vfs, &fms.keys)
        .aircraft(&args.aircraft)
        .map(|c| c.flight_model)
        .unwrap_or_default();
    let field = world::load_heights(&vfs, &args.map).unwrap_or_else(|e| fail(e));
    let mut traffic = mission.as_ref().filter(|_| !args.no_ai).map(|m| {
        traffic::Traffic::from_mission(&vfs, &fms, m, &field, &world::load_runways(&vfs, &args.map))
    });

    if args.test && !matches!(start, Start::Takeoff { .. }) {
        let fmd = fms.sect_file(&fmd_name).unwrap_or_else(|e| fail(e));
        let p = |k: &str| fmd.get("Params", k).unwrap_or("?").to_owned();
        for alt in [500.0, 3000.0, p("HofVmax").parse().unwrap_or(6000.0)] {
            let sim = world::spawn_airborne(assets.fm.clone(), &field, 0.0, 120.0);
            let (kmh, z) = level_flight_test(sim, alt);
            println!("level flight at {alt:5.0} m: {kmh:5.1} km/h (held {z:5.0} m)");
        }
        println!(
            "flight model reference: Vmax {} km/h at sea level, VmaxH {} km/h at {} m",
            p("Vmax"),
            p("VmaxH"),
            p("HofVmax")
        );
        let (pitch, z, v) = parked_test(
            world::spawn_airborne(assets.fm.clone(), &field, 0.0, 0.0),
            assets.parked,
        );
        println!("parked on flat ground: pitch {pitch:.2} deg, origin {z:.3} m above ground, drift {v:.3} m/s (reference: Pitch {}, H {})",
            fmd.get("Gear", "Pitch").unwrap_or("?"), fmd.get("Gear", "H").unwrap_or("?"));
        println!(
            "{}",
            engine_test(world::spawn_airborne(assets.fm.clone(), &field, 0.0, 0.0))
        );
        return;
    }

    let terrain = {
        let mut loader = il2_render::scene::Loader::new(&vfs);
        let t = loader.load_terrain(&args.map).unwrap_or_else(|e| fail(e));
        for w in &loader.warnings {
            eprintln!("warning: {w}");
        }
        t
    };
    let t0 = Instant::now();
    let map_objects = {
        let statics_ini = objects::statics_ini(&vfs).unwrap_or_else(|e| fail(e));
        let mut b = objects::Builder::new(&vfs, &field, &objects::season(&vfs, &args.map));
        b.add_map(&vfs, &args.map, &statics_ini)
            .unwrap_or_else(|e| fail(e));
        if let Some(m) = &mission {
            mission_objects::add_mission(&mut b, &vfs, &fms, m, &statics_ini);
        }
        b.finish()
    };
    if std::env::var_os("IL2_TRACE").is_some() {
        eprintln!("skipped classes: {:?}", map_objects.skipped);
    }
    eprintln!(
        "map objects: {} placed in {:.1} s ({} classes skipped)",
        map_objects.len(),
        t0.elapsed().as_secs_f64(),
        map_objects.skipped.len()
    );
    let (env, time_of_day) = {
        let ini = vfs
            .read(&args.map)
            .map(|d| il2_formats::SectFile::parse(&d))
            .unwrap_or_default();
        let wp = |k: &str| {
            ini.section_ci("WORLDPOS")
                .and_then(|s| s.get_ci(k))
                .and_then(|v| il2_formats::sectfile::atof(v.trim_start_matches(['=', ' '])))
        };
        let lat = wp("DECLIN").map_or(50.0, f64::from);
        let (month, day) = mission
            .as_ref()
            .and_then(|m| m.date)
            .unwrap_or((wp("MONTH").map_or(6, |m| m as u32), 15));
        let hours = args
            .time
            .or(mission.as_ref().map(|m| m.time_hours))
            .unwrap_or(12.0);
        (
            sky::environment(hours, sky::day_of_year(month, day), lat),
            hours,
        )
    };
    let forest = forest::Forest::load(&vfs, &args.map, &terrain.types);
    let roads = roads::Roads::load(&vfs, &args.map, &terrain.types, terrain.water);
    // Guns of the chosen loadout and their tracers.
    let guns = {
        let classes = il2_data::Classes::new(&vfs, &fms.keys);
        let loadout = args.loadout.clone().unwrap_or_else(|| {
            mission
                .as_ref()
                .and_then(|m| m.player())
                .map_or("default".to_owned(), |w| w.weapons.clone())
        });
        let mut armoury = weapons::Armoury::default();
        let w = classes.aircraft_weapons(&args.aircraft).unwrap_or_default();
        let guns = weapons::mount(&mut armoury, &classes, &w, &loadout, &assets.hook_frames);
        let stores = weapons::mount_stores(&classes, &w, &loadout, &assets.hook_frames);
        eprintln!(
            "loadout {loadout}: {}",
            guns.iter()
                .map(|g| format!("{} ({})", g.spec.name, g.ammo))
                .chain(
                    stores
                        .iter()
                        .map(|s| format!("{} ({})", s.spec.name, s.count))
                )
                .collect::<Vec<_>>()
                .join(", ")
        );
        (guns, stores)
    };
    let (guns, stores) = guns;
    let mut assets = assets;
    let plane_mesh = std::mem::take(&mut assets.mesh);
    let (h, pitch) = assets.parked;
    let mut sim = match start {
        Start::Takeoff { x, y, heading } => {
            world::spawn_on_ground(assets.fm.clone(), &field, x, y, heading, h, pitch)
        }
        Start::Air { pos, toward, speed } => world::spawn_at(assets.fm.clone(), pos, toward, speed),
        Start::Default => {
            let mut sim = world::spawn_airborne(assets.fm.clone(), &field, args.agl, 110.0);
            if let Some([x, y]) = args
                .at
                .as_deref()
                .and_then(|a| <[f64; 2]>::try_from(a).ok())
            {
                sim.pos = DVec3::new(x, y, field.height_at(x, y) + args.agl);
            }
            if let Some(d) = args.dive {
                sim.ori *= glam::DQuat::from_rotation_y(d.to_radians());
                sim.vel = sim.ori * DVec3::new(sim.vel.length(), 0.0, 0.0);
            }
            sim
        }
    };
    sim.fuel *= fuel;
    sim.structure.twin_fin = assets.twin_fin;
    if let Some(class) = &args.enemy {
        let fwd = sim.ori * DVec3::X;
        let pos = sim.pos + fwd * 150.0;
        traffic = traffic::Traffic::single(&vfs, &fms, class, pos, pos + fwd, sim.vel.length());
    }
    let ai_specs = traffic.as_ref().map(|t| t.gun_specs()).unwrap_or_default();
    let tracers = effects::Tracers::load(
        &vfs,
        guns.iter()
            .flat_map(|g| {
                g.spec
                    .bullets
                    .iter()
                    .filter_map(|b| b.trace_mesh.as_deref())
            })
            .chain(stores.iter().filter_map(|s| s.spec.mesh.as_deref()))
            .chain(
                ai_specs
                    .iter()
                    .flat_map(|g| g.bullets.iter().filter_map(|b| b.trace_mesh.as_deref())),
            ),
    );
    let effects =
        particles::Effects::load(&vfs, particles::known_effects().iter().map(String::as_str));
    let mut view = view::WorldView::new(map_objects, forest, roads, Some(tracers), effects);
    let on_ground_start = matches!(start, Start::Takeoff { .. });
    let gear0 = if on_ground_start { 1.0 } else { 0.0 };
    sim.ct.gear = gear0;
    let throttle0 = if on_ground_start { 0.0 } else { 0.8 };
    sim.ct.throttle = throttle0;
    if args.test && on_ground_start {
        println!("{}", takeoff_test(sim, &field));
        return;
    }

    if let Some(out) = &args.screenshot {
        let mut inputs = Inputs {
            throttle: throttle0,
            gear: gear0,
            ..Default::default()
        };
        if let Some(h) = &args.hold {
            let g = |i: usize, d: f64| h.get(i).copied().unwrap_or(d);
            inputs.aileron = g(0, 0.0);
            inputs.elevator = g(1, 0.0);
            inputs.rudder = g(2, 0.0);
            inputs.flaps = g(3, 0.0);
            inputs.gear = g(4, gear0);
            sim.ct = inputs;
        }
        let ground = |x: f64, y: f64| field.height_at(x, y);
        let mut combat = Combat::new(guns, stores);
        let triggers = [args.fire, args.fire, args.fire, args.fire];
        let mut af = new_airframe(&assets);
        for c in args.damage.iter().flatten() {
            af.collision(&mut sim, c);
        }
        let mut max_g = (0.0f64, 0.0f64);
        for _ in 0..(args.seconds / TICK) as usize {
            sim.tick(&inputs, &ground);
            let targets = build_targets(&sim, &assets, &af, traffic.as_ref());
            combat.step(&mut sim, &triggers, &field, &mut view, &targets);
            apply_aircraft_hits(&combat, &mut sim, &mut af, &assets, traffic.as_mut());
            airframe_step(&mut af, &mut sim, &field, &mut combat, &mut view);
            if let Some(t) = traffic.as_mut() {
                let pl = (sim.pos, sim.vel, t.player_army, !sim.structure.destroyed);
                t.step(&field, TICK, pl, &mut combat.rng, &mut combat.bullets);
            }
            max_g = (max_g.0.max(sim.overload), max_g.1.min(sim.overload));
        }
        eprintln!(
            "load factor {:+.1} .. {:+.1} g (limit {:.1}, ultimate {:.1})",
            max_g.1, max_g.0, sim.structure.limit_load, sim.structure.ultimate_load
        );
        for m in af.messages.drain(..) {
            eprintln!("{m}");
        }
        if let (Some(t), true) = (traffic.as_ref(), std::env::var_os("IL2_TRACE").is_some()) {
            eprint!("{}", t.report(&field));
        }
        let (w, h) = (1280, 720);
        let mut r = Renderer::headless(w, h);
        r.env = env;
        let (terrain, plane) = (r.upload_terrain(&terrain), r.upload(&plane_mesh));
        view.upload(&r);
        let (mut vp, mut eye) = chase_camera(&sim, &field, w as f32 / h as f32);
        if let Some(o) = &args.orbit {
            let fwd = sim.ori * DVec3::X;
            let heading = fwd.y.atan2(fwd.x);
            let (yaw, pitch, dist) = (
                heading + o[0].to_radians(),
                o.get(1).copied().unwrap_or(15.0).to_radians(),
                o.get(2).copied().unwrap_or(12.0),
            );
            eye = sim.pos
                + DVec3::new(
                    yaw.cos() * pitch.cos(),
                    yaw.sin() * pitch.cos(),
                    pitch.sin(),
                ) * dist;
            let view = DMat4::look_at_rh(DVec3::ZERO, sim.pos - eye, DVec3::Z);
            vp = Mat4::perspective_rh(45f32.to_radians(), w as f32 / h as f32, 0.2, 150_000.0)
                * view.as_mat4();
        }
        if let (Some(e), Some(t)) = (&args.eye, &args.target) {
            eye = DVec3::new(e[0], e[1], e[2]);
            let target = DVec3::new(t[0], t[1], t[2]);
            let view = DMat4::look_at_rh(DVec3::ZERO, target - eye, DVec3::Z);
            vp = Mat4::perspective_rh(45f32.to_radians(), w as f32 / h as f32, 0.2, 150_000.0)
                * view.as_mat4();
        }
        let mut fg_vp = None;
        let cockpit_gpu = match (&args.cockpit, assets.cockpit.as_mut()) {
            (Some(head), Some(cp)) => {
                let g = |i: usize| head.get(i).copied().unwrap_or(0.0).to_radians();
                let (world, near, e) =
                    cockpit_camera(&sim, cp.eye, (g(0), g(1)), w as f32 / h as f32);
                (vp, eye, fg_vp) = (world, e, Some(near));
                let c = r.upload(&cp.mesh.take().unwrap_or_default());
                r.set_transform(
                    &c,
                    DMat4::from_rotation_translation(sim.ori, sim.pos - eye).as_mat4(),
                );
                // Let the needles settle.
                for _ in 0..60 {
                    cp.pose(&sim, &gun_readings(&combat.guns), time_of_day, TICK);
                }
                r.set_nodes(
                    &c,
                    &cp.pose(&sim, &gun_readings(&combat.guns), time_of_day, TICK),
                );
                if std::env::var_os("IL2_TRACE").is_some() {
                    eprintln!("cockpit reads without values: {:?}", cp.missing());
                }
                Some(c)
            }
            (Some(_), None) => fail("this aircraft has no cockpit"),
            _ => None,
        };
        place(&r, &terrain, &plane, &sim, eye);
        let mut posed = world::pose(&assets, &sim.ct);
        af.apply(&mut posed, &sim);
        if std::env::var_os("IL2_TRACE").is_some() {
            let rest = assets.rig.pose(&[]);
            for (i, (a, b)) in posed.iter().zip(&rest).enumerate() {
                if !a.abs_diff_eq(*b, 1e-4) {
                    eprintln!("animated node {}", assets.rig.names[i]);
                }
            }
            eprintln!("ct: {:?}", sim.ct);
        }
        r.set_nodes(&plane, &posed);
        eprintln!(
            "{} bullets in flight, {} effect particles",
            combat.bullets.len(),
            view.effects.count()
        );
        let mut hud = hud::Hud::new(&args.root.join("lib/fonts/LucidaSansRegular.ttf"));
        if let Some(a) = &hud.atlas {
            r.set_overlay_atlas(a);
        }
        r.set_overlay(hud.build(
            w as f32,
            h as f32,
            &hud_lines(&sim, &field),
            args.orbit.is_none() && args.eye.is_none() && args.cockpit.is_none(),
            0.0,
        ));
        let cockpit: Vec<&GpuMesh> = cockpit_gpu.iter().collect();
        let mut outside: Vec<&GpuMesh> = traffic
            .as_mut()
            .map(|t| t.prepare(&r, eye))
            .unwrap_or_default();
        let fg = match fg_vp {
            Some(m) => Some((m, cockpit.as_slice())),
            None => {
                outside.push(&plane);
                None
            }
        };
        let px = view
            .draw(
                &mut r,
                &field,
                &terrain,
                vp,
                eye,
                &outside,
                &combat.bullets,
                &combat.projectiles,
                fg,
                true,
            )
            .unwrap_or_default();
        image::save_buffer(out, &px, w, h, image::ExtendedColorType::Rgba8)
            .unwrap_or_else(|e| fail(e));
        return;
    }
    let cockpit = assets.cockpit.take();
    sim.structure.twin_fin = assets.twin_fin;
    let airframe = new_airframe(&assets);
    let mut game = Game {
        airframe,
        traffic,
        inputs: Inputs {
            throttle: throttle0,
            gear: gear0,
            ..Default::default()
        },
        assets,
        sim,
        field,
        terrain_data: Some(terrain),
        plane_data: Some(plane_mesh),
        renderer: None,
        terrain: None,
        plane: None,
        cockpit_state: cockpit,
        time_of_day,
        cockpit: None,
        in_cockpit: true,
        head: (0.0, 0.0),
        view,
        env,
        combat: Combat::new(guns.clone(), stores.clone()),
        audio: audio::Audio::start(),
        hud: hud::Hud::new(&args.root.join("lib/fonts/LucidaSansRegular.ttf")),
        vfs: Arc::new(vfs),
        keys: HashSet::new(),
        #[cfg(feature = "gamepad")]
        gilrs: Gilrs::new().ok(),
        joystick: {
            let map = args
                .joy_map
                .as_deref()
                .map(joystick::AxisMap::parse)
                .transpose()
                .unwrap_or_else(|e| fail(e))
                .unwrap_or_default();
            joystick::Joystick::open().map(|(j, path)| {
                eprintln!("joystick: {path}");
                (j, map)
            })
        },
        last: Instant::now(),
        acc: 0.0,
        orbit: None,
        dragging: false,
        cursor: None,
    };
    let el = EventLoop::new().expect("event loop");
    el.run_app(&mut game).expect("run");
}

/// The AI lands from 8 km out at 600 m, abeam the runway, on flat ground.
fn ai_landing_test(a: &world::AircraftAssets) -> String {
    let flat = |_: f64, _: f64| 0.0;
    let mut sim = Sim::new(
        a.fm.clone(),
        DVec3::new(-8000.0, 2000.0, 600.0),
        glam::DQuat::IDENTITY,
        80.0,
    );
    sim.ai = true;
    sim.ct.gear = 0.0;
    let mut pilot = ai::AiPilot::new(Vec::new(), 0, 0, 2);
    pilot.phase = ai::Phase::Landing {
        at: DVec3::ZERO,
        heading: 0.0,
    };
    let mut out = String::new();
    let mut t = 0.0;
    while t < 400.0 && pilot.phase != ai::Phase::Parked {
        let inputs = pilot.update(&sim, 0.0, &flat, TICK);
        sim.tick(&inputs, &flat);
        t += TICK;
        let every = if std::env::var_os("IL2_TRACE").is_some() {
            2.0
        } else {
            10.0
        };
        if (t / TICK) as usize % (every / TICK) as usize == 0 {
            out += &format!(
                "land vz {:+.1} e {:+.2} thr {:.2} aoa {:+.1} pitch {:+.1}  ",
                sim.vel.z,
                inputs.elevator,
                inputs.throttle,
                sim.aoa,
                (sim.ori * DVec3::X).z.asin().to_degrees()
            );
            out += &format!(
                "land t {t:4.0}  pos {:6.0} {:6.0} {:5.0}  {:4.0} km/h  gear {:.1} flaps {:.1}{}{}\n",
                sim.pos.x, sim.pos.y, sim.pos.z, sim.vel.length() * 3.6, sim.ct.gear, sim.ct.flaps,
                if sim.on_ground { " ground" } else { "" },
                if sim.structure.destroyed { " DESTROYED" } else { "" }
            );
        }
    }
    out + &format!(
        "land: {:?} after {t:.0} s at {:.0} {:.0}{}\n",
        pilot.phase,
        sim.pos.x,
        sim.pos.y,
        if sim.structure.destroyed {
            " DESTROYED"
        } else {
            ""
        }
    )
}

/// The AI takes off from flat ground and climbs towards a waypoint 20 km ahead at 1500 m.
fn ai_takeoff_test(a: &world::AircraftAssets) -> String {
    use il2_formats::mission::Waypoint;
    let flat = |_: f64, _: f64| 0.0;
    let field = HeightField::from_raw(2, 2, &[0; 4]);
    let (h, pitch) = a.parked;
    let mut sim = world::spawn_on_ground(a.fm.clone(), &field, 0.0, 0.0, 0.0, h, pitch);
    sim.ai = true;
    let wp = |kind: &str, x: f64, z: f64| Waypoint {
        kind: kind.into(),
        x,
        y: 0.0,
        z,
        speed: 300.0,
        target: None,
    };
    let mut pilot = ai::AiPilot::new(
        vec![wp("TAKEOFF", 0.0, 0.0), wp("NORMFLY", 20_000.0, 1500.0)],
        0,
        0,
        2,
    );
    pilot.phase = ai::Phase::Lineup(1.0);
    let mut out = String::new();
    let mut t = 0.0;
    while t < 120.0 {
        let inputs = pilot.update(&sim, 0.0, &flat, TICK);
        sim.tick(&inputs, &flat);
        t += TICK;
        if (t / TICK) as usize % (4.0 / TICK) as usize == 0 {
            out += &format!(
                "t {t:4.0}  x {:6.0}  agl {:5.0}  {:4.0} km/h  pitch {:+5.1}  aoa {:+5.1}  elev {:+.2}  {:?}{}\n",
                sim.pos.x,
                sim.pos.z,
                sim.vel.length() * 3.6,
                (sim.ori * DVec3::X).z.asin().to_degrees(),
                sim.aoa,
                inputs.elevator,
                pilot.phase,
                if sim.on_ground { " ground" } else { "" }
            );
        }
    }
    out
}

/// The AI flies a 10 km square at 1500 m and 350 km/h over flat ground.
fn ai_test(ac: il2_fm::Aircraft) -> String {
    use il2_formats::mission::Waypoint;
    let flat = |_: f64, _: f64| 0.0;
    let mut sim = Sim::new(
        ac,
        DVec3::new(0.0, 0.0, 1000.0),
        glam::DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2),
        90.0,
    );
    sim.ai = true;
    sim.ct.gear = 0.0;
    let wp = |x: f64, y: f64| Waypoint {
        kind: "NORMFLY".into(),
        x,
        y,
        z: 1500.0,
        speed: 350.0,
        target: None,
    };
    let way = vec![
        wp(0.0, 10_000.0),
        wp(10_000.0, 10_000.0),
        wp(10_000.0, 0.0),
        wp(0.0, 0.0),
        wp(0.0, 10_000.0),
    ];
    let mut pilot = ai::AiPilot::new(way, 0, 0, 2);
    let mut out = String::new();
    let mut t = 0.0;
    while t < 400.0 {
        let inputs = pilot.update(&sim, 0.0, &flat, TICK);
        sim.tick(&inputs, &flat);
        t += TICK;
        if (t / TICK) as usize % (20.0 / TICK) as usize == 0 {
            let fwd = sim.ori * DVec3::X;
            out += &format!(
                "t {t:5.0}  pos {:7.0} {:7.0} {:5.0}  {:4.0} km/h  hdg {:3.0}  bank {:+4.0}  wp {}\n",
                sim.pos.x,
                sim.pos.y,
                sim.pos.z,
                sim.vel.length() * 3.6,
                (90.0 - fwd.y.atan2(fwd.x).to_degrees()).rem_euclid(360.0),
                sim.bank_deg(),
                pilot.cur
            );
        }
    }
    out
}

/// Cold start on the ground, then temperatures after 5 minutes of full power at 300 km/h and after
/// 5 minutes at 150 km/h with the radiator closed.
fn engine_test(mut sim: Sim) -> String {
    let flat = |_: f64, _: f64| 0.0;
    sim.set_engines_cold();
    sim.toggle_engines(true);
    let mut inputs = Inputs {
        throttle: 0.3,
        gear: 1.0,
        brakes: 1.0,
        ..Default::default()
    };
    let mut t = 0.0;
    while t < 30.0 && !sim.engines.iter().all(|e| e.running()) {
        sim.tick(&inputs, &flat);
        t += TICK;
    }
    let started = sim.engines.iter().all(|e| e.running());
    let e = &sim.engines[0];
    let mut out = format!(
        "cold start: {} after {t:.1} s, {:.0} rpm, oil {:.0} C",
        if started { "running" } else { "NOT running" },
        e.rpm(),
        e.t_oil_out
    );
    let hold = |sim: &mut Sim, inputs: &Inputs, speed: f64, secs: f64| {
        for _ in 0..(secs / TICK) as usize {
            let fwd = sim.ori * DVec3::X;
            sim.vel = fwd * speed;
            sim.pos.z = 1000.0;
            sim.tick(inputs, &flat);
        }
    };
    if started {
        inputs = Inputs {
            throttle: 1.0,
            ..Default::default()
        };
        sim.ori = glam::DQuat::IDENTITY;
        hold(&mut sim, &inputs, 300.0 / 3.6, 300.0);
        let e = &sim.engines[0];
        out += &format!(
            "; 5 min full power at 300 km/h: oil {:.0} C, water {:.0} C, radiator {:.2}",
            e.t_oil_out, e.t_water, e.radiator
        );
        sim.set_radiator(Some(0.0));
        hold(&mut sim, &inputs, 150.0 / 3.6, 300.0);
        let e = &sim.engines[0];
        out += &format!(
            "; 5 min at 150 km/h, radiator closed: oil {:.0} C, water {:.0} C, events {:?}",
            e.t_oil_out, e.t_water, e.events
        );
    }
    out
}

/// Level-speed check of every aircraft in `air.ini` against its flight model's `Vmax`/`VmaxH`.
fn survey(vfs: &il2_vfs::Vfs, fms: &il2_data::FlightModels) {
    let air = il2_formats::SectFile::parse(
        &vfs.read("com/maddox/il2/objects/air.ini")
            .unwrap_or_else(|e| fail(e)),
    );
    let mut seen = HashSet::new();
    let flat = HeightField::from_raw(2, 2, &[0; 4]);
    println!("class\tfmd\tV500\tVmax\tVH\tVmaxH\tHofVmax\tparked_pitch\tPitch\tafterburner\tboost");
    let classes = il2_data::Classes::new(vfs, &fms.keys);
    for (_, v) in air
        .section_ci("AIR")
        .map(|s| s.entries.as_slice())
        .unwrap_or_default()
    {
        let Some(class) = v.split_whitespace().next() else {
            continue;
        };
        if !seen.insert(class.to_owned()) {
            continue;
        }
        let assets = match world::load_aircraft(vfs, fms, class) {
            Ok(a) => a,
            Err(e) => {
                println!("{class}\tERROR {e}");
                continue;
            }
        };
        let fmd_name = classes
            .aircraft(class)
            .map(|c| c.flight_model)
            .unwrap_or_default();
        let fmd = fms.sect_file(&fmd_name).ok();
        let p = |k: &str| {
            fmd.as_ref()
                .and_then(|f| f.get("Params", k))
                .unwrap_or("?")
                .to_owned()
        };
        let hv: f64 = p("HofVmax").parse().unwrap_or(6000.0);
        let (v0, _) = level_flight_test(
            world::spawn_airborne(assets.fm.clone(), &flat, 0.0, 120.0),
            500.0,
        );
        let (vh, _) = level_flight_test(
            world::spawn_airborne(assets.fm.clone(), &flat, 0.0, 120.0),
            hv,
        );
        let (pitch, _, _) = parked_test(
            world::spawn_airborne(assets.fm.clone(), &flat, 0.0, 0.0),
            assets.parked,
        );
        let g = fmd
            .as_ref()
            .and_then(|f| f.get("Gear", "Pitch"))
            .unwrap_or("?")
            .to_owned();
        let e0 = assets.fm.engines.first();
        if std::env::var_os("IL2_ANIM").is_some() {
            for (ch, r) in classes.animation_report(class) {
                if let Err(why) = r {
                    eprintln!("anim {class} {ch:?}: {why}");
                }
            }
        }
        println!(
            "{class}\t{fmd_name}\t{v0:.0}\t{}\t{vh:.0}\t{}\t{hv:.0}\t{pitch:.1}\t{g}\t{}\t{}",
            p("Vmax"),
            p("VmaxH"),
            e0.map_or(-1, |e| e.afterburner),
            e0.map_or(0.0, |e| e.boost_factor)
        );
    }
}

fn fail(e: impl std::fmt::Display) -> ! {
    eprintln!("error: {e}");
    std::process::exit(1)
}
