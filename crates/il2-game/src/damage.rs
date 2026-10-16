//! Damage to houses and other map objects (`docs/behavior/damage.md`).

use crate::weapons::Rng;

/// Wall thickness range per material (`Panzer` values for the two energy rows).
const THICKNESS: [[f64; 2]; 4] = [[0.025, 0.15], [0.12, 0.24], [0.002, 0.008], [0.025, 0.15]];
/// Shot energy (J) for 10 %, 30 % and 100 % kill probability, per material and thickness row.
const ENERGY: [[[f64; 3]; 2]; 4] = [
    [[23_400.0, 131_400.0, 252_000.0], [131_400.0, 369_000.0, 1_224_000.0]],
    [[131_400.0, 252_000.0, 3_295_500.0], [369_000.0, 3_295_500.0, 5_120_000.0]],
    [[511.225, 2453.88, 23_400.0], [2453.88, 10_140.0, 23_400.0]],
    [[23_400.0, 131_400.0, 252_000.0], [131_400.0, 369_000.0, 1_224_000.0]],
];

/// How a house class takes damage, from its `Body` and `Panzer` (`static.ini`).
#[derive(Clone, Copy, Debug)]
pub struct HouseDamage {
    /// 0 wood, 1 stone, 2 fuel, 3 flesh.
    pub mat: usize,
    pub panzer: f64,
    pub min_tnt: f64,
    pub max_tnt: f64,
    pub p_death: f64,
}

impl HouseDamage {
    pub fn new(body: &str, panzer: f64) -> Option<Self> {
        let p = panzer;
        let (mat, min, k, pd) = match body.to_ascii_lowercase().as_str() {
            "woodsmall" => (0, 0.25 * p / 0.025, 2.0, 0.01),
            "woodmiddle" => (0, 5.0 * p / 0.15, 1.7, 0.01),
            "rockmiddle" => (1, 6.0 * p / 0.12, 1.7, 0.01),
            "rockbig" => (1, 12.0 * p / 0.24, 1.7, 0.05),
            "rockhuge" => (1, 30.0 * p / 0.48, 1.7, 0.05),
            "fuelsmall" => (2, 0.2 * p / 0.002, 1.7, 0.01),
            "fuelbig" => (2, 0.8 * p / 0.008, 1.7, 0.01),
            "flesh" => (3, 0.01 * p / 0.002, 1.7, 0.1),
            _ => return None,
        };
        Some(HouseDamage { mat, panzer: p, min_tnt: min, max_tnt: min * k, p_death: pd })
    }

    fn penetrate(&self, row: usize, e: f64) -> f64 {
        let t = ENERGY[self.mat][row];
        if e < t[1] {
            if e < t[0] {
                return 0.0;
            }
            let k = 0.2 / (t[1] - t[0]);
            e * k + (0.1 - t[0] * k)
        } else {
            if e >= t[2] {
                return 1.0;
            }
            let k = 0.7 / (t[2] - t[1]);
            e * k + (0.3 - t[1] * k)
        }
    }

    /// Kill probability of `hits` shots of energy `e`, interpolated by wall thickness.
    pub fn kill_probability(&self, e: f64, hits: u32) -> f64 {
        if hits == 0 {
            return 0.0;
        }
        let (p0, p1) = (self.penetrate(0, e), self.penetrate(1, e));
        let th = THICKNESS[self.mat];
        let k = (p1 - p0) / (th[1] - th[0]);
        let p = self.panzer * k + (p0 - th[0] * k);
        if p < 0.1 {
            0.0
        } else if p >= 1.0 {
            1.0
        } else if hits > 1 {
            1.0 - (1.0 - p).powi(hits as i32)
        } else {
            p
        }
    }

    /// A bullet hit with kinetic energy `e` (J) and the runtime's shot type (0 high explosive,
    /// 1 shaped charge, 2 plain, 3 with filler) at speed `v`.
    pub fn shot_kills(&self, e: f64, power_type: i32, v: f64, rng: &mut Rng) -> bool {
        if e <= 0.0 {
            return false;
        }
        match (self.mat, power_type) {
            (3, 1) => true,
            (3, _) => v >= 20.0,
            (2, 1) => {
                let x = e * (0.75 + 0.4 * rng.uniform());
                self.panzer <= 0.256 * x.sqrt().sqrt()
            }
            (_, 1) => false,
            _ => rng.uniform() < self.kill_probability(e, 1),
        }
    }

    /// A blast delivering `received` kg TNT.
    pub fn explosion_kills(&self, received: f64, rng: &mut Rng) -> bool {
        if received <= self.min_tnt {
            false
        } else if received >= self.max_tnt {
            true
        } else {
            let f = (received - self.min_tnt) / (self.max_tnt - self.min_tnt);
            rng.uniform() < self.p_death + (1.0 - self.p_death) * f
        }
    }
}

/// TNT reaching an object of collision radius `r` at distance `d` (realistic bomb sights, the full
/// realism setting): `power / ((d − 0.7r)/2)^1.5`, all of it within 2 m.
pub fn received_power_realistic(power: f64, d: f64, r: f64) -> f64 {
    let x = 0.5 * (d - 0.7 * r);
    if x < 1.0 {
        power
    } else {
        power / (x * x * x).sqrt()
    }
}

/// TNT reaching an object of collision radius `r` at distance `d` from a blast of `power` kg and
/// `radius` m without realistic bomb sights: `(1 − (d − r)/radius)²·power`.
#[allow(dead_code)]
pub fn received_power(power: f64, radius: f64, d: f64, r: f64) -> f64 {
    let x = d - r;
    if x >= radius {
        return 0.0;
    }
    let k = (1.0 - x / radius).powi(2);
    if k >= 1.0 {
        power
    } else {
        k * power
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heavier_hits_kill_more_often() {
        // A thin wooden shed: rifle rounds do nothing, a 37 mm round often kills.
        let wood = HouseDamage::new("WoodSmall", 0.03).unwrap();
        assert_eq!(wood.kill_probability(1000.0, 1), 0.0);
        let p20 = wood.kill_probability(0.5 * 0.115 * 800.0 * 800.0, 1);
        let p37 = wood.kill_probability(0.5 * 0.73 * 900.0 * 900.0, 1);
        assert!(p37 > p20, "{p20} {p37}");
        assert!(received_power(1.0, 10.0, 5.0, 2.0) < 1.0);
        assert_eq!(received_power(1.0, 10.0, 15.0, 2.0), 0.0);
    }
}
