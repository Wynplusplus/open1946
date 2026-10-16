//! Loads game assets from the VFS into renderer-ready mesh data.

use crate::render::{Batch, BlendMode, Image, MeshData, TerrainData, Vertex};
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
    unlit: bool,
}

/// Decodes IMF (by magic) or a standard image format (by extension, TGA by default).
pub fn decode_image(d: &[u8], path: &str) -> Result<Image, String> {
    if il2_formats::imf::is_imf(d) {
        let img = il2_formats::imf::decode(d).map_err(|e| e.to_string())?;
        return Ok(Image { width: img.width, height: img.height, rgba: img.pixels });
    }
    if let Some((width, height, rgba)) = il2_formats::tga::decode_colormapped(d) {
        return Ok(Image { width, height, rgba });
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
            // Textures are relative to the material that names them.
            mat.layers.first().and_then(|l| {
                let p = format!("{}{}", dir_of(&files[l.texture_from].0), l.texture.as_ref()?);
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
                    unlit: mat.ambient == 0.0 && mat.diffuse == 0.0,
                })
            })
        };
        self.materials.insert(key, tex);
        tex
    }

    /// Appends all LODs of a mesh, transformed by `xf`, as one LOD group bound to `node`.
    pub fn add_mesh(&mut self, out: &mut MeshData, msh_path: &str, xf: Mat4) -> Result<(), String> {
        self.add_mesh_node(out, msh_path, xf, 0)
    }

    fn add_mesh_node(&mut self, out: &mut MeshData, msh_path: &str, xf: Mat4, node: u32) -> Result<(), String> {
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
                out.vertices.push(Vertex { pos: p.into(), normal: n.into(), uv: lod.uvs[i], node });
            }
            for g in &lod.groups {
                let mat_name = lod.materials.get(g.material).cloned().unwrap_or_default();
                let info = self.material(&format!("{dir}{mat_name}.mat")).unwrap_or(MatInfo {
                    texture: 0,
                    alpha_cutoff: 0.0,
                    blend: BlendMode::Opaque,
                    depth_offset: false,
                    unlit: false,
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
                    unlit: info.unlit,
                    lod: Some((group, level as u8)),
                });
            }
        }
        Ok(())
    }

    /// Hook positions of a hierarchical mesh in object coordinates (rest pose), from every node's mesh.
    pub fn hier_hooks(&self, him_path: &str) -> Result<Vec<(String, Vec3)>, String> {
        let data = self.vfs.read(him_path).map_err(|e| e.to_string())?;
        let hier = Hier::parse(&data).map_err(|e| e.to_string())?;
        let world = node_transforms(&hier)?;
        self.hooks_with(him_path, &hier, &world)
    }

    /// Hook positions with the nodes posed by `world` (e.g. from `Rig::pose`).
    pub fn hier_hooks_posed(&self, him_path: &str, world: &[Mat4]) -> Result<Vec<(String, Vec3)>, String> {
        let data = self.vfs.read(him_path).map_err(|e| e.to_string())?;
        let hier = Hier::parse(&data).map_err(|e| e.to_string())?;
        if world.len() != hier.nodes.len() {
            return Err("pose does not match hierarchy".into());
        }
        self.hooks_with(him_path, &hier, world)
    }

    fn hooks_with(&self, him_path: &str, hier: &Hier, world: &[Mat4]) -> Result<Vec<(String, Vec3)>, String> {
        let dir = dir_of(him_path).to_owned();
        let mut out = Vec::new();
        for (i, n) in hier.nodes.iter().enumerate() {
            let Some(mesh) = &n.mesh else { continue };
            let Ok(d) = self.vfs.read(&format!("{dir}{mesh}.msh")) else { continue };
            let Ok(m) = Mesh::parse(&d) else { continue };
            for h in &m.hooks {
                let local = Vec3::new(h.matrix[9], h.matrix[10], h.matrix[11]);
                out.push((h.name.clone(), world[i].transform_point3(local)));
            }
        }
        Ok(out)
    }

    /// Hook frames (object coordinates, nodes posed by `world`): origin and the hook's local axes.
    pub fn hier_hook_frames(&self, him_path: &str, world: &[Mat4]) -> Result<Vec<(String, Mat4)>, String> {
        let data = self.vfs.read(him_path).map_err(|e| e.to_string())?;
        let hier = Hier::parse(&data).map_err(|e| e.to_string())?;
        let dir = dir_of(him_path).to_owned();
        let mut out = Vec::new();
        for (i, n) in hier.nodes.iter().enumerate() {
            let Some(mesh) = &n.mesh else { continue };
            let Ok(d) = self.vfs.read(&format!("{dir}{mesh}.msh")) else { continue };
            let Ok(m) = Mesh::parse(&d) else { continue };
            let node = world.get(i).copied().unwrap_or(Mat4::IDENTITY);
            for h in &m.hooks {
                out.push((h.name.clone(), node * attaching_to_mat4(&h.matrix)));
            }
        }
        Ok(out)
    }

    /// The node tree of a hierarchical mesh.
    pub fn hier_rig(&self, him_path: &str) -> Result<Rig, String> {
        let data = self.vfs.read(him_path).map_err(|e| e.to_string())?;
        Ok(Rig::from_hier(&Hier::parse(&data).map_err(|e| e.to_string())?))
    }

    /// Loads all visible nodes of a hierarchical mesh into one draw list. Geometry stays in node
    /// space; `MeshData::node_init` holds the rest pose.
    pub fn load_hier(&mut self, him_path: &str, show_hidden: bool) -> Result<MeshData, String> {
        let mut out = MeshData::default();
        self.append_hier(&mut out, him_path, show_hidden)?;
        out.node_count = out.node_count.max(1);
        Ok(out)
    }

    /// Appends a hierarchical mesh to `out` with its own nodes, LOD groups and batches; returns the
    /// range of batches it added. Textures index into this loader's images (`take_images`).
    pub fn append_hier(&mut self, out: &mut MeshData, him_path: &str, show_hidden: bool) -> Result<std::ops::Range<usize>, String> {
        let data = self.vfs.read(him_path).map_err(|e| e.to_string())?;
        let hier = Hier::parse(&data).map_err(|e| e.to_string())?;
        let dir = dir_of(him_path).to_owned();
        let world = node_transforms(&hier)?;
        // `Hidden` hides only the node's own mesh; its children (e.g. gear legs under a hidden
        // gear-bay node) stay visible.
        let visible: Vec<bool> = hier.nodes.iter().map(|n| show_hidden || !n.hidden).collect();
        let base = out.node_init.len();
        // Earlier nodes without explicit transforms are identity.
        out.node_init.resize(out.node_count.max(base), Mat4::IDENTITY);
        let base = out.node_init.len();
        out.node_init.extend(&world);
        out.node_count = out.node_init.len();
        let first = out.batches.len();
        for (i, n) in hier.nodes.iter().enumerate() {
            let Some(mesh) = &n.mesh else { continue };
            if !visible[i] {
                continue;
            }
            if let Err(e) = self.add_mesh_node(out, &format!("{dir}{mesh}.msh"), Mat4::IDENTITY, (base + i) as u32) {
                self.warnings.push(format!("node {}: {e}", n.name));
            }
        }
        Ok(first..out.batches.len())
    }

    /// Appends a unit billboard quad (spanning y and z in −1..1, facing +x) in material `mat_path`;
    /// returns its batch index, or `None` when the material can't be read.
    pub fn append_quad(&mut self, out: &mut MeshData, mat_path: &str) -> Option<usize> {
        let info = self.material(mat_path)?;
        let base = out.vertices.len() as u32;
        let node = out.node_init.len().max(out.node_count) as u32;
        out.node_init.resize(node as usize, Mat4::IDENTITY);
        out.node_init.push(Mat4::IDENTITY);
        out.node_count = out.node_init.len();
        for (y, z, u, v) in [(-1.0, -1.0, 0.0, 1.0), (1.0, -1.0, 1.0, 1.0), (1.0, 1.0, 1.0, 0.0), (-1.0, 1.0, 0.0, 0.0)] {
            out.vertices.push(Vertex { pos: [0.0, y, z], normal: [1.0, 0.0, 0.0], uv: [u, v], node });
        }
        let first = out.indices.len() as u32;
        out.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        out.batches.push(Batch {
            first,
            count: 6,
            texture: info.texture,
            alpha_cutoff: info.alpha_cutoff,
            blend: if info.blend == BlendMode::Opaque { BlendMode::Alpha } else { info.blend },
            depth_offset: false,
            unlit: info.unlit,
            lod: None,
        });
        Some(out.batches.len() - 1)
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
                    node: 0,
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
                unlit: false,
                lod: None,
            }],
            ..Default::default()
        })
    }
}

/// Extra local motion of one node (`docs/behavior/animation.md`, "Node transforms").
#[derive(Clone, Copy, Debug, Default)]
pub struct NodeMotion {
    /// Degrees: yaw, pitch, roll as in the runtime's chunk calls.
    pub angles: [f32; 3],
    pub offset: [f32; 3],
}

impl NodeMotion {
    pub fn matrix(&self) -> Mat4 {
        let [a, b, c] = self.angles.map(f32::to_radians);
        Mat4::from_translation(Vec3::from(self.offset))
            * Mat4::from_rotation_z(a)
            * Mat4::from_rotation_y(-b)
            * Mat4::from_rotation_x(-c)
    }
}

/// A hierarchy's node tree, for posing animated parts.
#[derive(Clone, Debug)]
pub struct Rig {
    pub names: Vec<String>,
    pub parents: Vec<Option<usize>>,
    pub attach: Vec<Mat4>,
}

impl Rig {
    pub fn from_hier(hier: &Hier) -> Self {
        Rig {
            names: hier.nodes.iter().map(|n| n.name.clone()).collect(),
            parents: hier.nodes.iter().map(|n| n.parent).collect(),
            attach: hier.nodes.iter().map(|n| attaching_to_mat4(&n.attaching)).collect(),
        }
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n.eq_ignore_ascii_case(name))
    }

    /// Object-space node transforms with per-node motions applied.
    pub fn pose(&self, motion: &[Option<NodeMotion>]) -> Vec<Mat4> {
        let n = self.names.len();
        let mut out: Vec<Option<Mat4>> = vec![None; n];
        while out.iter().any(Option::is_none) {
            let mut progress = false;
            for i in 0..n {
                if out[i].is_some() {
                    continue;
                }
                let parent = match self.parents[i] {
                    None => Mat4::IDENTITY,
                    Some(p) => match out[p] {
                        Some(m) => m,
                        None => continue,
                    },
                };
                let local = motion.get(i).copied().flatten().map_or(Mat4::IDENTITY, |m| m.matrix());
                out[i] = Some(parent * self.attach[i] * local);
                progress = true;
            }
            if !progress {
                break;
            }
        }
        out.into_iter().map(|m| m.unwrap_or(Mat4::IDENTITY)).collect()
    }
}

/// Object-space transform of every node of a hierarchy.
pub fn node_transforms(hier: &Hier) -> Result<Vec<Mat4>, String> {
    let mut world = vec![Mat4::IDENTITY; hier.nodes.len()];
    let mut done = vec![false; hier.nodes.len()];
    while done.iter().any(|d| !d) {
        let mut progress = false;
        for (i, n) in hier.nodes.iter().enumerate() {
            if done[i] {
                continue;
            }
            let pw = match n.parent {
                None => Mat4::IDENTITY,
                Some(p) if done[p] => world[p],
                Some(_) => continue,
            };
            world[i] = pw * attaching_to_mat4(&n.attaching);
            done[i] = true;
            progress = true;
        }
        if !progress {
            return Err("cycle in hierarchy".into());
        }
    }
    Ok(world)
}

/// Field texture slots in land-type order (`docs/formats/map.md`, "Land types").
pub const FIELD_GROUPS: [&str; 8] = ["LowLand", "MidLand", "Mount", "Country", "City", "AirField", "Wood", "Water"];
const FIELD_SIZE: u32 = 512;

impl Loader<'_> {
    /// Textured terrain of a map's `load.ini`.
    pub fn load_terrain(&mut self, ini_path: &str) -> Result<TerrainData, String> {
        let mut mesh = self.load_map(ini_path)?;
        // The terrain shader samples land textures, not the overview image; keep a placeholder.
        mesh.textures = vec![Image::solid([255, 255, 255, 255])];
        for b in &mut mesh.batches {
            b.texture = 0;
        }
        let ini = il2_formats::SectFile::parse(&self.vfs.read(ini_path).map_err(|e| e.to_string())?);
        let dir = dir_of(ini_path).to_owned();
        let value = |sect: &str, key: &str| {
            ini.section_ci(sect)
                .and_then(|s| s.get_ci(key))
                .map(|v| v.trim_start_matches(['=', ' ', '\t']).trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        let tpath = format!("{dir}{}", value("MAP", "TypeMap").ok_or("no TypeMap")?);
        let tdata = self.vfs.read(&tpath).map_err(|e| format!("{tpath}: {e}"))?;
        let timg = image::load_from_memory_with_format(&tdata, image::ImageFormat::Tga).map_err(|e| format!("{tpath}: {e}"))?.to_luma8();
        let types = (timg.width(), timg.height(), timg.into_raw());

        let mut fields: Vec<Option<Image>> = Vec::with_capacity(32);
        let mut tile = [400.0f32; 32];
        let mut water_tex = [false; 32];
        for (g, group) in FIELD_GROUPS.iter().enumerate() {
            for v in 0..4 {
                let slot = g * 4 + v;
                let entry = value("FIELDS", &format!("{group}{v}"));
                let img = entry.as_ref().and_then(|e| {
                    let (file, scale) = e.split_once(',').map_or((e.as_str(), None), |(f, s)| (f.trim(), s.trim().parse::<f32>().ok()));
                    if let Some(s) = scale {
                        tile[slot] = 400.0 * s;
                    }
                    water_tex[slot] = file.to_ascii_lowercase().starts_with("water/");
                    let path = format!("maps/_Tex/{file}");
                    match self.vfs.read(&path).map_err(|e| e.to_string()).and_then(|d| decode_image(&d, &path)) {
                        Ok(img) => Some(resize(img, FIELD_SIZE)),
                        Err(e) => {
                            self.warnings.push(format!("field {group}{v}: {e}"));
                            None
                        }
                    }
                });
                fields.push(img);
            }
        }
        // Empty slots fall back to the first texture of their group, then to LowLand0.
        let fallback = fields[0].clone().unwrap_or_else(|| Image::solid([90, 110, 60, 255]));
        let fallback = resize(fallback, FIELD_SIZE);
        let fields: Vec<Image> = (0..32)
            .map(|i| fields[i].clone().or_else(|| fields[i / 4 * 4].clone()).unwrap_or_else(|| fallback.clone()))
            .collect();
        // Water-group slots are water when their texture (or the group's first, for empty slots) is.
        let mut water = [false; 32];
        for (i, w) in water.iter_mut().enumerate().skip(28) {
            let has = value("FIELDS", &format!("Water{}", i - 28)).is_some();
            *w = if has { water_tex[i] } else { water_tex[28] };
        }
        Ok(TerrainData { mesh, cell: il2_formats::map::PIXEL_SIZE, types, fields, tile, water })
    }
}

fn resize(img: Image, size: u32) -> Image {
    if img.width == size && img.height == size {
        return img;
    }
    let buf = image::RgbaImage::from_raw(img.width, img.height, img.rgba).expect("image size");
    let r = image::imageops::resize(&buf, size, size, image::imageops::FilterType::Triangle);
    Image { width: size, height: size, rgba: r.into_raw() }
}

impl Clone for Image {
    fn clone(&self) -> Self {
        Image { width: self.width, height: self.height, rgba: self.rgba.clone() }
    }
}

/// Bounding sphere (center, radius) of mesh data in its rest pose, from LOD 0 (or LOD-less) batches.
pub fn bounds(m: &MeshData) -> (Vec3, f32) {
    let xf = |v: &Vertex| m.node_init.get(v.node as usize).copied().unwrap_or(Mat4::IDENTITY).transform_point3(Vec3::from(v.pos));
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let lod0 = m.batches.iter().filter(|b| b.lod.is_none_or(|(_, l)| l == 0));
    let verts = lod0.flat_map(|b| &m.indices[b.first as usize..(b.first + b.count) as usize]).map(|&i| &m.vertices[i as usize]);
    let mut any = false;
    for v in verts {
        any = true;
        lo = lo.min(xf(v));
        hi = hi.max(xf(v));
    }
    if !any {
        return (Vec3::ZERO, 1.0);
    }
    ((lo + hi) * 0.5, ((hi - lo) * 0.5).length().max(0.01))
}
