//! Row-oriented view of section files, covering both the binary encoding (`docs/formats/binsect.md`)
//! and text files (`docs/formats/sectfile.md`).

use crate::sectfile::SectFile;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i32),
    Float(f32),
    Str(String),
}

impl Value {
    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Value::Int(i) => Some(*i as f32),
            Value::Float(f) => Some(*f),
            Value::Str(s) => s.parse().ok(),
        }
    }

    pub fn as_i32(&self) -> Option<i32> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Float(f) if f.fract() == 0.0 => Some(*f as i32),
            Value::Float(_) => None,
            Value::Str(s) => s.parse().ok(),
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RowSection {
    /// Name without brackets.
    pub name: String,
    pub rows: Vec<Vec<Value>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RowFile {
    pub sections: Vec<RowSection>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("binary section file is corrupt: {0}")]
    Corrupt(&'static str),
}

const HEADER: usize = 28;

fn u32_at(d: &[u8], off: usize) -> Result<u32, Error> {
    d.get(off..off + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).ok_or(Error::Corrupt("truncated"))
}

pub fn is_binary(data: &[u8]) -> bool {
    data.len() > HEADER && data[..4] == [0x01, b'B', b'S', 0x00]
}

impl RowFile {
    pub fn section(&self, name: &str) -> Option<&RowSection> {
        self.sections.iter().find(|s| s.name == name)
    }

    /// Parses either encoding.
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        if is_binary(data) {
            Self::parse_binary(data)
        } else {
            Ok(Self::from_text(&SectFile::parse(data)))
        }
    }

    /// Splits text entries into rows of numbers and strings.
    pub fn from_text(f: &SectFile) -> Self {
        let parse = |t: &str| {
            if let Ok(i) = t.parse::<i32>() {
                Value::Int(i)
            } else if let Ok(x) = t.parse::<f32>() {
                Value::Float(x)
            } else {
                Value::Str(t.to_owned())
            }
        };
        RowFile {
            sections: f
                .sections
                .iter()
                .map(|s| RowSection {
                    name: s.name.clone(),
                    rows: s
                        .entries
                        .iter()
                        .map(|(k, v)| std::iter::once(k.as_str()).chain(v.split_whitespace()).map(parse).collect())
                        .collect(),
                })
                .collect(),
        }
    }

    pub fn parse_binary(d: &[u8]) -> Result<Self, Error> {
        if !is_binary(d) {
            return Err(Error::Corrupt("missing header"));
        }
        let size = u32_at(d, 4)? as usize;
        let nsec = u32_at(d, 8)? as usize;
        let nlines = u32_at(d, 12)? as usize;
        let sec_off = u32_at(d, 16)? as usize;
        let line_off = u32_at(d, 24)? as usize;
        if size != d.len() || nsec == 0 {
            return Err(Error::Corrupt("header"));
        }
        let mut offs = Vec::with_capacity(nlines + 1);
        let mut acc = 0u32;
        for i in 0..=nlines {
            acc = acc.wrapping_add(u32_at(d, line_off + 4 * i)?);
            offs.push(acc as usize);
        }
        if offs.last() != Some(&size) || offs.windows(2).any(|w| w[0] > w[1]) {
            return Err(Error::Corrupt("line table"));
        }
        let mut sections = Vec::with_capacity(nsec);
        for i in 0..nsec {
            let e = sec_off + 12 * i;
            let name_off = u32_at(d, e)? as usize;
            let first = u32_at(d, e + 4)? as usize;
            let count = u16::from_le_bytes([d[e + 8], d[e + 9]]) as usize;
            let (typ, delta) = (d[e + 10], d[e + 11] != 0);
            let n = *d.get(name_off).ok_or(Error::Corrupt("section name"))? as usize;
            let raw_name = d.get(name_off + 1..name_off + 1 + n).ok_or(Error::Corrupt("section name"))?;
            let raw_name = raw_name.strip_suffix(&[0]).unwrap_or(raw_name);
            // The name runs to the first ']'; some files carry junk after it ("[LOD] 500").
            let name = raw_name
                .strip_prefix(b"[")
                .and_then(|s| s.split(|&b| b == b']').next())
                .ok_or(Error::Corrupt("section name brackets"))?;
            let name: String = name.iter().map(|&b| b as char).collect();
            if first + count > nlines {
                return Err(Error::Corrupt("section lines"));
            }
            let base = offs[first];
            let mut data = d[base..offs[first + count]].to_vec();
            if delta {
                match typ {
                    1 => {
                        for k in 1..data.len() {
                            data[k] = data[k].wrapping_add(data[k - 1]);
                        }
                    }
                    2 => {
                        let mut s = 0i16;
                        for c in data.chunks_exact_mut(2) {
                            s = s.wrapping_add(i16::from_le_bytes([c[0], c[1]]));
                            c.copy_from_slice(&s.to_le_bytes());
                        }
                    }
                    _ => return Err(Error::Corrupt("delta flag on non-integer section")),
                }
            }
            let mut rows = Vec::with_capacity(count);
            for l in first..first + count {
                let line = &data[offs[l] - base..offs[l + 1] - base];
                rows.push(decode_row(line, typ)?);
            }
            sections.push(RowSection { name, rows });
        }
        Ok(RowFile { sections })
    }
}

fn decode_row(line: &[u8], typ: u8) -> Result<Vec<Value>, Error> {
    Ok(match typ {
        0 => {
            let mut row = Vec::new();
            let mut p = 0;
            while p < line.len() {
                let t = line[p] as usize;
                let get = |a: usize, b: usize| line.get(a..b).ok_or(Error::Corrupt("token"));
                match t {
                    0 => {
                        let b = get(p + 1, p + 3)?;
                        row.push(Value::Int(i16::from_le_bytes([b[0], b[1]]) as i32));
                        p += 3;
                    }
                    1 => {
                        let b = get(p + 1, p + 5)?;
                        row.push(Value::Float(f32::from_le_bytes(b.try_into().unwrap())));
                        p += 5;
                    }
                    n => {
                        row.push(Value::Str(get(p + 1, p + n)?.iter().map(|&b| b as char).collect()));
                        p += n;
                    }
                }
            }
            row
        }
        1 => line.iter().map(|&b| Value::Int(b as i32)).collect(),
        2 => line.chunks_exact(2).map(|c| Value::Int(i16::from_le_bytes([c[0], c[1]]) as i32)).collect(),
        3 => line
            .chunks_exact(3)
            .map(|c| Value::Float((i32::from_le_bytes([0, c[0], c[1], c[2]]) >> 8) as f32 / 65536.0))
            .collect(),
        4 => line.chunks_exact(4).map(|c| Value::Float(f32::from_le_bytes(c.try_into().unwrap()))).collect(),
        _ => return Err(Error::Corrupt("unknown column type")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a binary section file from (name, type, delta, lines).
    fn build(sections: &[(&str, u8, bool, Vec<Vec<u8>>)]) -> Vec<u8> {
        let mut names = Vec::new();
        let mut name_offs = Vec::new();
        for (n, ..) in sections {
            name_offs.push(HEADER + names.len());
            let s = format!("[{n}]");
            names.push(s.len() as u8 + 1);
            names.extend(s.bytes());
            names.push(0);
        }
        let sec_off = HEADER + names.len();
        let nlines: usize = sections.iter().map(|s| s.3.len()).sum();
        let line_off = sec_off + 12 * sections.len();
        let data_off = line_off + 4 * (nlines + 1);
        let mut table = Vec::new();
        let mut lines: Vec<u8> = Vec::new();
        let mut first = 0u32;
        let mut prev = 0u32;
        let mut deltas = Vec::new();
        for (i, (_, t, d, ls)) in sections.iter().enumerate() {
            table.extend((name_offs[i] as u32).to_le_bytes());
            table.extend(first.to_le_bytes());
            table.extend((ls.len() as u16).to_le_bytes());
            table.extend([*t, *d as u8]);
            for l in ls {
                let off = (data_off + lines.len()) as u32;
                deltas.push(off - prev);
                prev = off;
                lines.extend(l);
            }
            first += ls.len() as u32;
        }
        let size = (data_off + lines.len()) as u32;
        deltas.push(size - prev);
        let mut f = vec![0x01, b'B', b'S', 0];
        for v in [size, sections.len() as u32, nlines as u32, sec_off as u32, 0, line_off as u32] {
            f.extend(v.to_le_bytes());
        }
        f.extend(names);
        f.extend(table);
        for d in deltas {
            f.extend(d.to_le_bytes());
        }
        f.extend(lines);
        f
    }

    #[test]
    fn decodes_all_column_types() {
        let data = build(&[
            ("Common", 0, false, vec![vec![9, b'N', b'u', b'm', b'B', b'o', b'n', b'e', b's', 0, 5, 0, 1, 0, 0, 0xC0, 0x3F]]),
            ("D", 2, true, vec![vec![0x36, 0x03, 0xCC, 0xFF], vec![0xFE, 0xFC, 0, 0]]),
            ("U", 3, false, vec![vec![0xCF, 0xC2, 0x00, 0x00, 0x00, 0xFF]]),
            ("F", 4, false, vec![2.5f32.to_le_bytes().to_vec()]),
        ]);
        let f = RowFile::parse(&data).unwrap();
        assert_eq!(
            f.section("Common").unwrap().rows[0],
            [Value::Str("NumBones".into()), Value::Int(5), Value::Float(1.5)]
        );
        assert_eq!(f.section("D").unwrap().rows, [vec![Value::Int(822), Value::Int(770)], vec![Value::Int(0), Value::Int(0)]]);
        let u = &f.section("U").unwrap().rows[0];
        assert_eq!(u[0], Value::Float(0x00C2CF as f32 / 65536.0));
        assert_eq!(u[1], Value::Float(-1.0));
        assert_eq!(f.section("F").unwrap().rows[0], [Value::Float(2.5)]);
    }

    #[test]
    fn text_rows() {
        let f = RowFile::parse(b"[FaceGroups]\n822 770\n0 0 92 0 52 0\n[Materials]\nGloss1D0o\n").unwrap();
        assert_eq!(f.section("FaceGroups").unwrap().rows[1][2], Value::Int(92));
        assert_eq!(f.section("Materials").unwrap().rows[0][0], Value::Str("Gloss1D0o".into()));
    }
}
