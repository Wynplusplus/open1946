//! Assembles a playable scene from game data: map terrain, aircraft model and flight model.

use glam::{DQuat, DVec3};
use il2_fm::{Aircraft, ContactKind, GearPoint, Sim};
use il2_formats::map::HeightField;
use il2_formats::SectFile;
use il2_render::scene::Loader;
use il2_render::MeshData;
use il2_vfs::Vfs;

pub struct AircraftAssets {
    /// The model's hierarchy file.
    pub him: String,
    /// Hook frames in body coordinates (rest pose): gun muzzles, wheel and airframe points, …
    pub hook_frames: Vec<(String, glam::Mat4)>,
    /// The pilot's cockpit, in body coordinates.
    pub cockpit: Option<crate::cockpit::Cockpit>,
    pub mesh: MeshData,
    pub rig: il2_render::scene::Rig,
    pub tracks: Vec<il2_data::anim::Track>,
    pub fm: Aircraft,
    /// `[Gear] H` and `Pitch` (degrees): parked origin height and attitude.
    pub parked: (f64, f64),
    /// Nodes hidden at the start (`Hidden` in the hierarchy): damage states, caps, …
    pub hidden: Vec<bool>,
    /// The chunk each of `fm.gear`'s contact points belongs to.
    pub gear_chunks: Vec<String>,
    /// Twin-fin layout (`Scheme2a`, `Scheme5`, `Scheme7`).
    pub twin_fin: bool,
    /// Per node: the box around its geometry in node space (for bullet hits).
    pub boxes: Vec<Option<(glam::Vec3, glam::Vec3)>>,
    /// `[Toughness]` per part (`il2_fm::damage::PART_NAMES`), × 10⁻⁴; `NullPart` can't be damaged.
    pub toughness: [f64; 44],
    /// The class implements `TypeFighter`.
    pub fighter: bool,
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
    let img = image::load_from_memory_with_format(&data, image::ImageFormat::Tga)
        .map_err(|e| e.to_string())?
        .to_luma8();
    Ok(HeightField::from_raw(
        img.width() as usize,
        img.height() as usize,
        img.as_raw(),
    ))
}

/// Wheel contact points (`docs/behavior/gear.md`, "Contact points"): the gear-down hooks when the gear
/// animation moves them, else hook x/y with heights solved from `[Gear] H`/`Pitch`.
fn gear_points(hooks: &[(String, glam::Vec3)], fmd: &SectFile, gear_moves: bool) -> Vec<GearPoint> {
    let num = |k: &str, d: f64| {
        fmd.section_ci("Gear")
            .and_then(|s| s.get_ci(k))
            .and_then(il2_formats::sectfile::atof)
            .map_or(d, f64::from)
    };
    let (h, pitch) = (num("H", 2.0), num("Pitch", 10.0).to_radians());
    let hook = |n: &str| {
        hooks
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(n))
            .map(|(_, p)| *p)
    };
    let mut out = Vec::new();
    for (name, kind) in [
        ("_ClipLGear", ContactKind::Wheel),
        ("_ClipRGear", ContactKind::Wheel),
        ("_ClipCGear", ContactKind::Castor),
    ] {
        if let Some(p) = hook(name) {
            let x = p.x as f64;
            let z = if gear_moves {
                p.z as f64
            } else {
                // Lowered by the springs' static compression so the parked attitude matches H/Pitch.
                -(h + x * pitch.sin()) / pitch.cos() - il2_fm::sim::GEAR_STATIC_SINK
            };
            out.push(GearPoint {
                pos: DVec3::new(x, p.y as f64, z),
                kind,
            });
        }
    }
    // Airframe collision points: hooks named _Clip followed by digits.
    for (name, p) in hooks {
        if name.len() > 5
            && name[..5].eq_ignore_ascii_case("_clip")
            && name[5..].chars().all(|c| c.is_ascii_digit())
        {
            out.push(GearPoint {
                pos: p.as_dvec3(),
                kind: ContactKind::Body,
            });
        }
    }
    out
}

/// The hooks `gear_points` takes its points from, in the same order.
fn gear_hook_names(hooks: &[(String, glam::Vec3)]) -> Vec<String> {
    let mut out: Vec<String> = ["_ClipLGear", "_ClipRGear", "_ClipCGear"]
        .iter()
        .filter(|n| hooks.iter().any(|(h, _)| h.eq_ignore_ascii_case(n)))
        .map(|n| n.to_string())
        .collect();
    for (name, _) in hooks {
        if name.len() > 5
            && name[..5].eq_ignore_ascii_case("_clip")
            && name[5..].chars().all(|c| c.is_ascii_digit())
        {
            out.push(name.clone());
        }
    }
    out
}

/// Parked origin height and nose-up pitch (degrees) from the wheel points, like the runtime's land
/// pose: the line through the main-wheel mid-point and the third wheel is level; the height is
/// lowered by the springs' sink `m·g/2 700 000`.
fn land_pose(gear: &[GearPoint], mass: f64) -> Option<(f64, f64)> {
    let wheels: Vec<&GearPoint> = gear
        .iter()
        .filter(|g| g.kind == ContactKind::Wheel)
        .collect();
    let c = gear.iter().find(|g| g.kind == ContactKind::Castor)?;
    if wheels.len() < 2 {
        return None;
    }
    let (mx, mz) = (
        (wheels[0].pos.x + wheels[1].pos.x) * 0.5,
        (wheels[0].pos.z + wheels[1].pos.z) * 0.5,
    );
    let (dx, dz) = (mx - c.pos.x, mz - c.pos.z);
    let mut pitch = -dz.atan2(dx).to_degrees();
    if dx < 0.0 {
        pitch += 180.0;
    }
    let h = (dx * c.pos.z - dz * c.pos.x).abs() / dx.hypot(dz).max(1e-6);
    Some((h - mass * 9.80665 / 2_700_000.0, pitch))
}

/// Loads an aircraft by runtime class name (e.g. `air.BF_109G6`).
pub fn load_aircraft(
    vfs: &Vfs,
    fms: &il2_data::FlightModels,
    class: &str,
) -> Result<AircraftAssets, String> {
    load_aircraft_with(vfs, fms, class, true)
}

/// Like `load_aircraft`; `with_cockpit: false` skips the cockpit (AI aircraft).
pub fn load_aircraft_with(
    vfs: &Vfs,
    fms: &il2_data::FlightModels,
    class: &str,
    with_cockpit: bool,
) -> Result<AircraftAssets, String> {
    let classes = il2_data::Classes::new(vfs, &fms.keys);
    let info = classes
        .aircraft(class)
        .ok_or_else(|| format!("aircraft class {class} not found"))?;
    let him = info
        .meshes
        .first()
        .ok_or("aircraft class names no model")?
        .clone();
    let fmd_name = info.flight_model.as_str();
    let tracks = classes.animations(class);
    let mut loader = Loader::new(vfs);
    loader.season = Some("summer".into());
    // Every node is loaded, hidden damage states too; `hidden` says which show at first.
    let mut mesh = loader.load_hier(&him, true)?;
    let rig = loader.hier_rig(&him)?;
    let hidden = il2_formats::him::Hier::parse(&vfs.read(&him).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?
        .nodes
        .iter()
        .map(|n| n.hidden)
        .collect();
    // Wheel hooks are taken with the gear extended (the meshes are modelled retracted).
    let mut assets_for_pose = AircraftAssets {
        him: him.clone(),
        hook_frames: Vec::new(),
        cockpit: None,
        mesh: MeshData::default(),
        rig: rig.clone(),
        tracks: tracks.clone(),
        fm: Aircraft::empty(),
        parked: (0.0, 0.0),
        hidden: Vec::new(),
        gear_chunks: Vec::new(),
        twin_fin: false,
        boxes: Vec::new(),
        toughness: [0.01; 44],
        fighter: false,
    };
    let down = pose(
        &assets_for_pose,
        &il2_fm::Inputs {
            gear: 1.0,
            ..Default::default()
        },
    );
    assets_for_pose.tracks.clear();
    let hooks = loader.hier_hooks_posed(&him, &down)?;
    mesh.textures = loader.take_images();
    let fmd = fms.sect_file(fmd_name).map_err(|e| e.to_string())?;
    let rest = assets_for_pose.rig.pose(&[]);
    let gear_moves = down
        .iter()
        .zip(&rest)
        .any(|(a, b)| !a.abs_diff_eq(*b, 1e-4));
    let from_ini = fmd
        .section_ci("Gear")
        .and_then(|s| s.get_ci("FromIni"))
        .and_then(il2_formats::sectfile::atof)
        .unwrap_or(0.0)
        != 0.0;
    // Use the hooks only if they give a plausible stance: some classes' gear animations import only in
    // part (docs/behavior/gear.md, "Contact points").
    let fmd_pitch = fmd
        .section_ci("Gear")
        .and_then(|s| s.get_ci("Pitch"))
        .and_then(il2_formats::sectfile::atof)
        .map_or(10.0, f64::from);
    let hook_points = gear_points(&hooks, &fmd, true);
    let plausible = land_pose(&hook_points, 0.0).is_some_and(|(_, p)| {
        let nose = hook_points
            .iter()
            .any(|g| g.kind == ContactKind::Castor && g.pos.x > -1.0);
        if nose {
            p.abs() <= 6.0
        } else {
            (3.0..=25.0).contains(&p) && (p - fmd_pitch).abs() <= 4.0
        }
    });
    let use_hooks = gear_moves && !from_ini && plausible;
    let gear = if use_hooks {
        hook_points
    } else {
        gear_points(&hooks, &fmd, false)
    };
    if std::env::var_os("IL2_GEAR").is_some() {
        eprintln!("gear {class}: moves {gear_moves} from_ini {from_ini} plausible {plausible} use_hooks {use_hooks}");
        for g in &gear {
            if g.kind != ContactKind::Body {
                eprintln!("  {:?} {:.2?}", g.kind, g.pos);
            }
        }
    }
    let mut fm = Aircraft::load(
        &fmd,
        |fam| fms.sect_file(&format!("FlightModels/{fam}.emd")).ok(),
        gear,
    )
    .map_err(|e| e.to_string())?;
    // The runtime picks these rocket burn rates by aircraft class (docs/behavior/engine.md).
    let short = class.rsplit('.').next().unwrap_or(class);
    let lineage = classes.lineage(class);
    let is = |names: &[&str]| lineage.iter().any(|c| names.contains(&c.as_str()));
    let radiator = if is(&[
        "GLADIATOR",
        "DXXI_DK",
        "DXXI_DU",
        "DXXI_SARJA3_EARLY",
        "DXXI_SARJA3_LATE",
        "DXXI_SARJA3_SARVANTO",
    ]) {
        il2_fm::engine::RadiatorMode::Closed
    } else if is(&[
        "P_51", "P_38", "YAK_3", "YAK_3P", "YAK_9M", "YAK_9U", "YAK_9UT", "P_63C",
    ]) {
        il2_fm::engine::RadiatorMode::OilThermostat
    } else if is(&["SPITFIRE9", "SPITFIRE8", "SPITFIRE8CLP"]) {
        il2_fm::engine::RadiatorMode::WaterThermostat
    } else {
        il2_fm::engine::RadiatorMode::Governor
    };
    for e in &mut fm.engines {
        e.radiator_mode = radiator;
        e.rocket_burn = match short {
            "BI_1" | "BI_6" => 1.8,
            "MXY_7" => 0.5,
            _ => e.rocket_burn,
        };
    }
    let num = |k: &str, d: f64| {
        fmd.section_ci("Gear")
            .and_then(|s| s.get_ci(k))
            .and_then(il2_formats::sectfile::atof)
            .map_or(d, f64::from)
    };
    let mass = fm.mass_empty + fm.fuel_max + fm.nitro_max;
    let parked = if use_hooks {
        land_pose(&fm.gear, mass)
    } else {
        None
    }
    .unwrap_or((num("H", 2.0), num("Pitch", 10.0)));
    let hook_frames = loader.hier_hook_frames(&him, &rest).unwrap_or_default();
    let cockpit = with_cockpit
        .then(|| crate::cockpit::Cockpit::load(vfs, &classes, class, &him))
        .flatten();
    let owners = loader.hier_hook_owners(&him).unwrap_or_default();
    let gear_chunks = gear_hook_names(&hooks)
        .iter()
        .map(|h| {
            owners
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(h))
                .map_or_else(|| "CF_D0".to_owned(), |(_, node)| node.clone())
        })
        .collect();
    let twin_fin = is(&["Scheme2a", "Scheme5", "Scheme7"]);
    let mut boxes: Vec<Option<(glam::Vec3, glam::Vec3)>> = vec![None; rig.names.len()];
    for v in &mesh.vertices {
        let Some(b) = boxes.get_mut(v.node as usize) else {
            continue;
        };
        let p = glam::Vec3::from(v.pos);
        *b = Some(b.map_or((p, p), |(lo, hi)| (lo.min(p), hi.max(p))));
    }
    let mut toughness = [0.01; 44];
    for (i, part) in il2_fm::damage::PART_NAMES.iter().enumerate() {
        let v = fmd
            .get("Toughness", part)
            .and_then(|v| v.trim().parse::<f64>().ok())
            .unwrap_or(100.0);
        toughness[i] = v * 1e-4;
    }
    toughness[43] = f64::INFINITY;
    Ok(AircraftAssets {
        him,
        hook_frames,
        cockpit,
        mesh,
        rig,
        tracks,
        fm,
        parked,
        hidden,
        gear_chunks,
        twin_fin,
        boxes,
        toughness,
        fighter: classes.interfaces(class).iter().any(|i| i == "TypeFighter"),
    })
}

/// Node transforms for the current control positions (`docs/behavior/animation.md`).
pub fn pose(a: &AircraftAssets, ct: &il2_fm::Inputs) -> Vec<glam::Mat4> {
    pose_rig(&a.rig, &a.tracks, ct)
}

pub fn pose_rig(
    rig: &il2_render::scene::Rig,
    tracks: &[il2_data::anim::Track],
    ct: &il2_fm::Inputs,
) -> Vec<glam::Mat4> {
    use il2_data::anim::Channel;
    let mut motion: Vec<Option<il2_render::scene::NodeMotion>> = vec![None; rig.names.len()];
    for t in tracks {
        let x = match t.channel {
            Channel::Rudder => ct.rudder,
            Channel::Elevator => ct.elevator,
            Channel::Aileron => ct.aileron,
            Channel::Flaps => ct.flaps,
            Channel::Gear => ct.gear,
        } as f32;
        for (name, st) in t.eval(x) {
            if let Some(i) = rig.find(name) {
                motion[i] = Some(il2_render::scene::NodeMotion {
                    angles: st.angles,
                    offset: st.offset,
                });
            }
        }
    }
    rig.pose(&motion)
}

/// A parked aircraft for static display: model, gear-down node transforms, origin height above the
/// ground and nose-up pitch in degrees (`docs/formats/mission.md`, "Static aircraft").
pub struct Parked {
    pub him: String,
    pub nodes: Vec<glam::Mat4>,
    pub height: f32,
    pub pitch: f32,
}

pub fn parked_model(vfs: &Vfs, fms: &il2_data::FlightModels, class: &str) -> Option<Parked> {
    let classes = il2_data::Classes::new(vfs, &fms.keys);
    let info = classes.aircraft(class)?;
    let him = info.meshes.first()?.clone();
    let loader = Loader::new(vfs);
    let rig = loader.hier_rig(&him).ok()?;
    let nodes = pose_rig(
        &rig,
        &classes.animations(class),
        &il2_fm::Inputs {
            gear: 1.0,
            ..Default::default()
        },
    );
    let hooks = loader.hier_hooks_posed(&him, &nodes).ok()?;
    let hook = |n: &str| {
        hooks
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(n))
            .map(|(_, p)| *p)
    };
    let fmd = fms.sect_file(&info.flight_model).ok();
    // Without a decoded gear animation the hooks stay retracted; use the flight model's figures.
    let rest = rig.pose(&[]);
    let gear_moves = nodes
        .iter()
        .zip(&rest)
        .any(|(a, b)| !a.abs_diff_eq(*b, 1e-4));
    let from_ini = !gear_moves
        || fmd
            .as_ref()
            .and_then(|f| f.section_ci("Gear")?.get_ci("FromIni"))
            .and_then(il2_formats::sectfile::atof)
            .unwrap_or(0.0)
            != 0.0;
    let (height, pitch) = match (hook("_ClipLGear"), hook("_ClipRGear"), hook("_ClipCGear")) {
        (Some(l), Some(r), Some(c)) if !from_ini => {
            let (mx, mz) = ((l.x + r.x) * 0.5, (l.z + r.z) * 0.5);
            let (dx, dz) = (mx - c.x, mz - c.z);
            let mut pitch = -dz.atan2(dx).to_degrees();
            if dx < 0.0 {
                pitch += 180.0;
            }
            // Distance from the origin to the line through the main and tail wheels.
            let height = (dx * c.z - dz * c.x).abs() / dx.hypot(dz).max(1e-6);
            (height, pitch)
        }
        _ => {
            let num = |k: &str| {
                fmd.as_ref()
                    .and_then(|f| f.section_ci("Gear")?.get_ci(k))
                    .and_then(il2_formats::sectfile::atof)
                    .unwrap_or(-0.5)
            };
            (num("H"), num("Pitch"))
        }
    };
    Some(Parked {
        him,
        nodes,
        height,
        pitch,
    })
}

/// Runway ends of a map (from the `.static` files listed in its `load.ini`).
pub fn load_runways(vfs: &Vfs, ini_path: &str) -> Vec<il2_formats::statics::Placed> {
    let Ok(d) = vfs.read(ini_path) else {
        return Vec::new();
    };
    let ini = SectFile::parse(&d);
    let dir = &ini_path[..ini_path.rfind('/').map_or(0, |i| i + 1)];
    let mut out = Vec::new();
    for (name, _) in ini
        .section_ci("static")
        .map(|s| s.entries.as_slice())
        .unwrap_or_default()
    {
        if let Some(st) = vfs
            .read(&format!("{dir}{name}"))
            .ok()
            .and_then(|d| il2_formats::statics::Statics::parse(&d).ok())
        {
            out.extend(st.runway_ends().cloned());
        }
    }
    out
}

fn forward(yaw_deg: f32) -> (f64, f64) {
    let r = (yaw_deg as f64).rem_euclid(360.0).to_radians();
    (r.cos(), -r.sin())
}

/// Takeoff position for a waypoint (`docs/formats/static.md`, "Runways and takeoff"):
/// returns position on the ground and heading (radians, counter-clockwise from east).
pub fn takeoff_spot(
    ends: &[il2_formats::statics::Placed],
    x: f64,
    y: f64,
) -> Option<(f64, f64, f64)> {
    let near = ends
        .iter()
        .min_by(|a, b| {
            let da = (a.x as f64 - x).hypot(a.y as f64 - y);
            let db = (b.x as f64 - x).hypot(b.y as f64 - y);
            da.total_cmp(&db)
        })
        .filter(|e| (e.x as f64 - x).hypot(e.y as f64 - y) <= 15_000.0)?;
    let (fx, fy) = forward(near.yaw);
    let opposite = ends.iter().find(|b| {
        let (dx, dy) = (b.x as f64 - near.x as f64, b.y as f64 - near.y as f64);
        let along = dx * fx + dy * fy;
        let side = -dx * fy + dy * fx;
        let (bx, by) = forward(b.yaw);
        side.abs() < 15.0 && along < -800.0 && along > -2500.0 && bx * fx + by * fy < -0.9
    });
    let d = opposite.map_or(1000.0, |b| {
        (b.x as f64 - near.x as f64).hypot(b.y as f64 - near.y as f64)
    });
    Some((near.x as f64 - d * fx, near.y as f64 - d * fy, fy.atan2(fx)))
}

/// Parks the aircraft on the ground at a takeoff spot, at its `[Gear] H`/`Pitch` attitude.
pub fn spawn_on_ground(
    ac: Aircraft,
    field: &HeightField,
    x: f64,
    y: f64,
    heading: f64,
    h: f64,
    pitch_deg: f64,
) -> Sim {
    let z = field.height_at(x, y) + h;
    let ori = DQuat::from_rotation_z(heading) * DQuat::from_rotation_y(-pitch_deg.to_radians());
    let mut sim = Sim::new(ac, DVec3::new(x, y, z), ori, 0.0);
    sim.set_engines_idle(true);
    sim
}

/// Starts airborne at a point, heading towards another.
pub fn spawn_at(ac: Aircraft, pos: DVec3, toward: DVec3, speed: f64) -> Sim {
    let d = toward - pos;
    let heading = d.y.atan2(d.x);
    Sim::new(ac, pos, DQuat::from_rotation_z(heading), speed)
}

/// Starts airborne over the map center, heading north.
pub fn spawn_airborne(ac: Aircraft, field: &HeightField, agl: f64, speed: f64) -> Sim {
    let (x, y) = (field.width as f64 * 100.0, field.height as f64 * 100.0);
    let z = field.height_at(x, y) + agl;
    let heading_north = DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2);
    Sim::new(ac, DVec3::new(x, y, z), heading_north, speed)
}
