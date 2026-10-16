//! Visible airframe damage: chunk damage levels (`Name_D0`, `_D1`, …), parts torn off with
//! everything they carry, and the falling wreckage (`docs/behavior/damage.md`, "Aircraft").

use glam::{DMat4, DQuat, DVec3, Mat4};
use il2_fm::damage::{self, CF, NULL_PART};
use il2_fm::Sim;
use il2_formats::map::HeightField;

/// A torn-off piece: some of the aircraft's nodes, flying on their own.
struct Wreck {
    nodes: Vec<usize>,
    pos: DVec3,
    vel: DVec3,
    ori: DQuat,
    w: DVec3,
    life: f64,
}

pub struct Airframe {
    names: Vec<String>,
    parents: Vec<Option<usize>>,
    visible: Vec<bool>,
    /// The wreck a node now flies with.
    wreck_of: Vec<Option<usize>>,
    wrecks: Vec<Wreck>,
    gear_chunks: Vec<String>,
    /// Things worth telling the pilot ("Left wing off", …).
    pub messages: Vec<String>,
    /// Points where something broke off or the aircraft exploded, with the explosion size (kg TNT).
    pub crashes: Vec<(DVec3, f64)>,
    rng: u64,
}

impl Airframe {
    pub fn new(
        names: Vec<String>,
        parents: Vec<Option<usize>>,
        hidden: &[bool],
        gear_chunks: Vec<String>,
    ) -> Self {
        let n = names.len();
        Airframe {
            visible: (0..n)
                .map(|i| !hidden.get(i).copied().unwrap_or(false))
                .collect(),
            wreck_of: vec![None; n],
            names,
            parents,
            wrecks: Vec::new(),
            gear_chunks,
            messages: Vec::new(),
            crashes: Vec::new(),
            rng: 0x2545_F491_4F6C_DD1D,
        }
    }

    fn find(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n.eq_ignore_ascii_case(name))
    }

    fn uniform(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }

    /// The chunk was struck hard (the runtime's `msgCollision`): four damage levels at once,
    /// which takes off anything that has fewer.
    pub fn collision(&mut self, sim: &mut Sim, chunk: &str) {
        self.next_dmg_levels(sim, 4, chunk);
    }

    pub fn next_dmg_levels(&mut self, sim: &mut Sim, n: usize, chunk: &str) {
        for _ in 0..n.min(4) {
            self.next_dmg_level(sim, chunk);
        }
    }

    /// Moves a `Name_Dn` chunk one damage level on, or tears the part off after the last level.
    fn next_dmg_level(&mut self, sim: &mut Sim, chunk: &str) {
        let Some((base, mut level)) = split_level(chunk) else {
            if self.find(chunk).is_some_and(|i| self.visible[i]) {
                self.cut(sim, chunk);
            }
            return;
        };
        // Find the level showing now.
        loop {
            let Some(i) = self.find(&format!("{base}_D{level}")) else {
                return;
            };
            if self.visible[i] {
                break;
            }
            if level >= 9 {
                return;
            }
            level += 1;
        }
        let next = (level < 9)
            .then(|| self.find(&format!("{base}_D{}", level + 1)))
            .flatten();
        match next {
            Some(j) => {
                let cur = self.find(&format!("{base}_D{level}")).unwrap_or(j);
                sim.hit_part(damage::part(chunk));
                self.visible[cur] = false;
                self.visible[j] = true;
            }
            None => self.cut(sim, chunk),
        }
    }

    /// Tears a part off (the runtime's `nextCUTLevel`, `cut`): its `_D` chunks and all their
    /// children leave as wreckage, the `_CAP` chunk covers the break.
    fn cut(&mut self, sim: &mut Sim, chunk: &str) {
        let part = damage::part(chunk);
        let base = split_level(chunk).map_or(chunk.to_owned(), |(b, _)| b.to_owned());
        let roots: Vec<usize> = (0..self.names.len())
            .filter(|&i| {
                let n = &self.names[i];
                n.len() > base.len() + 2
                    && n[..base.len()].eq_ignore_ascii_case(&base)
                    && n[base.len()..].to_ascii_uppercase().starts_with("_D")
                    && self.wreck_of[i].is_none()
            })
            .collect();
        if roots.is_empty() || !roots.iter().any(|&i| self.visible[i]) {
            return;
        }
        if part == CF {
            // The fuselage: the aircraft is destroyed and stays where it is as a wreck.
            if !sim.structure.destroyed {
                self.crashes.push((sim.pos, 20.0));
                self.messages.push("Aircraft destroyed".into());
                sim.cut_part(CF);
            }
            return;
        } else if part < NULL_PART {
            self.messages.push(format!("{} lost", part_label(part)));
        }
        // The subtrees under the part.
        let mut nodes = Vec::new();
        for i in 0..self.names.len() {
            let mut p = Some(i);
            while let Some(k) = p {
                if roots.contains(&k) {
                    nodes.push(i);
                    break;
                }
                p = self.parents[k];
            }
        }
        if part != CF {
            let id = self.wrecks.len();
            let spin = DVec3::new(
                self.uniform() - 0.5,
                self.uniform() - 0.5,
                self.uniform() - 0.5,
            ) * 4.0;
            let kick = sim.ori * DVec3::new(0.0, 0.0, 3.0 * (self.uniform() - 0.3));
            self.wrecks.push(Wreck {
                nodes: nodes.clone(),
                pos: sim.pos,
                vel: sim.vel + kick,
                ori: sim.ori,
                w: spin,
                life: 0.0,
            });
            for &i in &nodes {
                self.wreck_of[i] = Some(id);
            }
            self.crashes.push((sim.pos, 0.0));
        } else {
            for &i in &nodes {
                self.visible[i] = false;
            }
        }
        if let Some(cap) = self.find(&format!("{base}_CAP")) {
            self.visible[cap] = true;
        }
        if (3..=6).contains(&part) || part == CF || sim.structure.intact(part) {
            sim.cut_part(part);
        }
    }

    /// Takes the flight model's structural failures and ground strikes, and moves the wreckage.
    pub fn update(&mut self, sim: &mut Sim, field: &HeightField, dt: f64) {
        for chunk in std::mem::take(&mut sim.structure.collisions) {
            self.collision(sim, &chunk);
        }
        let speed = sim.vel.length();
        for (i, severity) in std::mem::take(&mut sim.gear.hits) {
            let Some(chunk) = self.gear_chunks.get(i).cloned() else {
                continue;
            };
            match i {
                // The main wheels and the third wheel break.
                0 | 1 | 2 if chunk_is_gear(&chunk) => {
                    let leg = ["GearL2_D0", "GearR2_D0", "GearC2_D0"][i];
                    self.collision(sim, leg);
                }
                // Airframe points: hard enough strikes above 13 m/s break the part they're on.
                _ if speed >= 13.0 => {
                    let l = chunk.to_ascii_lowercase();
                    let threshold = if l == "cf_d0" {
                        2.0
                    } else if l == "tail1_d0" {
                        1.3
                    } else {
                        0.0
                    };
                    if severity > threshold {
                        self.collision(sim, &chunk);
                    }
                }
                _ => {}
            }
        }
        for w in &mut self.wrecks {
            w.life += dt;
            let ground = field.height_at(w.pos.x, w.pos.y);
            if w.pos.z > ground + 0.3 {
                // Tumbling with strong drag.
                let drag = -w.vel * w.vel.length() * 0.004;
                w.vel += (DVec3::new(0.0, 0.0, -9.81) + drag) * dt;
                w.ori = (w.ori * DQuat::from_scaled_axis(w.w * dt)).normalize();
            } else {
                w.pos.z = ground + 0.3;
                w.vel = DVec3::ZERO;
                w.w = DVec3::ZERO;
            }
            w.pos += w.vel * dt;
        }
    }

    /// Applies visibility and the wreckage to the aircraft's posed node transforms.
    pub fn apply(&self, nodes: &mut [Mat4], sim: &Sim) {
        let n = nodes.len().min(self.visible.len());
        // Hiding a node hides only its own mesh (children keep theirs), as `chunkVisible` does.
        for i in 0..n {
            if !self.visible[i] {
                nodes[i] = Mat4::ZERO;
            }
        }
        let body = DMat4::from_rotation_translation(sim.ori, sim.pos);
        for (id, w) in self.wrecks.iter().enumerate() {
            // The piece keeps the node layout it had on the aircraft; drawn with the aircraft's
            // transform, it needs the piece's pose relative to the aircraft's.
            let piece = DMat4::from_rotation_translation(w.ori, w.pos);
            let m = (body.inverse() * piece).as_mat4();
            let far = (w.pos - sim.pos).length() > 3000.0;
            for i in 0..n {
                if self.wreck_of[i] == Some(id) {
                    nodes[i] = if far || !self.visible[i] {
                        Mat4::ZERO
                    } else {
                        m * nodes[i]
                    };
                }
            }
        }
    }
}

/// `WingLIn_D2` -> (`WingLIn`, 2).
fn split_level(chunk: &str) -> Option<(&str, u32)> {
    let b = chunk.as_bytes();
    let n = b.len();
    (n >= 3
        && b[n - 3] == b'_'
        && b[n - 2].eq_ignore_ascii_case(&b'D')
        && b[n - 1].is_ascii_digit())
    .then(|| (&chunk[..n - 3], (b[n - 1] - b'0') as u32))
}

fn chunk_is_gear(chunk: &str) -> bool {
    chunk.to_ascii_lowercase().starts_with("gear")
}

fn part_label(part: usize) -> &'static str {
    match part {
        0 => "Left aileron",
        1 => "Right aileron",
        3..=6 => "Engine",
        7 => "Tail wheel",
        9 => "Left gear",
        10 => "Right gear",
        11 | 12 => "Fin",
        15 | 16 => "Rudder",
        17 => "Left stabilizer",
        18 => "Right stabilizer",
        19 | 20 => "Tail",
        31 => "Left elevator",
        32 => "Right elevator",
        33..=35 => "Left wing",
        36..=38 => "Right wing",
        _ => "Part",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damage_levels() {
        assert_eq!(split_level("WingLIn_D2"), Some(("WingLIn", 2)));
        assert_eq!(split_level("CF"), None);
        assert_eq!(damage::part("WingLMid_D0"), 34);
        assert_eq!(damage::part("Tail1_D0"), 19);
    }
}
