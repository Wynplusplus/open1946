//! How many aircraft classes from air.ini have each animation channel imported.
fn main() {
    let vfs = il2_vfs::Vfs::open_install(std::env::var("IL2_ROOT").expect("IL2_ROOT")).unwrap();
    let fms = il2_data::FlightModels::open(&vfs).unwrap();
    let classes = il2_data::Classes::new(&vfs, &fms.keys);
    let ini =
        String::from_utf8_lossy(&vfs.read("com/maddox/il2/objects/air.ini").unwrap()).into_owned();
    let mut names: Vec<String> = ini
        .split_whitespace()
        .filter(|t| t.starts_with("air."))
        .map(str::to_owned)
        .collect();
    names.sort();
    names.dedup();
    let mut counts = [0usize; 5];
    let mut missing_gear = Vec::new();
    for n in &names {
        for (i, t) in classes.animations(n).iter().enumerate() {
            if t.imported {
                counts[i] += 1;
            } else if i == 4 {
                missing_gear.push(n.clone());
            }
        }
    }
    println!("{} aircraft classes", names.len());
    for (ch, c) in il2_data::anim::Channel::ALL.iter().zip(counts) {
        println!("{ch:?}: {c} imported");
    }
    println!(
        "gear not imported (first 15): {:?}",
        &missing_gear[..missing_gear.len().min(15)]
    );
}
