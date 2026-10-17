//! Map terrain data: `docs/formats/map.md`.

/// Ground size of one height-map pixel, metres.
pub const PIXEL_SIZE: f32 = 200.0;

/// Converts a height-map byte to metres.
pub fn height(v: u8) -> f32 {
    let v = v as i32;
    (match v {
        0..=63 => v,
        64..=95 => 2 * v - 64,
        96..=127 => 4 * v - 256,
        128..=159 => 8 * v - 768,
        160..=191 => (v - 128) * 16,
        192..=223 => (v - 160) * 32,
        _ => (v - 192) * 64,
    }) as f32
}

/// Terrain heights of a map, queried in world metres (`docs/formats/map.md`, "Grid").
pub struct HeightField {
    pub width: usize,
    pub height: usize,
    /// Metres, row-major, top (north) row first.
    pub heights: Vec<f32>,
    /// Open water per land-type pixel, same layout (empty: no water anywhere).
    pub water: Vec<bool>,
}

impl HeightField {
    /// From the decoded 8-bit height map, rows top to bottom.
    pub fn from_raw(width: usize, height: usize, raw: &[u8]) -> Self {
        HeightField {
            width,
            height,
            heights: raw.iter().map(|&v| self::height(v)).collect(),
            water: Vec::new(),
        }
    }

    fn at(&self, c: usize, r_from_south: usize) -> f64 {
        let r = self.height - 1 - r_from_south;
        self.heights[r * self.width + c] as f64
    }

    /// Whether the land-type pixel under world `(x, y)` is water (never outside the map).
    pub fn is_water(&self, x: f64, y: f64) -> bool {
        if self.water.is_empty() {
            return false;
        }
        let (fx, fy) = (x / PIXEL_SIZE as f64, y / PIXEL_SIZE as f64);
        if fx < 0.0 || fy < 0.0 || fx >= self.width as f64 || fy >= self.height as f64 {
            return false;
        }
        let r = self.height - 1 - fy as usize;
        self.water[r * self.width + fx as usize]
    }

    /// Ground height at world `(x, y)`, interpolated over the two triangles of each cell; 0 outside.
    pub fn height_at(&self, x: f64, y: f64) -> f64 {
        let (fx, fy) = (x / PIXEL_SIZE as f64, y / PIXEL_SIZE as f64);
        if fx < 0.0 || fy < 0.0 {
            return 0.0;
        }
        let (c, r) = (fx.floor() as usize, fy.floor() as usize);
        if c + 1 >= self.width || r + 1 >= self.height {
            return 0.0;
        }
        let (u, v) = (fx - c as f64, fy - r as f64);
        let (h00, h10, h01, h11) = (
            self.at(c, r),
            self.at(c + 1, r),
            self.at(c, r + 1),
            self.at(c + 1, r + 1),
        );
        if u >= v {
            h00 + (h10 - h00) * u + (h11 - h10) * v
        } else {
            h00 + (h11 - h01) * u + (h01 - h00) * v
        }
    }
}

#[cfg(test)]
mod tests {
    use super::height;

    #[test]
    fn height_field_interpolates() {
        // 2x2 map, north row first: [10, 20] over [0, 0] (metres = byte for small values).
        let f = super::HeightField::from_raw(2, 2, &[10, 20, 0, 0]);
        assert_eq!(f.height_at(0.0, 0.0), 0.0);
        assert!((f.height_at(0.0, 199.999) - 10.0).abs() < 0.01);
        // Cell centre lies on the diagonal between (0,0) and (1,1): the average of those corners.
        assert!((f.height_at(100.0, 100.0) - 10.0).abs() < 1e-9);
    }

    #[test]
    fn table_is_continuous_and_monotonic() {
        assert_eq!(height(63), 63.0);
        assert_eq!(height(64), 64.0);
        assert_eq!(height(255), 4032.0);
        for v in 1..=255u8 {
            assert!(height(v) > height(v - 1));
        }
    }
}
