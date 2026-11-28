//! T1-25: the cost of a row of columns leaving the view: 39 columns × 24
//! sections of light removed in one frame, then the next frame prepared.
//! Prints the median and max over 20 rounds (CPU time on the calling
//! thread; on a software adapter the numbers show the call overhead only).
//!
//! `cargo run --release -p render --example light_churn`

use std::time::{Duration, Instant};

use glam::{IVec3, Mat4, Vec3};
use light::{Nibbles, SectionLight};
use render::terrain::renderer::{CameraParams, TerrainPath, TerrainRenderer, TerrainTextures};

fn main() {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle().with_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("adapter");
    println!(
        "adapter: {} ({:?})",
        adapter.get_info().name,
        adapter.get_info().backend
    );
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("device");
    let levels: Vec<Vec<u8>> = (0..5)
        .map(|m| [128u8; 4].repeat((16 >> m) * (16 >> m)))
        .collect();
    let mut r = TerrainRenderer::new(
        &device,
        &queue,
        TerrainPath::Pulling,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        &TerrainTextures {
            size: 16,
            layers: 1,
            levels: &levels,
        },
        &[0u32; 12],
        1 << 12,
    );
    // Mixed light: half the sections uniform, half with texels (bricks).
    let uniform = SectionLight {
        block: Nibbles::Single(0),
        sky: Nibbles::Single(15),
    };
    let mut varied = uniform.clone();
    varied.sky.set(5, 3);
    let row: Vec<IVec3> = (0..39)
        .flat_map(|x| (-4..20).map(move |y| IVec3::new(x, y, 7)))
        .collect();
    let camera = CameraParams {
        view_proj: Mat4::IDENTITY,
        position: Vec3::ZERO,
        daylight: 1.0,
        fog: (100.0, 200.0),
        sky: [0.5, 0.7, 0.9],
    };
    let mut times = Vec::new();
    for _ in 0..20 {
        for (i, &s) in row.iter().enumerate() {
            let l = if i % 2 == 0 { &uniform } else { &varied };
            r.set_section_light(&queue, s, l).expect("light");
        }
        r.prepare(&queue, &camera, None);
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(10)),
        });
        let t = Instant::now();
        for &s in &row {
            r.remove_section_light(&queue, s);
        }
        r.prepare(&queue, &camera, None);
        times.push(t.elapsed());
        queue.submit([]);
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(10)),
        });
    }
    times.sort();
    println!(
        "remove {} sections + prepare: median {:.3} ms, max {:.3} ms (20 rounds)",
        row.len(),
        times[times.len() / 2].as_secs_f64() * 1e3,
        times[times.len() - 1].as_secs_f64() * 1e3
    );
}
