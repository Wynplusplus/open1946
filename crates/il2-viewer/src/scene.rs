//! Loads game assets from the VFS into renderer-ready mesh data.

use crate::render::{Batch, BlendMode, Image, MeshData, Vertex};
use glam::{Mat4, Vec3, Vec4};
use il2_formats::{him::Hier, mat::{MatFile, Material}, msh::Mesh};
use il2_vfs::Vfs;
use std::collections::HashMap;

fn dir_of(path: &str) -> &str {
    path.rfind(['/', '\\']).map_or("", |i| &path[..=i])
}

/// Engine attachment matrix (`docs/formats/him.md`) as a column-major transform.
pub fn attaching_to_mat4(m: &[f32; 12]) -> Mat4 {
    Mat4::from_cols(
        Vec4::new(m[0], m[1], m[2], 0.0),
        Vec4::new(m[3], m[4], m[5], 0.0),
        Vec4::new(m[6], m[7], m[8], 0.0),
        Vec4::new(m[9], m[10], m[11], 1.0),
    )
}

/// What the renderer needs from a resolved material.
#[derive(Clone, Copy)]
struct MatInfo {
    texture: usize,
    alpha_cutoff: f32,
    blend: BlendMode,
    depth_offset: bool,
}

/// Decodes IMF (by magic) or a standard image format (by extension, TGA by default).
pub fn decode_image(d: &[u8], path: &str) -> Result<Image, String> {
    if il2_formats::imf::is_imf(d) {
        let img = il2_formats::imf::decode(d).map_err(|e| e.to_string())?;
        return Ok(Image { width: img.width, height: img.height, rgba: img.pixels });
    }
    let fmt = image::ImageFormat::from_path(path).unwrap_or(image::ImageFormat::Tga);
    let img = image::load_from_memory_with_format(d, fmt).map_err(|e| e.to_string())?;
    // One-channel images are alpha-only in the engine (docs/formats/mat.md).
    if let image::DynamicImage::ImageLuma8(g) = &img {
        let rgba = g.as_raw().iter().flat_map(|&a| [255, 255, 255, a]).collect();
        return Ok(Image { width: g.width(), height: g.height(), rgba });
    }
    let img = img.to_rgba8();
    Ok(Image { width: img.width(), height: img.height(), rgba: img.into_raw() })
}

pub struct Loader<'a> {
    pub vfs: &'a Vfs,
    /// Camouflage sub-directory tried first for textures (e.g. `summer`).
    pub season: Option<String>,
    textures: HashMap<String, usize>,
    images: Vec<Image>,
    materials: HashMap<String, Option<MatInfo>>,
    pub warnings: Vec<String>,
}

impl<'a> Loader<'a> {
    pub fn new(vfs: &'a Vfs) -> Self {
        Self { vfs, season: None, textures: HashMap::new(), images: vec![Image::solid([255, 0, 255, 255])], materials: HashMap::new(), warnings: Vec::new() }
    }

    fn texture(&mut self, path: &str) -> usize {
        let key = path.to_ascii_lowercase();
        if let Some(&i) = self.textures.get(&key) {
            return i;
        }
        // Season-specific skins replace the base texture of the same name, like the game does.
        let seasonal = self.season.as_ref().map(|s| format!("{}{s}/{}", dir_of(path), &path[dir_of(path).len()..]));
        let data = seasonal
            .and_then(|p| self.vfs.read(&p).ok())
            .map(Ok)
            .unwrap_or_else(|| self.vfs.read(path).map_err(|e| e.to_string()));
        let img = data.and_then(|d| decode_image(&d, path));
        let idx = match img {
            Ok(img) => {
                self.images.push(img);
                self.images.len() - 1
            }
            Err(e) => {
                self.warnings.push(format!("texture {path}: {e}"));
                0
            }
        };
        self.textures.insert(key, idx);
        idx
    }

    /// Resolves a material (with its `BasedOn` chain) to the texture and alpha test of its first layer.
    fn material(&mut self, path: &str) -> Option<MatInfo> {
        let key = path.to_ascii_lowercase();
        if let Some(&t) = self.materials.get(&key) {
            return t;
        }
        let mut files: Vec<(String, MatFile)> = Vec::new();
        let mut cur = path.to_owned();
        while files.len() < 8 {
            let Ok(d) = self.vfs.read(&cur) else {
                self.warnings.push(format!("material {cur}: not found"));
                break;
            };
            let m = MatFile::parse(&d);
            let next = m.based_on.as_ref().map(|b| format!("{}{}", dir_of(&cur), b));
            files.push((cur.clone(), m));
            match next {
                Some(n) => cur = n,
                None => break,
            }
        }
        let tex = if files.is_empty() {
            None
        } else {
            let chain: Vec<&MatFile> = files.iter().map(|(_, m)| m).collect();
            let mat = Material::resolve(&chain);
            // Textures are relative to the material that names them; approximate with the leaf's dir.
            mat.layers.first().and_then(|l| {
                let p = format!("{}{}", dir_of(path), l.texture.as_ref()?);
                Some(MatInfo {
                    texture: self.texture(&p),
                    alpha_cutoff: l.alpha_test.unwrap_or(0.0),
                    blend: if l.blend_add {
                        BlendMode::Additive
                    } else if l.blend {
                        BlendMode::Alpha
                    } else {
                        BlendMode::Opaque
                    },
                    depth_offset: l.depth_offset,
                })
            })
        };
        self.materials.insert(key, tex);
        tex
    }

    /// Appends all LODs of a mesh, transformed by `xf`, as one LOD group.
    pub fn add_mesh(&mut self, out: &mut MeshData, msh_path: &str, xf: Mat4) -> Result<(), String> {
        let data = self.vfs.read(msh_path).map_err(|e| e.to_string())?;
        let mesh = Mesh::parse(&data).map_err(|e| e.to_string())?;
        let dir = dir_of(msh_path).to_owned();
        let group = out.lod_distances.len() as u32;
        let lods = if mesh.lods.len() > 1 { &mesh.lods[..] } else { &mesh.lods[..1] };
        // Single-LOD meshes still honor their visibility limit, if any.
        out.lod_distances.push(mesh.lod_distances.iter().take(lods.len()).copied().collect());
        let nxf = xf.inverse().transpose();
        for (level, lod) in lods.iter().enumerate() {
            let base = out.vertices.len() as u32;
            for i in 0..lod.positions.len() {
                let p = xf.transform_point3(Vec3::from(lod.positions[i]));
                let n = nxf.transform_vector3(Vec3::from(lod.normals[i])).normalize_or_zero();
                out.vertices.push(Vertex { pos: p.into(), normal: n.into(), uv: lod.uvs[i] });
            }
            for g in &lod.groups {
                let mat_name = lod.materials.get(g.material).cloned().unwrap_or_default();
                let info = self.material(&format!("{dir}{mat_name}.mat")).unwrap_or(MatInfo {
                    texture: 0,
                    alpha_cutoff: 0.0,
                    blend: BlendMode::Opaque,
                    depth_offset: false,
                });
                let first = out.indices.len() as u32;
                for f in &lod.faces[g.start_face as usize..(g.start_face + g.face_count) as usize] {
                    out.indices.extend(f.iter().map(|&i| base + i));
                }
                out.batches.push(Batch {
                    first,
                    count: out.indices.len() as u32 - first,
                    texture: info.texture,
                    alpha_cutoff: info.alpha_cutoff,
                    blend: info.blend,
                    depth_offset: info.depth_offset,
                    lod: Some((group, level as u8)),
                });
            }
        }
        Ok(())
    }

    /// Loads all visible nodes of a hierarchical mesh into one draw list.
    pub fn load_hier(&mut self, him_path: &str, show_hidden: bool) -> Result<MeshData, String> {
        let data = self.vfs.read(him_path).map_err(|e| e.to_string())?;
        let hier = Hier::parse(&data).map_err(|e| e.to_string())?;
        let dir = dir_of(him_path).to_owned();
        let mut world = vec![Mat4::IDENTITY; hier.nodes.len()];
        let mut visible = vec![true; hier.nodes.len()];
        // Parents precede children in practice, but resolve iteratively to be safe.
        let mut done = vec![false; hier.nodes.len()];
        while done.iter().any(|d| !d) {
            let mut progress = false;
            for (i, n) in hier.nodes.iter().enumerate() {
                if done[i] {
                    continue;
                }
                let (pw, pv) = match n.parent {
                    None => (Mat4::IDENTITY, true),
                    Some(p) if done[p] => (world[p], visible[p]),
                    Some(_) => continue,
                };
                world[i] = pw * attaching_to_mat4(&n.attaching);
                visible[i] = pv && (show_hidden || !n.hidden);
                done[i] = true;
                progress = true;
            }
            if !progress {
                return Err("cycle in hierarchy".into());
            }
        }
        let mut out = MeshData::default();
        for (i, n) in hier.nodes.iter().enumerate() {
            let Some(mesh) = &n.mesh else { continue };
            if !visible[i] {
                continue;
            }
            if let Err(e) = self.add_mesh(&mut out, &format!("{dir}{mesh}.msh"), world[i]) {
                self.warnings.push(format!("node {}: {e}", n.name));
            }
        }
        Ok(out)
    }

    pub fn take_images(&mut self) -> Vec<Image> {
        std::mem::take(&mut self.images)
    }
}

impl Loader<'_> {
    /// Builds a terrain mesh from a map's `load.ini`, textured with its overview image.
    pub fn load_map(&mut self, ini_path: &str) -> Result<MeshData, String> {
        let ini = il2_formats::SectFile::parse(&self.vfs.read(ini_path).map_err(|e| e.to_string())?);
        let dir = dir_of(ini_path).to_owned();
        let map = ini.section_ci("MAP").ok_or("no [MAP] section")?;
        let get = |k: &str| map.get_ci(k).map(|v| v.trim_start_matches(['=', ' ']).trim().to_owned());
        let hpath = format!("{dir}{}", get("HeightMap").ok_or("no HeightMap")?);
        let hdata = self.vfs.read(&hpath).map_err(|e| format!("{hpath}: {e}"))?;
        let himg = image::load_from_memory_with_format(&hdata, image::ImageFormat::Tga)
            .map_err(|e| format!("{hpath}: {e}"))?
            .to_luma8();
        let (w, h) = (himg.width() as usize, himg.height() as usize);
        let texture = get("SmallMap").map(|m| self.texture(&format!("{dir}{m}"))).unwrap_or(0);
        let px = il2_formats::map::PIXEL_SIZE;
        let z = |c: usize, r: usize| il2_formats::map::height(himg.get_pixel(c as u32, r as u32)[0]);
        let mut vertices = Vec::with_capacity(w * h);
        for r in 0..h {
            for c in 0..w {
                let (cl, cr) = (c.saturating_sub(1), (c + 1).min(w - 1));
                let (ru, rd) = (r.saturating_sub(1), (r + 1).min(h - 1));
                let dx = (z(cr, r) - z(cl, r)) / ((cr - cl).max(1) as f32 * px);
                // Rows run north to south, so +y is towards smaller r.
                let dy = (z(c, ru) - z(c, rd)) / ((rd - ru).max(1) as f32 * px);
                let n = Vec3::new(-dx, -dy, 1.0).normalize();
                vertices.push(Vertex {
                    pos: [c as f32 * px, (h - 1 - r) as f32 * px, z(c, r)],
                    normal: n.into(),
                    uv: [(c as f32 + 0.5) / w as f32, (r as f32 + 0.5) / h as f32],
                });
            }
        }
        let mut indices = Vec::with_capacity((w - 1) * (h - 1) * 6);
        for r in 0..h - 1 {
            for c in 0..w - 1 {
                let i = (r * w + c) as u32;
                let (j, k, l) = (i + 1, i + w as u32, i + w as u32 + 1);
                indices.extend([i, k, j, j, k, l]);
            }
        }
        let count = indices.len() as u32;
        Ok(MeshData {
            vertices,
            indices,
            batches: vec![Batch {
                first: 0,
                count,
                texture,
                alpha_cutoff: 0.0,
                blend: BlendMode::Opaque,
                depth_offset: false,
                lod: None,
            }],
            ..Default::default()
        })
    }
}

/// Bounding sphere (center, radius) of mesh data, from the vertices of LOD 0 (or LOD-less) batches.
pub fn bounds(m: &MeshData) -> (Vec3, f32) {
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let lod0 = m.batches.iter().filter(|b| b.lod.is_none_or(|(_, l)| l == 0));
    let verts = lod0.flat_map(|b| &m.indices[b.first as usize..(b.first + b.count) as usize]).map(|&i| &m.vertices[i as usize]);
    let mut any = false;
    for v in verts {
        any = true;
        lo = lo.min(Vec3::from(v.pos));
        hi = hi.max(Vec3::from(v.pos));
    }
    if !any {
        return (Vec3::ZERO, 1.0);
    }
    ((lo + hi) * 0.5, ((hi - lo) * 0.5).length().max(0.01))
}
