//! Lift and drag polars: `docs/behavior/flight-model.md`, "Polars". Angles in degrees.

#[derive(Clone, Debug)]
pub struct PolarParams {
    pub line_cy: f64,
    pub aoa_min_cx_shift: f64,
    pub decline: f64,
    pub max_dist_ang: f64,
    pub parab_angle: f64,
    /// Clean and flapped configurations.
    pub cfg: [PolarCfg; 2],
}

#[derive(Clone, Copy, Debug)]
pub struct PolarCfg {
    pub cy0: f64,
    pub aoa_crit_h: f64,
    pub aoa_crit_l: f64,
    pub cy_crit_h: f64,
    pub cy_crit_l: f64,
    pub cx_min: f64,
    pub parab_cx: f64,
}

/// Wave drag tables (`[Polares] mc3, mc4, mm, mz`), prepared as the runtime does.
#[derive(Clone, Debug)]
pub struct MachTable {
    mc: [f64; 8],
    mm: [f64; 8],
    mc2: [f64; 8],
    mc3: [f64; 8],
    mc4: [f64; 8],
    md: [f64; 8],
    mc_min: f64,
}

impl MachTable {
    /// From the four 8-entry lists; `None` if any is missing or short (no wave drag then).
    pub fn new(mc3: &[f64], mc4: &[f64], mm: &[f64], mz: &[f64]) -> Option<Self> {
        if [mc3, mc4, mm, mz].iter().any(|t| t.len() < 8) {
            return None;
        }
        let mut t = MachTable {
            mc: [0.0; 8],
            mm: [0.0; 8],
            mc2: [0.0; 8],
            mc3: [0.0; 8],
            mc4: [0.0; 8],
            md: [0.0; 8],
            mc_min: 999.0,
        };
        for i in 0..8 {
            let z = mz[i];
            t.mm[i] = mm[i];
            t.mc4[i] = mc4[i] * z.powi(4);
            t.mc3[i] = mc3[i] * z.powi(3);
            t.mc2[i] = 0.25 * (z - 6.0 * t.mc3[i] - 8.0 * t.mc4[i]);
            t.mc[i] = mm[i] - z;
            t.mc_min = t.mc_min.min(t.mc[i]);
            t.md[i] = t.mc4[i] + t.mc3[i] + t.mc2[i] - z * 0.5;
        }
        Some(t)
    }

    /// Wave drag coefficient at Mach `m` (uncapped) for lift coefficient `cya`.
    pub fn cdw(&self, m: f64, cya: f64) -> f64 {
        if m < self.mc_min {
            return 0.0;
        }
        let x = cya + 0.9;
        let f = x * 3.333333;
        let i = f as i64;
        let fr = f - i as f64;
        if x >= 0.0 && i < 7 {
            let i = i as usize;
            let c = self.mc[i] + (self.mc[i + 1] - self.mc[i]) * fr;
            let mmax = self.mm[i] + (self.mm[i + 1] - self.mm[i]) * fr;
            if m < c {
                0.0
            } else if m < mmax {
                let u = (m - c) / (mmax - c);
                let a = (u * self.mc4[i] + self.mc3[i]) * u + self.mc2[i];
                let b = (u * self.mc4[i + 1] + self.mc3[i + 1]) * u + self.mc2[i + 1];
                (a + (b - a) * fr) * u * u
            } else {
                self.md[i] + (self.md[i + 1] - self.md[i]) * fr + 0.5 * (m - c)
            }
        } else {
            let i = if x < 0.0 { 0 } else { (i as usize).min(7) };
            let d = m - self.mc[i];
            if d < 0.0 {
                0.0
            } else if m < self.mm[i] {
                let u = d / (self.mm[i] - self.mc[i]);
                ((u * self.mc4[i] + self.mc3[i]) * u + self.mc2[i]) * u * u
            } else {
                0.5 * d + self.md[i]
            }
        }
    }
}

/// A polar evaluated for a given flap setting.
#[derive(Clone, Debug)]
pub struct Polar {
    p: PolarParams,
    cy0: f64,
    pub aoa_crit_h: f64,
    pub aoa_crit_l: f64,
    cy_crit_h: f64,
    cy_crit_l: f64,
    cx_min: f64,
    parab_cx: f64,
    line_h: f64,
    line_l: f64,
    parab_h: f64,
    parab_l: f64,
    aoa_min_cx: f64,
}

fn sin_deg(a: f64) -> f64 {
    a.to_radians().sin()
}

/// 0.9·sin(k·α) shape used beyond the stall, `k` in radians per degree.
fn deep(k: f64, a: f64) -> f64 {
    0.9 * (k * a).sin()
}

const K1: f64 = 0.039_269_91; // π/80
const K2: f64 = 0.031_415_93; // π/100

impl Polar {
    pub fn new(p: PolarParams) -> Self {
        let c = p.cfg[0];
        let mut s = Polar {
            p,
            cy0: c.cy0,
            aoa_crit_h: 0.0,
            aoa_crit_l: 0.0,
            cy_crit_h: 0.0,
            cy_crit_l: 0.0,
            cx_min: 0.0,
            parab_cx: 0.0,
            line_h: 0.0,
            line_l: 0.0,
            parab_h: 0.0,
            parab_l: 0.0,
            aoa_min_cx: 0.0,
        };
        s.set_flaps(0.0);
        s
    }

    /// Fixed polar without flap dependence.
    pub fn fixed(line_cy: f64, cy_crit: f64, aoa_crit: f64, cx_min: f64) -> Self {
        let c = PolarCfg {
            cy0: 0.0,
            aoa_crit_h: aoa_crit,
            aoa_crit_l: -aoa_crit,
            cy_crit_h: cy_crit,
            cy_crit_l: -cy_crit,
            cx_min,
            parab_cx: 6e-4,
        };
        Polar::new(PolarParams {
            line_cy,
            aoa_min_cx_shift: 0.0,
            decline: 0.007,
            max_dist_ang: 30.0,
            parab_angle: 5.0,
            cfg: [c, c],
        })
    }

    pub fn set_flaps(&mut self, f: f64) {
        let f = f.clamp(0.0, 1.0);
        let k = 0.25 * f + 0.75 * f.sqrt();
        let [a, b] = self.p.cfg;
        let lerp = |x: f64, y: f64, t: f64| x + (y - x) * t;
        self.cy0 = lerp(a.cy0, b.cy0, k);
        self.cy_crit_h = lerp(a.cy_crit_h, b.cy_crit_h, k);
        self.cy_crit_l = lerp(a.cy_crit_l, b.cy_crit_l, k);
        self.aoa_crit_h = lerp(a.aoa_crit_h, b.aoa_crit_h, k);
        self.aoa_crit_l = lerp(a.aoa_crit_l, b.aoa_crit_l, k);
        let lc = self.p.line_cy;
        self.line_h = 2.0 * (self.cy_crit_h - self.cy0) / lc - self.aoa_crit_h;
        self.parab_h = 0.5 * lc / (self.aoa_crit_h - self.line_h);
        self.line_l = 2.0 * (self.cy_crit_l - self.cy0) / lc - self.aoa_crit_l;
        self.parab_l = 0.5 * lc / (self.line_l - self.aoa_crit_l);
        self.cx_min = lerp(a.cx_min, b.cx_min, f);
        self.parab_cx = lerp(a.parab_cx, b.parab_cx, f);
        self.aoa_min_cx = self.p.aoa_min_cx_shift - self.cy0 / lc;
    }

    /// Lift coefficient along the wind.
    pub fn cya(&self, a: f64) -> f64 {
        if a <= self.line_h && a >= self.line_l {
            return self.cy0 + self.p.line_cy * a;
        }
        if a > 0.0 {
            if a <= self.aoa_crit_h {
                let d = self.aoa_crit_h - a;
                self.cy_crit_h - self.parab_h * d * d
            } else if a <= 40.0 {
                let mda = self.p.max_dist_ang;
                if a <= mda {
                    let d = a - self.aoa_crit_h;
                    if d < self.p.parab_angle {
                        self.cy_crit_h - self.p.decline * d * d
                    } else {
                        let end = deep(K1, mda);
                        let span = mda - self.p.parab_angle - self.aoa_crit_h;
                        let top = self.cy_crit_h
                            - self.p.decline * self.p.parab_angle * self.p.parab_angle
                            - end;
                        let d = mda - a;
                        end + top / (span * span) * d * d
                    }
                } else {
                    deep(K1, a)
                }
            } else if a <= 140.0 {
                let (sign, a) = if a > 90.0 {
                    (-1.0, 180.0 - a)
                } else {
                    (1.0, a)
                };
                sign * 0.9 * (std::f64::consts::FRAC_PI_2 + K2 * (a - 40.0)).sin()
            } else {
                -deep(K1, 180.0 - a)
            }
        } else if a >= self.aoa_crit_l {
            let d = a - self.aoa_crit_l;
            self.cy_crit_l + self.parab_l * d * d
        } else if a >= -40.0 {
            let d = self.aoa_crit_l - a;
            (self.cy_crit_l + 0.007 * d * d).min(-deep(K1, -a))
        } else {
            let a = -a;
            if a <= 140.0 {
                let (sign, a) = if a > 90.0 {
                    (1.0, 180.0 - a)
                } else {
                    (-1.0, a)
                };
                sign * 0.9 * (std::f64::consts::FRAC_PI_2 + K2 * (a - 40.0)).sin()
            } else {
                deep(K1, 180.0 - a)
            }
        }
    }

    /// Drag coefficient along the wind.
    pub fn cxa(&self, a: f64) -> f64 {
        let d = a - self.aoa_min_cx;
        let mut cx = self.cx_min + self.parab_cx * d * d;
        if (-6.0..=6.0).contains(&a) {
            return cx;
        }
        if a >= self.aoa_crit_h {
            cx += 0.03 * (a - self.aoa_crit_h);
        } else if a <= self.aoa_crit_l {
            cx += 0.03 * (self.aoa_crit_l - a);
        }
        cx.min(0.2 + 1.2 * sin_deg(a).abs())
    }

    /// Body-axis (Cx, Cy) at angle of attack `a`.
    pub fn coeffs(&self, a: f64) -> (f64, f64) {
        self.coeffs_mach(a, 0.0, None)
    }

    /// Body-axis (Cx, Cy) with wave drag at (uncapped) Mach `m`.
    pub fn coeffs_mach(&self, a: f64, m: f64, mach: Option<&MachTable>) -> (f64, f64) {
        let cya = self.cya(a);
        let cxa = self.cxa(a) + mach.map_or(0.0, |t| t.cdw(m, cya));
        let (s, c) = a.to_radians().sin_cos();
        (cxa * c - cya * s, cya * c + cxa * s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wing() -> Polar {
        Polar::new(PolarParams {
            line_cy: 0.095,
            aoa_min_cx_shift: 0.2,
            decline: 0.008,
            max_dist_ang: 38.0,
            parab_angle: 6.0,
            cfg: [
                PolarCfg {
                    cy0: 0.15,
                    aoa_crit_h: 21.0,
                    aoa_crit_l: -16.0,
                    cy_crit_h: 1.48,
                    cy_crit_l: -1.02,
                    cx_min: 0.0275,
                    parab_cx: 6.2e-4,
                },
                PolarCfg {
                    cy0: 0.63,
                    aoa_crit_h: 17.0,
                    aoa_crit_l: -19.0,
                    cy_crit_h: 1.72,
                    cy_crit_l: -0.7,
                    cx_min: 0.118,
                    parab_cx: 8.7e-4,
                },
            ],
        })
    }

    #[test]
    fn lift_curve_is_continuous_and_peaks_at_critical() {
        let p = wing();
        assert!((p.cya(0.0) - 0.15).abs() < 1e-9);
        assert!((p.cya(21.0) - 1.48).abs() < 1e-9);
        let mut prev = p.cya(-179.9);
        let mut a = -179.9;
        while a < 179.9 {
            a += 0.1;
            let c = p.cya(a);
            assert!((c - prev).abs() < 0.05, "jump at {a}: {prev} -> {c}");
            prev = c;
        }
        assert!(p.cya(20.0) < p.cya(21.0) && p.cya(23.0) < p.cya(21.0));
    }

    #[test]
    fn flaps_raise_lift() {
        let mut p = wing();
        let clean = p.cya(5.0);
        p.set_flaps(1.0);
        assert!(p.cya(5.0) > clean + 0.3);
        assert!(p.cxa(5.0) > 0.1);
    }
}
