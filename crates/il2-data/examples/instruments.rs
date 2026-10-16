//! Runs a cockpit's instrument code against a fixed flight state and prints the node angles and
//! the world values it asked for: `instruments air.BF_109G6`, or `instruments --all`.
use std::collections::HashMap;

fn main() {
    let name = std::env::args()
        .nth(1)
        .expect("usage: instruments <class>|--all");
    let vfs = il2_vfs::Vfs::open_install(std::env::var("IL2_ROOT").expect("IL2_ROOT")).unwrap();
    let fms = il2_data::FlightModels::open(&vfs).unwrap();
    let classes = il2_data::Classes::new(&vfs, &fms.keys);
    let world: HashMap<String, f64> = [
        ("fm.getAltitude()", 1500.0),
        ("fm.Loc.z", 1500.0),
        ("fm.getSpeedKMH()", 400.0),
        ("fm.EI.engines[0].getRPM()", 2400.0),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    let list: Vec<String> = if name == "--all" {
        let air =
            il2_formats::SectFile::parse(&vfs.read("com/maddox/il2/objects/air.ini").unwrap());
        let mut seen = std::collections::HashSet::new();
        air.section_ci("AIR")
            .map(|s| s.entries.as_slice())
            .unwrap_or_default()
            .iter()
            .filter_map(|(_, v)| v.split_whitespace().next().map(str::to_owned))
            .filter(|c| seen.insert(c.clone()))
            .collect()
    } else {
        vec![name.clone()]
    };
    let (mut ok, mut n) = (0, 0);
    for class in &list {
        let Some((cockpit, _)) = classes.cockpit(class) else {
            continue;
        };
        n += 1;
        let Some(mut ins) = il2_data::anim::Instruments::new(classes.cockpit_pool(&cockpit)) else {
            println!("{cockpit}: no classes");
            continue;
        };
        let init = ins.error.clone();
        let nodes = ins.update(world.clone(), 3).clone();
        if list.len() == 1 {
            for (k, v) in &nodes {
                println!("{k}: {:?} {:?} {:?}", v.angles, v.offset, v.visible);
            }
            println!("missing: {:#?}", ins.missing());
            println!(
                "init: {init:?}\nupdate: {:?}\nskipped: {:?}",
                ins.error,
                ins.soft_error()
            );
        } else {
            match &ins.error {
                None => ok += 1,
                Some(e) => println!("{cockpit}: {e}"),
            }
            if let Some(e) = ins.soft_error() {
                println!("{cockpit}: skipped {e}");
            }
            if std::env::var_os("MISSING").is_some() {
                for k in ins.missing() {
                    println!("missing {k}");
                }
            }
            if nodes.is_empty() {
                println!("{cockpit}: no nodes");
            }
        }
    }
    if list.len() > 1 {
        println!("{ok} of {n} cockpits update");
    }
}
