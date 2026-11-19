//! Offscreen B/C quad throughput and CPU command-recording benchmark.

use anyhow::{Context, Result, ensure};
use clap::{Parser, ValueEnum};
use render::{AdapterReport, RenderPath, select_path};
use serde::Serialize;
use std::io::{self, Write};
use std::num::NonZeroU64;
use std::ops::Range;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use wgpu::util::DeviceExt;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
const QUAD_BYTES: u64 = 8;

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Backend {
    Vulkan,
    Dx12,
    Gl,
}

impl Backend {
    fn mask(self) -> wgpu::Backends {
        match self {
            Self::Vulkan => wgpu::Backends::VULKAN,
            Self::Dx12 => wgpu::Backends::DX12,
            Self::Gl => wgpu::Backends::GL,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Path {
    Auto,
    B,
    C,
}

#[derive(Clone, Copy, Debug, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
enum Environment {
    Unknown,
    BareMetal,
    Vm,
}

#[derive(Debug, Parser)]
#[command(about = "Draw packed 8-byte quads offscreen and emit timing JSON")]
struct Args {
    #[arg(long, value_enum, default_value_t = Backend::Vulkan)]
    backend: Backend,
    /// Index within this backend, as printed by gpu-probe.
    #[arg(long, default_value_t = 0)]
    adapter: usize,
    #[arg(long, value_enum, default_value_t = Path::Auto)]
    path: Path,
    #[arg(long, default_value_t = 500_000, value_parser = clap::value_parser!(u32).range(1..=1_000_000))]
    quads: u32,
    #[arg(long, default_value_t = 600, value_parser = clap::value_parser!(u32).range(1..=10_000))]
    draws: u32,
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u32).range(1..=10_000))]
    frames: u32,
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u32).range(0..=10_000))]
    warmup: u32,
    /// CPU model, RAM, OS/driver versions and other machine details.
    #[arg(long, default_value = "unspecified")]
    machine: String,
    #[arg(long, value_enum, default_value_t = Environment::Unknown)]
    environment: Environment,
    /// Do not request TIMESTAMP_QUERY even if the adapter supports it.
    #[arg(long)]
    no_timestamps: bool,
}

#[derive(Debug, Serialize)]
struct Timing {
    mean: f64,
    min: f64,
    p50: f64,
    p95: f64,
    max: f64,
    samples: Vec<f64>,
}

impl Timing {
    fn from_samples(samples: Vec<f64>) -> Self {
        let mut sorted = samples.clone();
        sorted.sort_by(f64::total_cmp);
        let percentile = |percent: usize| sorted[(sorted.len() * percent).div_ceil(100) - 1];
        Self {
            mean: samples.iter().sum::<f64>() / samples.len() as f64,
            min: sorted[0],
            p50: percentile(50),
            p95: percentile(95),
            max: sorted[sorted.len() - 1],
            samples,
        }
    }
}

#[derive(Serialize)]
struct BenchReport {
    schema_version: u32,
    machine: String,
    environment: Environment,
    os: &'static str,
    architecture: &'static str,
    build_profile: &'static str,
    adapter: AdapterReport,
    path: RenderPath,
    quads: u32,
    quad_bytes: u64,
    vertex_invocations: u32,
    draws: u32,
    resolution: [u32; 2],
    warmup_frames: u32,
    measured_frames: u32,
    timestamps: &'static str,
    timestamp_period_ns: Option<f32>,
    gpu_geometry_ms: Option<Timing>,
    cpu_command_recording_ms: Timing,
    rendered_pixels: usize,
}

fn draw_ranges(quads: u32, draws: u32) -> Vec<Range<u32>> {
    (0..draws)
        .map(|i| {
            let start = (u64::from(i) * u64::from(quads) / u64::from(draws)) as u32;
            let end = (u64::from(i + 1) * u64::from(quads) / u64::from(draws)) as u32;
            start..end
        })
        .collect()
}

fn quad_data(ranges: &[Range<u32>]) -> Vec<[u32; 2]> {
    ranges
        .iter()
        .flat_map(|range| {
            range.clone().enumerate().map(|(local, global)| {
                let x = local as u32 % 64;
                let y = (local as u32 / 64) % 64;
                let z = (local as u32 / 4096) % 64;
                [x | (y << 6) | (z << 12), global & 65535]
            })
        })
        .collect()
}

fn shader_source(path: RenderPath) -> String {
    let vertex = match path {
        RenderPath::VertexPulling => include_str!("../quad-pulling.wgsl"),
        RenderPath::Instanced => include_str!("../quad-instance.wgsl"),
    };
    format!("{}\n{vertex}", include_str!("../quad-common.wgsl"))
}

fn wait(device: &wgpu::Device, submission: Option<wgpu::SubmissionIndex>) -> Result<()> {
    device.poll(wgpu::PollType::Wait {
        submission_index: submission,
        timeout: Some(Duration::from_secs(60)),
    })?;
    Ok(())
}

fn read_buffer(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Result<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
    wait(device, None)?;
    rx.recv()
        .context("buffer mapping callback did not arrive")??;
    let bytes = buffer.slice(..).get_mapped_range()?.to_vec();
    buffer.unmap();
    Ok(bytes)
}

fn benchmark(
    adapter: &wgpu::Adapter,
    path: RenderPath,
    args: &Args,
) -> Result<(BenchReport, Vec<u8>)> {
    ensure!(args.draws <= args.quads, "--draws must not exceed --quads");
    let caps = adapter.get_downlevel_capabilities();
    ensure!(
        path != RenderPath::VertexPulling || select_path(caps.flags) == path,
        "path B requires VERTEX_STORAGE; select --path c"
    );
    let timestamp_state = if args.no_timestamps {
        "disabled-by-flag"
    } else if adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
        "enabled"
    } else {
        "unsupported"
    };
    let timestamps = timestamp_state == "enabled";
    let mut limits = wgpu::Limits::downlevel_webgl2_defaults()
        .using_resolution(adapter.limits())
        .using_alignment(adapter.limits());
    if path == RenderPath::VertexPulling {
        limits.max_storage_buffers_per_shader_stage = 1;
        limits.max_storage_buffer_binding_size = u64::from(args.quads) * QUAD_BYTES;
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("quad-bench"),
        required_features: if timestamps {
            wgpu::Features::TIMESTAMP_QUERY
        } else {
            wgpu::Features::empty()
        },
        required_limits: limits,
        ..Default::default()
    }))?;
    let ranges = draw_ranges(args.quads, args.draws);
    let data = quad_data(&ranges);
    let quad_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("packed 8-byte quads"),
        contents: bytemuck::cast_slice(&data),
        usage: match path {
            RenderPath::VertexPulling => wgpu::BufferUsages::STORAGE,
            RenderPath::Instanced => wgpu::BufferUsages::VERTEX,
        },
    });
    let stride = u64::from(device.limits().min_uniform_buffer_offset_alignment).max(16);
    let columns = (f64::from(args.draws).sqrt().ceil()) as u32;
    let rows = args.draws.div_ceil(columns);
    let mut transforms = vec![0u8; (stride * u64::from(args.draws)) as usize];
    for i in 0..args.draws {
        let transform = [
            -1.0 + 2.0 * (i % columns) as f32 / columns as f32,
            -1.0 + 2.0 * (i / columns) as f32 / rows as f32,
            2.0 / (64.0 * columns as f32),
            2.0 / (64.0 * rows as f32),
        ];
        let offset = (u64::from(i) * stride) as usize;
        transforms[offset..offset + 16].copy_from_slice(bytemuck::bytes_of(&transform));
    }
    let uniforms = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("batch transforms"),
        contents: &transforms,
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let mut layout_entries = vec![wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: true,
            min_binding_size: NonZeroU64::new(16),
        },
        count: None,
    }];
    let mut binding_entries = vec![wgpu::BindGroupEntry {
        binding: 0,
        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: &uniforms,
            offset: 0,
            size: NonZeroU64::new(16),
        }),
    }];
    if path == RenderPath::VertexPulling {
        layout_entries.push(wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(QUAD_BYTES),
            },
            count: None,
        });
        binding_entries.push(wgpu::BindGroupEntry {
            binding: 1,
            resource: quad_buffer.as_entire_binding(),
        });
    }
    let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("quad bindings"),
        entries: &layout_entries,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("quad bindings"),
        layout: &bind_layout,
        entries: &binding_entries,
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("quad layout"),
        bind_group_layouts: &[Some(&bind_layout)],
        immediate_size: 0,
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("quad shader"),
        source: wgpu::ShaderSource::Wgsl(shader_source(path).into()),
    });
    let attributes = wgpu::vertex_attr_array![0 => Uint32x2];
    let instance_layout = [Some(wgpu::VertexBufferLayout {
        array_stride: QUAD_BYTES,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &attributes,
    })];
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("quad pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: if path == RenderPath::Instanced {
                &instance_layout
            } else {
                &[]
            },
        },
        primitive: wgpu::PrimitiveState {
            topology: match path {
                RenderPath::VertexPulling => wgpu::PrimitiveTopology::TriangleList,
                RenderPath::Instanced => wgpu::PrimitiveTopology::TriangleStrip,
            },
            ..Default::default()
        },
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let extent = wgpu::Extent3d {
        width: WIDTH,
        height: HEIGHT,
        depth_or_array_layers: 1,
    };
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen geometry target"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let timer = timestamps.then(|| {
        let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("geometry timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: 2,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("timestamp resolve"),
            size: 16,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("timestamp samples"),
            size: u64::from(args.frames) * 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        (queries, resolve, readback)
    });
    let mut cpu_samples = Vec::with_capacity(args.frames as usize);
    for frame in 0..args.warmup + args.frames {
        let measured = frame >= args.warmup;
        let start = Instant::now();
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("geometry"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                timestamp_writes: timer.as_ref().map(|(queries, _, _)| {
                    wgpu::RenderPassTimestampWrites {
                        query_set: queries,
                        beginning_of_pass_write_index: Some(0),
                        end_of_pass_write_index: Some(1),
                    }
                }),
                ..Default::default()
            });
            pass.set_pipeline(&pipeline);
            for (i, range) in ranges.iter().enumerate() {
                pass.set_bind_group(0, &bind_group, &[(i as u64 * stride) as u32]);
                match path {
                    RenderPath::VertexPulling => pass.draw(range.start * 6..range.end * 6, 0..1),
                    RenderPath::Instanced => {
                        pass.set_vertex_buffer(
                            0,
                            quad_buffer.slice(
                                u64::from(range.start) * QUAD_BYTES
                                    ..u64::from(range.end) * QUAD_BYTES,
                            ),
                        );
                        pass.draw(0..4, 0..range.end - range.start);
                    }
                }
            }
        }
        if measured && let Some((queries, resolve, readback)) = &timer {
            encoder.resolve_query_set(queries, 0..2, resolve, 0);
            encoder.copy_buffer_to_buffer(
                resolve,
                0,
                readback,
                u64::from(frame - args.warmup) * 16,
                16,
            );
        }
        let commands = encoder.finish();
        let cpu_ms = start.elapsed().as_secs_f64() * 1000.0;
        if measured {
            cpu_samples.push(cpu_ms);
        }
        let submission = queue.submit([commands]);
        wait(&device, Some(submission))?;
    }
    let gpu_geometry_ms = if let Some((_, _, readback)) = &timer {
        let bytes = read_buffer(&device, readback)?;
        let samples = bytes
            .chunks_exact(16)
            .map(|pair| {
                let begin = u64::from_le_bytes(pair[..8].try_into().expect("8-byte timestamp"));
                let end = u64::from_le_bytes(pair[8..].try_into().expect("8-byte timestamp"));
                end.wrapping_sub(begin) as f64 * f64::from(queue.get_timestamp_period())
                    / 1_000_000.0
            })
            .collect();
        Some(Timing::from_samples(samples))
    } else {
        None
    };
    let image_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("untimed image verification"),
        size: u64::from(WIDTH) * u64::from(HEIGHT) * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &image_buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 4),
                rows_per_image: Some(HEIGHT),
            },
        },
        extent,
    );
    wait(&device, Some(queue.submit([encoder.finish()])))?;
    let pixels = read_buffer(&device, &image_buffer)?;
    let rendered_pixels = pixels
        .chunks_exact(4)
        .filter(|pixel| pixel[..3] != [0, 0, 0])
        .count();
    ensure!(rendered_pixels > 0, "geometry produced a blank image");
    let report = BenchReport {
        schema_version: 1,
        machine: args.machine.clone(),
        environment: args.environment,
        os: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
        build_profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        adapter: AdapterReport::from_adapter(adapter, args.adapter),
        path,
        quads: args.quads,
        quad_bytes: QUAD_BYTES,
        vertex_invocations: args.quads
            * if path == RenderPath::VertexPulling {
                6
            } else {
                4
            },
        draws: args.draws,
        resolution: [WIDTH, HEIGHT],
        warmup_frames: args.warmup,
        measured_frames: args.frames,
        timestamps: timestamp_state,
        timestamp_period_ns: timestamps.then(|| queue.get_timestamp_period()),
        gpu_geometry_ms,
        cpu_command_recording_ms: Timing::from_samples(cpu_samples),
        rendered_pixels,
    };
    Ok((report, pixels))
}

fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(args.draws <= args.quads, "--draws must not exceed --quads");
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: args.backend.mask(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapters = pollster::block_on(instance.enumerate_adapters(args.backend.mask()));
    let adapter = adapters
        .get(args.adapter)
        .context("adapter index unavailable for this backend; run gpu-probe")?;
    let path = match args.path {
        Path::Auto => select_path(adapter.get_downlevel_capabilities().flags),
        Path::B => RenderPath::VertexPulling,
        Path::C => RenderPath::Instanced,
    };
    let (report, _) = benchmark(adapter, path, &args)?;
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &report)?;
    writeln!(stdout)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_ranges_cover_every_quad_once() {
        for (quads, draws) in [(1, 1), (7, 3), (500_000, 600), (1_000_000, 10_000)] {
            let ranges = draw_ranges(quads, draws);
            assert_eq!(ranges.len(), draws as usize);
            assert_eq!(ranges[0].start, 0);
            assert_eq!(ranges[ranges.len() - 1].end, quads);
            for pair in ranges.windows(2) {
                assert_eq!(pair[0].end, pair[1].start);
            }
            assert!(ranges.iter().all(|range| !range.is_empty()));
        }
    }

    #[test]
    fn data_is_eight_bytes_and_uses_the_specification_layout() {
        let data = quad_data(&draw_ranges(500_000, 600));
        assert_eq!(data.len(), 500_000);
        assert_eq!(std::mem::size_of_val(&data[0]), 8);
        for (i, quad) in data.iter().enumerate() {
            assert_eq!(quad[0] >> 18, 0);
            assert_eq!(quad[1], i as u32 & 65535);
        }
        assert_eq!(data[64][0], 1 << 6);
    }

    #[test]
    fn both_shaders_validate_without_a_gpu() -> Result<()> {
        for path in [RenderPath::VertexPulling, RenderPath::Instanced] {
            let module = naga::front::wgsl::parse_str(&shader_source(path))?;
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::empty(),
            )
            .validate(&module)?;
        }
        Ok(())
    }

    #[test]
    fn fallback_shader_has_no_storage_binding() {
        assert!(!shader_source(RenderPath::Instanced).contains("var<storage"));
        assert!(shader_source(RenderPath::VertexPulling).contains("var<storage"));
    }

    #[test]
    fn timing_summary_uses_nearest_rank_percentiles() {
        let timing = Timing::from_samples(vec![3.0, 1.0, 4.0, 2.0]);
        assert_eq!(timing.mean, 2.5);
        assert_eq!(timing.min, 1.0);
        assert_eq!(timing.p50, 2.0);
        assert_eq!(timing.p95, 4.0);
        assert_eq!(timing.max, 4.0);
        assert_eq!(timing.samples, [3.0, 1.0, 4.0, 2.0]);
    }

    #[test]
    fn cli_rejects_zero_and_excessive_counts() {
        for (option, value) in [
            ("--quads", "0"),
            ("--quads", "1000001"),
            ("--draws", "0"),
            ("--frames", "0"),
            ("--warmup", "10001"),
        ] {
            assert!(Args::try_parse_from(["quad-bench", option, value]).is_err());
        }
    }

    #[test]
    fn cli_parses_no_timestamps_flag() {
        let default = Args::try_parse_from(["quad-bench"]).expect("default args parse");
        assert!(!default.no_timestamps);
        let flagged = Args::try_parse_from(["quad-bench", "--no-timestamps"]).expect("flag parses");
        assert!(flagged.no_timestamps);
        assert!(Args::try_parse_from(["quad-bench", "--no-timestamps=yes"]).is_err());
    }

    #[test]
    #[ignore = "requires an adapter with vertex storage; run manually on hardware"]
    fn gpu_paths_render_identical_nonblank_images() -> Result<()> {
        let args = Args::parse_from([
            "quad-bench",
            "--quads",
            "1024",
            "--draws",
            "3",
            "--frames",
            "2",
            "--warmup",
            "1",
        ]);
        let descriptor = wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::GL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        }
        .with_env();
        let backends = descriptor.backends;
        let instance = wgpu::Instance::new(descriptor);
        let adapters = pollster::block_on(instance.enumerate_adapters(backends));
        let name_filter = std::env::var("QUAD_BENCH_ADAPTER_NAME").ok();
        let adapter = adapters
            .iter()
            .find(|adapter| {
                select_path(adapter.get_downlevel_capabilities().flags) == RenderPath::VertexPulling
                    && name_filter.as_ref().is_none_or(|filter| {
                        adapter
                            .get_info()
                            .name
                            .to_lowercase()
                            .contains(&filter.to_lowercase())
                    })
            })
            .with_context(|| match &name_filter {
                Some(filter) => format!(
                    "no adapter with vertex storage matches QUAD_BENCH_ADAPTER_NAME={filter:?}"
                ),
                None => "an adapter with vertex storage is required".to_string(),
            })?;
        let (b, image_b) = benchmark(adapter, RenderPath::VertexPulling, &args)?;
        let (c, image_c) = benchmark(adapter, RenderPath::Instanced, &args)?;
        assert_eq!(image_b, image_c);
        assert!(b.rendered_pixels > 0);
        assert_eq!(b.rendered_pixels, c.rendered_pixels);
        assert_eq!(b.cpu_command_recording_ms.samples.len(), 2);
        assert_eq!(
            b.gpu_geometry_ms.is_some(),
            adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY)
        );
        Ok(())
    }
}
