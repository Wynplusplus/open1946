//! Checks that control surfaces deflect the physically right way for positive inputs.
#[path = "../src/cockpit.rs"]
#[allow(dead_code)]
mod cockpit;
#[path = "../src/world.rs"]
#[allow(dead_code)]
mod world;
use glam::Vec3;

fn main() {
    let vfs = il2_vfs::Vfs::open_install(std::env::var("IL2_ROOT").unwrap()).unwrap();
    let fms = il2_data::FlightModels::open(&vfs).unwrap();
    let class = std::env::args().nth(1).unwrap_or("air.BF_109F4".into());
    let a = world::load_aircraft(&vfs, &fms, &class).unwrap();
    let rest = a.rig.pose(&[]);
    // Trailing-edge probe: 0.4 m aft of each node origin, in object space.
    let probe = |m: &glam::Mat4, rest_m: &glam::Mat4| {
        let p_obj = rest_m.transform_point3(Vec3::ZERO) + Vec3::new(-0.4, 0.0, 0.0);
        m.transform_point3(rest_m.inverse().transform_point3(p_obj)) - p_obj
    };
    let check = |label: &str, ct: il2_fm::Inputs, node: &str| {
        let Some(i) = a.rig.find(node) else {
            return println!("{label}: no node {node}");
        };
        let posed = world::pose(&a, &ct);
        let d = probe(&posed[i], &rest[i]);
        println!(
            "{label:28} {node:12} trailing edge moves dz={:+.3} dy={:+.3}",
            d.z, d.y
        );
    };
    let z = il2_fm::Inputs {
        gear: 1.0,
        ..Default::default()
    };
    check(
        "elevator +1 (want dz>0)",
        il2_fm::Inputs { elevator: 1.0, ..z },
        "VatorL_D0",
    );
    check(
        "aileron +1 left (want dz<0)",
        il2_fm::Inputs { aileron: 1.0, ..z },
        "AroneL_D0",
    );
    check(
        "aileron +1 right (want dz>0)",
        il2_fm::Inputs { aileron: 1.0, ..z },
        "AroneR_D0",
    );
    check(
        "rudder +1 (want dy<0)",
        il2_fm::Inputs { rudder: 1.0, ..z },
        "Rudder1_D0",
    );
    check(
        "flaps 1 (want dz<0)",
        il2_fm::Inputs { flaps: 1.0, ..z },
        "Flap01_D0",
    );
}
