//! Runs a validated render graph (ADR-0015) on the GPU: allocates the
//! targets, builds the pipelines of the pack's passes, and records a frame:
//! the opaque geometry (the pack's G-buffer pass or the built-in forward
//! pass), the full-screen deferred and composite passes, and the final pass
//! into the output view.
//!
//! The engine keeps the geometry: quad decoding, culling and draw
//! submission are [`TerrainRenderer`]'s. A pack supplies hooks and the
//! wiring of targets. Any error while building names the pass, so the
//! caller can fall back to the built-in look.

use std::collections::BTreeMap;

use glam::{Mat4, Vec3};

use super::{GraphError, GraphLimits, GraphManifest, Stage, ValidGraph};
use crate::terrain::hooks::{ReadKind, compose_fullscreen, validate};
use crate::terrain::renderer::{CameraParams, DEPTH_FORMAT, TerrainRenderer};

/// A version 2 pack: its manifest and the WGSL files it names.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphPack {
    /// `shaderpack.toml`.
    pub manifest: GraphManifest,
    /// File name → WGSL source.
    pub files: BTreeMap<String, String>,
}

/// What changes every frame besides the camera.
#[derive(Clone, Copy, Debug)]
pub struct FrameParams {
    /// The camera (same as for the terrain renderer).
    pub camera: CameraParams,
    /// Seconds since the world opened.
    pub time: f32,
    /// Direction towards the sun (normalized by the engine).
    pub sun: Vec3,
    /// Option values (up to 32).
    pub options: [f32; 32],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PackFrameUniform {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    prev_view_proj: [[f32; 4]; 4],
    camera: [f32; 4],
    prev_camera: [f32; 4],
    viewport: [f32; 4],
    frame: [f32; 4],
    fog: [f32; 4],
    sky: [f32; 4],
    sun: [f32; 4],
    options: [[f32; 4]; 8],
}

/// A full-screen pass, built.
struct FullscreenPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// Per read: `Some(slot)` or the engine's depth (`None`).
    reads: Vec<Option<usize>>,
    /// Slots written (empty: the output view, final pass).
    writes: Vec<usize>,
    /// Uniform of this pass (its own viewport size).
    frame_buf: wgpu::Buffer,
    scale: f32,
}

/// The target textures at the current size.
struct Targets {
    slots: Vec<(wgpu::Texture, wgpu::TextureView)>,
    depth: wgpu::TextureView,
    width: u32,
    height: u32,
}

/// A render graph ready to draw frames.
pub struct GraphRenderer {
    graph: ValidGraph,
    max_displacement: f32,
    targets: Targets,
    geometry: Vec<(wgpu::RenderPipeline, Vec<usize>)>,
    builtin: Option<wgpu::RenderPipeline>,
    fullscreen: Vec<FullscreenPass>,
    blit: Option<FullscreenPass>,
    sampler: wgpu::Sampler,
    output_format: wgpu::TextureFormat,
    frame_index: u32,
    prev: Option<(Mat4, Vec3)>,
    /// History slots swapped this frame.
    flipped: bool,
}

const BLIT: &str = "
fn pack_fullscreen(uv: vec2<f32>) -> PackTargets {
    var t: PackTargets;
    t.screen = textureSample(colortex0, pack_sampler, uv);
    return t;
}
";

fn scaled(size: u32, scale: f32) -> u32 {
    ((f64::from(size) * f64::from(scale)).ceil() as u32).max(1)
}

impl GraphRenderer {
    /// Validates `pack` and builds its passes for an output of
    /// `output_format` at `width`×`height`. The terrain renderer's culling
    /// margin is set to the pack's `max_displacement`.
    pub fn new(
        device: &wgpu::Device,
        terrain: &mut TerrainRenderer,
        pack: &GraphPack,
        output_format: wgpu::TextureFormat,
        (width, height): (u32, u32),
        limits: &GraphLimits,
    ) -> Result<Self, GraphError> {
        let graph = pack.manifest.validate(limits)?;
        let file = |p: &super::PassDecl| {
            pack.files.get(&p.file).ok_or_else(|| GraphError {
                pass: Some(p.label()),
                message: format!("the pack has no file {}", p.file),
            })
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pack sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let mut geometry = Vec::new();
        let mut fullscreen = Vec::new();
        let mut final_pass = None;
        for (p, io) in graph.passes.iter().zip(&graph.io) {
            let code = file(p)?;
            let err = |message: String| GraphError {
                pass: Some(p.label()),
                message,
            };
            match p.stage {
                Stage::GbufferOpaque => {
                    let targets: Vec<(String, wgpu::TextureFormat)> = p
                        .writes
                        .iter()
                        .zip(&io.writes)
                        .map(|(name, &slot)| (name.clone(), graph.slots[slot].format.wgpu()))
                        .collect();
                    let pipeline = terrain
                        .geometry_pipeline(device, code, &targets)
                        .map_err(err)?;
                    geometry.push((pipeline, io.writes.clone()));
                }
                Stage::Deferred | Stage::Composite | Stage::Final => {
                    let reads: Vec<(&str, ReadKind)> = p
                        .reads
                        .iter()
                        .zip(&io.reads)
                        .map(|(n, slot)| {
                            (
                                n.as_str(),
                                if slot.is_some() {
                                    ReadKind::Color
                                } else {
                                    ReadKind::Depth
                                },
                            )
                        })
                        .collect();
                    let (writes, formats): (Vec<&str>, Vec<wgpu::TextureFormat>) =
                        if p.stage == Stage::Final {
                            (vec!["screen"], vec![output_format])
                        } else {
                            (
                                p.writes.iter().map(String::as_str).collect(),
                                io.writes
                                    .iter()
                                    .map(|&s| graph.slots[s].format.wgpu())
                                    .collect(),
                            )
                        };
                    let source = compose_fullscreen(code, &reads, &writes);
                    validate(&source).map_err(err)?;
                    let scale = io.writes.first().map_or(1.0, |&s| graph.slots[s].scale);
                    let pass = build_fullscreen(
                        device,
                        &source,
                        &reads.iter().map(|r| r.1).collect::<Vec<_>>(),
                        &formats,
                        io.reads.clone(),
                        if p.stage == Stage::Final {
                            Vec::new()
                        } else {
                            io.writes.clone()
                        },
                        scale,
                    );
                    if p.stage == Stage::Final {
                        final_pass = Some(pass);
                    } else {
                        fullscreen.push(pass);
                    }
                }
                other => {
                    return Err(err(format!(
                        "the {} stage is not implemented",
                        other.name()
                    )));
                }
            }
        }
        // The built-in opaque pass draws into colortex0's first version.
        let builtin = graph
            .builtin_color
            .map(|slot| terrain.builtin_pipeline(device, graph.slots[slot].format.wgpu()));
        // The built-in final pass copies colortex0 to the output.
        let blit = match (final_pass, graph.output) {
            (Some(p), _) => Some(p),
            (None, Some(slot)) => {
                let source =
                    compose_fullscreen(BLIT, &[("colortex0", ReadKind::Color)], &["screen"]);
                validate(&source).map_err(|message| GraphError {
                    pass: None,
                    message,
                })?;
                Some(build_fullscreen(
                    device,
                    &source,
                    &[ReadKind::Color],
                    &[output_format],
                    vec![Some(slot)],
                    Vec::new(),
                    1.0,
                ))
            }
            (None, None) => None,
        };
        terrain.set_cull_margin(pack.manifest.max_displacement);
        let targets = make_targets(device, &graph, width, height);
        Ok(Self {
            max_displacement: pack.manifest.max_displacement,
            graph,
            targets,
            geometry,
            builtin,
            fullscreen,
            blit,
            sampler,
            output_format,
            frame_index: 0,
            prev: None,
            flipped: false,
        })
    }

    /// The validated graph (passes, slots, memory).
    #[must_use]
    pub fn graph(&self) -> &ValidGraph {
        &self.graph
    }

    /// The vertex displacement the pack declared (culling margin).
    #[must_use]
    pub fn max_displacement(&self) -> f32 {
        self.max_displacement
    }

    /// Recreates the targets for a new output size (history is lost).
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if (width, height) != (self.targets.width, self.targets.height) {
            self.targets = make_targets(device, &self.graph, width, height);
            self.prev = None;
        }
    }

    /// The slot a logical slot index refers to this frame (history pairs
    /// alternate).
    fn phys(&self, slot: usize) -> usize {
        if !self.flipped {
            return slot;
        }
        for &[a, b] in self.graph.history.values() {
            if slot == a {
                return b;
            }
            if slot == b {
                return a;
            }
        }
        slot
    }

    /// Records one frame into `encoder`, ending in `output`. The terrain
    /// renderer must have been prepared for this frame's camera.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        terrain: &TerrainRenderer,
        frame: &FrameParams,
        output: &wgpu::TextureView,
    ) {
        let cam = &frame.camera;
        let (prev_vp, prev_pos) = self.prev.unwrap_or((cam.view_proj, cam.position));
        let sun = frame.sun.normalize_or_zero();
        let mut options = [[0f32; 4]; 8];
        for (i, v) in frame.options.iter().enumerate() {
            options[i / 4][i % 4] = *v;
        }
        let uniform = |w: u32, h: u32| PackFrameUniform {
            view_proj: cam.view_proj.to_cols_array_2d(),
            inv_view_proj: cam.view_proj.inverse().to_cols_array_2d(),
            prev_view_proj: prev_vp.to_cols_array_2d(),
            camera: [cam.position.x, cam.position.y, cam.position.z, cam.daylight],
            prev_camera: [prev_pos.x, prev_pos.y, prev_pos.z, 0.0],
            viewport: [w as f32, h as f32, 1.0 / w as f32, 1.0 / h as f32],
            frame: [self.frame_index as f32, frame.time, 0.0, 0.0],
            fog: [cam.fog.0, cam.fog.1, 0.0, 0.0],
            sky: [cam.sky[0], cam.sky[1], cam.sky[2], 1.0],
            sun: [sun.x, sun.y, sun.z, 0.0],
            options,
        };
        let (w, h) = (self.targets.width, self.targets.height);
        let clear_sky = wgpu::Color {
            r: f64::from(cam.sky[0]),
            g: f64::from(cam.sky[1]),
            b: f64::from(cam.sky[2]),
            a: 1.0,
        };
        // Opaque geometry.
        let depth_attachment = || {
            Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.targets.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            })
        };

        if self.graph.custom_gbuffer {
            for (pipeline, writes) in &self.geometry {
                let attachments: Vec<_> = writes
                    .iter()
                    .map(|&s| {
                        color(
                            &self.targets.slots[self.phys(s)].1,
                            wgpu::Color::TRANSPARENT,
                        )
                    })
                    .collect();
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("pack gbuffer_opaque"),
                    color_attachments: &attachments,
                    depth_stencil_attachment: depth_attachment(),
                    ..Default::default()
                });
                terrain.render_with(&mut pass, pipeline);
            }
        } else {
            let view = match (self.graph.builtin_color, &self.builtin) {
                (Some(slot), Some(_)) => &self.targets.slots[self.phys(slot)].1,
                _ => output,
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("built-in opaque"),
                color_attachments: &[color(view, clear_sky)],
                depth_stencil_attachment: depth_attachment(),
                ..Default::default()
            });
            match &self.builtin {
                Some(p) => terrain.render_with(&mut pass, p),
                None => terrain.render(&mut pass),
            }
        }
        // Full-screen passes, then the final one.
        for (k, p) in self.fullscreen.iter().chain(&self.blit).enumerate() {
            let is_final = k == self.fullscreen.len();
            let (pw, ph) = (scaled(w, p.scale), scaled(h, p.scale));
            let (pw, ph) = if is_final { (w, h) } else { (pw, ph) };
            queue.write_buffer(&p.frame_buf, 0, bytemuck::bytes_of(&uniform(pw, ph)));
            let mut entries = vec![
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: p.frame_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ];
            for (i, r) in p.reads.iter().enumerate() {
                let view = match r {
                    Some(s) => &self.targets.slots[self.phys(*s)].1,
                    None => &self.targets.depth,
                };
                entries.push(wgpu::BindGroupEntry {
                    binding: i as u32 + 2,
                    resource: wgpu::BindingResource::TextureView(view),
                });
            }
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("pack pass"),
                layout: &p.layout,
                entries: &entries,
            });
            let attachments: Vec<_> = if p.writes.is_empty() {
                vec![color(output, wgpu::Color::BLACK)]
            } else {
                p.writes
                    .iter()
                    .map(|&s| {
                        color(
                            &self.targets.slots[self.phys(s)].1,
                            wgpu::Color::TRANSPARENT,
                        )
                    })
                    .collect()
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pack full-screen"),
                color_attachments: &attachments,
                ..Default::default()
            });
            pass.set_pipeline(&p.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        self.prev = Some((cam.view_proj, cam.position));
        self.frame_index = self.frame_index.wrapping_add(1);
        if !self.graph.history.is_empty() {
            self.flipped = !self.flipped;
        }
    }

    /// The output format the final pass writes.
    #[must_use]
    pub fn output_format(&self) -> wgpu::TextureFormat {
        self.output_format
    }
}

fn color(
    view: &wgpu::TextureView,
    clear: wgpu::Color,
) -> Option<wgpu::RenderPassColorAttachment<'_>> {
    Some(wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(clear),
            store: wgpu::StoreOp::Store,
        },
    })
}

fn make_targets(device: &wgpu::Device, graph: &ValidGraph, width: u32, height: u32) -> Targets {
    let slots = graph
        .slots
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("pack target slot {i}")),
                size: wgpu::Extent3d {
                    width: scaled(width, s.scale),
                    height: scaled(height, s.scale),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: s.format.wgpu(),
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let v = t.create_view(&Default::default());
            (t, v)
        })
        .collect();
    let depth = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("graph depth0"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default());
    Targets {
        slots,
        depth,
        width,
        height,
    }
}

fn build_fullscreen(
    device: &wgpu::Device,
    source: &str,
    kinds: &[ReadKind],
    formats: &[wgpu::TextureFormat],
    reads: Vec<Option<usize>>,
    writes: Vec<usize>,
    scale: f32,
) -> FullscreenPass {
    let mut entries = vec![
        wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        },
    ];
    for (i, k) in kinds.iter().enumerate() {
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: i as u32 + 2,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: match k {
                    ReadKind::Color => wgpu::TextureSampleType::Float { filterable: true },
                    ReadKind::Depth => wgpu::TextureSampleType::Depth,
                },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        });
    }
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("pack pass"),
        entries: &entries,
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("pack pass"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("pack pass"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let targets: Vec<Option<wgpu::ColorTargetState>> = formats
        .iter()
        .map(|&format| {
            Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })
        })
        .collect();
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("pack pass"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_full"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_full"),
            compilation_options: Default::default(),
            targets: &targets,
        }),
        multiview_mask: None,
        cache: None,
    });
    let frame_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pack frame"),
        size: std::mem::size_of::<PackFrameUniform>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    FullscreenPass {
        pipeline,
        layout,
        reads,
        writes,
        frame_buf,
        scale,
    }
}
