//! Animation importer: `docs/behavior/animation.md`, "Importer".
//!
//! Evaluates an aircraft's part-animation methods from the user's installed runtime classes in a
//! restricted interpreter and samples them into tables. Anything outside the allowed instruction
//! set aborts that method and the generic default is used instead.

use crate::classfile::{ClassFile, Const, Method};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Channel {
    Rudder,
    Elevator,
    Aileron,
    Flaps,
    Gear,
}

impl Channel {
    pub const ALL: [Channel; 5] = [Channel::Rudder, Channel::Elevator, Channel::Aileron, Channel::Flaps, Channel::Gear];

    fn range(self) -> (f32, f32) {
        match self {
            Channel::Rudder | Channel::Elevator | Channel::Aileron => (-1.0, 1.0),
            Channel::Flaps | Channel::Gear => (0.0, 1.0),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NodeState {
    /// `setAngles` a, b, c in degrees.
    pub angles: [f32; 3],
    pub offset: [f32; 3],
    pub visible: Option<bool>,
}

/// Sampled node states of one input channel.
#[derive(Clone, Debug)]
pub struct Track {
    pub channel: Channel,
    pub inputs: Vec<f32>,
    /// node name -> state per sample
    pub nodes: BTreeMap<String, Vec<NodeState>>,
    /// True when taken from the aircraft's classes, false for the generic default.
    pub imported: bool,
}

impl Track {
    /// Node states at input `x`, interpolated between samples.
    pub fn eval(&self, x: f32) -> impl Iterator<Item = (&str, NodeState)> {
        let n = self.inputs.len();
        let (lo, hi) = (self.inputs[0], self.inputs[n - 1]);
        let f = ((x.clamp(lo, hi) - lo) / (hi - lo) * (n - 1) as f32).clamp(0.0, (n - 1) as f32);
        let i = (f.floor() as usize).min(n - 2);
        let t = f - i as f32;
        self.nodes.iter().map(move |(name, s)| {
            let (a, b) = (s[i], s[i + 1]);
            let lerp = |u: [f32; 3], v: [f32; 3]| std::array::from_fn(|k| u[k] + (v[k] - u[k]) * t);
            (name.as_str(), NodeState { angles: lerp(a.angles, b.angles), offset: lerp(a.offset, b.offset), visible: if t < 0.5 { a.visible } else { b.visible } })
        })
    }
}

const SAMPLES: usize = 41;

/// The generic default for a channel.
pub fn default_track(ch: Channel) -> Track {
    let (lo, hi) = ch.range();
    let inputs: Vec<f32> = (0..SAMPLES).map(|i| lo + (hi - lo) * i as f32 / (SAMPLES - 1) as f32).collect();
    let (names, k): (&[&str], f32) = match ch {
        Channel::Rudder => (&["Rudder1_D0"], -30.0),
        Channel::Elevator => (&["VatorL_D0", "VatorR_D0"], -30.0),
        Channel::Aileron => (&["AroneL_D0", "AroneR_D0"], -30.0),
        Channel::Flaps => (&["Flap01_D0", "Flap02_D0", "Flap03_D0", "Flap04_D0"], -50.0),
        Channel::Gear => (&[], 0.0),
    };
    let nodes = names
        .iter()
        .map(|n| (n.to_string(), inputs.iter().map(|&x| NodeState { angles: [0.0, k * x, 0.0], ..Default::default() }).collect()))
        .collect();
    Track { channel: ch, inputs, nodes, imported: false }
}

#[derive(Clone, Debug, PartialEq)]
enum V {
    I(i32),
    L(i64),
    F(f32),
    D(f64),
    This,
    Hier,
    Str(String),
    Arr(usize),
    Null,
    /// Second slot of a long/double local.
    Top,
}

struct Abort;

/// Interpreter state shared across the calls of one sampling run.
struct Machine<'a> {
    chain: &'a [ClassFile],
    arrays: Vec<Vec<f32>>,
    out: BTreeMap<String, NodeState>,
    budget: usize,
}

fn desc_args(desc: &str) -> Vec<char> {
    let inner = &desc[1..desc.find(')').unwrap_or(1)];
    let mut out = Vec::new();
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        match c {
            'L' => {
                for d in it.by_ref() {
                    if d == ';' {
                        break;
                    }
                }
                out.push('L');
            }
            '[' => {
                let mut d = it.next().unwrap_or('I');
                while d == '[' {
                    d = it.next().unwrap_or('I');
                }
                if d == 'L' {
                    for e in it.by_ref() {
                        if e == ';' {
                            break;
                        }
                    }
                }
                out.push('L');
            }
            c => out.push(c),
        }
    }
    out
}

impl<'a> Machine<'a> {
    /// Finds a method along the chain, starting at class `from` (or the most derived when `None`).
    fn resolve(&self, from: Option<&str>, name: &str, desc: &str) -> Option<(&'a ClassFile, &'a Method)> {
        let start = from.map_or(0, |c| self.chain.iter().position(|cf| cf.this_class == c).unwrap_or(self.chain.len()));
        self.chain[start.min(self.chain.len())..].iter().find_map(|cf| cf.method(name, desc).map(|m| (cf, m)))
    }

    fn call(&mut self, cf: &'a ClassFile, m: &'a Method, args: Vec<V>, depth: usize) -> Result<Option<V>, Abort> {
        if depth > 8 || m.code.is_empty() {
            return Err(Abort);
        }
        let mut locals = vec![V::Null; m.max_locals.max(args.len() + 4)];
        let mut slot = 0;
        for a in args {
            let wide = matches!(a, V::L(_) | V::D(_));
            if slot >= locals.len() {
                return Err(Abort);
            }
            locals[slot] = a;
            slot += 1;
            if wide {
                locals[slot] = V::Top;
                slot += 1;
            }
        }
        let code = &m.code;
        let mut st: Vec<V> = Vec::new();
        let mut pc = 0usize;
        let u8_at = |p: usize| code.get(p).copied().ok_or(Abort);
        let u16_at = |p: usize| -> Result<u16, Abort> { Ok(u16::from_be_bytes([u8_at(p)?, u8_at(p + 1)?])) };
        let i16_at = |p: usize| -> Result<i16, Abort> { Ok(u16_at(p)? as i16) };
        macro_rules! pop {
            () => {
                st.pop().ok_or(Abort)?
            };
        }
        macro_rules! popf {
            () => {
                match pop!() {
                    V::F(f) => f,
                    V::I(i) => i as f32,
                    _ => return Err(Abort),
                }
            };
        }
        macro_rules! popd {
            () => {
                match pop!() {
                    V::D(d) => d,
                    _ => return Err(Abort),
                }
            };
        }
        macro_rules! popi {
            () => {
                match pop!() {
                    V::I(i) => i,
                    _ => return Err(Abort),
                }
            };
        }
        loop {
            self.budget = self.budget.checked_sub(1).ok_or(Abort)?;
            let op = u8_at(pc)?;
            let mut next = pc + 1;
            match op {
                0x00 => {}
                0x01 => st.push(V::Null),
                0x02..=0x08 => st.push(V::I(op as i32 - 3)),
                0x09 | 0x0A => st.push(V::L(op as i64 - 9)),
                0x0B..=0x0D => st.push(V::F((op - 0x0B) as f32)),
                0x0E | 0x0F => st.push(V::D((op - 0x0E) as f64)),
                0x10 => {
                    st.push(V::I(u8_at(pc + 1)? as i8 as i32));
                    next = pc + 2;
                }
                0x11 => {
                    st.push(V::I(i16_at(pc + 1)? as i32));
                    next = pc + 3;
                }
                0x12..=0x14 => {
                    let idx = if op == 0x12 { u8_at(pc + 1)? as u16 } else { u16_at(pc + 1)? };
                    next = pc + if op == 0x12 { 2 } else { 3 };
                    st.push(match cf.consts.get(idx as usize) {
                        Some(Const::Int(i)) => V::I(*i),
                        Some(Const::Float(f)) => V::F(*f),
                        Some(Const::Long(l)) => V::L(*l),
                        Some(Const::Double(d)) => V::D(*d),
                        Some(Const::Str(s)) => V::Str(cf.utf8(*s).ok_or(Abort)?.to_owned()),
                        _ => return Err(Abort),
                    });
                }
                0x15..=0x19 => {
                    st.push(locals.get(u8_at(pc + 1)? as usize).cloned().ok_or(Abort)?);
                    next = pc + 2;
                }
                0x1A..=0x2D => st.push(locals.get(((op - 0x1A) % 4) as usize).cloned().ok_or(Abort)?),
                0x2E | 0x30 | 0x32 => {
                    let i = popi!();
                    let V::Arr(a) = pop!() else { return Err(Abort) };
                    let v = *self.arrays[a].get(i as usize).ok_or(Abort)?;
                    st.push(if op == 0x30 { V::F(v) } else { V::I(v as i32) });
                }
                0x36..=0x3A => {
                    let v = pop!();
                    let s = u8_at(pc + 1)? as usize;
                    *locals.get_mut(s).ok_or(Abort)? = v;
                    next = pc + 2;
                }
                0x3B..=0x4E => {
                    let v = pop!();
                    *locals.get_mut(((op - 0x3B) % 4) as usize).ok_or(Abort)? = v;
                }
                0x51 => {
                    let v = popf!();
                    let i = popi!();
                    let V::Arr(a) = pop!() else { return Err(Abort) };
                    *self.arrays[a].get_mut(i as usize).ok_or(Abort)? = v;
                }
                0x57 => {
                    pop!();
                }
                0x59 => {
                    let t = st.last().cloned().ok_or(Abort)?;
                    st.push(t);
                }
                0x60 | 0x64 | 0x68 | 0x6C => {
                    let (b, a) = (popi!(), popi!());
                    st.push(V::I(match op {
                        0x60 => a.wrapping_add(b),
                        0x64 => a.wrapping_sub(b),
                        0x68 => a.wrapping_mul(b),
                        _ => a.checked_div(b).ok_or(Abort)?,
                    }));
                }
                0x62 | 0x66 | 0x6A | 0x6E | 0x72 => {
                    let (b, a) = (popf!(), popf!());
                    st.push(V::F(match op {
                        0x62 => a + b,
                        0x66 => a - b,
                        0x6A => a * b,
                        0x6E => a / b,
                        _ => a % b,
                    }));
                }
                0x63 | 0x67 | 0x6B | 0x6F => {
                    let (b, a) = (popd!(), popd!());
                    st.push(V::D(match op {
                        0x63 => a + b,
                        0x67 => a - b,
                        0x6B => a * b,
                        _ => a / b,
                    }));
                }
                0x74 => {
                    let a = popi!();
                    st.push(V::I(-a));
                }
                0x76 => {
                    let a = popf!();
                    st.push(V::F(-a));
                }
                0x77 => {
                    let a = popd!();
                    st.push(V::D(-a));
                }
                0x86 => {
                    let a = popi!();
                    st.push(V::F(a as f32));
                }
                0x87 => {
                    let a = popi!();
                    st.push(V::D(a as f64));
                }
                0x8B => {
                    let a = popf!();
                    st.push(V::I(a as i32));
                }
                0x8D => {
                    let a = popf!();
                    st.push(V::D(a as f64));
                }
                0x8E => {
                    let a = popd!();
                    st.push(V::I(a as i32));
                }
                0x90 => {
                    let a = popd!();
                    st.push(V::F(a as f32));
                }
                0x95..=0x98 => {
                    let (a, b) = if op <= 0x96 {
                        let (b, a) = (popf!() as f64, popf!() as f64);
                        (a, b)
                    } else {
                        let (b, a) = (popd!(), popd!());
                        (a, b)
                    };
                    let nan = if op == 0x95 || op == 0x97 { -1 } else { 1 };
                    st.push(V::I(if a.is_nan() || b.is_nan() {
                        nan
                    } else {
                        (a > b) as i32 - (a < b) as i32
                    }));
                }
                0x99..=0x9E | 0xC6 | 0xC7 => {
                    let take = match op {
                        0xC6 => pop!() == V::Null,
                        0xC7 => pop!() != V::Null,
                        _ => {
                            let v = popi!();
                            match op {
                                0x99 => v == 0,
                                0x9A => v != 0,
                                0x9B => v < 0,
                                0x9C => v >= 0,
                                0x9D => v > 0,
                                _ => v <= 0,
                            }
                        }
                    };
                    next = if take { (pc as i64 + i16_at(pc + 1)? as i64) as usize } else { pc + 3 };
                }
                0x9F..=0xA4 => {
                    let (b, a) = (popi!(), popi!());
                    let take = match op {
                        0x9F => a == b,
                        0xA0 => a != b,
                        0xA1 => a < b,
                        0xA2 => a >= b,
                        0xA3 => a > b,
                        _ => a <= b,
                    };
                    next = if take { (pc as i64 + i16_at(pc + 1)? as i64) as usize } else { pc + 3 };
                }
                0xA7 => next = (pc as i64 + i16_at(pc + 1)? as i64) as usize,
                0xAC | 0xAE | 0xAF | 0xB0 => return Ok(Some(pop!())),
                0xB1 => return Ok(None),
                0xB2 => {
                    let (_, name, _) = cf.member(u16_at(pc + 1)?).ok_or(Abort)?;
                    st.push(match name {
                        "xyz" => V::Arr(0),
                        "ypr" => V::Arr(1),
                        _ => return Err(Abort),
                    });
                    next = pc + 3;
                }
                0xC1 => {
                    let target = cf.class_name(u16_at(pc + 1)?).ok_or(Abort)?;
                    let v = pop!();
                    st.push(V::I((v == V::This && self.chain.iter().any(|c| c.this_class == target)) as i32));
                    next = pc + 3;
                }
                0xB6..=0xB8 => {
                    let (class, name, desc) = cf.member(u16_at(pc + 1)?).ok_or(Abort)?;
                    let nargs = desc_args(desc).len();
                    if st.len() < nargs {
                        return Err(Abort);
                    }
                    let args = st.split_off(st.len() - nargs);
                    next = pc + 3;
                    let ret = if op == 0xB8 {
                        self.static_call(class, name, desc, args, depth)?
                    } else {
                        let recv = pop!();
                        self.virtual_call(recv, op == 0xB7, class, name, desc, args, depth)?
                    };
                    if let Some(r) = ret {
                        st.push(r);
                    }
                }
                _ => return Err(Abort),
            }
            pc = next;
        }
    }

    fn static_call(&mut self, class: &str, name: &str, desc: &str, args: Vec<V>, depth: usize) -> Result<Option<V>, Abort> {
        let f = |v: &V| match v {
            V::F(x) => Ok(*x),
            V::I(i) => Ok(*i as f32),
            _ => Err(Abort),
        };
        let d = |v: &V| match v {
            V::D(x) => Ok(*x),
            _ => Err(Abort),
        };
        if class == "java/lang/Math" {
            let r = match (name, desc) {
                ("cos", "(D)D") => V::D(d(&args[0])?.cos()),
                ("sin", "(D)D") => V::D(d(&args[0])?.sin()),
                ("abs", "(F)F") => V::F(f(&args[0])?.abs()),
                ("abs", "(D)D") => V::D(d(&args[0])?.abs()),
                ("max", "(FF)F") => V::F(f(&args[0])?.max(f(&args[1])?)),
                ("min", "(FF)F") => V::F(f(&args[0])?.min(f(&args[1])?)),
                ("toRadians", "(D)D") => V::D(d(&args[0])?.to_radians()),
                _ => return Err(Abort),
            };
            return Ok(Some(r));
        }
        if name == "cvt" && desc == "(FFFFF)F" {
            let x: Vec<f32> = args.iter().map(f).collect::<Result<_, _>>()?;
            let v = x[0].max(x[1]).min(x[2]);
            return Ok(Some(V::F(x[3] + (x[4] - x[3]) * (v - x[1]) / (x[2] - x[1]))));
        }
        let (cf, m) = self.resolve(Some(class), name, desc).ok_or(Abort)?;
        self.call(cf, m, args, depth + 1)
    }

    #[allow(clippy::too_many_arguments)]
    fn virtual_call(&mut self, recv: V, special: bool, class: &str, name: &str, desc: &str, args: Vec<V>, depth: usize) -> Result<Option<V>, Abort> {
        match recv {
            V::Hier => {
                let node = match args.first() {
                    Some(V::Str(s)) => s.clone(),
                    _ => return Err(Abort),
                };
                let f = |v: &V| match v {
                    V::F(x) => Ok(*x),
                    V::I(i) => Ok(*i as f32),
                    _ => Err(Abort),
                };
                let entry = self.out.entry(node).or_default();
                match (name, desc) {
                    ("chunkSetAngles", "(Ljava/lang/String;FFF)V") => {
                        entry.angles = [f(&args[1])?, f(&args[2])?, f(&args[3])?];
                    }
                    ("chunkSetAngles", "(Ljava/lang/String;[F)V") => {
                        let V::Arr(a) = args[1] else { return Err(Abort) };
                        let v = &self.arrays[a];
                        entry.angles = [v[0], v[1], v[2]];
                    }
                    ("chunkSetLocate", "(Ljava/lang/String;[F[F)V") => {
                        let (V::Arr(x), V::Arr(r)) = (&args[1], &args[2]) else { return Err(Abort) };
                        let (x, r) = (&self.arrays[*x], &self.arrays[*r]);
                        entry.offset = [x[0], x[1], x[2]];
                        entry.angles = [r[0], r[1], r[2]];
                    }
                    ("chunkVisible", "(Ljava/lang/String;Z)V") => {
                        entry.visible = Some(matches!(args[1], V::I(v) if v != 0));
                    }
                    _ => return Err(Abort),
                }
                Ok(None)
            }
            V::This => {
                if name == "hierMesh" {
                    return Ok(Some(V::Hier));
                }
                if name == "resetYPRmodifier" {
                    for a in &mut self.arrays {
                        a.iter_mut().for_each(|v| *v = 0.0);
                    }
                    return Ok(None);
                }
                let (cf, m) = self.resolve(special.then_some(class), name, desc).ok_or(Abort)?;
                let mut full = vec![V::This];
                full.extend(args);
                self.call(cf, m, full, depth + 1)
            }
            _ => Err(Abort),
        }
    }
}

/// Samples one channel of an aircraft; `None` if its methods cannot be evaluated.
pub fn sample(chain: &[ClassFile], ch: Channel) -> Option<Track> {
    const HM: &str = "Lcom/maddox/il2/engine/HierMesh;";
    let (lo, hi) = ch.range();
    let inputs: Vec<f32> = (0..SAMPLES).map(|i| lo + (hi - lo) * i as f32 / (SAMPLES - 1) as f32).collect();
    let mut m = Machine { chain, arrays: vec![vec![0.0; 3], vec![0.0; 3]], out: BTreeMap::new(), budget: 0 };
    let mut frames: Vec<BTreeMap<String, NodeState>> = Vec::new();
    let virtual_name = match ch {
        Channel::Rudder => "moveRudder",
        Channel::Elevator => "moveElevator",
        Channel::Aileron => "moveAileron",
        Channel::Flaps => "moveFlap",
        Channel::Gear => "",
    };
    // Gear: the static methods of the most derived class that declares them.
    let gear3 = chain.iter().find_map(|c| c.method("moveGear", &format!("({HM}FFF)V")).filter(|m| m.is_static).map(|mm| (c, mm)));
    let gear1 = chain.iter().find_map(|c| c.method("moveGear", &format!("({HM}F)V")).filter(|m| m.is_static).map(|mm| (c, mm)));
    if ch == Channel::Gear && gear3.is_none() && gear1.is_none() {
        return None;
    }
    let virt = (ch != Channel::Gear).then(|| m.resolve(None, virtual_name, "(F)V")).flatten();
    // Only use an override below the generic Aircraft base (whose bodies are empty).
    if ch != Channel::Gear && virt.is_none_or(|(c, _)| c.this_class == "com/maddox/il2/objects/air/Aircraft") {
        return None;
    }
    for &x in &inputs {
        m.budget = 200_000;
        let ok = if ch == Channel::Gear {
            let mut ok = true;
            for (c, mm) in [gear3, gear1].into_iter().flatten() {
                let args = if mm.desc.contains("FFF") { vec![V::Hier, V::F(x), V::F(x), V::F(x)] } else { vec![V::Hier, V::F(x)] };
                ok &= m.call(c, mm, args, 0).is_ok();
            }
            ok
        } else {
            let (c, mm) = virt?;
            m.call(c, mm, vec![V::This, V::F(x)], 0).is_ok()
        };
        if !ok {
            return None;
        }
        frames.push(m.out.clone());
    }
    let mut nodes: BTreeMap<String, Vec<NodeState>> = BTreeMap::new();
    for name in frames.iter().flat_map(|f| f.keys()) {
        if !nodes.contains_key(name) {
            nodes.insert(name.clone(), frames.iter().map(|f| f.get(name).copied().unwrap_or_default()).collect());
        }
    }
    Some(Track { channel: ch, inputs, nodes, imported: true })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_args() {
        assert_eq!(desc_args("(Lcom/x/HierMesh;FFF)V"), ['L', 'F', 'F', 'F']);
        assert_eq!(desc_args("(Ljava/lang/String;[F[F)V"), ['L', 'L', 'L']);
        assert_eq!(desc_args("(DJI)V"), ['D', 'J', 'I']);
    }

    #[test]
    fn default_rudder_track() {
        let t = default_track(Channel::Rudder);
        let (name, s) = t.eval(0.5).next().unwrap();
        assert_eq!(name, "Rudder1_D0");
        assert!((s.angles[1] + 15.0).abs() < 1e-4);
    }
}
