//! Run with `IL2_ROOT=/path/to/game cargo test -- --ignored`.
use il2_data::FlightModels;

#[test]
#[ignore]
fn reads_bf109_flight_model_and_engine() {
    let vfs = il2_vfs::Vfs::open_install(std::env::var("IL2_ROOT").unwrap()).unwrap();
    let fms = FlightModels::open(&vfs).unwrap();
    assert!(fms.len() > 300);
    let fm = fms.sect_file("FlightModels/Bf-109G-6Early.fmd").unwrap();
    assert_eq!(fm.get("Engine", "Engine0Family"), Some("DB-600_Series"));
    assert!(fm.section("Polares").is_some());
    let emd = fms.sect_file("FlightModels/DB-600_Series.emd").unwrap();
    assert!(emd.section("DB605D").is_some());
}

#[test]
#[ignore]
fn aircraft_class_assets() {
    let vfs = il2_vfs::Vfs::open_install(std::env::var("IL2_ROOT").unwrap()).unwrap();
    let fms = FlightModels::open(&vfs).unwrap();
    let c = il2_data::Classes::new(&vfs, &fms.keys).aircraft("air.BF_109G6").expect("class");
    assert_eq!(c.flight_model, "FlightModels/Bf-109G-6Early.fmd");
    assert!(c.meshes[0].to_ascii_lowercase().ends_with("bf-109g-6early/hier.him"), "{:?}", c.meshes);
}

#[test]
#[ignore]
fn bf109_animations_import() {
    let vfs = il2_vfs::Vfs::open_install(std::env::var("IL2_ROOT").unwrap()).unwrap();
    let fms = FlightModels::open(&vfs).unwrap();
    let tracks = il2_data::Classes::new(&vfs, &fms.keys).animations("air.BF_109F4");
    for t in &tracks {
        println!("{:?} imported={} nodes={:?}", t.channel, t.imported, t.nodes.keys().collect::<Vec<_>>());
    }
    let gear = tracks.iter().find(|t| t.channel == il2_data::anim::Channel::Gear).unwrap();
    assert!(gear.imported);
    let down: Vec<_> = gear.eval(1.0).collect();
    let l3 = down.iter().find(|(n, _)| *n == "GearL3_D0").unwrap().1;
    assert!((l3.angles[1] + 77.5).abs() < 0.01, "{l3:?}");
    let flaps = tracks.iter().find(|t| t.channel == il2_data::anim::Channel::Flaps).unwrap();
    assert!(flaps.imported);
}
