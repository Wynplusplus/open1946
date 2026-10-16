//! Visual effects: bullet tracers (the weapons' own tracer meshes), drawn instanced.

use crate::weapons::Bullet;
use glam::{DQuat, DVec3, Mat4};
use il2_render::render::select_lod;
use il2_render::scene::Loader;
use il2_render::{GpuMesh, InstanceRaw, InstancedDraw, MeshData};
use il2_vfs::Vfs;
use std::collections::HashMap;
use std::ops::Range;

pub struct Tracers {
    pub data: Option<MeshData>,
    /// Mesh path → batches and LOD limits.
    classes: HashMap<String, (Range<usize>, Vec<f32>)>,
}

impl Tracers {
    /// Loads the tracer meshes of the given guns.
    pub fn load<'a>(vfs: &Vfs, meshes: impl Iterator<Item = &'a str>) -> Self {
        let mut loader = Loader::new(vfs);
        let mut mesh = MeshData::default();
        let mut classes = HashMap::new();
        for path in meshes {
            if classes.contains_key(path) {
                continue;
            }
            if let Ok(b) = loader.append_hier(&mut mesh, path, false) {
                if !b.is_empty() {
                    let lods = mesh.batches[b.start].lod.map(|(g, _)| mesh.lod_distances[g as usize].clone()).unwrap_or_default();
                    classes.insert(path.to_owned(), (b, lods));
                }
            }
        }
        if std::env::var_os("IL2_TRACE").is_some() {
            eprintln!("tracer meshes: {:?}, warnings {:?}", classes.keys().collect::<Vec<_>>(), loader.warnings);
            for (k, (b, l)) in &classes {
                eprintln!("  {k}: batches {b:?} lods {l:?}");
            }
        }
        mesh.textures = loader.take_images();
        mesh.node_count = mesh.node_count.max(1);
        Tracers { data: Some(mesh), classes }
    }

    /// Appends tracer instances (positions relative to `eye`) and returns their draws.
    pub fn gather<'a>(&self, mesh: &'a GpuMesh, bullets: &[Bullet], eye: DVec3, out: &mut Vec<InstanceRaw>) -> Vec<InstancedDraw<'a>> {
        let mut by_class: HashMap<&str, Vec<InstanceRaw>> = HashMap::new();
        for b in bullets.iter().filter(|b| b.tracer) {
            let Some(path) = b.ty().trace_mesh.as_deref() else { continue };
            let Some((_, lods)) = self.classes.get(path) else { continue };
            let rel = b.pos - eye;
            if select_lod(lods, rel.length() as f32).is_none() && !lods.is_empty() {
                continue;
            }
            let dir = b.vel.normalize_or(DVec3::X);
            let rot = DQuat::from_rotation_arc(DVec3::X, dir).as_quat();
            // OpenIL2: widen distant tracers so they stay a few pixels thick (the original draws
            // lines beyond 500 m).
            let w = (rel.length() / 50.0).max(1.0) as f32;
            let m = Mat4::from_rotation_translation(rot, rel.as_vec3()) * Mat4::from_scale(glam::Vec3::new(1.0, w, w));
            by_class.entry(path).or_default().push(InstanceRaw::from_mat4(m));
        }
        let mut draws = Vec::new();
        for (path, inst) in by_class {
            let (batches, _) = &self.classes[path];
            let start = out.len() as u32;
            out.extend(inst);
            draws.push(InstancedDraw { mesh, batches: batches.clone(), level: 0, instances: start..out.len() as u32 });
        }
        draws
    }
}
