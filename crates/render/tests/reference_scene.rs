//! T1-21: the render reference scene and golden-image tests
//! (`docs/process/testing-and-benchmarks.md`, "Render reference scene").
//!
//! A fixed scene is built in code (no world file, no Mojang assets, fallback
//! textures only), lit and meshed by the real `light` and `mesh` code, and
//! rendered offscreen from fixed cameras on every terrain path (B, Bi, C)
//! and with the example shader pack. Each image is compared with a golden
//! PNG in `tests/golden/` with a per-pixel tolerance and a maximum share of
//! differing pixels; on failure the actual image and a diff image are written
//! to `target/render-diffs/` (uploaded by CI).
//!
//! Refresh the golden images on purpose only:
//! `UPDATE_GOLDEN=1 cargo test -p render --test reference_scene`.
//!
//! Needs a GPU adapter; software Vulkan (lavapipe) is enough and is what the
//! golden images come from. Without an adapter the test skips, unless
//! `REQUIRE_GPU=1` (set in CI) makes that a failure.
//!
//! On a hardware GPU (the owner's laptop) rasterization may differ slightly
//! from software Vulkan, so mismatches are reported and their images written,
//! but fail the test only with `REFERENCE_STRICT=1`. `REFERENCE_OUT=<dir>`
//! writes every rendered image there (for comparing GPUs by eye).
//! `WGPU_BACKEND` and `ADAPTER_NAME` pick the backend and adapter.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use assets::{FALLBACK_KEYS, fallback_key, fallback_set};
use glam::{IVec3, Mat4, Vec3};
use light::{WorldLight, light_column, props_from_registry};
use mesh::batch::{RenderBatch, slot};
use mesh::{MeshChunkBuilder, PackedBatch, SectionNeighborhood};
use render::terrain::camera::{look_to, perspective};
use render::terrain::post::PostPass;
use render::terrain::renderer::{
    CameraParams, DEPTH_FORMAT, TerrainPath, TerrainRenderer, TerrainTextures,
};
use world::coords::{BlockPos, MeshChunkPos, SectionPos};
use world::{BlockStateId, Column, ColumnPos, Registry, Section, WorldBounds, WorldMap};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const SKY: [f32; 3] = [0.55, 0.72, 0.95];

/// A channel differing by more than this makes a pixel "different".
const CHANNEL_TOLERANCE: u8 = 8;
/// At most this share of pixels may differ.
const MAX_DIFFERENT: f64 = 0.005;

// ---------------------------------------------------------------- the scene

/// The registry: the shared blocks plus the advanced overworld's (spruce,
/// birch, snow), so the material stand shows every fallback texture.
fn registry() -> Arc<Registry> {
    let (mods, _) = world::modset::source_tree();
    let set =
        world::ModSet::resolve("reference", &["core", "overworld_plus"], &mods).expect("mods");
    Arc::new(
        set.registry_builder()
            .expect("blocks")
            .build()
            .expect("registry"),
    )
}

/// The scene inside x, z in -32..32 (two mesh chunks, one render batch
/// border and one mesh chunk border at x = 0 and z = 0):
///
/// - a grass floor at y = 63 over stone;
/// - a material stand: one block of every kind on a row at z = -20;
/// - a "white furnace": a closed snow room lit by a grid of glowstone in its
///   ceiling (an even light: any seam or tint inside is a bug);
/// - a light-level ramp: a closed corridor with glowstone at one end, so the
///   floor shows levels 15 → 0;
/// - a sky-light shaft: a closed box with a 1×1 hole in its roof;
/// - (the white furnace, shaft box and checker are cut by the x = 0 / z = 0
///   and x = 32 borders of mesh chunks and render batches)
/// - a water pool, 6 deep;
/// - a checker of planks and cobblestone across the x = 0 / z = 0 borders.
fn build_scene(reg: &Arc<Registry>) -> WorldMap {
    let bounds = WorldBounds::default();
    let mut map = WorldMap::new(Arc::clone(reg), bounds);
    for x in -3..=3 {
        for z in -3..=3 {
            map.insert_column(Column::new(ColumnPos { x, z }, bounds))
                .expect("column");
        }
    }
    let id = |n: &str| reg.state_id(n).unwrap_or_else(|| panic!("{n}"));
    let mut set = |x: i32, y: i32, z: i32, b: BlockStateId| {
        map.set_block(BlockPos::new(x, y, z), b).expect("set");
    };
    let (stone, grass, snow, glow) = (
        id("core:stone"),
        id("core:grass_block"),
        id("overworld_plus:snow_block"),
        id("core:glowstone"),
    );
    let air = BlockStateId::AIR;
    // Floor over the whole loaded area (so light and meshes see neighbors).
    for x in -48..48 {
        for z in -48..48 {
            for y in 56..63 {
                set(x, y, z, stone);
            }
            set(x, 63, z, grass);
        }
    }
    // Material stand: every block except air and the engine's `missing`.
    let mut x = -28;
    for i in 2..reg.len() as u16 {
        let b = BlockStateId(i);
        let name = reg.name(b).unwrap_or("");
        if name.contains('[') {
            continue; // one state per block
        }
        set(x, 64, -20, stone);
        set(x, 65, -20, b);
        x += 3;
    }
    // White furnace: snow box x 4..=14, z -12..=-2, y 63..=72.
    for x in 4..=14 {
        for z in -12..=-2 {
            for y in 63..=72 {
                let wall = x == 4 || x == 14 || z == -12 || z == -2 || y == 63 || y == 72;
                set(x, y, z, if wall { snow } else { air });
            }
        }
    }
    for x in (6..=12).step_by(3) {
        for z in (-10..=-4).step_by(3) {
            set(x, 72, z, glow);
        }
    }
    // Light ramp: corridor x -28..=-11, interior z 8..=10, y 64..=65.
    for x in -28..=-11 {
        for z in 7..=11 {
            for y in 64..=66 {
                let wall = x == -28 || x == -11 || z == 7 || z == 11 || y == 66;
                set(x, y, z, if wall { stone } else { air });
            }
        }
    }
    set(-27, 64, 9, glow);
    // Sky shaft: box x 14..=30, z 6..=22, roof at y = 70 with a 1×1 hole
    // above (22, 14): a spot of sky light 15 fading to the dark corners.
    for x in 14..=30 {
        for z in 6..=22 {
            for y in 64..=70 {
                let wall = x == 14 || x == 30 || z == 6 || z == 22 || y == 70;
                set(x, y, z, if wall { stone } else { air });
            }
        }
    }
    set(22, 70, 14, air);
    // Water pool x -10..=-2, z 20..=28, y 57..=63.
    let water = id("core:water");
    for x in -10..=-2 {
        for z in 20..=28 {
            for y in 57..=63 {
                set(x, y, z, water);
            }
            set(x, 56, z, stone);
        }
    }
    // A checker across the batch and chunk borders at x = 0 and z = 0.
    let (planks, cobble) = (id("core:oak_planks"), id("core:cobblestone"));
    for x in -3i32..3 {
        for z in -3..3 {
            let b = if (x + z).rem_euclid(2) == 0 {
                planks
            } else {
                cobble
            };
            set(x, 64, z, b);
            if x == -1 || x == 0 {
                set(x, 65, z, b);
            }
        }
    }
    map
}

/// Fixed cameras: name, eye, look direction.
fn cameras() -> Vec<(&'static str, Vec3, Vec3)> {
    vec![
        (
            "overview",
            Vec3::new(-30.0, 96.0, 34.0),
            Vec3::new(1.0, -0.8, -1.0),
        ),
        (
            "material_stand",
            Vec3::new(0.0, 70.0, -8.0),
            Vec3::new(0.0, -0.25, -1.0),
        ),
        (
            "white_furnace",
            Vec3::new(5.5, 66.5, -3.5),
            Vec3::new(1.0, 0.1, -0.8),
        ),
        (
            "light_ramp",
            Vec3::new(-12.5, 65.6, 9.5),
            Vec3::new(-1.0, -0.15, 0.0),
        ),
        (
            "sky_shaft",
            Vec3::new(15.5, 68.5, 7.5),
            Vec3::new(1.0, -0.55, 1.0),
        ),
        (
            "chunk_border",
            Vec3::new(6.0, 70.0, 6.0),
            Vec3::new(-1.0, -1.0, -1.0),
        ),
        (
            "underwater",
            Vec3::new(-3.0, 60.0, 21.0),
            Vec3::new(-1.0, -0.05, 1.0),
        ),
    ]
}

/// Light and meshes of the scene, ready for any renderer.
struct Prepared {
    batches: Vec<(IVec3, PackedBatch)>,
    lights: Vec<(IVec3, light::SectionLight)>,
    layer_table: Vec<u32>,
    texture_size: u32,
    texture_layers: usize,
    texture_levels: Vec<Vec<u8>>,
}

fn prepare_scene() -> Prepared {
    let reg = registry();
    let map = build_scene(&reg);
    let props = props_from_registry(&reg);
    let mut lights = WorldLight::new();
    for x in -2..=2 {
        for z in -2..=2 {
            light_column(&map, &mut lights, ColumnPos { x, z }, &props).expect("light");
        }
    }
    let bounds = map.bounds();
    let mut light_out = Vec::new();
    for x in -2..=2 {
        for z in -2..=2 {
            for i in 0..bounds.section_count as i32 {
                let sp = SectionPos {
                    x,
                    y: bounds.min_section_y + i,
                    z,
                };
                if let Some(l) = lights.get(sp) {
                    light_out.push((IVec3::new(sp.x, sp.y, sp.z), l.clone()));
                }
            }
        }
    }
    // Mesh chunks x, z in -1..=0 (blocks -32..32), y 1..=2 (blocks 32..96).
    let mut batches: HashMap<IVec3, RenderBatch> = HashMap::new();
    for mx in -1..=0 {
        for mz in -1..=0 {
            for my in 1..=2 {
                let m = MeshChunkPos {
                    x: mx,
                    y: my,
                    z: mz,
                };
                let n = SectionNeighborhood::from_fn(m, |sp| {
                    map.column(ColumnPos { x: sp.x, z: sp.z })
                        .and_then(|c| c.section(sp.y))
                        .cloned()
                        .unwrap_or_else(Section::new)
                });
                let quads = MeshChunkBuilder::new().build(&n, |b| reg.props(b).transparent);
                let p = IVec3::new(mx, my, mz);
                let bpos = p.div_euclid(IVec3::splat(2));
                let l = (p - bpos * 2).as_uvec3();
                batches
                    .entry(bpos)
                    .or_default()
                    .set_chunk(slot(l.x as usize, l.y as usize, l.z as usize), &quads);
            }
        }
    }
    let mut packed_batches: Vec<(IVec3, PackedBatch)> = batches
        .into_iter()
        .map(|(pos, b)| {
            let mut packed = PackedBatch::default();
            b.pack_into(&mut packed);
            (pos, packed)
        })
        .collect();
    packed_batches.sort_by_key(|(p, _)| (p.x, p.y, p.z));
    // Textures: one fallback layer per block, the same on every face.
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
    let mut keys = names.clone();
    keys.sort_unstable();
    keys.dedup();
    let tex = fallback_set(keys.iter().copied()).expect("textures");
    let array = &tex.arrays()[0];
    let layer_table = names
        .iter()
        .flat_map(|k| [u32::from(tex.location(k).map_or(0, |l| l.layer)); 6])
        .collect();
    Prepared {
        batches: packed_batches,
        lights: light_out,
        layer_table,
        texture_size: array.size(),
        texture_layers: array.layers(),
        texture_levels: array.levels().to_vec(),
    }
}

// ---------------------------------------------------------------- rendering

/// The adapter: `ADAPTER_NAME` (part of the name) or the default one.
fn device() -> Option<(wgpu::Device, wgpu::Queue, String)> {
    let desc = wgpu::InstanceDescriptor::new_without_display_handle().with_env();
    let backends = desc.backends;
    let instance = wgpu::Instance::new(desc);
    let adapter = match std::env::var("ADAPTER_NAME").ok().map(|n| n.to_lowercase()) {
        Some(name) => pollster::block_on(instance.enumerate_adapters(backends))
            .into_iter()
            .find(|a| a.get_info().name.to_lowercase().contains(&name))?,
        None => {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .ok()?
        }
    };
    let name = format!(
        "{} ({:?})",
        adapter.get_info().name,
        adapter.get_info().backend
    );
    SOFTWARE.with(|s| {
        s.set(matches!(
            adapter.get_info().device_type,
            wgpu::DeviceType::Cpu
        ))
    });
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("reference scene"),
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .ok()?;
    Some((device, queue, name))
}

thread_local! {
    /// Whether the adapter of this test is a software one (golden authority).
    static SOFTWARE: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// Whether a mismatch fails the test: always on software Vulkan, on hardware
/// only with `REFERENCE_STRICT=1`.
fn strict() -> bool {
    SOFTWARE.with(std::cell::Cell::get) || std::env::var_os("REFERENCE_STRICT").is_some()
}

/// Sorts a comparison result into failures (strict) or printed warnings.
fn record(result: Result<Diff, String>, failures: &mut Vec<String>) -> Option<Diff> {
    match result {
        Ok(d) => Some(d),
        Err(e) if strict() => {
            failures.push(e);
            None
        }
        Err(e) => {
            println!("warning (hardware GPU, not strict): {e}");
            None
        }
    }
}

/// A shader pack: terrain hooks, post code and option defaults.
struct Pack {
    shade: String,
    post: String,
    options: [f32; 8],
}

fn warm_glow() -> Pack {
    let dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples/shaderpacks/warm-glow");
    let read = |f: &str| std::fs::read_to_string(dir.join(f)).expect("pack file");
    let manifest: toml::Table = read("shaderpack.toml").parse().expect("manifest");
    let mut options = [0f32; 8];
    if let Some(list) = manifest.get("option").and_then(|o| o.as_array()) {
        for (i, o) in list.iter().take(8).enumerate() {
            options[i] = o
                .get("default")
                .and_then(toml::Value::as_float)
                .unwrap_or(0.0) as f32;
        }
    }
    Pack {
        shade: read("shade.wgsl"),
        post: read("post.wgsl"),
        options,
    }
}

struct Target {
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    scene_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    readback: wgpu::Buffer,
}

fn target(device: &wgpu::Device) -> Target {
    let extent = wgpu::Extent3d {
        width: WIDTH,
        height: HEIGHT,
        depth_or_array_layers: 1,
    };
    let tex = |label, format, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
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
        "color",
        FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let scene = tex(
        "scene",
        FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    );
    let depth = tex(
        "depth",
        DEPTH_FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    );
    Target {
        color_view: color.create_view(&Default::default()),
        scene_view: scene.create_view(&Default::default()),
        depth_view: depth.create_view(&Default::default()),
        color,
        readback: device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(WIDTH * HEIGHT * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        }),
    }
}

fn renderer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: TerrainPath,
    scene: &Prepared,
) -> TerrainRenderer {
    let mut r = TerrainRenderer::new(
        device,
        queue,
        path,
        FORMAT,
        &TerrainTextures {
            size: scene.texture_size,
            layers: scene.texture_layers as u32,
            levels: &scene.texture_levels,
        },
        &scene.layer_table,
        8 << 20,
    );
    for (pos, packed) in &scene.batches {
        r.upload_batch(queue, *pos, packed).expect("upload");
    }
    for (sp, l) in &scene.lights {
        r.set_section_light(queue, *sp, l).expect("light");
    }
    r
}

/// Renders one camera; returns RGB8 pixels, top row first.
fn shoot(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    r: &mut TerrainRenderer,
    post: Option<&PostPass>,
    t: &Target,
    eye: Vec3,
    dir: Vec3,
) -> Vec<u8> {
    let view_proj: Mat4 = perspective(
        70f32.to_radians(),
        WIDTH as f32 / HEIGHT as f32,
        0.05,
        256.0,
    ) * look_to(eye, dir.normalize(), Vec3::Y);
    r.prepare(
        queue,
        &CameraParams {
            view_proj,
            position: eye,
            daylight: 1.0,
            fog: (160.0, 240.0),
            sky: SKY,
        },
        None,
    );
    let mut enc = device.create_command_encoder(&Default::default());
    {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("terrain"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: if post.is_some() {
                    &t.scene_view
                } else {
                    &t.color_view
                },
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(SKY[0]),
                        g: f64::from(SKY[1]),
                        b: f64::from(SKY[2]),
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &t.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        r.render(&mut pass);
    }
    if let Some(p) = post {
        p.render(
            device,
            &mut enc,
            &t.scene_view,
            &t.depth_view,
            &t.color_view,
        );
    }
    enc.copy_texture_to_buffer(
        t.color.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &t.readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 4),
                rows_per_image: Some(HEIGHT),
            },
        },
        wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([enc.finish()]);
    t.readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.expect("map"));
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(120)),
        })
        .expect("poll");
    let rgba = t
        .readback
        .slice(..)
        .get_mapped_range()
        .expect("mapped")
        .to_vec();
    t.readback.unmap();
    rgba.chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect()
}

// ---------------------------------------------------------------- golden images

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn diff_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-diffs")
}

fn write_png(path: &Path, rgb: &[u8]) {
    let file = std::fs::File::create(path).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), WIDTH, HEIGHT);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .expect("png header")
        .write_image_data(rgb)
        .expect("png data");
}

fn read_png(path: &Path) -> Option<Vec<u8>> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    (info.width == WIDTH && info.height == HEIGHT && info.color_type == png::ColorType::Rgb)
        .then(|| buf[..info.buffer_size()].to_vec())
}

/// How two images differ.
#[derive(Debug)]
struct Diff {
    different: usize,
    max_channel: u8,
}

impl Diff {
    fn share(&self) -> f64 {
        self.different as f64 / f64::from(WIDTH * HEIGHT)
    }

    fn passes(&self) -> bool {
        self.share() <= MAX_DIFFERENT
    }
}

fn compare(a: &[u8], b: &[u8]) -> (Diff, Vec<u8>) {
    let mut d = Diff {
        different: 0,
        max_channel: 0,
    };
    let mut image = Vec::with_capacity(a.len());
    for (pa, pb) in a.chunks_exact(3).zip(b.chunks_exact(3)) {
        let m = (0..3).map(|i| pa[i].abs_diff(pb[i])).max().unwrap_or(0);
        d.max_channel = d.max_channel.max(m);
        if m > CHANNEL_TOLERANCE {
            d.different += 1;
            image.extend_from_slice(&[255, 0, 255]);
        } else {
            // The golden image, dimmed, as the background of the diff.
            image.extend(pb.iter().map(|v| v / 3));
        }
    }
    (d, image)
}

/// Compares with the golden image `name` (or writes it with UPDATE_GOLDEN);
/// `label` names the artifacts. Returns the difference, or an error message
/// with the artifact paths.
fn check(name: &str, label: &str, rgb: &[u8]) -> Result<Diff, String> {
    let golden = golden_dir().join(format!("{name}.png"));
    if let Some(dir) = std::env::var_os("REFERENCE_OUT") {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).expect("REFERENCE_OUT");
        write_png(&dir.join(format!("{label}.png")), rgb);
    }
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        write_png(&golden, rgb);
        return Ok(Diff {
            different: 0,
            max_channel: 0,
        });
    }
    let Some(expected) = read_png(&golden) else {
        return Err(format!(
            "{name}: no golden image {} (create it on purpose with UPDATE_GOLDEN=1)",
            golden.display()
        ));
    };
    let (d, diff) = compare(rgb, &expected);
    if d.passes() {
        return Ok(d);
    }
    let dir = diff_dir();
    std::fs::create_dir_all(&dir).expect("diff dir");
    let (actual, diff_path) = (
        dir.join(format!("{label}-actual.png")),
        dir.join(format!("{label}-diff.png")),
    );
    write_png(&actual, rgb);
    write_png(&diff_path, &diff);
    Err(format!(
        "{label}: {:.2}% of pixels differ by more than {CHANNEL_TOLERANCE} (max channel difference {}; allowed {:.2}%). \
         Actual: {}, diff (magenta = different): {}",
        d.share() * 100.0,
        d.max_channel,
        MAX_DIFFERENT * 100.0,
        actual.display(),
        diff_path.display()
    ))
}

// ---------------------------------------------------------------- tests

fn gpu_or_skip() -> Option<(wgpu::Device, wgpu::Queue, String)> {
    match device() {
        Some(d) => Some(d),
        None if std::env::var_os("REQUIRE_GPU").is_some() => {
            panic!("REQUIRE_GPU is set but no GPU adapter was found (install software Vulkan)")
        }
        None => {
            eprintln!("skipped: no GPU adapter");
            None
        }
    }
}

#[test]
fn reference_scene_matches_golden_images_on_every_path() {
    let Some((device, queue, adapter)) = gpu_or_skip() else {
        return;
    };
    println!("adapter: {adapter}");
    let scene = prepare_scene();
    let t = target(&device);
    let mut failures = Vec::new();
    for path in [
        TerrainPath::PullingIndexed,
        TerrainPath::Pulling,
        TerrainPath::Instanced,
    ] {
        let mut r = renderer(&device, &queue, path, &scene);
        for (name, eye, dir) in cameras() {
            let rgb = shoot(&device, &queue, &mut r, None, &t, eye, dir);
            // One golden image per camera (written from Bi): every path
            // must match it.
            if path != TerrainPath::PullingIndexed && std::env::var_os("UPDATE_GOLDEN").is_some() {
                continue;
            }
            if let Some(d) = record(
                check(name, &format!("{name}-{path:?}"), &rgb),
                &mut failures,
            ) {
                println!(
                    "{path:?} {name}: {:.3}% pixels differ, max channel difference {}",
                    d.share() * 100.0,
                    d.max_channel
                );
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn shader_pack_has_its_own_golden_images() {
    let Some((device, queue, _)) = gpu_or_skip() else {
        return;
    };
    let scene = prepare_scene();
    let pack = warm_glow();
    let t = target(&device);
    let mut r = renderer(&device, &queue, TerrainPath::PullingIndexed, &scene);
    r.set_shader_pack(&device, Some(&pack.shade))
        .expect("pack terrain hooks");
    r.set_pack_params(&queue, &pack.options, 0.0);
    let post = PostPass::new(&device, FORMAT, &pack.post).expect("pack post");
    post.set_params(&queue, &pack.options, 0.0);
    let mut failures = Vec::new();
    for (name, eye, dir) in cameras() {
        let rgb = shoot(&device, &queue, &mut r, Some(&post), &t, eye, dir);
        let golden = format!("warm-glow-{name}");
        record(check(&golden, &golden, &rgb), &mut failures);
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// The comparison is not blind: a one-line change of the surface shading
/// (through a shader pack, the same code path as the terrain shader) makes
/// the white furnace fail with a readable message and artifacts.
#[test]
fn a_one_line_shading_change_fails_the_comparison() {
    let Some((device, queue, _)) = gpu_or_skip() else {
        return;
    };
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        return;
    }
    let scene = prepare_scene();
    let t = target(&device);
    let mut r = renderer(&device, &queue, TerrainPath::PullingIndexed, &scene);
    // The built-in surface shading (hooks::DEFAULT_SURFACE), 30% darker.
    r.set_shader_pack(
        &device,
        Some(
            "fn pack_surface(s: Surface) -> vec3<f32> {
    let brightness = pack_light_curve(max(s.block_light, s.sky_light * s.daylight));
    let n = s.normal;
    let shade = select(select(0.8, 0.6, n.x != 0.0), select(0.5, 1.0, n.y > 0.0), n.y != 0.0);
    return s.albedo.rgb * brightness * shade * 0.7;
}
",
        ),
    )
    .expect("pack");
    let (name, eye, dir) = cameras()
        .into_iter()
        .find(|c| c.0 == "white_furnace")
        .expect("camera");
    let rgb = shoot(&device, &queue, &mut r, None, &t, eye, dir);
    let err = check(name, "self-test-darker", &rgb).expect_err("a darker shading must not pass");
    println!("{err}");
    assert!(err.contains("of pixels differ") && err.contains("diff (magenta = different)"));
}
