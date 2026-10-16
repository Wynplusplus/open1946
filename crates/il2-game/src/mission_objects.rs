//! Objects a mission places: `[NStationary]` vehicles, guns, ships and parked aircraft, and
//! `[Buildings]` houses and plates (`docs/formats/mission.md`).

use crate::objects::{Builder, Stand};
use il2_formats::mission::Mission;
use il2_formats::SectFile;
use il2_vfs::Vfs;

pub fn add_mission(b: &mut Builder, vfs: &Vfs, fms: &il2_data::FlightModels, m: &Mission, statics_ini: &SectFile) {
    let ini = |p: &str| vfs.read(p).map(|d| SectFile::parse(&d)).unwrap_or_default();
    let technics = ini("com/maddox/il2/objects/technics.ini");
    let ships = ini("com/maddox/il2/objects/ships.ini");
    let mesh_key = match b.season.as_str() {
        "winter" => "MeshWinter",
        "desert" => "MeshDesert",
        _ => "MeshSummer",
    };
    let mut skipped = Vec::new();
    let mut parked: std::collections::HashMap<String, Option<crate::world::Parked>> = Default::default();
    for s in &m.stationary {
        let Some((kind, short)) = s.class.split_once('$') else { continue };
        // Mission yaw is clockwise.
        let yaw = -s.yaw;
        let placed = match kind {
            "vehicles.stationary.Stationary" | "vehicles.artillery.Artillery" => technics.section(short).and_then(|sect| {
                let mesh = sect.get(mesh_key).or_else(|| sect.get("MeshSummer"))?.trim().to_owned();
                let c = b.class(&mesh, None)?;
                b.place(c, s.x, s.y, yaw, Stand::Ground { add: 0.0, align: true });
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
                            eprintln!("parked {class}: {:?}", p.as_ref().map(|p| (p.height, p.pitch)));
                        }
                        p
                    })
                    .as_ref()?;
                let c = b.class(&p.him, Some((class, &p.nodes)))?;
                b.place(c, s.x, s.y, yaw, Stand::Parked { height: p.height, pitch: p.pitch });
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
            Some((c, stand)) => b.place(c, h.x, h.y, -h.yaw, stand),
            None => skipped.push(full),
        }
    }
    b.skipped.extend(skipped);
}
