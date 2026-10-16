//! Dynamic campaign missions from the install's DGen data (`docs/behavior/campaign.md`, "Dynamic
//! campaigns"). OpenIL2's own generator: the runtime ships the original generator only as a
//! Windows program (`DGen.exe`).

use il2_formats::SectFile;
use il2_vfs::Vfs;
use std::collections::HashMap;
use std::path::Path;

/// A flight of an operation: count and role letters (`2Y`, `6Bx`, `4F`).
#[derive(Clone, Debug)]
pub struct Flight {
    pub n: u32,
    pub role: String,
}

#[derive(Clone, Debug)]
pub struct Op {
    pub name: String,
    /// Red's flights, blue's flights.
    pub sides: [Vec<Flight>; 2],
}

pub struct Theater {
    pub name: String,
    pub map: String,
    /// (weight, name)
    pub towns: Vec<(u32, String)>,
    /// Dates (`yMMDD`, year 194y) of the campaign's days.
    pub dates: Vec<u32>,
    /// Planes in service: (count, class without `air.`).
    pub planes: Vec<(u32, String)>,
    pub ops: Vec<Op>,
}

fn latin1(d: &[u8]) -> String {
    d.iter().map(|&b| b as char).collect()
}

/// Theaters: the `*.DB` files of `DGen/`.
pub fn theaters(root: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(root.join("DGen"))
        .map(|r| {
            r.flatten()
                .filter_map(|e| {
                    let p = e.path();
                    if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("db")) {
                        Some(p.file_stem()?.to_string_lossy().into_owned())
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

pub fn load(root: &Path, name: &str) -> Result<Theater, String> {
    let dir = root.join("DGen");
    let text = latin1(&std::fs::read(dir.join(format!("{name}.DB"))).map_err(|e| e.to_string())?);
    let mut lines = text.lines();
    let map = lines.next().unwrap_or("").trim().to_owned();
    let mut t = Theater {
        name: name.to_owned(),
        map,
        towns: Vec::new(),
        dates: Vec::new(),
        planes: Vec::new(),
        ops: Vec::new(),
    };
    let mut sect = String::new();
    for line in lines {
        let l = line.trim();
        if l.is_empty() || l.starts_with("//") {
            continue;
        }
        if l.starts_with('[') {
            sect = l[1..l.find(']').unwrap_or(l.len())].to_owned();
            if sect == "Operations" {
                // `[Operations] <<normal.dat easy.dat hard.dat`: the first (normal) file.
                if let Some(file) = l
                    .split("<<")
                    .nth(1)
                    .and_then(|f| f.split_whitespace().next())
                {
                    if let Ok(d) = std::fs::read(dir.join(file)) {
                        t.ops.extend(ops(&latin1(&d)));
                    }
                }
            }
            continue;
        }
        let mut tok = l.split_whitespace();
        match sect.as_str() {
            "Towns" => {
                if let (Some(w), Some(n)) = (tok.next().and_then(|w| w.parse().ok()), tok.next()) {
                    t.towns.push((w, n.to_owned()));
                }
            }
            "Schedule" => {
                if let Some(d) = tok.next().and_then(|d| d.parse().ok()) {
                    t.dates.push(d);
                }
            }
            "Planes" => {
                if let (Some(n), Some(c)) = (tok.next().and_then(|n| n.parse().ok()), tok.next()) {
                    t.planes.push((n, c.to_owned()));
                }
            }
            "Operations" => t.ops.extend(ops(l)),
            _ => {}
        }
    }
    if t.ops.is_empty() {
        return Err(format!("{name}: no operations"));
    }
    Ok(t)
}

/// `oRuRecon 10622:50509 VVS 2Y LW 2F`: name, date range, then each side's label and flights.
fn ops(text: &str) -> Vec<Op> {
    let mut out = Vec::new();
    for line in text.lines() {
        let l = line.split("//").next().unwrap_or("").trim();
        if !l.starts_with('o') {
            continue;
        }
        let mut sides: [Vec<Flight>; 2] = [Vec::new(), Vec::new()];
        let mut side: i32 = -1;
        let mut tok = l.split_whitespace();
        let name = tok.next().unwrap_or("").to_owned();
        for t in tok {
            if t.contains(':') {
                continue;
            }
            let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
            if digits.is_empty() {
                side += 1;
                continue;
            }
            if (0..2).contains(&side) {
                sides[side as usize].push(Flight {
                    n: digits.parse().unwrap_or(0),
                    role: t[digits.len()..].to_owned(),
                });
            }
        }
        out.push(Op { name, sides });
    }
    out
}

/// A small deterministic generator for one sortie.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Everything about the world a mission needs.
pub struct World<'a> {
    pub vfs: &'a Vfs,
    pub root: &'a Path,
    /// Class → (army, fighter, bomb loadout).
    pub plane_info: &'a dyn Fn(&str) -> (u8, bool, Option<String>),
}

struct Base {
    /// Sections to copy: (header, lines).
    chiefs: Vec<(String, String, Vec<String>)>,
    stationary: Vec<String>,
    /// Where its forces are.
    points: Vec<(f64, f64)>,
}

/// Reads `DGen/<Theater><Red|Blue>.mis`.
fn base(root: &Path, theater: &str, side: &str) -> Base {
    let text = std::fs::read(root.join("DGen").join(format!("{theater}{side}.mis")))
        .map(|d| latin1(&d))
        .unwrap_or_default();
    let f = SectFile::parse(text.as_bytes());
    let mut b = Base {
        chiefs: Vec::new(),
        stationary: Vec::new(),
        points: Vec::new(),
    };
    if let Some(s) = f.section_ci("Chiefs") {
        for (name, v) in &s.entries {
            let road: Vec<String> = f
                .section(&format!("{name}_Road"))
                .map(|r| r.entries.iter().map(|(k, v)| format!("{k} {v}")).collect())
                .unwrap_or_default();
            if let Some(p) = road.first() {
                let n: Vec<f64> = p
                    .split_whitespace()
                    .take(2)
                    .filter_map(|t| t.parse().ok())
                    .collect();
                if n.len() == 2 {
                    b.points.push((n[0], n[1]));
                }
            }
            b.chiefs.push((name.clone(), v.clone(), road));
        }
    }
    if let Some(s) = f.section_ci("NStationary") {
        for (k, v) in &s.entries {
            let n: Vec<f64> = v
                .split_whitespace()
                .skip(2)
                .take(2)
                .filter_map(|t| t.parse().ok())
                .collect();
            if n.len() == 2 {
                b.points.push((n[0], n[1]));
            }
            b.stationary.push(format!("{k} {v}"));
        }
    }
    b
}

fn centroid(p: &[(f64, f64)]) -> (f64, f64) {
    let n = p.len().max(1) as f64;
    (
        p.iter().map(|q| q.0).sum::<f64>() / n,
        p.iter().map(|q| q.1).sum::<f64>() / n,
    )
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// The sortie: the mission text and a one-line summary.
pub fn generate(
    w: &World,
    t: &Theater,
    army: u8,
    class: &str,
    sortie: u32,
    seed: u64,
) -> Result<(String, String), String> {
    let mut rng = Rng(seed | 1);
    let me = (army as usize).clamp(1, 2) - 1;
    // Operations where our side has the player's flight.
    let ops: Vec<&Op> = t
        .ops
        .iter()
        .filter(|o| o.sides[me].iter().any(|f| f.role.contains('Y')))
        .collect();
    let op = *ops
        .get(rng.below(ops.len()))
        .ok_or("no operation for this side")?;
    let red = base(w.root, &t.name, "Red");
    let blue = base(w.root, &t.name, "Blue");
    let forces = [&red, &blue];
    let centre = [centroid(&red.points), centroid(&blue.points)];
    // Who starts it: `oRu…` red, `oDe…` (and others) blue.
    let initiator =
        if op.name.starts_with("oRu") || op.name.starts_with("oUs") || op.name.starts_with("oGb") {
            0
        } else {
            1
        };
    let attack = [
        "Attack",
        "Bomb",
        "Recon",
        "Verify",
        "FreeHunt",
        "Intercept",
        "Rescue",
    ]
    .iter()
    .any(|k| op.name.contains(k));
    // The side whose ground the operation is over.
    let ground_side = if attack { 1 - initiator } else { initiator };
    let map = format!("maps/{}/load.ini", t.map);
    let runways = crate::world::load_runways(w.vfs, &map);
    let towns: HashMap<String, (f64, f64)> = w
        .vfs
        .read(&format!("maps/{}/texts.txt", t.map))
        .map(|d| {
            latin1(&d)
                .lines()
                .filter_map(|l| {
                    let mut it = l.split_whitespace();
                    let (x, y) = (it.next()?.parse().ok()?, it.next()?.parse().ok()?);
                    Some((it.last()?.to_owned(), (x, y)))
                })
                .collect()
        })
        .unwrap_or_default();
    // Airfields: the runways nearer each side's forces than the other's.
    let fields: Vec<(f64, f64, f64)> = runways
        .iter()
        .filter_map(|r| crate::world::takeoff_spot(&runways, r.x as f64, r.y as f64))
        .collect();
    let home = |side: usize| -> (f64, f64, f64) {
        let own = centre[side];
        let other = centre[1 - side];
        fields
            .iter()
            .copied()
            .filter(|f| dist((f.0, f.1), own) < dist((f.0, f.1), other))
            .min_by(|a, b| dist((a.0, a.1), own).total_cmp(&dist((b.0, b.1), own)))
            .or_else(|| fields.first().copied())
            .unwrap_or((own.0, own.1, 0.0))
    };
    // The target.
    let target_forces = forces[ground_side];
    let near_front = |p: (f64, f64)| dist(p, centre[1 - ground_side]);
    let kind = ["AF", "Armor", "Soft", "Cluster", "Bridge", "Town"]
        .iter()
        .find(|k| op.name.ends_with(*k))
        .copied()
        .unwrap_or("Area");
    let mut target_chief: Option<String> = None;
    let target: (f64, f64) = match kind {
        "AF" => {
            let h = home(ground_side);
            (h.0, h.1)
        }
        "Armor" | "Soft" | "Cluster" => {
            let want = if kind == "Armor" {
                "Armor."
            } else {
                "Vehicles."
            };
            let mut cands: Vec<(usize, f64)> = target_forces
                .chiefs
                .iter()
                .enumerate()
                .filter(|(_, c)| c.1.contains(want) || kind == "Cluster")
                .filter_map(|(i, c)| {
                    let n: Vec<f64> =
                        c.2.first()?
                            .split_whitespace()
                            .take(2)
                            .filter_map(|t| t.parse().ok())
                            .collect();
                    Some((i, near_front((n[0], n[1]))))
                })
                .collect();
            cands.sort_by(|a, b| a.1.total_cmp(&b.1));
            cands.truncate(5);
            match cands.get(rng.below(cands.len())) {
                Some(&(i, _)) => {
                    let c = &target_forces.chiefs[i];
                    target_chief = Some(format!("{}{}", ["r", "b"][ground_side], c.0));
                    let n: Vec<f64> = c.2[0]
                        .split_whitespace()
                        .take(2)
                        .filter_map(|t| t.parse().ok())
                        .collect();
                    (n[0], n[1])
                }
                None => centre[ground_side],
            }
        }
        _ => {
            // A town of the theater on the right side of the front, near it.
            let mut cands: Vec<((f64, f64), f64)> = t
                .towns
                .iter()
                .filter_map(|(_, n)| towns.get(n).copied())
                .filter(|&p| {
                    dist(p, centre[ground_side]) < dist(p, centre[1 - ground_side]) + 5000.0
                })
                .map(|p| (p, near_front(p)))
                .collect();
            cands.sort_by(|a, b| a.1.total_cmp(&b.1));
            cands.truncate(4);
            cands
                .get(rng.below(cands.len()))
                .map_or(centre[ground_side], |c| c.0)
        }
    };
    // Aircraft pools per side.
    let mut pools: [Vec<(String, bool, Option<String>)>; 2] = [Vec::new(), Vec::new()];
    for (_, c) in &t.planes {
        let full = format!("air.{c}");
        let (a, fighter, bombs) = (w.plane_info)(&full);
        if (1..=2).contains(&a) {
            pools[a as usize - 1].push((full, fighter, bombs));
        }
    }
    let alt = 1500.0 + 500.0 * rng.below(5) as f64;
    let mut wings: Vec<(String, String)> = Vec::new();
    let mut player_wing = String::new();
    let date = t
        .dates
        .get((sortie as usize / 2).min(t.dates.len().saturating_sub(1)))
        .copied()
        .unwrap_or(10622);
    let (year, month, day) = (1940 + date / 10000, (date / 100) % 100, date % 100);
    for side in 0..2 {
        let (hx, hy, _) = home(side);
        for (k, f) in op.sides[side].iter().enumerate().take(16) {
            if f.n == 0 {
                continue;
            }
            let name = format!("{}{}{}", ["r01", "g01"][side], k / 4, k % 4);
            let player = f.role.contains('Y') && side == me && player_wing.is_empty();
            let wants_bombs = player && kind != "Area" && attack && !f.role.contains('F')
                || ["B", "A", "T", "R", "P", "U"]
                    .iter()
                    .any(|r| f.role.trim_end_matches('x').ends_with(r));
            let (plane, loadout) = if player {
                (
                    (format!("air.{}", class.trim_start_matches("air."))),
                    (w.plane_info)(&format!("air.{}", class.trim_start_matches("air.")))
                        .2
                        .filter(|_| wants_bombs),
                )
            } else {
                let pool: Vec<&(String, bool, Option<String>)> = pools[side]
                    .iter()
                    .filter(|p| {
                        p.1 == !wants_bombs || pools[side].iter().all(|q| q.1 != !wants_bombs)
                    })
                    .collect();
                let Some(p) = pool.get(rng.below(pool.len())) else {
                    continue;
                };
                (p.0.clone(), p.2.clone().filter(|_| wants_bombs))
            };
            if player {
                player_wing = name.clone();
            }
            let n = f.n.min(4);
            let goes = side == initiator;
            let (tx, ty) = target;
            let mut way: Vec<String> = vec![format!("TAKEOFF {hx:.2} {hy:.2} 0 0")];
            let dx = tx - hx;
            let dy = ty - hy;
            let d = dx.hypot(dy).max(1.0);
            let (ux, uy) = (dx / d, dy / d);
            way.push(format!(
                "NORMFLY {:.2} {:.2} {:.0} 300",
                hx + ux * 3000.0,
                hy + uy * 3000.0,
                alt * 0.5
            ));
            if goes {
                way.push(format!(
                    "NORMFLY {:.2} {:.2} {alt:.0} 330",
                    tx - ux * 8000.0,
                    ty - uy * 8000.0
                ));
                if loadout.is_some() {
                    let tgt = target_chief
                        .as_ref()
                        .map(|c| format!(" {c} 0"))
                        .unwrap_or_default();
                    way.push(format!("GATTACK {tx:.2} {ty:.2} {alt:.0} 330{tgt}"));
                } else {
                    way.push(format!("NORMFLY {tx:.2} {ty:.2} {alt:.0} 330"));
                }
                way.push(format!(
                    "NORMFLY {:.2} {:.2} {alt:.0} 330",
                    tx + uy * 5000.0,
                    ty - ux * 5000.0
                ));
            } else {
                // Defenders patrol over the target.
                for a in 0..4 {
                    let ang = a as f64 * std::f64::consts::FRAC_PI_2 + rng.unit();
                    way.push(format!(
                        "NORMFLY {:.2} {:.2} {:.0} 330",
                        tx + ang.cos() * 3000.0,
                        ty + ang.sin() * 3000.0,
                        alt + 500.0
                    ));
                }
            }
            way.push(format!(
                "NORMFLY {:.2} {:.2} 600 300",
                hx + ux * 4000.0,
                hy + uy * 4000.0
            ));
            way.push(format!("LANDING {hx:.2} {hy:.2} 0 0"));
            let skill = if player { 2 } else { 1 + rng.below(2) };
            let body = format!(
                "  Planes {n}\n  Skill {skill}\n  Class {plane}\n  Fuel 100\n  weapons {}\n",
                loadout.unwrap_or_else(|| "default".into())
            );
            wings.push((
                name.clone(),
                format!(
                    "[{name}]\n{body}[{name}_Way]\n{}\n",
                    way.iter()
                        .map(|w| format!("  {w}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
            ));
        }
    }
    if player_wing.is_empty() {
        return Err("the operation has no flight for the player".into());
    }
    let hour = 7 + rng.below(10);
    let mut m = String::new();
    m += &format!("[MAIN]\n  MAP {}/load.ini\n  TIME {hour}.0\n  CloudType 1\n  CloudHeight 1500.0\n  army {}\n  playerNum 0\n  player {player_wing}\n", t.map, me + 1);
    m += &format!("[SEASON]\n  Year {year}\n  Month {month}\n  Day {day}\n");
    m += "[Wing]\n";
    for (n, _) in &wings {
        m += &format!("  {n}\n");
    }
    for (_, s) in &wings {
        m += s;
    }
    m += "[Chiefs]\n";
    for (side, b) in forces.iter().enumerate() {
        for (name, v, _) in &b.chiefs {
            m += &format!("  {}{name} {v}\n", ["r", "b"][side]);
        }
    }
    for (side, b) in forces.iter().enumerate() {
        for (name, _, road) in &b.chiefs {
            m += &format!("[{}{name}_Road]\n", ["r", "b"][side]);
            for p in road {
                m += &format!("  {p}\n");
            }
        }
    }
    m += "[NStationary]\n";
    for (side, b) in forces.iter().enumerate() {
        for s in &b.stationary {
            m += &format!("  {}{s}\n", ["r", "b"][side]);
        }
    }
    // The player's objective.
    let (tx, ty) = target;
    let objective = match (me == initiator, attack, &target_chief) {
        (true, true, Some(c)) if kind != "Area" => format!("0 0 0 0 500 {tx:.0} {ty:.0} 0 0 {c}"),
        (true, true, None) if kind != "Area" => format!("1 0 0 0 500 {tx:.0} {ty:.0} 1000"),
        (false, _, _) => format!("6 0 0 0 500 {tx:.0} {ty:.0} 2000"),
        _ => format!("3 0 0 0 0 {tx:.0} {ty:.0} 3000"),
    };
    m += &format!("[Target]\n  {objective}\n");
    let town = t
        .towns
        .iter()
        .filter_map(|(_, n)| Some((n, towns.get(n)?)))
        .min_by(|a, b| dist(*a.1, target).total_cmp(&dist(*b.1, target)))
        .map_or(String::new(), |(n, _)| format!(" near {n}"));
    let summary = format!(
        "{} {}{town}, {day}.{month}.{year}, {hour}:00",
        op.name.trim_start_matches('o'),
        kind
    );
    Ok((m, summary))
}
