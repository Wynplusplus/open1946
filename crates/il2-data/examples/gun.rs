//! Prints a weapon class's evaluated properties: `gun MGunMG131si`.
fn main() {
    let name = std::env::args().nth(1).expect("usage: gun <WeaponClass>");
    let vfs = il2_vfs::Vfs::open_install(std::env::var("IL2_ROOT").expect("IL2_ROOT")).unwrap();
    let fms = il2_data::FlightModels::open(&vfs).unwrap();
    let classes = il2_data::Classes::new(&vfs, &fms.keys);
    match classes.gun_properties(&name) {
        Ok(v) => println!("{v:#?}"),
        Err(e) => println!("error: {e}"),
    }
}
