mod effects;
mod forest;
mod joystick;
mod mission_objects;
mod objects;
mod roads;
mod sky;
mod view;
mod weapons;
mod world;

use clap::Parser;
#[cfg(feature = "gamepad")]
use gilrs::{Axis, Gilrs};
use glam::{DMat4, DVec3, Mat4};
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
    /// Screenshot: hold the triggers (all guns) during the simulated seconds.
    #[arg(long)]
    fire: bool,
    /// Screenshot: hold these inputs (aileron, elevator, rudder, flaps, gear).
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    hold: Option<Vec<f64>>,
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

fn place(r: &Renderer, terrain: &il2_render::GpuTerrain, plane: &GpuMesh, s: &Sim, eye: DVec3) {
    r.set_transform(&terrain.mesh, Mat4::from_translation((-eye).as_vec3()));
    r.set_transform(plane, DMat4::from_rotation_translation(s.ori, s.pos - eye).as_mat4());
}

/// The player's guns and the bullets in flight.
struct Combat {
    guns: Vec<weapons::Gun>,
    bullets: Vec<weapons::Bullet>,
    rng: weapons::Rng,
    hits: Vec<(weapons::Bullet, weapons::Hit)>,
}

impl Combat {
    fn new(guns: Vec<weapons::Gun>) -> Self {
        Combat { guns, bullets: Vec::new(), rng: weapons::Rng::new(0x5EED), hits: Vec::new() }
    }

    fn step(&mut self, sim: &mut Sim, triggers: &[bool; 4], field: &HeightField) {
        let recoil = weapons::fire(&mut self.guns, triggers, sim.pos, sim.ori, sim.vel, TICK, &mut self.rng, 0, &mut self.bullets);
        if recoil != DVec3::ZERO {
            // Gun recoil acts on the aircraft.
            sim.vel += sim.ori * recoil / sim.mass();
        }
        self.hits.clear();
        weapons::advance(&mut self.bullets, TICK, field, &mut self.hits);
    }
}

struct Game {
    sim: Sim,
    assets: world::AircraftAssets,
    field: HeightField,
    terrain_data: Option<il2_render::TerrainData>,
    plane_data: Option<il2_render::MeshData>,
    renderer: Option<Renderer>,
    terrain: Option<il2_render::GpuTerrain>,
    plane: Option<GpuMesh>,
    view: view::WorldView,
    env: il2_render::Environment,
    combat: Combat,
    keys: HashSet<KeyCode>,
    #[cfg(feature = "gamepad")]
    gilrs: Option<Gilrs>,
    joystick: Option<(joystick::Joystick, joystick::AxisMap)>,
    inputs: Inputs,
    last: Instant,
    acc: f64,
    title_timer: f64,
    /// Free orbit camera around the aircraft: (yaw, pitch, distance), relative to the heading.
    orbit: Option<(f64, f64, f64)>,
    dragging: bool,
    cursor: Option<(f64, f64)>,
}

/// Orbit camera "yaw, pitch, distance" (radians relative to the aircraft heading, metres).
fn orbit_camera(s: &Sim, field: &HeightField, aspect: f32, (yaw, pitch, dist): (f64, f64, f64)) -> (Mat4, DVec3) {
    let fwd = s.ori * DVec3::X;
    let heading = fwd.y.atan2(fwd.x);
    let a = heading + yaw;
    let mut eye = s.pos + DVec3::new(a.cos() * pitch.cos(), a.sin() * pitch.cos(), pitch.sin()) * dist;
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
                let (x, y) = (dz(pad.value(Axis::LeftStickX)), dz(pad.value(Axis::LeftStickY)));
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
        let triggers = [self.keys.contains(&KeyCode::Space), self.keys.contains(&KeyCode::Space) || self.keys.contains(&KeyCode::Enter), false, false];
        while self.acc >= TICK {
            self.sim.tick(&self.inputs, &ground);
            self.combat.step(&mut self.sim, &triggers, field);
            self.acc -= TICK;
        }
        let (Some(r), Some(terrain), Some(plane)) = (self.renderer.as_mut(), &self.terrain, &self.plane) else { return };

        // Chase camera, rendered relative to the eye to keep precision far from the origin.
        let s = &self.sim;
        let (vp, eye) = match self.orbit {
            Some(o) => orbit_camera(s, field, r.aspect(), o),
            None => chase_camera(s, field, r.aspect()),
        };
        place(r, terrain, plane, s, eye);
        r.set_nodes(plane, &world::pose(&self.assets, &s.ct));
        self.view.draw(r, field, terrain, vp, eye, &[plane], &self.combat.bullets, false);

        for (i, e) in self.sim.engines.iter_mut().enumerate() {
            for ev in e.events.drain(..) {
                eprintln!("engine {}: {ev}", i + 1);
            }
        }
        let s = &self.sim;
        self.title_timer -= dt;
        if self.title_timer <= 0.0 {
            self.title_timer = 0.25;
            let agl = s.pos.z - field.height_at(s.pos.x, s.pos.y);
            r.window.as_ref().unwrap().set_title(&format!(
                "OpenIL2 | IAS {:4.0} km/h | ALT {:5.0} m (AGL {:5.0}) | THR {:3.0}%{} | FLAPS {:3.0}% | GEAR {} | AoA {:5.1} | FUEL {:4.0} kg | TRIM {:+.3} | {}{}{}",
                s.ias * 3.6,
                s.pos.z,
                agl,
                s.ct.throttle * 100.0,
                if s.ct.wep { " WEP" } else { "" },
                s.ct.flaps * 100.0,
                if s.ct.gear > 0.98 { "DOWN" } else if s.ct.gear < 0.02 { "UP" } else { "MOVING" },
                s.aoa,
                s.fuel,
                s.trim[1],
                s.engines.first().map_or(String::new(), |e| format!(
                    "ENG {} {:4.0} rpm PROP {} OIL {:3.0} WATER {:3.0} RAD {}",
                    match e.stage { 0 => "OFF", 1..=5 => "START", 6 => "ON", 7 => "DEAD", _ => "SEIZED" },
                    e.rpm(), e.prop_manual.map_or("AUTO".to_owned(), |l| format!("{:.0}%", l * 100.0)), e.t_oil_out, e.t_water,
                    e.radiator_manual.map_or("AUTO".to_owned(), |r| format!("{:.0}%", r * 100.0))
                )),
                if s.airbrake > 0.5 { " | AIRBRAKE" } else { "" },
                if s.on_ground { " | GROUND" } else { "" }
            ));
        }
        r.window.as_ref().unwrap().request_redraw();
    }
}

impl ApplicationHandler for Game {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.renderer.is_some() {
            return;
        }
        let window = Arc::new(el.create_window(winit::window::Window::default_attributes().with_title("OpenIL2")).expect("window"));
        let mut r = Renderer::new(window);
        r.env = self.env;
        self.terrain = self.terrain_data.take().map(|d| r.upload_terrain(&d));
        self.plane = self.plane_data.take().map(|d| r.upload(&d));
        self.view.upload(&r);
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
                            KeyCode::F1 => self.orbit = None,
                            KeyCode::F2 => self.orbit = Some(self.orbit.unwrap_or((std::f64::consts::PI, 0.25, 25.0))),
                            KeyCode::KeyF => self.inputs.flaps = self.sim.flap_step(self.inputs.flaps, true),
                            KeyCode::KeyV => self.inputs.flaps = self.sim.flap_step(self.inputs.flaps, false),
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
                                let next = match self.sim.engines.first().and_then(|e| e.radiator_manual) {
                                    None => Some(1.0 / 3.0),
                                    Some(r) if r < 0.9 => Some((r + 1.0 / 3.0).min(1.0)),
                                    Some(_) => None,
                                };
                                self.sim.set_radiator(next);
                            }
                            KeyCode::KeyO => {
                                let auto = self.sim.engines.first().is_none_or(|e| e.prop_manual.is_none());
                                self.sim.set_prop(if auto { Some(1.0) } else { None });
                            }
                            KeyCode::KeyQ | KeyCode::KeyE => {
                                let cur = self.sim.engines.first().and_then(|e| e.prop_manual).unwrap_or(1.0);
                                let d = if code == KeyCode::KeyQ { -0.05 } else { 0.05 };
                                self.sim.set_prop(Some((cur + d).clamp(0.0, 1.0)));
                            }
                            KeyCode::KeyL => self.inputs.tail_lock = !self.inputs.tail_lock,
                            KeyCode::KeyN => self.inputs.airbrake = if self.inputs.airbrake > 0.5 { 0.0 } else { 1.0 },
                            KeyCode::KeyG => self.inputs.gear = if self.inputs.gear > 0.5 { 0.0 } else { 1.0 },
                            KeyCode::Digit0 => self.inputs.throttle = 1.0,
                            // Boost: throttle to 110 % where allowed, and the WEP switch.
                            KeyCode::KeyW => {
                                self.inputs.wep = !self.inputs.wep;
                                if self.inputs.wep {
                                    self.inputs.throttle = 1.1;
                                }
                            }
                            KeyCode::Digit1 => self.inputs.throttle = 0.0,
                            c @ (KeyCode::Digit2 | KeyCode::Digit3 | KeyCode::Digit4 | KeyCode::Digit5 | KeyCode::Digit6
                            | KeyCode::Digit7 | KeyCode::Digit8 | KeyCode::Digit9) => {
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
            WindowEvent::MouseInput { state, button: winit::event::MouseButton::Left, .. } => {
                self.dragging = state == ElementState::Pressed;
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let (Some((x, y)), true, Some(o)) = (self.cursor, self.dragging, self.orbit.as_mut()) {
                    o.0 -= (position.x - x) * 0.008;
                    o.1 = (o.1 + (position.y - y) * 0.008).clamp(-1.5, 1.5);
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
    let mut inputs = Inputs { throttle: 1.1, wep: true, ..Default::default() };
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
    if let Some(e) = sim.engines.first().filter(|_| std::env::var_os("IL2_QUIET").is_none()) {
        eprintln!(
            "  engine at {alt:.0} m: {:.0} rpm, manifold {:.2} ata, pitch {:.1} deg, thrust {:.0} N, prop torque {:.0} N·m",
            e.rpm(), e.manifold, e.phi.to_degrees(), e.thrust, e.prop_torque
        );
    }
    let n = last.len() as f64;
    (last.iter().map(|v| v.0).sum::<f64>() / n * 3.6, last.iter().map(|v| v.1).sum::<f64>() / n)
}

/// Full-throttle takeoff with a simple pilot: keep straight with rudder, raise the tail at 100 km/h,
/// rotate at 170 km/h. Reports lift-off speed and ground roll.
fn takeoff_test(mut sim: Sim, field: &HeightField) -> String {
    let start = sim.pos;
    let heading0 = (sim.ori * DVec3::X).truncate().normalize();
    let mut inputs = Inputs { throttle: 1.1, gear: 1.0, tail_lock: true, ..Default::default() };
    let ground = |x: f64, y: f64| field.height_at(x, y);
    let mut t = 0.0;
    let mut last_contact = (0.0, sim.pos, 0.0, heading0);
    for _ in 0..(90.0 / TICK) as usize {
        let fwd = (sim.ori * DVec3::X).truncate().normalize();
        let yaw_err = heading0.perp_dot(fwd); // >0: nose left of the runway heading
        let kmh = sim.vel.length() * 3.6;
        inputs.rudder = (2.0 * yaw_err + 0.8 * sim.w.z).clamp(-1.0, 1.0);
        let pitch = (sim.ori * DVec3::X).z.asin().to_degrees();
        let target = if kmh < 100.0 { 99.0 } else if kmh < 170.0 { 1.5 } else { 8.0 };
        inputs.elevator = if target > 90.0 { 0.0 } else { (0.08 * (target - pitch) + 0.3 * sim.w.y).clamp(-1.0, 1.0) };
        inputs.aileron = (-0.05 * sim.bank_deg() - 0.3 * sim.w.x).clamp(-1.0, 1.0);
        sim.tick(&inputs, &ground);
        t += TICK;
        if sim.on_ground {
            last_contact = (t, sim.pos, sim.ias, fwd);
        }
        if std::env::var_os("IL2_TRACE").is_some() && ((t / TICK).round() as usize).is_multiple_of(33) {
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
    format!("takeoff: no lift-off within 90 s (speed {:.0} km/h, on ground {})", sim.vel.length() * 3.6, sim.on_ground)
}

/// Drops the aircraft from 0.5 m onto flat ground at its parked attitude, brakes on, engine off.
fn parked_test(mut sim: Sim, (h, pitch_deg): (f64, f64)) -> (f64, f64, f64) {
    let ground = |_: f64, _: f64| 0.0;
    // Dropped from 10 cm above its parked stance.
    sim.ori = glam::DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2) * glam::DQuat::from_rotation_y(-pitch_deg.to_radians());
    sim.pos.z = h + 0.1;
    sim.vel = DVec3::ZERO;
    sim.set_engines_idle(true);
    let inputs = Inputs { brakes: 1.0, gear: 1.0, ..Default::default() };
    for _ in 0..(10.0 / TICK) as usize {
        sim.tick(&inputs, &ground);
    }
    let pitch = (sim.ori * DVec3::X).z.asin().to_degrees();
    if let Some(e) = sim.engines.first().filter(|_| std::env::var_os("IL2_QUIET").is_none()) {
        eprintln!("  parked: engine {:.0} rpm, thrust {:.0} N", e.rpm(), e.thrust);
    }
    (pitch, sim.pos.z, sim.vel.length())
}

/// How the player starts: from a mission, or airborne over the map centre.
enum Start {
    Takeoff { x: f64, y: f64, heading: f64 },
    Air { pos: DVec3, toward: DVec3, speed: f64 },
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
    let mut start = Start::Default;
    let mut fuel = 1.0;
    let mission = args.mission.as_ref().map(|path| il2_formats::mission::Mission::parse(&vfs.read(path).unwrap_or_else(|e| fail(e))));
    if let (Some(path), Some(m)) = (&args.mission, &mission) {
        let wing = m.player().unwrap_or_else(|| fail("mission has no player flight"));
        args.map = format!("maps/{}", m.map);
        args.aircraft = wing.class.clone();
        fuel = wing.fuel_percent / 100.0;
        eprintln!("mission: {} on {}, flight {} ({} x {})", path, m.map, wing.name, wing.planes, wing.class);
        let ends = world::load_runways(&vfs, &args.map);
        if let Some(w0) = wing.waypoints.first() {
            start = if w0.kind.starts_with("TAKEOFF") {
                match world::takeoff_spot(&ends, w0.x, w0.y) {
                    Some((x, y, heading)) => Start::Takeoff { x, y, heading },
                    None => Start::Air { pos: DVec3::new(w0.x, w0.y, 500.0), toward: DVec3::new(w0.x, w0.y + 1.0, 500.0), speed: 100.0 },
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
    let fmd_name = il2_data::Classes::new(&vfs, &fms.keys).aircraft(&args.aircraft).map(|c| c.flight_model).unwrap_or_default();
    let field = world::load_heights(&vfs, &args.map).unwrap_or_else(|e| fail(e));

    if args.test && !matches!(start, Start::Takeoff { .. }) {
        let fmd = fms.sect_file(&fmd_name).unwrap_or_else(|e| fail(e));
        let p = |k: &str| fmd.get("Params", k).unwrap_or("?").to_owned();
        for alt in [500.0, 3000.0, p("HofVmax").parse().unwrap_or(6000.0)] {
            let sim = world::spawn_airborne(assets.fm.clone(), &field, 0.0, 120.0);
            let (kmh, z) = level_flight_test(sim, alt);
            println!("level flight at {alt:5.0} m: {kmh:5.1} km/h (held {z:5.0} m)");
        }
        println!("flight model reference: Vmax {} km/h at sea level, VmaxH {} km/h at {} m", p("Vmax"), p("VmaxH"), p("HofVmax"));
        let (pitch, z, v) = parked_test(world::spawn_airborne(assets.fm.clone(), &field, 0.0, 0.0), assets.parked);
        println!("parked on flat ground: pitch {pitch:.2} deg, origin {z:.3} m above ground, drift {v:.3} m/s (reference: Pitch {}, H {})",
            fmd.get("Gear", "Pitch").unwrap_or("?"), fmd.get("Gear", "H").unwrap_or("?"));
        println!("{}", engine_test(world::spawn_airborne(assets.fm.clone(), &field, 0.0, 0.0)));
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
        b.add_map(&vfs, &args.map, &statics_ini).unwrap_or_else(|e| fail(e));
        if let Some(m) = &mission {
            mission_objects::add_mission(&mut b, &vfs, &fms, m, &statics_ini);
        }
        b.finish()
    };
    if std::env::var_os("IL2_TRACE").is_some() {
        eprintln!("skipped classes: {:?}", map_objects.skipped);
    }
    eprintln!("map objects: {} placed in {:.1} s ({} classes skipped)", map_objects.len(), t0.elapsed().as_secs_f64(), map_objects.skipped.len());
    let env = {
        let ini = vfs.read(&args.map).map(|d| il2_formats::SectFile::parse(&d)).unwrap_or_default();
        let wp = |k: &str| ini.section_ci("WORLDPOS").and_then(|s| s.get_ci(k)).and_then(|v| il2_formats::sectfile::atof(v.trim_start_matches(['=', ' '])));
        let lat = wp("DECLIN").map_or(50.0, f64::from);
        let (month, day) = mission.as_ref().and_then(|m| m.date).unwrap_or((wp("MONTH").map_or(6, |m| m as u32), 15));
        let hours = args.time.or(mission.as_ref().map(|m| m.time_hours)).unwrap_or(12.0);
        sky::environment(hours, sky::day_of_year(month, day), lat)
    };
    let forest = forest::Forest::load(&vfs, &args.map, &terrain.types);
    let roads = roads::Roads::load(&vfs, &args.map, &terrain.types, terrain.water);
    // Guns of the chosen loadout and their tracers.
    let guns = {
        let classes = il2_data::Classes::new(&vfs, &fms.keys);
        let loadout = mission.as_ref().and_then(|m| m.player()).map_or("default".to_owned(), |w| w.weapons.clone());
        let mut armoury = weapons::Armoury::default();
        let w = classes.aircraft_weapons(&args.aircraft).unwrap_or_default();
        let guns = weapons::mount(&mut armoury, &classes, &w, &loadout, &assets.hook_frames);
        eprintln!(
            "loadout {loadout}: {}",
            guns.iter().map(|g| format!("{} ({})", g.spec.name, g.ammo)).collect::<Vec<_>>().join(", ")
        );
        guns
    };
    let tracers = effects::Tracers::load(&vfs, guns.iter().flat_map(|g| g.spec.bullets.iter().filter_map(|b| b.trace_mesh.as_deref())));
    let mut view = view::WorldView::new(map_objects, forest, roads, Some(tracers));
    let mut assets = assets;
    let plane_mesh = std::mem::take(&mut assets.mesh);
    let (h, pitch) = assets.parked;
    let mut sim = match start {
        Start::Takeoff { x, y, heading } => world::spawn_on_ground(assets.fm.clone(), &field, x, y, heading, h, pitch),
        Start::Air { pos, toward, speed } => world::spawn_at(assets.fm.clone(), pos, toward, speed),
        Start::Default => {
            let mut sim = world::spawn_airborne(assets.fm.clone(), &field, args.agl, 110.0);
            if let Some([x, y]) = args.at.as_deref().and_then(|a| <[f64; 2]>::try_from(a).ok()) {
                sim.pos = DVec3::new(x, y, field.height_at(x, y) + args.agl);
            }
            sim
        }
    };
    sim.fuel *= fuel;
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
        let mut inputs = Inputs { throttle: throttle0, gear: gear0, ..Default::default() };
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
        let mut combat = Combat::new(guns);
        let triggers = [args.fire, args.fire, false, false];
        for _ in 0..(args.seconds / TICK) as usize {
            sim.tick(&inputs, &ground);
            combat.step(&mut sim, &triggers, &field);
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
            let (yaw, pitch, dist) = (heading + o[0].to_radians(), o.get(1).copied().unwrap_or(15.0).to_radians(), o.get(2).copied().unwrap_or(12.0));
            eye = sim.pos + DVec3::new(yaw.cos() * pitch.cos(), yaw.sin() * pitch.cos(), pitch.sin()) * dist;
            let view = DMat4::look_at_rh(DVec3::ZERO, sim.pos - eye, DVec3::Z);
            vp = Mat4::perspective_rh(45f32.to_radians(), w as f32 / h as f32, 0.2, 150_000.0) * view.as_mat4();
        }
        place(&r, &terrain, &plane, &sim, eye);
        let posed = world::pose(&assets, &sim.ct);
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
        eprintln!("{} bullets in flight", combat.bullets.len());
        let px = view.draw(&mut r, &field, &terrain, vp, eye, &[&plane], &combat.bullets, true).unwrap_or_default();
        image::save_buffer(out, &px, w, h, image::ExtendedColorType::Rgba8).unwrap_or_else(|e| fail(e));
        return;
    }
    let mut game = Game {
        inputs: Inputs { throttle: throttle0, gear: gear0, ..Default::default() },
        assets,
        sim,
        field,
        terrain_data: Some(terrain),
        plane_data: Some(plane_mesh),
        renderer: None,
        terrain: None,
        plane: None,
        view,
        env,
        combat: Combat::new(guns),
        keys: HashSet::new(),
        #[cfg(feature = "gamepad")]
        gilrs: Gilrs::new().ok(),
        joystick: {
            let map = args.joy_map.as_deref().map(joystick::AxisMap::parse).transpose().unwrap_or_else(|e| fail(e)).unwrap_or_default();
            joystick::Joystick::open().map(|(j, path)| {
                eprintln!("joystick: {path}");
                (j, map)
            })
        },
        last: Instant::now(),
        acc: 0.0,
        title_timer: 0.0,
        orbit: None,
        dragging: false,
        cursor: None,
    };
    let el = EventLoop::new().expect("event loop");
    el.run_app(&mut game).expect("run");
}

/// Cold start on the ground, then temperatures after 5 minutes of full power at 300 km/h and after
/// 5 minutes at 150 km/h with the radiator closed.
fn engine_test(mut sim: Sim) -> String {
    let flat = |_: f64, _: f64| 0.0;
    sim.set_engines_cold();
    sim.toggle_engines(true);
    let mut inputs = Inputs { throttle: 0.3, gear: 1.0, brakes: 1.0, ..Default::default() };
    let mut t = 0.0;
    while t < 30.0 && !sim.engines.iter().all(|e| e.running()) {
        sim.tick(&inputs, &flat);
        t += TICK;
    }
    let started = sim.engines.iter().all(|e| e.running());
    let e = &sim.engines[0];
    let mut out = format!("cold start: {} after {t:.1} s, {:.0} rpm, oil {:.0} C", if started { "running" } else { "NOT running" }, e.rpm(), e.t_oil_out);
    let hold = |sim: &mut Sim, inputs: &Inputs, speed: f64, secs: f64| {
        for _ in 0..(secs / TICK) as usize {
            let fwd = sim.ori * DVec3::X;
            sim.vel = fwd * speed;
            sim.pos.z = 1000.0;
            sim.tick(inputs, &flat);
        }
    };
    if started {
        inputs = Inputs { throttle: 1.0, ..Default::default() };
        sim.ori = glam::DQuat::IDENTITY;
        hold(&mut sim, &inputs, 300.0 / 3.6, 300.0);
        let e = &sim.engines[0];
        out += &format!("; 5 min full power at 300 km/h: oil {:.0} C, water {:.0} C, radiator {:.2}", e.t_oil_out, e.t_water, e.radiator);
        sim.set_radiator(Some(0.0));
        hold(&mut sim, &inputs, 150.0 / 3.6, 300.0);
        let e = &sim.engines[0];
        out += &format!("; 5 min at 150 km/h, radiator closed: oil {:.0} C, water {:.0} C, events {:?}", e.t_oil_out, e.t_water, e.events);
    }
    out
}

/// Level-speed check of every aircraft in `air.ini` against its flight model's `Vmax`/`VmaxH`.
fn survey(vfs: &il2_vfs::Vfs, fms: &il2_data::FlightModels) {
    let air = il2_formats::SectFile::parse(&vfs.read("com/maddox/il2/objects/air.ini").unwrap_or_else(|e| fail(e)));
    let mut seen = HashSet::new();
    let flat = HeightField::from_raw(2, 2, &[0; 4]);
    println!("class\tfmd\tV500\tVmax\tVH\tVmaxH\tHofVmax\tparked_pitch\tPitch\tafterburner\tboost");
    let classes = il2_data::Classes::new(vfs, &fms.keys);
    for (_, v) in air.section_ci("AIR").map(|s| s.entries.as_slice()).unwrap_or_default() {
        let Some(class) = v.split_whitespace().next() else { continue };
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
        let fmd_name = classes.aircraft(class).map(|c| c.flight_model).unwrap_or_default();
        let fmd = fms.sect_file(&fmd_name).ok();
        let p = |k: &str| fmd.as_ref().and_then(|f| f.get("Params", k)).unwrap_or("?").to_owned();
        let hv: f64 = p("HofVmax").parse().unwrap_or(6000.0);
        let (v0, _) = level_flight_test(world::spawn_airborne(assets.fm.clone(), &flat, 0.0, 120.0), 500.0);
        let (vh, _) = level_flight_test(world::spawn_airborne(assets.fm.clone(), &flat, 0.0, 120.0), hv);
        let (pitch, _, _) = parked_test(world::spawn_airborne(assets.fm.clone(), &flat, 0.0, 0.0), assets.parked);
        let g = fmd.as_ref().and_then(|f| f.get("Gear", "Pitch")).unwrap_or("?").to_owned();
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
