//! LZSS block decoder (4 KiB ring, 3..=18 byte matches). See `docs/formats/sfs.md`.

pub fn decode(src: &[u8], out: &mut Vec<u8>) {
    let mut ring = [0u8; 4096];
    let mut r = 0xFEE;
    let mut flags: u32 = 0;
    let mut i = 0;
    loop {
        flags >>= 1;
        if flags & 0x100 == 0 {
            let Some(&f) = src.get(i) else { return };
            flags = f as u32 | 0xFF00;
            i += 1;
        }
        if flags & 1 != 0 {
            let Some(&c) = src.get(i) else { return };
            i += 1;
            out.push(c);
            ring[r] = c;
            r = (r + 1) & 0xFFF;
        } else {
            let (Some(&b0), Some(&b1)) = (src.get(i), src.get(i + 1)) else {
                return;
            };
            i += 2;
            let mut p = b0 as usize | ((b1 as usize & 0xF0) << 4);
            for _ in 0..(b1 & 0x0F) + 3 {
                let c = ring[p];
                p = (p + 1) & 0xFFF;
                out.push(c);
                ring[r] = c;
                r = (r + 1) & 0xFFF;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn literals_and_backref() {
        // flags 0b011: two literals 'a','b', then a reference to ring position 0xFEE, length 4.
        let src = [0b0000_0011, b'a', b'b', 0xEE, 0xF1];
        let mut out = Vec::new();
        super::decode(&src, &mut out);
        assert_eq!(out, b"ababab");
    }
}
