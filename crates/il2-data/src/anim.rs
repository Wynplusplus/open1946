//! Animation importer: `docs/behavior/animation.md`, "Importer".
//!
//! Evaluates an aircraft's part-animation methods from the user's installed runtime classes in a
//! restricted interpreter and samples them into tables. Anything outside the allowed instruction
//! set aborts that method and the generic default is used instead.

use crate::classfile::{ClassFile, Const, Method};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Channel {
    Rudder,
    Elevator,
    Aileron,
    Flaps,
    Gear,
    /// `moveArrestorHook`: 0 up .. 1 down.
    Arrestor,
    /// `moveWingFold(HierMesh, float)`: 0 spread .. 1 folded.
    WingFold,
}

impl Channel {
    pub const ALL: [Channel; 7] = [
        Channel::Rudder,
        Channel::Elevator,
        Channel::Aileron,
        Channel::Flaps,
        Channel::Gear,
        Channel::Arrestor,
        Channel::WingFold,
    ];

    fn range(self) -> (f32, f32) {
        match self {
            Channel::Rudder | Channel::Elevator | Channel::Aileron => (-1.0, 1.0),
            Channel::Flaps | Channel::Gear | Channel::Arrestor | Channel::WingFold => (0.0, 1.0),
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
            (
                name.as_str(),
                NodeState {
                    angles: lerp(a.angles, b.angles),
                    offset: lerp(a.offset, b.offset),
                    visible: if t < 0.5 { a.visible } else { b.visible },
                },
            )
        })
    }
}

const SAMPLES: usize = 41;

/// The generic default for a channel.
pub fn default_track(ch: Channel) -> Track {
    let (lo, hi) = ch.range();
    let inputs: Vec<f32> = (0..SAMPLES)
        .map(|i| lo + (hi - lo) * i as f32 / (SAMPLES - 1) as f32)
        .collect();
    let (names, k): (&[&str], f32) = match ch {
        Channel::Rudder => (&["Rudder1_D0"], -30.0),
        Channel::Elevator => (&["VatorL_D0", "VatorR_D0"], -30.0),
        Channel::Aileron => (&["AroneL_D0", "AroneR_D0"], -30.0),
        Channel::Flaps => (&["Flap01_D0", "Flap02_D0", "Flap03_D0", "Flap04_D0"], -50.0),
        Channel::Gear | Channel::Arrestor | Channel::WingFold => (&[], 0.0),
    };
    let nodes = names
        .iter()
        .map(|n| {
            (
                n.to_string(),
                inputs
                    .iter()
                    .map(|&x| NodeState {
                        angles: [0.0, k * x, 0.0],
                        ..Default::default()
                    })
                    .collect(),
            )
        })
        .collect();
    Track {
        channel: ch,
        inputs,
        nodes,
        imported: false,
    }
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
    /// Array of references (e.g. `float[][]`), in `Machine::oarrays`.
    OArr(usize),
    /// An object on the heap (`Machine::objects`).
    Obj(usize),
    Null,
    /// Second slot of a long/double local.
    Top,
    /// A value of the running simulation, named by its access path (`fm.EI.engines[0]`); see
    /// `Instruments`.
    Sym(String),
}

struct Abort;

thread_local! {
    static WHY: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

/// Records why evaluation stopped (see `last_abort`).
fn why(reason: String) -> Abort {
    WHY.with(|w| *w.borrow_mut() = reason);
    Abort
}

/// Why the last failed sampling on this thread stopped, for diagnostics.
pub fn last_abort() -> String {
    WHY.with(|w| w.borrow().clone())
}

/// Interpreter state shared across the calls of one sampling run.
struct Machine<'a> {
    chain: &'a [ClassFile],
    arrays: Vec<Vec<f32>>,
    oarrays: Vec<Vec<V>>,
    /// Heap objects: class and fields.
    objects: Vec<(String, HashMap<String, V>)>,
    out: BTreeMap<String, NodeState>,
    budget: usize,
    /// Static fields (`class.name`), filled by the classes' static initializers.
    statics: HashMap<String, V>,
    /// Instance fields of `this`.
    fields: HashMap<String, V>,
    clinit_done: HashSet<String>,
    /// Inside a static initializer: unknown calls yield defaults instead of aborting.
    lenient: bool,
    /// Static methods whose calls are recorded (name and arguments) instead of run.
    record: &'static [&'static str],
    recorded: Vec<(String, Vec<Value>)>,
    /// Values of the simulation by access path; `Some` when running cockpit code.
    world: Option<HashMap<String, f64>>,
    /// Interpolators the code registered with `interpPut`.
    interps: Vec<V>,
    /// Paths read from `world` that it did not have.
    missing: HashSet<String>,
    /// Position of the instruction being run (diagnostics).
    pc: usize,
    /// The first failure inside a helper that cockpit code recovered from.
    soft_error: Option<String>,
    /// Running damage code (`run_hit_bone`): world calls and reads go here.
    host: Option<&'a mut (dyn Host + 'a)>,
    /// The `Shot` object on the heap while `host` is set.
    shot_obj: Option<usize>,
}

/// A value passed between class code and its `Host`.
#[derive(Clone, Debug, PartialEq)]
pub enum HostVal {
    Num(f64),
    Str(String),
    /// A symbolic object: `this`, `shot`, `point`, or a path such as `FM.AS`,
    /// `FM.EI.engines[0]`, `World.Rnd()`.
    Path(String),
    Null,
}

impl HostVal {
    pub fn num(&self) -> f64 {
        match self {
            HostVal::Num(x) => *x,
            _ => 0.0,
        }
    }
    pub fn str(&self) -> &str {
        match self {
            HostVal::Str(s) | HostVal::Path(s) => s,
            _ => "",
        }
    }
}

/// The round damage code works on (the runtime's `Shot`): energy (J), mass (kg) and kind.
#[derive(Clone, Debug, Default)]
pub struct ShotState {
    pub power: f64,
    pub mass: f64,
    pub power_type: i32,
}

/// What class code reaches outside itself while it runs under `run_hit_bone`.
pub trait Host {
    /// A call on `recv`: `this` (the aircraft; return `None` to run the class's own code), a
    /// static class's short name (`Aircraft`, `World`), `shot`, or a symbolic path. `None`
    /// means "not handled" (the result is then a default).
    fn call(
        &mut self,
        recv: &str,
        name: &str,
        args: &[HostVal],
        shot: &mut ShotState,
    ) -> Option<HostVal>;
    /// A number of the world at `path` (`FM.AS.astateTankStates[0]`, …).
    fn get(&mut self, path: &str) -> f64;
}

/// Zero value of a field or return descriptor.
fn default_of(desc: &str) -> V {
    match desc.chars().next() {
        Some('F') => V::F(0.0),
        Some('D') => V::D(0.0),
        Some('J') => V::L(0),
        Some('I' | 'Z' | 'B' | 'C' | 'S') => V::I(0),
        _ => V::Null,
    }
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

/// A number of the type a descriptor names.
fn typed(desc: &str, x: f64) -> V {
    match desc.chars().next() {
        Some('F') => V::F(x as f32),
        Some('D') => V::D(x),
        Some('J') => V::L(x as i64),
        _ => V::I(x as i32),
    }
}

/// The return descriptor of a method descriptor.
fn ret_of(desc: &str) -> &str {
    &desc[desc.find(')').map_or(desc.len(), |i| i + 1)..]
}

/// `com/maddox/il2/ai/World` -> `World`.
fn short(class: &str) -> &str {
    class.rsplit('/').next().unwrap_or(class)
}

impl<'a> Machine<'a> {
    fn to_host(&self, v: &V) -> HostVal {
        match v {
            V::I(i) => HostVal::Num(*i as f64),
            V::L(l) => HostVal::Num(*l as f64),
            V::F(f) => HostVal::Num(*f as f64),
            V::D(d) => HostVal::Num(*d),
            V::Str(s) => HostVal::Str(s.clone()),
            V::Sym(p) => HostVal::Path(p.clone()),
            V::This => HostVal::Path("this".into()),
            V::Obj(o) if Some(*o) == self.shot_obj => HostVal::Path("shot".into()),
            V::Obj(o) => match self.objects[*o].1.get("$s") {
                Some(V::Str(s)) => HostVal::Str(s.clone()),
                _ => HostVal::Path(format!("obj{o}")),
            },
            _ => HostVal::Null,
        }
    }

    fn shot_state(&self) -> ShotState {
        let Some(o) = self.shot_obj else {
            return ShotState::default();
        };
        let f = |k: &str| match self.objects[o].1.get(k) {
            Some(V::F(x)) => *x as f64,
            Some(V::D(x)) => *x,
            Some(V::I(x)) => *x as f64,
            _ => 0.0,
        };
        ShotState {
            power: f("power"),
            mass: f("mass"),
            power_type: f("powerType") as i32,
        }
    }

    fn set_shot_state(&mut self, s: &ShotState) {
        if let Some(o) = self.shot_obj {
            let fields = &mut self.objects[o].1;
            fields.insert("power".into(), V::F(s.power as f32));
            fields.insert("mass".into(), V::F(s.mass as f32));
            fields.insert("powerType".into(), V::I(s.power_type));
        }
    }

    /// Offers a call to the host; `Some` when it handled it (with the typed result).
    fn host_call(&mut self, recv: &str, name: &str, desc: &str, args: &[V]) -> Option<Option<V>> {
        self.host.as_ref()?;
        let hargs: Vec<HostVal> = args.iter().map(|a| self.to_host(a)).collect();
        let mut shot = self.shot_state();
        let r = self
            .host
            .as_deref_mut()?
            .call(recv, name, &hargs, &mut shot);
        self.set_shot_state(&shot);
        let ret = ret_of(desc);
        r.map(|h| {
            (ret != "V").then(|| match h {
                HostVal::Num(x) => typed(ret, x),
                HostVal::Str(s) => V::Str(s),
                HostVal::Path(p) => V::Sym(p),
                HostVal::Null => default_of(ret),
            })
        })
    }

    /// `java.lang.String` methods on a string value.
    fn string_call(s: &str, name: &str, desc: &str, args: &[V]) -> Option<Option<V>> {
        let arg_str = |i: usize| match args.get(i) {
            Some(V::Str(t)) => Some(t.as_str()),
            _ => None,
        };
        let arg_int = |i: usize| match args.get(i) {
            Some(V::I(n)) => Some(*n),
            _ => None,
        };
        let chars: Vec<char> = s.chars().collect();
        let b = |x: bool| Some(Some(V::I(x as i32)));
        match (name, desc) {
            ("startsWith", _) => b(s.starts_with(arg_str(0)?)),
            ("endsWith", _) => b(s.ends_with(arg_str(0)?)),
            ("equals", _) => b(arg_str(0) == Some(s)),
            ("equalsIgnoreCase", _) => b(arg_str(0).is_some_and(|t| t.eq_ignore_ascii_case(s))),
            ("length", _) => Some(Some(V::I(chars.len() as i32))),
            ("charAt", _) => Some(Some(V::I(
                chars.get(arg_int(0)? as usize).map_or(0, |&c| c as i32),
            ))),
            ("substring", "(I)Ljava/lang/String;") => Some(Some(V::Str(
                chars.get(arg_int(0)?.max(0) as usize..)?.iter().collect(),
            ))),
            ("substring", _) => {
                let (a, e) = (arg_int(0)?.max(0) as usize, arg_int(1)?.max(0) as usize);
                Some(Some(V::Str(chars.get(a..e)?.iter().collect())))
            }
            ("toLowerCase", _) => Some(Some(V::Str(s.to_lowercase()))),
            ("toUpperCase", _) => Some(Some(V::Str(s.to_uppercase()))),
            ("trim", _) | ("intern", _) | ("toString", _) => {
                Some(Some(V::Str(s.trim().to_owned())))
            }
            ("compareTo", _) => Some(Some(V::I(match s.cmp(arg_str(0)?) {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            }))),
            ("indexOf", "(Ljava/lang/String;)I") => Some(Some(V::I(
                s.find(arg_str(0)?)
                    .map_or(-1, |i| s[..i].chars().count() as i32),
            ))),
            ("indexOf", "(I)I") => Some(Some(V::I(
                chars
                    .iter()
                    .position(|&c| c as i32 == arg_int(0).unwrap_or(-1))
                    .map_or(-1, |i| i as i32),
            ))),
            ("lastIndexOf", "(Ljava/lang/String;)I") => Some(Some(V::I(
                s.rfind(arg_str(0)?)
                    .map_or(-1, |i| s[..i].chars().count() as i32),
            ))),
            ("lastIndexOf", "(I)I") => Some(Some(V::I(
                chars
                    .iter()
                    .rposition(|&c| c as i32 == arg_int(0).unwrap_or(-1))
                    .map_or(-1, |i| i as i32),
            ))),
            ("concat", _) => Some(Some(V::Str(format!("{s}{}", arg_str(0)?)))),
            ("hashCode", _) => {
                Some(Some(V::I(s.chars().fold(0i32, |h, c| {
                    h.wrapping_mul(31).wrapping_add(c as i32)
                }))))
            }
            _ => None,
        }
    }

    /// The simulation's value at `path`, typed by `desc`: objects stay symbolic, numbers come from
    /// `world` (0 when unknown).
    fn world_value(&mut self, path: String, desc: &str) -> Option<V> {
        // The player's flight model, however the code reaches it.
        let path = match path.strip_prefix("World.getPlayerFM()") {
            Some(rest) => format!("fm{rest}"),
            None => path.replacen("fm.actor.FM.", "fm.", 1),
        };
        match desc.chars().next() {
            Some('V') => None,
            Some('L' | '[') => Some(V::Sym(path)),
            _ if self.host.is_some() => {
                let x = self.host.as_deref_mut().map_or(0.0, |h| h.get(&path));
                Some(typed(desc, x))
            }
            _ => {
                let x = match self.world.as_ref().and_then(|w| w.get(&path)) {
                    Some(&x) => x,
                    // The engine's answers when nothing more is known: the aircraft's parts
                    // are all there, and cockpit materials have no night variants.
                    None if path.contains(".isChunkVisible(") => 1.0,
                    None if path.contains(".materialFind(") => -1.0,
                    None => {
                        self.missing.insert(path);
                        0.0
                    }
                };
                Some(typed(desc, x))
            }
        }
    }

    /// Finds a method along the chain, starting at class `from` (or the most derived when `None`).
    fn resolve(
        &self,
        from: Option<&str>,
        name: &str,
        desc: &str,
    ) -> Option<(&'a ClassFile, &'a Method)> {
        let start = from.or_else(|| self.chain.first().map(|c| c.this_class.as_str()))?;
        self.resolve_class(start, name, desc)
    }

    /// Finds a method in `class` or its superclasses, among the loaded classes.
    fn resolve_class(
        &self,
        class: &str,
        name: &str,
        desc: &str,
    ) -> Option<(&'a ClassFile, &'a Method)> {
        let chain: &'a [ClassFile] = self.chain;
        let mut cur = Some(class.to_owned());
        let mut guard = 0;
        while let Some(c) = cur.take() {
            guard += 1;
            let cf = chain.iter().find(|cf| cf.this_class == c)?;
            if let Some(m) = cf.method(name, desc) {
                return Some((cf, m));
            }
            if guard > 32 {
                return None;
            }
            cur = cf.super_class.clone();
        }
        None
    }

    fn class_of(&self, v: &V) -> Option<String> {
        match v {
            V::Obj(o) => Some(self.objects[*o].0.clone()),
            V::This => self.chain.first().map(|c| c.this_class.clone()),
            _ => None,
        }
    }

    /// Converts a value to a plain tree (for property objects).
    fn export(&self, v: &V, depth: usize) -> Value {
        if depth > 8 {
            return Value::Null;
        }
        match v {
            V::I(i) => Value::Num(*i as f64),
            V::L(l) => Value::Num(*l as f64),
            V::F(f) => Value::Num(*f as f64),
            V::D(d) => Value::Num(*d),
            V::Str(s) => Value::Str(s.clone()),
            V::Arr(a) => Value::List(
                self.arrays[*a]
                    .iter()
                    .map(|x| Value::Num(*x as f64))
                    .collect(),
            ),
            V::OArr(a) => Value::List(
                self.oarrays[*a]
                    .iter()
                    .map(|x| self.export(x, depth + 1))
                    .collect(),
            ),
            V::Obj(o) => Value::Obj(
                self.objects[*o].0.clone(),
                self.objects[*o]
                    .1
                    .iter()
                    .map(|(k, x)| (k.clone(), self.export(x, depth + 1)))
                    .collect(),
            ),
            _ => Value::Null,
        }
    }

    fn call(
        &mut self,
        cf: &'a ClassFile,
        m: &'a Method,
        args: Vec<V>,
        depth: usize,
    ) -> Result<Option<V>, Abort> {
        if depth == 0 {
            WHY.with(|w| w.borrow_mut().clear());
        }
        let r = self.call_inner(cf, m, args, depth);
        if r.is_err() && WHY.with(|w| w.borrow().is_empty()) {
            let op = m.code.get(self.pc).copied().unwrap_or(0);
            let at = format!(
                "in {}.{} at pc {} (op 0x{op:02X})",
                short(&cf.this_class),
                m.name,
                self.pc
            );
            WHY.with(|w| *w.borrow_mut() = at);
        }
        // Cockpit code: a failing helper returns its default, so one unsupported call doesn't
        // stop the remaining instruments.
        if r.is_err() && self.world.is_some() && depth > 0 && self.budget > 0 {
            if self.soft_error.is_none() {
                self.soft_error = Some(last_abort());
            }
            WHY.with(|w| w.borrow_mut().clear());
            let ret = ret_of(&m.desc);
            return Ok((ret != "V").then(|| default_of(ret)));
        }
        r
    }

    fn call_inner(
        &mut self,
        cf: &'a ClassFile,
        m: &'a Method,
        args: Vec<V>,
        depth: usize,
    ) -> Result<Option<V>, Abort> {
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
        let u16_at =
            |p: usize| -> Result<u16, Abort> { Ok(u16::from_be_bytes([u8_at(p)?, u8_at(p + 1)?])) };
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
        macro_rules! popl {
            () => {
                match pop!() {
                    V::L(l) => l,
                    V::I(i) => i as i64,
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
            self.budget = self
                .budget
                .checked_sub(1)
                .ok_or_else(|| why("budget".into()))?;
            self.pc = pc;
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
                    let idx = if op == 0x12 {
                        u8_at(pc + 1)? as u16
                    } else {
                        u16_at(pc + 1)?
                    };
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
                0x1A..=0x2D => st.push(
                    locals
                        .get(((op - 0x1A) % 4) as usize)
                        .cloned()
                        .ok_or(Abort)?,
                ),
                0x32 => {
                    let i = popi!();
                    match pop!() {
                        V::OArr(a) => {
                            st.push(self.oarrays[a].get(i as usize).cloned().ok_or(Abort)?)
                        }
                        V::Sym(p) => st.push(V::Sym(format!("{p}[{i}]"))),
                        V::Null if self.lenient => st.push(V::Null),
                        _ => return Err(why("aaload".into())),
                    }
                }
                0x53 => {
                    let v = pop!();
                    let i = popi!();
                    match pop!() {
                        V::OArr(a) => *self.oarrays[a].get_mut(i as usize).ok_or(Abort)? = v,
                        V::Sym(_) | V::Null if self.lenient => {}
                        _ => return Err(why("aastore".into())),
                    }
                }
                0xBD => {
                    let n = popi!();
                    if !(0..=4096).contains(&n) {
                        return Err(why("anewarray size".into()));
                    }
                    self.oarrays.push(vec![V::Null; n as usize]);
                    st.push(V::OArr(self.oarrays.len() - 1));
                    next = pc + 3;
                }
                0x2E..=0x31 => {
                    let i = popi!();
                    let desc = ["I", "J", "F", "D"][(op - 0x2E) as usize];
                    match pop!() {
                        V::Arr(a) => {
                            let v = *self.arrays[a].get(i as usize).ok_or(Abort)?;
                            st.push(typed(desc, v as f64));
                        }
                        V::Sym(p) => {
                            st.push(self.world_value(format!("{p}[{i}]"), desc).ok_or(Abort)?)
                        }
                        V::Null if self.lenient => st.push(typed(desc, 0.0)),
                        _ => return Err(Abort),
                    }
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
                    next = if take {
                        (pc as i64 + i16_at(pc + 1)? as i64) as usize
                    } else {
                        pc + 3
                    };
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
                    next = if take {
                        (pc as i64 + i16_at(pc + 1)? as i64) as usize
                    } else {
                        pc + 3
                    };
                }
                0xA7 => next = (pc as i64 + i16_at(pc + 1)? as i64) as usize,
                0xAC..=0xB0 => return Ok(Some(pop!())),
                0xB1 => return Ok(None),
                0xB2 => {
                    let (class, name, desc) = cf.member(u16_at(pc + 1)?).ok_or(Abort)?;
                    let v = match name {
                        "xyz" => V::Arr(0),
                        "ypr" => V::Arr(1),
                        _ => {
                            let key = format!("{class}.{name}");
                            if !self.statics.contains_key(&key) {
                                self.run_clinit(class, depth);
                            }
                            match self.statics.get(&key) {
                                Some(v) => v.clone(),
                                None if self.world.is_some()
                                    && !self.chain.iter().any(|c| c.this_class == class) =>
                                {
                                    self.world_value(format!("{}.{name}", short(class)), desc)
                                        .unwrap_or(V::Null)
                                }
                                None => default_of(desc),
                            }
                        }
                    };
                    st.push(v);
                    next = pc + 3;
                }
                0xB3 => {
                    let (class, name, _) = cf.member(u16_at(pc + 1)?).ok_or(Abort)?;
                    let v = pop!();
                    self.statics.insert(format!("{class}.{name}"), v);
                    next = pc + 3;
                }
                0xB4 => {
                    let (_, name, desc) = cf.member(u16_at(pc + 1)?).ok_or(Abort)?;
                    let v = match pop!() {
                        V::This => match self.fields.get(name).cloned() {
                            None if self.host.is_some() => self.world_value(name.to_owned(), desc),
                            v => v,
                        },
                        V::Obj(o) => self.objects[o].1.get(name).cloned(),
                        V::Sym(p) => self.world_value(format!("{p}.{name}"), desc),
                        _ if self.lenient => None,
                        _ => return Err(why(format!("getfield {name} of another object"))),
                    };
                    st.push(v.unwrap_or_else(|| default_of(desc)));
                    next = pc + 3;
                }
                0xB5 => {
                    let (_, name, _) = cf.member(u16_at(pc + 1)?).ok_or(Abort)?;
                    let v = pop!();
                    match pop!() {
                        V::This => {
                            self.fields.insert(name.to_owned(), v);
                        }
                        V::Obj(o) => {
                            self.objects[o].1.insert(name.to_owned(), v);
                        }
                        _ if self.lenient => {}
                        _ => return Err(why(format!("putfield {name} of another object"))),
                    }
                    next = pc + 3;
                }
                0xBC => {
                    let n = popi!();
                    if !(0..=4096).contains(&n) {
                        return Err(why("newarray size".into()));
                    }
                    self.arrays.push(vec![0.0; n as usize]);
                    st.push(V::Arr(self.arrays.len() - 1));
                    next = pc + 2;
                }
                0xBE => {
                    let n = match pop!() {
                        V::Arr(a) => self.arrays[a].len(),
                        V::OArr(a) => self.oarrays[a].len(),
                        V::Sym(p) => match self.world_value(format!("{p}.length"), "I") {
                            Some(V::I(n)) => n.max(0) as usize,
                            _ => 0,
                        },
                        V::Null if self.lenient => 0,
                        _ => return Err(Abort),
                    };
                    st.push(V::I(n as i32));
                }
                0x5B => {
                    let (v1, v2, v3) = (pop!(), pop!(), pop!());
                    st.extend([v1.clone(), v3, v2, v1]);
                }
                0x5A => {
                    let (v1, v2) = (pop!(), pop!());
                    st.extend([v1.clone(), v2, v1]);
                }
                0x5F => {
                    let (v1, v2) = (pop!(), pop!());
                    st.extend([v1, v2]);
                }
                0x33 | 0x34 | 0x35 => {
                    let i = popi!();
                    match pop!() {
                        V::Arr(a) => {
                            let v = *self.arrays[a].get(i as usize).ok_or(Abort)?;
                            st.push(V::I(v as i32));
                        }
                        V::Sym(p) => {
                            st.push(self.world_value(format!("{p}[{i}]"), "I").ok_or(Abort)?)
                        }
                        V::Null if self.lenient => st.push(V::I(0)),
                        _ => return Err(Abort),
                    }
                }
                0x4F | 0x54 | 0x55 | 0x56 => {
                    let v = popi!();
                    let i = popi!();
                    let V::Arr(a) = pop!() else { return Err(Abort) };
                    *self.arrays[a].get_mut(i as usize).ok_or(Abort)? = v as f32;
                }
                // checkcast: the reference stays.
                0xC0 => next = pc + 3,
                0xC2 | 0xC3 => {
                    pop!();
                }
                0x84 => {
                    let i = u8_at(pc + 1)? as usize;
                    let c = u8_at(pc + 2)? as i8 as i32;
                    let V::I(v) = locals.get(i).cloned().ok_or(Abort)? else {
                        return Err(Abort);
                    };
                    locals[i] = V::I(v.wrapping_add(c));
                    next = pc + 3;
                }
                0xBB => {
                    let class = cf.class_name(u16_at(pc + 1)?).ok_or(Abort)?.to_owned();
                    self.objects.push((class, HashMap::new()));
                    st.push(V::Obj(self.objects.len() - 1));
                    next = pc + 3;
                }
                0xC1 => {
                    let target = cf.class_name(u16_at(pc + 1)?).ok_or(Abort)?;
                    let v = pop!();
                    st.push(V::I(
                        ((v == V::This && self.chain.iter().any(|c| c.this_class == target))
                            || matches!(v, V::Sym(_))) as i32,
                    ));
                    next = pc + 3;
                }
                0xB6..=0xB9 => {
                    let (class, name, desc) = cf.member(u16_at(pc + 1)?).ok_or(Abort)?;
                    let nargs = desc_args(desc).len();
                    if st.len() < nargs {
                        return Err(Abort);
                    }
                    let args = st.split_off(st.len() - nargs);
                    next = pc + if op == 0xB9 { 5 } else { 3 };
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
                0x58 => {
                    if !matches!(pop!(), V::L(_) | V::D(_)) {
                        pop!();
                    }
                }
                0x5C => {
                    let a = pop!();
                    if matches!(a, V::L(_) | V::D(_)) {
                        st.extend([a.clone(), a]);
                    } else {
                        let b = pop!();
                        st.extend([b.clone(), a.clone(), b, a]);
                    }
                }
                0x61 | 0x65 | 0x69 | 0x6D | 0x71 | 0x7F | 0x81 | 0x83 => {
                    let (b, a) = (popl!(), popl!());
                    st.push(V::L(match op {
                        0x61 => a.wrapping_add(b),
                        0x65 => a.wrapping_sub(b),
                        0x69 => a.wrapping_mul(b),
                        0x6D => a.checked_div(b).ok_or(Abort)?,
                        0x71 => a.checked_rem(b).ok_or(Abort)?,
                        0x7F => a & b,
                        0x81 => a | b,
                        _ => a ^ b,
                    }));
                }
                0x70 | 0x78 | 0x7A | 0x7C | 0x7E | 0x80 | 0x82 => {
                    let (b, a) = (popi!(), popi!());
                    st.push(V::I(match op {
                        0x70 => a.checked_rem(b).ok_or(Abort)?,
                        0x78 => a.wrapping_shl(b as u32 & 31),
                        0x7A => a >> (b & 31),
                        0x7C => ((a as u32) >> (b & 31)) as i32,
                        0x7E => a & b,
                        0x80 => a | b,
                        _ => a ^ b,
                    }));
                }
                0x79 | 0x7B | 0x7D => {
                    let b = popi!();
                    let a = popl!();
                    st.push(V::L(match op {
                        0x79 => a.wrapping_shl(b as u32 & 63),
                        0x7B => a >> (b & 63),
                        _ => ((a as u64) >> (b & 63)) as i64,
                    }));
                }
                0x73 => {
                    let (b, a) = (popd!(), popd!());
                    st.push(V::D(a % b));
                }
                0x75 => {
                    let a = popl!();
                    st.push(V::L(-a));
                }
                0x85 => {
                    let a = popi!();
                    st.push(V::L(a as i64));
                }
                0x88 => {
                    let a = popl!();
                    st.push(V::I(a as i32));
                }
                0x89 => {
                    let a = popl!();
                    st.push(V::F(a as f32));
                }
                0x8A => {
                    let a = popl!();
                    st.push(V::D(a as f64));
                }
                0x8C => {
                    let a = popf!();
                    st.push(V::L(a as i64));
                }
                0x8F => {
                    let a = popd!();
                    st.push(V::L(a as i64));
                }
                0x91 => {
                    let a = popi!();
                    st.push(V::I(a as i8 as i32));
                }
                0x92 => {
                    let a = popi!();
                    st.push(V::I(a as u16 as i32));
                }
                0x93 => {
                    let a = popi!();
                    st.push(V::I(a as i16 as i32));
                }
                0x94 => {
                    let (b, a) = (popl!(), popl!());
                    st.push(V::I((a > b) as i32 - (a < b) as i32));
                }
                0xA5 | 0xA6 => {
                    let (b, a) = (pop!(), pop!());
                    let take = (a == b) == (op == 0xA5);
                    next = if take {
                        (pc as i64 + i16_at(pc + 1)? as i64) as usize
                    } else {
                        pc + 3
                    };
                }
                0xAA | 0xAB => {
                    let key = popi!();
                    let base = (pc + 4) & !3;
                    let i32_at = |p: usize| -> Result<i32, Abort> {
                        Ok(i32::from_be_bytes([
                            u8_at(p)?,
                            u8_at(p + 1)?,
                            u8_at(p + 2)?,
                            u8_at(p + 3)?,
                        ]))
                    };
                    let default = i32_at(base)?;
                    let off = if op == 0xAA {
                        let (lo, hi) = (i32_at(base + 4)?, i32_at(base + 8)?);
                        if key < lo || key > hi {
                            default
                        } else {
                            i32_at(base + 12 + 4 * (key - lo) as usize)?
                        }
                    } else {
                        let n = i32_at(base + 4)?.max(0) as usize;
                        let mut off = default;
                        for k in 0..n {
                            if i32_at(base + 8 + 8 * k)? == key {
                                off = i32_at(base + 12 + 8 * k)?;
                                break;
                            }
                        }
                        off
                    };
                    next = (pc as i64 + off as i64) as usize;
                }
                0xBF => return Err(why("athrow".into())),
                _ => return Err(why(format!("opcode 0x{op:02X}"))),
            }
            pc = next;
        }
    }

    fn static_call(
        &mut self,
        class: &str,
        name: &str,
        desc: &str,
        args: Vec<V>,
        depth: usize,
    ) -> Result<Option<V>, Abort> {
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
                ("sqrt", "(D)D") => V::D(d(&args[0])?.sqrt()),
                ("toDegrees", "(D)D") => V::D(d(&args[0])?.to_degrees()),
                ("tan", "(D)D") => V::D(d(&args[0])?.tan()),
                ("asin", "(D)D") => V::D(d(&args[0])?.asin()),
                ("acos", "(D)D") => V::D(d(&args[0])?.acos()),
                ("atan", "(D)D") => V::D(d(&args[0])?.atan()),
                ("atan2", "(DD)D") => V::D(d(&args[0])?.atan2(d(&args[1])?)),
                ("pow", "(DD)D") => V::D(d(&args[0])?.powf(d(&args[1])?)),
                ("exp", "(D)D") => V::D(d(&args[0])?.exp()),
                ("log", "(D)D") => V::D(d(&args[0])?.ln()),
                ("log10", "(D)D") => V::D(d(&args[0])?.log10()),
                ("floor", "(D)D") => V::D(d(&args[0])?.floor()),
                ("ceil", "(D)D") => V::D(d(&args[0])?.ceil()),
                ("rint", "(D)D") => V::D(d(&args[0])?.round_ties_even()),
                ("round", "(F)I") => V::I(f(&args[0])?.round() as i32),
                ("round", "(D)J") => V::L(d(&args[0])?.round() as i64),
                ("signum", "(F)F") => V::F(f(&args[0])?.signum()),
                ("signum", "(D)D") => V::D(d(&args[0])?.signum()),
                ("max", "(DD)D") => V::D(d(&args[0])?.max(d(&args[1])?)),
                ("min", "(DD)D") => V::D(d(&args[0])?.min(d(&args[1])?)),
                ("max", "(II)I") | ("min", "(II)I") | ("abs", "(I)I") => {
                    let i = |v: &V| match v {
                        V::I(x) => Ok(*x),
                        _ => Err(Abort),
                    };
                    match name {
                        "max" => V::I(i(&args[0])?.max(i(&args[1])?)),
                        "min" => V::I(i(&args[0])?.min(i(&args[1])?)),
                        _ => V::I(i(&args[0])?.abs()),
                    }
                }
                ("random", "()D") => V::D(0.5),
                _ => return Err(why(format!("Math.{name}{desc}"))),
            };
            return Ok(Some(r));
        }
        // `class$(String)`: the compiler's helper for class literals; keep the name.
        if name == "class$" {
            if let Some(V::Str(n)) = args.first() {
                return Ok(Some(V::Str(n.clone())));
            }
        }
        if name != "cvt" {
            if let Some(r) = self.host_call(short(class), name, desc, &args) {
                return Ok(r);
            }
        }
        if self.record.contains(&name) {
            let vals = args.iter().map(|a| self.export(a, 0)).collect();
            self.recorded.push((name.to_owned(), vals));
            return Ok(None);
        }
        if name == "cvt" && desc == "(FFFFF)F" {
            let x: Vec<f32> = args.iter().map(f).collect::<Result<_, _>>()?;
            let v = x[0].max(x[1]).min(x[2]);
            return Ok(Some(V::F(
                x[3] + (x[4] - x[3]) * (v - x[1]) / (x[2] - x[1]),
            )));
        }
        let Some((cf, m)) = self.resolve(Some(class), name, desc) else {
            if self.world.is_some() || self.host.is_some() {
                return Ok(self.world_value(format!("{}.{name}()", short(class)), ret_of(desc)));
            }
            if self.lenient {
                let ret = &desc[desc.find(')').map_or(desc.len(), |i| i + 1)..];
                return Ok((ret != "V").then(|| default_of(ret)));
            }
            return Err(why(format!("static {class}.{name}{desc}")));
        };
        self.call(cf, m, args, depth + 1)
    }

    /// Runs a class's static initializer once, leniently; statics it sets before any failure are kept.
    fn run_clinit(&mut self, class: &str, depth: usize) {
        if !self.clinit_done.insert(class.to_owned()) {
            return;
        }
        let Some(cf) = self.chain.iter().find(|c| c.this_class == class) else {
            return;
        };
        let Some(m) = cf.method("<clinit>", "()V") else {
            return;
        };
        let (was, budget) = (self.lenient, self.budget);
        self.lenient = true;
        self.budget = 2_000_000;
        let r = self.call(cf, m, Vec::new(), depth + 1);
        if r.is_err() && std::env::var_os("IL2_TRACE_CLINIT").is_some() {
            eprintln!("clinit {class}: {}", last_abort());
        }
        self.lenient = was;
        self.budget = budget;
    }

    #[allow(clippy::too_many_arguments)]
    fn virtual_call(
        &mut self,
        recv: V,
        special: bool,
        class: &str,
        name: &str,
        desc: &str,
        args: Vec<V>,
        depth: usize,
    ) -> Result<Option<V>, Abort> {
        if let V::Str(s) = &recv {
            if let Some(r) = Self::string_call(s, name, desc, &args) {
                return Ok(r);
            }
        }
        let host_recv = match &recv {
            V::Sym(p) => Some(p.clone()),
            V::This => Some("this".to_owned()),
            V::Obj(o) if Some(*o) == self.shot_obj => Some("shot".to_owned()),
            _ => None,
        };
        if let Some(hr) = host_recv.filter(|_| self.host.is_some()) {
            if let Some(r) = self.host_call(&hr, name, desc, &args) {
                return Ok(r);
            }
            if hr == "shot" {
                return Ok((ret_of(desc) != "V").then(|| default_of(ret_of(desc))));
            }
        }
        match recv {
            V::Sym(p) => {
                let strs: Vec<String> = args
                    .iter()
                    .filter_map(|a| match a {
                        V::Str(s) => Some(format!("{s:?}")),
                        _ => None,
                    })
                    .collect();
                Ok(self.world_value(format!("{p}.{name}({})", strs.join(",")), ret_of(desc)))
            }
            V::This if self.world.is_some() && name == "interpPut" => {
                if let Some(a) = args.first() {
                    self.interps.push(a.clone());
                }
                Ok((ret_of(desc) != "V").then(|| default_of(ret_of(desc))))
            }
            // Cockpit code: the engine base classes' constructors are not run.
            V::This
                if self.world.is_some()
                    && name == "<init>"
                    && self.chain.first().is_some_and(|c| c.this_class != class) =>
            {
                Ok(None)
            }
            V::Hier => {
                let node = match args.first() {
                    Some(V::Str(s)) => s.clone(),
                    _ if self.lenient => {
                        return Ok((ret_of(desc) != "V").then(|| default_of(ret_of(desc))));
                    }
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
                        let V::Arr(a) = args[1] else {
                            return Err(Abort);
                        };
                        let v = &self.arrays[a];
                        entry.angles = [v[0], v[1], v[2]];
                    }
                    ("chunkSetLocate", "(Ljava/lang/String;[F[F)V") => {
                        let (V::Arr(x), V::Arr(r)) = (&args[1], &args[2]) else {
                            return Err(Abort);
                        };
                        let (x, r) = (&self.arrays[*x], &self.arrays[*r]);
                        entry.offset = [x[0], x[1], x[2]];
                        entry.angles = [r[0], r[1], r[2]];
                    }
                    ("chunkVisible", "(Ljava/lang/String;Z)V") => {
                        entry.visible = Some(matches!(args[1], V::I(v) if v != 0));
                    }
                    // Assume the named node exists.
                    ("chunkFindCheck", "(Ljava/lang/String;)I") => return Ok(Some(V::I(0))),
                    _ if self.lenient => {
                        return Ok((ret_of(desc) != "V").then(|| default_of(ret_of(desc))))
                    }
                    _ => return Err(why(format!("HierMesh.{name}{desc}"))),
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
                let (cf, m) = self
                    .resolve(special.then_some(class), name, desc)
                    .ok_or_else(|| why(format!("this.{name}{desc}")))?;
                let mut full = vec![V::This];
                full.extend(args);
                self.call(cf, m, full, depth + 1)
            }
            V::Obj(o)
                if matches!(
                    self.objects[o].0.as_str(),
                    "java/lang/StringBuffer" | "java/lang/StringBuilder"
                ) =>
            {
                let text = |m: &Self, v: &V| match v {
                    V::Str(s) => s.clone(),
                    V::I(i) if desc.contains("(C)") => char::from_u32(*i as u32)
                        .map(String::from)
                        .unwrap_or_default(),
                    V::I(i) if desc.contains("(Z)") => (*i != 0).to_string(),
                    V::I(i) => i.to_string(),
                    V::L(l) => l.to_string(),
                    V::F(f) => format!("{f:?}"),
                    V::D(d) => format!("{d:?}"),
                    V::Sym(p) => p.clone(),
                    V::Obj(x) => match m.objects[*x].1.get("$s") {
                        Some(V::Str(s)) => s.clone(),
                        _ => String::new(),
                    },
                    _ => "null".into(),
                };
                let cur = match self.objects[o].1.get("$s") {
                    Some(V::Str(s)) => s.clone(),
                    _ => String::new(),
                };
                match name {
                    "<init>" => {
                        let init = args
                            .first()
                            .filter(|_| desc.contains("String"))
                            .map(|a| text(self, a))
                            .unwrap_or_default();
                        self.objects[o].1.insert("$s".into(), V::Str(init));
                        Ok(None)
                    }
                    "append" => {
                        let add = args.first().map(|a| text(self, a)).unwrap_or_default();
                        self.objects[o].1.insert("$s".into(), V::Str(cur + &add));
                        Ok(Some(recv))
                    }
                    "toString" => Ok(Some(V::Str(cur))),
                    "length" => Ok(Some(V::I(cur.chars().count() as i32))),
                    _ => Err(why(format!("StringBuffer.{name}"))),
                }
            }
            V::Obj(_) => {
                // Constructors and `super.` calls resolve from the referenced class, others from the object's.
                let start = if special {
                    class.to_owned()
                } else {
                    self.class_of(&recv).unwrap_or_else(|| class.to_owned())
                };
                match self.resolve_class(&start, name, desc) {
                    Some((cf, m)) => {
                        let mut full = vec![recv];
                        full.extend(args);
                        self.call(cf, m, full, depth + 1)
                    }
                    // Unknown library classes (java/lang/Object, vectors): constructors do nothing.
                    None if name == "<init>" => Ok(None),
                    None if self.lenient => {
                        let ret = &desc[desc.find(')').map_or(desc.len(), |i| i + 1)..];
                        Ok((ret != "V").then(|| default_of(ret)))
                    }
                    None => Err(why(format!("call {start}.{name}{desc}"))),
                }
            }
            _ if self.lenient => {
                let ret = &desc[desc.find(')').map_or(desc.len(), |i| i + 1)..];
                Ok((ret != "V").then(|| default_of(ret)))
            }
            _ => Err(why(format!("call {class}.{name}{desc} on {recv:?}"))),
        }
    }
}

/// A plain value produced by evaluating property code.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Num(f64),
    Str(String),
    List(Vec<Value>),
    Obj(String, BTreeMap<String, Value>),
}

impl Value {
    pub fn get(&self, field: &str) -> Option<&Value> {
        match self {
            Value::Obj(_, f) => f.get(field),
            _ => None,
        }
    }
    pub fn num(&self, field: &str) -> Option<f64> {
        match self.get(field)? {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn str(&self, field: &str) -> Option<&str> {
        match self.get(field)? {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn list(&self, field: &str) -> &[Value] {
        match self.get(field) {
            Some(Value::List(l)) => l,
            _ => &[],
        }
    }
}

/// Runs the instance method `name desc` (no arguments) of `classes[0]` and returns its result.
/// `classes` holds the class, its superclasses and every other class the code may create.
pub fn evaluate(classes: &[ClassFile], name: &str, desc: &str) -> Result<Value, String> {
    let mut m = Machine {
        chain: classes,
        arrays: vec![vec![0.0; 3], vec![0.0; 3]],
        oarrays: Vec::new(),
        objects: Vec::new(),
        out: BTreeMap::new(),
        budget: 500_000,
        statics: HashMap::new(),
        fields: HashMap::new(),
        clinit_done: HashSet::new(),
        lenient: false,
        record: &[],
        recorded: Vec::new(),
        world: None,
        interps: Vec::new(),
        missing: HashSet::new(),
        pc: 0,
        soft_error: None,
        host: None,
        shot_obj: None,
    };
    let (cf, meth) = m
        .resolve(None, name, desc)
        .ok_or_else(|| format!("no method {name}{desc}"))?;
    match m.call(cf, meth, vec![V::This], 0) {
        Ok(Some(v)) => Ok(m.export(&v, 0)),
        Ok(None) => Ok(Value::Null),
        Err(_) => Err(last_abort()),
    }
}

/// An argument for `call_bool`.
pub enum Arg {
    Int(i32),
    Floats(Vec<f32>),
}

/// Runs instance method `name desc` of `classes[0]` with `args` and returns its boolean result
/// (lenient: unknown calls answer their defaults).
pub fn call_bool(
    classes: &[ClassFile],
    name: &str,
    desc: &str,
    args: &[Arg],
) -> Result<bool, String> {
    let mut m = Machine {
        chain: classes,
        arrays: vec![vec![0.0; 3], vec![0.0; 3]],
        oarrays: Vec::new(),
        objects: Vec::new(),
        out: BTreeMap::new(),
        budget: 100_000,
        statics: HashMap::new(),
        fields: HashMap::new(),
        clinit_done: HashSet::new(),
        lenient: true,
        record: &[],
        recorded: Vec::new(),
        world: None,
        interps: Vec::new(),
        missing: HashSet::new(),
        pc: 0,
        soft_error: None,
        host: None,
        shot_obj: None,
    };
    let (cf, meth) = m
        .resolve(None, name, desc)
        .ok_or_else(|| format!("no method {name}{desc}"))?;
    let mut full = vec![V::This];
    for a in args {
        full.push(match a {
            Arg::Int(i) => V::I(*i),
            Arg::Floats(f) => {
                m.arrays.push(f.clone());
                V::Arr(m.arrays.len() - 1)
            }
        });
    }
    match m.call(cf, meth, full, 0) {
        Ok(Some(V::I(i))) => Ok(i != 0),
        Ok(_) => Err("no boolean result".into()),
        Err(_) => Err(last_abort()),
    }
}

/// Runs the static initializer of `classes[0]` leniently and returns the recorded calls to the
/// static methods named in `record`.
pub fn record_clinit(
    classes: &[ClassFile],
    record: &'static [&'static str],
) -> Vec<(String, Vec<Value>)> {
    let mut m = Machine {
        chain: classes,
        arrays: vec![vec![0.0; 3], vec![0.0; 3]],
        oarrays: Vec::new(),
        objects: Vec::new(),
        out: BTreeMap::new(),
        budget: 2_000_000,
        statics: HashMap::new(),
        fields: HashMap::new(),
        clinit_done: HashSet::new(),
        lenient: true,
        record,
        recorded: Vec::new(),
        world: None,
        interps: Vec::new(),
        missing: HashSet::new(),
        pc: 0,
        soft_error: None,
        host: None,
        shot_obj: None,
    };
    if let Some(c) = classes.first() {
        let name = c.this_class.clone();
        m.run_clinit(&name, 0);
    }
    m.recorded
}

/// Samples one channel of an aircraft; `None` if its methods cannot be evaluated.
pub fn sample(chain: &[ClassFile], ch: Channel) -> Option<Track> {
    const HM: &str = "Lcom/maddox/il2/engine/HierMesh;";
    let (lo, hi) = ch.range();
    let inputs: Vec<f32> = (0..SAMPLES)
        .map(|i| lo + (hi - lo) * i as f32 / (SAMPLES - 1) as f32)
        .collect();
    let mut m = Machine {
        chain,
        arrays: vec![vec![0.0; 3], vec![0.0; 3]],
        oarrays: Vec::new(),
        objects: Vec::new(),
        out: BTreeMap::new(),
        budget: 0,
        statics: HashMap::new(),
        fields: HashMap::new(),
        clinit_done: HashSet::new(),
        lenient: false,
        record: &[],
        recorded: Vec::new(),
        world: None,
        interps: Vec::new(),
        missing: HashSet::new(),
        pc: 0,
        soft_error: None,
        host: None,
        shot_obj: None,
    };
    let mut frames: Vec<BTreeMap<String, NodeState>> = Vec::new();
    let virtual_name = match ch {
        Channel::Rudder => "moveRudder",
        Channel::Elevator => "moveElevator",
        Channel::Aileron => "moveAileron",
        Channel::Flaps => "moveFlap",
        Channel::Arrestor => "moveArrestorHook",
        Channel::WingFold => "moveWingFold",
        Channel::Gear => "",
    };
    // Gear: the static methods of the most derived class that declares them.
    let gear3 = chain.iter().find_map(|c| {
        c.method("moveGear", &format!("({HM}FFF)V"))
            .filter(|m| m.is_static)
            .map(|mm| (c, mm))
    });
    let gear1 = chain.iter().find_map(|c| {
        c.method("moveGear", &format!("({HM}F)V"))
            .filter(|m| m.is_static)
            .map(|mm| (c, mm))
    });
    if ch == Channel::Gear && gear3.is_none() && gear1.is_none() {
        return None;
    }
    // Wing folding: the variant on a HierMesh (the plain one also hides guns and pods).
    let virt_desc = if ch == Channel::WingFold {
        format!("({HM}F)V")
    } else {
        "(F)V".to_owned()
    };
    let virt = (ch != Channel::Gear)
        .then(|| m.resolve(None, virtual_name, &virt_desc))
        .flatten();
    // Only use an override below the generic Aircraft base (whose bodies are empty).
    if ch != Channel::Gear
        && virt.is_none_or(|(c, _)| c.this_class == "com/maddox/il2/objects/air/Aircraft")
    {
        return None;
    }
    for &x in &inputs {
        m.budget = 200_000;
        let ok = if ch == Channel::Gear {
            let mut ok = true;
            for (c, mm) in [gear3, gear1].into_iter().flatten() {
                let args = if mm.desc.contains("FFF") {
                    vec![V::Hier, V::F(x), V::F(x), V::F(x)]
                } else {
                    vec![V::Hier, V::F(x)]
                };
                ok &= m.call(c, mm, args, 0).is_ok();
            }
            ok
        } else {
            let (c, mm) = virt?;
            let args = if ch == Channel::WingFold {
                vec![V::This, V::Hier, V::F(x)]
            } else {
                vec![V::This, V::F(x)]
            };
            m.call(c, mm, args, 0).is_ok()
        };
        if !ok {
            return None;
        }
        frames.push(m.out.clone());
    }
    let mut nodes: BTreeMap<String, Vec<NodeState>> = BTreeMap::new();
    for name in frames.iter().flat_map(|f| f.keys()) {
        if !nodes.contains_key(name) {
            nodes.insert(
                name.clone(),
                frames
                    .iter()
                    .map(|f| f.get(name).copied().unwrap_or_default())
                    .collect(),
            );
        }
    }
    Some(Track {
        channel: ch,
        inputs,
        nodes,
        imported: true,
    })
}

/// Runs the aircraft class's `hitBone(bone, shot, point)` (`docs/behavior/damage.md`, "Bullet
/// hits"): `chain` from the class up, `point` the hit in aircraft coordinates and `dir` the round's
/// direction there (the runtime's `Aircraft.v1`). Everything the code does to the aircraft goes
/// through `host`; `shot` carries the round's energy in and out.
pub fn run_hit_bone(
    chain: &[ClassFile],
    host: &mut dyn Host,
    bone: &str,
    shot: &mut ShotState,
    point: [f64; 3],
    dir: [f64; 3],
) -> Result<(), String> {
    const DESC: &str = "(Ljava/lang/String;Lcom/maddox/il2/ai/Shot;Lcom/maddox/JGP/Point3d;)V";
    let mut m = Machine {
        chain,
        arrays: vec![vec![0.0; 3], vec![0.0; 3]],
        oarrays: Vec::new(),
        objects: Vec::new(),
        out: BTreeMap::new(),
        budget: 300_000,
        statics: HashMap::new(),
        fields: HashMap::new(),
        clinit_done: HashSet::new(),
        lenient: true,
        record: &[],
        recorded: Vec::new(),
        world: None,
        interps: Vec::new(),
        missing: HashSet::new(),
        pc: 0,
        soft_error: None,
        host: Some(host),
        shot_obj: None,
    };
    let vec3 = |m: &mut Machine, class: &str, v: [f64; 3]| {
        let mut f = HashMap::new();
        f.insert("x".to_owned(), V::D(v[0]));
        f.insert("y".to_owned(), V::D(v[1]));
        f.insert("z".to_owned(), V::D(v[2]));
        m.objects.push((class.to_owned(), f));
        V::Obj(m.objects.len() - 1)
    };
    let p = vec3(&mut m, "com/maddox/JGP/Point3d", point);
    let v1 = vec3(&mut m, "com/maddox/JGP/Vector3d", dir);
    m.statics
        .insert("com/maddox/il2/objects/air/Aircraft.v1".into(), v1);
    let shot_p = vec3(&mut m, "com/maddox/JGP/Point3d", point);
    let shot_v = vec3(&mut m, "com/maddox/JGP/Vector3d", dir);
    let mut f = HashMap::new();
    f.insert("power".to_owned(), V::F(shot.power as f32));
    f.insert("mass".to_owned(), V::F(shot.mass as f32));
    f.insert("powerType".to_owned(), V::I(shot.power_type));
    f.insert("initiator".to_owned(), V::Sym("initiator".into()));
    f.insert("p".to_owned(), shot_p);
    f.insert("v".to_owned(), shot_v);
    f.insert("tickOffset".to_owned(), V::F(0.0));
    m.objects.push(("com/maddox/il2/ai/Shot".to_owned(), f));
    let shot_obj = m.objects.len() - 1;
    m.shot_obj = Some(shot_obj);
    m.fields.insert("FM".into(), V::Sym("FM".into()));
    let (cf, meth) = m
        .resolve(None, "hitBone", DESC)
        .ok_or_else(|| "no hitBone".to_owned())?;
    let r = m.call(
        cf,
        meth,
        vec![V::This, V::Str(bone.to_owned()), V::Obj(shot_obj), p],
        0,
    );
    *shot = m.shot_state();
    r.map(|_| ()).map_err(|_| last_abort())
}

/// Whether the class (or a superclass short of `Aircraft`) overrides `hitBone`.
pub fn has_hit_bone(chain: &[ClassFile]) -> bool {
    const DESC: &str = "(Ljava/lang/String;Lcom/maddox/il2/ai/Shot;Lcom/maddox/JGP/Point3d;)V";
    chain
        .iter()
        .take_while(|c| c.this_class != "com/maddox/il2/objects/air/Aircraft")
        .any(|c| c.method("hitBone", DESC).is_some())
}

/// A cockpit's instrument code, run against the simulation (`docs/behavior/cockpit.md`,
/// "Instruments").
///
/// The cockpit object is constructed once (its own constructor and field initializers; the
/// engine's base constructors are skipped). Each update runs the interpolators it registered and
/// then `reflectWorldToInstruments`. Everything the code reads from the flight model or the world
/// is symbolic: a read of `this.fm.EI.engines[0].getRPM()` yields the number stored under that
/// path in the table passed to `update`.
pub struct Instruments {
    m: Machine<'static>,
    reflect: Option<(&'static ClassFile, &'static Method)>,
    /// Why the last update stopped early, if it did.
    pub error: Option<String>,
}

impl Instruments {
    /// `pool` holds the cockpit class and its superclasses first, then any helper classes.
    pub fn new(pool: Vec<ClassFile>) -> Option<Self> {
        // The machine borrows its classes for its whole life; cockpits are few, so leak them.
        let chain: &'static [ClassFile] = Box::leak(pool.into_boxed_slice());
        let mut m = Machine {
            chain,
            arrays: vec![vec![0.0; 3], vec![0.0; 3]],
            oarrays: Vec::new(),
            objects: Vec::new(),
            out: BTreeMap::new(),
            budget: 2_000_000,
            statics: HashMap::new(),
            fields: HashMap::from([
                ("fm".to_owned(), V::Sym("fm".into())),
                ("mesh".to_owned(), V::Hier),
            ]),
            clinit_done: HashSet::new(),
            lenient: true,
            record: &[],
            recorded: Vec::new(),
            world: Some(HashMap::new()),
            interps: Vec::new(),
            missing: HashSet::new(),
            pc: 0,
            soft_error: None,
            host: None,
            shot_obj: None,
        };
        let first = chain.first()?;
        m.run_clinit(&first.this_class, 0);
        let error = match first.method("<init>", "()V") {
            Some(init) => m
                .call(first, init, vec![V::This], 0)
                .err()
                .map(|_| last_abort()),
            None => Some("no constructor".into()),
        };
        // Whatever the constructor did, these are the flight model and the cockpit mesh.
        m.fields.insert("fm".into(), V::Sym("fm".into()));
        m.fields.insert("mesh".into(), V::Hier);
        let reflect = m.resolve(None, "reflectWorldToInstruments", "(F)V");
        Some(Instruments { m, reflect, error })
    }

    /// Runs `ticks` interpolator steps and one instrument update against `world`, and returns the
    /// state of every node the cockpit code has set so far.
    pub fn update(
        &mut self,
        world: HashMap<String, f64>,
        ticks: usize,
    ) -> &BTreeMap<String, NodeState> {
        let m = &mut self.m;
        m.world = Some(world);
        self.error = None;
        for _ in 0..ticks {
            for k in 0..m.interps.len() {
                let obj = m.interps[k].clone();
                let Some(class) = m.class_of(&obj) else {
                    continue;
                };
                if let Some((cf, meth)) = m.resolve_class(&class, "tick", "()Z") {
                    m.budget = 200_000;
                    if m.call(cf, meth, vec![obj], 0).is_err() {
                        self.error = Some(format!("{class}.tick: {}", last_abort()));
                    }
                }
            }
        }
        if let Some((cf, meth)) = self.reflect {
            m.budget = 400_000;
            if m.call(cf, meth, vec![V::This, V::F(1.0)], 0).is_err() {
                self.error = Some(format!("reflectWorldToInstruments: {}", last_abort()));
            }
        }
        &m.out
    }

    /// The first failure inside a helper call that was skipped.
    pub fn soft_error(&self) -> Option<&str> {
        self.m.soft_error.as_deref()
    }

    /// Paths the code read that the last tables did not have (for wiring up new values).
    pub fn missing(&self) -> Vec<String> {
        let mut v: Vec<String> = self.m.missing.iter().cloned().collect();
        v.sort();
        v
    }
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
