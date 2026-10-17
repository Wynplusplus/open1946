//! Bullet hits on aircraft: the visible chunks' collision hulls (`docs/formats/msh.md`, "Collision
//! parts"), posed with the aircraft; chunks without hulls (and ground units) use the box around
//! their geometry.

use crate::world::Hull;
use glam::{DMat4, DQuat, DVec3, Mat4, Vec3};
use std::sync::Arc;

/// One collision part a shot passes through: the node, the hull (`None`: the node's box) and the
/// fraction along the traced segment where it enters.
#[derive(Clone, Debug)]
pub struct PartHit {
    pub node: usize,
    pub hull: Option<usize>,
    pub t: f64,
    /// Where the part is entered, aircraft (body) coordinates.
    pub point: glam::DVec3,
}

pub struct Target {
    /// Owner id the bullets carry: 0 the player, 1 + n the AI aircraft n.
    pub id: usize,
    pub pos: DVec3,
    pub radius: f64,
    /// World to body.
    inv: DMat4,
    /// (node, body to node, box).
    chunks: Vec<(usize, Mat4, Vec3, Vec3)>,
    /// Collision hulls per node, when the model has them.
    pub hulls: Option<Arc<Vec<Vec<Hull>>>>,
}

impl Target {
    /// `nodes` are the posed node transforms (zero for hidden nodes).
    pub fn new(
        id: usize,
        pos: DVec3,
        ori: DQuat,
        nodes: &[Mat4],
        boxes: &[Option<(Vec3, Vec3)>],
    ) -> Self {
        let mut chunks = Vec::new();
        let mut radius: f64 = 1.0;
        for (i, (m, b)) in nodes.iter().zip(boxes).enumerate() {
            let Some((lo, hi)) = *b else { continue };
            if m.x_axis.length_squared() < 1e-6 {
                continue;
            }
            for c in [lo, hi] {
                radius = radius.max(m.transform_point3(c).length() as f64);
            }
            chunks.push((i, m.inverse(), lo, hi));
        }
        Target {
            id,
            pos,
            radius,
            inv: DMat4::from_rotation_translation(ori, pos).inverse(),
            chunks,
            hulls: None,
        }
    }

    /// The parts a round striking at `at` along `dir` passes through, as the runtime traces them:
    /// from 0.1 m before the impact to 48.9 m beyond (fractions of those 50 m).
    pub fn shot_trace(&self, at: DVec3, dir: DVec3) -> Vec<PartHit> {
        self.trace(at - dir * 0.1, at + dir * 48.9)
    }

    /// Uses the model's collision hulls for the nodes that have any.
    pub fn with_hulls(mut self, hulls: &Arc<Vec<Vec<Hull>>>) -> Self {
        if hulls.iter().any(|h| !h.is_empty()) {
            self.hulls = Some(hulls.clone());
        }
        self
    }

    /// Every collision part the segment `a`→`b` (world) passes through, in order: a visible
    /// chunk's hulls, or its box when it has none.
    pub fn trace(&self, a: DVec3, b: DVec3) -> Vec<PartHit> {
        let d = b - a;
        let t = ((self.pos - a).dot(d) / d.length_squared().max(1e-12)).clamp(0.0, 1.0);
        if (a + d * t - self.pos).length_squared() > self.radius * self.radius {
            return Vec::new();
        }
        let (ab, bb) = (
            self.inv.transform_point3(a).as_vec3(),
            self.inv.transform_point3(b).as_vec3(),
        );
        let mut out = Vec::new();
        for (node, inv, lo, hi) in &self.chunks {
            let (p, q) = (inv.transform_point3(ab), inv.transform_point3(bb));
            let hulls = self
                .hulls
                .as_ref()
                .and_then(|h| h.get(*node))
                .filter(|h| !h.is_empty());
            match hulls {
                Some(hulls) => {
                    for (k, h) in hulls.iter().enumerate() {
                        if segment_box(p, q, h.lo, h.hi).is_none() {
                            continue;
                        }
                        if let Some(t) = segment_hull(p, q, h) {
                            out.push(PartHit {
                                node: *node,
                                hull: Some(k),
                                t,
                                point: ab.lerp(bb, t as f32).as_dvec3(),
                            });
                        }
                    }
                }
                None => {
                    if let Some(t) = segment_box(p, q, *lo, *hi) {
                        out.push(PartHit {
                            node: *node,
                            hull: None,
                            t,
                            point: ab.lerp(bb, t as f32).as_dvec3(),
                        });
                    }
                }
            }
        }
        out.sort_by(|x, y| x.t.total_cmp(&y.t));
        out
    }

    /// The visible chunk closest to a world point (where a nearby blast does its damage).
    pub fn nearest_node(&self, at: DVec3) -> Option<usize> {
        let p = self.inv.transform_point3(at).as_vec3();
        self.chunks
            .iter()
            .map(|(node, inv, lo, hi)| {
                let q = inv.transform_point3(p);
                (*node, (q.clamp(*lo, *hi) - q).length_squared())
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(n, _)| n)
    }

    /// The first chunk the segment `a`→`b` (world) passes through: node and fraction along it.
    pub fn hit(&self, a: DVec3, b: DVec3) -> Option<(usize, f64)> {
        if self.hulls.is_some() {
            return self.trace(a, b).first().map(|h| (h.node, h.t));
        }
        // Quick reject against the bounding sphere.
        let d = b - a;
        let t = ((self.pos - a).dot(d) / d.length_squared().max(1e-12)).clamp(0.0, 1.0);
        if (a + d * t - self.pos).length_squared() > self.radius * self.radius {
            return None;
        }
        let (ab, bb) = (
            self.inv.transform_point3(a).as_vec3(),
            self.inv.transform_point3(b).as_vec3(),
        );
        let mut best: Option<(usize, f64)> = None;
        for (node, inv, lo, hi) in &self.chunks {
            let (p, q) = (inv.transform_point3(ab), inv.transform_point3(bb));
            if let Some(t) = segment_box(p, q, *lo, *hi) {
                if best.is_none_or(|(_, bt)| t < bt) {
                    best = Some((*node, t));
                }
            }
        }
        best
    }
}

/// Slab test: where along `p`→`q` (0..1) the segment enters the box.
fn segment_box(p: Vec3, q: Vec3, lo: Vec3, hi: Vec3) -> Option<f64> {
    let d = q - p;
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for k in 0..3 {
        if d[k].abs() < 1e-9 {
            if p[k] < lo[k] || p[k] > hi[k] {
                return None;
            }
            continue;
        }
        let (mut a, mut b) = ((lo[k] - p[k]) / d[k], (hi[k] - p[k]) / d[k]);
        if a > b {
            std::mem::swap(&mut a, &mut b);
        }
        t0 = t0.max(a);
        t1 = t1.min(b);
        if t0 > t1 {
            return None;
        }
    }
    Some(t0 as f64)
}

/// Where along `p`→`q` (0..1) the segment enters the closed hull `h` (0 if it starts inside).
fn segment_hull(p: Vec3, q: Vec3, h: &Hull) -> Option<f64> {
    let d = q - p;
    let (mut first, mut count) = (f32::MAX, 0);
    for f in &h.faces {
        let (a, b, c) = (
            h.verts[f[0] as usize],
            h.verts[f[1] as usize],
            h.verts[f[2] as usize],
        );
        // Möller–Trumbore.
        let (e1, e2) = (b - a, c - a);
        let pv = d.cross(e2);
        let det = e1.dot(pv);
        if det.abs() < 1e-12 {
            continue;
        }
        let inv = 1.0 / det;
        let tv = p - a;
        let u = tv.dot(pv) * inv;
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let qv = tv.cross(e1);
        let v = d.dot(qv) * inv;
        if v < 0.0 || u + v > 1.0 {
            continue;
        }
        let t = e2.dot(qv) * inv;
        if (0.0..=1.0).contains(&t) {
            first = first.min(t);
            count += 1;
        }
    }
    match count {
        0 => None,
        // One crossing: the segment started (or ended) inside.
        1 => Some(if inside_hint(p, h) { 0.0 } else { first as f64 }),
        _ => Some(first as f64),
    }
}

/// Whether `p` lies within the hull's box (enough to tell a start inside from an end inside).
fn inside_hint(p: Vec3, h: &Hull) -> bool {
    p.cmpge(h.lo).all() && p.cmple(h.hi).all()
}

/// Which of `targets` the segment hits first, skipping the shooter `owner`.
pub fn first_hit(
    targets: &[Target],
    a: DVec3,
    b: DVec3,
    owner: usize,
) -> Option<(usize, usize, DVec3, Vec<PartHit>)> {
    let mut best: Option<(usize, usize, f64)> = None;
    for (i, t) in targets.iter().enumerate() {
        if t.id == owner {
            continue;
        }
        if let Some((node, f)) = t.hit(a, b) {
            if best.is_none_or(|(_, _, bf)| f < bf) {
                best = Some((i, node, f));
            }
        }
    }
    best.map(|(i, node, f)| {
        let at = a.lerp(b, f);
        let parts = targets[i].shot_trace(at, (b - a).normalize_or_zero());
        (targets[i].id, node, at, parts)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_hits() {
        let lo = Vec3::splat(-1.0);
        let hi = Vec3::splat(1.0);
        let t = segment_box(Vec3::new(-3.0, 0.0, 0.0), Vec3::new(3.0, 0.0, 0.0), lo, hi).unwrap();
        assert!((t - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(
            segment_box(Vec3::new(-3.0, 2.0, 0.0), Vec3::new(3.0, 2.0, 0.0), lo, hi),
            None
        );
    }
}
