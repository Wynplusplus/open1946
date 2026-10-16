//! Java `.properties` text (`i18n/*.properties`, mission briefings): `key value` or `key=value`
//! lines, `#`/`!` comments, `\` line continuations and `\n`, `\t`, `\uXXXX` escapes. The bytes are
//! ISO-8859-1.

use std::collections::HashMap;

pub fn parse(data: &[u8]) -> HashMap<String, String> {
    let text: String = data.iter().map(|&b| b as char).collect();
    let mut out = HashMap::new();
    let mut lines = text.lines();
    while let Some(first) = lines.next() {
        let mut line = first.trim_start().to_owned();
        if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }
        // Continuation: an odd number of trailing backslashes.
        while line.chars().rev().take_while(|&c| c == '\\').count() % 2 == 1 {
            line.pop();
            match lines.next() {
                Some(next) => line.push_str(next.trim_start()),
                None => break,
            }
        }
        let mut key = String::new();
        let mut chars = line.chars().peekable();
        while let Some(&c) = chars.peek() {
            if c == '\\' {
                chars.next();
                if let Some(e) = chars.next() {
                    key.push(e);
                }
                continue;
            }
            if c.is_whitespace() || c == '=' || c == ':' {
                break;
            }
            key.push(c);
            chars.next();
        }
        let rest: String = chars.collect();
        let rest = rest.trim_start();
        let rest = rest.strip_prefix(['=', ':']).unwrap_or(rest).trim_start();
        out.insert(key, unescape(rest));
    }
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => {}
            Some('u') => {
                let hex: String = it.by_ref().take(4).collect();
                if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    out.push(ch);
                }
            }
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn keys_values_escapes() {
        let p = super::parse(
            b"# c\nName Bf-109F-4 N1\nShort A\\nB\nx=1\nlong one \\\n  two\nu \\u00e9t\\u00e9\n",
        );
        assert_eq!(p["Name"], "Bf-109F-4 N1");
        assert_eq!(p["Short"], "A\nB");
        assert_eq!(p["x"], "1");
        assert_eq!(p["long"], "one two");
        assert_eq!(p["u"], "été");
    }
}
