//! Damage to an aircraft's systems: fuel tanks, engines, oil, crew, controls and hydraulics
//! (the runtime's `AircraftState`; `docs/behavior/damage.md`, "Systems").

use crate::sim::Sim;

/// Seconds between state updates (the runtime: every 44 ticks of 0.03 s).
pub const UPDATE: f64 = 1.32;

/// Something that happened to a system, for the airframe, effects and messages.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Tank `n` exploded: its chunk takes a hard collision (and the fuselage, single-engine).
    TankExplodes(usize),
    /// Engine `n` exploded: its chunk takes a hard collision (piston engines also a blast).
    EngineExplodes(usize),
    /// A crew member died (index).
    CrewDead(usize),
    /// The aircraft is beyond saving (fire, dead pilot).
    Mortal,
    /// The crew bails out (AI).
    BailOut,
    /// A message for the pilot (`FailedTank`, `FailedEngine`, …).
    Message(&'static str),
}

#[derive(Clone, Debug)]
pub struct Systems {
    /// Fuel tank states 0..6: 1–2 leaking, 3 heavy leak, 4 smoking, 5 burning, 6 blazing.
    pub tanks: [u8; 4],
    /// Engine states 0..4: 1 hit, 2 smoking, 3 heavy smoke, 4 burning.
    pub engines: Vec<u8>,
    /// Oil system hit (0/1) per engine.
    pub oil: Vec<u8>,
    /// Crew wounds 0..100 per seat (above 95: dead, set to 100).
    pub crew: Vec<u8>,
    /// Bleeding: seconds between further 1-point wounds per seat (0: none), and time left.
    bleed_every: Vec<f64>,
    bleed_left: Vec<f64>,
    pub arms_wounded: bool,
    pub legs_wounded: bool,
    /// Cockpit damage bits (`setCockpitState`: 1 instruments, 2 armour glass, …, 128 all
    /// engines out).
    pub cockpit: u32,
    /// Control runs cut: aileron, elevator, rudder, stabilizer trim / autopilot.
    pub controls_cut: [bool; 4],
    /// Hydraulics gone (gear and flaps that need them stay put), flaps' drive gone, gear jammed.
    pub hydraulics_dead: bool,
    pub flaps_dead: bool,
    pub gear_jammed: bool,
    /// The part each tank and engine sits in (`_Tank<n>Burn`/`_Engine<n>Smoke` hook chunks):
    /// tanks and engines in parts already shot away can't change state.
    pub tank_parts: [Option<usize>; 4],
    pub engine_parts: Vec<Option<usize>>,
    /// Single-engine, engine in the nose (`Scheme1`): fires reach the pilot.
    pub scheme1: bool,
    pub mortal: bool,
    pub bailing_out: bool,
    pub events: Vec<Event>,
    timer: f64,
    rng: u64,
}

impl Systems {
    /// For an aircraft (engine count, crew, where its tanks and engines sit).
    pub fn for_aircraft(ac: &crate::Aircraft) -> Self {
        let mut s = Systems::new(ac.engines.len(), ac.crew);
        s.tank_parts = ac.tank_parts;
        for (i, p) in ac
            .engine_parts
            .iter()
            .enumerate()
            .take(s.engine_parts.len())
        {
            s.engine_parts[i] = *p;
        }
        s.scheme1 = ac.scheme1;
        s
    }

    pub fn new(engines: usize, crew: usize) -> Self {
        Systems {
            tanks: [0; 4],
            engines: vec![0; engines],
            oil: vec![0; engines],
            crew: vec![0; crew.max(1)],
            bleed_every: vec![0.0; crew.max(1)],
            bleed_left: vec![0.0; crew.max(1)],
            arms_wounded: false,
            legs_wounded: false,
            cockpit: 0,
            controls_cut: [false; 4],
            hydraulics_dead: false,
            flaps_dead: false,
            gear_jammed: false,
            tank_parts: [None; 4],
            engine_parts: vec![None; engines],
            scheme1: false,
            mortal: false,
            bailing_out: false,
            events: Vec::new(),
            timer: 0.0,
            rng: 0x9E37_79B9_7F4A_7C15,
        }
    }

    fn uniform(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn seed(&mut self, seed: u64) {
        self.rng = seed | 1;
    }
}

impl Sim {
    fn part_there(&self, part: Option<usize>) -> bool {
        part.is_none_or(|p| self.structure.intact(p))
    }

    /// Oil system `e` hit (`hitOil`): a leak, the engine wears down from then on.
    pub fn hit_oil(&mut self, e: usize) {
        let Some(&s) = self.systems.oil.get(e) else {
            return;
        };
        if s == 0 && self.part_there(self.systems.engine_parts.get(e).copied().flatten()) {
            self.systems.oil[e] = 1;
            self.systems.events.push(Event::Message("FailedOil"));
            if self.systems.oil.iter().all(|&o| o > 0) {
                self.systems.cockpit |= 128;
            }
        }
    }

    /// Tank `t` hit `n` levels (`hitTank`), at most to 6.
    pub fn hit_tank(&mut self, t: usize, n: u8) {
        let Some(&s) = self.systems.tanks.get(t) else {
            return;
        };
        if s != 6 {
            self.set_tank(t, (s + n).min(6));
        }
    }

    /// `setTankState`: each level gained may make the crew give up.
    pub fn set_tank(&mut self, t: usize, to: u8) {
        if t >= 4 || !self.part_there(self.systems.tank_parts[t]) {
            return;
        }
        let from = self.systems.tanks[t];
        if from == 0 && (to == 1 || to == 2) {
            self.systems.events.push(Event::Message("FailedTank"));
        }
        if from < 5 && to >= 5 {
            self.systems.events.push(Event::Message("FailedTankOnFire"));
            self.systems.mortal = true;
            self.systems.events.push(Event::Mortal);
        }
        self.systems.tanks[t] = to;
        for _ in from..to {
            if self.systems.tanks[t] == 6 && (self.systems.uniform() < 0.75 || self.systems.scheme1)
            {
                self.systems.mortal = true;
                if self.systems.uniform() < 0.75 {
                    self.bail_out();
                }
            }
        }
    }

    /// Engine `e` hit `n` levels (`hitEngine`), at most to 4 (burning).
    pub fn hit_engine(&mut self, e: usize, n: u8) {
        let Some(&s) = self.systems.engines.get(e) else {
            return;
        };
        if s != 4 {
            self.set_engine(e, (s + n).min(4));
        }
    }

    /// `setEngineState`.
    pub fn set_engine(&mut self, e: usize, to: u8) {
        if e >= self.systems.engines.len()
            || !self.part_there(self.systems.engine_parts[e])
            || self.systems.engines[e] == to
        {
            return;
        }
        let from = self.systems.engines[e];
        if from < 4 && to >= 4 {
            self.systems
                .events
                .push(Event::Message("FailedEngineOnFire"));
            if self.on_ground {
                self.systems.mortal = true;
            }
        }
        self.systems.engines[e] = to;
        if to > from && to == 4 && self.systems.scheme1 {
            self.systems.mortal = true;
            if self.systems.uniform() < 0.5 {
                self.bail_out();
            }
        }
        if self.systems.engines.iter().all(|&s| s > 2) {
            self.systems.cockpit |= 128;
        }
    }

    /// `setEngineSpecificDamage(e, kind)`: 0 supercharger, 1 throttle linkage, 3 propeller pitch
    /// gear, 4 pitch-change speed, 5 fire extinguisher used, 6 propeller lever, 7 mixture lever.
    pub fn engine_specific(&mut self, e: usize, kind: i32) {
        let throttle = self.ct.throttle;
        let Some(st) = self.engines.get_mut(e) else {
            return;
        };
        match kind {
            0 => st.supercharger_dead = true,
            1 => st.cut_throttle(throttle),
            3 => st.pitch_frozen = true,
            4 => st.pitch_slow = true,
            6 => st.prop_lever_cut = true,
            7 => st.mix_lever_cut = true,
            _ => {}
        }
    }

    /// `setControlsDamage`: 0 aileron, 1 elevator, 2 rudder, 3 stabilizer trim and autopilot.
    pub fn cut_controls(&mut self, which: usize) {
        if let Some(c) = self.systems.controls_cut.get_mut(which) {
            if !*c {
                *c = true;
                self.systems.events.push(Event::Message(match which {
                    0 => "FailedAroneControls",
                    1 => "FailedVatorControls",
                    2 => "FailedRudderControls",
                    _ => "FailedTrimControls",
                }));
            }
        }
    }

    /// `setInternalDamage`: 0 hydraulics, 1 flap drive, 2 nitrous oxide lost, 3 gear jammed and
    /// hydraulics, 5 second engine's propeller reduction gear.
    pub fn internal_damage(&mut self, kind: i32) {
        match kind {
            0 => self.systems.hydraulics_dead = true,
            1 => self.systems.flaps_dead = true,
            2 => self.nitro = 0.0,
            3 => {
                self.systems.hydraulics_dead = true;
                self.systems.gear_jammed = true;
            }
            _ => {}
        }
    }

    /// Crew member `i` hit for `n` points (`hitPilot`); above 95 is dead.
    pub fn hit_crew(&mut self, i: usize, n: i32) {
        let Some(&s) = self.systems.crew.get(i) else {
            return;
        };
        let mut to = (s as i32 + n).clamp(0, 100);
        if to > 95 {
            to = 100;
        }
        if to as u8 > s {
            self.systems.crew[i] = to as u8;
            if to == 100 {
                self.systems.events.push(Event::CrewDead(i));
                if i == 0 {
                    self.systems.mortal = true;
                    self.systems.events.push(Event::Mortal);
                }
            }
        }
    }

    /// A seat bleeding `n` more (`setBleedingPilot`): a further point every 120/n s.
    pub fn bleed(&mut self, i: usize, n: i32) {
        if n <= 0 || i >= self.systems.crew.len() {
            return;
        }
        let every = &mut self.systems.bleed_every[i];
        if *every == 0.0 {
            *every = 120.0 / n as f64;
        } else {
            let rate = 120.0 / *every + n as f64;
            *every = (120.0 / rate).max(0.1);
        }
        self.systems.bleed_left[i] = self.systems.bleed_every[i];
    }

    /// `setPilotWound`: 1 arms (roll and pitch control × 0.6), 2 legs (yaw × 0.2).
    pub fn wound(&mut self, kind: i32) {
        match kind {
            1 if !self.systems.arms_wounded => {
                self.systems.arms_wounded = true;
                self.sq.sens[0] *= 0.6;
                self.sq.sens[1] *= 0.6;
            }
            2 if !self.systems.legs_wounded => {
                self.systems.legs_wounded = true;
                self.sq.sens[2] *= 0.2;
            }
            _ => {}
        }
    }

    fn bail_out(&mut self) {
        if !self.systems.bailing_out {
            self.systems.bailing_out = true;
            self.systems.mortal = true;
            self.systems.events.push(Event::BailOut);
        }
    }

    fn explode_tank(&mut self, t: usize) {
        if self.part_there(self.systems.tank_parts[t]) {
            self.systems.events.push(Event::TankExplodes(t));
            self.systems
                .events
                .push(Event::Message("FailedTankExplodes"));
        }
    }

    /// The periodic part (`AircraftState.update`, every 1.32 s): leaks drain fuel, fires grow,
    /// go out or explode, damaged engines and oil systems wear the engines down, wounds bleed.
    pub fn update_systems(&mut self, dt: f64) {
        for i in 0..self.systems.crew.len() {
            if self.systems.bleed_every[i] > 0.0 && self.systems.crew[i] < 100 {
                self.systems.bleed_left[i] -= dt;
                while self.systems.bleed_left[i] <= 0.0 && self.systems.crew[i] < 100 {
                    self.hit_crew(i, 1);
                    self.systems.bleed_left[i] += self.systems.bleed_every[i];
                }
            }
        }
        self.systems.timer += dt;
        if self.systems.timer < UPDATE {
            return;
        }
        let dt = self.systems.timer;
        self.systems.timer = 0.0;
        let speed = self.vel.length();
        let cvt = |x: f64, x0: f64, x1: f64, y0: f64, y1: f64| {
            y0 + (y1 - y0) * ((x - x0) / (x1 - x0)).clamp(0.0, 1.0)
        };
        // Leaks: fuel lost with the square of each tank's state.
        let leak: f64 = self
            .systems
            .tanks
            .iter()
            .map(|&s| (s as f64) * (s as f64))
            .sum();
        self.fuel = (self.fuel - leak * 0.12 * dt).max(0.0);
        for t in 0..4 {
            match self.systems.tanks[t] {
                1 | 2 => {
                    if self.systems.tanks[t] == 1 && self.systems.uniform() < 0.0125 {
                        // The self-sealing lining closes the hole.
                        self.systems.tanks[t] -= 1;
                    } else if self.fuel <= 0.0 {
                        self.systems.tanks[t] -= 1;
                    }
                }
                4 => {
                    if self.systems.uniform() < 0.06 {
                        self.hit_tank(t, 0);
                    }
                    if self.systems.uniform() < 0.00333 {
                        self.hit_tank(t, 1);
                    }
                }
                5 => {
                    if speed > 90.0 && self.systems.uniform() < cvt(speed, 90.0, 200.0, 0.0, 0.2) {
                        self.systems.tanks[t] -= 1;
                    }
                    if self.systems.uniform() < 0.0048 {
                        self.hit_tank(t, 1);
                    }
                    if self.systems.uniform() < 0.002 {
                        self.explode_tank(t);
                    }
                    if self.systems.scheme1 && self.systems.crew[0] < 96 {
                        self.hit_crew(0, 5);
                    }
                }
                6 => {
                    if speed > 90.0 && self.systems.uniform() < cvt(speed, 90.0, 200.0, 0.0, 0.1) {
                        self.systems.tanks[t] -= 1;
                    }
                    if self.systems.uniform() < 0.0075 {
                        self.explode_tank(t);
                    }
                    if self.systems.scheme1 && self.systems.crew[0] < 96 {
                        self.hit_crew(0, 7);
                    }
                }
                _ => {}
            }
        }
        for e in 0..self.systems.engines.len().min(self.engines.len()) {
            let state = self.systems.engines[e];
            let stage = self.engines[e].stage;
            if state > 1 {
                let k = if state > 3 { 15.0 } else { 1.0 };
                let r = self.engines[e].readyness - state as f64 * k * 2.5e-4 * dt;
                self.engines[e].set_readyness(r);
                if self.engines[e].readyness < 0.2 && self.engines[e].readyness != 0.0 {
                    self.engines[e].die();
                }
            }
            match state {
                3 => {
                    if stage == 6
                        && self.systems.uniform() < cvt(self.throttle_load(e), 1.0, 3.0, 0.015, 0.0)
                    {
                        self.hit_engine(e, 1);
                    } else if stage > 0 && stage < 6 {
                        if self.systems.uniform() < 0.2 {
                            self.engines[e].die();
                        }
                    } else if stage != 6 && self.systems.uniform() < 0.0075 {
                        self.systems.engines[e] -= 1;
                    }
                }
                4 => {
                    if speed > 80.0 && self.systems.uniform() < cvt(speed, 80.0, 200.0, 0.0, 0.3) {
                        self.systems.engines[e] -= 1;
                    }
                    if self.systems.scheme1 && self.systems.crew[0] < 96 {
                        self.hit_crew(0, 4);
                    }
                    let piston = self.ac.engines.get(e).is_some_and(|p| p.piston);
                    if !piston {
                        if self.systems.scheme1 && self.systems.uniform() < 0.05 {
                            self.systems.events.push(Event::EngineExplodes(e));
                        }
                    } else if stage == 6 {
                        let p = cvt(self.throttle_load(e), 1.0, 5.0, 0.01, 0.0);
                        if self.systems.uniform() < p {
                            self.engines[e].die();
                            self.systems.events.push(Event::EngineExplodes(e));
                        }
                    } else if stage > 0 && stage < 6 {
                        if self.systems.uniform() < 0.3 {
                            self.engines[e].die();
                        }
                    } else if self.systems.uniform() < 0.15 {
                        self.systems.engines[e] -= 1;
                    }
                }
                _ => {}
            }
            if self.systems.oil[e] > 0 {
                let r = self.engines[e].readyness - 0.001875 * dt;
                self.engines[e].set_readyness(r);
            }
        }
    }

    /// The runtime's engine load: power output relative to nominal (1 at full throttle,
    /// above with boost).
    fn throttle_load(&self, _e: usize) -> f64 {
        self.ct.throttle.max(0.0) * 1.0
    }
}
