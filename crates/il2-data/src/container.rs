//! Named-stream container (`InOutStreams`): `docs/formats/krypto.md`, "Stream container".

use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("stream container: {0}")]
    Bad(String),
}

const SIGNATURE: i32 = 0x4A3B_2C1D;

struct Entry {
    len: usize,
    blocks: Vec<usize>,
}

pub struct Container {
    data: Vec<u8>,
    compression: i32,
    block_size: usize,
    block_offsets: Vec<usize>,
    entries: HashMap<String, Entry>,
}

fn be32(d: &[u8], o: usize) -> Result<i32, Error> {
    d.get(o..o + 4)
        .map(|b| i32::from_be_bytes(b.try_into().unwrap()))
        .ok_or_else(|| Error::Bad("truncated".into()))
}

impl Container {
    pub fn parse(data: Vec<u8>) -> Result<Self, Error> {
        let bad = |s: &str| Error::Bad(s.to_owned());
        if be32(&data, 0)? != SIGNATURE || be32(&data, 4)? != 101 {
            return Err(bad("signature/version"));
        }
        let compression = be32(&data, 8)?;
        let block_size = be32(&data, 12)?;
        if !(0..=2).contains(&compression) || !(2048..=131072).contains(&block_size) {
            return Err(bad("header"));
        }
        let end = data.len().checked_sub(8).ok_or_else(|| bad("truncated"))?;
        let idx_size = be32(&data, end)? as usize;
        let ent_size = be32(&data, end + 4)? as usize;
        let idx_start = end
            .checked_sub(ent_size + idx_size)
            .ok_or_else(|| bad("tailer"))?;
        let mut block_offsets = vec![16];
        for i in 0..idx_size / 4 {
            let len = be32(&data, idx_start + 4 * i)?;
            block_offsets.push(block_offsets.last().unwrap() + len.max(0) as usize);
        }
        if *block_offsets.last().unwrap() > idx_start {
            return Err(bad("block table"));
        }
        let mut entries = HashMap::new();
        let mut p = idx_start + idx_size;
        while p < end {
            let n = u16::from_be_bytes(
                data.get(p..p + 2)
                    .ok_or_else(|| bad("entry"))?
                    .try_into()
                    .unwrap(),
            ) as usize;
            // Java modified UTF-8; names here are ASCII.
            let name =
                String::from_utf8_lossy(data.get(p + 2..p + 2 + n).ok_or_else(|| bad("entry"))?)
                    .into_owned();
            p += 2 + n;
            let len = be32(&data, p)?.max(0) as usize;
            let count = be32(&data, p + 4)?.max(0) as usize;
            p += 8;
            let blocks = (0..count)
                .map(|i| be32(&data, p + 4 * i).map(|b| b.max(0) as usize))
                .collect::<Result<Vec<_>, _>>()?;
            p += 4 * count;
            if blocks.iter().any(|&b| b + 1 >= block_offsets.len()) {
                return Err(bad("block index"));
            }
            entries.insert(name, Entry { len, blocks });
        }
        Ok(Container {
            data,
            compression,
            block_size: block_size as usize,
            block_offsets,
            entries,
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn read(&self, name: &str) -> Result<Option<Vec<u8>>, Error> {
        let Some(e) = self.entries.get(name) else {
            return Ok(None);
        };
        let mut out = Vec::with_capacity(e.len);
        for (k, &b) in e.blocks.iter().enumerate() {
            let raw = &self.data[self.block_offsets[b]..self.block_offsets[b + 1]];
            let want = self.block_size.min(e.len - k * self.block_size);
            if self.compression != 0 && raw.len() < want {
                match self.compression {
                    2 if raw.starts_with(&[8, 0]) => {
                        let d = miniz_oxide::inflate::decompress_to_vec_with_limit(&raw[2..], want)
                            .map_err(|e| Error::Bad(format!("inflate: {e:?}")))?;
                        out.extend_from_slice(&d);
                    }
                    2 => out.extend_from_slice(&raw[2.min(raw.len())..]),
                    _ => return Err(Error::Bad("LZSS streams not supported yet".into())),
                }
            } else {
                out.extend_from_slice(&raw[..want.min(raw.len())]);
            }
        }
        out.truncate(e.len);
        if out.len() != e.len {
            return Err(Error::Bad(format!("{name}: short stream")));
        }
        Ok(Some(out))
    }
}
