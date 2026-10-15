//! International Standard Atmosphere (troposphere + lower stratosphere).

pub const G: f64 = 9.80665;
pub const RHO0: f64 = 1.225;

/// Temperature in kelvin at altitude `h` metres.
pub fn temperature(h: f64) -> f64 {
    288.15 - 0.0065 * h.clamp(0.0, 11000.0)
}

/// Air density in kg/m³.
pub fn density(h: f64) -> f64 {
    let h = h.max(0.0);
    if h <= 11000.0 {
        RHO0 * (temperature(h) / 288.15).powf(4.2559)
    } else {
        density(11000.0) * (-(h - 11000.0) / 6341.6).exp()
    }
}

pub fn sonic_speed(h: f64) -> f64 {
    (1.4 * 287.053 * temperature(h)).sqrt()
}

#[cfg(test)]
mod tests {
    #[test]
    fn reference_values() {
        assert!((super::density(0.0) - 1.225).abs() < 1e-9);
        assert!((super::density(5000.0) - 0.7364).abs() < 0.002);
        assert!((super::density(11000.0) - 0.3639).abs() < 0.002);
        assert!((super::sonic_speed(0.0) - 340.3).abs() < 0.2);
    }
}
