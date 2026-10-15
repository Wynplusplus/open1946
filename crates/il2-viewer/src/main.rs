mod camera;
mod render;
mod scene;

use camera::OrbitCamera;
use clap::Parser;
use render::{GpuMesh, MeshData, Renderer};
use std::path::PathBuf;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::WindowId;

#[derive(Parser)]
#[command(about = "OpenIL2 asset viewer")]
struct Args {
    /// Game install directory.
    #[arg(long, env = "IL2_ROOT")]
    root: PathBuf,
    /// Hierarchical mesh (.him), single mesh (.msh) or map (load.ini) to show.
    path: String,
    /// Also show initially hidden nodes (damage states).
    #[arg(long)]
    show_hidden: bool,
    /// Camouflage season tried first for skins (summer, winter, desert, ...).
    #[arg(long, default_value = "summer")]
    season: String,
    /// Render one frame to this PNG file and exit.
    #[arg(long)]
    screenshot: Option<PathBuf>,
    /// Camera yaw and pitch in degrees.
    #[arg(long, default_value_t = 45.0)]
    yaw: f32,
    #[arg(long, default_value_t = 20.0)]
    pitch: f32,
    /// Camera distance as a multiple of the default.
    #[arg(long, default_value_t = 1.0)]
    zoom: f32,
    /// Camera target as "x,y,z" in metres (default: center of the object).
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    target: Option<Vec<f32>>,
}

struct App {
    data: Option<MeshData>,
    renderer: Option<Renderer>,
    meshes: Vec<GpuMesh>,
    camera: OrbitCamera,
    dragging: bool,
    last_cursor: Option<(f64, f64)>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.is_some() {
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window(winit::window::Window::default_attributes().with_title("OpenIL2 viewer"))
                .expect("create window"),
        );
        let r = Renderer::new(window);
        if let Some(d) = self.data.take() {
            self.meshes.push(r.upload(&d));
        }
        self.renderer = Some(r);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(r) = self.renderer.as_mut() else { return };
        let redraw = |r: &Renderer| {
            if let Some(w) = &r.window {
                w.request_redraw();
            }
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(s) => r.resize(s.width, s.height),
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                self.dragging = state == ElementState::Pressed;
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let (true, Some((x, y))) = (self.dragging, self.last_cursor) {
                    self.camera.rotate((position.x - x) as f32, (position.y - y) as f32);
                    redraw(r);
                }
                self.last_cursor = Some((position.x, position.y));
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let steps = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                self.camera.zoom(steps);
                redraw(r);
            }
            WindowEvent::RedrawRequested => {
                let vp = self.camera.view_proj(r.aspect());
                let meshes: Vec<&GpuMesh> = self.meshes.iter().collect();
                r.render(vp, &meshes);
            }
            _ => {}
        }
    }
}

fn load(args: &Args) -> Result<MeshData, String> {
    let vfs = il2_vfs::Vfs::open_install(&args.root).map_err(|e| e.to_string())?;
    let mut loader = scene::Loader::new(&vfs);
    loader.season = Some(args.season.clone());
    let lower = args.path.to_ascii_lowercase();
    let mut data = if lower.ends_with(".him") {
        loader.load_hier(&args.path, args.show_hidden)?
    } else if lower.ends_with(".ini") {
        loader.load_map(&args.path)?
    } else {
        let mut d = MeshData { vertices: Vec::new(), indices: Vec::new(), batches: Vec::new(), textures: Vec::new() };
        loader.add_mesh(&mut d, &args.path, glam::Mat4::IDENTITY)?;
        d
    };
    for w in &loader.warnings {
        log::warn!("{w}");
        eprintln!("warning: {w}");
    }
    data.textures = loader.take_images();
    Ok(data)
}

fn main() {
    env_logger::init();
    let args = Args::parse();
    let data = match load(&args) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{}: {e}", args.path);
            std::process::exit(1);
        }
    };
    let (center, radius) = scene::bounds(&data);
    let mut camera = OrbitCamera::new(radius);
    camera.target = center;
    camera.yaw = args.yaw.to_radians();
    camera.pitch = args.pitch.to_radians();
    camera.distance *= args.zoom;
    if let Some(t) = &args.target {
        if t.len() != 3 {
            eprintln!("--target needs x,y,z");
            std::process::exit(2);
        }
        camera.target = glam::Vec3::new(t[0], t[1], t[2]);
    }
    eprintln!(
        "{}: {} vertices, {} triangles, {} textures, radius {radius:.2} m",
        args.path,
        data.vertices.len(),
        data.indices.len() / 3,
        data.textures.len()
    );

    if let Some(out) = &args.screenshot {
        let (w, h) = (1280, 720);
        let r = Renderer::headless(w, h);
        let mesh = r.upload(&data);
        let px = r.screenshot(camera.view_proj(w as f32 / h as f32), &[&mesh]);
        image::save_buffer(out, &px, w, h, image::ExtendedColorType::Rgba8).expect("save screenshot");
        return;
    }

    let event_loop = EventLoop::new().expect("event loop");
    let mut app = App { data: Some(data), renderer: None, meshes: Vec::new(), camera, dragging: false, last_cursor: None };
    event_loop.run_app(&mut app).expect("run");
}
