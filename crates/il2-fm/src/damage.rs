//! Airframe damage on the flight-model side: lifting areas and control sensitivities lost to
//! damaged or missing parts, and structural failure from overload and overspeed
//! (`docs/behavior/damage.md`, "Aircraft").

use crate::aircraft::{Aircraft, ContactKind};
use glam::DVec3;

/// The runtime's part list; a mesh chunk belongs to the first part its name starts with.
pub const PART_NAMES: [&str; 44] = [
    "AroneL", "AroneR", "CF", "Engine1", "Engine2", "Engine3", "Engine4", "GearC2", "FlapR",
    "GearL2", "GearR2", "Keel1", "Keel2", "Nose", "Oil", "Rudder1", "Rudder2", "StabL", "StabR",
    "Tail1", "Tail2", "Tank1", "Tank2", "Tank3", "Tank4", "Turret1B", "Turret2B", "Turret3B",
    "Turret4B", "Turret5B", "Turret6B", "VatorL", "VatorR", "WingLIn", "WingLMid", "WingLOut",
    "WingRIn", "WingRMid", "WingROut", "Flap01", "Flap02", "Flap03", "Flap04", "NullPart",
];

pub const AILERON_L: usize = 0;
pub const AILERON_R: usize = 1;
pub const CF: usize = 2;
pub const GEAR_C: usize = 7;
pub const GEAR_L: usize = 9;
pub const GEAR_R: usize = 10;
pub const KEEL1: usize = 11;
pub const KEEL2: usize = 12;
pub const RUDDER1: usize = 15;
pub const RUDDER2: usize = 16;
pub const STAB_L: usize = 17;
pub const STAB_R: usize = 18;
pub const TAIL1: usize = 19;
pub const TAIL2: usize = 20;
pub const VATOR_L: usize = 31;
pub const VATOR_R: usize = 32;
pub const WING_L_IN: usize = 33;
pub const WING_R_IN: usize = 36;
pub const NULL_PART: usize = 43;

/// The part a mesh chunk name belongs to (`NullPart` when none).
pub fn part(chunk: &str) -> usize {
    PART_NAMES
        .iter()
        .position(|p| chunk.starts_with(p))
        .unwrap_or(NULL_PART)
}

/// Lifting and control areas as damage leaves them (the runtime's `Squares`).
#[derive(Clone, Debug)]
pub struct Squares {
    /// Wing sections, root to tip.
    pub wing_l: [f64; 3],
    pub wing_r: [f64; 3],
    pub stab: f64,
    pub elevators: f64,
    pub keel: f64,
    pub rudders: f64,
    pub ailerons: f64,
    /// Extra drag coefficient from damage, both wing halves.
    pub drag_produced: f64,
    /// Control sensitivity factors: roll, pitch, yaw.
    pub sens: [f64; 3],
}

impl Squares {
    pub fn new(ac: &Aircraft) -> Self {
        Squares {
            wing_l: ac.wing_sections,
            wing_r: ac.wing_sections,
            stab: ac.stab_area,
            elevators: ac.elevator_area,
            keel: ac.keel_area,
            rudders: ac.rudder_area,
            ailerons: 1.0,
            drag_produced: 0.0,
            sens: [1.0; 3],
        }
    }
}

/// Load limits and the structure's fatigue.
#[derive(Clone, Debug)]
pub struct Structure {
    /// `G_CLASS / 1.5` times the reference weight, N per (m/s²): the limit load at any mass.
    reference_force: f64,
    pub limit_load: f64,
    pub ultimate_load: f64,
    pub negative_limit: f64,
    pub safety_factor: f64,
    /// Indicated airspeed above which flutter can damage parts, m/s.
    pub vmax_allowed: f64,
    max_speed: f64,
    damaged_parts: [u32; 7],
    cut_part: Option<usize>,
    time_counter: f64,
    cycle_counter: u32,
    max_g_cycle: f64,
    /// Parts still attached (bit per part index).
    ops: u64,
    /// Twin-fin layouts (`Scheme2a`, `Scheme5`, `Scheme7`) keep some yaw control with one fin gone.
    pub twin_fin: bool,
    pub flaps_failed: bool,
    pub ailerons_failed: bool,
    /// The fuselage broke: the aircraft is destroyed.
    pub destroyed: bool,
    /// Chunks the structure hit this step (`msgCollision`), for the airframe to damage.
    pub collisions: Vec<String>,
    rng: u64,
}

impl Structure {
    pub fn new(ac: &Aircraft) -> Self {
        let limit = ac.g_class / 1.5;
        let reference_mass = ac.mass_empty + ac.fuel_max + ac.nitro_max + ac.reference_weight;
        Structure {
            reference_force: limit * reference_mass,
            limit_load: limit,
            ultimate_load: limit * 1.5,
            negative_limit: -0.5 * limit,
            safety_factor: 1.5,
            vmax_allowed: ac.vmax_allowed,
            max_speed: ac.vmax_allowed,
            damaged_parts: [0; 7],
            cut_part: None,
            time_counter: 0.0,
            cycle_counter: 0,
            max_g_cycle: 1.0,
            ops: u64::MAX,
            twin_fin: false,
            flaps_failed: false,
            ailerons_failed: false,
            destroyed: false,
            collisions: Vec::new(),
            rng: 0x9E37_79B9_7F4A_7C15,
        }
    }

    pub fn intact(&self, part: usize) -> bool {
        self.ops & (1 << part) != 0
    }

    fn op(&self, part: usize) -> f64 {
        if self.intact(part) {
            1.0
        } else {
            0.0
        }
    }

    /// Uniform in [0, 1).
    pub fn uniform(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }

    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.uniform()
    }

    /// Integer in [a, b], both included.
    fn int(&mut self, a: i32, b: i32) -> i32 {
        a + (self.uniform() * (b - a + 1) as f64) as i32
    }

    fn set_dmg_load_limit(&mut self, by: f64, slot: usize) {
        self.limit_load -= by;
        self.damaged_parts[slot] += 1;
        for (i, &n) in self.damaged_parts.iter().enumerate() {
            if n > 0 {
                self.cut_part = Some(i);
            }
        }
    }

    fn collide(&mut self, chunk: &str) {
        self.collisions.push(format!("{chunk}_D0"));
    }

    /// A random wing or tail failure (`cutWing`), or the weakened part after damage.
    fn break_wing(&mut self) {
        let chunk = match self.cut_part {
            Some(0) => "WingLOut",
            Some(1) => "WingLMid",
            Some(2) => "WingLIn",
            Some(3) => "WingRIn",
            Some(4) => "WingRMid",
            Some(5) => "WingROut",
            Some(_) => "Tail1",
            None => match self.int(0, 8) {
                0 => "Tail1",
                1 | 2 => "WingRMid",
                3 | 4 => "WingLMid",
                5 | 6 => "WingLIn",
                _ => "WingRIn",
            },
        };
        self.collide(chunk);
    }

    /// Overspeed flutter tears off a control surface or wing section.
    fn flutter_damage(&mut self) {
        let chunk = match self.int(0, 29) {
            0..=3 | 20 => "AroneL",
            4..=7 | 21 => "AroneR",
            8..=10 | 22 => "VatorL",
            11..=13 | 23 => "VatorR",
            14 => "WingLOut",
            15 => "WingROut",
            16 => "WingLMid",
            17 => "WingRMid",
            18 => "WingLIn",
            19 => "WingRIn",
            24..=26 => "Rudder1",
            _ => "Rudder2",
        };
        self.collide(chunk);
    }
}

/// What the structure check needs from the flight-model step.
pub struct StructureInputs {
    pub dt: f64,
    pub mass: f64,
    /// Aerodynamic and thrust force along body z over weight.
    pub overload: f64,
    /// The aerodynamic force exceeded 50 g before limiting.
    pub excessive_force: bool,
    pub ias: f64,
    pub on_ground: bool,
    pub flaps_lever: f64,
    pub flaps: f64,
}

impl crate::Sim {
    /// A hit that takes a part one damage level further (the runtime's `FlightModelMain.hit`).
    pub fn hit_part(&mut self, part: usize) {
        let (sq, st) = (&mut self.sq, &mut self.structure);
        match part {
            AILERON_L => {
                sq.wing_l[2] *= 0.95;
                sq.sens[0] *= 0.68;
            }
            AILERON_R => {
                sq.wing_r[2] *= 0.95;
                sq.sens[0] *= 0.68;
            }
            KEEL1 | KEEL2 => {
                sq.keel *= 0.68;
                sq.drag_produced += 0.12;
            }
            RUDDER1 | RUDDER2 => {
                sq.rudders *= 0.5;
                sq.sens[2] *= 0.68;
            }
            STAB_L | STAB_R => {
                sq.stab *= 0.68;
                sq.drag_produced += 0.12;
            }
            TAIL1 | TAIL2 => st.set_dmg_load_limit(0.5, 6),
            VATOR_L | VATOR_R => {
                sq.elevators *= 0.68;
                sq.sens[1] *= 0.68;
            }
            33..=38 => {
                let i = part - WING_L_IN;
                let (load, slot) = [(0.6, 2), (0.7, 1), (0.8, 0), (0.6, 3), (0.7, 4), (0.8, 5)][i];
                st.set_dmg_load_limit(load, slot);
                // The runtime subtracts 0.8 m² per hit; OpenIL2 stops at zero.
                let w = if i < 3 {
                    &mut sq.wing_l[i]
                } else {
                    &mut sq.wing_r[i - 3]
                };
                *w = (*w - 0.8).max(0.0);
                sq.drag_produced += 0.12;
            }
            _ => {}
        }
    }

    /// A part torn off (the runtime's `FlightModelMain.cut`); parts it carries go with it.
    pub fn cut_part(&mut self, part: usize) {
        if part >= NULL_PART || !self.structure.intact(part) {
            return;
        }
        self.structure.ops &= !(1 << part);
        match part {
            CF | TAIL1 | TAIL2 => {
                if part == CF {
                    self.structure.destroyed = true;
                    for e in &mut self.engines {
                        e.readyness = 0.0;
                        e.stage = 7;
                    }
                    self.structure.ops &= !(1 << TAIL1 | 1 << TAIL2);
                }
                // The tail is gone: the aircraft tumbles.
                self.ac.gcenter = 0.0;
                let (a, b) = (
                    self.structure.range(-0.1, 0.1),
                    self.structure.range(-0.1, 0.1),
                );
                self.w.y += a;
                self.w.z += b;
                self.ac.j0.y *= 0.5;
                for p in [STAB_L, STAB_R, KEEL1, KEEL2] {
                    self.cut_part(p);
                }
            }
            3..=6 => {
                if let Some(e) = self.engines.get_mut(part - 3) {
                    e.stage = 7;
                }
            }
            GEAR_C | GEAR_L | GEAR_R => {
                // The wheel no longer rolls; its contact point scrapes like the airframe.
                let wanted = match part {
                    GEAR_L => 0,
                    GEAR_R => 1,
                    _ => 2,
                };
                let mut n = 0;
                for g in &mut self.ac.gear {
                    if matches!(g.kind, ContactKind::Wheel | ContactKind::Castor) {
                        if n == wanted {
                            g.kind = ContactKind::Body;
                            break;
                        }
                        n += 1;
                    }
                }
            }
            KEEL1 | KEEL2 => {
                let other = if part == KEEL1 { KEEL2 } else { KEEL1 };
                let rudder = if part == KEEL1 { RUDDER1 } else { RUDDER2 };
                if self.structure.twin_fin {
                    self.sq.keel *= 0.25 * self.structure.op(other);
                } else {
                    self.sq.keel = 0.0;
                }
                self.structure.ops &= !(1 << rudder);
                self.lose_rudder();
            }
            RUDDER1 | RUDDER2 => self.lose_rudder(),
            STAB_L | STAB_R => {
                let left = part == STAB_L;
                let other = if left { STAB_R } else { STAB_L };
                let other_op = self.structure.op(other);
                self.sq.stab *= 0.5 * other_op + 0.1;
                self.sq.drag_produced += 0.12;
                let (weak, strong) = if left {
                    (&mut self.sq.wing_r, &mut self.sq.wing_l)
                } else {
                    (&mut self.sq.wing_l, &mut self.sq.wing_r)
                };
                weak[1] *= 0.9;
                strong[1] *= 1.1;
                weak[2] *= 0.9;
                strong[2] *= 1.1;
                if other_op == 0.0 {
                    let d = if left { -0.25 } else { 0.25 };
                    self.trim_target[0] = (self.trim_target[0] + d).clamp(-0.5, 0.5);
                    let (a, b) = if self.structure.uniform() < 0.5 {
                        (0.95, 0.75)
                    } else {
                        (0.75, 0.95)
                    };
                    self.sq.wing_l.iter_mut().for_each(|w| *w *= a);
                    self.sq.wing_r.iter_mut().for_each(|w| *w *= b);
                }
                let vator = if left { VATOR_L } else { VATOR_R };
                self.structure.ops &= !(1 << vator);
                self.lose_elevator(vator);
            }
            VATOR_L | VATOR_R => self.lose_elevator(part),
            33..=38 => {
                let left = part < WING_R_IN;
                let section = (part - WING_L_IN) % 3;
                let gear = if left { GEAR_L } else { GEAR_R };
                let aileron = if left { AILERON_L } else { AILERON_R };
                if section == 0 {
                    let w = if left {
                        &mut self.sq.wing_l
                    } else {
                        &mut self.sq.wing_r
                    };
                    w[0] *= 0.25;
                    self.cut_part(gear);
                }
                for s in section.max(1)..3 {
                    self.structure.ops &= !(1 << (part - section + s));
                }
                // As the runtime does: without the tip, the whole half loses its lift.
                let w = if left {
                    &mut self.sq.wing_l
                } else {
                    &mut self.sq.wing_r
                };
                *w = [0.0; 3];
                self.structure.ailerons_failed = true;
                self.structure.ops &= !(1 << aileron);
                self.lose_aileron(aileron);
            }
            AILERON_L | AILERON_R => self.lose_aileron(part),
            _ => {}
        }
    }

    fn lose_rudder(&mut self) {
        if self.structure.twin_fin {
            self.sq.rudders *= 0.5;
            self.sq.sens[2] *= 0.25;
        } else {
            self.sq.rudders = 0.0;
            self.sq.sens[2] = 0.0;
        }
    }

    fn lose_elevator(&mut self, vator: usize) {
        let other = if vator == VATOR_L { VATOR_R } else { VATOR_L };
        let k = 0.5 * self.structure.op(other);
        self.sq.elevators *= k;
        self.sq.sens[1] *= k;
    }

    fn lose_aileron(&mut self, aileron: usize) {
        let other = if aileron == AILERON_L {
            AILERON_R
        } else {
            AILERON_L
        };
        self.sq.ailerons *= 0.5;
        self.sq.sens[0] *= 0.5 * self.structure.op(other);
    }

    /// Load limits, fatigue and overspeed (the runtime's checks with G limits and flutter on).
    pub(crate) fn check_structure(&mut self, c: &StructureInputs) {
        let st = &mut self.structure;
        // The limit load follows the mass.
        st.limit_load = st.reference_force / c.mass.max(1.0);
        st.negative_limit = -0.5 * st.limit_load;
        st.ultimate_load = st.limit_load * st.safety_factor;
        if c.excessive_force {
            st.collide("CF");
            return;
        }
        let g = c.overload;
        let over = st.ultimate_load + st.range(2.0, 4.0);
        let under = st.negative_limit - st.range(1.0, 2.5);
        if (g > over || g < under) && !c.on_ground && st.int(0, 100) > 98 {
            st.break_wing();
        }
        if g <= st.limit_load && g >= st.negative_limit {
            st.time_counter = 0.0;
            st.max_g_cycle = 1.0;
        } else {
            st.max_g_cycle = st.max_g_cycle.max(g.abs());
            st.time_counter += c.dt;
            if st.time_counter > 0.85 {
                st.cycle_counter += 1;
                if st.cycle_counter > 1 {
                    let x = if g > 1.0 {
                        (st.max_g_cycle - st.limit_load) / st.limit_load
                    } else {
                        (st.max_g_cycle + st.negative_limit) / st.negative_limit
                    };
                    st.safety_factor -= x * x;
                    st.vmax_allowed = st.max_speed * (st.safety_factor * 0.3 + 0.55);
                    let r = st.uniform();
                    // Rare lasting damage from the bending; the gear and drag items of the
                    // runtime's list are left out.
                    if r < 0.0035 {
                        st.safety_factor -= 125.0 * r;
                    } else if r < 0.04 {
                        self.sq.sens[2] *= 0.68;
                    } else if r < 0.05 {
                        self.sq.sens[1] *= 0.68;
                    } else if r < 0.06 {
                        self.sq.sens[0] *= 0.68;
                    } else if (0.061..0.065).contains(&r) {
                        self.structure.flaps_failed = true;
                    } else if r >= 0.6 {
                        let w = if (c.mass as i64) % 2 == 0 {
                            &mut self.sq.wing_r
                        } else {
                            &mut self.sq.wing_l
                        };
                        w[2] *= 0.95 - 0.2 * r;
                    }
                }
                let st = &mut self.structure;
                st.time_counter = 0.0;
                st.max_g_cycle = 1.0;
            }
        }
        let st = &mut self.structure;
        // Flaps fail when lowered too fast.
        if c.ias > 81.0
            && c.flaps_lever > 0.21
            && (c.ias - 81.0) * c.flaps > 8.0
            && !st.flaps_failed
        {
            st.flaps_failed = true;
        }
        if c.ias > st.vmax_allowed
            && st.range(0.0, 16.0) < c.ias - st.vmax_allowed
            && st.int(0, 99) < 2
        {
            st.flutter_damage();
        }
        if c.ias > 310.0 {
            st.collide("CF");
        }
    }

    /// The tail is gone: the airframe pitches and yaws uncontrollably.
    pub(crate) fn tail_moments(&self, vflow_x: f64, prop_sign: f64) -> DVec3 {
        if self.structure.intact(TAIL1) {
            DVec3::ZERO
        } else {
            DVec3::new(
                0.0,
                8.0 * self.ac.wing_area * vflow_x,
                200.0 * self.ac.wing_area * prop_sign,
            )
        }
    }
}
