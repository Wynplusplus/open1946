//! Carrier flight decks (`docs/behavior/carrier.md`): the surfaces aircraft stand on, the take-off
//! and landing spots and the arrestor wires, in ship coordinates.

use glam::{Mat4, Vec3};
use il2_render::MeshData;

/// Grid cell size, m.
const CELL: f32 = 1.0;
/// Surfaces whose normal is closer than this to vertical count as deck.
const FLAT: f32 = 0.7;
/// The runtime looks for the deck along a line this long below the aircraft.
const REACH: f32 = 20.0;

pub struct Deck {
    x0: f32,
    y0: f32,
    nx: usize,
    ny: usize,
    /// Heights of the near-horizontal surfaces over each cell, ascending.
    cells: Vec<Vec<f32>>,
    /// `_RWY_TO` and `_RWY_LDG` frames: where take-off runs start and landings aim.
    pub takeoff: Option<Mat4>,
    pub landing: Option<Mat4>,
    /// Arrestor wire ends (`_TOWnnA`, `_TOWnnB`), aft to fore as numbered.
    pub wires: Vec<(Vec3, Vec3)>,
    /// Corners of the take-off parking area (`_ParkNN`).
    pub park: Vec<Vec3>,
}

impl Deck {
    /// From an airport ship's mesh (`ships.ini` `IsAirport`), its posed nodes and hook frames.
    pub fn build(mesh: &MeshData, nodes: &[Mat4], hooks: &[(String, Mat4)]) -> Option<Deck> {
        let hook = |n: &str| {
            hooks
                .iter()
                .find(|(h, _)| h.eq_ignore_ascii_case(n))
                .map(|(_, m)| *m)
        };
        let mut tris = Vec::new();
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for b in &mesh.batches {
            if b.lod.is_some_and(|(_, l)| l != 0) {
                continue;
            }
            let idx = &mesh.indices[b.first as usize..(b.first + b.count) as usize];
            for t in idx.chunks_exact(3) {
                let p = [t[0], t[1], t[2]].map(|i| {
                    let v = &mesh.vertices[i as usize];
                    let n = nodes
                        .get(v.node as usize)
                        .copied()
                        .unwrap_or(Mat4::IDENTITY);
                    n.transform_point3(Vec3::from(v.pos))
                });
                let n = (p[1] - p[0]).cross(p[2] - p[0]);
                if n.length_squared() < 1e-8 || n.normalize().z.abs() < FLAT {
                    continue;
                }
                for q in p {
                    lo = lo.min(q);
                    hi = hi.max(q);
                }
                tris.push(p);
            }
        }
        if tris.is_empty() {
            return None;
        }
        let nx = ((hi.x - lo.x) / CELL).ceil() as usize + 1;
        let ny = ((hi.y - lo.y) / CELL).ceil() as usize + 1;
        let mut cells: Vec<Vec<f32>> = vec![Vec::new(); nx * ny];
        for p in &tris {
            let (a, b, c) = (p[0], p[1], p[2]);
            let det = (b.y - c.y) * (a.x - c.x) + (c.x - b.x) * (a.y - c.y);
            if det.abs() < 1e-6 {
                continue;
            }
            let (tx0, tx1) = (a.x.min(b.x).min(c.x), a.x.max(b.x).max(c.x));
            let (ty0, ty1) = (a.y.min(b.y).min(c.y), a.y.max(b.y).max(c.y));
            let i0 = ((tx0 - lo.x) / CELL).floor().max(0.0) as usize;
            let i1 = (((tx1 - lo.x) / CELL).ceil() as usize).min(nx - 1);
            let j0 = ((ty0 - lo.y) / CELL).floor().max(0.0) as usize;
            let j1 = (((ty1 - lo.y) / CELL).ceil() as usize).min(ny - 1);
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let (x, y) = (
                        lo.x + (i as f32 + 0.5) * CELL,
                        lo.y + (j as f32 + 0.5) * CELL,
                    );
                    let l1 = ((b.y - c.y) * (x - c.x) + (c.x - b.x) * (y - c.y)) / det;
                    let l2 = ((c.y - a.y) * (x - c.x) + (a.x - c.x) * (y - c.y)) / det;
                    let l3 = 1.0 - l1 - l2;
                    let e = -1e-3;
                    if l1 < e || l2 < e || l3 < e {
                        continue;
                    }
                    let z = l1 * a.z + l2 * b.z + l3 * c.z;
                    let cell = &mut cells[j * nx + i];
                    if cell.iter().all(|h| (h - z).abs() > 0.3) {
                        cell.push(z);
                    }
                }
            }
        }
        for c in &mut cells {
            c.sort_by(f32::total_cmp);
        }
        let mut wires = Vec::new();
        for n in 0.. {
            let (Some(a), Some(b)) = (hook(&format!("_TOW{n:02}A")), hook(&format!("_TOW{n:02}B")))
            else {
                break;
            };
            wires.push((a.w_axis.truncate(), b.w_axis.truncate()));
        }
        let mut park = Vec::new();
        for n in 0.. {
            let Some(p) = hook(&format!("_Park{n:02}")) else {
                break;
            };
            park.push(p.w_axis.truncate());
        }
        Some(Deck {
            park,
            x0: lo.x,
            y0: lo.y,
            nx,
            ny,
            cells,
            takeoff: hook("_RWY_TO"),
            landing: hook("_RWY_LDG"),
            wires,
        })
    }

    /// The highest deck surface at ship coordinates (x, y) that lies at most 20 m below `z` and
    /// not above it (`docs/behavior/carrier.md`, "Deck").
    pub fn surface(&self, x: f32, y: f32, z: f32) -> Option<f32> {
        let (fx, fy) = ((x - self.x0) / CELL, (y - self.y0) / CELL);
        if fx < 0.0 || fy < 0.0 || fx >= self.nx as f32 || fy >= self.ny as f32 {
            return None;
        }
        self.cells[fy as usize * self.nx + fx as usize]
            .iter()
            .rev()
            .find(|&&h| h <= z && h >= z - REACH)
            .copied()
    }

    /// Ship-coordinate spot of take-off slot `slot` for an aircraft `length` × `span` m, and the
    /// take-off heading (radians from +x). OpenIL2's layout: rows across the parking area from its
    /// forward edge aft, `span + 1` m apart sideways and `length + 2` m fore and aft; `None` when
    /// the area is full or missing.
    pub fn takeoff_slot(&self, slot: usize, length: f32, span: f32) -> Option<(Vec3, f32)> {
        let f = self.takeoff?;
        let fwd = f.x_axis.truncate();
        let heading = fwd.y.atan2(fwd.x);
        if self.park.len() < 3 {
            return None;
        }
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in &self.park {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        let (dx, dy) = (length + 2.0, span + 1.0);
        let per_row = (((hi.y - lo.y) / dy).floor() as usize).max(1);
        let (row, col) = (slot / per_row, slot % per_row);
        let x = hi.x - 0.5 * length - row as f32 * dx;
        if x - 0.5 * length < lo.x - 0.5 * dx {
            return None;
        }
        let y = (col as f32 - 0.5 * (per_row as f32 - 1.0)) * dy;
        let z = self.surface(x, y, f.w_axis.z + 1.0).unwrap_or(f.w_axis.z);
        Some((Vec3::new(x, y, z), heading))
    }

    /// Half the diagonal of the deck's extent around the ship's origin, m (a quick reach test).
    pub fn radius(&self) -> f32 {
        let (x1, y1) = (
            self.x0 + self.nx as f32 * CELL,
            self.y0 + self.ny as f32 * CELL,
        );
        self.x0
            .abs()
            .max(x1.abs())
            .hypot(self.y0.abs().max(y1.abs()))
    }
}

#[cfg(test)]
mod tests {
    /// Prints a carrier's deck extent and hooks. `IL2_ROOT=… cargo test -- --ignored deck_dump`
    #[test]
    #[ignore]
    fn deck_dump() {
        let vfs = il2_vfs::Vfs::open_install(std::env::var("IL2_ROOT").unwrap()).unwrap();
        let him =
            std::env::var("IL2_SHIP").unwrap_or("3do/Ships/USSCasablancaCVE55/Hier.him".into());
        let mut loader = il2_render::scene::Loader::new(&vfs);
        let mesh = loader.load_hier(&him, false).unwrap();
        let nodes = loader.hier_rig(&him).unwrap().pose(&[]);
        let hooks = loader.hier_hook_frames(&him, &nodes).unwrap();
        for (n, m) in &hooks {
            if n.starts_with("_RWY") || n.starts_with("_TOW") || n.starts_with("_Park") {
                eprintln!(
                    "{n}: at {:.1?} x {:.2?}",
                    m.w_axis.truncate(),
                    m.x_axis.truncate()
                );
            }
        }
        let d = super::Deck::build(&mesh, &nodes, &hooks).unwrap();
        eprintln!(
            "deck x {:.1}..{:.1} y {:.1}..{:.1}",
            d.x0,
            d.x0 + d.nx as f32,
            d.y0,
            d.y0 + d.ny as f32
        );
        for x in (-90..=90).step_by(10) {
            eprintln!(
                "  x {x}: {:?}",
                d.cells[((0.0 - d.y0) as usize) * d.nx
                    + ((x as f32 - d.x0).max(0.0) as usize).min(d.nx - 1)]
            );
        }
    }
}
