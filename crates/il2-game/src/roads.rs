//! Roads, highways and railways from the land-type map (`docs/formats/map.md`, "Roads"), draped on
//! the terrain near the camera. The geometry is OpenIL2's own design.

use glam::{DVec2, DVec3};
use il2_formats::map::HeightField;
use il2_formats::SectFile;
use il2_render::render::{Batch, BlendMode, Image, MeshData, Vertex};
use il2_render::scene::decode_image;
use il2_vfs::Vfs;

const CELL: f64 = 200.0;
const RADIUS: f64 = 8000.0;
const REBUILD: f64 = 1000.0;
/// Segments are split so they follow the terrain.
const STEP: f64 = 25.0;
const LIFT: f64 = 0.3;

struct Kind {
    bit: u8,
    key: &'static str,
    /// Width on the ground, metres.
    width: f32,
    /// Columns of the straight piece in the 256-pixel-wide texture atlas.
    u: (f32, f32),
}

/// Bit 0x20 is the runtime's road type 1 (friction 0.8), 0x80 type 2 (0.0), 0x40 type 3 (5.0).
const KINDS: [Kind; 3] = [
    Kind { bit: 0x20, key: "Road", width: 7.0, u: (204.0, 252.0) },
    Kind { bit: 0x80, key: "Highway", width: 10.0, u: (202.0, 256.0) },
    Kind { bit: 0x40, key: "Rail", width: 6.0, u: (210.0, 240.0) },
];

pub struct Roads {
    cols: usize,
    rows: usize,
    types: Vec<u8>,
    /// Texture per kind (`None` if the map has none).
    textures: Vec<Option<Image>>,
    built_for: Option<(i64, i64)>,
    pub origin: DVec3,
}

impl Roads {
    pub fn load(vfs: &Vfs, ini_path: &str, types: &(u32, u32, Vec<u8>)) -> Option<Self> {
        let ini = SectFile::parse(&vfs.read(ini_path).ok()?);
        let sect = ini.section_ci("ROADS")?;
        let textures = KINDS
            .iter()
            .map(|k| {
                let v = sect.get_ci(k.key)?.trim_start_matches(['=', ' ', '\t']).trim();
                let v = v.split(',').next()?.trim();
                let path = format!("maps/_Tex/{v}.tga");
                decode_image(&vfs.read(&path).ok()?, &path).ok()
            })
            .collect();
        let (w, h, t) = types;
        Some(Roads { cols: *w as usize, rows: *h as usize, types: t.clone(), textures, built_for: None, origin: DVec3::ZERO })
    }

    fn has(&self, c: i64, r: i64, bit: u8) -> bool {
        c >= 0 && r >= 0 && (c as usize) < self.cols && (r as usize) < self.rows && self.types[r as usize * self.cols + c as usize] & bit != 0
    }

    fn water(&self, c: i64, r: i64) -> bool {
        c >= 0 && r >= 0 && (c as usize) < self.cols && (r as usize) < self.rows && self.types[r as usize * self.cols + c as usize] & 31 >= 28
    }

    /// World position of a land-type pixel's centre.
    fn centre(&self, c: i64, r: i64) -> DVec2 {
        DVec2::new((c as f64 + 0.5) * CELL, (self.rows as f64 - 1.5 - r as f64) * CELL)
    }

    pub fn update(&mut self, field: &HeightField, eye: DVec3) -> Option<MeshData> {
        let key = ((eye.x / REBUILD).floor() as i64, (eye.y / REBUILD).floor() as i64);
        if self.built_for == Some(key) {
            return None;
        }
        self.built_for = Some(key);
        let centre = DVec2::new((key.0 as f64 + 0.5) * REBUILD, (key.1 as f64 + 0.5) * REBUILD);
        self.origin = DVec3::new(centre.x, centre.y, field.height_at(centre.x, centre.y));
        Some(self.build(field, centre))
    }

    fn build(&self, field: &HeightField, centre: DVec2) -> MeshData {
        let r = RADIUS + REBUILD;
        let c0 = ((centre.x - r) / CELL).floor() as i64;
        let c1 = ((centre.x + r) / CELL).ceil() as i64;
        let rr = |y: f64| (self.rows as f64 - 1.5 - y / CELL).round() as i64;
        let (r0, r1) = (rr(centre.y + r), rr(centre.y - r));
        let mut out = MeshData { node_count: 1, ..Default::default() };
        let o = self.origin;
        for (ki, k) in KINDS.iter().enumerate() {
            let Some(tex) = &self.textures[ki] else { continue };
            let first = out.indices.len() as u32;
            let (tw, th) = (tex.width as f32, tex.height as f32);
            let (ua, ub) = (k.u.0 / 256.0, k.u.1 / 256.0);
            // Along-road repeat keeps the texture's aspect ratio.
            let repeat = k.width * (th / tw) / (ub - ua);
            for row in r0..=r1 {
                for col in c0..=c1 {
                    if !self.has(col, row, k.bit) {
                        continue;
                    }
                    // Forward neighbours: E, S, SE, SW (rows grow southwards).
                    for (dc, dr) in [(1, 0), (0, 1), (1, 1), (-1, 1)] {
                        if !self.has(col + dc, row + dr, k.bit) {
                            continue;
                        }
                        // Bridges carry roads over water.
                        if self.water(col, row) || self.water(col + dc, row + dr) {
                            continue;
                        }
                        // A diagonal next to both orthogonal links would make a triangle.
                        if dc != 0 && dr != 0 && self.has(col + dc, row, k.bit) && self.has(col, row + dr, k.bit) {
                            continue;
                        }
                        let a = self.centre(col, row);
                        let b = self.centre(col + dc, row + dr);
                        if (a - centre).length() > r {
                            continue;
                        }
                        // Extend past the centres so joints overlap.
                        let dir = (b - a).normalize();
                        let side = DVec2::new(-dir.y, dir.x) * (k.width as f64 / 2.0);
                        let (a, b) = (a - dir * (k.width as f64 / 2.0), b + dir * (k.width as f64 / 2.0));
                        let len = (b - a).length();
                        let n = (len / STEP).ceil().max(1.0) as usize;
                        let base = out.vertices.len() as u32;
                        for i in 0..=n {
                            let t = i as f64 / n as f64;
                            let p = a + (b - a) * t;
                            let v = (len * t) as f32 / repeat;
                            for (s, u) in [(-1.0, ua), (1.0, ub)] {
                                let q = p + side * s;
                                let pos = DVec3::new(q.x, q.y, field.height_at(q.x, q.y) + LIFT) - o;
                                out.vertices.push(Vertex { pos: pos.as_vec3().into(), normal: [0.0, 0.0, 1.0], uv: [u, v], node: 0 });
                            }
                        }
                        for i in 0..n as u32 {
                            let q = base + 2 * i;
                            out.indices.extend([q, q + 1, q + 3, q, q + 3, q + 2]);
                        }
                    }
                }
            }
            out.textures.push(tex.clone());
            out.batches.push(Batch {
                first,
                count: out.indices.len() as u32 - first,
                texture: out.textures.len() - 1,
                alpha_cutoff: 0.0,
                blend: BlendMode::Alpha,
                depth_offset: true,
                lod: None,
            });
        }
        out
    }
}
