//! Forest canopy near the camera (OpenIL2's own design, see `docs/formats/map.md`, "Forests").
//!
//! Wood land-type cells get stacked, terrain-following canopy shells textured with the map's
//! `[WOOD] Wood0..Wood4` layers, and their edges get vertical strips with `SideWood`. Far away the
//! terrain's own forest field texture stands in.

use glam::DVec3;
use il2_formats::map::HeightField;
use il2_formats::SectFile;
use il2_render::render::{Batch, BlendMode, Image, MeshData, Vertex};
use il2_render::scene::decode_image;
use il2_vfs::Vfs;

/// Canopy is built this far around the camera.
const RADIUS: f64 = 3000.0;
/// Rebuilt when the camera enters another tile of this size.
const REBUILD: f64 = 1000.0;
/// Edge resolution: map cells are split into sub-cells this large.
const SUB: f64 = 50.0;
const CELL: f64 = 200.0;
/// Height of the canopy top above ground.
const TOP: f32 = 20.0;
/// Shell heights, lowest (densest layer) first.
const SHELLS: [f32; 5] = [7.0, 10.5, 14.0, 17.0, 20.0];
/// Canopy textures repeat every this many metres.
const SHELL_TILE: f32 = 160.0;
/// Edge strips repeat horizontally every this many metres.
const SIDE_TILE: f32 = 20.0;

pub struct Forest {
    cols: usize,
    rows: usize,
    /// Per map pixel: is it a wood cell (land-type slots 24..27)?
    wood: Vec<bool>,
    /// Shell layers, then the side texture.
    textures: Vec<Image>,
    built_for: Option<(i64, i64)>,
    /// Origin of the current mesh's coordinates.
    pub origin: DVec3,
}

/// Deterministic noise in [-0.5, 0.5) per sub-cell.
fn noise(ix: i64, iy: i64) -> f64 {
    let mut h = (ix as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (iy as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    (h >> 11) as f64 / (1u64 << 53) as f64 - 0.5
}

impl Forest {
    /// `types`: the land-type map (width, height, bytes, row 0 at the north edge).
    pub fn load(vfs: &Vfs, ini_path: &str, types: &(u32, u32, Vec<u8>)) -> Option<Self> {
        let ini = SectFile::parse(&vfs.read(ini_path).ok()?);
        let wood = ini.section_ci("WOOD")?;
        let tex = |key: &str| -> Option<Image> {
            let v = wood.get_ci(key)?.trim_start_matches(['=', ' ', '\t']).trim();
            let path = format!("maps/_Tex/{v}");
            decode_image(&vfs.read(&path).ok()?, &path).ok()
        };
        let mut textures: Vec<Image> = (0..5).filter_map(|i| tex(&format!("Wood{i}"))).collect();
        if textures.is_empty() {
            return None;
        }
        textures.push(tex("SideWood")?);
        let (w, h, t) = types;
        Some(Forest {
            cols: *w as usize,
            rows: *h as usize,
            wood: t.iter().map(|&b| (24..28).contains(&(b & 31))).collect(),
            textures,
            built_for: None,
            origin: DVec3::ZERO,
        })
    }

    /// Wood indicator blended between cell centres, like the terrain's land-type blending.
    fn density(&self, x: f64, y: f64) -> f64 {
        let fx = x / CELL - 0.5;
        let fy = (self.rows as f64 - 1.0) - y / CELL + 0.5;
        let (c0, r0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - c0, fy - r0);
        let at = |c: f64, r: f64| {
            let (c, r) = (c as i64, r as i64);
            if c < 0 || r < 0 || c >= self.cols as i64 || r >= self.rows as i64 {
                return 0.0;
            }
            self.wood[r as usize * self.cols + c as usize] as u8 as f64
        };
        let top = at(c0, r0) * (1.0 - tx) + at(c0 + 1.0, r0) * tx;
        let bottom = at(c0, r0 + 1.0) * (1.0 - tx) + at(c0 + 1.0, r0 + 1.0) * tx;
        top * (1.0 - ty) + bottom * ty
    }

    fn inside(&self, ix: i64, iy: i64) -> bool {
        let (x, y) = ((ix as f64 + 0.5) * SUB, (iy as f64 + 0.5) * SUB);
        self.density(x, y) + 0.6 * noise(ix, iy) > 0.5
    }

    /// A new mesh when the camera has moved into another tile, else `None`.
    pub fn update(&mut self, field: &HeightField, eye: DVec3) -> Option<MeshData> {
        let key = ((eye.x / REBUILD).floor() as i64, (eye.y / REBUILD).floor() as i64);
        if self.built_for == Some(key) {
            return None;
        }
        self.built_for = Some(key);
        let centre = DVec3::new((key.0 as f64 + 0.5) * REBUILD, (key.1 as f64 + 0.5) * REBUILD, 0.0);
        self.origin = DVec3::new(centre.x, centre.y, field.height_at(centre.x, centre.y));
        Some(self.build(field, centre))
    }

    fn build(&self, field: &HeightField, centre: DVec3) -> MeshData {
        let r = RADIUS + REBUILD;
        let (x0, x1) = (((centre.x - r) / SUB).floor() as i64, ((centre.x + r) / SUB).ceil() as i64);
        let (y0, y1) = (((centre.y - r) / SUB).floor() as i64, ((centre.y + r) / SUB).ceil() as i64);
        let o = self.origin;
        let point = |x: f64, y: f64, h: f32| -> [f32; 3] {
            let z = field.height_at(x, y) + h as f64;
            (DVec3::new(x, y, z) - o).as_vec3().into()
        };
        let layers = SHELLS.len().min(self.textures.len() - 1);
        let mut shells: Vec<(Vec<Vertex>, Vec<u32>)> = vec![(Vec::new(), Vec::new()); layers];
        let mut sides: (Vec<Vertex>, Vec<u32>) = (Vec::new(), Vec::new());
        let quad = |buf: &mut (Vec<Vertex>, Vec<u32>), v: [Vertex; 4]| {
            let b = buf.0.len() as u32;
            buf.0.extend(v);
            buf.1.extend([b, b + 1, b + 2, b, b + 2, b + 3]);
        };
        let cols = (x1 - x0) as usize;
        let inside: Vec<bool> = (y0..y1).flat_map(|iy| (x0..x1).map(move |ix| (ix, iy))).map(|(ix, iy)| self.inside(ix, iy)).collect();
        let is_in = |ix: i64, iy: i64| {
            if ix < x0 || iy < y0 || ix >= x1 || iy >= y1 {
                return false;
            }
            inside[(iy - y0) as usize * cols + (ix - x0) as usize]
        };
        let up = [0.0, 0.0, 1.0];
        for iy in y0..y1 {
            for ix in x0..x1 {
                if !is_in(ix, iy) {
                    continue;
                }
                let (xa, ya, xb, yb) = (ix as f64 * SUB, iy as f64 * SUB, (ix + 1) as f64 * SUB, (iy + 1) as f64 * SUB);
                if (DVec3::new((xa + xb) / 2.0, (ya + yb) / 2.0, 0.0) - centre).truncate().length() > r {
                    continue;
                }
                // UVs relative to the sub-cell's tile so they stay precise; the sampler repeats.
                let (u0, v0) = ((xa / SHELL_TILE as f64).floor(), (ya / SHELL_TILE as f64).floor());
                let local = |x: f64, y: f64| [(x / SHELL_TILE as f64 - u0) as f32, (y / SHELL_TILE as f64 - v0) as f32];
                for (l, h) in SHELLS.iter().take(layers).enumerate() {
                    let v = |x: f64, y: f64| Vertex { pos: point(x, y, *h), normal: up, uv: local(x, y), node: 0 };
                    quad(&mut shells[l], [v(xa, ya), v(xb, ya), v(xb, yb), v(xa, yb)]);
                }
                // Edge strips where a neighbour is outside, facing out.
                for (dx, dy, a, b) in [
                    (0, -1, (xa, ya), (xb, ya)),
                    (1, 0, (xb, ya), (xb, yb)),
                    (0, 1, (xb, yb), (xa, yb)),
                    (-1, 0, (xa, yb), (xa, ya)),
                ] {
                    if is_in(ix + dx, iy + dy) {
                        continue;
                    }
                    let n = [dx as f32, dy as f32, 0.0];
                    // Along-edge texture coordinate from world position (relative to the sub-cell
                    // start for precision), so neighbouring strips line up.
                    let start = if dx == 0 { xa } else { ya };
                    let base = (start / SIDE_TILE as f64).floor();
                    let along = |p: (f64, f64)| ((if dx == 0 { p.0 } else { p.1 }) / SIDE_TILE as f64 - base) as f32;
                    let (ua, ub) = (along(a), along(b));
                    quad(
                        &mut sides,
                        [
                            Vertex { pos: point(a.0, a.1, 0.0), normal: n, uv: [ua, 1.0], node: 0 },
                            Vertex { pos: point(b.0, b.1, 0.0), normal: n, uv: [ub, 1.0], node: 0 },
                            Vertex { pos: point(b.0, b.1, TOP), normal: n, uv: [ub, 0.0], node: 0 },
                            Vertex { pos: point(a.0, a.1, TOP), normal: n, uv: [ua, 0.0], node: 0 },
                        ],
                    );
                }
            }
        }
        let mut out = MeshData { node_count: 1, ..Default::default() };
        let side_tex = self.textures.len() - 1;
        for (texture, (v, i)) in shells.into_iter().enumerate().chain(std::iter::once((side_tex, sides))) {
            let base = out.vertices.len() as u32;
            let first = out.indices.len() as u32;
            out.vertices.extend(v);
            out.indices.extend(i.iter().map(|i| i + base));
            out.batches.push(Batch {
                first,
                count: out.indices.len() as u32 - first,
                texture,
                alpha_cutoff: 0.5,
                blend: BlendMode::Opaque,
                depth_offset: false,
                lod: None,
            });
        }
        out.textures = self.textures.clone();
        out
    }
}
