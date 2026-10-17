//! Smoke, fire and leak trails of damaged aircraft (`docs/behavior/damage.md`, "Systems"): the
//! effects the runtime attaches to the tank, engine and oil hooks for each system state.

use crate::particles::Effects;
use glam::Mat4;
use il2_fm::Sim;
use std::collections::HashMap;

const DIR: &str = "3DO/Effects/Aircraft/";

/// Per tank state (0..6), three slots; first in flight, then below 10 m/s.
const TANK: [[&str; 21]; 2] = [
    [
        "",
        "",
        "",
        "RedLeakTSPD",
        "",
        "",
        "RedLeakTSPD",
        "",
        "",
        "TankBlackMediumSPD",
        "TankBlackMediumTSPD",
        "",
        "BlackHeavySPD",
        "BlackHeavyTSPD",
        "",
        "FireSPD",
        "BlackHeavySPD",
        "BlackHeavyTSPD",
        "FireSPDLong",
        "BlackHeavySPD",
        "BlackHeavyTSPD",
    ],
    [
        "",
        "",
        "",
        "RedLeakGND",
        "",
        "",
        "RedLeakGND",
        "",
        "",
        "BlackMediumGND",
        "",
        "",
        "BlackHeavyGND",
        "",
        "",
        "FireGND",
        "BlackHeavyGND",
        "",
        "FireGND",
        "BlackHeavyGND",
        "",
    ],
];

/// Per engine state (0..4), three slots; in flight, then below 10 m/s.
const ENGINE: [[&str; 15]; 2] = [
    [
        "",
        "",
        "",
        "GraySmallSPD",
        "GraySmallTSPD",
        "",
        "EngineBlackMediumSPD",
        "EngineBlackMediumTSPD",
        "",
        "BlackHeavySPD",
        "EngineBlackHeavyTSPD",
        "",
        "FireSPD",
        "BlackHeavySPD",
        "EngineBlackHeavyTSPD",
    ],
    [
        "",
        "",
        "",
        "GraySmallGND",
        "",
        "",
        "BlackMediumGND",
        "",
        "",
        "BlackHeavyGND",
        "",
        "",
        "FireGND",
        "BlackHeavyGND",
        "",
    ],
];

/// Oil leak (state 1), two slots; in flight, then below 10 m/s (none).
const OIL: [[&str; 2]; 2] = [["OilBlackMediumSPD", "OilBlackMediumTSPD"], ["", ""]];

/// Every effect file these tables (and the wing variants) can use, for preloading.
pub fn effect_files() -> Vec<String> {
    let mut v: Vec<String> = TANK
        .iter()
        .flatten()
        .chain(ENGINE.iter().flatten())
        .chain(OIL.iter().flatten())
        .chain(["FireSPDWing", "FireSPDWingLong", "BlackHeavySPDWing"].iter())
        .filter(|s| !s.is_empty())
        .map(|s| format!("{DIR}{s}.eff"))
        .collect();
    v.sort();
    v.dedup();
    v
}

/// The trails one aircraft shows.
#[derive(Default)]
pub struct Trails {
    /// (system 0 tank / 1 engine / 2 oil, index, slot) → (effect, emitter).
    live: HashMap<(u8, u8, u8), (&'static str, u64)>,
}

impl Trails {
    /// Starts, moves and stops the trails for the aircraft's system states. `hooks` are the
    /// aircraft's hook frames in body coordinates; `wing_tank[t]`: tank `t` is in a wing.
    pub fn update(
        &mut self,
        fx: &mut Effects,
        sim: &Sim,
        hooks: &[(String, Mat4)],
        wing_tank: &[bool; 4],
    ) {
        let ground = usize::from(sim.vel.length() <= 10.0);
        let mut want: Vec<((u8, u8, u8), &'static str, String)> = Vec::new();
        let sys = &sim.systems;
        for (t, &s) in sys.tanks.iter().enumerate() {
            for slot in 0..3 {
                let mut name = TANK[ground][s as usize * 3 + slot];
                // Burning wing tanks use the wing fire.
                if ground == 0 && s > 2 && wing_tank[t] {
                    name = match name {
                        "FireSPD" => "FireSPDWing",
                        "FireSPDLong" => "FireSPDWingLong",
                        "BlackHeavySPD" if s == 5 => "",
                        "BlackHeavySPD" => "BlackHeavySPDWing",
                        n => n,
                    };
                }
                if !name.is_empty() {
                    want.push((
                        (0, t as u8, slot as u8),
                        name,
                        format!("_Tank{}Burn", t + 1),
                    ));
                }
            }
        }
        for (e, &s) in sys.engines.iter().enumerate() {
            for slot in 0..3 {
                let name = ENGINE[ground][s as usize * 3 + slot];
                if !name.is_empty() {
                    want.push((
                        (1, e as u8, slot as u8),
                        name,
                        format!("_Engine{}Smoke", e + 1),
                    ));
                }
            }
        }
        for (e, &s) in sys.oil.iter().enumerate() {
            if s > 0 {
                for slot in 0..2 {
                    let name = OIL[ground][slot];
                    if !name.is_empty() {
                        want.push((
                            (2, e as u8, slot as u8),
                            name,
                            format!("_Engine{}Oil", e + 1),
                        ));
                    }
                }
            }
        }
        // Stop what is no longer wanted (or changed).
        self.live.retain(|k, (name, id)| {
            let keep = want.iter().any(|(wk, wn, _)| wk == k && wn == name);
            if !keep {
                fx.detach(*id);
            }
            keep
        });
        for (k, name, hook) in want {
            let Some((_, m)) = hooks.iter().find(|(h, _)| h.eq_ignore_ascii_case(&hook)) else {
                continue;
            };
            let at = sim.pos + sim.ori * m.w_axis.truncate().as_dvec3();
            let axis = sim.ori * m.x_axis.truncate().as_dvec3();
            match self.live.get(&k) {
                Some(&(_, id)) => fx.follow(id, at, axis),
                None => {
                    if let Some(id) = fx.attach(&format!("{DIR}{name}.eff"), at, axis, 1.0) {
                        self.live.insert(k, (name, id));
                    }
                }
            }
        }
    }

    /// Stops every trail (the aircraft is gone).
    pub fn clear(&mut self, fx: &mut Effects) {
        for (_, (_, id)) in self.live.drain() {
            fx.detach(id);
        }
    }
}
