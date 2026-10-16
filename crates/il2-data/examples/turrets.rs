//! Prints a class's turret arcs (`turretAngles`): `turrets air.HE_111H2 5`.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let vfs = il2_vfs::Vfs::open_install(std::env::var("IL2_ROOT").expect("IL2_ROOT")).unwrap();
    let fms = il2_data::FlightModels::open(&vfs).unwrap();
    let classes = il2_data::Classes::new(&vfs, &fms.keys);
    let n = a.get(2).and_then(|n| n.parse().ok()).unwrap_or(1);
    match classes.turret_limits(&a[1], n) {
        None => println!("no turret limits"),
        Some(t) => {
            for (i, grid) in t.iter().enumerate() {
                println!("turret {i} (rows pitch +90 at top, yaw -180..180 left to right):");
                for row in grid.iter().rev() {
                    println!(
                        "  {}",
                        row.iter()
                            .map(|&b| if b { '#' } else { '.' })
                            .collect::<String>()
                    );
                }
            }
        }
    }
}
