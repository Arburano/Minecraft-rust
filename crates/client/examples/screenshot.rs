//! T1-11: renders a generated world offscreen with the terrain renderer and
//! saves a BMP. Also prints culling statistics for a surface view and an
//! underground view (cave culling share) and the CPU cost of `prepare`.
//!
//! `cargo run --release -p client --example screenshot -- [radius] [out.bmp] [B|Bi|C]`
//!
//! `FRAMES=300` also renders that many timed frames (GPU timestamp queries
//! when the adapter has them) and prints medians and percentiles: the T1-11
//! laptop measurement (open questions 13 and 14). `ADAPTER_NAME=vega` picks
//! an adapter by name; `WGPU_BACKEND=vulkan|dx12|gl` picks the backend.
//!
//! Works on any adapter, including software Vulkan (llvmpipe) in a VM: the
//! image is meaningful there, frame times are not.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use assets::{FALLBACK_KEYS, fallback_key, fallback_set};
use client::loading::{LoadConfig, Loader, Viewer};
use engine_core::jobs::JobPool;
use glam::{IVec3, Vec3};
use light::WorldLight;
use mesh::batch::{RenderBatch, slot};
use mesh::{ChunkVisibility, PackedBatch};
use render::terrain::camera::{look_to, perspective};
use render::terrain::culling::{Frustum, cave_visible};
use render::terrain::profiles::QualityProfile;
use render::terrain::renderer::{
    CameraParams, DEPTH_FORMAT, TerrainPath, TerrainRenderer, TerrainTextures,
};
use world::{BlockStateId, ColumnPos, WorldBounds, WorldMap};
use worldgen::{GenContext, Pipeline, WorldgenConfig};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;

fn main() {
    let mut args = std::env::args().skip(1);
    let radius: i32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(8);
    let out = PathBuf::from(args.next().unwrap_or_else(|| "terrain.bmp".into()));
    let forced_path = args.next();

    // World.
    let mods = world::modset::source_build(world::modset::DEFAULT_BUILD).expect("build");
    let b = mods.registry_builder().expect("blocks");
    let reg = Arc::new(b.build().expect("registry"));
    let src = mods.read("worldgen/overworld.toml").expect("config");
    let bounds = WorldBounds::default();
    let ctx = Arc::new(GenContext::new(
        7,
        WorldgenConfig::from_toml(&src, &reg).expect("config"),
        bounds,
    ));
    let mut loader = Loader::new(
        LoadConfig {
            radius,
            ..LoadConfig::default()
        },
        Arc::clone(&reg),
        Arc::new(Pipeline::overworld()),
        ctx,
    );
    let mut map = WorldMap::new(Arc::clone(&reg), bounds);
    let mut light = WorldLight::new();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let pool = JobPool::new(threads).expect("pool");
    let viewer = Viewer {
        pos: [8.0, 80.0, 8.0],
        velocity: [0.0; 3],
    };
    let t = Instant::now();
    let all = |r: i32| (-r..=r).flat_map(move |x| (-r..=r).map(move |z| ColumnPos { x, z }));
    while !all(radius).all(|c| loader.is_ready(&map, c)) && t.elapsed() < Duration::from_secs(600) {
        loader.tick(&viewer, &mut map, &mut light, &pool);
        std::thread::sleep(Duration::from_millis(2));
    }
    println!(
        "world radius {radius} loaded in {:?} ({threads} workers)",
        t.elapsed()
    );

    // GPU.
    let desc = wgpu::InstanceDescriptor::new_without_display_handle().with_env();
    let backends = desc.backends;
    let instance = wgpu::Instance::new(desc);
    // ADAPTER_NAME picks an adapter by name (indices are not stable, T0-14).
    let wanted = std::env::var("ADAPTER_NAME").ok().map(|n| n.to_lowercase());
    let adapter = match &wanted {
        Some(name) => pollster::block_on(instance.enumerate_adapters(backends))
            .into_iter()
            .find(|a| a.get_info().name.to_lowercase().contains(name))
            .expect("no adapter matches ADAPTER_NAME"),
        None => pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .expect("adapter"),
    };
    let path = match forced_path.as_deref() {
        Some("B") => TerrainPath::Pulling,
        Some("Bi") => TerrainPath::PullingIndexed,
        Some("C") => TerrainPath::Instanced,
        _ => TerrainPath::default_for(adapter.get_downlevel_capabilities().flags),
    };
    let frames: usize = std::env::var("FRAMES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let timestamps = frames > 0 && adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
    println!(
        "adapter: {} ({:?}), path {path:?}",
        adapter.get_info().name,
        adapter.get_info().backend
    );
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("screenshot"),
        required_features: if timestamps {
            wgpu::Features::TIMESTAMP_QUERY
        } else {
            wgpu::Features::empty()
        },
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .expect("device");

    // Textures: one fallback layer per block, same on every face.
    let names: Vec<&str> = (0..reg.len() as u16)
        .map(|i| {
            let name = reg.name(BlockStateId(i)).unwrap_or("missing");
            let key = fallback_key(name.split('[').next().unwrap_or(name));
            if FALLBACK_KEYS.contains(&key) {
                key
            } else {
                "missing"
            }
        })
        .collect();
    let mut keys: Vec<&str> = names.clone();
    keys.sort_unstable();
    keys.dedup();
    let set = fallback_set(keys.iter().copied()).expect("textures");
    let array = &set.arrays()[0];
    let layer_table: Vec<u32> = names
        .iter()
        .flat_map(|k| {
            let l = u32::from(set.location(k).map_or(0, |l| l.layer));
            [l; 6]
        })
        .collect();
    let mut renderer = TerrainRenderer::new(
        &device,
        &queue,
        path,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        &TerrainTextures {
            size: array.size(),
            layers: array.layers() as u32,
            levels: array.levels(),
        },
        &layer_table,
        8 << 20,
    );

    // Batches: 2×2×2 mesh chunks each.
    let mut batches: HashMap<IVec3, RenderBatch> = HashMap::new();
    for (m, quads) in loader.meshes() {
        let p = IVec3::new(m.x, m.y, m.z);
        let bpos = p.div_euclid(IVec3::splat(2));
        let l = (p - bpos * 2).as_uvec3();
        batches
            .entry(bpos)
            .or_default()
            .set_chunk(slot(l.x as usize, l.y as usize, l.z as usize), quads);
    }
    let upload = Instant::now();
    let mut packed = PackedBatch::default();
    for (pos, batch) in &batches {
        batch.pack_into(&mut packed);
        renderer
            .upload_batch(&queue, *pos, &packed)
            .expect("upload");
    }
    let mut sections = 0;
    for c in map.column_positions() {
        for i in 0..bounds.section_count as i32 {
            let sp = world::coords::SectionPos {
                x: c.x,
                y: bounds.min_section_y + i,
                z: c.z,
            };
            if let Some(l) = light.get(sp) {
                renderer
                    .set_section_light(&queue, IVec3::new(sp.x, sp.y, sp.z), l)
                    .expect("light");
                sections += 1;
            }
        }
    }
    println!(
        "uploaded {} non-empty batches, {sections} light sections in {:?}",
        batches.values().filter(|b| !b.is_empty()).count(),
        upload.elapsed()
    );

    let settings = QualityProfile::Default.settings();
    let far = (settings.render_distance.min(radius as u32) * 16) as f32;
    let sky = [0.55, 0.72, 0.95];
    let surface_y = map.height(8, 8).unwrap_or(64) as f32;
    let eye = Vec3::new(8.0, surface_y + 24.0, 8.0);
    let dir = Vec3::new(1.0, -0.35, 0.6);
    let aspect = WIDTH as f32 / HEIGHT as f32;
    let view_proj = |eye: Vec3, dir: Vec3| {
        perspective(70f32.to_radians(), aspect, 0.1, far + 64.0) * look_to(eye, dir, Vec3::Y)
    };

    // Culling statistics: surface and underground (cave culling share).
    let vis: HashMap<IVec3, ChunkVisibility> = loader
        .visibility()
        .iter()
        .map(|(m, v)| (IVec3::new(m.x, m.y, m.z), *v))
        .collect();
    let nonempty: Vec<IVec3> = loader
        .meshes()
        .iter()
        .filter(|(_, q)| !q.is_empty())
        .map(|(m, _)| IVec3::new(m.x, m.y, m.z))
        .collect();
    // An air cell in a cave below y = 0 near the center, if any.
    let cave = (-50..0)
        .flat_map(|y| (-24..24).flat_map(move |x| (-24..24).map(move |z| (x, y, z))))
        .find(|&(x, y, z)| {
            map.get_block(world::coords::BlockPos::new(x, y, z)) == Some(BlockStateId::AIR)
        })
        .map_or(Vec3::new(8.0, -20.0, 8.0), |(x, y, z)| {
            Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5)
        });
    for (name, eye, dir) in [
        ("surface", eye, dir),
        ("cave", cave, Vec3::new(1.0, 0.0, 0.3)),
    ] {
        let frustum = Frustum::from_view_proj(view_proj(eye, dir));
        let cam = (eye / 32.0).floor().as_ivec3();
        let mesh_radius = (far / 32.0).ceil() as i32;
        let t = Instant::now();
        let reach: HashSet<IVec3> = cave_visible(cam, &vis, Some(&frustum), mesh_radius, 32.0)
            .into_iter()
            .collect();
        let traversal = t.elapsed();
        let in_frustum: Vec<&IVec3> = nonempty
            .iter()
            .filter(|c| {
                let min = c.as_vec3() * 32.0;
                frustum.intersects(min, min + Vec3::splat(32.0))
            })
            .collect();
        let kept = in_frustum.iter().filter(|c| reach.contains(c)).count();
        let discarded = 1.0 - kept as f64 / in_frustum.len().max(1) as f64;
        println!(
            "{name} at {eye}: mesh chunks with geometry in frustum {}, after cave culling {kept} ({:.0}% discarded), traversal {traversal:?}",
            in_frustum.len(),
            discarded * 100.0
        );
    }

    // Render the surface view.
    let camera = CameraParams {
        view_proj: view_proj(eye, dir),
        position: eye,
        daylight: 1.0,
        fog: (far * settings.fog_start, far),
        sky,
    };
    let mut prepare_times = Vec::new();
    for _ in 0..20 {
        let t = Instant::now();
        renderer.prepare(&queue, &camera, None);
        prepare_times.push(t.elapsed());
    }
    prepare_times.sort();
    let stats = renderer.stats();
    println!(
        "prepare (CPU culling + draw list): median {:?}; {stats:?}",
        prepare_times[prepare_times.len() / 2]
    );

    let extent = wgpu::Extent3d {
        width: WIDTH,
        height: HEIGHT,
        depth_or_array_layers: 1,
    };
    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("color"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let color_view = color.create_view(&Default::default());
    let depth_view = depth.create_view(&Default::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(WIDTH * HEIGHT * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    if frames > 0 {
        timed_frames(
            &device,
            &queue,
            &mut renderer,
            &camera,
            &color_view,
            &depth_view,
            sky,
            frames,
            timestamps,
        );
    }
    let mut encoder = device.create_command_encoder(&Default::default());
    let record = Instant::now();
    record_pass(&mut encoder, &renderer, &color_view, &depth_view, sky, None);
    println!("pass recording: {:?}", record.elapsed());
    encoder.copy_texture_to_buffer(
        color.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 4),
                rows_per_image: Some(HEIGHT),
            },
        },
        extent,
    );
    let gpu = Instant::now();
    queue.submit([encoder.finish()]);
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.expect("map"));
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(120)),
        })
        .expect("poll");
    println!(
        "frame + readback: {:?} (software adapters are not representative)",
        gpu.elapsed()
    );
    let pixels = readback
        .slice(..)
        .get_mapped_range()
        .expect("mapped")
        .to_vec();
    write_bmp(&out, &pixels).expect("write");
    println!("saved {}", out.display());
}

fn record_pass(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &TerrainRenderer,
    color: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    sky: [f32; 3],
    queries: Option<&wgpu::QuerySet>,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("terrain"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: color,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: f64::from(sky[0]),
                    g: f64::from(sky[1]),
                    b: f64::from(sky[2]),
                    a: 1.0,
                }),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.0),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: queries.map(|q| wgpu::RenderPassTimestampWrites {
            query_set: q,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: Some(1),
        }),
        ..Default::default()
    });
    renderer.render(&mut pass);
}

/// Renders `frames` frames one at a time; prints CPU (prepare + recording)
/// and GPU (timestamp) medians and 95th/99th percentiles.
#[allow(clippy::too_many_arguments)]
fn timed_frames(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut TerrainRenderer,
    camera: &CameraParams,
    color: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    sky: [f32; 3],
    frames: usize,
    timestamps: bool,
) {
    let queries = timestamps.then(|| {
        device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("frame"),
            ty: wgpu::QueryType::Timestamp,
            count: 2,
        })
    });
    let resolve = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("resolve"),
        size: 16,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("timestamps"),
        size: 16,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let period = f64::from(queue.get_timestamp_period());
    let (mut cpu, mut gpu) = (Vec::new(), Vec::new());
    for _ in 0..frames {
        let t = Instant::now();
        renderer.prepare(queue, camera, None);
        let mut encoder = device.create_command_encoder(&Default::default());
        record_pass(&mut encoder, renderer, color, depth, sky, queries.as_ref());
        cpu.push(t.elapsed().as_secs_f64() * 1e3);
        if let Some(q) = &queries {
            encoder.resolve_query_set(q, 0..2, &resolve, 0);
            encoder.copy_buffer_to_buffer(&resolve, 0, &read, 0, 16);
        }
        queue.submit([encoder.finish()]);
        if queries.is_some() {
            read.slice(..)
                .map_async(wgpu::MapMode::Read, |r| r.expect("map"));
        }
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(60)),
            })
            .expect("poll");
        if queries.is_some() {
            let ticks: [u64; 2] =
                bytemuck_pair(&read.slice(..).get_mapped_range().expect("mapped"));
            read.unmap();
            gpu.push(ticks[1].saturating_sub(ticks[0]) as f64 * period / 1e6);
        }
    }
    // The first frame pays one-time driver and validation work; report it
    // apart from the steady-state statistics.
    println!(
        "first frame: CPU {:.3} ms, GPU {:.3} ms",
        cpu[0],
        gpu.first().copied().unwrap_or(f64::NAN)
    );
    if frames > 1 {
        cpu.remove(0);
        if !gpu.is_empty() {
            gpu.remove(0);
        }
    }
    let pct = |v: &mut Vec<f64>, p: f64| {
        v.sort_by(f64::total_cmp);
        v[((v.len() - 1) as f64 * p).round() as usize]
    };
    println!(
        "next {} frames, CPU prepare+record ms: median {:.3}, p95 {:.3}, p99 {:.3}",
        cpu.len(),
        pct(&mut cpu, 0.5),
        pct(&mut cpu, 0.95),
        pct(&mut cpu, 0.99)
    );
    if gpu.is_empty() {
        println!("GPU: no timestamp queries on this adapter");
    } else {
        println!(
            "GPU pass ms: median {:.3}, p95 {:.3}, p99 {:.3}",
            pct(&mut gpu, 0.5),
            pct(&mut gpu, 0.95),
            pct(&mut gpu, 0.99)
        );
    }
}

fn bytemuck_pair(bytes: &[u8]) -> [u64; 2] {
    let word = |i: usize| {
        let mut b = [0u8; 8];
        b.copy_from_slice(&bytes[i * 8..i * 8 + 8]);
        u64::from_le_bytes(b)
    };
    [word(0), word(1)]
}

/// 24-bit bottom-up BMP from RGBA rows (top-down).
fn write_bmp(path: &std::path::Path, rgba: &[u8]) -> std::io::Result<()> {
    let row = (WIDTH * 3).div_ceil(4) * 4;
    let size = 54 + row * HEIGHT;
    let mut b = Vec::with_capacity(size as usize);
    b.extend_from_slice(b"BM");
    b.extend_from_slice(&size.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&54u32.to_le_bytes());
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&(WIDTH as i32).to_le_bytes());
    b.extend_from_slice(&(HEIGHT as i32).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&24u16.to_le_bytes());
    b.extend_from_slice(&[0; 24]);
    for y in (0..HEIGHT).rev() {
        let start = b.len();
        for x in 0..WIDTH {
            let p = ((y * WIDTH + x) * 4) as usize;
            b.extend_from_slice(&[rgba[p + 2], rgba[p + 1], rgba[p]]);
        }
        b.resize(start + row as usize, 0);
    }
    std::fs::write(path, b)
}
