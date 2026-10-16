//! Section files: `docs/formats/sectfile.md`.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Section {
    pub name: String,
    pub entries: Vec<(String, String)>,
}

impl Section {
    /// First value for `key`.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// First value for `key`, matched like the engine's native reader: ASCII case-insensitive, and
    /// `key=value` entries are accepted.
    pub fn get_ci(&self, key: &str) -> Option<&str> {
        self.entries.iter().find_map(|(k, v)| {
            if k.eq_ignore_ascii_case(key) {
                Some(v.as_str())
            } else {
                let (kk, vv) = k.split_once('=')?;
                kk.eq_ignore_ascii_case(key).then_some(vv)
            }
        })
    }

    pub fn get_all<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.entries
            .iter()
            .filter(move |(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn get_f32(&self, key: &str) -> Option<f32> {
        self.get(key)?.split_whitespace().next()?.parse().ok()
    }

    pub fn get_i32(&self, key: &str) -> Option<i32> {
        self.get(key)?.split_whitespace().next()?.parse().ok()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SectFile {
    pub sections: Vec<Section>,
}

/// Parses the longest numeric prefix of `s` like C `atof` (`"0.12'"` gives `0.12`, `"x"` gives `None`).
pub fn atof(s: &str) -> Option<f32> {
    let s = s.trim_start();
    let b = s.as_bytes();
    let mut end = 0;
    let digits = |i: &mut usize| {
        let st = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        *i > st
    };
    if end < b.len() && matches!(b[end], b'+' | b'-') {
        end += 1;
    }
    let mut any = digits(&mut end);
    if end < b.len() && b[end] == b'.' {
        end += 1;
        any |= digits(&mut end);
    }
    if !any {
        return None;
    }
    if end < b.len() && matches!(b[end], b'e' | b'E') {
        let mut e = end + 1;
        if e < b.len() && matches!(b[e], b'+' | b'-') {
            e += 1;
        }
        if digits(&mut e) {
            end = e;
        }
    }
    s[..end].parse().ok()
}

/// Decodes 8-bit text. Windows-1252 differs from Latin-1 only in 0x80..0x9F, which never
/// matters for the syntax; those bytes are mapped as Latin-1.
pub fn decode_8bit(data: &[u8]) -> String {
    data.iter().map(|&b| b as char).collect()
}

fn strip_comment(line: &str) -> &str {
    let mut prev_slash = false;
    for (i, c) in line.char_indices() {
        if c == '\t' {
            prev_slash = false;
            continue;
        }
        if (c as u32) < 0x20 || c == '#' || c == ';' {
            return &line[..i];
        }
        if c == '/' {
            if prev_slash {
                return &line[..i - 1];
            }
            prev_slash = true;
        } else {
            prev_slash = false;
        }
    }
    line
}

fn is_blank(c: char) -> bool {
    c as u32 <= 0x20
}

impl SectFile {
    pub fn parse(data: &[u8]) -> Self {
        Self::parse_str(&decode_8bit(data), true)
    }

    pub fn parse_str(text: &str, comments: bool) -> Self {
        let mut f = SectFile::default();
        for raw in text.split('\n') {
            let raw = raw.strip_suffix('\r').unwrap_or(raw);
            let mut line = raw.trim_start_matches(is_blank);
            if comments {
                line = strip_comment(line);
            }
            let line = line.trim_end_matches(is_blank);
            if line.is_empty() {
                continue;
            }
            if line.len() > 2 && line.starts_with('[') && line.ends_with(']') {
                f.sections.push(Section {
                    name: line[1..line.len() - 1].to_owned(),
                    entries: Vec::new(),
                });
            } else if line.starts_with('[') && line.ends_with(']') {
                // "[]" is neither a section nor an entry.
            } else if let Some(sect) = f.sections.last_mut() {
                let (key, value) = line.split_once(is_blank).unwrap_or((line, ""));
                sect.entries.push((
                    key.to_owned(),
                    value.trim_start_matches(is_blank).to_owned(),
                ));
            }
        }
        f
    }

    /// First section with this name.
    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name == name)
    }

    /// First section with this name, ASCII case-insensitive.
    pub fn section_ci(&self, name: &str) -> Option<&Section> {
        self.sections
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.section(section)?.get(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sections_and_comments() {
        let f = SectFile::parse(
            b"junk before\r\n[General]\r\n  tfDoubleSide 0\r\n\tAlphaTestVal 0.5   // of [0.1]\r\n\
              # comment\r\nPaletteName\r\nurl http:x ; tail\r\n[ ]\r\n[Two Words]\r\nk  v1  v2  \r\n",
        );
        assert_eq!(f.sections.len(), 3);
        let g = &f.sections[0];
        assert_eq!(g.get("tfDoubleSide"), Some("0"));
        assert_eq!(g.get_f32("AlphaTestVal"), Some(0.5));
        assert_eq!(g.get("PaletteName"), Some(""));
        assert_eq!(g.get("url"), Some("http:x"));
        assert_eq!(f.sections[1].name, " ");
        assert_eq!(f.get("Two Words", "k"), Some("v1  v2"));
    }

    #[test]
    fn atof_prefix() {
        assert_eq!(atof("0.12'"), Some(0.12));
        assert_eq!(atof("-4.12e-008"), Some(-4.12e-8));
        assert_eq!(atof("5."), Some(5.0));
        assert_eq!(atof("1e"), Some(1.0));
        assert_eq!(atof("x1"), None);
    }

    #[test]
    fn single_slash_is_not_a_comment() {
        let f = SectFile::parse(b"[a]\npath 3do/plane/x\n");
        assert_eq!(f.get("a", "path"), Some("3do/plane/x"));
    }
}
