//! Reader for IL-2 Sturmovik 1946 `.SFS` archives. Format: `docs/formats/sfs.md`.

mod hash;
mod lzss;

pub use hash::{int_hash, normalize_path, path_hash, HashState};

use memmap2::Mmap;
use std::fs::File;
use std::path::{Path, PathBuf};

pub const BLOCK_SIZE: usize = 0x8000;
const HEADER_SIZE: usize = 0x100;
const ENTRY_SIZE: usize = 32;

pub const ATTR_DIRECTORY: u32 = 0x10;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("bad SFS header: {0}")]
    BadHeader(&'static str),
    #[error("unsupported SFS version {0:#x}")]
    UnsupportedVersion(u32),
    #[error("archive is truncated or corrupt: {0}")]
    Corrupt(&'static str),
    #[error("block {block}: {reason}")]
    Block { block: usize, reason: String },
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Path hash, `hi << 32 | lo`.
    pub hash: u64,
    pub offset: u32,
    pub size: u32,
    pub attributes: u32,
    /// Purpose not fully known; looks like a content checksum.
    pub checksum: u32,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.attributes & ATTR_DIRECTORY != 0
    }
}

pub struct Archive {
    path: PathBuf,
    map: Mmap,
    entries: Vec<Entry>,
    blocks: Vec<u32>,
    stream_size: u32,
}

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}

fn key_bytes(key: HashState) -> [u8; 8] {
    let mut k = [0u8; 8];
    k[..4].copy_from_slice(&key.lo.to_le_bytes());
    k[4..].copy_from_slice(&key.hi.to_le_bytes());
    k
}

fn unscramble(data: &mut [u8], key: &[u8; 8], div: usize) {
    for (i, b) in data.iter_mut().enumerate() {
        *b ^= key[(i + i / div) & 7];
    }
}

impl Archive {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let io = |source| Error::Io {
            path: path.clone(),
            source,
        };
        let file = File::open(&path).map_err(io)?;
        // SAFETY: game archives are not modified while we hold the mapping.
        let map = unsafe { Mmap::map(&file) }.map_err(io)?;
        Self::from_map(path, map)
    }

    fn from_map(path: PathBuf, map: Mmap) -> Result<Self> {
        let raw_hdr = map
            .get(..HEADER_SIZE)
            .ok_or(Error::BadHeader("file shorter than header"))?;
        if &raw_hdr[0..4] != b"SFS\0" {
            return Err(Error::BadHeader("magic"));
        }
        let version = u32_at(raw_hdr, 4);
        let mut hdr = raw_hdr.to_vec();
        let key = match version {
            0xC9 => HashState::of(raw_hdr),
            // Keyed by the lower-case archive file name; header fields 0x08..0x20 are scrambled too.
            0xCA => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                let key = HashState::of(name.as_bytes());
                unscramble(&mut hdr[0x08..0x20], &key_bytes(key), 3);
                key
            }
            v => return Err(Error::UnsupportedVersion(v)),
        };
        let hdr = &hdr[..];
        let checksum = u32_at(hdr, 8);
        let sum: u32 = hdr
            .iter()
            .enumerate()
            .filter(|(i, _)| !(8..12).contains(i))
            .map(|(_, &b)| b as u32)
            .sum();
        if sum != checksum {
            return Err(Error::BadHeader("checksum"));
        }
        let count = u32_at(hdr, 0x0C) as usize;
        let index_off = u32_at(hdr, 0x10) as usize;
        let blocks_off = u32_at(hdr, 0x14) as usize;
        let stream_size = u32_at(hdr, 0x1C);
        let scrambled = version & 3 != 0;
        let key_bytes = key_bytes(key);

        let mut index = map
            .get(index_off..index_off + count * ENTRY_SIZE)
            .ok_or(Error::Corrupt("index out of bounds"))?
            .to_vec();
        if scrambled {
            unscramble(&mut index, &key_bytes, 3);
        }
        let entries = index
            .chunks_exact(ENTRY_SIZE)
            .map(|e| Entry {
                hash: (u32_at(e, 4) as u64) << 32 | u32_at(e, 0) as u64,
                offset: u32_at(e, 0x0C),
                size: u32_at(e, 0x10),
                attributes: u32_at(e, 0x18),
                checksum: u32_at(e, 0x1C),
            })
            .collect();

        let nblocks = (stream_size as usize).div_ceil(BLOCK_SIZE);
        let mut table = map
            .get(blocks_off..blocks_off + (nblocks + 1) * 4)
            .ok_or(Error::Corrupt("block table out of bounds"))?
            .to_vec();
        if scrambled {
            unscramble(&mut table, &key_bytes, 5);
        }
        let blocks: Vec<u32> = table.chunks_exact(4).map(|c| u32_at(c, 0)).collect();
        if blocks.windows(2).any(|w| w[0] > w[1]) || *blocks.last().unwrap() as usize > map.len() {
            return Err(Error::Corrupt("block table"));
        }

        Ok(Self {
            path,
            map,
            entries,
            blocks,
            stream_size,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn block_count(&self) -> usize {
        self.blocks.len() - 1
    }

    /// Finds an entry by path hash (see [`path_hash`]).
    pub fn find_hash(&self, hash: u64) -> Option<&Entry> {
        let lo = hash as u32;
        let start = self.entries.partition_point(|e| (e.hash as u32) < lo);
        self.entries[start..]
            .iter()
            .take_while(|e| e.hash as u32 == lo)
            .find(|e| e.hash == hash)
    }

    pub fn find(&self, path: &str) -> Option<&Entry> {
        self.find_hash(path_hash(path))
    }

    /// Decompresses one 32 KiB block of the logical data stream.
    pub fn read_block(&self, block: usize, out: &mut Vec<u8>) -> Result<()> {
        let err = |reason: &str| Error::Block {
            block,
            reason: reason.to_owned(),
        };
        let start = self.blocks[block] as usize;
        let raw = &self.map[start..self.blocks[block + 1] as usize];
        out.clear();
        if raw.len() >= BLOCK_SIZE {
            out.extend_from_slice(&raw[..BLOCK_SIZE]);
        } else {
            match raw.first() {
                None => return Err(err("empty block")),
                Some(1) => lzss::decode(&raw[1..], out),
                Some(_) => {
                    let method = raw.get(1..3).ok_or_else(|| err("truncated"))?;
                    match u16::from_le_bytes([method[0], method[1]]) {
                        0 => out.extend_from_slice(&raw[3..]),
                        8 => {
                            let d = miniz_oxide::inflate::decompress_to_vec_with_limit(
                                &raw[3..],
                                BLOCK_SIZE,
                            )
                            .map_err(|e| err(&format!("inflate: {e:?}")))?;
                            out.extend_from_slice(&d);
                        }
                        m => return Err(err(&format!("unknown method {m}"))),
                    }
                }
            }
        }
        let expected = BLOCK_SIZE.min(self.stream_size as usize - block * BLOCK_SIZE);
        if out.len() < expected {
            return Err(err(&format!(
                "decoded {} bytes, expected {expected}",
                out.len()
            )));
        }
        Ok(())
    }

    /// Reads the whole content of an entry.
    pub fn read(&self, entry: &Entry) -> Result<Vec<u8>> {
        let size = entry.size as usize;
        let mut out = Vec::with_capacity(size);
        let mut block = Vec::with_capacity(BLOCK_SIZE);
        let mut pos = entry.offset as usize;
        if pos + size > self.stream_size as usize {
            return Err(Error::Corrupt("entry beyond data stream"));
        }
        while out.len() < size {
            let (b, off) = (pos / BLOCK_SIZE, pos % BLOCK_SIZE);
            self.read_block(b, &mut block)?;
            let n = (size - out.len()).min(BLOCK_SIZE - off);
            out.extend_from_slice(&block[off..off + n]);
            pos += n;
        }
        Ok(out)
    }
}

impl std::fmt::Debug for Archive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Archive")
            .field("path", &self.path)
            .field("entries", &self.entries.len())
            .finish()
    }
}
