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
}

struct Object {
    class: u32,
    pos: DVec3,
    rot: glam::Quat,
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
}

/// Per-class placement rules from `static.ini`.
struct Props {
    mesh: String,
    add_height: f32,
    align_to_land: bool,
    /// Lift the origin by the mesh's own ground level (houses) rather than a fixed amount (plates).
    use_ground_level: bool,
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
        return Some(Props { mesh: sect.get_ci("Mesh")?.trim().to_owned(), add_height: 0.25, align_to_land: true, use_ground_level: false });
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
        mesh: sect.get_ci(&format!("Mesh{state}")).map(str::trim).filter(|m| !m.is_empty())?.to_owned(),
        add_height: num(&format!("AddHeight{state}")).clamp(-100.0, 100.0),
        align_to_land: num("AlignToLand") > 0.0,
        use_ground_level: true,
    })
}

/// Height of the object origin above its lowest point: the `Ground_Level` hook if present, else
/// the bottom of the bounding box.
fn lift(loader: &Loader, mesh: &MeshData, path: &str, batches: &Range<usize>) -> f32 {
    if let Some((_, p)) = loader.hier_hooks(path).ok().and_then(|h| h.into_iter().find(|(n, _)| n.eq_ignore_ascii_case("Ground_Level"))) {
        return -p.z;
    }
    let mut min_z = f32::MAX;
    for b in &mesh.batches[batches.clone()] {
        for &i in &mesh.indices[b.first as usize..(b.first + b.count) as usize] {
            let v = &mesh.vertices[i as usize];
            let node = mesh.node_init.get(v.node as usize).copied().unwrap_or(Mat4::IDENTITY);
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
                "3do/bridges/{kind}{}/{}/{}/mono1.sim",
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
                let lods = self.mesh.batches[b.start].lod.map(|(g, _)| self.mesh.lod_distances[g as usize].clone()).unwrap_or_default();
                self.classes.push(Class { batches: b, lods });
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
    pub fn place(&mut self, (class, lift): (u32, f32), x: f64, y: f64, yaw: f64, stand: Stand) {
        let field = self.field;
        let ground = field.height_at(x, y);
        let yaw_q = DQuat::from_rotation_z(yaw.to_radians());
        let tilt = || DQuat::from_rotation_arc(DVec3::Z, ground_normal(field, x, y));
        let (z, rot) = match stand {
            Stand::Ground { add, align } => (ground + (lift + add) as f64, if align { tilt() * yaw_q } else { yaw_q }),
            Stand::Plate { add } => (ground + add as f64, tilt() * yaw_q),
            Stand::Absolute(z) => (z, yaw_q),
            Stand::Parked { height, pitch } => {
                (ground + height as f64, tilt() * yaw_q * DQuat::from_rotation_y(-(pitch as f64).to_radians()))
            }
        };
        self.objects.push(Object { class, pos: DVec3::new(x, y, z), rot: rot.as_quat() });
    }

    /// Adds a map's `.static` houses, plates and bridges.
    pub fn add_map(&mut self, vfs: &Vfs, ini_path: &str, statics_ini: &SectFile) -> Result<(), String> {
        let ini = SectFile::parse(&vfs.read(ini_path).map_err(|e| e.to_string())?);
        let dir = &ini_path[..ini_path.rfind('/').map_or(0, |i| i + 1)];
        let winter = self.season == "winter";
        let rows = self.field.height as f64;
        let mut by_name: HashMap<String, Option<((u32, f32), Stand)>> = HashMap::new();
        for (name, _) in ini.section_ci("static").map(|s| s.entries.as_slice()).unwrap_or_default() {
            let Some(st) = vfs.read(&format!("{dir}{name}")).ok().and_then(|d| Statics::parse(&d).ok()) else { continue };
            let local: Vec<Option<((u32, f32), Stand)>> = st
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
                let Some((class, stand)) = local[h.class as usize] else { continue };
                self.place(class, h.x, h.y, h.yaw as f64, stand);
            }
            for b in &st.bridges {
                for seg in bridge_segments(b, winter, rows) {
                    let Some(class) = self.class(&seg.mesh, None) else { continue };
                    self.place(class, seg.pos.x, seg.pos.y, seg.yaw, Stand::Absolute(seg.pos.z));
                }
            }
        }
        self.skipped.extend(by_name.iter().filter(|(_, v)| v.is_none()).map(|(k, _)| k.clone()));
        Ok(())
    }

    /// A `buildings.House$*` or `buildings.Plate$*` class; `mission` plates sit 0.04 m higher.
    pub fn house_class(&mut self, statics_ini: &SectFile, class: &str, alive: bool, mission: bool) -> Option<((u32, f32), Stand)> {
        let p = props(statics_ini, class, alive)?;
        let c = self.class(&p.mesh, None)?;
        let stand = if p.use_ground_level {
            Stand::Ground { add: p.add_height, align: p.align_to_land }
        } else {
            Stand::Plate { add: p.add_height + if mission { 0.04 } else { 0.0 } }
        };
        Some((c, stand))
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
        let mut objects = self.objects;
        let tile_of = |p: DVec3| ((p.x / TILE).floor() as i32, (p.y / TILE).floor() as i32);
        objects.sort_by_key(|o| tile_of(o.pos));
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
        let max_dist = classes.iter().map(|c| c.lods.last().copied().unwrap_or(DEFAULT_MAX_DIST)).fold(0.0, f32::max);
        let buckets = vec![Vec::new(); classes.len() * LEVELS];
        MapObjects { data: Some(mesh), classes, objects, tiles, max_dist, buckets, skipped: self.skipped }
    }
}

/// The map's camouflage (`summer`, `winter`, `desert`, …), lower case.
pub fn season(vfs: &Vfs, ini_path: &str) -> String {
    vfs.read(ini_path)
        .ok()
        .map(|d| SectFile::parse(&d))
        .and_then(|ini| ini.section_ci("WORLDPOS")?.get_ci("CAMOUFLAGE").map(|c| c.trim_start_matches(['=', ' ']).trim().to_ascii_lowercase()))
        .unwrap_or_else(|| "summer".to_owned())
}

pub fn statics_ini(vfs: &Vfs) -> Result<SectFile, String> {
    Ok(SectFile::parse(&vfs.read("com/maddox/il2/objects/static.ini").map_err(|e| e.to_string())?))
}

impl MapObjects {
    /// The map's own objects only.
    #[cfg(test)]
    pub fn load(vfs: &Vfs, ini_path: &str, field: &HeightField) -> Result<Self, String> {
        let mut b = Builder::new(vfs, field, &season(vfs, ini_path));
        b.add_map(vfs, ini_path, &statics_ini(vfs)?)?;
        Ok(b.finish())
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// Instances near `eye` (positions relative to it), with one draw per class and LOD level.
    pub fn gather<'a>(&mut self, mesh: &'a GpuMesh, eye: DVec3, out: &mut Vec<InstanceRaw>) -> Vec<InstancedDraw<'a>> {
        for b in &mut self.buckets {
            b.clear();
        }
        let r = self.max_dist as f64;
        let (x0, x1) = (((eye.x - r) / TILE).floor() as i32, ((eye.x + r) / TILE).floor() as i32);
        let (y0, y1) = (((eye.y - r) / TILE).floor() as i32, ((eye.y + r) / TILE).floor() as i32);
        for ty in y0..=y1 {
            for tx in x0..=x1 {
                let Some(range) = self.tiles.get(&(tx, ty)) else { continue };
                for o in &self.objects[range.clone()] {
                    let rel = o.pos - eye;
                    let c = &self.classes[o.class as usize];
                    let limit = c.lods.last().copied().unwrap_or(DEFAULT_MAX_DIST);
                    let d = rel.length() as f32;
                    if d >= limit {
                        continue;
                    }
                    let Some(level) = select_lod(&c.lods, d) else { continue };
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
            let field = crate::world::load_heights(&vfs, &ini).unwrap_or_else(|e| panic!("{ini}: {e}"));
            let objects = super::MapObjects::load(&vfs, &ini, &field).unwrap_or_else(|e| panic!("{ini}: {e}"));
            let bad: Vec<_> = objects.skipped.iter().filter(|c| !c.contains(".Furniture$")).collect();
            assert!(bad.is_empty(), "{ini}: {bad:?}");
            eprintln!("{ini}: {} objects", objects.len());
        }
    }
}
