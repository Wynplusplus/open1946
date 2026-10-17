//! Ground and naval columns from a mission's `[Chiefs]` (`docs/behavior/ground.md`): units moving
//! along their road, drawn with shared meshes, destroyed by gunfire and blasts.

use glam::{DMat4, DQuat, DVec2, DVec3, Mat4, Vec3};
use il2_formats::map::HeightField;
use il2_formats::mission::{Mission, RoadPoint};
use il2_formats::SectFile;
use il2_render::scene::Loader;
use il2_render::{GpuMesh, MeshData, Renderer};
use il2_vfs::Vfs;
use std::collections::HashMap;

/// First owner id of ground units in hit tests (`hitbox::Target::id`).
pub const ID_BASE: usize = 1_000_000;

struct UnitClass {
    mesh: Option<MeshData>,
    gpu: Option<GpuMesh>,
    nodes: Vec<Mat4>,
    boxes: Vec<Option<(Vec3, Vec3)>>,
    /// Typical speed, m/s, and spacing in the column, m.
    speed: f64,
    spacing: f64,
    ship: bool,
    /// Armour, m (`PanzerBodyTop`; ships: none, they take blasts only).
    armour: f64,
    /// Flight deck of airport ships (carriers).
    deck: Option<crate::deck::Deck>,
}

pub struct Unit {
    class: usize,
    pub alive: bool,
    pub pos: DVec3,
    pub yaw: f64,
    /// World velocity over the last step, m/s.
    pub vel: DVec3,
    gpu: Option<GpuMesh>,
}

pub struct Column {
    pub name: String,
    pub army: u8,
    road: Vec<RoadPoint>,
    /// Distance travelled along the road by the head of the column, m.
    s: f64,
    waited: Vec<bool>,
    wait_left: f64,
    pub units: Vec<Unit>,
}

pub struct Ground {
    classes: Vec<UnitClass>,
    pub columns: Vec<Column>,
}

/// The terrain as the flight model feels it: heights, open water and the ships' decks.
pub struct Surface<'a>(pub &'a HeightField, pub Option<&'a Ground>);

impl il2_fm::gear::Ground for Surface<'_> {
    fn height(&self, x: f64, y: f64) -> f64 {
        self.0.height_at(x, y)
    }
    fn is_water(&self, x: f64, y: f64) -> bool {
        self.0.is_water(x, y)
    }
    fn deck(&self, x: f64, y: f64, z: f64) -> Option<(f64, DVec3)> {
        self.1?.deck_at(DVec3::new(x, y, z))
    }
    fn wires(&self, x: f64, y: f64) -> Vec<(DVec3, DVec3)> {
        self.1.map_or_else(Vec::new, |g| {
            g.wires_near(DVec3::new(x, y, 0.0))
                .into_iter()
                .map(|(a, b, _)| (a, b))
                .collect()
        })
    }
}

fn ini(vfs: &Vfs, p: &str) -> SectFile {
    vfs.read(p).map(|d| SectFile::parse(&d)).unwrap_or_default()
}

impl Ground {
    pub fn from_mission(
        vfs: &Vfs,
        fms: &il2_data::FlightModels,
        m: &Mission,
        field: &HeightField,
        season: &str,
    ) -> Ground {
        let chief_ini = ini(vfs, "com/maddox/il2/objects/chief.ini");
        let technics = ini(vfs, "com/maddox/il2/objects/technics.ini");
        let ships = ini(vfs, "com/maddox/il2/objects/ships.ini");
        let classes_db = il2_data::Classes::new(vfs, &fms.keys);
        let mesh_key = match season {
            "winter" => "MeshWinter",
            "desert" => "MeshDesert",
            _ => "MeshSummer",
        };
        let mut g = Ground {
            classes: Vec::new(),
            columns: Vec::new(),
        };
        let mut by_name: HashMap<String, Option<usize>> = HashMap::new();
        let mut skipped = 0;
        for c in &m.chiefs {
            let Some(sect) = chief_ini.section(&c.kind) else {
                skipped += 1;
                continue;
            };
            let mut units = Vec::new();
            for (class, _) in &sect.entries {
                let class = class.trim().to_owned();
                let idx = *by_name.entry(class.clone()).or_insert_with(|| {
                    let uc = unit_class(vfs, &classes_db, &technics, &ships, &class, mesh_key)?;
                    g.classes.push(uc);
                    Some(g.classes.len() - 1)
                });
                if let Some(i) = idx {
                    units.push(Unit {
                        class: i,
                        alive: true,
                        pos: DVec3::ZERO,
                        yaw: 0.0,
                        vel: DVec3::ZERO,
                        gpu: None,
                    });
                }
            }
            if units.is_empty() || c.road.len() < 2 {
                skipped += 1;
                continue;
            }
            let mut col = Column {
                name: c.name.clone(),
                army: c.army,
                waited: vec![false; c.road.len()],
                road: c.road.clone(),
                s: 0.0,
                wait_left: 0.0,
                units,
            };
            // The column starts drawn out behind its first point.
            let head = col.units.len() as f64 * g.classes[col.units[0].class].spacing;
            col.s = head.min(road_length(&col.road) * 0.5);
            g.columns.push(col);
        }
        let n: usize = g.columns.iter().map(|c| c.units.len()).sum();
        eprintln!(
            "ground: {} columns, {n} units, {} classes ({skipped} columns skipped)",
            g.columns.len(),
            g.classes.len()
        );
        g.place_all(field);
        // Moving from the start (ships carry the aircraft on their decks).
        for col in &mut g.columns {
            let (i, _) = segment_at(&col.road, col.s);
            let lead = &g.classes[col.units[0].class];
            let speed = col.road[i].speed.unwrap_or(lead.speed).max(0.5);
            let moving = col.s < road_length(&col.road) && col.road[0].wait <= 0.0;
            for u in &mut col.units {
                u.vel = if moving {
                    DVec3::new(u.yaw.cos(), u.yaw.sin(), 0.0) * speed
                } else {
                    DVec3::ZERO
                };
            }
        }
        if std::env::var_os("IL2_TRACE").is_some() {
            for c in &g.columns {
                let u = &c.units[0];
                eprintln!(
                    "column {} army {}: {} units at {:.0} {:.0} {:.0}",
                    c.name,
                    c.army,
                    c.units.len(),
                    u.pos.x,
                    u.pos.y,
                    u.pos.z
                );
            }
        }
        g
    }

    fn place_all(&mut self, field: &HeightField) {
        for col in &mut self.columns {
            let mut back = 0.0;
            for u in &mut col.units {
                let c = &self.classes[u.class];
                let (p, dir) = point_at(&col.road, (col.s - back).max(0.0));
                let z = if c.ship {
                    0.0
                } else {
                    field.height_at(p.x, p.y)
                };
                u.pos = DVec3::new(p.x, p.y, z);
                u.yaw = dir.y.atan2(dir.x);
                back += c.spacing;
            }
        }
    }

    /// Moves every column along its road.
    pub fn step(&mut self, field: &HeightField, dt: f64) {
        for col in &mut self.columns {
            if col.units.iter().all(|u| !u.alive) {
                continue;
            }
            if col.wait_left > 0.0 {
                col.wait_left -= dt;
                continue;
            }
            let len = road_length(&col.road);
            if col.s >= len {
                continue;
            }
            let (i, _) = segment_at(&col.road, col.s);
            let lead = &self.classes[col.units[0].class];
            let speed = col.road[i].speed.unwrap_or(lead.speed).max(0.5);
            let before = col.s;
            col.s = (col.s + speed * dt).min(len);
            // Waits at points passed.
            let mut acc = 0.0;
            for k in 0..col.road.len() {
                if k > 0 {
                    acc += dist(&col.road[k - 1], &col.road[k]);
                }
                if acc > before && acc <= col.s && col.road[k].wait > 0.0 && !col.waited[k] {
                    col.waited[k] = true;
                    col.wait_left = col.road[k].wait * 60.0;
                    col.s = acc;
                }
            }
        }
        let before: Vec<DVec3> = self
            .columns
            .iter()
            .flat_map(|c| c.units.iter().map(|u| u.pos))
            .collect();
        self.place_all(field);
        let mut k = 0;
        for col in &mut self.columns {
            for u in &mut col.units {
                u.vel = if dt > 0.0 {
                    (u.pos - before[k]) / dt
                } else {
                    DVec3::ZERO
                };
                k += 1;
            }
        }
    }

    /// The carrier deck under `p` (within 20 m below it): its height and velocity.
    pub fn deck_at(&self, p: DVec3) -> Option<(f64, DVec3)> {
        let mut best: Option<(f64, DVec3)> = None;
        for col in &self.columns {
            for u in &col.units {
                let Some(deck) = &self.classes[u.class].deck else {
                    continue;
                };
                let rel = p - u.pos;
                if !u.alive || rel.truncate().length() > deck.radius() as f64 {
                    continue;
                }
                let local = DQuat::from_rotation_z(-u.yaw) * rel;
                if let Some(h) = deck.surface(local.x as f32, local.y as f32, local.z as f32) {
                    let z = u.pos.z + h as f64;
                    if best.is_none_or(|(b, _)| z > b) {
                        best = Some((z, u.vel));
                    }
                }
            }
        }
        best
    }

    /// Where aircraft `slot` (`length` × `span` m) of a flight taking off from unit `unit` of
    /// column `chief` starts: world position on the deck, heading (radians, counter-clockwise from
    /// east) and the ship's velocity.
    pub fn carrier_takeoff(
        &self,
        chief: &str,
        unit: usize,
        slot: usize,
        length: f64,
        span: f64,
    ) -> Option<(DVec3, f64, DVec3)> {
        let col = self.columns.iter().find(|c| c.name == chief)?;
        let u = col.units.get(unit).or(col.units.first())?;
        let deck = self.classes[u.class].deck.as_ref()?;
        let (p, heading) = deck.takeoff_slot(slot, length as f32, span as f32)?;
        let at = u.pos + DQuat::from_rotation_z(u.yaw) * p.as_dvec3();
        Some((at, u.yaw + heading as f64, u.vel))
    }

    /// Where aircraft landing on unit `unit` of column `chief` aim: the deck's landing spot
    /// (`_RWY_LDG`) in world coordinates on the deck, and the ship's heading.
    pub fn carrier_landing(&self, chief: &str, unit: usize) -> Option<(DVec3, f64)> {
        let col = self.columns.iter().find(|c| c.name == chief)?;
        let u = col.units.get(unit).or(col.units.first())?;
        if !u.alive {
            return None;
        }
        let deck = self.classes[u.class].deck.as_ref()?;
        let f = deck.landing.or(deck.takeoff)?;
        let p = f.w_axis.truncate();
        let z = deck.surface(p.x, p.y, p.z + 1.0).unwrap_or(p.z);
        let at =
            u.pos + DQuat::from_rotation_z(u.yaw) * DVec3::new(p.x as f64, p.y as f64, z as f64);
        Some((at, u.yaw))
    }

    /// Arrestor wires of the carriers near `p` in world coordinates, with the ship's velocity.
    pub fn wires_near(&self, p: DVec3) -> Vec<(DVec3, DVec3, DVec3)> {
        let mut out = Vec::new();
        for col in &self.columns {
            for u in &col.units {
                let Some(deck) = &self.classes[u.class].deck else {
                    continue;
                };
                if !u.alive || (p - u.pos).truncate().length() > deck.radius() as f64 + 50.0 {
                    continue;
                }
                let rot = DQuat::from_rotation_z(u.yaw);
                for (a, b) in &deck.wires {
                    out.push((
                        u.pos + rot * a.as_dvec3(),
                        u.pos + rot * b.as_dvec3(),
                        u.vel,
                    ));
                }
            }
        }
        out
    }

    /// Hit targets for the living units.
    pub fn targets(&self, out: &mut Vec<crate::hitbox::Target>) {
        let mut id = ID_BASE;
        for col in &self.columns {
            for u in &col.units {
                if u.alive {
                    let c = &self.classes[u.class];
                    out.push(crate::hitbox::Target::new(
                        id,
                        u.pos,
                        DQuat::from_rotation_z(u.yaw),
                        &c.nodes,
                        &c.boxes,
                    ));
                }
                id += 1;
            }
        }
    }

    fn unit_mut(&mut self, id: usize) -> Option<(&mut Unit, &UnitClass)> {
        let mut k = id.checked_sub(ID_BASE)?;
        for col in &mut self.columns {
            if k < col.units.len() {
                let u = &mut col.units[k];
                let c = &self.classes[u.class];
                return Some((u, c));
            }
            k -= col.units.len();
        }
        None
    }

    /// A round of kinetic energy `e` (J) hit unit `id`. Returns where it was destroyed.
    pub fn shot(&mut self, id: usize, e: f64, rng: &mut crate::weapons::Rng) -> Option<DVec3> {
        let (u, c) = self.unit_mut(id)?;
        if !u.alive || c.ship {
            return None;
        }
        // OpenIL2: a round that can pierce the top armour kills one time in four.
        if e > 1.5e8 * c.armour * c.armour && rng.uniform() < 0.25 {
            u.alive = false;
            return Some(u.pos);
        }
        None
    }

    /// A blast of `power` kg TNT at `at`; returns the units it destroyed.
    pub fn explode(&mut self, at: DVec3, power: f64) -> Vec<DVec3> {
        let mut out = Vec::new();
        for col in &mut self.columns {
            for u in &mut col.units {
                if !u.alive {
                    continue;
                }
                let c = &self.classes[u.class];
                let d = (u.pos - at).length();
                if d > 200.0 {
                    continue;
                }
                let got = crate::damage::received_power_realistic(power, d, 3.0);
                let needed = if c.ship {
                    30.0
                } else {
                    (40.0 * c.armour).max(0.08)
                };
                if got > needed {
                    u.alive = false;
                    out.push(u.pos);
                }
            }
        }
        out
    }

    /// Places the units relative to the camera and returns the meshes to draw.
    pub fn prepare(&mut self, r: &Renderer, eye: DVec3) -> Vec<&GpuMesh> {
        for c in &mut self.classes {
            if c.gpu.is_none() {
                if let Some(m) = c.mesh.take() {
                    c.gpu = Some(r.upload(&m));
                }
            }
        }
        for col in &mut self.columns {
            for u in &mut col.units {
                let c = &self.classes[u.class];
                let Some(base) = &c.gpu else { continue };
                if (u.pos - eye).length() > 12_000.0 {
                    continue;
                }
                let g = u.gpu.get_or_insert_with(|| r.instance_of(base));
                // Wrecks sink a little and lean.
                let rot = if u.alive {
                    DQuat::from_rotation_z(u.yaw)
                } else {
                    DQuat::from_rotation_z(u.yaw) * DQuat::from_rotation_x(0.15)
                };
                let sink = if u.alive {
                    0.0
                } else if c.ship {
                    -3.0
                } else {
                    -0.3
                };
                r.set_transform(
                    g,
                    DMat4::from_rotation_translation(rot, u.pos + DVec3::new(0.0, 0.0, sink) - eye)
                        .as_mat4(),
                );
                r.set_nodes(g, &c.nodes);
            }
        }
        self.columns
            .iter()
            .flat_map(|c| c.units.iter())
            .filter(|u| (u.pos - eye).length() <= 12_000.0)
            .filter_map(|u| u.gpu.as_ref())
            .collect()
    }
}

fn dist(a: &RoadPoint, b: &RoadPoint) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

fn road_length(r: &[RoadPoint]) -> f64 {
    r.windows(2).map(|w| dist(&w[0], &w[1])).sum()
}

/// The segment `s` metres along the road lies in, and how far into it.
fn segment_at(r: &[RoadPoint], s: f64) -> (usize, f64) {
    let mut left = s;
    for i in 0..r.len() - 1 {
        let d = dist(&r[i], &r[i + 1]);
        if left <= d || i == r.len() - 2 {
            return (i, left.min(d));
        }
        left -= d;
    }
    (0, 0.0)
}

/// Position and direction `s` metres along the road.
fn point_at(r: &[RoadPoint], s: f64) -> (DVec2, DVec2) {
    let (i, t) = segment_at(r, s);
    let (a, b) = (
        DVec2::new(r[i].x, r[i].y),
        DVec2::new(r[i + 1].x, r[i + 1].y),
    );
    let dir = (b - a).normalize_or(DVec2::X);
    (a + dir * t, dir)
}

/// A unit class's model and motion data: `technics.ini` and `ships.ini` by the class's short name
/// (`Tank$BT5` → `[BT5]`), else the first `3do/…` model its code names (trains).
fn unit_class(
    vfs: &Vfs,
    classes: &il2_data::Classes,
    technics: &SectFile,
    ships: &SectFile,
    class: &str,
    mesh_key: &str,
) -> Option<UnitClass> {
    let short = class.rsplit('$').next().unwrap_or(class);
    let num = |s: &il2_formats::sectfile::Section, k: &str| {
        s.get(k)
            .and_then(il2_formats::sectfile::atof)
            .map(f64::from)
    };
    let mut airport = false;
    let (him, speed, spacing, ship, armour) = if class.contains(".ships.") {
        let s = ships.section(short)?;
        airport = num(s, "IsAirport").is_some_and(|v| v != 0.0);
        (
            s.get("Mesh")?.trim().to_owned(),
            num(s, "Speed").map_or(5.0, |v| v / 3.6 * 2.0),
            250.0,
            true,
            0.0,
        )
    } else if let Some(s) = technics.section(short).filter(|_| class.contains('$')) {
        let mesh = s
            .get(mesh_key)
            .or_else(|| s.get("MeshSummer"))?
            .trim()
            .to_owned();
        (
            mesh,
            num(s, "SpeedAverage").map_or(20.0, |v| v / 3.6),
            num(s, "BestSpace").map_or(15.0, |v| v.max(8.0) * 1.6),
            false,
            num(s, "PanzerBodyTop").unwrap_or(0.002),
        )
    } else {
        let full = class.replace('/', ".");
        let mesh = classes.class_strings(&full).into_iter().find(|s| {
            let l = s.to_ascii_lowercase();
            l.starts_with("3do/") && (l.ends_with(".him") || l.ends_with(".sim"))
        })?;
        (mesh, 40.0 / 3.6, 18.0, false, 0.008)
    };
    let mut loader = Loader::new(vfs);
    let mut mesh = loader.load_hier(&him, false).ok()?;
    mesh.textures = loader.take_images();
    let nodes = loader
        .hier_rig(&him)
        .map(|r| r.pose(&[]))
        .unwrap_or_default();
    let mut boxes: Vec<Option<(Vec3, Vec3)>> = vec![None; nodes.len().max(1)];
    for v in &mesh.vertices {
        let Some(b) = boxes.get_mut(v.node as usize) else {
            continue;
        };
        let p = Vec3::from(v.pos);
        *b = Some(b.map_or((p, p), |(lo, hi)| (lo.min(p), hi.max(p))));
    }
    let nodes = if nodes.is_empty() {
        vec![Mat4::IDENTITY]
    } else {
        nodes
    };
    let deck = if airport {
        let hooks = loader.hier_hook_frames(&him, &nodes).unwrap_or_default();
        let d = crate::deck::Deck::build(&mesh, &nodes, &hooks);
        if d.is_none() {
            eprintln!("warning: carrier {short}: no deck surface");
        }
        d
    } else {
        None
    };
    Some(UnitClass {
        mesh: Some(mesh),
        gpu: None,
        nodes,
        boxes,
        speed,
        spacing,
        ship,
        armour,
        deck,
    })
}
