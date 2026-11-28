//! The wgpu side of the terrain renderer: one quad buffer shared by all
//! render batches (ranges from [`RangeArena`]), paths B and C, block texture
//! array, light bricks, per-frame culling into a reused draw list.
//!
//! Each resident batch keeps one range per render bucket (ADR-0016,
//! [`mesh::RenderBucket`]): the packed quads with their six direction
//! sub-ranges, and the block-model instances. Model instances are only
//! accounted for (their own arena, [`FrameStats::models_resident`]) until the
//! block-model pipeline exists; no mesher produces them yet.

use std::collections::HashMap;
use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use glam::{IVec3, Mat4, Vec3};
use light::{Nibbles, SectionLight};
use mesh::batch::PackedBatch;

use super::arena::RangeArena;
use super::culling::{Frustum, visible_face_mask};
use super::light_atlas::{ATLAS_BRICKS, INDIRECTION, LightAtlas, SectionLightData};
use crate::RenderPath;

/// Terrain drawing variant: the two T0-06 paths plus the indexed form of B
/// (open question 13 compares all three on real terrain).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainPath {
    /// Path B: six pulled vertices per quad.
    Pulling,
    /// Path B indexed: four pulled vertices per quad and a shared index buffer.
    PullingIndexed,
    /// Path C: one instance per quad, 4-vertex strip.
    Instanced,
}

impl From<RenderPath> for TerrainPath {
    fn from(p: RenderPath) -> Self {
        match p {
            RenderPath::VertexPulling => Self::Pulling,
            RenderPath::Instanced => Self::Instanced,
        }
    }
}

impl TerrainPath {
    /// The default for an adapter (open question 13, decided 2026-11-22 on
    /// the Vega 10: indexed B is the fastest): indexed B where vertex storage
    /// and `base_vertex` exist, plain B without `base_vertex`, C without
    /// vertex storage.
    #[must_use]
    pub fn default_for(flags: wgpu::DownlevelFlags) -> Self {
        match crate::select_path(flags) {
            RenderPath::VertexPulling if flags.contains(wgpu::DownlevelFlags::BASE_VERTEX) => {
                Self::PullingIndexed
            }
            RenderPath::VertexPulling => Self::Pulling,
            RenderPath::Instanced => Self::Instanced,
        }
    }
}

/// Quads covered by the shared index buffer; longer draws are split.
const INDEX_QUADS: u32 = 1 << 16;

/// Render batch slots (the batch origin uniform array has this many entries).
pub const MAX_BATCHES: u32 = 4096;
/// Depth buffer format the pipelines expect.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Edge of a render batch in blocks.
pub const BATCH_BLOCKS: i32 = 64;
/// Bytes per quad.
const QUAD_BYTES: u64 = 8;
/// Indirection value of a section nobody uploaded: open sky.
const DEFAULT_LIGHT: u32 = 15 << 4;
/// The shader-pack uniform: 8 options, the time, then 24 more options.
const PACK_PARAMS_BYTES: u64 = 48 + 96;
// One dirty bit per z slice of the light indirection (`index_dirty: u64`).
const _: () = assert!(INDIRECTION[2] <= 64);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
    position: [f32; 4],
    fog: [f32; 4],
    sky: [f32; 4],
}

/// Per-frame camera input.
#[derive(Clone, Copy, Debug)]
pub struct CameraParams {
    /// Projection × view.
    pub view_proj: Mat4,
    /// Eye position in blocks.
    pub position: Vec3,
    /// Sky light multiplier, `0..=1`.
    pub daylight: f32,
    /// Fog start and end distance in blocks.
    pub fog: (f32, f32),
    /// Sky (fog) color, linear RGB.
    pub sky: [f32; 3],
}

/// Block textures as stored by the asset pipeline: square RGBA8 layers with
/// a full mip chain, `levels[mip]` holding every layer.
pub struct TerrainTextures<'a> {
    /// Edge of mip 0.
    pub size: u32,
    /// Number of layers.
    pub layers: u32,
    /// Mip levels, layer after layer.
    pub levels: &'a [Vec<u8>],
}

/// Renderer errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainError {
    /// No free range of the quad buffer is large enough.
    QuadBufferFull,
    /// All batch slots are taken.
    BatchSlotsFull,
    /// The light atlas has no free brick.
    LightAtlasFull,
    /// No free range of the block-model instance arena is large enough.
    ModelBufferFull,
}

impl std::fmt::Display for TerrainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::QuadBufferFull => "terrain quad buffer is full",
            Self::BatchSlotsFull => "all render batch slots are in use",
            Self::LightAtlasFull => "light atlas is full",
            Self::ModelBufferFull => "block-model instance arena is full",
        })
    }
}

impl std::error::Error for TerrainError {}

/// What the last [`TerrainRenderer::prepare`] selected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    /// Batches resident on the GPU.
    pub batches_resident: u32,
    /// Batches that passed culling.
    pub batches_drawn: u32,
    /// Draw calls.
    pub draws: u32,
    /// Quads resident.
    pub quads_resident: u32,
    /// Quads submitted.
    pub quads_drawn: u32,
    /// Block-model instances resident (not drawn yet, see the module docs).
    pub models_resident: u32,
}

/// Capacity of the block-model instance arena (bookkeeping only for now).
const MODEL_CAPACITY: u32 = 1 << 20;

/// Per-bucket ranges of one resident render batch.
struct GpuBatch {
    slot: u32,
    /// [`mesh::RenderBucket::PackedVoxelQuads`]: range in the quad buffer.
    quads: Range<u32>,
    /// Absolute quad ranges per direction inside `quads`.
    dirs: [Range<u32>; 6],
    /// [`mesh::RenderBucket::ModelInstances`]: range in the model arena.
    models: Range<u32>,
}

/// Terrain renderer for one device.
pub struct TerrainRenderer {
    path: TerrainPath,
    pipeline: wgpu::RenderPipeline,
    pipeline_layout: wgpu::PipelineLayout,
    color_format: wgpu::TextureFormat,
    layout0: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    atlas_view: wgpu::TextureView,
    index_view: wgpu::TextureView,
    pack_buf: wgpu::Buffer,
    bind0: wgpu::BindGroup,
    bind1: Option<wgpu::BindGroup>,
    index_buf: Option<wgpu::Buffer>,
    camera_buf: wgpu::Buffer,
    batch_buf: wgpu::Buffer,
    quad_buf: wgpu::Buffer,
    light_atlas_tex: wgpu::Texture,
    light_index_tex: wgpu::Texture,
    arena: RangeArena,
    model_arena: RangeArena,
    batches: HashMap<IVec3, GpuBatch>,
    free_slots: Vec<u32>,
    atlas: LightAtlas,
    staging: Vec<u64>,
    texels: Box<[u8; 4096]>,
    /// CPU copy of the light indirection texture: entries change here and
    /// reach the GPU once per frame, one write per changed z slice (T1-25:
    /// a row of columns leaving the view used to cost ~900 tiny writes).
    index_mirror: Vec<u32>,
    /// Bit per z slice of `index_mirror` changed since the last upload.
    index_dirty: u64,
    /// Culling boxes grow by this (vertex displacement of a pack).
    cull_margin: f32,
    /// (slot, quad range) per draw, rebuilt by `prepare`.
    draws: Vec<(u32, Range<u32>)>,
    stats: FrameStats,
}

fn shader_source(path: TerrainPath) -> String {
    super::hooks::compose(path, None)
}

/// Full WGSL of a path (for validation in tests and tools).
#[must_use]
pub fn terrain_shader(path: TerrainPath) -> String {
    shader_source(path)
}

fn create_block_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    textures: &TerrainTextures<'_>,
    layer_table: &[u32],
) -> (wgpu::TextureView, wgpu::TextureView) {
    let mips = textures.levels.len() as u32;
    let block_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("block textures"),
        size: wgpu::Extent3d {
            width: textures.size,
            height: textures.size,
            depth_or_array_layers: textures.layers.max(1),
        },
        mip_level_count: mips.max(1),
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (mip, data) in textures.levels.iter().enumerate() {
        let side = (textures.size >> mip).max(1);
        if textures.layers == 0 {
            break;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &block_tex,
                mip_level: mip as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(side * 4),
                rows_per_image: Some(side),
            },
            wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: textures.layers,
            },
        );
    }
    // State × direction → layer.
    let states = (layer_table.len() / 6) as u32;
    let layers_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("block layers"),
        size: wgpu::Extent3d {
            width: 6,
            height: states,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Uint,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        layers_tex.as_image_copy(),
        bytemuck::cast_slice(layer_table),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(6 * 4),
            rows_per_image: Some(states),
        },
        wgpu::Extent3d {
            width: 6,
            height: states,
            depth_or_array_layers: 1,
        },
    );
    let view = |t: &wgpu::Texture, dim| {
        t.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(dim),
            ..Default::default()
        })
    };
    (
        view(&block_tex, wgpu::TextureViewDimension::D2Array),
        view(&layers_tex, wgpu::TextureViewDimension::D2),
    )
}

/// Bind group 0: buffers (camera, batch origins, pack params), the sampler,
/// and views (block textures, layer table, light atlas, light indirection).
fn make_bind0(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffers: [&wgpu::Buffer; 3],
    sampler: &wgpu::Sampler,
    views: [&wgpu::TextureView; 4],
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("terrain globals"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffers[0].as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buffers[1].as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(views[0]),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(views[1]),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(views[2]),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(views[3]),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: buffers[2].as_entire_binding(),
            },
        ],
    })
}

fn create_pipeline(
    device: &wgpu::Device,
    pipeline_layout: &wgpu::PipelineLayout,
    path: TerrainPath,
    color_format: wgpu::TextureFormat,
    source: &str,
) -> wgpu::RenderPipeline {
    create_pipeline_with(
        device,
        pipeline_layout,
        path,
        &[Some(wgpu::ColorTargetState {
            format: color_format,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })],
        source,
        "fs_main",
    )
}

/// A terrain pipeline with any color targets and fragment entry point
/// (render graph geometry passes, ADR-0015).
fn create_pipeline_with(
    device: &wgpu::Device,
    pipeline_layout: &wgpu::PipelineLayout,
    path: TerrainPath,
    targets: &[Option<wgpu::ColorTargetState>],
    source: &str,
    fs_entry: &str,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("terrain"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let instance_layout = [Some(wgpu::VertexBufferLayout {
        array_stride: QUAD_BYTES,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![0 => Uint32x2],
    })];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("terrain"),
        layout: Some(pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: match path {
                TerrainPath::Pulling | TerrainPath::PullingIndexed => &[],
                TerrainPath::Instanced => &instance_layout,
            },
        },
        primitive: wgpu::PrimitiveState {
            topology: match path {
                TerrainPath::Pulling | TerrainPath::PullingIndexed => {
                    wgpu::PrimitiveTopology::TriangleList
                }
                TerrainPath::Instanced => wgpu::PrimitiveTopology::TriangleStrip,
            },
            // Face-direction culling happens on the CPU per batch.
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some(fs_entry),
            compilation_options: Default::default(),
            targets,
        }),
        multiview_mask: None,
        cache: None,
    })
}

impl TerrainRenderer {
    /// Creates pipelines and resources. `layer_table[state * 6 + dir]` is the
    /// texture layer of a block state's face; `quad_capacity` sizes the quad
    /// buffer (8 bytes per quad).
    ///
    /// # Panics
    /// Panics if `layer_table` is empty or not a multiple of 6 long.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        path: TerrainPath,
        color_format: wgpu::TextureFormat,
        textures: &TerrainTextures<'_>,
        layer_table: &[u32],
        quad_capacity: u32,
    ) -> Self {
        assert!(!layer_table.is_empty() && layer_table.len() % 6 == 0);
        let camera_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain camera"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let batch_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain batch origins"),
            size: u64::from(MAX_BATCHES) * 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let quad_usage = match path {
            TerrainPath::Pulling | TerrainPath::PullingIndexed => wgpu::BufferUsages::STORAGE,
            TerrainPath::Instanced => wgpu::BufferUsages::VERTEX,
        };
        let quad_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain quads"),
            size: u64::from(quad_capacity.max(1)) * QUAD_BYTES,
            usage: quad_usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (block_view, layers_view) = create_block_textures(device, queue, textures, layer_table);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("block sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let pack_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shader pack params"),
            // v1: 8 options and the time (48 bytes); v2 adds 24 options.
            size: PACK_PARAMS_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Light.
        let light_atlas_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("light atlas"),
            size: wgpu::Extent3d {
                width: ATLAS_BRICKS[0] * 16,
                height: ATLAS_BRICKS[1] * 16,
                depth_or_array_layers: ATLAS_BRICKS[2] * 16,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::R8Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let index_extent = wgpu::Extent3d {
            width: INDIRECTION[0],
            height: INDIRECTION[1],
            depth_or_array_layers: INDIRECTION[2],
        };
        let light_index_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("light indirection"),
            size: index_extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::R32Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let cells = (INDIRECTION[0] * INDIRECTION[1] * INDIRECTION[2]) as usize;
        queue.write_texture(
            light_index_tex.as_image_copy(),
            bytemuck::cast_slice(&vec![DEFAULT_LIGHT; cells]),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(INDIRECTION[0] * 4),
                rows_per_image: Some(INDIRECTION[1]),
            },
            index_extent,
        );

        let vs_fs = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let tex_entry = |binding, sample_type, dim| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: vs_fs,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: dim,
                multisampled: false,
            },
            count: None,
        };
        let uniform_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: vs_fs,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("terrain globals"),
            entries: &[
                uniform_entry(0),
                uniform_entry(1),
                tex_entry(
                    2,
                    wgpu::TextureSampleType::Float { filterable: true },
                    wgpu::TextureViewDimension::D2Array,
                ),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                tex_entry(
                    4,
                    wgpu::TextureSampleType::Uint,
                    wgpu::TextureViewDimension::D2,
                ),
                tex_entry(
                    5,
                    wgpu::TextureSampleType::Uint,
                    wgpu::TextureViewDimension::D3,
                ),
                tex_entry(
                    6,
                    wgpu::TextureSampleType::Uint,
                    wgpu::TextureViewDimension::D3,
                ),
                uniform_entry(7),
            ],
        });
        let view = |t: &wgpu::Texture, dim| {
            t.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(dim),
                ..Default::default()
            })
        };
        let atlas_view = view(&light_atlas_tex, wgpu::TextureViewDimension::D3);
        let index_view = view(&light_index_tex, wgpu::TextureViewDimension::D3);
        let bind0 = make_bind0(
            device,
            &layout0,
            [&camera_buf, &batch_buf, &pack_buf],
            &sampler,
            [&block_view, &layers_view, &atlas_view, &index_view],
        );
        let (layouts, bind1) = match path {
            TerrainPath::Pulling | TerrainPath::PullingIndexed => {
                let layout1 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("terrain quads"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });
                let bind1 = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("terrain quads"),
                    layout: &layout1,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: quad_buf.as_entire_binding(),
                    }],
                });
                (vec![layout1], Some(bind1))
            }
            TerrainPath::Instanced => (vec![], None),
        };
        let layout_refs: Vec<Option<&wgpu::BindGroupLayout>> = std::iter::once(Some(&layout0))
            .chain(layouts.iter().map(Some))
            .collect();
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain"),
            bind_group_layouts: &layout_refs,
            immediate_size: 0,
        });
        let pipeline = create_pipeline(
            device,
            &pipeline_layout,
            path,
            color_format,
            &shader_source(path),
        );

        let index_buf = (path == TerrainPath::PullingIndexed).then(|| {
            let indices: Vec<u32> = (0..INDEX_QUADS)
                .flat_map(|q| [0, 1, 2, 2, 1, 3].map(|c| q * 4 + c))
                .collect();
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("terrain quad indices"),
                size: (indices.len() * 4) as u64,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buf, 0, bytemuck::cast_slice(&indices));
            buf
        });
        Self {
            path,
            pipeline,
            pipeline_layout,
            color_format,
            layout0,
            sampler,
            atlas_view,
            index_view,
            pack_buf,
            index_buf,
            bind0,
            bind1,
            camera_buf,
            batch_buf,
            quad_buf,
            light_atlas_tex,
            light_index_tex,
            arena: RangeArena::new(quad_capacity),
            model_arena: RangeArena::new(MODEL_CAPACITY),
            batches: HashMap::new(),
            free_slots: (0..MAX_BATCHES).rev().collect(),
            atlas: LightAtlas::new(),
            staging: Vec::new(),
            texels: Box::new([0; 4096]),
            index_mirror: vec![DEFAULT_LIGHT; cells],
            index_dirty: 0,
            cull_margin: 0.0,
            draws: Vec::new(),
            stats: FrameStats::default(),
        }
    }

    /// The path this renderer draws with.
    #[must_use]
    pub fn path(&self) -> TerrainPath {
        self.path
    }

    /// Replaces the block textures and the layer table (resource packs),
    /// keeping everything uploaded.
    ///
    /// # Panics
    /// Panics if `layer_table` is empty or not a multiple of 6 long.
    pub fn set_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        textures: &TerrainTextures<'_>,
        layer_table: &[u32],
    ) {
        assert!(!layer_table.is_empty() && layer_table.len() % 6 == 0);
        let (block_view, layers_view) = create_block_textures(device, queue, textures, layer_table);
        self.bind0 = make_bind0(
            device,
            &self.layout0,
            [&self.camera_buf, &self.batch_buf, &self.pack_buf],
            &self.sampler,
            [
                &block_view,
                &layers_view,
                &self.atlas_view,
                &self.index_view,
            ],
        );
    }

    /// Switches to a shader pack's hooks (`None` = built-in look). The
    /// composed shader is validated first; on error nothing changes and the
    /// message is returned.
    pub fn set_shader_pack(
        &mut self,
        device: &wgpu::Device,
        pack: Option<&str>,
    ) -> Result<(), String> {
        let source = super::hooks::compose(self.path, pack);
        super::hooks::validate(&source)?;
        self.pipeline = create_pipeline(
            device,
            &self.pipeline_layout,
            self.path,
            self.color_format,
            &source,
        );
        Ok(())
    }

    /// Option values and time for shader packs (`pack_option`, `pack_time`).
    pub fn set_pack_params(&self, queue: &wgpu::Queue, options: &[f32; 8], time: f32) {
        let mut data = [0f32; 12];
        data[..8].copy_from_slice(options);
        data[8] = time;
        queue.write_buffer(&self.pack_buf, 0, bytemuck::cast_slice(&data));
    }

    /// Option values (up to 32, hook version 2) and time for shader packs.
    pub fn set_pack_params_v2(&self, queue: &wgpu::Queue, options: &[f32; 32], time: f32) {
        let mut data = [0f32; 36];
        data[..8].copy_from_slice(&options[..8]);
        data[8] = time;
        data[12..36].copy_from_slice(&options[8..]);
        queue.write_buffer(&self.pack_buf, 0, bytemuck::cast_slice(&data));
    }

    /// Grows the culling boxes by `blocks` (a version 2 pack's
    /// `max_displacement`), so displaced geometry is not culled too early.
    pub fn set_cull_margin(&mut self, blocks: f32) {
        self.cull_margin = blocks.max(0.0);
    }

    /// The built-in look into a target of `format` (the render graph's
    /// built-in opaque pass writes `colortex0`).
    #[must_use]
    pub fn builtin_pipeline(
        &self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        create_pipeline(
            device,
            &self.pipeline_layout,
            self.path,
            format,
            &super::hooks::compose(self.path, None),
        )
    }

    /// A version 2 geometry pass: the pack's `pack_vertex` / `pack_fragment`
    /// writing `targets` (name, format) in order. The composed shader is
    /// validated first; the error names the line.
    pub fn geometry_pipeline(
        &self,
        device: &wgpu::Device,
        pack: &str,
        targets: &[(String, wgpu::TextureFormat)],
    ) -> Result<wgpu::RenderPipeline, String> {
        let names: Vec<&str> = targets.iter().map(|(n, _)| n.as_str()).collect();
        let source = super::hooks::compose_geometry_v2(self.path, pack, &names);
        super::hooks::validate(&source)?;
        let states: Vec<Option<wgpu::ColorTargetState>> = targets
            .iter()
            .map(|(_, f)| {
                Some(wgpu::ColorTargetState {
                    format: *f,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })
            })
            .collect();
        Ok(create_pipeline_with(
            device,
            &self.pipeline_layout,
            self.path,
            &states,
            &source,
            "fs_pack",
        ))
    }

    /// Draws what the last `prepare` selected with another pipeline (render
    /// graph passes).
    pub fn render_with(&self, pass: &mut wgpu::RenderPass<'_>, pipeline: &wgpu::RenderPipeline) {
        self.draw(pass, pipeline);
    }

    /// Uploads (or replaces) the render batch at batch position `pos`
    /// (units of 64 blocks). An empty batch just removes the old one.
    pub fn upload_batch(
        &mut self,
        queue: &wgpu::Queue,
        pos: IVec3,
        packed: &PackedBatch,
    ) -> Result<(), TerrainError> {
        self.remove_batch(pos);
        if packed.is_empty() {
            return Ok(());
        }
        let total = packed.len() as u32;
        let slot = self.free_slots.pop().ok_or(TerrainError::BatchSlotsFull)?;
        let Some(range) = self.arena.alloc(total) else {
            self.free_slots.push(slot);
            return Err(TerrainError::QuadBufferFull);
        };
        let Some(models) = self.model_arena.alloc(packed.models.len() as u32) else {
            self.arena.free(range);
            self.free_slots.push(slot);
            return Err(TerrainError::ModelBufferFull);
        };
        self.staging.clear();
        let mut dirs: [Range<u32>; 6] = Default::default();
        for (dir, list) in packed.quads.iter().enumerate() {
            let start = range.start + self.staging.len() as u32;
            // The direction travels in bits 61..63 of the quad (unused by the
            // mesher: the block id ends at bit 47).
            self.staging
                .extend(list.iter().map(|q| q.0 | ((dir as u64) << 61)));
            dirs[dir] = start..range.start + self.staging.len() as u32;
        }
        queue.write_buffer(
            &self.quad_buf,
            u64::from(range.start) * QUAD_BYTES,
            bytemuck::cast_slice(&self.staging),
        );
        let origin = pos * BATCH_BLOCKS;
        queue.write_buffer(
            &self.batch_buf,
            u64::from(slot) * 16,
            bytemuck::cast_slice(&[origin.x, origin.y, origin.z, 0]),
        );
        self.batches.insert(
            pos,
            GpuBatch {
                slot,
                quads: range,
                dirs,
                models,
            },
        );
        Ok(())
    }

    /// Drops a batch and frees its buffer range.
    pub fn remove_batch(&mut self, pos: IVec3) {
        if let Some(b) = self.batches.remove(&pos) {
            self.arena.free(b.quads);
            self.model_arena.free(b.models);
            self.free_slots.push(b.slot);
        }
    }

    /// Whether a batch is resident.
    #[must_use]
    pub fn has_batch(&self, pos: IVec3) -> bool {
        self.batches.contains_key(&pos)
    }

    /// Uploads a section's light (`section` in units of 16 blocks).
    pub fn set_section_light(
        &mut self,
        queue: &wgpu::Queue,
        section: IVec3,
        light: &SectionLight,
    ) -> Result<(), TerrainError> {
        let data = match (&light.block, &light.sky) {
            (Nibbles::Single(block), Nibbles::Single(sky)) => SectionLightData::Uniform {
                block: *block,
                sky: *sky,
            },
            _ => {
                for (i, t) in self.texels.iter_mut().enumerate() {
                    *t = light.block.get(i) | (light.sky.get(i) << 4);
                }
                SectionLightData::Texels(&self.texels)
            }
        };
        let (entry, slot) = self
            .atlas
            .update(section, &data)
            .map_err(|_| TerrainError::LightAtlasFull)?;
        if let Some(slot) = slot {
            let o = LightAtlas::slot_origin(slot);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.light_atlas_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: o[0],
                        y: o[1],
                        z: o[2],
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &self.texels[..],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(16),
                    rows_per_image: Some(16),
                },
                wgpu::Extent3d {
                    width: 16,
                    height: 16,
                    depth_or_array_layers: 16,
                },
            );
        }
        self.write_index(queue, section, entry);
        Ok(())
    }

    /// Forgets an unloaded section's light (its cell reads as open sky).
    pub fn remove_section_light(&mut self, queue: &wgpu::Queue, section: IVec3) {
        self.atlas.remove(section);
        self.write_index(queue, section, DEFAULT_LIGHT);
    }

    /// Records an indirection entry; [`Self::flush_light_index`] uploads it.
    fn write_index(&mut self, _queue: &wgpu::Queue, section: IVec3, entry: u32) {
        let [x, y, z] = LightAtlas::indirection_index(section);
        let i = ((z * INDIRECTION[1] + y) * INDIRECTION[0] + x) as usize;
        if self.index_mirror[i] != entry {
            self.index_mirror[i] = entry;
            self.index_dirty |= 1 << z;
        }
    }

    /// Uploads the indirection entries changed since the last call: one
    /// `write_texture` per changed z slice (64 × 32 entries, 8 KiB). Called by
    /// [`Self::prepare`]; light changes take effect in the next frame drawn.
    pub fn flush_light_index(&mut self, queue: &wgpu::Queue) -> u32 {
        let mut writes = 0;
        let slice = (INDIRECTION[0] * INDIRECTION[1]) as usize;
        while self.index_dirty != 0 {
            let z = self.index_dirty.trailing_zeros();
            self.index_dirty &= self.index_dirty - 1;
            let start = z as usize * slice;
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.light_index_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: 0, y: 0, z },
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(&self.index_mirror[start..start + slice]),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(INDIRECTION[0] * 4),
                    rows_per_image: Some(INDIRECTION[1]),
                },
                wgpu::Extent3d {
                    width: INDIRECTION[0],
                    height: INDIRECTION[1],
                    depth_or_array_layers: 1,
                },
            );
            writes += 1;
        }
        writes
    }

    /// Culls and builds the draw list for a frame. `visible` limits drawing
    /// to these batches (cave culling result); `None` draws every resident
    /// batch inside the frustum.
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        camera: &CameraParams,
        visible: Option<&std::collections::HashSet<IVec3>>,
    ) -> FrameStats {
        self.flush_light_index(queue);
        let u = CameraUniform {
            view_proj: camera.view_proj.to_cols_array_2d(),
            position: [
                camera.position.x,
                camera.position.y,
                camera.position.z,
                camera.daylight,
            ],
            fog: [camera.fog.0, camera.fog.1, 0.0, 0.0],
            sky: [camera.sky[0], camera.sky[1], camera.sky[2], 1.0],
        };
        queue.write_buffer(&self.camera_buf, 0, bytemuck::bytes_of(&u));
        let frustum = Frustum::from_view_proj(camera.view_proj);
        self.draws.clear();
        let mut stats = FrameStats {
            batches_resident: self.batches.len() as u32,
            ..FrameStats::default()
        };
        let size = BATCH_BLOCKS as f32;
        for (pos, b) in &self.batches {
            stats.quads_resident += b.quads.end - b.quads.start;
            stats.models_resident += b.models.end - b.models.start;
            if visible.is_some_and(|v| !v.contains(pos)) {
                continue;
            }
            let min = pos.as_vec3() * size - Vec3::splat(self.cull_margin);
            let max = pos.as_vec3() * size + Vec3::splat(size + self.cull_margin);
            if !frustum.intersects(min, max) {
                continue;
            }
            stats.batches_drawn += 1;
            let mask = visible_face_mask(camera.position, min, max);
            // Merge neighboring visible directions into one draw.
            let mut open: Option<Range<u32>> = None;
            for (dir, r) in b.dirs.iter().enumerate() {
                if mask & (1 << dir) != 0 && !r.is_empty() {
                    match &mut open {
                        Some(o) if o.end == r.start => o.end = r.end,
                        _ => {
                            if let Some(o) = open.take() {
                                self.draws.push((b.slot, o));
                            }
                            open = Some(r.clone());
                        }
                    }
                }
            }
            if let Some(o) = open {
                self.draws.push((b.slot, o));
            }
        }
        for (_, r) in &self.draws {
            stats.quads_drawn += r.end - r.start;
        }
        stats.draws = self.draws.len() as u32;
        self.stats = stats;
        stats
    }

    /// Records the prepared draws into a pass whose depth attachment uses
    /// [`DEPTH_FORMAT`].
    pub fn render(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw(pass, &self.pipeline);
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, pipeline: &wgpu::RenderPipeline) {
        if self.draws.is_empty() {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.bind0, &[]);
        if let Some(b) = &self.bind1 {
            pass.set_bind_group(1, b, &[]);
        }
        match self.path {
            TerrainPath::Pulling => {
                for (slot, r) in &self.draws {
                    pass.draw(r.start * 6..r.end * 6, *slot..slot + 1);
                }
            }
            TerrainPath::PullingIndexed => {
                if let Some(ib) = &self.index_buf {
                    pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                }
                for (slot, r) in &self.draws {
                    let mut first = r.start;
                    while first < r.end {
                        let n = (r.end - first).min(INDEX_QUADS);
                        pass.draw_indexed(0..n * 6, (first * 4) as i32, *slot..slot + 1);
                        first += n;
                    }
                }
            }
            TerrainPath::Instanced => {
                pass.set_vertex_buffer(0, self.quad_buf.slice(..));
                for (slot, r) in &self.draws {
                    pass.draw(slot * 4..slot * 4 + 4, r.clone());
                }
            }
        }
    }

    /// Statistics of the last `prepare`.
    #[must_use]
    pub fn stats(&self) -> FrameStats {
        self.stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_path_follows_open_question_13() {
        use wgpu::DownlevelFlags as F;
        assert_eq!(
            TerrainPath::default_for(F::VERTEX_STORAGE | F::BASE_VERTEX),
            TerrainPath::PullingIndexed
        );
        assert_eq!(
            TerrainPath::default_for(F::VERTEX_STORAGE),
            TerrainPath::Pulling
        );
        assert_eq!(TerrainPath::default_for(F::empty()), TerrainPath::Instanced);
    }

    #[test]
    fn shaders_validate() {
        for path in [
            TerrainPath::Pulling,
            TerrainPath::PullingIndexed,
            TerrainPath::Instanced,
        ] {
            let module = naga::front::wgsl::parse_str(&terrain_shader(path)).unwrap_or_else(|e| {
                panic!("{path:?}: {}", e.emit_to_string(&terrain_shader(path)))
            });
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::empty(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{path:?}: {e:?}"));
        }
    }
}
