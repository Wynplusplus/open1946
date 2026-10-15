//! Just enough PE32 parsing to read initialized data of the launcher executable.

pub struct Section {
    pub va: u32,
    pub data: Vec<u8>,
}

pub struct Image {
    pub sections: Vec<Section>,
}

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

impl Image {
    pub fn parse(d: &[u8]) -> Option<Self> {
        let pe = u32_at(d, 0x3C)? as usize;
        if d.get(pe..pe + 4)? != b"PE\0\0" {
            return None;
        }
        let nsec = u16_at(d, pe + 6)? as usize;
        let opt_size = u16_at(d, pe + 20)? as usize;
        let opt = pe + 24;
        if u16_at(d, opt)? != 0x10B {
            return None; // not PE32
        }
        let base = u32_at(d, opt + 28)?;
        let mut sections = Vec::new();
        for i in 0..nsec {
            let s = opt + opt_size + i * 40;
            let va = u32_at(d, s + 12)?;
            let raw_size = u32_at(d, s + 16)? as usize;
            let raw_off = u32_at(d, s + 20)? as usize;
            let data = d.get(raw_off..raw_off + raw_size)?.to_vec();
            sections.push(Section { va: base + va, data });
        }
        Some(Image { sections })
    }

    /// Bytes at a virtual address, if it lies in initialized section data.
    pub fn at(&self, va: u32) -> Option<&[u8]> {
        self.sections
            .iter()
            .find(|s| va >= s.va && ((va - s.va) as usize) < s.data.len())
            .map(|s| &s.data[(va - s.va) as usize..])
    }
}
