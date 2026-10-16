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
}

#[derive(Clone, Debug)]
pub struct Wing {
    pub name: String,
    pub class: String,
    pub planes: u32,
    pub fuel_percent: f64,
    pub waypoints: Vec<Waypoint>,
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
    let Some(s) = f.section_ci(sect) else { return Vec::new() };
    s.entries
        .iter()
        .filter_map(|(name, v)| {
            let mut t = v.split_whitespace();
            let class = t.next()?.to_owned();
            let mut num = || t.next().map_or(0.0, |v| v.parse::<f64>().ok().or_else(|| atof(v).map(f64::from)).unwrap_or(0.0));
            let (flag, x, y, yaw) = (num() as i32, num(), num(), num());
            Some(Placed { name: name.clone(), class, flag, x, y, yaw })
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
    pub wings: Vec<Wing>,
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
                                let n: Vec<f64> = v.split_whitespace().take(4).map(|t| atof(t).map_or(0.0, f64::from)).collect();
                                let at = |i: usize| n.get(i).copied().unwrap_or(0.0);
                                Waypoint { kind: kind.clone(), x: at(0), y: at(1), z: at(2), speed: at(3) }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Some(Wing {
                    class: s.get_ci("Class")?.trim().to_owned(),
                    planes: num("Planes", 1.0) as u32,
                    fuel_percent: num("Fuel", 100.0),
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
            wings,
        }
    }

    pub fn player(&self) -> Option<&Wing> {
        let p = self.player_wing.as_deref()?;
        self.wings.iter().find(|w| w.name == p)
    }
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
        assert_eq!((m.stationary[0].flag, m.stationary[0].x, m.stationary[0].yaw), (1, 8015.45, 580.0));
        assert_eq!((m.buildings[0].flag, m.buildings[0].y), (0, 20.0));
    }
}
