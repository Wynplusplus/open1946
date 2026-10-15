//! IMF images: `docs/formats/imf.md`.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IMF: not an IMF image")]
    NotImf,
    #[error("IMF: {0}")]
    Bad(String),
}

pub const MAGIC: &[u8; 6] = b"IMF\x1a10";

pub fn is_imf(data: &[u8]) -> bool {
    data.len() >= 12 && &data[..6] == MAGIC
}

/// Decoded image, RGBA8, top row first.
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (ia, ib, ic) = (a as i16, b as i16, c as i16);
    let p = ia + ib - ic;
    let (pa, pb, pc) = ((p - ia).abs(), (p - ib).abs(), (p - ic).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Decodes a filtered plane starting at `data[0]`; returns the pixels and the bytes consumed.
fn plane(data: &[u8], w: usize, h: usize, bpp: usize) -> Result<(Vec<u8>, usize), Error> {
    let stride = w * bpp;
    let len = h + stride * h;
    let data = data.get(..len).ok_or_else(|| Error::Bad("truncated plane".into()))?;
    let (filters, px) = data.split_at(h);
    let mut out = vec![0u8; stride * h];
    for y in 0..h {
        let (done, rest) = out.split_at_mut(y * stride);
        let prev = if y > 0 { &done[(y - 1) * stride..] } else { &[][..] };
        let cur = &mut rest[..stride];
        let src = &px[y * stride..(y + 1) * stride];
        let f = filters[y];
        for x in 0..stride {
            let a = if x >= bpp { cur[x - bpp] } else { 0 };
            let b = prev.get(x).copied().unwrap_or(0);
            let c = if x >= bpp { prev.get(x - bpp).copied().unwrap_or(0) } else { 0 };
            let pred = match f {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((a as u16 + b as u16) / 2) as u8,
                4 => paeth(a, b, c),
                _ => return Err(Error::Bad(format!("unknown filter {f} in row {y}"))),
            };
            cur[x] = src[x].wrapping_add(pred);
        }
    }
    Ok((out, len))
}

pub fn decode(data: &[u8]) -> Result<Rgba, Error> {
    if !is_imf(data) {
        return Err(Error::NotImf);
    }
    let typ = data[7];
    let w = u16::from_le_bytes([data[8], data[9]]) as usize;
    let h = u16::from_le_bytes([data[10], data[11]]) as usize;
    if w == 0 || h == 0 || w > 0x1800 || h > 0x1000 {
        return Err(Error::Bad(format!("bad size {w}x{h}")));
    }
    let mut pos = 12;
    let rgb = match typ {
        0 | 1 => {
            let (rgb, n) = plane(&data[pos..], w, h, 3)?;
            pos += n;
            rgb
        }
        2 => {
            let n = data.get(12..16).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize);
            let n = n.ok_or_else(|| Error::Bad("truncated".into()))?;
            let jpg = data.get(16..16 + n).ok_or_else(|| Error::Bad("truncated JPEG".into()))?;
            let opts = zune_jpeg::zune_core::options::DecoderOptions::default()
                .jpeg_set_out_colorspace(zune_jpeg::zune_core::colorspace::ColorSpace::RGB);
            let mut dec = zune_jpeg::JpegDecoder::new_with_options(jpg, opts);
            let rgb = dec.decode().map_err(|e| Error::Bad(format!("JPEG: {e:?}")))?;
            if rgb.len() != w * h * 3 {
                return Err(Error::Bad("JPEG size mismatch".into()));
            }
            pos = 16 + n;
            rgb
        }
        t => return Err(Error::Bad(format!("unknown type {t}"))),
    };
    let alpha = if typ == 0 {
        None
    } else {
        let (a, n) = plane(&data[pos..], w, h, 1)?;
        pos += n;
        Some(a)
    };
    if pos != data.len() {
        return Err(Error::Bad(format!("size mismatch: {} bytes, expected {pos}", data.len())));
    }
    let mut pixels = Vec::with_capacity(w * h * 4);
    for i in 0..w * h {
        pixels.extend_from_slice(&rgb[i * 3..i * 3 + 3]);
        pixels.push(alpha.as_ref().map_or(255, |a| a[i]));
    }
    Ok(Rgba { width: w as u32, height: h as u32, pixels })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(typ: u8, w: u16, h: u16) -> Vec<u8> {
        let mut d = MAGIC.to_vec();
        d.extend([0, typ]);
        d.extend(w.to_le_bytes());
        d.extend(h.to_le_bytes());
        d
    }

    #[test]
    fn decodes_filtered_rgb_alpha() {
        // 2x2, RGB rows: [10,20,30 | 11,21,31], [12,22,32 | 13,23,33]
        let mut d = header(1, 2, 2);
        d.extend([1, 2]); // Sub for row 0, Up for row 1
        d.extend([10, 20, 30, 1, 1, 1]);
        d.extend([2, 2, 2, 2, 2, 2]);
        d.extend([0, 4]); // alpha: None, Paeth
        d.extend([200, 201]);
        d.extend([1, 1]);
        let img = decode(&d).unwrap();
        assert_eq!(&img.pixels[..8], &[10, 20, 30, 200, 11, 21, 31, 201]);
        assert_eq!(&img.pixels[8..], &[12, 22, 32, 201, 13, 23, 33, 202]);
    }

    #[test]
    fn rejects_wrong_size() {
        let mut d = header(0, 1, 1);
        d.extend([0, 1, 2, 3, 99]);
        assert!(decode(&d).is_err());
    }
}
