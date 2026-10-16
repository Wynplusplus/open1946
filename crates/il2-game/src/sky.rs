//! Sun position and sky colours from the mission time, date and map latitude. OpenIL2's own model
//! (standard solar-position formulas), not a port of the original's sky renderer.

use glam::Vec3;
use il2_render::Environment;

/// Direction towards the sun (x east, y north, z up) at local solar time `hours` on `day` of the
/// year, at latitude `lat_deg`.
pub fn sun_direction(hours: f64, day: f64, lat_deg: f64) -> Vec3 {
    let decl = (-23.44f64).to_radians() * ((360.0 / 365.0) * (day + 10.0)).to_radians().cos();
    let h = (15.0 * (hours - 12.0)).to_radians();
    let lat = lat_deg.to_radians();
    let east = -decl.cos() * h.sin();
    let north = decl.sin() * lat.cos() - decl.cos() * h.cos() * lat.sin();
    let up = decl.sin() * lat.sin() + decl.cos() * h.cos() * lat.cos();
    Vec3::new(east as f32, north as f32, up as f32).normalize()
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

pub fn environment(hours: f64, day: f64, lat_deg: f64) -> Environment {
    let sun = sun_direction(hours, day, lat_deg);
    let day_light = smoothstep(-0.08, 0.25, sun.z);
    // Warm horizon while the sun is low.
    let low = smoothstep(-0.12, 0.0, sun.z) * (1.0 - smoothstep(0.05, 0.3, sun.z));
    let night_h = [0.03, 0.04, 0.07];
    let night_z = [0.01, 0.015, 0.035];
    let day_h = Environment::default().horizon;
    let day_z = Environment::default().zenith;
    let horizon = mix(
        mix(night_h, day_h, day_light),
        [0.80, 0.52, 0.36],
        low * 0.8,
    );
    let zenith = mix(
        mix(night_z, day_z, day_light),
        [0.22, 0.26, 0.45],
        low * 0.5,
    );
    // Below the horizon, light comes from the moon/sky, from above.
    let lit_from = if sun.z > 0.0 {
        sun
    } else {
        Vec3::new(sun.x * 0.3, sun.y * 0.3, 1.0).normalize()
    };
    Environment {
        sun: lit_from,
        light: 0.18 + 0.82 * day_light,
        horizon,
        zenith,
    }
}

/// Day of the year for the middle of `day` in `month` (1-based).
pub fn day_of_year(month: u32, day: u32) -> f64 {
    const START: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    START[(month.clamp(1, 12) - 1) as usize] as f64 + day.clamp(1, 31) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noon_sun_is_south_and_high_in_summer() {
        let s = sun_direction(12.0, day_of_year(6, 21), 50.0);
        assert!(s.y < 0.0 && s.x.abs() < 1e-3);
        // 90 - 50 + 23.44 = 63.4 degrees elevation.
        assert!((s.z.asin().to_degrees() - 63.44).abs() < 0.3);
        // Morning sun is in the east, evening in the west, midnight below the horizon.
        assert!(sun_direction(8.0, 172.0, 50.0).x > 0.0);
        assert!(sun_direction(16.0, 172.0, 50.0).x < 0.0);
        assert!(sun_direction(0.0, 172.0, 50.0).z < 0.0);
    }
}
