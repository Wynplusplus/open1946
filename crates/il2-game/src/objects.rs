//! Map houses and trees (`docs/formats/static.md`): loaded from the map's `.static` files and
//! `com/maddox/il2/objects/static.ini`, drawn instanced around the camera.

use glam::{DQuat, DVec3, Mat4, Vec3};
use il2_formats::map::HeightField;
use il2_formats::sectfile::atof;
use il2_formats::statics::Statics;
use il2_formats::SectFile;
use il2_render::render::select_lod;
use il2_render::scene::Loader;
use il2_render::{GpuMesh, InstanceRaw, InstancedDraw, MeshData};
use il2_vfs::Vfs;
use std::collections::HashMap;
use std::ops::Range;

/// Side of the square tiles objects are bucketed into for distance queries.
const TILE: f64 = 1000.0;
/// Draw limit for meshes whose LODs set none.
const DEFAULT_MAX_DIST: f32 = 3000.0;
/// LOD levels kept apart per class; deeper levels share the last bucket.
const LEVELS: usize = 8;

struct Class {
    batches: Range<usize>,
    /// LOD limits of the class's first mesh.
    lods: Vec<f32>,
    /// Bounding sphere in object coordinates (centre, radius).
    sphere: (Vec3, f32),
}

struct Object {
    class: u32,
    pos: DVec3,
    rot: glam::Quat,
    /// Index into `profiles` (0: cannot be destroyed).
    profile: u16,
    alive: bool,
}

/// How a destructible object dies: the mesh it then shows (none: it disappears) and its damage model.
#[derive(Clone, Copy)]
pub struct Profile {
    pub dead: Option<u32>,
    pub damage: crate::damage::HouseDamage,
    /// Height change from the live to the dead mesh's ground offset.
    pub dz: f32,
}

/// Bounding sphere of a class's batches.
fn sphere(mesh: &MeshData, batches: &Range<usize>) -> (Vec3, f32) {
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for b in &mesh.batches[batches.clone()] {
        if b.lod.is_some_and(|(_, l)| l != 0) {
            continue;
        }
        for &i in &mesh.indices[b.first as usize..(b.first + b.count) as usize] {
            let v = &mesh.vertices[i as usize];
            let node = mesh
                .node_init
                .get(v.node as usize)
                .copied()
                .unwrap_or(Mat4::IDENTITY);
            let p = node.transform_point3(Vec3::from(v.pos));
            lo = lo.min(p);
            hi = hi.max(p);
        }
    }
    if lo.x > hi.x {
        return (Vec3::ZERO, 0.0);
    }
    ((lo + hi) * 0.5, (hi - lo).length() * 0.5)
}

pub struct MapObjects {
    /// Shared mesh of all classes, until uploaded.
    pub data: Option<MeshData>,
    classes: Vec<Class>,
    objects: Vec<Object>,
    /// Object ranges per tile, objects sorted by tile.
    tiles: HashMap<(i32, i32), Range<usize>>,
    max_dist: f32,
    /// Scratch: instances per (class, level).
    buckets: Vec<Vec<InstanceRaw>>,
    /// Classes named by the map that can't be drawn.
    pub skipped: Vec<String>,
    profiles: Vec<Profile>,
    /// Objects destroyed so far, in order (track recording); bridge parts are `BRIDGE_KILL` codes.
    pub killed: Vec<usize>,
    bridges: Vec<Bridge>,
    /// Object index -> (bridge, segment).
    bridge_of: HashMap<usize, (u32, u32)>,
    /// Bridge parts that collapsed since the last drain: deck line and width (effects).
    pub collapses: Vec<(DVec3, DVec3, f64)>,
}

/// Per-class placement rules from `static.ini`.
struct Props {
    mesh: String,
    add_height: f32,
    align_to_land: bool,
    /// Lift the origin by the mesh's own ground level (houses) rather than a fixed amount (plates).
    use_ground_level: bool,
    /// `Body` and `Panzer` (houses).
    body: Option<String>,
    panzer: f64,
}

/// Rules for the runtime classes that map objects use: `buildings.House$*` and `buildings.Plate$*`.
/// Other classes (e.g. `Furniture$*`) have no runtime class in the install and are not drawn.
fn props(ini: &SectFile, class: &str, alive: bool) -> Option<Props> {
    // The runtime matches the section whose name ends with the short class name, e.g. `House$Urban_05`.
    let short = &class[class.rfind('.').map_or(0, |i| i + 1)..];
    let plate = short.starts_with("Plate$");
    if !plate && !short.starts_with("House$") {
        return None;
    }
    let sect = ini.sections.iter().find(|s| s.name.ends_with(short))?;
    if plate {
        return Some(Props {
            mesh: sect.get_ci("Mesh")?.trim().to_owned(),
            add_height: 0.25,
            align_to_land: true,
            use_ground_level: false,
            body: None,
            panzer: 0.0,
        });
    }
    // `equals` names the section that holds the remaining properties.
    let rest = sect
        .get_ci("equals")
        .filter(|e| !e.is_empty())
        .and_then(|e| ini.sections.iter().find(|s| s.name == e))
        .unwrap_or(sect);
    let num = |k: &str| rest.get_ci(k).and_then(atof).unwrap_or(0.0);
    let state = if alive { "Live" } else { "Dead" };
    Some(Props {
        mesh: sect
            .get_ci(&format!("Mesh{state}"))
            .map(str::trim)
            .filter(|m| !m.is_empty())?
            .to_owned(),
        add_height: num(&format!("AddHeight{state}")).clamp(-100.0, 100.0),
        align_to_land: num("AlignToLand") > 0.0,
        use_ground_level: true,
        body: rest.get_ci("Body").map(|b| b.trim().to_owned()),
        panzer: num("Panzer") as f64,
    })
}

/// Height of the object origin above its lowest point: the `Ground_Level` hook if present, else
/// the bottom of the bounding box.
fn lift(loader: &Loader, mesh: &MeshData, path: &str, batches: &Range<usize>) -> f32 {
    if let Some((_, p)) = loader.hier_hooks(path).ok().and_then(|h| {
        h.into_iter()
            .find(|(n, _)| n.eq_ignore_ascii_case("Ground_Level"))
    }) {
        return -p.z;
    }
    let mut min_z = f32::MAX;
    for b in &mesh.batches[batches.clone()] {
        for &i in &mesh.indices[b.first as usize..(b.first + b.count) as usize] {
            let v = &mesh.vertices[i as usize];
            let node = mesh
                .node_init
                .get(v.node as usize)
                .copied()
                .unwrap_or(Mat4::IDENTITY);
            min_z = min_z.min(node.transform_point3(Vec3::from(v.pos)).z);
        }
    }
    if min_z == f32::MAX {
        0.0
    } else {
        -min_z
    }
}

fn ground_normal(field: &HeightField, x: f64, y: f64) -> DVec3 {
    let d = 2.0;
    let dx = field.height_at(x + d, y) - field.height_at(x - d, y);
    let dy = field.height_at(x, y + d) - field.height_at(x, y - d);
    DVec3::new(-dx, -dy, 2.0 * d).normalize()
}

struct Segment {
    /// Mesh directory (`…/mid/` or `…/end/`); the file is `mono1.sim` intact, `mono2.sim` with one
    /// half and `mono3.sim` with both halves destroyed.
    mesh: String,
    pos: DVec3,
    /// Degrees, counter-clockwise from east.
    yaw: f64,
}

/// The segments of a bridge (`docs/formats/static.md`, "Bridges"), live state.
fn bridge_segments(b: &il2_formats::statics::Bridge, winter: bool, rows: f64) -> Vec<Segment> {
    let kind = match b.kind {
        0x80 => "highway",
        0x20 => "country",
        0x40 => "rail",
        _ => return Vec::new(),
    };
    let (dx, dy) = (b.end.0 - b.begin.0, b.end.1 - b.begin.1);
    let (ix, iy) = (dx.signum(), dy.signum());
    let mid_cells = dx.abs().max(dy.abs()) - 1;
    if mid_cells < 0 {
        return Vec::new();
    }
    // Direction octant; pixel rows grow southwards.
    let oct = match (ix, iy) {
        (1, 1) => 7,
        (1, -1) => 1,
        (1, _) => 0,
        (-1, 1) => 5,
        (-1, -1) => 3,
        (-1, _) => 4,
        (_, 1) => 6,
        _ => 2,
    };
    let last = 1 + 2 * mid_cells;
    let mut out = Vec::new();
    for i in 0..=last {
        let (steps, first_half) = if i == 0 {
            (0, true)
        } else if i == last {
            (1 + mid_cells, false)
        } else {
            ((i + 1) >> 1, i & 1 == 0)
        };
        let k = b.offset as f64 + if first_half { 0.25 } else { -0.25 };
        let px = (b.begin.0 + ix * steps) as f64 + 0.5 + ix as f64 * k;
        let py = (b.begin.1 + iy * steps) as f64 + 0.5 + iy as f64 * k;
        let mid = i != 0 && i != last;
        let yaw = oct as f64 * 45.0 + if i == last { 0.0 } else { 180.0 };
        out.push(Segment {
            mesh: format!(
                "3do/bridges/{kind}{}/{}/{}/",
                if winter { "_W" } else { "" },
                if oct & 1 == 0 { "short" } else { "long" },
                if mid { "mid" } else { "end" }
            ),
            pos: DVec3::new(px * 200.0, (rows - 1.0 - py) * 200.0, 0.0),
            yaw,
        });
    }
    out
}

/// Bridge segment object classes are tagged with this profile; their state lives in `Bridge`.
const BRIDGE: u16 = u16::MAX;
/// High bit of a `killed` entry for a destroyed bridge part: `bridge << 17 | segment << 1 | half`.
const BRIDGE_KILL: usize = 1 << 31;

/// A destructible bridge (`docs/behavior/damage.md`, "Bridges").
struct Bridge {
    /// Object index of each segment, from the begin end.
    objects: Vec<usize>,
    /// Segment centres at the water line.
    pos: Vec<DVec3>,
    /// Life of each segment's begin (0) and end (1) half.
    life: Vec<[f32; 2]>,
    max_life: f32,
    ignore_tnt: f32,
    /// Direction from begin to end (not normalised).
    dir: glam::DVec2,
    /// From one segment centre to the next.
    step: DVec3,
    width: f64,
    height: f64,
    height_end: f64,
    length_end: f64,
    /// Yaw of the bridge's direction, degrees.
    yaw: f64,
    /// Classes for [end, mid] segments: intact, one half and both halves destroyed.
    meshes: [[u32; 3]; 2],
    /// Some part is down (the bridge counts as destroyed).
    damaged: bool,
}

impl Bridge {
    /// Begin, middle and end points of segment `i`'s deck (`ComputeSegmentKeyPoints`).
    fn key_points(&self, i: usize) -> [DVec3; 3] {
        let p = self.pos[i];
        let (mut a, mut c) = (p - 0.5 * self.step, p + 0.5 * self.step);
        let unit = self.step.normalize_or_zero();
        let n = self.pos.len();
        let mut b = p;
        if i == 0 {
            b = a + unit * self.length_end;
            a.z = 0.0;
            b.z = self.height_end;
            c.z = self.height;
        } else if i == n - 1 {
            b = c - unit * self.length_end;
            a.z = self.height;
            b.z = self.height_end;
            c.z = 0.0;
        } else {
            a.z = self.height;
            b.z = self.height;
            c.z = self.height;
        }
        [a, b, c]
    }

    /// Distance from `p` to segment `i`'s deck line.
    fn dist(&self, i: usize, p: DVec3) -> f64 {
        let [a, _, c] = self.key_points(i);
        let (v, w) = (p - a, c - a);
        let t = v.dot(w);
        if t <= 0.0 {
            return a.distance(p);
        }
        let l2 = w.length_squared();
        if t >= l2 {
            return c.distance(p);
        }
        (v.length_squared() - t * t / l2).max(0.0).sqrt()
    }
}

/// How an object stands on the ground.
#[derive(Clone, Copy)]
pub enum Stand {
    /// On its `Ground_Level` hook (or bounding-box bottom) plus `add` metres; tilted to the ground
    /// normal if `align`.
    Ground { add: f32, align: bool },
    /// Fixed height above the ground, tilted to the ground normal (plates).
    Plate { add: f32 },
    /// At an absolute height (bridges, ships at the waterline).
    Absolute(f64),
    /// Parked aircraft: origin `height` above the ground, nose up by `pitch` degrees, on the ground normal.
    Parked { height: f32, pitch: f32 },
}

/// Collects object classes into one shared mesh and places instances.
pub struct Builder<'a> {
    pub loader: Loader<'a>,
    field: &'a HeightField,
    mesh: MeshData,
    classes: Vec<Class>,
    /// Mesh path (plus pose key) -> class index and ground lift.
    by_mesh: HashMap<String, Option<(u32, f32)>>,
    objects: Vec<Object>,
    profiles: Vec<Profile>,
    bridges: Vec<Bridge>,
    pub skipped: Vec<String>,
    pub season: String,
}

impl<'a> Builder<'a> {
    pub fn new(vfs: &'a Vfs, field: &'a HeightField, season: &str) -> Self {
        let mut loader = Loader::new(vfs);
        loader.season = Some(season.to_owned());
        Builder {
            loader,
            field,
            mesh: MeshData::default(),
            classes: Vec::new(),
            by_mesh: HashMap::new(),
            objects: Vec::new(),
            profiles: Vec::new(),
            bridges: Vec::new(),
            skipped: Vec::new(),
            season: season.to_owned(),
        }
    }

    /// Class for a mesh file (`.sim`/`.him`), optionally with posed node transforms.
    pub fn class(&mut self, path: &str, pose: Option<(&str, &[Mat4])>) -> Option<(u32, f32)> {
        let key = match pose {
            Some((k, _)) => format!("{path}#{k}"),
            None => path.to_owned(),
        };
        if let Some(c) = self.by_mesh.get(&key) {
            return *c;
        }
        let node_base = self.mesh.node_init.len().max(self.mesh.node_count);
        let c = match self.loader.append_hier(&mut self.mesh, path, false) {
            Ok(b) if !b.is_empty() => {
                if let Some((_, nodes)) = pose {
                    for (i, m) in nodes.iter().enumerate() {
                        if let Some(n) = self.mesh.node_init.get_mut(node_base + i) {
                            *n = *m;
                        }
                    }
                }
                let lift = lift(&self.loader, &self.mesh, path, &b);
                let lods = self.mesh.batches[b.start]
                    .lod
                    .map(|(g, _)| self.mesh.lod_distances[g as usize].clone())
                    .unwrap_or_default();
                let sphere = sphere(&self.mesh, &b);
                self.classes.push(Class {
                    batches: b,
                    lods,
                    sphere,
                });
                Some(((self.classes.len() - 1) as u32, lift))
            }
            Ok(_) => None,
            Err(e) => {
                self.loader.warnings.push(format!("{path}: {e}"));
                None
            }
        };
        self.by_mesh.insert(key, c);
        c
    }

    pub fn ground(&self, x: f64, y: f64) -> f64 {
        self.field.height_at(x, y)
    }

    /// Places an instance of `class` at `(x, y)` with `yaw` degrees counter-clockwise from east.
    pub fn place(&mut self, class: (u32, f32), x: f64, y: f64, yaw: f64, stand: Stand) {
        self.place_destructible(class, x, y, yaw, stand, 0);
    }

    /// Like `place`, with a damage profile from `profile()`.
    pub fn place_destructible(
        &mut self,
        (class, lift): (u32, f32),
        x: f64,
        y: f64,
        yaw: f64,
        stand: Stand,
        profile: u16,
    ) {
        let field = self.field;
        let ground = field.height_at(x, y);
        let yaw_q = DQuat::from_rotation_z(yaw.to_radians());
        let tilt = || DQuat::from_rotation_arc(DVec3::Z, ground_normal(field, x, y));
        let (z, rot) = match stand {
            Stand::Ground { add, align } => (
                ground + (lift + add) as f64,
                if align { tilt() * yaw_q } else { yaw_q },
            ),
            Stand::Plate { add } => (ground + add as f64, tilt() * yaw_q),
            Stand::Absolute(z) => (z, yaw_q),
            Stand::Parked { height, pitch } => (
                ground + height as f64,
                tilt() * yaw_q * DQuat::from_rotation_y(-(pitch as f64).to_radians()),
            ),
        };
        self.objects.push(Object {
            class,
            pos: DVec3::new(x, y, z),
            rot: rot.as_quat(),
            profile,
            alive: true,
        });
    }

    /// Adds a map's `.static` houses, plates and bridges.
    pub fn add_map(
        &mut self,
        vfs: &Vfs,
        ini_path: &str,
        statics_ini: &SectFile,
    ) -> Result<(), String> {
        let ini = SectFile::parse(&vfs.read(ini_path).map_err(|e| e.to_string())?);
        let dir = &ini_path[..ini_path.rfind('/').map_or(0, |i| i + 1)];
        let winter = self.season == "winter";
        let rows = self.field.height as f64;
        let mut by_name: HashMap<String, Option<((u32, f32), Stand, u16)>> = HashMap::new();
        for (name, _) in ini
            .section_ci("static")
            .map(|s| s.entries.as_slice())
            .unwrap_or_default()
        {
            let Some(st) = vfs
                .read(&format!("{dir}{name}"))
                .ok()
                .and_then(|d| Statics::parse(&d).ok())
            else {
                continue;
            };
            let local: Vec<Option<((u32, f32), Stand, u16)>> = st
                .house_classes
                .iter()
                .map(|c| {
                    if let Some(v) = by_name.get(c) {
                        return *v;
                    }
                    let v = self.house_class(statics_ini, c, true, false);
                    by_name.insert(c.clone(), v);
                    v
                })
                .collect();
            for h in &st.houses {
                let Some((class, stand, profile)) = local[h.class as usize] else {
                    continue;
                };
                self.place_destructible(class, h.x, h.y, h.yaw as f64, stand, profile);
            }
            for b in &st.bridges {
                self.add_bridge(b, winter, rows);
            }
        }
        self.skipped.extend(
            by_name
                .iter()
                .filter(|(_, v)| v.is_none())
                .map(|(k, _)| k.clone()),
        );
        Ok(())
    }

    /// Places a bridge's segments; every bridge, drawable or not, takes the next index
    /// (`Bridge<index>` in missions).
    fn add_bridge(&mut self, b: &il2_formats::statics::Bridge, winter: bool, rows: f64) {
        let segs = bridge_segments(b, winter, rows);
        let mut bridge = Bridge {
            objects: Vec::new(),
            pos: segs.iter().map(|s| s.pos).collect(),
            life: Vec::new(),
            max_life: if b.kind == 0x20 { 120.0 } else { 240.0 },
            ignore_tnt: if b.kind == 0x20 { 10.0 } else { 100.0 },
            dir: glam::DVec2::ZERO,
            step: DVec3::ZERO,
            width: 0.0,
            height: 0.0,
            height_end: 0.0,
            length_end: 0.0,
            yaw: 0.0,
            meshes: [[u32::MAX; 3]; 2],
            damaged: false,
        };
        let classes = |me: &mut Self, dir: &str| -> Option<[(u32, f32); 3]> {
            Some([
                me.class(&format!("{dir}mono1.sim"), None)?,
                me.class(&format!("{dir}mono2.sim"), None)?,
                me.class(&format!("{dir}mono3.sim"), None)?,
            ])
        };
        let (Some(first), Some(mid)) = (segs.first(), segs.get(1)) else {
            self.bridges.push(bridge);
            return;
        };
        let (Some(end), Some(mids)) = (classes(self, &first.mesh), classes(self, &mid.mesh)) else {
            self.bridges.push(bridge);
            return;
        };
        bridge.meshes = [end.map(|c| c.0), mids.map(|c| c.0)];
        // Deck height and width from the intact end's `Top` hook, ramp from its `Mid` hook.
        let hooks = self
            .loader
            .hier_hooks(&format!("{}mono1.sim", first.mesh))
            .unwrap_or_default();
        let hook = |n: &str| hooks.iter().find(|(h, _)| h == n).map(|(_, p)| *p);
        let diagonal = segs.len() > 1
            && (mid.pos.x - first.pos.x).abs() > 1.0
            && (mid.pos.y - first.pos.y).abs() > 1.0;
        let seg_len = 200.0 * if diagonal { 0.7071 } else { 0.5 };
        if let Some(t) = hook("Top") {
            bridge.height = t.z as f64;
            bridge.width = 2.0 * (t.y as f64).abs();
        }
        if let Some(m) = hook("Mid") {
            bridge.height_end = m.z as f64;
            bridge.length_end = -(m.x as f64) + 0.5 * seg_len;
        }
        bridge.step = mid.pos - first.pos;
        let sign = |v: f64| {
            if v > 1.0 {
                1.0
            } else if v < -1.0 {
                -1.0
            } else {
                0.0
            }
        };
        bridge.dir = glam::DVec2::new(sign(bridge.step.x), sign(bridge.step.y));
        // The begin segment faces backwards (`bridge_segments`).
        bridge.yaw = first.yaw - 180.0;
        let n = segs.len();
        for (i, seg) in segs.iter().enumerate() {
            let class = if i == 0 || i == n - 1 {
                end[0]
            } else {
                mids[0]
            };
            bridge.objects.push(self.objects.len());
            bridge.life.push([bridge.max_life; 2]);
            self.place_destructible(
                class,
                seg.pos.x,
                seg.pos.y,
                seg.yaw,
                Stand::Absolute(seg.pos.z),
                BRIDGE,
            );
        }
        self.bridges.push(bridge);
    }

    /// A `buildings.House$*` or `buildings.Plate$*` class; `mission` plates sit 0.04 m higher.
    pub fn house_class(
        &mut self,
        statics_ini: &SectFile,
        class: &str,
        alive: bool,
        mission: bool,
    ) -> Option<((u32, f32), Stand, u16)> {
        let p = props(statics_ini, class, alive)?;
        let c = self.class(&p.mesh, None)?;
        let stand = if p.use_ground_level {
            Stand::Ground {
                add: p.add_height,
                align: p.align_to_land,
            }
        } else {
            Stand::Plate {
                add: p.add_height + if mission { 0.04 } else { 0.0 },
            }
        };
        // Live houses can be destroyed: they then show their dead mesh.
        let profile = if alive {
            let damage = p
                .body
                .as_deref()
                .and_then(|b| crate::damage::HouseDamage::new(b, p.panzer));
            match damage {
                Some(damage) => {
                    let dp = props(statics_ini, class, false);
                    let dead = dp
                        .as_ref()
                        .and_then(|d| self.class(&d.mesh, None).map(|c| (c, d.add_height)));
                    let dz =
                        dead.map_or(0.0, |((_, lift), add)| (lift + add) - (c.1 + p.add_height));
                    self.profile(Profile {
                        dead: dead.map(|((c, _), _)| c),
                        damage,
                        dz,
                    })
                }
                None => 0,
            }
        } else {
            0
        };
        Some((c, stand, profile))
    }

    /// Registers a damage profile; returns its index (from 1).
    pub fn profile(&mut self, p: Profile) -> u16 {
        self.profiles.push(p);
        self.profiles.len() as u16
    }

    pub fn finish(mut self) -> MapObjects {
        for w in &self.loader.warnings {
            eprintln!("warning: {w}");
        }
        self.skipped.sort();
        self.skipped.dedup();
        let mut mesh = self.mesh;
        mesh.textures = self.loader.take_images();
        mesh.node_count = mesh.node_count.max(1);
        let tile_of = |p: DVec3| ((p.x / TILE).floor() as i32, (p.y / TILE).floor() as i32);
        let mut order: Vec<usize> = (0..self.objects.len()).collect();
        order.sort_by_key(|&i| tile_of(self.objects[i].pos));
        let mut new_index = vec![0; order.len()];
        for (n, &o) in order.iter().enumerate() {
            new_index[o] = n;
        }
        let mut slots: Vec<Option<Object>> = self.objects.into_iter().map(Some).collect();
        let objects: Vec<Object> = order.iter().map(|&i| slots[i].take().unwrap()).collect();
        let mut bridges = self.bridges;
        for b in &mut bridges {
            for o in &mut b.objects {
                *o = new_index[*o];
            }
        }
        let mut bridge_of = HashMap::new();
        for (bi, b) in bridges.iter().enumerate() {
            for (si, &o) in b.objects.iter().enumerate() {
                bridge_of.insert(o, (bi as u32, si as u32));
            }
        }
        let mut tiles = HashMap::new();
        let mut i = 0;
        while i < objects.len() {
            let t = tile_of(objects[i].pos);
            let start = i;
            while i < objects.len() && tile_of(objects[i].pos) == t {
                i += 1;
            }
            tiles.insert(t, start..i);
        }
        let classes = self.classes;
        let max_dist = classes
            .iter()
            .map(|c| c.lods.last().copied().unwrap_or(DEFAULT_MAX_DIST))
            .fold(0.0, f32::max);
        let buckets = vec![Vec::new(); classes.len() * LEVELS];
        MapObjects {
            data: Some(mesh),
            classes,
            objects,
            tiles,
            max_dist,
            buckets,
            skipped: self.skipped,
            profiles: self.profiles,
            killed: Vec::new(),
            bridges,
            bridge_of,
            collapses: Vec::new(),
        }
    }
}

/// The map's camouflage (`summer`, `winter`, `desert`, …), lower case.
pub fn season(vfs: &Vfs, ini_path: &str) -> String {
    vfs.read(ini_path)
        .ok()
        .map(|d| SectFile::parse(&d))
        .and_then(|ini| {
            ini.section_ci("WORLDPOS")?
                .get_ci("CAMOUFLAGE")
                .map(|c| c.trim_start_matches(['=', ' ']).trim().to_ascii_lowercase())
        })
        .unwrap_or_else(|| "summer".to_owned())
}

pub fn statics_ini(vfs: &Vfs) -> Result<SectFile, String> {
    Ok(SectFile::parse(
        &vfs.read("com/maddox/il2/objects/static.ini")
            .map_err(|e| e.to_string())?,
    ))
}

impl MapObjects {
    /// Destructible objects within `r` of (x, y): (still standing, all).
    pub fn count_in(&self, x: f64, y: f64, r: f64) -> (usize, usize) {
        let (mut alive, mut all) = (0, 0);
        let (x0, x1) = (
            ((x - r) / TILE).floor() as i32,
            ((x + r) / TILE).floor() as i32,
        );
        let (y0, y1) = (
            ((y - r) / TILE).floor() as i32,
            ((y + r) / TILE).floor() as i32,
        );
        for ty in y0..=y1 {
            for tx in x0..=x1 {
                let Some(range) = self.tiles.get(&(tx, ty)) else {
                    continue;
                };
                for o in &self.objects[range.clone()] {
                    if o.profile == 0 || o.profile == BRIDGE || (o.pos.x - x).hypot(o.pos.y - y) > r
                    {
                        continue;
                    }
                    all += 1;
                    alive += o.alive as usize;
                }
            }
        }
        (alive, all)
    }
    /// The map's own objects only.
    #[cfg(test)]
    pub fn load(vfs: &Vfs, ini_path: &str, field: &HeightField) -> Result<Self, String> {
        let mut b = Builder::new(vfs, field, &season(vfs, ini_path));
        b.add_map(vfs, ini_path, &statics_ini(vfs)?)?;
        Ok(b.finish())
    }

    /// The first live destructible object whose bounding sphere the segment `a → b` crosses, with
    /// the entry point.
    pub fn hit(&self, a: DVec3, b: DVec3) -> Option<(usize, DVec3)> {
        let (lo, hi) = (a.min(b), a.max(b));
        let pad = 60.0;
        let (x0, x1) = (
            ((lo.x - pad) / TILE).floor() as i32,
            ((hi.x + pad) / TILE).floor() as i32,
        );
        let (y0, y1) = (
            ((lo.y - pad) / TILE).floor() as i32,
            ((hi.y + pad) / TILE).floor() as i32,
        );
        let d = b - a;
        let len2 = d.length_squared().max(1e-9);
        let mut best: Option<(f64, usize)> = None;
        for ty in y0..=y1 {
            for tx in x0..=x1 {
                let Some(range) = self.tiles.get(&(tx, ty)) else {
                    continue;
                };
                for i in range.clone() {
                    let o = &self.objects[i];
                    if !o.alive || o.profile == 0 || o.class == u32::MAX {
                        continue;
                    }
                    if o.profile == BRIDGE
                        && self
                            .bridge_part(i)
                            .is_none_or(|(b, s)| self.bridges[b].life[s].iter().all(|&l| l <= 0.0))
                    {
                        continue;
                    }
                    let (c, r) = self.classes[o.class as usize].sphere;
                    let centre = o.pos + o.rot.as_dquat() * c.as_dvec3();
                    let t = ((centre - a).dot(d) / len2).clamp(0.0, 1.0);
                    let dist2 = (a + d * t - centre).length_squared();
                    let r = r as f64;
                    if dist2 <= r * r && best.is_none_or(|(bt, _)| t < bt) {
                        best = Some((t, i));
                    }
                }
            }
        }
        best.map(|(t, i)| (i, a + d * t))
    }

    fn bridge_part(&self, i: usize) -> Option<(usize, usize)> {
        self.bridge_of
            .get(&i)
            .map(|&(b, s)| (b as usize, s as usize))
    }

    /// Whether mission bridge `name` (`Bridge<index>`) has a part down; `None` if there is none.
    pub fn bridge_damaged(&self, name: &str) -> Option<bool> {
        let i: usize = name.trim().strip_prefix("Bridge")?.parse().ok()?;
        self.bridges.get(i).map(|b| b.damaged)
    }

    /// `tnt` kg reaching segment `s` of bridge `b` from `from` (`damagedByTNT`).
    fn bridge_tnt(
        &mut self,
        b: usize,
        s: usize,
        from: DVec3,
        tnt: f64,
        rng: &mut crate::weapons::Rng,
    ) {
        let br = &self.bridges[b];
        let ignore = br.ignore_tnt as f64;
        if tnt <= ignore {
            return;
        }
        let r = rng.uniform();
        let dmg = r * r * tnt * 2.0 + (tnt * 0.8 - ignore / 2.0);
        let rel = from - br.pos[s];
        let half = if br.dir.x * rel.x + br.dir.y * rel.y < 0.0 {
            0
        } else {
            1
        };
        let life = br.life[s][half];
        if life > 0.0 {
            self.bridge_life(b, s, half, life - dmg as f32);
        }
    }

    /// Sets a bridge half's life; a half that goes down shows its collapse, swaps the segment's
    /// mesh and brings down the halves joined to it (`LifeChanged`, `SetSegmentDamageState`).
    fn bridge_life(&mut self, b: usize, s: usize, half: usize, life: f32) {
        let mut work = vec![(s, half, life)];
        while let Some((s, half, life)) = work.pop() {
            let br = &mut self.bridges[b];
            let life = life.max(0.0);
            let was = br.life[s][half] <= 0.0;
            br.life[s][half] = life;
            if (life <= 0.0) == was {
                continue;
            }
            if life <= 0.0 {
                let [a, m, c] = br.key_points(s);
                let (p, q) = if half == 0 { (a, m) } else { (m, c) };
                self.collapses.push((p, q, br.width));
                self.killed.push(BRIDGE_KILL | b << 17 | s << 1 | half);
            }
            let state = self.bridge_mesh(b, s);
            if state == 0 {
                continue;
            }
            let br = &self.bridges[b];
            let n = br.life.len();
            let mut down = |s: usize, h: usize| {
                if br.life[s][h] > 0.0 {
                    work.push((s, h, 0.0));
                }
            };
            if state & 1 != 0 && s > 0 {
                down(s - 1, 1);
            }
            if state & 2 != 0 && s + 1 < n {
                down(s + 1, 0);
            }
            if s == 0 && state & 1 != 0 {
                down(s, 1);
            }
            if s == n - 1 && state & 2 != 0 {
                down(s, 0);
            }
        }
    }

    /// Shows segment `s` of bridge `b` in its damage state; returns the state (bit 0: begin half
    /// down, bit 1: end half down).
    fn bridge_mesh(&mut self, b: usize, s: usize) -> usize {
        let br = &mut self.bridges[b];
        let [l0, l1] = br.life[s];
        let state = (if l1 <= 0.0 { 2 } else { 0 }) + (if l0 <= 0.0 { 1 } else { 0 });
        let n = br.life.len();
        let (first, last) = (s == 0, s == n - 1);
        let mid = !first && !last;
        // Mid segments with only the end half down show the one-half mesh turned around.
        let forward = last || (mid && state == 2);
        let yaw = br.yaw + if forward { 0.0 } else { 180.0 };
        let class = br.meshes[mid as usize][match state {
            0 => 0,
            3 => 2,
            _ => 1,
        }];
        if state != 0 {
            br.damaged = true;
        }
        if class != u32::MAX {
            let o = &mut self.objects[br.objects[s]];
            o.class = class;
            o.rot = glam::Quat::from_rotation_z(yaw.to_radians() as f32);
        }
        state
    }

    /// Destroys object `i` (also for track playback); logged in `killed`.
    pub fn kill(&mut self, i: usize) {
        if i & BRIDGE_KILL != 0 {
            let (b, s, h) = ((i & !BRIDGE_KILL) >> 17, (i >> 1) & 0xffff, i & 1);
            if self
                .bridges
                .get(b)
                .and_then(|br| br.life.get(s))
                .is_some_and(|l| l[h] > 0.0)
            {
                self.bridge_life(b, s, h, 0.0);
            }
            return;
        }
        if !self
            .objects
            .get(i)
            .is_some_and(|o| o.alive && o.profile > 0 && o.profile != BRIDGE)
        {
            return;
        }
        self.killed.push(i);
        let o = &mut self.objects[i];
        o.alive = false;
        let p = self.profiles[o.profile as usize - 1];
        o.class = p.dead.unwrap_or(u32::MAX);
        o.pos.z += p.dz as f64;
    }

    /// A bullet hit on object `i` at `at`; returns whether it destroyed it.
    pub fn shot(
        &mut self,
        i: usize,
        at: DVec3,
        energy: f64,
        power_type: i32,
        speed: f64,
        rng: &mut crate::weapons::Rng,
    ) -> bool {
        let o = &self.objects[i];
        if !o.alive || o.profile == 0 {
            return false;
        }
        if o.profile == BRIDGE {
            // Kinetic energy as TNT; armour-piercing rounds don't count (`msgShot`).
            let Some((b, s)) = self.bridge_part(i) else {
                return false;
            };
            if energy <= 0.0 || power_type == 1 {
                return false;
            }
            let before = self.killed.len();
            self.bridge_tnt(b, s, at, energy * 2.4e-7, rng);
            return self.killed.len() > before;
        }
        let dmg = self.profiles[o.profile as usize - 1].damage;
        if dmg.shot_kills(energy, power_type, speed, rng) {
            self.kill(i);
            return true;
        }
        false
    }

    /// A blast of `power` kg TNT with `radius`; returns how many objects it destroyed.
    pub fn explode(
        &mut self,
        at: DVec3,
        power: f64,
        radius: f64,
        rng: &mut crate::weapons::Rng,
    ) -> usize {
        let reach = radius + 60.0;
        let (x0, x1) = (
            ((at.x - reach) / TILE).floor() as i32,
            ((at.x + reach) / TILE).floor() as i32,
        );
        let (y0, y1) = (
            ((at.y - reach) / TILE).floor() as i32,
            ((at.y + reach) / TILE).floor() as i32,
        );
        let mut victims = Vec::new();
        for ty in y0..=y1 {
            for tx in x0..=x1 {
                let Some(range) = self.tiles.get(&(tx, ty)) else {
                    continue;
                };
                for i in range.clone() {
                    let o = &self.objects[i];
                    if !o.alive || o.profile == 0 || o.profile == BRIDGE {
                        continue;
                    }
                    let (c, r) = self.classes[o.class as usize].sphere;
                    let d = (o.pos + o.rot.as_dquat() * c.as_dvec3() - at).length();
                    let got = if d - r as f64 > radius {
                        0.0
                    } else {
                        crate::damage::received_power_realistic(power, d, r as f64)
                    };
                    if got > 0.0
                        && self.profiles[o.profile as usize - 1]
                            .damage
                            .explosion_kills(got, rng)
                    {
                        victims.push(i);
                    }
                }
            }
        }
        for &i in &victims {
            self.kill(i);
        }
        victims.len() + self.explode_bridges(at, power, radius, rng)
    }

    /// A blast's effect on bridges (`msgExplosion`): TNT by the inverse square of the distance to
    /// each segment's deck beyond the bridge's width; returns the parts brought down.
    fn explode_bridges(
        &mut self,
        at: DVec3,
        power: f64,
        radius: f64,
        rng: &mut crate::weapons::Rng,
    ) -> usize {
        let before = self.killed.len();
        for b in 0..self.bridges.len() {
            for s in 0..self.bridges[b].pos.len() {
                let br = &self.bridges[b];
                let p = br.pos[s];
                if (p.x - at.x).hypot(p.y - at.y) > radius + 200.0
                    || br.life[s].iter().all(|&l| l <= 0.0)
                {
                    continue;
                }
                let d = (br.dist(s, at) - br.width).max(0.0);
                let tnt = if d >= radius {
                    0.0
                } else if d < 1.0 {
                    power
                } else {
                    power / (d * d)
                };
                self.bridge_tnt(b, s, at, tnt, rng);
            }
        }
        self.killed[before..]
            .iter()
            .filter(|&&k| k & BRIDGE_KILL != 0)
            .count()
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// Instances near `eye` (positions relative to it), with one draw per class and LOD level.
    pub fn gather<'a>(
        &mut self,
        mesh: &'a GpuMesh,
        eye: DVec3,
        out: &mut Vec<InstanceRaw>,
    ) -> Vec<InstancedDraw<'a>> {
        for b in &mut self.buckets {
            b.clear();
        }
        let r = self.max_dist as f64;
        let (x0, x1) = (
            ((eye.x - r) / TILE).floor() as i32,
            ((eye.x + r) / TILE).floor() as i32,
        );
        let (y0, y1) = (
            ((eye.y - r) / TILE).floor() as i32,
            ((eye.y + r) / TILE).floor() as i32,
        );
        for ty in y0..=y1 {
            for tx in x0..=x1 {
                let Some(range) = self.tiles.get(&(tx, ty)) else {
                    continue;
                };
                for o in &self.objects[range.clone()] {
                    if o.class == u32::MAX {
                        continue;
                    }
                    let rel = o.pos - eye;
                    let c = &self.classes[o.class as usize];
                    let limit = c.lods.last().copied().unwrap_or(DEFAULT_MAX_DIST);
                    let d = rel.length() as f32;
                    if d >= limit {
                        continue;
                    }
                    let Some(level) = select_lod(&c.lods, d) else {
                        continue;
                    };
                    let level = (level as usize).min(LEVELS - 1);
                    let m = Mat4::from_rotation_translation(o.rot, rel.as_vec3());
                    self.buckets[o.class as usize * LEVELS + level].push(InstanceRaw::from_mat4(m));
                }
            }
        }
        out.clear();
        let mut draws = Vec::new();
        for (i, b) in self.buckets.iter().enumerate() {
            if b.is_empty() {
                continue;
            }
            let start = out.len() as u32;
            out.extend_from_slice(b);
            draws.push(InstancedDraw {
                mesh,
                batches: self.classes[i / LEVELS].batches.clone(),
                level: (i % LEVELS) as u8,
                instances: start..out.len() as u32,
            });
        }
        draws
    }
}

#[cfg(test)]
mod tests {
    /// A 250 kg-class blast on a bridge's middle brings down parts of it and swaps their meshes.
    /// Run with `IL2_ROOT=/path/to/game cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn bridge_goes_down() {
        let vfs = il2_vfs::Vfs::open_install(std::env::var("IL2_ROOT").unwrap()).unwrap();
        let ini = "maps/Smolensk/load.ini";
        let field = crate::world::load_heights(&vfs, ini).unwrap();
        let mut objects = super::MapObjects::load(&vfs, ini, &field).unwrap();
        let (bi, b) = objects
            .bridges
            .iter()
            .enumerate()
            .find(|(_, b)| b.pos.len() >= 5 && b.meshes[1][0] != u32::MAX)
            .expect("a bridge");
        eprintln!(
            "bridge {bi}: {} segments, width {:.1} height {:.1} end {:.1}/{:.1}, life {}",
            b.pos.len(),
            b.width,
            b.height,
            b.height_end,
            b.length_end,
            b.max_life
        );
        assert!(b.width > 2.0 && b.height > 1.0);
        let s = b.pos.len() / 2;
        let at = b.pos[s] + glam::DVec3::new(0.0, 0.0, b.height + 1.0);
        let class0 = objects.objects[b.objects[s]].class;
        let name = format!("Bridge{bi}");
        assert_eq!(objects.bridge_damaged(&name), Some(false));
        let mut rng = crate::weapons::Rng::new(1);
        // Bridges are tough: a direct hit takes 50..300 of 240 life.
        let mut down = 0;
        let mut drops = 0;
        while down == 0 && drops < 20 {
            down = objects.explode(at, 125.0, 150.0, &mut rng);
            drops += 1;
        }
        let b = &objects.bridges[bi];
        eprintln!(
            "parts down after {drops} blasts: {down}, lives {:?}",
            b.life
        );
        assert!(down >= 2, "a hit halves and its neighbour");
        assert_eq!(objects.bridge_damaged(&name), Some(true));
        assert_ne!(objects.objects[b.objects[s]].class, class0);
        assert!(!objects.collapses.is_empty());
        // A rifle round does nothing.
        let far = objects.bridges[bi].objects[0];
        assert!(!objects.shot(far, at, 3500.0, 0, 800.0, &mut rng));
    }

    /// Every shipped map: static files parse and every class except `Furniture$*` gets a mesh.
    /// Run with `IL2_ROOT=/path/to/game cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn all_maps_place_objects() {
        let vfs = il2_vfs::Vfs::open_install(std::env::var("IL2_ROOT").unwrap()).unwrap();
        let all = il2_formats::SectFile::parse(&vfs.read("maps/all.ini").unwrap());
        let mut seen = std::collections::HashSet::new();
        for (_, v) in &all.section_ci("all").unwrap().entries {
            let ini = format!("maps/{}", v.split_whitespace().next().unwrap());
            if !seen.insert(ini.to_ascii_lowercase()) {
                continue;
            }
            let field =
                crate::world::load_heights(&vfs, &ini).unwrap_or_else(|e| panic!("{ini}: {e}"));
            let objects = super::MapObjects::load(&vfs, &ini, &field)
                .unwrap_or_else(|e| panic!("{ini}: {e}"));
            let bad: Vec<_> = objects
                .skipped
                .iter()
                .filter(|c| !c.contains(".Furniture$"))
                .collect();
            assert!(bad.is_empty(), "{ini}: {bad:?}");
            eprintln!("{ini}: {} objects", objects.len());
        }
    }
}
