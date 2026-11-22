//! The wgpu side of the terrain renderer: one quad buffer shared by all
//! render batches (ranges from [`RangeArena`]), paths B and C, block texture
//! array, light bricks, per-frame culling into a reused draw list.

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
}

impl std::fmt::Display for TerrainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::QuadBufferFull => "terrain quad buffer is full",
            Self::BatchSlotsFull => "all render batch slots are in use",
            Self::LightAtlasFull => "light atlas is full",
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
}

struct GpuBatch {
    slot: u32,
    range: Range<u32>,
    /// Absolute quad ranges per direction inside `range`.
    dirs: [Range<u32>; 6],
}

/// Terrain renderer for one device.
pub struct TerrainRenderer {
    path: TerrainPath,
    pipeline: wgpu::RenderPipeline,
    bind0: wgpu::BindGroup,
    bind1: Option<wgpu::BindGroup>,
    index_buf: Option<wgpu::Buffer>,
    camera_buf: wgpu::Buffer,
    batch_buf: wgpu::Buffer,
    quad_buf: wgpu::Buffer,
    light_atlas_tex: wgpu::Texture,
    light_index_tex: wgpu::Texture,
    arena: RangeArena,
    batches: HashMap<IVec3, GpuBatch>,
    free_slots: Vec<u32>,
    atlas: LightAtlas,
    staging: Vec<u64>,
    texels: Box<[u8; 4096]>,
    /// (slot, quad range) per draw, rebuilt by `prepare`.
    draws: Vec<(u32, Range<u32>)>,
    stats: FrameStats,
}

fn shader_source(path: TerrainPath) -> String {
    let common = include_str!("terrain-common.wgsl");
    let entry = match path {
        TerrainPath::Pulling => include_str!("terrain-pulling.wgsl"),
        TerrainPath::PullingIndexed => include_str!("terrain-pulling-indexed.wgsl"),
        TerrainPath::Instanced => include_str!("terrain-instance.wgsl"),
    };
    format!("{common}\n{entry}")
}

/// Full WGSL of a path (for validation in tests and tools).
#[must_use]
pub fn terrain_shader(path: TerrainPath) -> String {
    shader_source(path)
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

        // Block textures.
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
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("block sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

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
            ],
        });
        let view = |t: &wgpu::Texture, dim| {
            t.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(dim),
                ..Default::default()
            })
        };
        let block_view = view(&block_tex, wgpu::TextureViewDimension::D2Array);
        let layers_view = view(&layers_tex, wgpu::TextureViewDimension::D2);
        let atlas_view = view(&light_atlas_tex, wgpu::TextureViewDimension::D3);
        let index_view = view(&light_index_tex, wgpu::TextureViewDimension::D3);
        let bind0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terrain globals"),
            layout: &layout0,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: batch_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&block_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&layers_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&index_view),
                },
            ],
        });
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
                (vec![layout0, layout1], Some(bind1))
            }
            TerrainPath::Instanced => (vec![layout0], None),
        };
        let layout_refs: Vec<Option<&wgpu::BindGroupLayout>> = layouts.iter().map(Some).collect();
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain"),
            bind_group_layouts: &layout_refs,
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("terrain"),
            source: wgpu::ShaderSource::Wgsl(shader_source(path).into()),
        });
        let instance_layout = [Some(wgpu::VertexBufferLayout {
            array_stride: QUAD_BYTES,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Uint32x2],
        })];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain"),
            layout: Some(&pipeline_layout),
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
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

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
            index_buf,
            bind0,
            bind1,
            camera_buf,
            batch_buf,
            quad_buf,
            light_atlas_tex,
            light_index_tex,
            arena: RangeArena::new(quad_capacity),
            batches: HashMap::new(),
            free_slots: (0..MAX_BATCHES).rev().collect(),
            atlas: LightAtlas::new(),
            staging: Vec::new(),
            texels: Box::new([0; 4096]),
            draws: Vec::new(),
            stats: FrameStats::default(),
        }
    }

    /// The path this renderer draws with.
    #[must_use]
    pub fn path(&self) -> TerrainPath {
        self.path
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
        let total = packed.len() as u32;
        if total == 0 {
            return Ok(());
        }
        let slot = self.free_slots.pop().ok_or(TerrainError::BatchSlotsFull)?;
        let Some(range) = self.arena.alloc(total) else {
            self.free_slots.push(slot);
            return Err(TerrainError::QuadBufferFull);
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
        self.batches.insert(pos, GpuBatch { slot, range, dirs });
        Ok(())
    }

    /// Drops a batch and frees its buffer range.
    pub fn remove_batch(&mut self, pos: IVec3) {
        if let Some(b) = self.batches.remove(&pos) {
            self.arena.free(b.range);
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

    fn write_index(&self, queue: &wgpu::Queue, section: IVec3, entry: u32) {
        let i = LightAtlas::indirection_index(section);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.light_index_tex,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: i[0],
                    y: i[1],
                    z: i[2],
                },
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::bytes_of(&entry),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
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
            stats.quads_resident += b.range.end - b.range.start;
            if visible.is_some_and(|v| !v.contains(pos)) {
                continue;
            }
            let min = pos.as_vec3() * size;
            let max = min + Vec3::splat(size);
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
        if self.draws.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
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
