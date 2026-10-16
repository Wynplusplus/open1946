//! Everything drawn around the player's aircraft: map objects, forest, roads and tracers.

use crate::effects::Tracers;
use crate::forest::Forest;
use crate::objects::MapObjects;
use crate::roads::Roads;
use crate::weapons::{Bullet, Projectile};
use glam::{DVec3, Mat4};
use il2_formats::map::HeightField;
use il2_render::{GpuMesh, GpuTerrain, InstanceRaw, Renderer};

pub struct WorldView {
    pub objects: MapObjects,
    objects_gpu: Option<GpuMesh>,
    pub forest: Option<Forest>,
    forest_gpu: Option<GpuMesh>,
    pub roads: Option<Roads>,
    roads_gpu: Option<GpuMesh>,
    pub tracers: Option<Tracers>,
    tracers_gpu: Option<GpuMesh>,
    pub effects: crate::particles::Effects,
    effects_gpu: Option<GpuMesh>,
    instances: Vec<InstanceRaw>,
}

impl WorldView {
    pub fn new(
        objects: MapObjects,
        forest: Option<Forest>,
        roads: Option<Roads>,
        tracers: Option<Tracers>,
        effects: crate::particles::Effects,
    ) -> Self {
        WorldView {
            objects,
            objects_gpu: None,
            forest,
            forest_gpu: None,
            roads,
            roads_gpu: None,
            tracers,
            tracers_gpu: None,
            effects,
            effects_gpu: None,
            instances: Vec::new(),
        }
    }

    /// Uploads the static meshes (once, after the renderer exists).
    pub fn upload(&mut self, r: &Renderer) {
        if let Some(d) = self.objects.data.take() {
            self.objects_gpu = Some(r.upload(&d));
        }
        if let Some(d) = self.tracers.as_mut().and_then(|t| t.data.take()) {
            self.tracers_gpu = Some(r.upload(&d));
        }
        if let Some(d) = self.effects.data.take() {
            self.effects_gpu = Some(r.upload(&d));
        }
    }

    /// Draws a frame with the camera at `eye`, plus the given meshes (already placed relative to
    /// `eye`). With `capture`, renders offscreen and returns the pixels.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        r: &mut Renderer,
        field: &HeightField,
        terrain: &GpuTerrain,
        vp: Mat4,
        eye: DVec3,
        extra: &[&GpuMesh],
        bullets: &[Bullet],
        projectiles: &[Projectile],
        foreground: Option<(Mat4, &[&GpuMesh])>,
        capture: bool,
    ) -> Option<Vec<u8>> {
        if let Some(f) = self.forest.as_mut() {
            if let Some(d) = f.update(field, eye) {
                self.forest_gpu = Some(r.upload(&d));
            }
        }
        if let Some(f) = self.roads.as_mut() {
            if let Some(d) = f.update(field, eye) {
                self.roads_gpu = Some(r.upload(&d));
            }
        }
        let mut draws = match &self.objects_gpu {
            Some(m) => self.objects.gather(m, eye, &mut self.instances),
            None => {
                self.instances.clear();
                Vec::new()
            }
        };
        if let (Some(t), Some(m)) = (&self.tracers, &self.tracers_gpu) {
            draws.extend(t.gather(m, bullets, eye, &mut self.instances));
            let items = projectiles.iter().filter_map(|p| {
                Some((
                    p.spec.mesh.as_deref()?,
                    glam::DQuat::from_rotation_arc(DVec3::X, p.dir),
                    p.pos,
                ))
            });
            draws.extend(t.gather_meshes(m, items, eye, &mut self.instances));
        }
        if let Some(m) = &self.effects_gpu {
            draws.extend(self.effects.gather(m, eye, &mut self.instances));
        }
        r.set_instances(&self.instances);
        let mut meshes: Vec<&GpuMesh> = extra.to_vec();
        if let (Some(f), Some(g)) = (&self.roads, &self.roads_gpu) {
            r.set_transform(g, Mat4::from_translation((f.origin - eye).as_vec3()));
            meshes.push(g);
        }
        if let (Some(f), Some(g)) = (&self.forest, &self.forest_gpu) {
            r.set_transform(g, Mat4::from_translation((f.origin - eye).as_vec3()));
            meshes.push(g);
        }
        if capture {
            Some(r.screenshot(vp, 22.0, Some(terrain), &meshes, &draws, foreground))
        } else {
            r.render(vp, 22.0, Some(terrain), &meshes, &draws, foreground);
            None
        }
    }
}
