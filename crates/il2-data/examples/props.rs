//! Prints a class's `Property.set` values: `props com.maddox.il2.objects.weapons.BombSC250`.
fn main() {
    let name = std::env::args().nth(1).expect("usage: props <class>");
    let vfs = il2_vfs::Vfs::open_install(std::env::var("IL2_ROOT").expect("IL2_ROOT")).unwrap();
    let fms = il2_data::FlightModels::open(&vfs).unwrap();
    let classes = il2_data::Classes::new(&vfs, &fms.keys);
    let mut p: Vec<_> = classes.class_properties(&name).into_iter().collect();
    p.sort_by(|a, b| a.0.cmp(&b.0));
    for (k, v) in p {
        println!("{k} = {v:?}");
    }
}
