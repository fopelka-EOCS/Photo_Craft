//! GPU-resident model preview. Only camera uniforms change during navigation.
//! Separate depth/MSAA attachments keep the editor's own render pass untouched.
use eframe::{
    egui_wgpu::RenderState,
    wgpu::{self, util::DeviceExt},
};
use photocraft_uv::{
    Scene,
    prepared::Geometry,
    preview::{Camera, Image},
};
use std::{collections::BTreeSet, sync::Arc};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const STRIDE: u64 = 72;
type Key = (u64, u64, u64, u64);
struct GpuSurface {
    maps: Vec<Map>,
    present: [bool; 5],
    version: u64,
}
impl GpuSurface {
    fn flags(&self) -> u32 {
        self.present.iter().enumerate().fold(0, |bits, (i, yes)| bits | (u32::from(*yes) << i))
    }
}
struct GpuEnvironment {
    source: Arc<photocraft_uv::lighting::Environment>,
    layout: wgpu::BindGroupLayout,
    output_hash: u64,
    group: wgpu::BindGroup,
    _textures: Vec<wgpu::Texture>,
}
impl GpuEnvironment {
    fn new(
        rs: &RenderState,
        _mips: &crate::uv_mips::Mips,
        source: Arc<photocraft_uv::lighting::Environment>,
        output: Option<&(u64, Image)>,
    ) -> Result<Self, String> {
        let device = &rs.device;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uv environment"),
            entries: &[
                env_texture_entry(0),
                env_texture_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                env_texture_entry(3),
                env_texture_entry(4),
            ],
        });
        let upload = |levels: &[Image]| -> Result<(wgpu::Texture, wgpu::TextureView), String> {
            let first = levels.first().ok_or("Empty environment")?;
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("uv linear HDR environment"),
                size: extent([first.width as u32, first.height as u32]),
                mip_level_count: levels.len() as u32,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            for (mip, im) in levels.iter().enumerate() {
                let data: Vec<_> = im.rgba.iter().flat_map(|p| p.iter().flat_map(|x| crate::gpu_canvas::f32_to_f16(*x).to_le_bytes())).collect();
                rs.queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: mip as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                    &data,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(im.width as u32 * 8), rows_per_image: Some(im.height as u32) },
                    extent([im.width as u32, im.height as u32]),
                );
            }
            let view = tex.create_view(&Default::default());
            Ok((tex, view))
        };
        let (specular, sv) = upload(&source.specular)?;
        let (diffuse, dv) = upload(std::slice::from_ref(&source.diffuse))?;
        let (brdf, bv) = upload(std::slice::from_ref(&photocraft_uv::lighting::brdf_lut()))?;
        let (output_texture, ov) = upload(std::slice::from_ref(output.map_or(&crate::uv_lighting::identity_lut(), |(_, im)| im)))?;
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("uv sky sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("uv sky"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&sv) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&dv) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&bv) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&ov) },
            ],
        });
        Ok(Self { source, layout, output_hash: output.map_or(0, |(hash, _)| *hash), group, _textures: vec![specular, diffuse, brdf, output_texture] })
    }
}
fn env_texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}
struct Map {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: [u32; 2],
    key: Key,
    data: Vec<u8>,
    cutout: bool,
    levels: Vec<wgpu::TextureView>,
}
struct Material {
    masked: bool,
    cutout: bool,
    color: Map,
    normal: Map,
    group: wgpu::BindGroup,
    _uniform: wgpu::Buffer,
    surface_version: u64,
}
struct Batch {
    material: usize,
    vertices: std::ops::Range<u32>,
}
struct Mesh {
    geometry: Arc<Geometry>,
    vertices: wgpu::Buffer,
    batches: Vec<Batch>,
    selection: wgpu::Buffer,
    group: wgpu::BindGroup,
    selected: BTreeSet<usize>,
}
struct Target {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    _depth: wgpu::Texture,
    depth: wgpu::TextureView,
    _depth_msaa: Option<wgpu::Texture>,
    depth_msaa: Option<wgpu::TextureView>,
    _msaa: Option<wgpu::Texture>,
    msaa: Option<wgpu::TextureView>,
    size: [u32; 2],
    samples: u32,
}

pub(crate) struct Renderer {
    rs: RenderState,
    health: photocraft_gpu::DeviceHealth,
    adapter_label: String,
    camera: wgpu::Buffer,
    lighting: photocraft_uv::lighting::Settings,
    environment: GpuEnvironment,
    surfaces: Vec<GpuSurface>,
    background_pipelines: Vec<wgpu::RenderPipeline>,
    camera_layout: wgpu::BindGroupLayout,
    material_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    mips: crate::uv_mips::Mips,
    pipelines: Vec<wgpu::RenderPipeline>,
    aa_samples: u32,
    mesh: Option<Mesh>,
    materials: Vec<Option<Material>>,
    target: Option<Target>,
    spare_target: Option<Target>,
    texture_id: Option<egui::TextureId>,
    last: Option<(u64, Camera, [u32; 2], bool, u64)>,
    map_generation: u64,
    regions: std::collections::BTreeMap<(usize, bool), crate::uv_lighting::RegionHint>,
    #[cfg(test)]
    timer: Option<photocraft_gpu::timing::GpuTimer>,
}
pub(crate) struct Draw<'a> {
    pub scene: &'a Scene,
    pub geometry: Arc<Geometry>,
    pub camera: Camera,
    pub size: [u32; 2],
    pub antialias: bool,
    pub selected: &'a BTreeSet<usize>,
    pub hidden: &'a BTreeSet<usize>,
    pub generation: u64,
    pub colors: &'a [Option<Image>],
    pub normals: &'a [Option<Image>],
    pub keys: &'a [Key],
    pub normal_keys: &'a [(u64, u64, u64)],
    pub masked: &'a BTreeSet<usize>,
}
fn floats(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn extent(size: [u32; 2]) -> wgpu::Extent3d {
    wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 }
}
fn texture(device: &wgpu::Device, size: [u32; 2], format: wgpu::TextureFormat, samples: u32, usage: wgpu::TextureUsages, label: &str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: extent(size),
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}
impl Renderer {
    fn ensure_surface(&mut self, id: usize) -> Result<(), String> {
        while self.surfaces.len() <= id {
            let mut maps = Vec::new();
            for _ in 0..5 {
                maps.push(self.map_region(None, Key::default(), true, None, None)?);
            }
            self.surfaces.push(GpuSurface { maps, present: [false; 5], version: 0 });
        }
        Ok(())
    }
    pub(crate) fn set_lighting(
        &mut self,
        settings: photocraft_uv::lighting::Settings,
        maps: &crate::uv_lighting::SurfaceMaps,
        environment: Arc<photocraft_uv::lighting::Environment>,
    ) -> Result<(), String> {
        let settings = settings.sanitized();
        if settings != self.lighting {
            self.lighting = settings;
            self.last = None;
        }
        if !Arc::ptr_eq(&self.environment.source, &environment) || self.environment.output_hash != maps.output.as_ref().map_or(0, |(hash, _)| *hash) {
            self.environment = GpuEnvironment::new(&self.rs, &self.mips, environment, maps.output.as_ref())?;
            self.last = None;
        }
        for material in 0..maps.images.iter().map(Vec::len).max().unwrap_or(0) {
            self.ensure_surface(material)?;
            for kind in 0..5 {
                let image = maps.images.get(kind).and_then(|m| m.get(material)).and_then(Option::as_ref);
                let key = maps.keys.get(kind).and_then(|m| m.get(material)).copied().unwrap_or_default();
                let changed = self.surfaces.get(material).and_then(|s| s.maps.get(kind)).is_none_or(|m| m.key != key)
                    || self.surfaces.get(material).and_then(|s| s.present.get(kind)).copied() != Some(image.is_some());
                if changed {
                    let previous = self.surfaces.get_mut(material).and_then(|s| if kind < s.maps.len() { Some(s.maps.remove(kind)) } else { None });
                    let hint = maps.regions.get(kind).and_then(|slots| slots.get(material)).cloned().flatten();
                    let map = self.map_region(image, key, true, previous, hint)?;
                    if let Some(surface) = self.surfaces.get_mut(material) {
                        surface.maps.insert(kind, map);
                        if let Some(v) = surface.present.get_mut(kind) {
                            *v = image.is_some();
                        }
                        surface.version = surface.version.wrapping_add(1);
                    }
                    self.map_generation = self.map_generation.wrapping_add(1);
                }
            }
        }
        Ok(())
    }
    pub fn adapter_label(&self) -> &str {
        &self.adapter_label
    }
    pub fn texture_sizes(&self) -> Vec<([u32; 2], [u32; 2])> {
        self.materials.iter().map(|m| m.as_ref().map_or(([0; 2], [0; 2]), |m| (m.color.size, m.normal.size))).collect()
    }
    pub fn max_texture_side(&self) -> u32 {
        self.rs.device.limits().max_texture_dimension_2d
    }
    pub fn frame_info(&self) -> Option<([u32; 2], u32)> {
        self.target.as_ref().map(|t| (t.size, t.samples))
    }
    pub fn clear(&mut self) {
        self.regions.clear();
        self.mesh = None;
        self.materials.clear();
        self.surfaces.clear();
        self.target = None;
        self.spare_target = None;
        self.last = None;
        if let Some(id) = self.texture_id.take() {
            self.rs.renderer.write().free_texture(&id);
        }
    }
    pub fn new(rs: RenderState, health: photocraft_gpu::DeviceHealth) -> Result<Self, String> {
        use std::future::Future;
        let validation = rs.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let memory = rs.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Self::create(rs.clone(), health)));
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        for pop in [memory.pop(), validation.pop()] {
            if let std::task::Poll::Ready(Some(error)) = std::pin::pin!(pop).poll(&mut cx) {
                return Err(format!("3D GPU setup failed: {error}"));
            }
        }
        result.map_err(|_| "3D GPU setup panicked; using CPU preview".to_owned())?
    }
    fn create(rs: RenderState, health: photocraft_gpu::DeviceHealth) -> Result<Self, String> {
        if !crate::gpu_canvas::supports_f16_canvas(&rs.adapter) {
            return Err("GPU does not support filtered half-float normal-map mip targets".into());
        }
        let device = &rs.device;
        if device.limits().max_storage_buffers_per_shader_stage == 0 {
            return Err("GPU model preview needs fragment storage buffers".into());
        }
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uv camera"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uv material"),
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let mips = crate::uv_mips::Mips::new(&rs);
        let environment = GpuEnvironment::new(&rs, &mips, Arc::new(photocraft_uv::lighting::Environment::studio()), None)?;
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("uv"),
            bind_group_layouts: &[Some(&camera_layout), Some(&material_layout), Some(&environment.layout)],
            immediate_size: 0,
        });
        let shader = device
            .create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("uv"), source: wgpu::ShaderSource::Wgsl(include_str!("uv_gpu.wgsl").into()) });
        let aa_samples = if [FORMAT, DEPTH].iter().all(|f| rs.adapter.get_texture_format_features(*f).flags.sample_count_supported(4)) { 4 } else { 1 };
        let make_pipeline = |count, cull, cutout| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("uv model"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: STRIDE,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x3,3=>Float32x3,4=>Float32x3,5=>Float32x2,6=>Uint32],
                    })],
                    compilation_options: Default::default(),
                },
                primitive: wgpu::PrimitiveState { cull_mode: cull, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState { count, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(if cutout { "fs_cutout" } else { "fs_opaque" }),
                    targets: &[Some(wgpu::ColorTargetState { format: FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let background_pipelines = [1, aa_samples]
            .into_iter()
            .map(|count| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("uv sky"),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_background"), buffers: &[], compilation_options: Default::default() },
                    primitive: Default::default(),
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: DEPTH,
                        depth_write_enabled: Some(false),
                        depth_compare: Some(wgpu::CompareFunction::Always),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: wgpu::MultisampleState { count, ..Default::default() },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_background"),
                        targets: &[Some(wgpu::ColorTargetState { format: FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                        compilation_options: Default::default(),
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            })
            .collect();
        let mut pipelines = Vec::new();
        for samples in [1, aa_samples] {
            for cull in [None, Some(wgpu::Face::Back), Some(wgpu::Face::Front)] {
                for cutout in [false, true] {
                    pipelines.push(make_pipeline(samples, cull, cutout));
                }
            }
        }
        let camera = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uv camera"),
            size: 128,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("uv repeat bilinear"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: if rs.adapter.get_downlevel_capabilities().flags.contains(wgpu::DownlevelFlags::ANISOTROPIC_FILTERING) { 4 } else { 1 },
            ..Default::default()
        });
        #[cfg(test)]
        let timer = photocraft_gpu::timing::GpuTimer::new(&rs.device, &rs.queue);
        let adapter_label = format!("{} · {:?}", rs.adapter.get_info().name, rs.adapter.get_info().backend);
        Ok(Self {
            rs,
            health,
            adapter_label,
            camera,
            lighting: photocraft_uv::lighting::Settings { pbr: false, ..Default::default() },
            environment,
            surfaces: Vec::new(),
            background_pipelines,
            camera_layout,
            material_layout,
            sampler,
            mips,
            pipelines,
            aa_samples,
            mesh: None,
            materials: Vec::new(),
            target: None,
            spare_target: None,
            texture_id: None,
            last: None,
            map_generation: 0,
            regions: Default::default(),
            #[cfg(test)]
            timer,
        })
    }
    fn mesh(&mut self, geometry: Arc<Geometry>) -> Result<bool, String> {
        if self.mesh.as_ref().is_some_and(|m| Arc::ptr_eq(&m.geometry, &geometry)) {
            return Ok(false);
        }
        let count = geometry.triangles.len().checked_mul(3).ok_or("GPU mesh too large")?;
        let len = count.checked_mul(STRIDE as usize).ok_or("GPU mesh too large")?;
        if len as u64 > self.rs.device.limits().max_buffer_size {
            return Err("Model exceeds the GPU vertex buffer limit".into());
        }
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(len).map_err(|_| "Not enough memory to upload model")?;
        let mut order: Vec<_> = (0..geometry.triangles.len()).collect();
        order.sort_unstable_by_key(|id| geometry.triangles.get(*id).map_or(usize::MAX, |t| t.material));
        let mut batches: Vec<Batch> = Vec::new();
        let mut at = 0u32;
        for id in order {
            let Some(t) = geometry.triangles.get(id) else { continue };
            if let Some(batch) = batches.last_mut().filter(|b| b.material == t.material) {
                batch.vertices.end += 3;
            } else {
                batches.push(Batch { material: t.material, vertices: at..at + 3 });
            }
            at += 3;
            for corner in 0..3 {
                for value in t.positions[corner].into_iter().chain(t.normals[corner]).chain(t.face).chain(t.tangent).chain(t.bitangent).chain(t.uv[corner]) {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
                bytes.extend_from_slice(&t.shell.to_le_bytes());
            }
        }
        let device = &self.rs.device;
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("uv static vertices"),
            contents: &bytes,
            usage: wgpu::BufferUsages::VERTEX,
        });
        let shells = geometry.triangles.iter().map(|t| u64::from(t.shell) + 1).max().unwrap_or(1);
        if shells * 4 > device.limits().max_storage_buffer_binding_size {
            return Err("UV selection exceeds GPU storage limit".into());
        }
        let selection = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uv shell selection"),
            size: shells * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("uv view"),
            layout: &self.camera_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.camera.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: selection.as_entire_binding() },
            ],
        });
        self.mesh = Some(Mesh { geometry, vertices, batches, selection, group, selected: BTreeSet::new() });
        self.materials.clear();
        self.last = None;
        Ok(true)
    }
    pub(crate) fn hint_region(&mut self, material: usize, normal: bool, previous: Key, rect: Vec<[u32; 4]>) {
        self.regions.insert((material, normal), (previous, rect));
    }
    #[cfg(test)]
    fn map(&self, image: Option<&Image>, key: Key, normal: bool, previous: Option<Map>) -> Result<Map, String> {
        self.map_region(image, key, normal, previous, None)
    }
    fn map_region(
        &self,
        image: Option<&Image>,
        key: Key,
        normal: bool,
        previous: Option<Map>,
        hint: Option<crate::uv_lighting::RegionHint>,
    ) -> Result<Map, String> {
        if let Some((expected, regions)) = hint
            && previous.as_ref().is_some_and(|m| m.key == expected)
            && image.is_some_and(|im| previous.as_ref().is_some_and(|m| m.size == [im.width as u32, im.height as u32]))
        {
            let mut current = previous;
            if regions.is_empty()
                && let Some(m) = current.as_mut()
            {
                m.key = key;
            }
            let mut from = expected;
            for rect in regions {
                current = Some(self.map_single(image, key, normal, current, Some((from, rect)))?);
                from = key;
            }
            return current.ok_or_else(|| "Missing texture".into());
        }
        self.map_single(image, key, normal, previous, None)
    }
    fn map_single(&self, image: Option<&Image>, key: Key, normal: bool, mut previous: Option<Map>, hint: Option<(Key, [u32; 4])>) -> Result<Map, String> {
        if hint.is_none() && previous.as_ref().is_some_and(|m| m.key == key) {
            return previous.ok_or_else(|| "Missing cached texture".into());
        }
        let size = image.map_or([1, 1], |im| [im.width as u32, im.height as u32]);
        let max = self.rs.device.limits().max_texture_dimension_2d;
        if size.contains(&0)
            || image.is_some_and(|im| im.width > max as usize || im.height > max as usize || im.width.checked_mul(im.height) != Some(im.rgba.len()))
        {
            return Err("Invalid material image dimensions or length".into());
        }
        if let (Some(im), Some((expected, [x, y, w, h]))) = (image, hint)
            && previous.as_ref().is_some_and(|old| old.key == expected && old.size == size)
            && x.checked_add(w).is_some_and(|end| end <= size[0])
            && y.checked_add(h).is_some_and(|end| end <= size[1])
        {
            let mut old = previous.take().ok_or("Missing previous map")?;
            let stride = if normal { 8usize } else { 4 };
            let mut bytes = Vec::new();
            bytes.try_reserve_exact(w as usize * h as usize * stride).map_err(|_| "Not enough memory for texture update")?;
            for row in im.rgba.chunks_exact(im.width).skip(y as usize).take(h as usize) {
                for pixel in row.get(x as usize..(x + w) as usize).ok_or("Invalid texture update")? {
                    for value in pixel {
                        if normal {
                            bytes.extend_from_slice(&crate::gpu_canvas::f32_to_f16(*value).to_le_bytes());
                        } else {
                            bytes.push((value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
                        }
                    }
                    old.cutout |= !normal && pixel[3] < 1.0;
                }
            }
            if w > 0 && h > 0 {
                self.rs.queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: &old.texture, mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All },
                    &bytes,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * stride as u32), rows_per_image: Some(h) },
                    extent([w, h]),
                );
                for (row, source) in bytes.chunks_exact(w as usize * stride).enumerate() {
                    let start = ((y as usize + row) * im.width + x as usize) * stride;
                    old.data.get_mut(start..start + source.len()).ok_or("Invalid cached texture update")?.copy_from_slice(source);
                }
                self.mips.regenerate(&self.rs, &old.texture, size, &old.levels, [x, y, w, h]);
            }
            old.key = key;
            return Ok(old);
        }
        let cutout = !normal && image.is_some_and(|im| im.rgba.iter().any(|p| p[3] < 1.0));
        let max = self.rs.device.limits().max_texture_dimension_2d;
        if size.contains(&0) || size.iter().any(|x| *x > max) {
            return Err("Material map exceeds GPU texture limit".into());
        }
        if image.is_some_and(|im| im.width.checked_mul(im.height) != Some(im.rgba.len())) {
            return Err("Invalid material image length".into());
        }
        let data: Vec<u8> = if !normal {
            image.map_or_else(|| vec![255u8; 4], |im| im.rgba.iter().flat_map(|p| p.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)).collect())
        } else if let Some(im) = image {
            im.rgba.iter().flat_map(|p| p.iter().flat_map(|x| crate::gpu_canvas::f32_to_f16(*x).to_le_bytes())).collect()
        } else {
            [0.5, 0.5, 1.0, 1.0].into_iter().flat_map(|x| crate::gpu_canvas::f32_to_f16(x).to_le_bytes()).collect()
        };
        let bytes_per_texel = if normal { 8 } else { 4 };
        if let Some(mut old) = previous.filter(|m| m.size == size) {
            if let Some([x, y, w, h]) = changed_region(&old.data, &data, size, bytes_per_texel) {
                let offset = (y as usize * size[0] as usize + x as usize) * bytes_per_texel;
                let Some(bytes) = data.get(offset..) else {
                    return Err("Invalid texture update region".into());
                };
                self.rs.queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: &old.texture, mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All },
                    bytes,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size[0] * bytes_per_texel as u32), rows_per_image: Some(h) },
                    extent([w, h]),
                );
                self.mips.regenerate(&self.rs, &old.texture, size, &old.levels, [x, y, w, h]);
            }
            old.data = data;
            old.key = key;
            old.cutout = cutout;
            return Ok(old);
        }
        let levels_n = 32 - size[0].max(size[1]).leading_zeros();
        let texture = self.rs.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("uv full-resolution map"),
            size: extent(size),
            mip_level_count: levels_n,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: if normal { wgpu::TextureFormat::Rgba16Float } else { FORMAT },
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let levels: Vec<_> = (0..levels_n)
            .map(|level| texture.create_view(&wgpu::TextureViewDescriptor { base_mip_level: level, mip_level_count: Some(1), ..Default::default() }))
            .collect();
        self.rs.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size[0] * bytes_per_texel as u32), rows_per_image: Some(size[1]) },
            extent(size),
        );
        let view = texture.create_view(&Default::default());
        self.mips.regenerate(&self.rs, &texture, size, &levels, [0, 0, size[0], size[1]]);
        Ok(Map { texture, view, size, key, data, cutout, levels })
    }
    fn materials(&mut self, draw: &Draw<'_>) -> Result<(), String> {
        if draw.scene.materials.len() > 4096 {
            return Err("Model exceeds GPU material budget (4096)".into());
        }
        let keys: Vec<_> = draw
            .scene
            .materials
            .iter()
            .enumerate()
            .map(|(id, _)| {
                let c = draw.keys.get(id).copied().unwrap_or_default();
                let n = draw.normal_keys.get(id).copied().unwrap_or_default();
                (
                    draw.colors.get(id).and_then(Option::as_ref).map_or((0, 0, 0, 0), |im| (c.0, c.1, c.2, c.3 ^ ((im.width as u64) << 32) ^ im.height as u64)),
                    draw.normals.get(id).and_then(Option::as_ref).map_or((0, 0, 0, 0), |im| (n.0, n.1, n.2, ((im.width as u64) << 32) | im.height as u64)),
                )
            })
            .collect();
        for (id, material) in draw.scene.materials.iter().enumerate() {
            let Some(&(ck, nk)) = keys.get(id) else { continue };
            if self.materials.get(id).and_then(Option::as_ref).is_some_and(|m| {
                m.masked == draw.masked.contains(&id)
                    && m.color.key == ck
                    && m.normal.key == nk
                    && m.surface_version == self.surfaces.get(id).map_or(0, |s| s.version)
            }) {
                continue;
            }
            // Cache by document revision: moving the camera never uploads these maps.
            let previous = self.materials.get_mut(id).and_then(Option::take);
            let (pc, pn) = previous.map_or((None, None), |m| (Some(m.color), Some(m.normal)));
            let color_hint = self.regions.remove(&(id, false));
            let normal_hint = self.regions.remove(&(id, true));
            let color = self.map_region(draw.colors.get(id).and_then(Option::as_ref), ck, false, pc, color_hint)?;
            let normal = self.map_region(draw.normals.get(id).and_then(Option::as_ref), nk, true, pn, normal_hint)?;
            self.ensure_surface(id)?;
            let surface = self.surfaces.get(id).ok_or("Missing PBR maps")?;
            let uniform = self.rs.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("uv material"),
                contents: &floats(&[
                    material.color[0],
                    material.color[1],
                    material.color[2],
                    material.color[3],
                    if nk.3 != 0 { 1.0 } else { 0.0 },
                    surface.flags() as f32,
                    f32::from(draw.masked.contains(&id)),
                    0.0,
                ]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let group = self.rs.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("uv material"),
                layout: &self.material_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&color.view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&normal.view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 3, resource: uniform.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&surface.maps[0].view) },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&surface.maps[1].view) },
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&surface.maps[2].view) },
                    wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&surface.maps[3].view) },
                    wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&surface.maps[4].view) },
                ],
            });
            let value = Material {
                masked: draw.masked.contains(&id),
                surface_version: self.surfaces.get(id).map_or(0, |s| s.version),
                cutout: !draw.masked.contains(&id) && (color.cutout || material.color[3] < 1.0),
                color,
                normal,
                group,
                _uniform: uniform,
            };
            if let Some(slot) = self.materials.get_mut(id) {
                *slot = Some(value);
            } else {
                self.materials.push(Some(value));
            }
            self.map_generation = self.map_generation.wrapping_add(1);
        }
        self.regions.clear();
        Ok(())
    }
    pub fn draw(&mut self, draw: Draw<'_>) -> Result<egui::TextureId, String> {
        if !self.health.is_ok() {
            return Err(self.health.fault().map_or_else(|| "GPU unavailable".into(), |f| f.to_string()));
        }
        self.mesh(draw.geometry.clone())?;
        self.materials(&draw)?;
        let samples = if draw.antialias { self.aa_samples } else { 1 };
        let key = (draw.generation, draw.camera, draw.size, draw.antialias, self.map_generation);
        if self.last == Some(key) && self.mesh.as_ref().is_some_and(|mesh| &mesh.selected == draw.selected) {
            return self.texture_id.ok_or_else(|| "GPU viewport texture unavailable".into());
        }
        let device = &self.rs.device;
        if draw.size.contains(&0)
            || draw.size.iter().any(|s| *s > device.limits().max_texture_dimension_2d)
            || u64::from(draw.size[0]) * u64::from(draw.size[1]) > 16_777_216
        {
            return Err("GPU viewport exceeds texture budget".into());
        }
        if self.target.as_ref().is_none_or(|t| t.size != draw.size) && self.spare_target.as_ref().is_some_and(|t| t.size == draw.size) {
            std::mem::swap(&mut self.target, &mut self.spare_target);
            if let (Some(id), Some(target)) = (self.texture_id, &self.target) {
                self.rs.renderer.write().update_egui_texture_from_wgpu_texture(device, &target.view, wgpu::FilterMode::Linear, id);
            }
        }
        if self.target.as_ref().is_none_or(|t| t.size != draw.size) {
            self.spare_target = self.target.take();
            let texture = texture(
                device,
                draw.size,
                FORMAT,
                1,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                "uv viewport",
            );
            let view = texture.create_view(&Default::default());
            let depth_tex = super_texture(device, draw.size, DEPTH, 1, "uv depth");
            let depth = depth_tex.create_view(&Default::default());
            let mut renderer = self.rs.renderer.write();
            if let Some(id) = self.texture_id {
                renderer.update_egui_texture_from_wgpu_texture(device, &view, wgpu::FilterMode::Linear, id);
            } else {
                self.texture_id = Some(renderer.register_native_texture(device, &view, wgpu::FilterMode::Linear));
            }
            self.target = Some(Target {
                _texture: texture,
                view,
                _depth: depth_tex,
                depth,
                _depth_msaa: None,
                depth_msaa: None,
                _msaa: None,
                msaa: None,
                size: draw.size,
                samples,
            });
        }
        if let Some(target) = &mut self.target {
            if samples > 1 && target.msaa.is_none() {
                let color = super_texture(device, draw.size, FORMAT, samples, "uv cached MSAA");
                let depth = super_texture(device, draw.size, DEPTH, samples, "uv cached MSAA depth");
                target.msaa = Some(color.create_view(&Default::default()));
                target._msaa = Some(color);
                target.depth_msaa = Some(depth.create_view(&Default::default()));
                target._depth_msaa = Some(depth);
            }
            target.samples = samples;
        }
        let target = self.target.as_ref().ok_or("Missing GPU viewport")?;
        let mesh = self.mesh.as_mut().ok_or("Missing GPU mesh")?;
        if &mesh.selected != draw.selected {
            let count = (mesh.selection.size() / 4) as usize;
            let mut data = vec![0u8; count * 4];
            for shell in draw.selected {
                if let Some(bytes) = data.get_mut(shell.saturating_mul(4)..shell.saturating_mul(4).saturating_add(4)) {
                    bytes.copy_from_slice(&1u32.to_le_bytes());
                }
            }
            self.rs.queue.write_buffer(&mesh.selection, 0, &data);
            mesh.selected = draw.selected.clone();
        }
        let mut values = draw.geometry.camera_values(draw.camera, draw.size).to_vec();
        let s = self.lighting;
        values.extend_from_slice(&[
            f32::from(s.pbr),
            s.brightness,
            s.rotation.to_radians(),
            f32::from(s.background),
            s.roughness,
            s.metallic,
            s.emissive,
            s.background_brightness,
        ]);
        values.extend_from_slice(&[0.0; 8]);
        self.rs.queue.write_buffer(&self.camera, 0, &floats(&values));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("uv model") });
        #[cfg(test)]
        if let Some(timer) = &mut self.timer {
            timer.collect();
            timer.start(&mut encoder, u64::from(draw.size[0]) * u64::from(draw.size[1]));
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("uv model"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: if samples > 1 { target.msaa.as_ref().unwrap_or(&target.view) } else { &target.view },
                    resolve_target: (samples > 1).then_some(&target.view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 35.0 / 255.0, g: 35.0 / 255.0, b: 35.0 / 255.0, a: 1.0 }),
                        store: if samples > 1 { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store },
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: if samples > 1 { target.depth_msaa.as_ref().unwrap_or(&target.depth) } else { &target.depth },
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &mesh.group, &[]);
            pass.set_bind_group(2, &self.environment.group, &[]);
            if self.lighting.background
                && draw.camera.channel == photocraft_uv::preview::Channel::Material
                && let (Some(pipeline), Some(material)) = (self.background_pipelines.get(usize::from(samples > 1)), self.materials.iter().flatten().next())
            {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(1, &material.group, &[]);
                pass.draw(0..3, 0..1);
            }
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            for batch in &mesh.batches {
                if !draw.hidden.contains(&batch.material)
                    && let Some(m) = self.materials.get(batch.material).and_then(Option::as_ref)
                {
                    let cull = if !draw.camera.cull_backfaces {
                        0
                    } else if draw.camera.flip_faces {
                        2
                    } else {
                        1
                    };
                    let index = usize::from(samples > 1) * 6 + cull * 2 + usize::from(m.cutout);
                    let Some(pipeline) = self.pipelines.get(index) else { continue };
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(1, &m.group, &[]);
                    pass.draw(batch.vertices.clone(), 0..1);
                }
            }
        }
        #[cfg(test)]
        if let Some(timer) = &mut self.timer {
            timer.finish(&mut encoder);
        }
        self.rs.queue.submit([encoder.finish()]);
        #[cfg(test)]
        if let Some(timer) = &mut self.timer {
            timer.submitted();
        }
        self.last = Some(key);
        self.texture_id.ok_or_else(|| "Missing GPU texture id".into())
    }
}
/// Smallest texel-aligned rectangle containing changed half-RGBA texels.
fn changed_region(old: &[u8], new: &[u8], size: [u32; 2], stride: usize) -> Option<[u32; 4]> {
    if old.len() != new.len() {
        return Some([0, 0, size[0], size[1]]);
    }
    let (mut x0, mut y0, mut x1, mut y1) = (size[0], size[1], 0, 0);
    for (id, (a, b)) in old.chunks_exact(stride.max(1)).zip(new.chunks_exact(stride.max(1))).enumerate() {
        if a != b {
            let (x, y) = (id as u32 % size[0].max(1), id as u32 / size[0].max(1));
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
        }
    }
    (x0 < x1 && y0 < y1).then(|| [x0, y0, x1 - x0, y1 - y0])
}
fn super_texture(device: &wgpu::Device, size: [u32; 2], format: wgpu::TextureFormat, samples: u32, label: &str) -> wgpu::Texture {
    texture(device, size, format, samples, wgpu::TextureUsages::RENDER_ATTACHMENT, label)
}
impl Drop for Renderer {
    fn drop(&mut self) {
        if let Some(id) = self.texture_id {
            self.rs.renderer.write().free_texture(&id);
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[allow(clippy::unwrap_used)] // Assertions in test-only GPU setup/readback helpers.
mod tests {
    use super::*;
    use photocraft_uv::{
        import,
        preview::{self, Materials},
        topology::Topology,
    };
    fn renderer() -> Renderer {
        let rs = egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default());
        let health = photocraft_gpu::DeviceHealth::watch(&rs.device);
        let renderer = Renderer::new(rs, health).unwrap();
        assert!(renderer.health.is_ok(), "{:?}", renderer.health.fault());
        renderer
    }
    #[test]
    fn uv_gpu_mask_is_unlit_in_material_and_normal_modes_and_restores_cached_material() {
        let _gpu = photocraft_testkit::gpu_lock();
        let (mut scene, g) = scene();
        let mut r = renderer();
        let empty = BTreeSet::new();
        let masked = BTreeSet::from([0usize]);
        scene.materials[0].color = [0.2, 0.8, 0.1, 0.0]; // A transparent material must still show its mask.
        let colors = vec![Some(Image { width: 4, height: 4, rgba: vec![[0.25, 0.25, 0.25, 1.0]; 16] }); scene.materials.len()];
        for channel in [photocraft_uv::preview::Channel::Material, photocraft_uv::preview::Channel::Normal] {
            r.draw(Draw {
                scene: &scene,
                geometry: g.clone(),
                camera: Camera { channel, shaded: true, ..Default::default() },
                size: [256, 256],
                antialias: false,
                selected: &empty,
                hidden: &empty,
                generation: 1,
                colors: &colors,
                normals: &[],
                keys: &[(1, 1, 0, 0)],
                normal_keys: &[],
                masked: &masked,
            })
            .unwrap();
            let px = read(&r);
            assert!(
                px.iter().filter(|p| p[0].abs_diff(64) < 2 && p[1].abs_diff(64) < 2 && p[2].abs_diff(64) < 2).count() > 10,
                "mask is visible, grayscale and unlit"
            );
            assert!(r.materials[0].as_ref().unwrap().masked);
        }
        r.draw(Draw {
            scene: &scene,
            geometry: g,
            camera: Camera::default(),
            size: [256, 256],
            antialias: false,
            selected: &empty,
            hidden: &empty,
            generation: 1,
            colors: &colors,
            normals: &[],
            keys: &[(1, 1, 0, 0)],
            normal_keys: &[],
            masked: &empty,
        })
        .unwrap();
        assert!(!r.materials[0].as_ref().unwrap().masked);
        assert!(r.health.is_ok());
    }
    fn scene() -> (Scene, Arc<Geometry>) {
        let bytes = include_bytes!("../../uv/tests/fixtures/normals-reflected.fbx");
        let scene = import::load("test.fbx", bytes, 0, |_| Err(photocraft_uv::Error("absent".into()))).unwrap();
        let geometry = Arc::new(Geometry::new(&scene, &Topology::build(&scene).unwrap()).unwrap());
        (scene, geometry)
    }
    fn read(renderer: &Renderer) -> Vec<[u8; 4]> {
        let target = renderer.target.as_ref().unwrap();
        read_texture(renderer, &target._texture, 0)
    }
    fn read_texture(renderer: &Renderer, texture: &wgpu::Texture, mip: u32) -> Vec<[u8; 4]> {
        let (w, h) = ((texture.width() >> mip).max(1), (texture.height() >> mip).max(1));
        let row = (w * 4).div_ceil(256) * 256;
        let buffer = renderer.rs.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uv test readback"),
            size: u64::from(row) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = renderer.rs.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture, mip_level: mip, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            extent([w, h]),
        );
        renderer.rs.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        assert!(renderer.health.wait(&renderer.rs.device, None), "{:?}", renderer.health.fault());
        rx.recv().unwrap().unwrap();
        let mapped = buffer.get_mapped_range(..).unwrap();
        let pixels = mapped.chunks_exact(row as usize).flat_map(|r| r[..w as usize * 4].as_chunks::<4>().0.iter().copied()).collect();
        drop(mapped);
        buffer.unmap();
        pixels
    }
    #[test]
    fn pbr_environment_emissive_and_resident_navigation() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut renderer = renderer();
        let (scene, geometry) = scene();
        let env = Arc::new(photocraft_uv::lighting::Environment::studio());
        let mut surfaces = crate::uv_lighting::SurfaceMaps::default();
        for (kind, value) in [(0, [0.4, 0.1, 0.03, 1.0]), (1, [0.8; 4]), (2, [0.15; 4]), (3, [1.0; 4]), (4, [0.0, 0.0, 0.0, 1.0])] {
            surfaces.images[kind] = scene.materials.iter().map(|_| Some(Image { width: 2, height: 2, rgba: vec![value; 4] })).collect();
            surfaces.keys[kind] = scene.materials.iter().map(|_| (kind as u64 + 1, 1, 0, 0)).collect();
        }
        let hidden = BTreeSet::new();
        let selected = BTreeSet::new();
        let render = |r: &mut Renderer, camera: Camera, hidden: &BTreeSet<usize>| {
            r.draw(Draw {
                masked: &BTreeSet::new(),
                scene: &scene,
                geometry: geometry.clone(),
                camera,
                size: [256, 256],
                antialias: false,
                selected: &selected,
                hidden,
                generation: 1,
                colors: &[],
                normals: &[],
                keys: &[],
                normal_keys: &[],
            })
            .unwrap();
            assert!(r.health.is_ok(), "{:?}", r.health.fault());
            read_texture(r, &r.target.as_ref().unwrap()._texture, 0)
        };
        let camera = Camera { shaded: true, ..Default::default() };
        let settings = photocraft_uv::lighting::Settings::default();
        renderer.set_lighting(settings, &surfaces, env.clone()).unwrap();
        let first = render(&mut renderer, camera, &hidden);
        let generation = renderer.map_generation;
        let map_pointer = renderer.surfaces[0].maps[0].data.as_ptr();
        for i in 0..12 {
            renderer.set_lighting(settings, &surfaces, env.clone()).unwrap();
            render(&mut renderer, Camera { yaw: camera.yaw + i as f64 * 0.01, ..camera }, &hidden);
        }
        assert_eq!(renderer.map_generation, generation, "orbit must not upload maps");
        assert_eq!(renderer.surfaces[0].maps[0].data.as_ptr(), map_pointer);
        for im in surfaces.images[2].iter_mut().flatten() {
            im.rgba.fill([1.0; 4]);
        }
        for key in &mut surfaces.keys[2] {
            key.1 += 1;
        }
        renderer.set_lighting(settings, &surfaces, env.clone()).unwrap();
        let rough = render(&mut renderer, camera, &hidden);
        assert!(first != rough, "roughness must change reflected lighting");
        let old = surfaces.keys[0][0];
        let pointer = renderer.surfaces[0].maps[0].data.as_ptr();
        surfaces.images[0][0].as_mut().unwrap().rgba[0] = [0.1, 0.5, 0.2, 1.0];
        surfaces.keys[0][0].1 += 1;
        surfaces.regions[0] = vec![None; scene.materials.len()];
        surfaces.regions[0][0] = Some((old, vec![[0, 0, 1, 1]]));
        renderer.set_lighting(settings, &surfaces, env.clone()).unwrap();
        assert_eq!(pointer, renderer.surfaces[0].maps[0].data.as_ptr(), "PBR brush damage updates the existing encoded map");
        renderer.set_lighting(photocraft_uv::lighting::Settings { brightness: 0.0, ..settings }, &surfaces, env.clone()).unwrap();
        let dark = render(&mut renderer, camera, &hidden);
        for im in surfaces.images[4].iter_mut().flatten() {
            im.rgba.fill([4.0, 0.0, 0.0, 1.0]);
        }
        for key in &mut surfaces.keys[4] {
            key.1 += 1;
        }
        renderer.set_lighting(photocraft_uv::lighting::Settings { brightness: 0.0, ..settings }, &surfaces, env.clone()).unwrap();
        let glow = render(&mut renderer, camera, &hidden);
        assert!(glow.iter().zip(&dark).any(|(a, b)| a[0] > 200 && a[1] < 5 && b[0] < 5), "emissive must remain visible with no environment light");
        renderer.set_lighting(photocraft_uv::lighting::Settings { pbr: false, ..settings }, &surfaces, env.clone()).unwrap();
        assert!(glow != render(&mut renderer, camera, &hidden));
        let hidden = (0..scene.materials.len()).collect();
        let source = Image { width: 8, height: 4, rgba: (0..32).map(|i| [(i % 8) as f32, 0.2, 0.1, 1.0]).collect() };
        let env = Arc::new(photocraft_uv::lighting::Environment::prepare(&source).unwrap());
        renderer.set_lighting(photocraft_uv::lighting::Settings { background: true, ..settings }, &surfaces, env.clone()).unwrap();
        let sky = render(&mut renderer, Camera { perspective: true, ..camera }, &hidden);
        renderer.set_lighting(photocraft_uv::lighting::Settings { background: true, rotation: 90.0, ..settings }, &surfaces, env).unwrap();
        assert!(sky != render(&mut renderer, Camera { perspective: true, ..camera }, &hidden), "environment rotation changes the background");
    }
    #[test]
    fn mip_levels_preserve_native_texels_and_refresh_after_small_edits() {
        let _gpu = photocraft_testkit::gpu_lock();
        let renderer = renderer();
        let mut image = Image {
            width: 8,
            height: 8,
            rgba: (0..64)
                .map(|i| {
                    let c = if (i / 8 + i % 8) % 2 == 0 { 0.0 } else { 1.0 };
                    [c, c, c, 1.0]
                })
                .collect(),
        };
        let map = renderer.map(Some(&image), (1, 1, 0, 0), false, None).unwrap();
        assert_eq!(map.texture.size(), extent([8, 8]));
        assert_eq!(map.levels.len(), 4);
        let before = read_texture(&renderer, &map.texture, 3);
        assert!(before[0][0].abs_diff(128) <= 1);
        assert_eq!(read_texture(&renderer, &map.texture, 0)[0], [0, 0, 0, 255]);
        image.rgba[0] = [1.0; 4];
        let map = renderer.map(Some(&image), (1, 2, 0, 0), false, Some(map)).unwrap();
        assert_eq!(read_texture(&renderer, &map.texture, 0)[0], [255; 4]);
        assert!(read_texture(&renderer, &map.texture, 3)[0][0] > before[0][0]);
    }
    #[test]
    fn dirty_region_tracks_changed_pixels_across_rows() {
        let _gpu = photocraft_testkit::gpu_lock();
        let old = vec![0; 6 * 5 * 8];
        let mut new = old.clone();
        assert_eq!(changed_region(&old, &new, [6, 5], 8), None);
        new[(2 * 6 + 4) * 8] = 1;
        new[(4 * 6 + 1) * 8 + 7] = 1;
        assert_eq!(changed_region(&old, &new, [6, 5], 8), Some([1, 2, 4, 3]));
    }
    #[test]
    fn hinted_map_updates_only_the_region_and_stale_hints_fall_back_safely() {
        let _gpu = photocraft_testkit::gpu_lock();
        let renderer = renderer();
        for normal in [false, true] {
            let mut image = Image { width: 8, height: 8, rgba: vec![[0.5, 0.5, 1.0, 1.0]; 64] };
            let map = renderer.map(Some(&image), (1, 1, 0, 0), normal, None).unwrap();
            let pointer = map.data.as_ptr();
            image.rgba[3 * 8 + 2] = [1.0, 0.0, 0.0, 0.0];
            let map = renderer.map_region(Some(&image), (1, 2, 0, 0), normal, Some(map), Some(((1, 1, 0, 0), vec![[2, 3, 1, 1]]))).unwrap();
            assert_eq!(pointer, map.data.as_ptr(), "small edits must retain the full encoded snapshot allocation");
            let stride = if normal { 8 } else { 4 };
            if normal {
                assert_eq!(&map.data[(3 * 8 + 2) * stride..(3 * 8 + 2) * stride + 2], &crate::gpu_canvas::f32_to_f16(1.0).to_le_bytes());
            } else {
                assert_eq!(read_texture(&renderer, &map.texture, 0)[3 * 8 + 2], [255, 0, 0, 0]);
                assert!(map.cutout);
            }
            // The cache skipped an update: an old key cannot authorize a partial upload.
            image.rgba[7 * 8 + 7] = [0.0, 1.0, 0.0, 1.0];
            let map = renderer.map_region(Some(&image), (1, 3, 0, 0), normal, Some(map), Some(((1, 1, 0, 0), vec![[2, 3, 1, 1]]))).unwrap();
            if !normal {
                assert_eq!(read_texture(&renderer, &map.texture, 0)[7 * 8 + 7], [0, 255, 0, 255]);
            }
        }
    }
    #[test]
    fn gpu_matches_reference_normals_camera_selection_hidden_materials_and_live_maps() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut renderer = renderer();
        let (scene, geometry) = scene();
        let mut colors: Vec<_> = scene.materials.iter().map(|_| Some(Image { width: 8, height: 8, rgba: vec![[0.8, 0.4, 0.2, 1.0]; 64] })).collect();
        let normals: Vec<_> = scene.materials.iter().map(|_| Some(Image { width: 8, height: 8, rgba: vec![[0.7, 0.4, 0.95, 1.0]; 64] })).collect();
        for step in 0..16u64 {
            let selected = if step == 1 { BTreeSet::from([0]) } else { BTreeSet::new() };
            let hidden = if step == 4 { BTreeSet::from([0]) } else { BTreeSet::new() };
            let camera = Camera {
                channel: match step {
                    12 | 14 => preview::Channel::Color,
                    13 | 15 => preview::Channel::Normal,
                    _ => preview::Channel::Material,
                },
                perspective: step >= 9 && step != 12 && step != 13,
                zoom: if step == 11 { 4.0 } else { 1.0 },
                yaw: 0.65 + step as f64 * 0.15,
                shaded: true,
                authored_normals: step != 2,
                flip_faces: step == 3,
                flip_normal_y: step == 5,
                cull_backfaces: step != 8,
                ..Default::default()
            };
            if step == 6 {
                colors[0].as_mut().unwrap().rgba[3 * 8 + 3] = [0.0, 1.0, 0.0, 1.0];
            }
            if step >= 7 {
                for image in colors.iter_mut().flatten() {
                    for p in &mut image.rgba {
                        p[3] = if step == 7 { 0.0 } else { 1.0 };
                    }
                }
            }
            let keys = vec![(1, if step >= 6 { step - 4 } else { 1 }, 0, 0); colors.len()];
            let nkeys = vec![(2, 1, 0); normals.len()];
            let draw = Draw {
                masked: &BTreeSet::new(),
                scene: &scene,
                geometry: geometry.clone(),
                camera,
                size: [256, 192],
                antialias: false,
                selected: &selected,
                hidden: &hidden,
                generation: step,
                colors: &colors,
                normals: &normals,
                keys: &keys,
                normal_keys: &nkeys,
            };
            renderer.draw(draw).unwrap();
            if step > 0 && step < 6 {
                assert_eq!(renderer.map_generation, scene.materials.len() as u64, "camera and shell selection must not reload materials");
            }
            let actual = read(&renderer);
            let maps = Materials { colors: &colors, normals: &normals, hidden: hidden.clone() };
            let selection: Vec<_> = geometry.triangles.iter().map(|t| selected.contains(&(t.shell as usize))).collect();
            let expected = preview::render_materials(&scene, camera, &maps, &selection, 256, 192).unwrap();
            let mut bad = 0;
            let mut covered = 0;
            for y in 1..191 {
                for x in 1..255 {
                    let i = y * 256 + x;
                    // Raster edge ownership differs: compare pixels whose four neighbours share the same primitive.
                    let t = expected.triangles[i];
                    if [i - 1, i + 1, i - 256, i + 256].iter().any(|j| expected.triangles[*j] != t) {
                        continue;
                    }
                    covered += usize::from(t.is_some());
                    if actual[i].iter().zip(expected.rgba[i]).any(|(a, b)| a.abs_diff(b) > 3) {
                        bad += 1;
                    }
                }
            }
            assert!(bad < 15, "step {step}: {bad} incorrect interior pixels; {covered} covered");
        }
        // MSAA resolve, resize and cached frame reuse must leave a valid GPU image.
        let empty = BTreeSet::new();
        let keys = vec![(1, 7, 0, 0); colors.len()];
        let nkeys = vec![(2, 1, 0); normals.len()];
        renderer
            .draw(Draw {
                masked: &BTreeSet::new(),
                scene: &scene,
                geometry,
                camera: Camera::default(),
                size: [300, 220],
                antialias: true,
                selected: &empty,
                hidden: &empty,
                generation: 8,
                colors: &colors,
                normals: &normals,
                keys: &keys,
                normal_keys: &nkeys,
            })
            .unwrap();
        assert_eq!(read(&renderer).len(), 300 * 220);
        assert_eq!(renderer.target.as_ref().unwrap()._depth.sample_count(), 1);
        let retained = renderer.target.as_ref().unwrap()._texture.clone();
        // The shared preview budget can resize a map without changing its document revision.
        let geometry = renderer.mesh.as_ref().unwrap().geometry.clone();
        for image in colors.iter_mut().flatten() {
            *image = Image { width: 1, height: 1, rgba: vec![[0.0, 0.8, 0.0, 1.0]] };
        }
        renderer
            .draw(Draw {
                masked: &BTreeSet::new(),
                scene: &scene,
                geometry: geometry.clone(),
                camera: Camera::default(),
                size: [300, 220],
                antialias: true,
                selected: &empty,
                hidden: &empty,
                generation: 9,
                colors: &colors,
                normals: &normals,
                keys: &keys,
                normal_keys: &nkeys,
            })
            .unwrap();
        assert!(renderer.materials.iter().flatten().all(|m| m.color.size == [1, 1]));
        assert!(read(&renderer).iter().any(|p| p[1] > p[0] + 10));
        renderer
            .draw(Draw {
                masked: &BTreeSet::new(),
                scene: &scene,
                geometry,
                camera: Camera::default(),
                size: [300, 220],
                antialias: false,
                selected: &empty,
                hidden: &empty,
                generation: 10,
                colors: &colors,
                normals: &normals,
                keys: &keys,
                normal_keys: &nkeys,
            })
            .unwrap();
        assert_eq!(retained, renderer.target.as_ref().unwrap()._texture, "navigation must reuse the existing render target");
        // A small motion frame and the full idle frame retain their attachments across gestures.
        let geometry = renderer.mesh.as_ref().unwrap().geometry.clone();
        for (size, antialias) in [([150, 110], false), ([300, 220], true), ([150, 110], false), ([300, 220], true)] {
            renderer
                .draw(Draw {
                    masked: &BTreeSet::new(),
                    scene: &scene,
                    geometry: geometry.clone(),
                    camera: Camera::default(),
                    size,
                    antialias,
                    selected: &empty,
                    hidden: &empty,
                    generation: 10,
                    colors: &colors,
                    normals: &normals,
                    keys: &keys,
                    normal_keys: &nkeys,
                })
                .unwrap();
            assert_eq!(read(&renderer).len(), size[0] as usize * size[1] as usize);
            if size == [300, 220] {
                assert_eq!(retained, renderer.target.as_ref().unwrap()._texture, "idle target must survive repeated motion gestures");
            }
        }
        let plain = read(&renderer);
        let selected = geometry.triangles.iter().map(|t| t.shell as usize).collect();
        renderer
            .draw(Draw {
                masked: &BTreeSet::new(),
                scene: &scene,
                geometry: geometry.clone(),
                camera: Camera::default(),
                size: [300, 220],
                antialias: true,
                selected: &selected,
                hidden: &empty,
                generation: 10,
                colors: &colors,
                normals: &normals,
                keys: &keys,
                normal_keys: &nkeys,
            })
            .unwrap();
        assert_ne!(plain, read(&renderer), "a UI highlight toggle must invalidate the GPU frame even without an engine generation change");
    }
    #[test]
    #[ignore = "explicit GPU/CPU benchmark; waits for completed submissions, excludes first upload"]
    fn benchmark_gpu_and_cpu_same_scene() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut renderer = renderer();
        let (scene, geometry) = scene();
        let empty = BTreeSet::new();
        let colors: Vec<_> = scene.materials.iter().map(|_| Some(Image { width: 2048, height: 2048, rgba: vec![[0.8, 0.8, 0.8, 1.0]; 2048 * 2048] })).collect();
        let normals: Vec<_> =
            scene.materials.iter().map(|_| Some(Image { width: 2048, height: 2048, rgba: vec![[0.5, 0.5, 1.0, 1.0]; 2048 * 2048] })).collect();
        let keys = vec![(1, 1, 0, 0); colors.len()];
        let nkeys = vec![(2, 1, 0); normals.len()];
        eprintln!("GPU adapter: {:?}", renderer.rs.adapter.get_info());
        for size in [[1024, 576], [1920, 1080], [3840, 2160]] {
            for aa in [false, true] {
                let mut gpu = Vec::new();
                let mut cpu = Vec::new();
                let mut timestamps = Vec::new();
                for step in 0..32 {
                    let camera = Camera { yaw: 0.65 + f64::from(step) * 0.01, shaded: true, ..Default::default() };
                    let start = std::time::Instant::now();
                    renderer
                        .draw(Draw {
                            masked: &BTreeSet::new(),
                            scene: &scene,
                            geometry: geometry.clone(),
                            camera,
                            size,
                            antialias: aa,
                            selected: &empty,
                            hidden: &empty,
                            generation: 1,
                            colors: &colors,
                            normals: &normals,
                            keys: &keys,
                            normal_keys: &nkeys,
                        })
                        .unwrap();
                    assert!(renderer.health.wait(&renderer.rs.device, None), "{:?}", renderer.health.fault());
                    if step > 7 {
                        gpu.push(start.elapsed().as_secs_f64() * 1000.0);
                        if let Some(timing) = renderer.timer.as_mut().and_then(|t| t.take()) {
                            timestamps.push(timing.ms);
                        }
                    }
                }
                for step in 0..3 {
                    if !aa {
                        let camera = Camera { yaw: 0.65 + f64::from(step) * 0.01, shaded: true, ..Default::default() };
                        let start = std::time::Instant::now();
                        std::hint::black_box(
                            preview::render_materials(
                                &scene,
                                camera,
                                &Materials { colors: &colors, normals: &normals, ..Default::default() },
                                &[],
                                size[0] as usize,
                                size[1] as usize,
                            )
                            .unwrap(),
                        );
                        cpu.push(start.elapsed().as_secs_f64() * 1000.0);
                    }
                }
                gpu.sort_by(f64::total_cmp);
                cpu.sort_by(f64::total_cmp);
                timestamps.sort_by(f64::total_cmp);
                eprintln!(
                    "{}x{} MSAA={}: GPU completed median {:.2} ms; CPU median {:?} ms",
                    size[0],
                    size[1],
                    if aa { renderer.aa_samples } else { 1 },
                    gpu[gpu.len() / 2],
                    cpu.get(cpu.len() / 2)
                );
                eprintln!("Hardware timestamp median {:?} ms", timestamps.get(timestamps.len() / 2));
            }
        }
    }
    #[test]
    #[ignore = "local-model CPU/GPU zoom diagnosis; supply PHOTOCRAFT_UV_PROFILE_MODEL"]
    fn profile_local_model_zoom() {
        let _gpu = photocraft_testkit::gpu_lock();
        use crate::{PhotocraftApp, Services};
        use egui_kittest::kittest::Queryable as _;
        let path = std::env::var("PHOTOCRAFT_UV_PROFILE_MODEL").expect("Supply a local model path");
        let (dialog, _) = crate::file_dialog::fake(vec![Some(crate::file_dialog::FileDialogAnswer::Paths(vec![path]))]);
        let capture = Arc::new(std::sync::Mutex::new(None));
        let slot = capture.clone();
        let mut harness =
            egui_kittest::Harness::builder().with_size(egui::vec2(2560.0, 1440.0)).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(move |cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services { file_dialog: Some(dialog), ..Default::default() });
                let rs = cc.wgpu_render_state.clone().unwrap();
                *slot.lock().unwrap() = Some(rs.clone());
                app.set_uv_wgpu(rs).unwrap();
                app.ui.uv_workbench.view_3d = true;
                app.ui.uv_workbench.fast_navigation = false;
                crate::uv_ui::reveal(&mut app).unwrap();
                app
            });
        let rs = capture.lock().unwrap().clone().unwrap();
        assert_ne!(rs.adapter.get_info().device_type, wgpu::DeviceType::Cpu);
        harness.input_mut().max_texture_side = Some(rs.device.limits().max_texture_dimension_2d as usize);
        harness.get_by_label(tl!("Import model…")).click();
        for _ in 0..4 {
            harness.step();
        }
        assert!(harness.state().session.uv.scene.is_some());
        assert!(harness.state().uv_gpu_active(), "native picker discarded the GPU renderer");
        eprintln!("Profile adapter {:?}", rs.adapter.get_info());
        eprintln!("Resident maps {:?}", harness.state().uv_gpu_texture_sizes());
        let median = |values: &mut Vec<f64>| {
            values.sort_by(f64::total_cmp);
            values.get(values.len() / 2).copied().unwrap_or(f64::NAN)
        };
        let panel = true;
        {
            for aa in [false, true] {
                for zoom in [1.0, 3.0, 8.0] {
                    let mut cpu = Vec::new();
                    let mut wait = Vec::new();
                    let mut gpu = Vec::new();
                    for frame in 0..48 {
                        let state = &mut harness.state_mut().ui.uv_workbench;
                        state.zoom = zoom;
                        state.antialias = aa;
                        state.yaw = 0.65 + f64::from(frame) * 0.003;
                        let start = std::time::Instant::now();
                        harness.step();
                        let submitted = start.elapsed().as_secs_f64() * 1000.0;
                        let wait_start = std::time::Instant::now();
                        rs.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
                        let waited = wait_start.elapsed().as_secs_f64() * 1000.0;
                        let renderer = harness.state_mut().uv_cache.gpu.as_mut().unwrap();
                        if frame >= 12 {
                            cpu.push(submitted);
                            wait.push(waited);
                            if let Some(timing) = renderer.timer.as_mut().and_then(|t| t.take()) {
                                gpu.push(timing.ms);
                            }
                        }
                    }
                    eprintln!(
                        "panel={panel} AA={aa} zoom={zoom}: CPU UI/submit {:.3}ms; completion wait {:.3}ms; GPU timestamp {:.3}ms ({} samples); target {:?}",
                        median(&mut cpu),
                        median(&mut wait),
                        median(&mut gpu),
                        gpu.len(),
                        harness.state().uv_cache.gpu.as_ref().unwrap().frame_info()
                    );
                }
            }
        }
        if let Ok(output) = std::env::var("PHOTOCRAFT_UV_PROFILE_SCREENSHOT") {
            harness.state_mut().ui.uv_workbench.zoom = 1.0;
            harness.step();
            harness.render().unwrap().save(output).unwrap();
        }
    }
    #[test]
    #[ignore = "explicit 100k-triangle navigation benchmark"]
    fn benchmark_large_grid() {
        let _gpu = photocraft_testkit::gpu_lock();
        use photocraft_uv::{Corner, Material, Triangle};
        let side = 224usize;
        let mut scene = Scene { materials: vec![Material::default()], ..Default::default() };
        for y in 0..=side {
            for x in 0..=side {
                scene.positions.push([x as f64 / side as f64, y as f64 / side as f64, 0.0]);
            }
        }
        for y in 0..side {
            for x in 0..side {
                let a = y * (side + 1) + x;
                let b = a + 1;
                let c = a + side + 1;
                let d = c + 1;
                for ids in [[a, b, d], [a, d, c]] {
                    scene.triangles.push(Triangle {
                        material: 0,
                        corners: ids.map(|vertex| Corner { vertex, uv: [scene.positions[vertex][0], scene.positions[vertex][1]], normal: None }),
                    });
                }
            }
        }
        let geometry = Arc::new(Geometry::new(&scene, &Topology::build(&scene).unwrap()).unwrap());
        let mut renderer = renderer();
        let empty = BTreeSet::new();
        let colors = vec![Some(Image { width: 2048, height: 2048, rgba: vec![[0.8, 0.6, 0.4, 1.0]; 2048 * 2048] })];
        let normals = vec![Some(Image { width: 2048, height: 2048, rgba: vec![[0.5, 0.5, 1.0, 1.0]; 2048 * 2048] })];
        for size in [[1920, 1080], [3840, 2160]] {
            let mut times = Vec::new();
            for step in 0..32 {
                let camera = Camera { yaw: 0.3 + f64::from(step) * 0.002, pitch: 0.3, shaded: true, ..Default::default() };
                let start = std::time::Instant::now();
                renderer
                    .draw(Draw {
                        masked: &BTreeSet::new(),
                        scene: &scene,
                        geometry: geometry.clone(),
                        camera,
                        size,
                        antialias: false,
                        selected: &empty,
                        hidden: &empty,
                        generation: 1,
                        colors: &colors,
                        normals: &normals,
                        keys: &[(1, 1, 0, 0)],
                        normal_keys: &[(2, 1, 0)],
                    })
                    .unwrap();
                assert!(renderer.health.wait(&renderer.rs.device, None));
                if step > 7 {
                    times.push(start.elapsed().as_secs_f64() * 1000.0);
                }
            }
            times.sort_by(f64::total_cmp);
            let start = std::time::Instant::now();
            std::hint::black_box(
                preview::render_materials(
                    &scene,
                    Camera { yaw: 0.3, pitch: 0.3, shaded: true, ..Default::default() },
                    &Materials { colors: &colors, normals: &normals, ..Default::default() },
                    &[],
                    size[0] as usize,
                    size[1] as usize,
                )
                .unwrap(),
            );
            eprintln!(
                "{} triangles {}x{}: GPU completed median {:.2} ms; CPU single frame {:.2} ms",
                scene.triangles.len(),
                size[0],
                size[1],
                times[times.len() / 2],
                start.elapsed().as_secs_f64() * 1000.0
            );
        }
    }
}
