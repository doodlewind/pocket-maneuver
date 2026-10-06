//! The renderer: the PS Vita's programs in WGSL (`shaders/`), its buffers,
//! and the passes of a frame (`vita/src/main.rs`, `vita/src/post.rs`).
//!
//! A frame is the scene into a target with several samples a pixel (the
//! sky, the world, the skinned models, the cloak and the wires, the gas),
//! a quarter-size chain (what is bright, blurred twice, smeared from the sun),
//! and one pass onto the screen that composes and grades, with the marks on
//! the world after it. The interface's picture goes over that
//! (`pocket_web_wgpu::overlay`).

use maneuver_pack::{self as pack, ModelHeader, Pack, SkinVertex, TexHeader};
use maneuver_sim::math::V3;
use pocket_web_wgpu::gpu::{Gpu, Screen, DEPTH};
use pocket_web_wgpu::overlay::Overlay;
use pocket_web_wgpu::wgpu::{self, util::DeviceExt, TextureFormat};

use crate::actors::{self, Actors, ColorVertex, Skinned, FAN, SKIN_FLOATS};
use crate::marks::{self, Marks};
use crate::mat::{self, Mat4};
use crate::scene::Scene;
use crate::world::Draw;

/// The scene's target and the quarter-size chain: colours as the display shows them.
const TARGET: TextureFormat = TextureFormat::Rgba8Unorm;
/// A record of a uniform buffer starts on a multiple of this (WebGPU's `minUniformBufferOffsetAlignment`).
const ALIGN: u64 = 256;
const WORLD_DRAWS: u64 = 2048;
const SKIN_STRIDE: u64 = (SKIN_FLOATS as u64 * 4).div_ceil(ALIGN) * ALIGN;
/// The player and every giant.
const SKIN_DRAWS: u64 = 40;
const POST_FLOATS: usize = 8 * 4 + 8;

/// The look of the frame (`Look` in `vita/src/post.rs`).
#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub bloom: bool,
    pub rays: bool,
    pub speed: bool,
    pub threshold: f32,
    pub bloom_gain: f32,
    pub rays_gain: f32,
    pub vignette: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub warm: f32,
    pub cool: f32,
}

impl Look {
    pub const DEFAULT: Look = Look { bloom: true, rays: true, speed: true, threshold: 0.86, bloom_gain: 0.7, rays_gain: 0.9, vignette: 0.24, contrast: 1.1, saturation: 1.16, warm: 0.05, cool: 0.06 };
}

/// What a frame draws.
pub struct View<'a> {
    pub vp: Mat4,
    pub eye: V3,
    /// The direction to the sun.
    pub sun: V3,
    pub world: &'a [Draw],
    pub skinned: &'a [Skinned],
    pub actors: &'a Actors,
    pub moving: actors::Frame,
    /// The player's cloak is drawn (the camera is not inside the character).
    pub character: bool,
    pub marks: &'a Marks,
    pub look: Look,
    /// 0 at rest and 1 at full speed: the blur toward the centre.
    pub fast: f32,
}

/// The targets of one screen size.
struct Targets {
    width: u32,
    height: u32,
    several: wgpu::TextureView,
    depth: wgpu::TextureView,
    scene: wgpu::TextureView,
    a: wgpu::TextureView,
    b: wgpu::TextureView,
    rays: wgpu::TextureView,
    /// The pictures each screen pass reads: bright, blur across, blur down, shafts, composite.
    groups: [wgpu::BindGroup; 5],
}

struct Model {
    vtx: wgpu::Buffer,
    idx: wgpu::Buffer,
    count: u32,
}

pub struct Renderer {
    samples: u32,
    world: wgpu::RenderPipeline,
    skin: wgpu::RenderPipeline,
    /// Coloured geometry: over everything with no depth (the sky), blended the same way (the sun), with depth from
    /// both sides (the cloak, the wires), and blended behind what is nearer (the gas, the shadow).
    sky: wgpu::RenderPipeline,
    sun: wgpu::RenderPipeline,
    cloth: wgpu::RenderPipeline,
    soft: wgpu::RenderPipeline,
    marks: wgpu::RenderPipeline,
    bright: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    shafts: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    post_layout: wgpu::BindGroupLayout,
    post_sampler: wgpu::Sampler,
    scene_buf: wgpu::Buffer,
    atlas_group: wgpu::BindGroup,
    font_group: wgpu::BindGroup,
    world_buf: wgpu::Buffer,
    world_group: wgpu::BindGroup,
    color_buf: wgpu::Buffer,
    color_group: wgpu::BindGroup,
    skin_buf: wgpu::Buffer,
    skin_group: wgpu::BindGroup,
    post_buf: wgpu::Buffer,
    vtx: wgpu::Buffer,
    idx: wgpu::Buffer,
    /// The player, then each giant build detailed and coarse (`Skinned::model`).
    models: Vec<Model>,
    sky_vb: wgpu::Buffer,
    sky_ib: wgpu::Buffer,
    sky_count: u32,
    sun_vb: wgpu::Buffer,
    cloak_ib: wgpu::Buffer,
    cloak_count: u32,
    quad_ib: wgpu::Buffer,
    fan_ib: wgpu::Buffer,
    moving_vb: wgpu::Buffer,
    marks_vb: wgpu::Buffer,
    targets: Option<Targets>,
    fog: [f32; 4],
    /// Bytes of geometry and texels handed to the GPU, for the status.
    pub bytes: u64,
    scratch: Vec<u8>,
}

fn bytes<T: Copy>(v: &[T]) -> &[u8] {
    pack::slice_bytes(v)
}

fn target(gpu: &Gpu, label: &str, width: u32, height: u32, format: TextureFormat, samples: u32, sampled: bool) -> wgpu::TextureView {
    gpu.device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | if sampled { wgpu::TextureUsages::TEXTURE_BINDING } else { wgpu::TextureUsages::empty() },
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

/// A pass over a whole target with no depth.
fn screen_pass<'a>(encoder: &'a mut wgpu::CommandEncoder, label: &str, view: &'a wgpu::TextureView) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    })
}

impl Renderer {
    /// The programs for a screen of `format`, and the pack's geometry and pictures on the GPU.
    pub fn new(gpu: &Gpu, format: TextureFormat, samples: u32, p: &Pack, scene: &Scene, actors: &Actors, marks: &Marks) -> Result<Renderer, String> {
        let device = &gpu.device;
        let mut held = 0u64;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("scene"), source: wgpu::ShaderSource::Wgsl(include_str!("shaders/scene.wgsl").into()) });
        let post = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("post"), source: wgpu::ShaderSource::Wgsl(include_str!("shaders/post.wgsl").into()) });

        // ---- layouts
        let uniform = |dynamic: bool| wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: dynamic, min_binding_size: None };
        let texture = wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
        let sampler = wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering);
        let entry = |binding: u32, visibility: wgpu::ShaderStages, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None };
        let both = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("scene"), entries: &[entry(0, both, uniform(false)), entry(1, wgpu::ShaderStages::FRAGMENT, texture), entry(2, wgpu::ShaderStages::FRAGMENT, sampler)] });
        // (a program's draw record has a binding of its own: scene.wgsl)
        let draw_layouts = [0, 1, 2].map(|binding| device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("draw"), entries: &[entry(binding, wgpu::ShaderStages::VERTEX, uniform(true))] }));
        let post_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post"),
            entries: &[entry(0, both, uniform(true)), entry(1, wgpu::ShaderStages::FRAGMENT, sampler), entry(2, wgpu::ShaderStages::FRAGMENT, texture), entry(3, wgpu::ShaderStages::FRAGMENT, texture), entry(4, wgpu::ShaderStages::FRAGMENT, texture)],
        });
        let [world_pipeline, color_pipeline, skin_pipeline] = [0, 1, 2].map(|i| device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("scene"), bind_group_layouts: &[&scene_layout, &draw_layouts[i]], push_constant_ranges: &[] }));
        let marks_pipeline = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("marks"), bind_group_layouts: &[&scene_layout], push_constant_ranges: &[] });
        let post_pipeline = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("post"), bind_group_layouts: &[&post_layout], push_constant_ranges: &[] });

        // ---- programs
        use wgpu::VertexFormat as F;
        let attr = |location: u32, offset: u64, format: F| wgpu::VertexAttribute { shader_location: location, offset, format };
        let world_attrs = [attr(0, 0, F::Uint16x4), attr(1, 8, F::Sint16x2), attr(2, 12, F::Unorm8x4)];
        let color_attrs = [attr(0, 0, F::Float32x3), attr(1, 12, F::Unorm8x4)];
        let skin_attrs = [attr(0, 0, F::Float32x3), attr(1, 12, F::Sint8x4), attr(2, 16, F::Unorm8x4), attr(3, 20, F::Uint8x4)];
        let mark_attrs = [attr(0, 0, F::Float32x2), attr(1, 8, F::Float32x2), attr(2, 16, F::Unorm8x4)];
        let buffer = |stride: u64, attributes: &'static [wgpu::VertexAttribute]| wgpu::VertexBufferLayout { array_stride: stride, step_mode: wgpu::VertexStepMode::Vertex, attributes };
        // (the layouts live as long as the closures that build the pipelines)
        let world_attrs: &'static [wgpu::VertexAttribute] = Box::leak(Box::new(world_attrs));
        let color_attrs: &'static [wgpu::VertexAttribute] = Box::leak(Box::new(color_attrs));
        let skin_attrs: &'static [wgpu::VertexAttribute] = Box::leak(Box::new(skin_attrs));
        let mark_attrs: &'static [wgpu::VertexAttribute] = Box::leak(Box::new(mark_attrs));

        // Depth: `Some((compare, write))`, or none for a pass without a depth buffer.
        #[allow(clippy::too_many_arguments)]
        let pipeline = |label: &str, layout: &wgpu::PipelineLayout, module: &wgpu::ShaderModule, vertex: &str, fragment: &str, buffers: &[wgpu::VertexBufferLayout], to: TextureFormat, blend: bool, cull: bool, depth: Option<(wgpu::CompareFunction, bool)>, count: u32| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState { module, entry_point: Some(vertex), compilation_options: Default::default(), buffers },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: to, blend: blend.then_some(wgpu::BlendState { color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add }, alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add } }), write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: wgpu::PrimitiveState { front_face: wgpu::FrontFace::Ccw, cull_mode: cull.then_some(wgpu::Face::Back), ..Default::default() },
                depth_stencil: depth.map(|(compare, write)| wgpu::DepthStencilState { format: DEPTH, depth_write_enabled: write, depth_compare: compare, stencil: Default::default(), bias: Default::default() }),
                multisample: wgpu::MultisampleState { count, ..Default::default() },
                multiview: None,
                cache: None,
            })
        };
        use wgpu::CompareFunction::{Always, LessEqual};
        let world = pipeline("world", &world_pipeline, &shader, "world_vertex", "world_fragment", &[buffer(16, world_attrs)], TARGET, false, true, Some((LessEqual, true)), samples);
        let skin = pipeline("skin", &skin_pipeline, &shader, "skin_vertex", "color_fragment", &[buffer(24, skin_attrs)], TARGET, false, true, Some((LessEqual, true)), samples);
        let color = |label: &str, blend: bool, depth: (wgpu::CompareFunction, bool)| pipeline(label, &color_pipeline, &shader, "color_vertex", "color_fragment", &[buffer(16, color_attrs)], TARGET, blend, false, Some(depth), samples);
        let sky = color("sky", false, (Always, false));
        let sun = color("sun", true, (Always, false));
        let cloth = color("cloth", false, (LessEqual, true));
        let soft = color("soft", true, (LessEqual, false));
        let marks_program = pipeline("marks", &marks_pipeline, &shader, "mark_vertex", "mark_fragment", &[buffer(20, mark_attrs)], format, true, false, None, 1);
        let screen = |label: &str, fragment: &str, to: TextureFormat| pipeline(label, &post_pipeline, &post, "corner", fragment, &[], to, false, false, None, 1);
        let (bright, blur, shafts, composite) = (screen("bright", "bright", TARGET), screen("blur", "blur", TARGET), screen("shafts", "shafts", TARGET), screen("composite", "composite", format));

        // ---- the atlas: BC1 in the pack, texels here
        let tex = p.section(pack::TEX0)?;
        let head: TexHeader = pack::read(tex, 0).ok_or("atlas header")?;
        if head.format != pack::tex_format::BC1 {
            return Err(format!("the pack's atlas has format {} (this renderer reads the PS Vita's pack, vita60)", head.format));
        }
        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("atlas"),
            size: wgpu::Extent3d { width: head.width, height: head.height, depth_or_array_layers: 1 },
            mip_level_count: head.mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut at = core::mem::size_of::<TexHeader>();
        for level in 0..head.mips {
            let (w, h) = ((head.width >> level).max(1), (head.height >> level).max(1));
            let size = pack::tex_format::level_bytes(head.format, w, h);
            let blocks = tex.get(at..at + size).ok_or("the atlas is cut short")?;
            at += size;
            let texels = crate::pack::bc1(blocks, w as usize, h as usize);
            held += texels.len() as u64;
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &atlas, mip_level: level, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                &texels,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
        }
        // Strips repeat along U and never along V.
        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let linear = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("linear"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });

        // ---- the font of the marks: coverage, one byte a texel
        let font = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("font"),
            size: wgpu::Extent3d { width: marks.width, height: marks.height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &font, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &marks.cover,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(marks.width), rows_per_image: Some(marks.height) },
            wgpu::Extent3d { width: marks.width, height: marks.height, depth_or_array_layers: 1 },
        );
        held += marks.cover.len() as u64;

        // ---- uniforms
        let uniforms = |label: &str, size: u64| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let scene_buf = uniforms("scene", 32);
        let scene_group = |label: &str, view: &wgpu::TextureView, sampler: &wgpu::Sampler| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &scene_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: scene_buf.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(view) }, wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) }],
            })
        };
        let atlas_group = scene_group("atlas", &atlas.create_view(&Default::default()), &atlas_sampler);
        let font_group = scene_group("font", &font.create_view(&Default::default()), &linear);
        let draws = |label: &str, binding: u32, buffer: &wgpu::Buffer, size: u64| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &draw_layouts[binding as usize],
                entries: &[wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer, offset: 0, size: wgpu::BufferSize::new(size) }) }],
            })
        };
        let world_buf = uniforms("world draws", WORLD_DRAWS * ALIGN);
        let world_group = draws("world draws", 0, &world_buf, 64);
        let color_buf = uniforms("colour draws", 2 * ALIGN);
        let color_group = draws("colour draws", 1, &color_buf, 80);
        let skin_buf = uniforms("skin draws", SKIN_DRAWS * SKIN_STRIDE);
        let skin_group = draws("skin draws", 2, &skin_buf, SKIN_FLOATS as u64 * 4);
        let post_buf = uniforms("post", 5 * ALIGN);

        // ---- geometry
        let init = |label: &str, contents: &[u8], usage: wgpu::BufferUsages| device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage });
        let (vtx_src, idx_src) = (p.section(pack::VTX0)?, p.section(pack::IDX0)?);
        let vtx = init("world vertices", vtx_src, wgpu::BufferUsages::VERTEX);
        let idx = init("world indices", idx_src, wgpu::BufferUsages::INDEX);
        held += (vtx_src.len() + idx_src.len()) as u64;

        // Skinned models, as they are in the pack.
        let modl = p.section(pack::MODL)?;
        let count: u32 = pack::read(modl, 0).ok_or("model count")?;
        let mut models: Vec<Option<Model>> = (0..7).map(|_| None).collect();
        let mut at = 4;
        for _ in 0..count {
            let h: ModelHeader = pack::read(modl, at).ok_or("model header")?;
            at += core::mem::size_of::<ModelHeader>();
            let vbytes = h.vtx_count as usize * core::mem::size_of::<SkinVertex>();
            let ibytes = h.idx_count as usize * 2;
            if at + vbytes + ibytes > modl.len() {
                return Err("model section is truncated".into());
            }
            let slot = match h.id {
                0 => Some(0),
                10..=12 => Some(1 + (h.id - 10) as usize * 2),
                20..=22 => Some(2 + (h.id - 20) as usize * 2),
                _ => None,
            };
            if let Some(slot) = slot {
                models[slot] = Some(Model { vtx: init("model vertices", &modl[at..at + vbytes], wgpu::BufferUsages::VERTEX), idx: init("model indices", &modl[at + vbytes..at + vbytes + ibytes], wgpu::BufferUsages::INDEX), count: h.idx_count });
                held += (vbytes + ibytes) as u64;
            }
            at = (at + vbytes + ibytes + 3) & !3;
        }
        let models: Vec<Model> = models.into_iter().collect::<Option<_>>().ok_or("the pack lacks a skinned model")?;

        let sky_vb = init("sky", bytes(&actors.sky), wgpu::BufferUsages::VERTEX);
        let sky_ib = init("sky indices", bytes(&actors.sky_idx), wgpu::BufferUsages::INDEX);
        let sun_vb = init("sun", bytes(&actors.sun), wgpu::BufferUsages::VERTEX);
        let cloak_ib = init("cloak indices", bytes(&actors.cloak_idx), wgpu::BufferUsages::INDEX);
        let quad_ib = init("quad indices", bytes(&actors.quad_idx), wgpu::BufferUsages::INDEX);
        let fan_ib = init("fan indices", bytes(&actors.fan_idx), wgpu::BufferUsages::INDEX);
        let dynamic = |label: &str, size: usize| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let moving_vb = dynamic("moving", Actors::moving_capacity() * core::mem::size_of::<ColorVertex>());
        let marks_vb = dynamic("marks", marks::MAX_QUADS * 4 * core::mem::size_of::<marks::Vertex>());

        let fog = scene.fog_srgb();
        Ok(Renderer {
            samples,
            world,
            skin,
            sky,
            sun,
            cloth,
            soft,
            marks: marks_program,
            bright,
            blur,
            shafts,
            composite,
            post_layout,
            post_sampler: linear,
            scene_buf,
            atlas_group,
            font_group,
            world_buf,
            world_group,
            color_buf,
            color_group,
            skin_buf,
            skin_group,
            post_buf,
            vtx,
            idx,
            models,
            sky_vb,
            sky_ib,
            sky_count: actors.sky_idx.len() as u32,
            sun_vb,
            cloak_ib,
            cloak_count: actors.cloak_idx.len() as u32,
            quad_ib,
            fan_ib,
            moving_vb,
            marks_vb,
            targets: None,
            fog: [fog[0], fog[1], fog[2], scene.fog_density],
            bytes: held,
            scratch: Vec::new(),
        })
    }

    /// The targets for a screen of this size: the scene's, with its depth buffer, and the quarter-size chain.
    fn targets(&mut self, gpu: &Gpu, width: u32, height: u32) {
        if self.targets.as_ref().is_some_and(|t| (t.width, t.height) == (width, height)) {
            return;
        }
        let (qw, qh) = ((width / 4).max(1), (height / 4).max(1));
        let several = target(gpu, "scene samples", width, height, TARGET, self.samples, false);
        let depth = target(gpu, "scene depth", width, height, DEPTH, self.samples, false);
        let scene = target(gpu, "scene", width, height, TARGET, 1, true);
        let [a, b, rays] = ["bright a", "bright b", "shafts"].map(|label| target(gpu, label, qw, qh, TARGET, 1, true));
        let group = |label: &str, textures: [&wgpu::TextureView; 3]| {
            gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &self.post_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &self.post_buf, offset: 0, size: wgpu::BufferSize::new(POST_FLOATS as u64 * 4) }) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.post_sampler) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(textures[0]) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(textures[1]) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(textures[2]) },
                ],
            })
        };
        // (a pass binds the scene where it reads nothing: no pass draws into the scene's picture)
        let groups = [group("bright", [&scene, &scene, &scene]), group("blur across", [&a, &scene, &scene]), group("blur down", [&b, &scene, &scene]), group("shafts", [&a, &scene, &scene]), group("composite", [&scene, &a, &rays])];
        self.targets = Some(Targets { width, height, several, depth, scene, a, b, rays, groups });
    }

    /// Draws a frame onto `screen`, with the interface's picture over it. Returns the draws and the triangles
    /// of the moving geometry.
    pub fn frame(&mut self, gpu: &Gpu, screen: &Screen, overlay: &Overlay, view: &View) -> Result<(u32, u32), String> {
        self.targets(gpu, screen.width, screen.height);
        let frame = screen.frame(gpu)?;
        let (w, h) = (screen.width as f32, screen.height as f32);
        let queue = &gpu.queue;
        let (mut draws, mut tris) = (0u32, 0u32);

        // ---- this frame's records and vertices
        queue.write_buffer(&self.scene_buf, 0, bytes(&[self.fog[0], self.fog[1], self.fog[2], self.fog[3], w, h, 0.0, 0.0]));
        let world = &view.world[..view.world.len().min(WORLD_DRAWS as usize)];
        self.scratch.clear();
        self.scratch.resize(world.len() * ALIGN as usize, 0);
        for (i, d) in world.iter().enumerate() {
            self.scratch[i * ALIGN as usize..][..64].copy_from_slice(bytes(&d.mvp));
        }
        queue.write_buffer(&self.world_buf, 0, &self.scratch);
        // (the sky is centred on the eye and has no haze; the rest is in the world)
        let mut colors = [0.0f32; ALIGN as usize / 4 * 2];
        colors[..16].copy_from_slice(&mat::translated(&view.vp, view.eye));
        colors[ALIGN as usize / 4..][..16].copy_from_slice(&view.vp);
        colors[ALIGN as usize / 4 + 16] = self.fog[3];
        queue.write_buffer(&self.color_buf, 0, bytes(&colors));
        let skinned = &view.skinned[..view.skinned.len().min(SKIN_DRAWS as usize)];
        self.scratch.clear();
        self.scratch.resize(skinned.len() * SKIN_STRIDE as usize, 0);
        for (i, s) in skinned.iter().enumerate() {
            self.scratch[i * SKIN_STRIDE as usize..][..SKIN_FLOATS * 4].copy_from_slice(bytes(&s.record));
        }
        queue.write_buffer(&self.skin_buf, 0, &self.scratch);
        queue.write_buffer(&self.moving_vb, 0, bytes(&view.actors.moving));
        queue.write_buffer(&self.marks_vb, 0, bytes(&view.marks.verts));

        // ---- the screen passes' taps (`Post` in vita/src/post.rs)
        let look = &view.look;
        let t = self.targets.as_ref().ok_or("no targets")?;
        let (qw, qh) = ((t.width / 4).max(1) as f32, (t.height / 4).max(1) as f32);
        let taps = || {
            let mut r = [0.0f32; POST_FLOATS];
            for k in 0..8 {
                r[k * 4] = 1.0;
                r[k * 4 + 1] = 1.0;
            }
            r
        };
        let mut post = [[0.0f32; ALIGN as usize / 4]; 5];
        let mut put = |slot: usize, r: [f32; POST_FLOATS]| post[slot][..POST_FLOATS].copy_from_slice(&r);
        // Bright: four taps one source texel off the centre, each a 2 x 2.
        let mut r = taps();
        for (k, (sx, sy)) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)].into_iter().enumerate() {
            r[k * 4 + 2] = sx / w;
            r[k * 4 + 3] = sy / h;
        }
        r[32..36].copy_from_slice(&[look.threshold, 1.0 / (1.0 - look.threshold).max(0.05), 0.0, 0.0]);
        put(0, r);
        // Blur: across, then down.
        for (slot, (dx, dy)) in [(1, (1.0 / qw, 0.0)), (2, (0.0, 1.0 / qh))] {
            let mut r = taps();
            for (k, o) in [0.0f32, 1.3846, -1.3846, 3.2308, -3.2308].into_iter().enumerate() {
                r[k * 4 + 2] = o * dx;
                r[k * 4 + 3] = o * dy;
            }
            put(slot, r);
        }
        // Shafts, when the sun is in front of the eye and near the frame.
        let mut lit = false;
        if look.bloom && look.rays {
            let (vp, p) = (&view.vp, view.eye + view.sun * 1000.0);
            let pw = vp[12] * p.x + vp[13] * p.y + vp[14] * p.z + vp[15];
            if pw > 1.0 {
                let u = (vp[0] * p.x + vp[1] * p.y + vp[2] * p.z + vp[3]) / pw * 0.5 + 0.5;
                let v = 0.5 - (vp[4] * p.x + vp[5] * p.y + vp[6] * p.z + vp[7]) / pw * 0.5;
                if (-0.4..1.4).contains(&u) && (-0.4..1.4).contains(&v) {
                    // Each tap is the pixel moved a step toward the sun: uv' = sun + (uv - sun) × s.
                    let mut r = taps();
                    for k in 0..8 {
                        let s = 1.0 - k as f32 * 0.062;
                        r[k * 4..k * 4 + 4].copy_from_slice(&[s, s, u * (1.0 - s), v * (1.0 - s)]);
                    }
                    put(3, r);
                    lit = true;
                }
            }
        }
        // Composite: tap 1 is the screen as -1..1; taps 2..4 pull the scene toward the centre at speed.
        let mut r = taps();
        r[4..8].copy_from_slice(&[2.0, 2.0, -1.0, -1.0]);
        let smear = look.bloom && look.speed && view.fast > 0.02;
        for (k, s) in [(2usize, 0.985f32), (3, 0.97), (4, 0.955)] {
            let s = 1.0 - (1.0 - s) * if smear { view.fast } else { 0.0 };
            r[k * 4..k * 4 + 4].copy_from_slice(&[s, s, 0.5 * (1.0 - s), 0.5 * (1.0 - s)]);
        }
        r[32..36].copy_from_slice(&[if look.bloom { look.bloom_gain } else { 0.0 }, if lit { look.rays_gain } else { 0.0 }, look.vignette, if smear { view.fast * 1.6 } else { 0.0 }]);
        r[36..40].copy_from_slice(&[look.contrast, look.saturation, look.warm, look.cool]);
        put(4, r);
        queue.write_buffer(&self.post_buf, 0, bytes(&post));

        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        // ---- the scene
        {
            let several = self.samples > 1;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: if several { &t.several } else { &t.scene },
                    resolve_target: several.then_some(&t.scene),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: self.fog[0] as f64, g: self.fog[1] as f64, b: self.fog[2] as f64, a: 1.0 }), store: if several { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store } },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &t.depth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }), stencil_ops: None }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_bind_group(0, &self.atlas_group, &[]);
            // The sky, first in the frame: no depth, no haze. Then the sun's halo and its disc.
            pass.set_bind_group(1, &self.color_group, &[0]);
            pass.set_pipeline(&self.sky);
            pass.set_vertex_buffer(0, self.sky_vb.slice(..));
            pass.set_index_buffer(self.sky_ib.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..self.sky_count, 0, 0..1);
            pass.set_pipeline(&self.sun);
            pass.set_vertex_buffer(0, self.sun_vb.slice(..));
            pass.set_index_buffer(self.fan_ib.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..2 * (FAN * 3) as u32, 0, 0..1);
            // The world: one program, one picture, one matrix a draw.
            if !world.is_empty() {
                pass.set_pipeline(&self.world);
                pass.set_vertex_buffer(0, self.vtx.slice(..));
                pass.set_index_buffer(self.idx.slice(..), wgpu::IndexFormat::Uint16);
                for (i, d) in world.iter().enumerate() {
                    pass.set_bind_group(1, &self.world_group, &[(i as u64 * ALIGN) as u32]);
                    pass.draw_indexed(d.idx_first..d.idx_first + d.idx_count, d.vtx_first as i32, 0..1);
                }
            }
            // The player and the giants.
            if !skinned.is_empty() {
                pass.set_pipeline(&self.skin);
                for (i, s) in skinned.iter().enumerate() {
                    let model = &self.models[s.model];
                    pass.set_bind_group(1, &self.skin_group, &[(i as u64 * SKIN_STRIDE) as u32]);
                    pass.set_vertex_buffer(0, model.vtx.slice(..));
                    pass.set_index_buffer(model.idx.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..model.count, 0, 0..1);
                    draws += 1;
                    tris += model.count / 3;
                }
            }
            // The cloak and the wires: lit on the processor, drawn from both sides.
            pass.set_bind_group(1, &self.color_group, &[ALIGN as u32]);
            pass.set_vertex_buffer(0, self.moving_vb.slice(..));
            if view.character || view.moving.rope_quads > 0 {
                pass.set_pipeline(&self.cloth);
                if view.character {
                    pass.set_index_buffer(self.cloak_ib.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..self.cloak_count, 0, 0..1);
                    draws += 1;
                    tris += self.cloak_count / 3;
                }
                if view.moving.rope_quads > 0 {
                    pass.set_index_buffer(self.quad_ib.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..view.moving.rope_quads * 6, view.moving.rope_first as i32, 0..1);
                    draws += 1;
                    tris += view.moving.rope_quads * 2;
                }
            }
            // The ground shadow, the gas and the steam, blended over the scene.
            if view.moving.discs > 0 {
                pass.set_pipeline(&self.soft);
                pass.set_index_buffer(self.fan_ib.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..view.moving.discs * (FAN * 3) as u32, view.moving.disc_first as i32, 0..1);
                draws += 1;
                tris += view.moving.discs * FAN as u32;
            }
        }
        // ---- what is bright, blurred, and smeared from the sun
        if look.bloom {
            let mut chain = vec![(&self.bright, 0usize, &t.a), (&self.blur, 1, &t.b), (&self.blur, 2, &t.a)];
            if lit {
                chain.push((&self.shafts, 3, &t.rays));
            }
            for (program, slot, to) in chain {
                let mut pass = screen_pass(&mut encoder, "bright", to);
                pass.set_pipeline(program);
                pass.set_bind_group(0, &t.groups[slot], &[(slot as u64 * ALIGN) as u32]);
                pass.draw(0..3, 0..1);
            }
        }
        // ---- the frame as shown, and the marks on the world over it
        {
            let mut pass = screen_pass(&mut encoder, "composite", frame.shown());
            pass.set_pipeline(&self.composite);
            pass.set_bind_group(0, &t.groups[4], &[(4 * ALIGN) as u32]);
            pass.draw(0..3, 0..1);
            let quads = (view.marks.verts.len() / 4) as u32;
            if quads > 0 {
                pass.set_pipeline(&self.marks);
                pass.set_bind_group(0, &self.font_group, &[]);
                pass.set_vertex_buffer(0, self.marks_vb.slice(..));
                pass.set_index_buffer(self.quad_ib.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..quads * 6, 0, 0..1);
            }
        }
        overlay.draw(&mut encoder, &frame);
        gpu.queue.submit([encoder.finish()]);
        frame.present();
        Ok((draws, tris))
    }
}
