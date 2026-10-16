//! Objects a mission places: `[NStationary]` vehicles, guns, ships and parked aircraft, and
//! `[Buildings]` houses and plates (`docs/formats/mission.md`).

use crate::objects::{Builder, Stand};
use il2_formats::mission::Mission;
use il2_formats::SectFile;
use il2_vfs::Vfs;

pub fn add_mission(
    b: &mut Builder,
    vfs: &Vfs,
    fms: &il2_data::FlightModels,
    m: &Mission,
    statics_ini: &SectFile,
) {
    let ini = |p: &str| vfs.read(p).map(|d| SectFile::parse(&d)).unwrap_or_default();
    let technics = ini("com/maddox/il2/objects/technics.ini");
    let ships = ini("com/maddox/il2/objects/ships.ini");
    let mesh_key = match b.season.as_str() {
        "winter" => "MeshWinter",
        "desert" => "MeshDesert",
        _ => "MeshSummer",
    };
    let mut skipped = Vec::new();
    let mut parked: std::collections::HashMap<String, Option<crate::world::Parked>> =
        Default::default();
    for s in &m.stationary {
        let Some((kind, short)) = s.class.split_once('$') else {
            continue;
        };
        // Mission yaw is clockwise.
        let yaw = -s.yaw;
        let placed = match kind {
            "vehicles.stationary.Stationary" | "vehicles.artillery.Artillery" => {
                technics.section(short).and_then(|sect| {
                    let mesh = sect
                        .get(mesh_key)
                        .or_else(|| sect.get("MeshSummer"))?
                        .trim()
                        .to_owned();
                    let c = b.class(&mesh, None)?;
                    b.place(
                        c,
                        s.x,
                        s.y,
                        yaw,
                        Stand::Ground {
                            add: 0.0,
                            align: true,
                        },
                    );
                    Some(())
                })
            }
            // Barrage and observation balloons: anchor, balloon `Height` m above the ground, and the
            // rope as one segment mesh repeated every `RopeSegLength` m.
            "vehicles.aeronautics.Aeronautics" => technics.section(short).and_then(|sect| {
                let suffix = &mesh_key[4..];
                let mesh = |part: &str| {
                    sect.get(&format!("Mesh{part}{suffix}"))
                        .or_else(|| sect.get(&format!("Mesh{part}Summer")))
                        .map(|m| m.trim().to_owned())
                };
                let num = |k: &str, d: f64| {
                    sect.get(k)
                        .and_then(il2_formats::sectfile::atof)
                        .map_or(d, f64::from)
                };
                let (height, seg) = (
                    num("Height", 25.0).clamp(25.0, 4001.0),
                    num("RopeSegLength", 10.0).clamp(10.0, 1000.0),
                );
                let anchor = b.class(&mesh("Anchor")?, None)?;
                b.place(
                    anchor,
                    s.x,
                    s.y,
                    yaw,
                    Stand::Ground {
                        add: 0.0,
                        align: true,
                    },
                );
                let ground = b.ground(s.x, s.y);
                if let Some(c) = mesh("Balloon").and_then(|m| b.class(&m, None)) {
                    b.place(c, s.x, s.y, yaw, Stand::Absolute(ground + height));
                }
                if let Some(c) = mesh("Rope").and_then(|m| b.class(&m, None)) {
                    // Segment meshes are centred on their origin.
                    let n = 1 + (height / seg) as usize;
                    for i in 0..n {
                        b.place(
                            c,
                            s.x,
                            s.y,
                            yaw,
                            Stand::Absolute(ground + height - i as f64 * seg - seg / 2.0),
                        );
                    }
                }
                Some(())
            }),
            "ships.Ship" => ships.section(short).and_then(|sect| {
                let c = b.class(sect.get("Mesh")?.trim(), None)?;
                b.place(c, s.x, s.y, yaw, Stand::Absolute(0.0));
                Some(())
            }),
            "vehicles.planes.Plane" => technics.section(short).and_then(|sect| {
                let class = sect.get("Class")?.trim();
                let p = parked
                    .entry(class.to_owned())
                    .or_insert_with(|| {
                        let p = crate::world::parked_model(vfs, fms, class);
                        if std::env::var_os("IL2_TRACE").is_some() {
                            eprintln!(
                                "parked {class}: {:?}",
                                p.as_ref().map(|p| (p.height, p.pitch))
                            );
                        }
                        p
                    })
                    .as_ref()?;
                let c = b.class(&p.him, Some((class, &p.nodes)))?;
                b.place(
                    c,
                    s.x,
                    s.y,
                    yaw,
                    Stand::Parked {
                        height: p.height,
                        pitch: p.pitch,
                    },
                );
                Some(())
            }),
            _ => None,
        };
        if placed.is_none() {
            skipped.push(s.class.clone());
        }
    }
    for h in &m.buildings {
        let full = format!("com.maddox.il2.objects.buildings.{}", h.class);
        match b.house_class(statics_ini, &full, h.flag != 0, true) {
            Some((c, stand, profile)) => b.place_destructible(c, h.x, h.y, -h.yaw, stand, profile),
            // A destroyed house without a `MeshDead` is not drawn by the runtime either.
            None if h.flag == 0 && b.house_class(statics_ini, &full, true, true).is_some() => {}
            None => skipped.push(full),
        }
    }
    b.skipped.extend(skipped);
}

#[cfg(test)]
mod tests {
    /// Every shipped mission: all placed objects resolve, except effect classes (smoke, fires, sirens,
    /// searchlights, beacons) and `Furniture$*`. Run with `IL2_ROOT=/path/to/game cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn all_missions_place_objects() {
        let root = std::path::PathBuf::from(std::env::var("IL2_ROOT").unwrap());
        let vfs = il2_vfs::Vfs::open_install(&root).unwrap();
        let fms = il2_data::FlightModels::open(&vfs).unwrap();
        let statics_ini = crate::objects::statics_ini(&vfs).unwrap();
        let flat = il2_formats::map::HeightField::from_raw(2, 2, &[0; 4]);
        let mut files = Vec::new();
        let mut stack = vec![root.join("Missions")];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("mis")) {
                    files.push(p);
                }
            }
        }
        let effect = |c: &str| {
            [
                "Smoke$",
                "Campfire$",
                "Siren$",
                "Searchlight$",
                "Beacon$",
                "Furniture$",
            ]
            .iter()
            .any(|e| c.contains(e))
        };
        let mut bad = std::collections::BTreeSet::new();
        let mut b = crate::objects::Builder::new(&vfs, &flat, "summer");
        for f in &files {
            let m = il2_formats::mission::Mission::parse(&std::fs::read(f).unwrap());
            b.skipped.clear();
            super::add_mission(&mut b, &vfs, &fms, &m, &statics_ini);
            bad.extend(b.skipped.iter().filter(|c| !effect(c)).cloned());
        }
        assert!(
            bad.is_empty(),
            "{} missions; unresolved: {bad:?}",
            files.len()
        );
    }
}
