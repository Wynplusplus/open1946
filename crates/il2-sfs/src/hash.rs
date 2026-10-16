//! The 64-bit two-lane hash used for SFS path lookup and archive scramble keys.
//! See `docs/formats/sfs.md`, section "Path hash".

const BASIS_LO: [u32; 8] = [
    0x2378_8D5E,
    0x46F1_1ABC,
    0x0DE2_3578,
    0x38BC_E7AE,
    0x7179_CF5C,
    0x62F3_9EB8,
    0x669F_B02E,
    0x6E47_ED02,
];
const BASIS_HI: [u32; 8] = [
    0x1434_182E,
    0x3C5C_2872,
    0x6C8C_48CA,
    0x4D2C_89BA,
    0x1A59_1374,
    0x2086_3EC6,
    0x5538_65A2,
    0x3E44_D36A,
];

/// Expands a linear table from its values at the powers of two.
const fn expand(basis: [u32; 8]) -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 1;
    while i < 256 {
        let mut v = 0;
        let mut bit = 0;
        while bit < 8 {
            if i & (1 << bit) != 0 {
                v ^= basis[bit];
            }
            bit += 1;
        }
        t[i] = v;
        i += 1;
    }
    t
}

static TABLE_LO: [u32; 256] = expand(BASIS_LO);
static TABLE_HI: [u32; 256] = expand(BASIS_HI);

/// Incremental hash state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HashState {
    pub lo: u32,
    pub hi: u32,
}

impl HashState {
    pub fn update(&mut self, data: &[u8]) {
        for &c in data {
            self.lo = ((self.lo << 8) | c as u32) ^ TABLE_LO[(self.lo >> 24) as usize];
            self.hi = ((self.hi << 8) | c as u32) ^ TABLE_HI[(self.hi >> 24) as usize];
        }
    }

    pub fn of(data: &[u8]) -> Self {
        let mut s = Self::default();
        s.update(data);
        s
    }

    /// Combined key, `hi` in the upper half.
    pub fn key(self) -> u64 {
        (self.hi as u64) << 32 | self.lo as u64
    }
}

/// Normalizes a game path the way the original runtime does: ASCII `a-z` upper-cased, `/` turned into `\\`,
/// no leading separator. The runtime never looks up paths containing characters `>= 0x80` in archives
/// (they go straight to the file system), so other characters are passed through as their low byte.
pub fn normalize_path(path: &str) -> Vec<u8> {
    path.trim_start_matches(['/', '\\'])
        .chars()
        .map(|c| match c {
            '/' => b'\\',
            c => (c as u32 as u8).to_ascii_uppercase(),
        })
        .collect()
}

/// 32-bit string hash used by the runtime for class and stream names (`IntHash` in
/// `docs/formats/krypto.md`): the low lane over UTF-16 code units, low byte first, as a signed int.
pub fn int_hash(s: &str) -> i32 {
    let mut h = HashState::default();
    for u in s.encode_utf16() {
        h.update(&u.to_le_bytes());
    }
    h.lo as i32
}

/// Hash key of a game path as stored in SFS indices.
pub fn path_hash(path: &str) -> u64 {
    HashState::of(&normalize_path(path)).key()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_linear() {
        for a in 0..256 {
            for b in 0..256 {
                assert_eq!(TABLE_LO[a ^ b], TABLE_LO[a] ^ TABLE_LO[b]);
            }
        }
    }

    #[test]
    fn int_hash_is_low_lane_over_utf16() {
        let mut h = HashState::default();
        h.update(&[b'a', 0, b'b', 0]);
        assert_eq!(int_hash("ab"), h.lo as i32);
    }

    #[test]
    fn known_path() {
        let h = path_hash("com/maddox/il2/objects/air.ini");
        assert_eq!(h, 0xF3CC_1777_2C47_365D);
        assert_eq!(path_hash("COM\\MADDOX\\IL2\\OBJECTS\\AIR.INI"), h);
    }
}
