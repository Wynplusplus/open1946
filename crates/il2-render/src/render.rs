//! Minimal wgpu renderer: textured, lit, alpha-tested triangle meshes.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use std::sync::Arc;
use wgpu::util::DeviceExt;
use winit::window::Window;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, Default)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

#[cfg(test)]
mod tests {
    use super::select_lod;

    #[test]
    fn lod_selection() {
        let l = [40.0, 100.0, 250.0];
        assert_eq!(select_lod(&l, 10.0), Some(0));
        assert_eq!(select_lod(&l, 40.0), Some(1));
        assert_eq!(select_lod(&l, 249.0), Some(2));
        assert_eq!(select_lod(&l, 300.0), None);
        assert_eq!(select_lod(&[], 1e6), Some(0));
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    light_dir: [f32; 4],
}

/// RGBA8 image, top row first.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn solid(rgba: [u8; 4]) -> Self {
        Self { width: 1, height: 1, rgba: rgba.to_vec() }
    }
}

/// A draw call: a range of indices using one texture.
#[derive(Clone, Copy, Debug)]
pub struct Batch {
    pub first: u32,
    pub count: u32,
    pub texture: usize,
    /// Alpha-test cutoff; 0 disables the test.
    pub alpha_cutoff: f32,
    pub blend: BlendMode,
    /// Pull the surface towards the camera so decals win over coplanar skin.
    pub depth_offset: bool,
    /// `(LOD group, level)`; the batch is drawn only when that level is selected for the group.
    pub lod: Option<(u32, u8)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BlendMode {
    #[default]
    Opaque,
    Alpha,
    Additive,
}

impl BlendMode {
    const ALL: [BlendMode; 3] = [BlendMode::Opaque, BlendMode::Alpha, BlendMode::Additive];

    fn index(self) -> usize {
        self as usize
    }
}

/// CPU-side mesh ready for upload.
#[derive(Default)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub batches: Vec<Batch>,
    pub textures: Vec<Image>,
    /// Per LOD group, the distance up to which each level is used.
    pub lod_distances: Vec<Vec<f32>>,
}

/// Level used at `distance`: the first whose limit is beyond it; `None` past the last (culled).
pub fn select_lod(limits: &[f32], distance: f32) -> Option<u8> {
    if limits.is_empty() {
        return Some(0);
    }
    limits.iter().position(|&l| distance < l).map(|i| i as u8)
}

pub struct GpuMesh {
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    /// Batches with the index of their bind group in `materials`.
    batches: Vec<(Batch, usize)>,
    materials: Vec<wgpu::BindGroup>,
    lod_distances: Vec<Vec<f32>>,
    /// Per-mesh model transform.
    instance: wgpu::Buffer,
    instance_bg: wgpu::BindGroup,
}

pub struct Renderer {
    pub window: Option<Arc<Window>>,
    surface: Option<wgpu::Surface<'static>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    depth: wgpu::TextureView,
    /// Indexed by `BlendMode::index() * 2 + depth_offset`.
    pipelines: Vec<wgpu::RenderPipeline>,
    globals: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    instance_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

fn create_depth(device: &wgpu::Device, w: u32, h: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

fn uniform_layout(device: &wgpu::Device, label: &str) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

impl Renderer {
    pub fn new(window: Arc<Window>) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(Box::new(window.clone())));
        let surface = instance.create_surface(window.clone()).expect("create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("no suitable GPU adapter");
        let size = window.inner_size();
        let config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .expect("surface not supported by adapter");
        Self::build(adapter, Some(window), Some(surface), config)
    }

    /// Renderer without a window, for screenshots.
    pub fn headless(width: u32, height: u32) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).expect("no GPU adapter");
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            color_space: Default::default(),
        };
        Self::build(adapter, None, None, config)
    }

    fn build(
        adapter: wgpu::Adapter,
        window: Option<Arc<Window>>,
        surface: Option<wgpu::Surface<'static>>,
        config: wgpu::SurfaceConfiguration,
    ) -> Self {
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("request device");
        if let Some(s) = &surface {
            s.configure(&device, &config);
        }
        let depth = create_depth(&device, config.width, config.height);

        let globals_layout = uniform_layout(&device, "globals");
        let instance_layout = uniform_layout(&device, "instance");
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() }],
        });
        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("main"),
            bind_group_layouts: &[Some(&globals_layout), Some(&instance_layout), Some(&texture_layout)],
            ..Default::default()
        });
        let mut pipelines = Vec::new();
        for mode in BlendMode::ALL {
            for offset in [false, true] {
                let blend = match mode {
                    BlendMode::Opaque => None,
                    BlendMode::Alpha => Some(wgpu::BlendState::ALPHA_BLENDING),
                    BlendMode::Additive => Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::OVER,
                    }),
                };
                pipelines.push(device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("main"),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_main"),
                        buffers: &[Some(wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<Vertex>() as u64,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2],
                        })],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_main"),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: config.format,
                            blend,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: Default::default(),
                    }),
                    primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: DEPTH_FORMAT,
                        depth_write_enabled: Some(mode == BlendMode::Opaque),
                        depth_compare: Some(wgpu::CompareFunction::LessEqual),
                        stencil: Default::default(),
                        bias: if offset {
                            wgpu::DepthBiasState { constant: -2, slope_scale: -1.0, clamp: 0.0 }
                        } else {
                            Default::default()
                        },
                    }),
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                }));
            }
        }
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            window,
            surface,
            device,
            queue,
            config,
            depth,
            pipelines,
            globals,
            globals_bg,
            instance_layout,
            texture_layout,
            sampler,
        }
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        if let Some(s) = &self.surface {
            s.configure(&self.device, &self.config);
        }
        self.depth = create_depth(&self.device, w, h);
    }

    pub fn aspect(&self) -> f32 {
        self.config.width as f32 / self.config.height as f32
    }

    fn upload_texture(&self, img: &Image) -> wgpu::TextureView {
        let size = wgpu::Extent3d { width: img.width, height: img.height, depth_or_array_layers: 1 };
        let tex = self.device.create_texture_with_data(
            &self.queue,
            &wgpu::TextureDescriptor {
                label: None,
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &img.rgba,
        );
        tex.create_view(&Default::default())
    }

    fn material_bind_group(&self, view: &wgpu::TextureView, alpha_cutoff: f32, blended: bool) -> wgpu::BindGroup {
        let params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("material"),
            contents: bytemuck::cast_slice(&[alpha_cutoff, blended as u32 as f32, 0.0, 0.0]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
            ],
        })
    }

    pub fn upload(&self, mesh: &MeshData) -> GpuMesh {
        let vbuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vertices"),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ibuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let instance = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("instance"),
            contents: bytemuck::cast_slice(&Mat4::IDENTITY.to_cols_array_2d()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let instance_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("instance"),
            layout: &self.instance_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: instance.as_entire_binding() }],
        });
        let views: Vec<wgpu::TextureView> = mesh.textures.iter().map(|t| self.upload_texture(t)).collect();
        let mut keys: Vec<(usize, u32, bool)> = Vec::new();
        let mut materials = Vec::new();
        let batches = mesh
            .batches
            .iter()
            .map(|b| {
                let blended = b.blend != BlendMode::Opaque;
                let key = (b.texture, b.alpha_cutoff.to_bits(), blended);
                let idx = keys.iter().position(|k| *k == key).unwrap_or_else(|| {
                    keys.push(key);
                    materials.push(self.material_bind_group(&views[b.texture], b.alpha_cutoff, blended));
                    keys.len() - 1
                });
                (*b, idx)
            })
            .collect();
        GpuMesh { vbuf, ibuf, batches, materials, lod_distances: mesh.lod_distances.clone(), instance, instance_bg }
    }

    /// Sets a mesh's model transform.
    pub fn set_transform(&self, mesh: &GpuMesh, m: Mat4) {
        self.queue.write_buffer(&mesh.instance, 0, bytemuck::cast_slice(&m.to_cols_array_2d()));
    }

    fn draw(&self, target: &wgpu::TextureView, view_proj: Mat4, distance: f32, meshes: &[&GpuMesh]) -> wgpu::CommandEncoder {
        let light = Vec3::new(-0.4, -0.3, -1.0).normalize();
        let g = Globals { view_proj: view_proj.to_cols_array_2d(), light_dir: [light.x, light.y, light.z, 0.0] };
        self.queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&g));
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.42, g: 0.55, b: 0.70, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &self.globals_bg, &[]);
            // Opaque geometry first, then blended surfaces on top without depth writes.
            for opaque_pass in [true, false] {
                for m in meshes {
                    pass.set_bind_group(1, &m.instance_bg, &[]);
                    pass.set_vertex_buffer(0, m.vbuf.slice(..));
                    pass.set_index_buffer(m.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                    for (b, mat) in &m.batches {
                        if (b.blend == BlendMode::Opaque) != opaque_pass {
                            continue;
                        }
                        if let Some((g, level)) = b.lod {
                            if select_lod(&m.lod_distances[g as usize], distance) != Some(level) {
                                continue;
                            }
                        }
                        pass.set_pipeline(&self.pipelines[b.blend.index() * 2 + b.depth_offset as usize]);
                        pass.set_bind_group(2, &m.materials[*mat], &[]);
                        pass.draw_indexed(b.first..b.first + b.count, 0, 0..1);
                    }
                }
            }
        }
        enc
    }

    pub fn render(&mut self, view_proj: Mat4, distance: f32, meshes: &[&GpuMesh]) {
        let Some(surface) = &self.surface else { return };
        let frame = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            _ => {
                surface.configure(&self.device, &self.config);
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let enc = self.draw(&view, view_proj, distance, meshes);
        self.queue.submit([enc.finish()]);
        if let Some(w) = &self.window {
            w.pre_present_notify();
        }
        self.queue.present(frame);
    }

    /// Renders one frame offscreen and returns tightly packed RGBA8 pixels.
    pub fn screenshot(&self, view_proj: Mat4, distance: f32, meshes: &[&GpuMesh]) -> Vec<u8> {
        let (w, h) = (self.config.width, self.config.height);
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("screenshot"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let mut enc = self.draw(&view, view_proj, distance, meshes);
        let row = (w * 4).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        enc.copy_texture_to_buffer(
            tex.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map readback buffer"));
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
        let data = slice.get_mapped_range().expect("map readback range");
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            out.extend_from_slice(&data[y * row as usize..y * row as usize + w as usize * 4]);
        }
        out
    }
}
