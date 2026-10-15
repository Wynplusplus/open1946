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

#[cfg(test)]
mod tests {
    use super::height;

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
