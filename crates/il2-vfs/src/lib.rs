//! Virtual file system over an IL-2 1946 install. Lookup rules: `docs/formats/vfs.md`.

use il2_sfs::{path_hash, Archive, Entry};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Sfs(#[from] il2_sfs::Error),
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("{0}: not found")]
    NotFound(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Top-level directories that never fall back to loose files.
const PROTECTED_DIRS: &[&str] = &["3do", "maps", "gui", "cod", "com", "effects", "presets"];

#[derive(Clone, Copy, Debug)]
pub struct Location {
    pub archive: usize,
    pub entry: Entry,
}

pub struct Vfs {
    root: PathBuf,
    archives: Vec<Archive>,
    table: HashMap<u64, Location>,
}

impl Vfs {
    /// Empty file system rooted at a game directory; archives are added with [`Vfs::mount`].
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into(), archives: Vec::new(), table: HashMap::new() }
    }

    /// Mounts the archives of a stock install in the order the game uses: `files.SFS`, then the
    /// `sfs MOUNT` commands of the `.rc` boot script.
    pub fn open_install(root: impl Into<PathBuf>) -> Result<Self> {
        let mut vfs = Self::new(root);
        vfs.mount("files.SFS")?;
        let rc = vfs.read(".rc")?;
        for name in mount_commands(&String::from_utf8_lossy(&rc)) {
            vfs.mount(&name)?;
        }
        Ok(vfs)
    }

    /// Mounts an archive (path relative to the root, matched case-insensitively).
    /// Entries override those of earlier mounts.
    pub fn mount(&mut self, name: &str) -> Result<()> {
        let path = self.resolve_loose(name).ok_or_else(|| Error::NotFound(name.to_owned()))?;
        let archive = Archive::open(path)?;
        let idx = self.archives.len();
        for e in archive.entries() {
            self.table.insert(e.hash, Location { archive: idx, entry: *e });
        }
        self.archives.push(archive);
        Ok(())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn archives(&self) -> &[Archive] {
        &self.archives
    }

    /// All visible (non-shadowed) archive entries.
    pub fn entries(&self) -> impl Iterator<Item = (u64, &Location)> {
        self.table.iter().map(|(h, l)| (*h, l))
    }

    pub fn lookup_hash(&self, hash: u64) -> Option<&Location> {
        self.table.get(&hash)
    }

    pub fn read_location(&self, loc: &Location) -> Result<Vec<u8>> {
        Ok(self.archives[loc.archive].read(&loc.entry)?)
    }

    pub fn read_hash(&self, hash: u64) -> Result<Vec<u8>> {
        let loc = self.lookup_hash(hash).ok_or_else(|| Error::NotFound(format!("{hash:#018x}")))?;
        self.read_location(loc)
    }

    pub fn exists(&self, path: &str) -> bool {
        let path = normalize(path);
        self.table.contains_key(&path_hash(&path)) || self.loose_path(&path).is_some()
    }

    pub fn read(&self, path: &str) -> Result<Vec<u8>> {
        let path = &normalize(path);
        if let Some(loc) = self.table.get(&path_hash(path)) {
            return self.read_location(loc);
        }
        let p = self.loose_path(path).ok_or_else(|| Error::NotFound(path.to_owned()))?;
        std::fs::read(&p).map_err(|e| Error::Io(p, e))
    }

    /// Loose-file fallback, honoring the protected directories.
    fn loose_path(&self, path: &str) -> Option<PathBuf> {
        let path = path.trim_start_matches(['/', '\\']);
        match path.split_once(['/', '\\']) {
            Some((top, _)) if PROTECTED_DIRS.iter().any(|d| d.eq_ignore_ascii_case(top)) => return None,
            None if path.get(..3).is_some_and(|p| p.eq_ignore_ascii_case(".rc")) => return None,
            _ => {}
        }
        self.resolve_loose(path)
    }

    /// Case-insensitive resolution of a relative path under the root.
    fn resolve_loose(&self, rel: &str) -> Option<PathBuf> {
        let mut cur = self.root.clone();
        for comp in rel.split(['/', '\\']).filter(|c| !c.is_empty()) {
            let exact = cur.join(comp);
            cur = if exact.exists() {
                exact
            } else {
                std::fs::read_dir(&cur)
                    .ok()?
                    .filter_map(|e| e.ok())
                    .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(comp))?
                    .path()
            };
        }
        cur.is_file().then_some(cur)
    }
}

/// Collapses `.` and `..` components and unifies separators to `/`. Asset files refer to each other
/// with relative paths such as `3do/plane/X/../Textures/a.tga`, while archive hashes are over the
/// collapsed form.
pub fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for p in path.split(['/', '\\']) {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

/// Extracts archive names from `sfs MOUNT <name>` lines of a command script.
pub fn mount_commands(script: &str) -> Vec<String> {
    script
        .lines()
        .filter_map(|l| {
            let mut w = l.trim().trim_start_matches('@').split_whitespace();
            (w.next()?.eq_ignore_ascii_case("sfs") && w.next()?.eq_ignore_ascii_case("mount"))
                .then(|| w.next().map(str::to_owned))?
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn normalizes_relative_paths() {
        assert_eq!(super::normalize("3DO/Plane/A/..\\TEXTURES\\gear.tga"), "3DO/Plane/TEXTURES/gear.tga");
        assert_eq!(super::normalize("./a//b/./c"), "a/b/c");
    }

    #[test]
    fn parses_mount_lines() {
        let s = "HELP sfs [MOUNT <lib name>]\r\n@sfs MOUNT fb_3do.sfs\r\nsfs mount b.sfs\r\n@load x\r\n";
        assert_eq!(super::mount_commands(s), ["fb_3do.sfs", "b.sfs"]);
    }
}
