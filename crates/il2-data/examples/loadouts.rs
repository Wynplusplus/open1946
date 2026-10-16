//! Prints an aircraft class's weapon slots and loadouts: `loadouts air.BF_109G6`.
fn main() {
    let name = std::env::args().nth(1).expect("usage: loadouts <class>");
    let vfs = il2_vfs::Vfs::open_install(std::env::var("IL2_ROOT").expect("IL2_ROOT")).unwrap();
    let fms = il2_data::FlightModels::open(&vfs).unwrap();
    let classes = il2_data::Classes::new(&vfs, &fms.keys);
    let w = classes.aircraft_weapons(&name).expect("class not found");
    println!("triggers {:?}\nhooks {:?}", w.triggers, w.hooks);
    for (n, slots) in &w.loadouts {
        let s: Vec<String> = slots.iter().flatten().map(|(g, c)| format!("{g} {c}")).collect();
        println!("{n}: {}", s.join(", "));
    }
}
