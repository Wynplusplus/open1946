//! Missions (`.mis`): `docs/formats/mission.md`.

use crate::sectfile::{atof, SectFile};

#[derive(Clone, Debug, PartialEq)]
pub struct Waypoint {
    pub kind: String,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// km/h
    pub speed: f64,
    /// The wing or object this waypoint is about (escort, attack), with the target's waypoint index.
    pub target: Option<(String, u32)>,
}

#[derive(Clone, Debug)]
pub struct Wing {
    pub name: String,
    pub class: String,
    pub planes: u32,
    pub fuel_percent: f64,
    /// Loadout name (`weapons`), e.g. `default`.
    pub weapons: String,
    pub waypoints: Vec<Waypoint>,
    /// AI skill per plane: 0 rookie, 1 average, 2 veteran, 3 ace (`Skill`, or `Skill0`..`Skill3`).
    pub skills: Vec<u8>,
}

/// A point of a ground or naval column's route (`[<name>_Road]`).
#[derive(Clone, Debug, PartialEq)]
pub struct RoadPoint {
    pub x: f64,
    pub y: f64,
    /// Road width (negative: a bridge segment index).
    pub width: f64,
    /// Time to wait here, minutes.
    pub wait: f64,
    /// The mission builder's speed for the leg from here, m/s, when given.
    pub speed: Option<f64>,
}

/// A moving column (`[Chiefs]`): `name type army`, e.g. `0_Chief Armor.3-BT5 1`.
#[derive(Clone, Debug)]
pub struct Chief {
    pub name: String,
    /// `Category.Type` in `com/maddox/il2/objects/chief.ini`.
    pub kind: String,
    pub army: u8,
    pub road: Vec<RoadPoint>,
}

/// A placed object from `[NStationary]` or `[Buildings]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub name: String,
    /// Class relative to `com.maddox.il2.objects.` (`[NStationary]`, e.g. `ships.Ship$USSEssexCV9`) or
    /// to `com.maddox.il2.objects.buildings.` (`[Buildings]`, e.g. `House$Hangar1`).
    pub class: String,
    /// `[NStationary]`: army; `[Buildings]`: 1 alive, 0 destroyed.
    pub flag: i32,
    pub x: f64,
    pub y: f64,
    /// Degrees, **clockwise** from east (the runtime passes it through `Orient.set`).
    pub yaw: f64,
}

fn placed(f: &SectFile, sect: &str) -> Vec<Placed> {
    let Some(s) = f.section_ci(sect) else {
        return Vec::new();
    };
    s.entries
        .iter()
        .filter_map(|(name, v)| {
            let mut t = v.split_whitespace();
            let class = t.next()?.to_owned();
            let mut num = || {
                t.next().map_or(0.0, |v| {
                    v.parse::<f64>()
                        .ok()
                        .or_else(|| atof(v).map(f64::from))
                        .unwrap_or(0.0)
                })
            };
            let (flag, x, y, yaw) = (num() as i32, num(), num(), num());
            Some(Placed {
                name: name.clone(),
                class,
                flag,
                x,
                y,
                yaw,
            })
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct Mission {
    pub stationary: Vec<Placed>,
    pub buildings: Vec<Placed>,
    /// Relative to `maps/`.
    pub map: String,
    pub time_hours: f64,
    /// `[SEASON] Month`, `Day` if given.
    pub date: Option<(u32, u32)>,
    pub player_wing: Option<String>,
    /// The player's plane in the wing (`playerNum`).
    pub player_num: u32,
    pub wings: Vec<Wing>,
    pub chiefs: Vec<Chief>,
}

impl Mission {
    pub fn parse(data: &[u8]) -> Self {
        let f = SectFile::parse(data);
        let main = f.section_ci("MAIN");
        let get = |k: &str| main.and_then(|s| s.get_ci(k)).map(str::trim);
        let wings = f
            .section_ci("Wing")
            .map(|s| s.entries.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|name| {
                let s = f.section(&name)?;
                let num = |k: &str, d: f64| s.get_ci(k).and_then(atof).map_or(d, f64::from);
                let waypoints = f
                    .section(&format!("{name}_Way"))
                    .map(|w| {
                        w.entries
                            .iter()
                            .map(|(kind, v)| {
                                let n: Vec<f64> = v
                                    .split_whitespace()
                                    .take(4)
                                    .map(|t| atof(t).map_or(0.0, f64::from))
                                    .collect();
                                let at = |i: usize| n.get(i).copied().unwrap_or(0.0);
                                let rest: Vec<&str> = v.split_whitespace().skip(4).collect();
                                let target =
                                    rest.first().filter(|t| !t.starts_with('&')).map(|t| {
                                        (
                                            t.to_string(),
                                            rest.get(1).and_then(|i| i.parse().ok()).unwrap_or(0),
                                        )
                                    });
                                Waypoint {
                                    kind: kind.clone(),
                                    x: at(0),
                                    y: at(1),
                                    z: at(2),
                                    speed: at(3),
                                    target,
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let base = num("Skill", 1.0) as u8;
                let skills = (0..4)
                    .map(|i| num(&format!("Skill{i}"), base as f64) as u8)
                    .collect();
                Some(Wing {
                    skills,
                    class: s.get_ci("Class")?.trim().to_owned(),
                    planes: num("Planes", 1.0) as u32,
                    fuel_percent: num("Fuel", 100.0),
                    weapons: s
                        .get_ci("weapons")
                        .map_or("default".to_owned(), |w| w.trim().to_owned()),
                    waypoints,
                    name,
                })
            })
            .collect();
        Mission {
            stationary: placed(&f, "NStationary"),
            buildings: placed(&f, "Buildings"),
            map: get("MAP").unwrap_or_default().to_owned(),
            time_hours: get("TIME").and_then(atof).map_or(12.0, f64::from),
            date: f.section_ci("SEASON").and_then(|s| {
                let n = |k: &str| s.get_ci(k).and_then(atof).map(|v| v as u32);
                Some((n("Month")?, n("Day").unwrap_or(15)))
            }),
            player_wing: get("player").map(str::to_owned),
            player_num: get("playerNum").and_then(atof).map_or(0, |v| v as u32),
            wings,
            chiefs: chiefs(&f),
        }
    }

    pub fn player(&self) -> Option<&Wing> {
        let p = self.player_wing.as_deref()?;
        self.wings.iter().find(|w| w.name == p)
    }
}

fn chiefs(f: &SectFile) -> Vec<Chief> {
    let Some(sect) = f.section_ci("Chiefs") else {
        return Vec::new();
    };
    sect.entries
        .iter()
        .filter_map(|(name, v)| {
            let mut t = v.split_whitespace();
            let kind = t.next()?.to_owned();
            let army = t.next().and_then(|a| a.parse().ok()).unwrap_or(1);
            let road = f
                .section(&format!("{name}_Road"))
                .map(|r| {
                    r.entries
                        .iter()
                        .filter_map(|(k, v)| {
                            let n: Vec<f64> = std::iter::once(k.as_str())
                                .chain(v.split_whitespace())
                                .map(|t| atof(t).map_or(f64::NAN, f64::from))
                                .collect();
                            (n.len() >= 3 && n[0].is_finite() && n[1].is_finite()).then(|| {
                                RoadPoint {
                                    x: n[0],
                                    y: n[1],
                                    width: n[2],
                                    wait: n
                                        .get(3)
                                        .copied()
                                        .filter(|w| w.is_finite())
                                        .unwrap_or(0.0),
                                    speed: if n.len() >= 6 {
                                        n.last().copied().filter(|v| v.is_finite() && *v > 0.0)
                                    } else {
                                        None
                                    },
                                }
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(Chief {
                name: name.clone(),
                kind,
                army,
                road,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_player_wing() {
        let m = super::Mission::parse(
            b"[MAIN]\n  MAP Kuban/load.ini\n  TIME 7.08\n  player sk0100\n[Wing]\n  sk0100\n[sk0100]\n  Planes 2\n  Class air.BF_109F4\n  Fuel 80\n\
              [sk0100_Way]\n  TAKEOFF 173452.33 132971.58 0 0 &0\n  NORMFLY 170000 140000 1500.00 400.00 &0\n",
        );
        assert_eq!(m.map, "Kuban/load.ini");
        let w = m.player().unwrap();
        assert_eq!(w.class, "air.BF_109F4");
        assert_eq!(w.planes, 2);
        assert_eq!(w.waypoints[0].kind, "TAKEOFF");
        assert_eq!(w.waypoints[1].z, 1500.0);
        let m = super::Mission::parse(b"[NStationary]\n  0_Static ships.Ship$USSEssexCV9 1 8015.45 4946.58 580.00 0.0 0 2\n[Buildings]\n  0_bld House$Hangar 0 10 20 90\n");
        assert_eq!(m.stationary[0].class, "ships.Ship$USSEssexCV9");
        assert_eq!(
            (m.stationary[0].flag, m.stationary[0].x, m.stationary[0].yaw),
            (1, 8015.45, 580.0)
        );
        assert_eq!((m.buildings[0].flag, m.buildings[0].y), (0, 20.0));
    }
}
