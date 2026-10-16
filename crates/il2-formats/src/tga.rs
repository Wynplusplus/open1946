//! Colour-mapped TGA (types 1 and 9), which some shipped textures use with header values that
//! general-purpose decoders reject (for example 8 attribute bits on a 24-bit palette).
//! The format is the public Truevision TGA specification.

/// Decodes a colour-mapped TGA into RGBA8, top row first. `None` if it isn't one.
pub fn decode_colormapped(d: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let h = d.get(..18)?;
    let (id_len, map_type, kind) = (h[0] as usize, h[1], h[2]);
    if map_type != 1 || !(kind == 1 || kind == 9) {
        return None;
    }
    let u16_at = |i: usize| u16::from_le_bytes([h[i], h[i + 1]]) as usize;
    let (map_first, map_len, entry_bits) = (u16_at(3), u16_at(5), h[7] as usize);
    let (w, ht, depth, desc) = (u16_at(12), u16_at(14), h[16] as usize, h[17]);
    if depth != 8 || !matches!(entry_bits, 15 | 16 | 24 | 32) {
        return None;
    }
    let entry_bytes = entry_bits.div_ceil(8);
    let map_start = 18 + id_len;
    let map = d.get(map_start..map_start + map_len * entry_bytes)?;
    let palette: Vec<[u8; 4]> = map
        .chunks_exact(entry_bytes)
        .map(|e| match entry_bytes {
            2 => {
                let v = u16::from_le_bytes([e[0], e[1]]);
                let c = |s: u16| (((v >> s) & 31) * 255 / 31) as u8;
                [c(10), c(5), c(0), 255]
            }
            3 => [e[2], e[1], e[0], 255],
            _ => [e[2], e[1], e[0], e[3]],
        })
        .collect();
    let mut idx = Vec::with_capacity(w * ht);
    let mut p = map_start + map.len();
    if kind == 1 {
        idx.extend_from_slice(d.get(p..p + w * ht)?);
    } else {
        while idx.len() < w * ht {
            let c = *d.get(p)?;
            p += 1;
            let n = (c & 0x7f) as usize + 1;
            if c & 0x80 != 0 {
                let v = *d.get(p)?;
                p += 1;
                idx.extend(std::iter::repeat_n(v, n));
            } else {
                idx.extend_from_slice(d.get(p..p + n)?);
                p += n;
            }
        }
        idx.truncate(w * ht);
    }
    let top_first = desc & 0x20 != 0;
    let mut out = vec![0u8; w * ht * 4];
    for y in 0..ht {
        let src = if top_first { y } else { ht - 1 - y };
        for x in 0..w {
            let i = idx[src * w + x] as usize;
            let c = palette
                .get(i.wrapping_sub(map_first))
                .copied()
                .unwrap_or([0, 0, 0, 255]);
            out[(y * w + x) * 4..][..4].copy_from_slice(&c);
        }
    }
    Some((w as u32, ht as u32, out))
}

#[cfg(test)]
mod tests {
    #[test]
    fn decodes_bottom_up_palette_image() {
        // 2x2, palette of two 24-bit entries (BGR), bottom row first.
        let mut d = vec![0, 1, 1, 0, 0, 2, 0, 24, 0, 0, 0, 0, 2, 0, 2, 0, 8, 8];
        d.extend([0, 0, 255, 255, 0, 0]); // red, blue
        d.extend([0, 1, 1, 0]);
        let (w, h, px) = super::decode_colormapped(&d).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(&px[..8], &[0, 0, 255, 255, 255, 0, 0, 255]); // top row = last stored row
        assert_eq!(&px[8..], &[255, 0, 0, 255, 0, 0, 255, 255]);
    }
}
