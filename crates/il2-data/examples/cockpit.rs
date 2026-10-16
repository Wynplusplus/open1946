//! Prints an aircraft class's cockpit class and mesh: `cockpit air.BF_109G6`, or `cockpit --all`
//! for every class in `air.ini`.
fn main() {
    let name = std::env::args()
        .nth(1)
        .expect("usage: cockpit <class>|--all");
    let vfs = il2_vfs::Vfs::open_install(std::env::var("IL2_ROOT").expect("IL2_ROOT")).unwrap();
    let fms = il2_data::FlightModels::open(&vfs).unwrap();
    let classes = il2_data::Classes::new(&vfs, &fms.keys);
    if name != "--all" {
        println!("{:?}", classes.cockpit(&name));
        return;
    }
    let air = il2_formats::SectFile::parse(&vfs.read("com/maddox/il2/objects/air.ini").unwrap());
    let mut seen = std::collections::HashSet::new();
    let (mut n, mut found) = (0, 0);
    for (_, v) in air
        .section_ci("AIR")
        .map(|s| s.entries.as_slice())
        .unwrap_or_default()
    {
        let Some(class) = v.split_whitespace().next() else {
            continue;
        };
        if !seen.insert(class.to_owned()) {
            continue;
        }
        n += 1;
        match classes.cockpit(class) {
            Some(_) => found += 1,
            None => println!("{class}: no cockpit"),
        }
    }
    println!("{found} of {n} classes have a cockpit");
}
