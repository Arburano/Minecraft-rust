//! Entities on screen (T2-03): other players as simple colored boxes until
//! models exist (stage 4). One instanced draw after the terrain, in the same
//! pass and depth buffer; only vertex attributes and a uniform, so it works
//! on render paths B, Bi and C alike.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

use crate::terrain::renderer::DEPTH_FORMAT;

/// One box: minimum corner, size and color (linear RGB + unused alpha).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct BoxInstance {
    /// Minimum corner, world blocks.
    pub min: [f32; 3],
    /// Size, blocks.
    pub size: [f32; 3],
    /// Linear RGBA.
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraBlock {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    fog: [f32; 4],
    sky: [f32; 4],
}

/// The camera the boxes are drawn with (the terrain's).
#[derive(Clone, Copy, Debug)]
pub struct BoxCamera {
    /// Projection × view.
    pub view_proj: Mat4,
    /// Eye position.
    pub eye: Vec3,
    /// Fog start and end, blocks.
    pub fog: (f32, f32),
    /// Fog color, linear RGB.
    pub sky: [f32; 3],
}

/// Most boxes drawn per frame.
pub const MAX_BOXES: usize = 4096;

/// The box renderer.
pub struct BoxRenderer {
    pipeline: wgpu::RenderPipeline,
    camera: wgpu::Buffer,
    instances: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    count: u32,
}

impl BoxRenderer {
    /// Creates the pipeline for `color_format` and the terrain's depth format.
    #[must_use]
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("entity boxes"),
            source: wgpu::ShaderSource::Wgsl(include_str!("boxes.wgsl").into()),
        });
        let camera = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("entity camera"),
            size: std::mem::size_of::<CameraBlock>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("entity boxes"),
            size: (MAX_BOXES * std::mem::size_of::<BoxInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("entity boxes"),
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
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("entity boxes"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("entity boxes"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("entity boxes"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<BoxInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3, 1 => Float32x3, 2 => Float32x4
                    ],
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
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
        Self {
            pipeline,
            camera,
            instances,
            bind_group,
            count: 0,
        }
    }

    /// Uploads this frame's camera and boxes (at most [`MAX_BOXES`]).
    pub fn prepare(&mut self, queue: &wgpu::Queue, camera: &BoxCamera, boxes: &[BoxInstance]) {
        let block = CameraBlock {
            view_proj: camera.view_proj.to_cols_array_2d(),
            eye: camera.eye.extend(1.0).to_array(),
            fog: [camera.fog.0, camera.fog.1, 0.0, 0.0],
            sky: [camera.sky[0], camera.sky[1], camera.sky[2], 1.0],
        };
        queue.write_buffer(&self.camera, 0, bytemuck::bytes_of(&block));
        let n = boxes.len().min(MAX_BOXES);
        if n > 0 {
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&boxes[..n]));
        }
        self.count = n as u32;
    }

    /// Draws the boxes into a pass that uses the terrain's depth buffer.
    pub fn render(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.draw(0..36, 0..self.count);
    }
}

/// The boxes of a player standing at `feet` facing `yaw` (0 looks towards
/// −Z): body, head and a small nose that shows where they look.
#[must_use]
pub fn player_boxes(feet: Vec3, yaw: f32, color: [f32; 3]) -> [BoxInstance; 3] {
    let body = BoxInstance {
        min: [feet.x - 0.3, feet.y, feet.z - 0.3],
        size: [0.6, 1.35, 0.6],
        color: [color[0], color[1], color[2], 1.0],
    };
    let head = BoxInstance {
        min: [feet.x - 0.25, feet.y + 1.35, feet.z - 0.25],
        size: [0.5, 0.45, 0.5],
        color: [0.85, 0.7, 0.55, 1.0],
    };
    let (s, c) = yaw.sin_cos();
    let nose = feet + Vec3::new(-s * 0.28, 1.5, -c * 0.28);
    let nose = BoxInstance {
        min: [nose.x - 0.06, nose.y - 0.06, nose.z - 0.06],
        size: [0.12, 0.12, 0.12],
        color: [0.2, 0.15, 0.1, 1.0],
    };
    [body, head, nose]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_validates() {
        let module = naga::front::wgsl::parse_str(include_str!("boxes.wgsl")).expect("parse");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .expect("valid");
    }

    #[test]
    fn nose_points_where_the_player_looks() {
        let [_, _, nose] = player_boxes(Vec3::ZERO, 0.0, [0.0; 3]);
        assert!(nose.min[2] < -0.2, "yaw 0 looks towards -Z");
    }
}
