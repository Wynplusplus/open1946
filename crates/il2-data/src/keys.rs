//! Locates the runtime key table (`K` in `docs/formats/krypto.md`) in the user's own install.
//!
//! The table is a static array of the runtime's `Finger` class. That class is stored scrambled
//! with one of a few short bootstrap keys held by the launcher executable. Instead of relying on
//! fixed addresses, every length-prefixed byte string referenced by a pointer in the launcher's
//! data is tried as a key until the class decodes.

use crate::classfile::ClassFile;
use crate::pe;
use il2_vfs::Vfs;

/// Archive key (path hash) of the scrambled `Finger` class used by the launcher's bootstrap loader.
const FINGER_CLASS_HASH: u64 = 0x0830_D1EA_0C04_8808;
pub(crate) const CLASS_HEADER: [u8; 8] = [0xCA, 0xFE, 0xBA, 0xBE, 0x00, 0x00, 0x00, 0x2F];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Vfs(#[from] il2_vfs::Error),
    #[error("launcher {0}: {1}")]
    Launcher(String, String),
    #[error("could not recover the key table from this install")]
    NotFound,
}

pub struct KeyTable(pub Vec<u8>);

impl KeyTable {
    pub fn from_install(vfs: &Vfs) -> Result<Self, Error> {
        let exe = vfs.root().join("il2fb.exe");
        let launcher_err = |e: String| Error::Launcher(exe.display().to_string(), e);
        let bytes = std::fs::read(&exe).map_err(|e| launcher_err(e.to_string()))?;
        let image = pe::Image::parse(&bytes).ok_or_else(|| launcher_err("not a PE32 image".into()))?;
        let scrambled = vfs.read_hash(FINGER_CLASS_HASH)?;

        let mut tried = std::collections::HashSet::new();
        for sect in &image.sections {
            for chunk in sect.data.chunks_exact(4) {
                let ptr = u32::from_le_bytes(chunk.try_into().unwrap());
                let Some(at) = image.at(ptr) else { continue };
                let n = at[0] as usize;
                if !(8..=40).contains(&n) || at.len() <= n {
                    continue;
                }
                let key = &at[1..=n];
                if !tried.insert(key.to_vec()) {
                    continue;
                }
                if let Some(table) = try_key(&scrambled, key) {
                    return Ok(KeyTable(table));
                }
            }
        }
        Err(Error::NotFound)
    }

    /// `key(h)` slice for flight-model streams (`docs/formats/krypto.md`, "Flight-model tables").
    pub fn fm_key(&self, h: i32, len: usize) -> Vec<u8> {
        let k = self.0.len() as i64;
        let h = (h as i64).abs();
        let n = len as i64;
        let klen = ((n + h / 5) % 16 + 14).max(10);
        let start = (n + h / 19) % k;
        (0..klen).map(|i| self.0[((start + i) % k) as usize]).collect()
    }
}

fn try_key(scrambled: &[u8], key: &[u8]) -> Option<Vec<u8>> {
    // Cheap pre-check on the constant pool count before decoding everything.
    let cp = u16::from_be_bytes([scrambled[0] ^ key[0], scrambled[1] ^ key[1 % key.len()]]);
    if !(16..20000).contains(&cp) {
        return None;
    }
    let mut class = CLASS_HEADER.to_vec();
    class.extend(scrambled.iter().enumerate().map(|(i, b)| b ^ key[i % key.len()]));
    let arrays = ClassFile::parse(&class).ok()?.static_arrays().ok()?;
    let k = arrays.get("kTable")?;
    (k.len() >= 256).then(|| k.iter().map(|&v| v as u8).collect())
}

impl KeyTable {
    /// Key slice for a game class with name hash `h` (`docs/formats/krypto.md`, "Game classes").
    pub fn class_key(&self, h: i32) -> Vec<u8> {
        let k = self.0.len() as i64;
        let h = (h as i64).abs();
        let len = (h % 16 + 14).max(10);
        let start = h % k;
        (0..len).map(|i| self.0[((start + i) % k) as usize]).collect()
    }
}

/// XORs a stream with `key`, starting at key element 1 (`docs/formats/krypto.md`, "Stream XOR").
pub fn unscramble_stream(data: &mut [u8], key: &[u8]) {
    for (i, b) in data.iter_mut().enumerate() {
        *b ^= key[(i + 1) % key.len()];
    }
}
