//! Mission objectives (`[Target]`, `docs/behavior/campaign.md`): followed during the flight and
//! judged when it ends.

use crate::ground::Ground;
use crate::objects::MapObjects;
use crate::traffic::Traffic;
use il2_fm::Sim;
use il2_formats::mission::Objective;

pub struct Tracker {
    pub objectives: Vec<Objective>,
    /// Inspect: the player came within the radius (and landed there, if asked).
    inspected: Vec<bool>,
    /// Ground objects standing in the radius at the start.
    start: Vec<usize>,
    pub time: f64,
    /// The player has flown (inspections count only after that).
    flown: bool,
}

impl Tracker {
    pub fn new(objectives: Vec<Objective>, objects: &MapObjects) -> Self {
        let start = objectives
            .iter()
            .map(|o| objects.count_in(o.x, o.y, o.radius).0)
            .collect();
        Tracker {
            inspected: vec![false; objectives.len()],
            objectives,
            start,
            time: 0.0,
            flown: false,
        }
    }

    pub fn update(&mut self, sim: &Sim, dt: f64) {
        self.time += dt;
        if !sim.on_ground && sim.vel.length() > 30.0 {
            self.flown = true;
        }
        if !self.flown {
            return;
        }
        for (i, o) in self.objectives.iter().enumerate() {
            if o.kind != 3
                || self.inspected[i]
                || o.timeout.is_some_and(|m| self.time > m as f64 * 60.0)
            {
                continue;
            }
            let near = (sim.pos.x - o.x).hypot(sim.pos.y - o.y) < o.radius;
            if near && (!o.land || (sim.on_ground && sim.vel.length() < 1.0)) {
                self.inspected[i] = true;
            }
        }
    }

    /// Whether each objective is met now.
    pub fn judge(
        &self,
        traffic: Option<&Traffic>,
        ground: Option<&Ground>,
        objects: &MapObjects,
    ) -> Vec<bool> {
        self.objectives
            .iter()
            .enumerate()
            .map(|(i, o)| {
                let need = o.percent.max(1) as f64 / 100.0;
                let name = o.object.as_deref().unwrap_or("");
                // Share of the named flight or column lost.
                let lost = || -> Option<f64> {
                    if let Some(t) = traffic {
                        let wing: Vec<_> = t
                            .planes
                            .iter()
                            .filter(|p| {
                                p.name.strip_suffix(|c: char| c.is_ascii_digit()) == Some(name)
                            })
                            .collect();
                        if !wing.is_empty() {
                            return Some(
                                wing.iter().filter(|p| p.sim.structure.destroyed).count() as f64
                                    / wing.len() as f64,
                            );
                        }
                    }
                    let col = ground?.columns.iter().find(|c| c.name == name)?;
                    Some(
                        col.units.iter().filter(|u| !u.alive).count() as f64
                            / col.units.len().max(1) as f64,
                    )
                };
                let (alive, _) = objects.count_in(o.x, o.y, o.radius);
                let destroyed = if self.start[i] == 0 {
                    0.0
                } else {
                    1.0 - alive as f64 / self.start[i] as f64
                };
                match o.kind {
                    0 => lost().is_some_and(|l| l >= need),
                    1 => self.start[i] > 0 && destroyed >= need,
                    3 => self.inspected[i],
                    4 | 5 => lost().is_none_or(|l| 1.0 - l >= need),
                    6 => destroyed <= 1.0 - need,
                    // Bridges can't be destroyed yet: attacks on them fail, defending them succeeds.
                    2 => false,
                    _ => true,
                }
            })
            .collect()
    }
}

/// The flight's outcome as the menu reads it: `result success|failure`, then one line per
/// objective (`objective <kind> <importance> <met>`).
pub fn report(t: &Tracker, met: &[bool], player_lost: bool) -> String {
    let primary_ok = t
        .objectives
        .iter()
        .zip(met)
        .filter(|(o, _)| o.importance == 0)
        .all(|(_, &m)| m);
    let success = primary_ok && !player_lost;
    let mut s = format!("result {}\n", if success { "success" } else { "failure" });
    s += &format!("player {}\n", if player_lost { "lost" } else { "alive" });
    for (o, m) in t.objectives.iter().zip(met) {
        s += &format!("objective {} {} {}\n", o.kind, o.importance, *m as u8);
    }
    s
}
