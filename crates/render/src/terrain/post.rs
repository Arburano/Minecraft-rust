//! A shader pack's full-screen post pass (ADR-0014, version 1): the pack's
//! `post.wgsl` defines `fn pack_post(color: vec3<f32>, uv: vec2<f32>, depth:
//! f32) -> vec3<f32>`, run over the rendered world before the UI is drawn.

/// The WGSL around the pack's `pack_post`.
#[must_use]
pub fn post_shader(pack: &str) -> String {
    format!(
        "struct PackParams {{
    options: array<vec4<f32>, 2>,
    time: vec4<f32>,
}}
@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_depth: texture_depth_2d;
@group(0) @binding(2) var scene_sampler: sampler;
@group(0) @binding(3) var<uniform> pack_params: PackParams;

fn pack_option(i: u32) -> f32 {{
    return pack_params.options[i / 4u][i % 4u];
}}

fn pack_time() -> f32 {{
    return pack_params.time.x;
}}

struct PostOut {{
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}}

@vertex
fn vs_post(@builtin(vertex_index) i: u32) -> PostOut {{
    // One triangle covering the screen.
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: PostOut;
    out.pos = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(xy.x, 1.0 - xy.y);
    return out;
}}

// ---- shader pack ----
{pack}
// ---- end of shader pack ----

@fragment
fn fs_post(in: PostOut) -> @location(0) vec4<f32> {{
    let color = textureSample(scene, scene_sampler, in.uv).rgb;
    let depth = textureLoad(scene_depth, vec2<i32>(in.pos.xy), 0);
    return vec4<f32>(pack_post(color, in.uv, depth), 1.0);
}}
"
    )
}

/// The post pass of one shader pack.
pub struct PostPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params: wgpu::Buffer,
}

impl PostPass {
    /// Compiles the pack's post code for `format`; validation errors are
    /// returned as text.
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        pack: &str,
    ) -> Result<Self, String> {
        let source = post_shader(pack);
        super::hooks::validate(&source)?;
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty,
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    1,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    2,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                entry(
                    3,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("post"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_post"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_post"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("post params"),
            size: 48,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            pipeline,
            layout,
            sampler,
            params,
        })
    }

    /// Option values and time (same meaning as for the terrain hooks).
    pub fn set_params(&self, queue: &wgpu::Queue, options: &[f32; 8], time: f32) {
        let mut data = [0f32; 12];
        data[..8].copy_from_slice(options);
        data[8] = time;
        queue.write_buffer(&self.params, 0, bytemuck::cast_slice(&data));
    }

    /// Draws `scene` (with its depth) through the pack into `target`.
    pub fn render(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scene: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        target: &wgpu::TextureView,
    ) {
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(scene),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.params.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("post"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_code_validates_and_errors_are_reported() {
        let vignette = "
fn pack_post(color: vec3<f32>, uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let d = distance(uv, vec2<f32>(0.5));
    return color * (1.0 - pack_option(0u) * d * d) + vec3<f32>(depth * 0.0);
}";
        super::super::hooks::validate(&post_shader(vignette)).expect("valid");
        assert!(super::super::hooks::validate(&post_shader("fn pack_post() {}")).is_err());
        assert!(
            super::super::hooks::validate(&post_shader("")).is_err(),
            "pack_post is required"
        );
    }
}
