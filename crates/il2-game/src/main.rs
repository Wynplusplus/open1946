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
    #[arg(long, default_value = "3DO/Plane/Bf-109G-6Early/hier.him")]
    model: String,
    #[arg(long, default_value = "FlightModels/Bf-109G-6Early.fmd")]
    fmd: String,
    /// Start altitude above ground, metres.
    #[arg(long, default_value_t = 1500.0)]
    agl: f64,
    /// Fly headless level-flight tests instead of opening a window.
    #[arg(long)]
    test: bool,
    /// Fly hands-off for `--seconds`, render one chase-camera frame to this PNG and exit.
    #[arg(long)]
    screenshot: Option<PathBuf>,
    #[arg(long, default_value_t = 3.0)]
    seconds: f64,
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

fn place(r: &Renderer, terrain: &GpuMesh, plane: &GpuMesh, s: &Sim, eye: DVec3) {
    r.set_transform(terrain, Mat4::from_translation((-eye).as_vec3()));
    r.set_transform(plane, DMat4::from_rotation_translation(s.ori, s.pos - eye).as_mat4());
}

struct Game {
    sim: Sim,
    field: HeightField,
    terrain_data: Option<il2_render::MeshData>,
    plane_data: Option<il2_render::MeshData>,
    renderer: Option<Renderer>,
    terrain: Option<GpuMesh>,
    plane: Option<GpuMesh>,
    keys: HashSet<KeyCode>,
    #[cfg(feature = "gamepad")]
    gilrs: Option<Gilrs>,
    inputs: Inputs,
    last: Instant,
    acc: f64,
    title_timer: f64,
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
            i.throttle = (i.throttle + 0.5 * dt).min(1.0);
        }
        if k(KeyCode::PageDown) || k(KeyCode::Minus) {
            i.throttle = (i.throttle - 0.5 * dt).max(0.0);
        }
        i.brakes = if k(KeyCode::KeyB) { 1.0 } else { 0.0 };
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
        while self.acc >= TICK {
            self.sim.tick(&self.inputs, &ground);
            self.acc -= TICK;
        }
        let (Some(r), Some(terrain), Some(plane)) = (self.renderer.as_mut(), &self.terrain, &self.plane) else { return };

        // Chase camera, rendered relative to the eye to keep precision far from the origin.
        let s = &self.sim;
        let (vp, eye) = chase_camera(s, field, r.aspect());
        place(r, terrain, plane, s, eye);
        r.render(vp, 22.0, &[terrain, plane]);

        self.title_timer -= dt;
        if self.title_timer <= 0.0 {
            self.title_timer = 0.25;
            let agl = s.pos.z - field.height_at(s.pos.x, s.pos.y);
            r.window.as_ref().unwrap().set_title(&format!(
                "OpenIL2 | IAS {:4.0} km/h | ALT {:5.0} m (AGL {:5.0}) | THR {:3.0}% | FLAPS {:3.0}% | AoA {:5.1}{}",
                s.ias * 3.6,
                s.pos.z,
                agl,
                s.ct.throttle * 100.0,
                s.ct.flaps * 100.0,
                s.aoa,
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
        let r = Renderer::new(window);
        self.terrain = self.terrain_data.take().map(|d| r.upload(&d));
        self.plane = self.plane_data.take().map(|d| r.upload(&d));
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
                            KeyCode::KeyF => self.inputs.flaps = (self.inputs.flaps + 1.0 / 3.0).min(1.0),
                            KeyCode::KeyV => self.inputs.flaps = (self.inputs.flaps - 1.0 / 3.0).max(0.0),
                            KeyCode::Digit0 => self.inputs.throttle = 1.0,
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
            _ => {}
        }
    }
}

/// Headless check: hold level flight at full throttle with a simple autopilot and report the
/// stabilised speed, for comparison with the flight model's own Vmax figures.
fn level_flight_test(mut sim: Sim, alt: f64) -> (f64, f64) {
    let ground = |_: f64, _: f64| 0.0;
    sim.pos.z = alt;
    let mut inputs = Inputs { throttle: 1.0, ..Default::default() };
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
    let n = last.len() as f64;
    (last.iter().map(|v| v.0).sum::<f64>() / n * 3.6, last.iter().map(|v| v.1).sum::<f64>() / n)
}

/// Drops the aircraft from 0.5 m onto flat ground at its parked attitude, brakes on, engine off.
fn parked_test(mut sim: Sim) -> (f64, f64, f64) {
    let ground = |_: f64, _: f64| 0.0;
    let pitch0 = 12.0f64.to_radians();
    sim.ori = glam::DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2) * glam::DQuat::from_rotation_y(-pitch0);
    sim.pos.z = 2.2;
    sim.vel = DVec3::ZERO;
    let inputs = Inputs { brakes: 1.0, ..Default::default() };
    for _ in 0..(10.0 / TICK) as usize {
        sim.tick(&inputs, &ground);
    }
    let pitch = (sim.ori * DVec3::X).z.asin().to_degrees();
    (pitch, sim.pos.z, sim.vel.length())
}

fn main() {
    let args = Args::parse();
    let vfs = il2_vfs::Vfs::open_install(&args.root).unwrap_or_else(|e| fail(e));
    let fms = il2_data::FlightModels::open(&vfs).unwrap_or_else(|e| fail(e));
    let assets = world::load_aircraft(&vfs, &fms, &args.model, &args.fmd).unwrap_or_else(|e| fail(e));
    let field = world::load_heights(&vfs, &args.map).unwrap_or_else(|e| fail(e));
    eprintln!("gear contact points: {:?}", assets.fm.gear.iter().map(|g| g.pos).collect::<Vec<_>>());

    if args.test {
        let fmd = fms.sect_file(&args.fmd).unwrap_or_else(|e| fail(e));
        let p = |k: &str| fmd.get("Params", k).unwrap_or("?").to_owned();
        for alt in [500.0, 3000.0, p("HofVmax").parse().unwrap_or(6000.0)] {
            let sim = world::spawn_airborne(assets.fm.clone(), &field, 0.0, 120.0);
            let (kmh, z) = level_flight_test(sim, alt);
            println!("level flight at {alt:5.0} m: {kmh:5.1} km/h (held {z:5.0} m)");
        }
        println!("flight model reference: Vmax {} km/h at sea level, VmaxH {} km/h at {} m", p("Vmax"), p("VmaxH"), p("HofVmax"));
        let (pitch, z, v) = parked_test(world::spawn_airborne(assets.fm.clone(), &field, 0.0, 0.0));
        println!("parked on flat ground: pitch {pitch:.2} deg, origin {z:.3} m above ground, drift {v:.3} m/s (reference: Pitch {}, H {})",
            fmd.get("Gear", "Pitch").unwrap_or("?"), fmd.get("Gear", "H").unwrap_or("?"));
        return;
    }

    let terrain = {
        let mut loader = il2_render::scene::Loader::new(&vfs);
        let mut d = loader.load_map(&args.map).unwrap_or_else(|e| fail(e));
        d.textures = loader.take_images();
        d
    };
    let plane_mesh = assets.mesh;
    let mut sim = world::spawn_airborne(assets.fm, &field, args.agl, 110.0);
    sim.ct.throttle = 0.8;

    if let Some(out) = &args.screenshot {
        let inputs = Inputs { throttle: 0.8, ..Default::default() };
        let ground = |x: f64, y: f64| field.height_at(x, y);
        for _ in 0..(args.seconds / TICK) as usize {
            sim.tick(&inputs, &ground);
        }
        let (w, h) = (1280, 720);
        let r = Renderer::headless(w, h);
        let (terrain, plane) = (r.upload(&terrain), r.upload(&plane_mesh));
        let (vp, eye) = chase_camera(&sim, &field, w as f32 / h as f32);
        place(&r, &terrain, &plane, &sim, eye);
        let px = r.screenshot(vp, 22.0, &[&terrain, &plane]);
        image::save_buffer(out, &px, w, h, image::ExtendedColorType::Rgba8).unwrap_or_else(|e| fail(e));
        return;
    }
    let mut game = Game {
        inputs: Inputs { throttle: 0.8, ..Default::default() },
        sim,
        field,
        terrain_data: Some(terrain),
        plane_data: Some(plane_mesh),
        renderer: None,
        terrain: None,
        plane: None,
        keys: HashSet::new(),
        #[cfg(feature = "gamepad")]
        gilrs: Gilrs::new().ok(),
        last: Instant::now(),
        acc: 0.0,
        title_timer: 0.0,
    };
    let el = EventLoop::new().expect("event loop");
    el.run_app(&mut game).expect("run");
}

fn fail(e: impl std::fmt::Display) -> ! {
    eprintln!("error: {e}");
    std::process::exit(1)
}
