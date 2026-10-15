//! Assembles a playable scene from game data: map terrain, aircraft model and flight model.

use glam::{DQuat, DVec3};
use il2_fm::{Aircraft, GearPoint, Sim};
use il2_formats::map::HeightField;
use il2_formats::SectFile;
use il2_render::scene::Loader;
use il2_render::MeshData;
use il2_vfs::Vfs;

pub struct AircraftAssets {
    pub mesh: MeshData,
    pub fm: Aircraft,
}

/// Loads the height field of a map's `load.ini`.
pub fn load_heights(vfs: &Vfs, ini_path: &str) -> Result<HeightField, String> {
    let ini = SectFile::parse(&vfs.read(ini_path).map_err(|e| e.to_string())?);
    let dir = &ini_path[..ini_path.rfind('/').map_or(0, |i| i + 1)];
    let h = ini
        .section_ci("MAP")
        .and_then(|s| s.get_ci("HeightMap"))
        .map(|v| v.trim_start_matches(['=', ' ']).trim().to_owned())
        .ok_or("no HeightMap")?;
    let data = vfs.read(&format!("{dir}{h}")).map_err(|e| e.to_string())?;
    let img = image::load_from_memory_with_format(&data, image::ImageFormat::Tga).map_err(|e| e.to_string())?.to_luma8();
    Ok(HeightField::from_raw(img.width() as usize, img.height() as usize, img.as_raw()))
}

/// Wheel contact points: hook x/y positions, with heights solved from the flight model's parked
/// attitude (`[Gear] H`, `Pitch`) so all wheels touch the ground when parked.
fn gear_points(hooks: &[(String, glam::Vec3)], fmd: &SectFile) -> Vec<GearPoint> {
    let num = |k: &str, d: f64| {
        fmd.section_ci("Gear")
            .and_then(|s| s.get_ci(k))
            .and_then(il2_formats::sectfile::atof)
            .map_or(d, f64::from)
    };
    let (h, pitch) = (num("H", 2.0), num("Pitch", 10.0).to_radians());
    let hook = |n: &str| hooks.iter().find(|(name, _)| name.eq_ignore_ascii_case(n)).map(|(_, p)| *p);
    let mut out = Vec::new();
    for (name, castor) in [("_ClipLGear", false), ("_ClipRGear", false), ("_ClipCGear", true)] {
        if let Some(p) = hook(name) {
            let x = p.x as f64;
            // Lowered by the springs' static compression so the parked attitude matches H/Pitch.
            let z = -(h + x * pitch.sin()) / pitch.cos() - il2_fm::sim::GEAR_STATIC_SINK;
            out.push(GearPoint { pos: DVec3::new(x, p.y as f64, z), castor });
        }
    }
    out
}

pub fn load_aircraft(vfs: &Vfs, fms: &il2_data::FlightModels, him: &str, fmd_name: &str) -> Result<AircraftAssets, String> {
    let mut loader = Loader::new(vfs);
    loader.season = Some("summer".into());
    let mut mesh = loader.load_hier(him, false)?;
    let hooks = loader.hier_hooks(him)?;
    mesh.textures = loader.take_images();
    let fmd = fms.sect_file(fmd_name).map_err(|e| e.to_string())?;
    let gear = gear_points(&hooks, &fmd);
    let fm = Aircraft::load(&fmd, |fam| fms.sect_file(&format!("FlightModels/{fam}.emd")).ok(), gear).map_err(|e| e.to_string())?;
    Ok(AircraftAssets { mesh, fm })
}

/// Starts airborne over the map center, heading north.
pub fn spawn_airborne(ac: Aircraft, field: &HeightField, agl: f64, speed: f64) -> Sim {
    let (x, y) = (field.width as f64 * 100.0, field.height as f64 * 100.0);
    let z = field.height_at(x, y) + agl;
    let heading_north = DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2);
    Sim::new(ac, DVec3::new(x, y, z), heading_north, speed)
}
