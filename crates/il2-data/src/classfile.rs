//! Minimal Java class-file reader, used only to extract data (key tables, asset names, animation
//! curves) from the user's own installed runtime classes.

use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("class file: {0}")]
    Bad(&'static str),
    #[error("class file: unsupported opcode {0:#04x} in <clinit>")]
    Opcode(u8),
}

#[derive(Clone, Debug)]
pub enum Const {
    Utf8(String),
    Int(i32),
    Float(f32),
    Long(i64),
    Double(f64),
    Class(u16),
    Str(u16),
    /// class, name_and_type
    FieldRef(u16, u16),
    MethodRef(u16, u16),
    /// name, descriptor
    NameAndType(u16, u16),
    Other,
}

#[derive(Clone, Debug)]
pub struct Method {
    pub name: String,
    pub desc: String,
    pub is_static: bool,
    pub code: Vec<u8>,
    pub max_locals: usize,
}

struct Reader<'a> {
    d: &'a [u8],
    p: usize,
}

impl Reader<'_> {
    fn bytes(&mut self, n: usize) -> Result<&[u8], Error> {
        let s = self
            .d
            .get(self.p..self.p + n)
            .ok_or(Error::Bad("truncated"))?;
        self.p += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.bytes(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, Error> {
        let b = self.bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32, Error> {
        let b = self.bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
}

pub struct ClassFile {
    pub consts: Vec<Const>,
    pub this_class: String,
    pub super_class: Option<String>,
    /// Implemented interfaces (`com/maddox/il2/objects/air/TypeFighter`, …).
    pub interfaces: Vec<String>,
    pub methods: Vec<Method>,
}

impl ClassFile {
    pub fn parse(d: &[u8]) -> Result<Self, Error> {
        let mut r = Reader { d, p: 0 };
        if r.u32()? != 0xCAFE_BABE {
            return Err(Error::Bad("magic"));
        }
        r.bytes(4)?;
        let n = r.u16()? as usize;
        let mut consts = Vec::with_capacity(n);
        consts.push(Const::Other);
        while consts.len() < n {
            let tag = r.u8()?;
            let c = match tag {
                1 => {
                    let len = r.u16()? as usize;
                    Const::Utf8(String::from_utf8_lossy(r.bytes(len)?).into_owned())
                }
                3 => Const::Int(r.u32()? as i32),
                4 => Const::Float(f32::from_bits(r.u32()?)),
                5 | 6 => {
                    let hi = r.u32()? as u64;
                    let lo = r.u32()? as u64;
                    let v = hi << 32 | lo;
                    // Value in the first slot, the second slot is unusable.
                    consts.push(if tag == 5 {
                        Const::Long(v as i64)
                    } else {
                        Const::Double(f64::from_bits(v))
                    });
                    Const::Other
                }
                7 => Const::Class(r.u16()?),
                8 => Const::Str(r.u16()?),
                16 => {
                    r.u16()?;
                    Const::Other
                }
                9 => Const::FieldRef(r.u16()?, r.u16()?),
                10 | 11 => Const::MethodRef(r.u16()?, r.u16()?),
                18 => {
                    r.bytes(4)?;
                    Const::Other
                }
                12 => Const::NameAndType(r.u16()?, r.u16()?),
                15 => {
                    r.bytes(3)?;
                    Const::Other
                }
                _ => return Err(Error::Bad("constant pool tag")),
            };
            consts.push(c);
        }
        r.u16()?; // access
        let this_idx = r.u16()?;
        let super_idx = r.u16()?;
        let ifaces = r.u16()? as usize;
        let mut iface_idx = Vec::with_capacity(ifaces);
        for _ in 0..ifaces {
            iface_idx.push(r.u16()?);
        }
        let skip_attrs = |r: &mut Reader| -> Result<(), Error> {
            for _ in 0..r.u16()? {
                r.u16()?;
                let len = r.u32()? as usize;
                r.bytes(len)?;
            }
            Ok(())
        };
        for _ in 0..r.u16()? {
            r.bytes(6)?;
            skip_attrs(&mut r)?;
        }
        let mut cf = ClassFile {
            consts,
            this_class: String::new(),
            super_class: None,
            interfaces: Vec::new(),
            methods: Vec::new(),
        };
        cf.interfaces = iface_idx
            .iter()
            .filter_map(|&i| cf.class_name(i).map(str::to_owned))
            .collect();
        cf.this_class = cf.class_name(this_idx).unwrap_or_default().to_owned();
        cf.super_class = cf.class_name(super_idx).map(str::to_owned);
        for _ in 0..r.u16()? {
            let access = r.u16()?;
            let name = cf.utf8(r.u16()?).unwrap_or_default().to_owned();
            let desc = cf.utf8(r.u16()?).unwrap_or_default().to_owned();
            let mut code = Vec::new();
            let mut max_locals = 0;
            for _ in 0..r.u16()? {
                let aname = cf.utf8(r.u16()?).map(str::to_owned);
                let len = r.u32()? as usize;
                let body = r.bytes(len)?;
                if aname.as_deref() == Some("Code") {
                    let mut c = Reader { d: body, p: 2 };
                    max_locals = c.u16()? as usize;
                    let code_len = c.u32()? as usize;
                    code = c.bytes(code_len)?.to_vec();
                }
            }
            cf.methods.push(Method {
                name,
                desc,
                is_static: access & 0x0008 != 0,
                code,
                max_locals,
            });
        }
        Ok(cf)
    }

    pub fn utf8(&self, i: u16) -> Option<&str> {
        match self.consts.get(i as usize) {
            Some(Const::Utf8(s)) => Some(s),
            _ => None,
        }
    }

    pub fn class_name(&self, i: u16) -> Option<&str> {
        match self.consts.get(i as usize) {
            Some(Const::Class(n)) => self.utf8(*n),
            _ => None,
        }
    }

    /// (class, name, descriptor) of a field or method reference.
    pub fn member(&self, i: u16) -> Option<(&str, &str, &str)> {
        let (c, nt) = match self.consts.get(i as usize)? {
            Const::FieldRef(c, nt) | Const::MethodRef(c, nt) => (*c, *nt),
            _ => return None,
        };
        let Const::NameAndType(n, d) = self.consts.get(nt as usize)? else {
            return None;
        };
        Some((self.class_name(c)?, self.utf8(*n)?, self.utf8(*d)?))
    }

    pub fn method(&self, name: &str, desc: &str) -> Option<&Method> {
        self.methods
            .iter()
            .find(|m| m.name == name && m.desc == desc)
    }

    /// All UTF-8 string constants.
    pub fn strings(&self) -> impl Iterator<Item = &str> {
        self.consts.iter().filter_map(|c| match c {
            Const::Utf8(s) => Some(s.as_str()),
            _ => None,
        })
    }

    /// Evaluates `<clinit>` far enough to collect static fields assigned constant int/byte arrays.
    /// Calls to static `()V` methods are skipped; any other opcode is an error.
    pub fn static_arrays(&self) -> Result<HashMap<String, Vec<i32>>, Error> {
        #[derive(Clone)]
        enum V {
            Int(i32),
            Arr(usize),
        }
        let code = self
            .method("<clinit>", "()V")
            .map(|m| m.code.as_slice())
            .ok_or(Error::Bad("no <clinit>"))?;
        let mut arrays: Vec<Vec<i32>> = Vec::new();
        let mut stack: Vec<V> = Vec::new();
        let mut out = HashMap::new();
        let mut pc = 0;
        let pop_int = |s: &mut Vec<V>| match s.pop() {
            Some(V::Int(i)) => Ok(i),
            _ => Err(Error::Bad("stack")),
        };
        let u16_at = |pc: usize| -> Result<u16, Error> {
            code.get(pc..pc + 2)
                .map(|b| u16::from_be_bytes([b[0], b[1]]))
                .ok_or(Error::Bad("truncated code"))
        };
        while pc < code.len() {
            let op = code[pc];
            match op {
                0x02..=0x08 => stack.push(V::Int(op as i32 - 3)),
                0x10 => {
                    stack.push(V::Int(
                        *code.get(pc + 1).ok_or(Error::Bad("truncated code"))? as i8 as i32,
                    ));
                    pc += 1;
                }
                0x11 => {
                    stack.push(V::Int(u16_at(pc + 1)? as i16 as i32));
                    pc += 2;
                }
                0x12 | 0x13 => {
                    let idx = if op == 0x12 {
                        *code.get(pc + 1).ok_or(Error::Bad("truncated code"))? as u16
                    } else {
                        u16_at(pc + 1)?
                    };
                    match self.consts.get(idx as usize) {
                        Some(Const::Int(i)) => stack.push(V::Int(*i)),
                        _ => return Err(Error::Bad("non-int ldc")),
                    }
                    pc += if op == 0x12 { 1 } else { 2 };
                }
                0xBC => {
                    let n = pop_int(&mut stack)?;
                    if !(0..=1 << 20).contains(&n) {
                        return Err(Error::Bad("array size"));
                    }
                    arrays.push(vec![0; n as usize]);
                    stack.push(V::Arr(arrays.len() - 1));
                    pc += 1;
                }
                0x59 => {
                    let top = stack.last().cloned().ok_or(Error::Bad("stack"))?;
                    stack.push(top);
                }
                0x4F | 0x54 => {
                    let v = pop_int(&mut stack)?;
                    let i = pop_int(&mut stack)?;
                    let Some(V::Arr(a)) = stack.pop() else {
                        return Err(Error::Bad("stack"));
                    };
                    let slot = arrays[a].get_mut(i as usize).ok_or(Error::Bad("index"))?;
                    *slot = if op == 0x54 { v as i8 as i32 } else { v };
                }
                0xB3 => {
                    let name = self
                        .member(u16_at(pc + 1)?)
                        .ok_or(Error::Bad("field ref"))?
                        .1
                        .to_owned();
                    if let Some(V::Arr(a)) = stack.pop() {
                        out.insert(name, arrays[a].clone());
                    }
                    pc += 2;
                }
                0xB8 => pc += 2,
                0xB1 => break,
                _ => return Err(Error::Opcode(op)),
            }
            pc += 1;
        }
        Ok(out)
    }
}
