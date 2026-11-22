//! T1-19: textures and shader packs swap in a live renderer without
//! re-uploading the world, and swapping back restores the exact picture.
//! Needs a GPU adapter (software Vulkan is enough); skips without one (CI).

use glam::{Mat4, Vec3};
use mesh::batch::{RenderBatch, slot};
use mesh::{MeshChunkBuilder, PackedBatch, SectionNeighborhood};
use render::terrain::camera::{look_to, perspective};
use render::terrain::renderer::{
    CameraParams, DEPTH_FORMAT, TerrainPath, TerrainRenderer, TerrainTextures,
};
use world::{BlockStateId, Section};

const SIZE: u32 = 64;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

/// One 16×16 layer of a single color with its mip chain.
fn tex(l: &[Vec<u8>]) -> TerrainTextures<'_> {
    TerrainTextures {
        size: 16,
        layers: 1,
        levels: l,
    }
}

fn levels(color: [u8; 4]) -> Vec<Vec<u8>> {
    (0..5)
        .map(|m| color.repeat(((16 >> m) * (16 >> m)) as usize))
        .collect()
}

fn one_block_batch() -> PackedBatch {
    let n = SectionNeighborhood::from_fn(world::coords::MeshChunkPos { x: 0, y: 0, z: 0 }, |sp| {
        let mut s = Section::new();
        if (sp.x, sp.y, sp.z) == (0, 0, 0) {
            s.set((8 * 16 + 8) * 16 + 8, BlockStateId(1));
        }
        s
    });
    let quads = MeshChunkBuilder::new().build(&n, |_| false);
    let mut batch = RenderBatch::new();
    batch.set_chunk(slot(0, 0, 0), &quads);
    let mut packed = PackedBatch::default();
    batch.pack_into(&mut packed);
    packed
}

fn render(device: &wgpu::Device, queue: &wgpu::Queue, r: &mut TerrainRenderer) -> Vec<u8> {
    let view_proj: Mat4 = perspective(1.0, 1.0, 0.1, 100.0)
        * look_to(
            Vec3::new(8.5, 12.0, 14.0),
            Vec3::new(0.0, -3.5, -5.5),
            Vec3::Y,
        );
    r.prepare(
        queue,
        &CameraParams {
            view_proj,
            position: Vec3::new(8.5, 12.0, 14.0),
            daylight: 1.0,
            fog: (1000.0, 2000.0),
            sky: [0.0, 0.0, 0.0],
        },
        None,
    );
    let extent = wgpu::Extent3d {
        width: SIZE,
        height: SIZE,
        depth_or_array_layers: 1,
    };
    let tex = |format, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let color = tex(
        FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let depth = tex(DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT);
    let (cv, dv) = (
        color.create_view(&Default::default()),
        depth.create_view(&Default::default()),
    );
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(SIZE * SIZE * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&Default::default());
    {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &cv,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &dv,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        r.render(&mut pass);
    }
    enc.copy_texture_to_buffer(
        color.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE * 4),
                rows_per_image: Some(SIZE),
            },
        },
        extent,
    );
    queue.submit([enc.finish()]);
    buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
    buf.slice(..).get_mapped_range().expect("mapped").to_vec()
}

#[test]
fn textures_and_shader_packs_swap_live_and_back_exactly() {
    let Some((device, queue)) = device() else {
        eprintln!("skipped: no GPU adapter");
        return;
    };
    let red = levels([220, 30, 30, 255]);
    let blue = levels([30, 30, 220, 255]);
    let table = [0u32; 12];
    let mut r = TerrainRenderer::new(
        &device,
        &queue,
        TerrainPath::Pulling,
        FORMAT,
        &tex(&red),
        &table,
        1 << 12,
    );
    r.upload_batch(&queue, glam::IVec3::ZERO, &one_block_batch())
        .expect("upload");
    let a = render(&device, &queue, &mut r);
    assert!(
        a.chunks(4).any(|p| p[0] > 100 && p[2] < 80),
        "the red block is visible"
    );

    r.set_textures(&device, &queue, &tex(&blue), &table);
    let b = render(&device, &queue, &mut r);
    assert!(
        b.chunks(4).any(|p| p[2] > 100 && p[0] < 80),
        "textures swapped without re-upload"
    );
    r.set_textures(&device, &queue, &tex(&red), &table);
    assert_eq!(
        render(&device, &queue, &mut r),
        a,
        "swapping back restores the exact picture"
    );

    assert!(
        r.set_shader_pack(
            &device,
            Some("fn pack_surface(s: Surface) -> f32 { return 1.0; }")
        )
        .is_err()
    );
    assert_eq!(
        render(&device, &queue, &mut r),
        a,
        "a broken pack changes nothing"
    );
    r.set_shader_pack(
        &device,
        Some("fn pack_surface(s: Surface) -> vec3<f32> { return vec3<f32>(0.0, 1.0, 0.0); }"),
    )
    .expect("valid pack");
    let g = render(&device, &queue, &mut r);
    assert!(
        g.chunks(4).any(|p| p[1] > 200 && p[0] < 20),
        "the pack's surface hook draws"
    );
    r.set_shader_pack(&device, None).expect("built-in");
    assert_eq!(
        render(&device, &queue, &mut r),
        a,
        "built-in look restored exactly"
    );
}
