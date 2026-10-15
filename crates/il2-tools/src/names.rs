//! Recovers archive path names. Archives store only path hashes, so names are found by
//! scanning file contents for path-like tokens, resolving them against known directories, and
//! confirming candidates by hash.

use il2_sfs::path_hash;
use il2_vfs::Vfs;
use std::collections::{HashMap, HashSet};

/// Files larger than this are not scanned for tokens (terrain rasters, sounds).
const MAX_SCAN: u32 = 4 << 20;

/// Extensions tried for bare names (e.g. `Mesh CF_D0` refers to `CF_D0.msh`).
const EXTENSIONS: &[&str] = &[
    "msh", "mat", "tga", "tgb", "bmp", "him", "sim", "ini", "txt", "wav", "ogg", "prs", "eff", "properties",
];

pub struct Names {
    pub found: HashMap<u64, String>,
}

fn is_path_char(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(b, b'_' | b'-' | b'.' | b'/' | b'\\' | b'$' | b'+' | b'#' | b'!' | b'~' | b'(' | b')' | b'&' | b'\'')
}

/// Path-like tokens in arbitrary (text or binary) data, with `/` separators.
fn tokens(data: &[u8]) -> impl Iterator<Item = String> + '_ {
    data.split(|&b| !is_path_char(b)).filter_map(|t| {
        if t.len() < 2 || t.len() > 200 {
            return None;
        }
        let s = String::from_utf8_lossy(t).replace('\\', "/");
        Some(s.trim_matches(['/']).trim_end_matches('.').to_owned()).filter(|s| s.len() >= 2)
    })
}

/// Joins `rel` onto directory `dir` (which ends in `/` or is empty), resolving `.` and `..`.
fn join(dir: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    for p in rel.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

fn parent_dirs(path: &str) -> impl Iterator<Item = &str> {
    path.char_indices().filter(|&(_, c)| c == '/').map(move |(i, _)| &path[..=i])
}

fn has_extension(t: &str) -> bool {
    t.rsplit('/').next().is_some_and(|n| n.contains('.'))
}

struct Crawl<'a> {
    total: HashSet<u64>,
    found: HashMap<u64, String>,
    dirs: Vec<String>,
    dir_set: HashSet<String>,
    queue: Vec<String>,
    vfs: &'a Vfs,
}

impl Crawl<'_> {
    fn try_add(&mut self, cand: &str) -> bool {
        let h = path_hash(cand);
        if !self.total.contains(&h) || self.found.contains_key(&h) {
            return false;
        }
        for d in parent_dirs(cand) {
            if self.dir_set.insert(d.to_owned()) {
                self.dirs.push(d.to_owned());
            }
        }
        // Archive directory entries are hashed without the trailing separator.
        if let Some(loc) = self.vfs.lookup_hash(h) {
            if loc.entry.is_dir() {
                let d = format!("{cand}/");
                if self.dir_set.insert(d.clone()) {
                    self.dirs.push(d);
                }
            }
        }
        self.found.insert(h, cand.to_owned());
        self.queue.push(cand.to_owned());
        true
    }

    fn try_with_extensions(&mut self, cand: &str) {
        if has_extension(cand) {
            self.try_add(cand);
        } else {
            for e in EXTENSIONS {
                self.try_add(&format!("{cand}.{e}"));
            }
            self.try_add(cand);
        }
    }
}

impl Names {
    pub fn recover(vfs: &Vfs, seeds: &[String]) -> Self {
        let mut c = Crawl {
            total: vfs.entries().map(|(h, _)| h).collect(),
            found: HashMap::new(),
            dirs: Vec::new(),
            dir_set: HashSet::new(),
            queue: Vec::new(),
            vfs,
        };
        for s in std::iter::once(".rc".to_owned()).chain(seeds.iter().cloned()) {
            let s = s.replace('\\', "/");
            c.try_add(&s);
        }
        // File-name tokens (with extension) seen anywhere, for the global pass.
        let mut names: Vec<String> = Vec::new();
        let mut name_set: HashSet<String> = HashSet::new();
        let (mut dirs_done, mut names_done) = (0usize, 0usize);
        loop {
            while let Some(path) = c.queue.pop() {
                let Some(loc) = vfs.lookup_hash(path_hash(&path)).copied() else { continue };
                if loc.entry.is_dir() || loc.entry.size > MAX_SCAN {
                    continue;
                }
                let Ok(data) = vfs.read_location(&loc) else { continue };
                let local: Vec<String> =
                    std::iter::once(String::new()).chain(parent_dirs(&path).map(str::to_owned)).collect();
                let toks: HashSet<String> = tokens(&data).collect();
                for t in toks {
                    // Bare words are only tried next to the referencing file.
                    let dirs = if has_extension(&t) || t.contains('/') { &local[..] } else { &local[local.len() - 1..] };
                    for d in dirs {
                        if let Some(cand) = join(d, &t) {
                            c.try_with_extensions(&cand);
                        }
                    }
                    if let Some(n) = t.rsplit('/').next().filter(|n| n.contains('.')) {
                        if name_set.insert(n.to_owned()) {
                            names.push(n.to_owned());
                        }
                    }
                }
            }
            // Global pass, incremental: new dirs x all names, then old dirs x new names.
            let before = c.found.len();
            let (nd, nn) = (c.dirs.len(), names.len());
            for di in 0..nd {
                let from = if di < dirs_done { names_done } else { 0 };
                for name in &names[from..nn] {
                    let cand = format!("{}{}", c.dirs[di], name);
                    c.try_add(&cand);
                }
            }
            dirs_done = nd;
            names_done = nn;
            eprintln!(
                "names: {} of {} entries named, {} dirs, {} file names",
                c.found.len(),
                c.total.len(),
                c.dirs.len(),
                names.len()
            );
            if c.found.len() == before && c.queue.is_empty() && c.dirs.len() == dirs_done {
                break;
            }
        }
        Names { found: c.found }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_relative_paths() {
        assert_eq!(join("3do/plane/A/", "../B/hier.him").as_deref(), Some("3do/plane/B/hier.him"));
        assert_eq!(join("", "../x"), None);
    }

    #[test]
    fn finds_tokens() {
        let t: Vec<_> = tokens(b"Mesh = 3do\\plane\\A(B)\\x.msh\r\nfoo bar.mat\0\x01abc").collect();
        assert_eq!(t, ["Mesh", "3do/plane/A(B)/x.msh", "foo", "bar.mat", "abc"]);
    }
}
