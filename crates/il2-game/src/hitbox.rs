//! Bullet hits on aircraft: each visible chunk is a box around its geometry in node space, posed
//! with the aircraft (OpenIL2's stand-in for the meshes' collision shapes).

use glam::{DMat4, DQuat, DVec3, Mat4, Vec3};

pub struct Target {
    /// Owner id the bullets carry: 0 the player, 1 + n the AI aircraft n.
    pub id: usize,
    pub pos: DVec3,
    pub radius: f64,
    /// World to body.
    inv: DMat4,
    /// (node, body to node, box).
    chunks: Vec<(usize, Mat4, Vec3, Vec3)>,
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
        }
    }

    /// The first chunk the segment `a`→`b` (world) passes through: node and fraction along it.
    pub fn hit(&self, a: DVec3, b: DVec3) -> Option<(usize, f64)> {
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

/// Which of `targets` the segment hits first, skipping the shooter `owner`.
pub fn first_hit(
    targets: &[Target],
    a: DVec3,
    b: DVec3,
    owner: usize,
) -> Option<(usize, usize, DVec3)> {
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
    best.map(|(i, node, f)| (targets[i].id, node, a.lerp(b, f)))
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
