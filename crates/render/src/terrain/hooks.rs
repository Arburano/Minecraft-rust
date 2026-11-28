//! Shader-pack hooks for the terrain shader (ADR-0014, version 1). A pack's
//! `shade.wgsl` may define any of [`HOOKS`]; the engine supplies the rest.
//! Geometry (quad decoding, culling, draw submission) stays the engine's.
//!
//! What pack code can use (all documented in `docs/modding/packs.md`):
//! `struct Surface { albedo, normal, world, block_light, sky_light, daylight }`,
//! `camera` (view-projection, eye position, fog, sky color) and
//! `pack_option(i) -> f32` / `pack_time() -> f32` from the pack uniform.

use crate::terrain::renderer::TerrainPath;

/// Hook interface version. Packs state the version they target.
pub const HOOK_VERSION: u32 = 1;

/// Function names a pack may provide.
pub const HOOKS: [&str; 3] = ["pack_light_curve", "pack_surface", "pack_fog"];

/// Number of float options a pack can expose.
pub const MAX_OPTIONS: usize = 8;

const DEFAULT_LIGHT_CURVE: &str = "
fn pack_light_curve(level: f32) -> f32 {
    return 0.05 + 0.95 * pow(0.8, 15.0 - level);
}
";

const DEFAULT_SURFACE: &str = "
fn pack_surface(s: Surface) -> vec3<f32> {
    let level = max(s.block_light, s.sky_light * s.daylight);
    let brightness = pack_light_curve(level);
    // Fixed per-direction shade, the classic look.
    let n = s.normal;
    let shade = select(select(0.8, 0.6, n.x != 0.0), select(0.5, 1.0, n.y > 0.0), n.y != 0.0);
    return s.albedo.rgb * brightness * shade;
}
";

const DEFAULT_FOG: &str = "
fn pack_fog(rgb: vec3<f32>, world: vec3<f32>, eye: vec3<f32>, sky: vec3<f32>, start: f32, end: f32) -> vec3<f32> {
    let d = distance(world, eye);
    let fog = clamp((d - start) / max(end - start, 1.0), 0.0, 1.0);
    return mix(rgb, sky, fog);
}
";

/// Whether `code` defines function `name`.
#[must_use]
pub fn defines(code: &str, name: &str) -> bool {
    code.contains(&format!("fn {name}(")) || code.contains(&format!("fn {name} ("))
}

/// The full terrain shader for `path`, with the pack's code (if any) and the
/// built-in version of every hook the pack does not define.
#[must_use]
pub fn compose(path: TerrainPath, pack: Option<&str>) -> String {
    let common = include_str!("terrain-common.wgsl");
    let entry = match path {
        TerrainPath::Pulling => include_str!("terrain-pulling.wgsl"),
        TerrainPath::PullingIndexed => include_str!("terrain-pulling-indexed.wgsl"),
        TerrainPath::Instanced => include_str!("terrain-instance.wgsl"),
    };
    let pack = pack.unwrap_or("");
    let mut out = String::with_capacity(common.len() + entry.len() + pack.len() + 1024);
    out.push_str(common);
    out.push_str("\n// ---- shader pack ----\n");
    out.push_str(pack);
    out.push_str("\n// ---- built-in hooks ----\n");
    for (name, default) in HOOKS
        .iter()
        .zip([DEFAULT_LIGHT_CURVE, DEFAULT_SURFACE, DEFAULT_FOG])
    {
        if !defines(pack, name) {
            out.push_str(default);
        }
    }
    out.push_str(entry);
    out
}

/// Varyings a version 2 vertex hook can pass to its fragment hook.
pub const PACK_VARYINGS: usize = 4;

const V2_GEOMETRY_PRELUDE: &str = "
// ---- hook version 2: geometry pass (render-graph.md) ----
/// What the vertex hook gets for one vertex.
struct PackVertex {
    world: vec3<f32>,
    normal: vec3<f32>,
    layer: u32,
}
/// What it returns: the displaced world position (the engine projects it)
/// and up to four values for the fragment hook.
struct PackVaryings {
    world: vec3<f32>,
    v0: vec4<f32>,
    v1: vec4<f32>,
    v2: vec4<f32>,
    v3: vec4<f32>,
}
/// What the fragment hook gets.
struct PackFragment {
    albedo: vec4<f32>,
    normal: vec3<f32>,
    world: vec3<f32>,
    uv: vec2<f32>,
    block_light: f32,
    sky_light: f32,
    daylight: f32,
    v0: vec4<f32>,
    v1: vec4<f32>,
    v2: vec4<f32>,
    v3: vec4<f32>,
}
";

const V2_DEFAULT_VERTEX: &str = "
fn pack_vertex(v: PackVertex) -> PackVaryings {
    var o: PackVaryings;
    o.world = v.world;
    return o;
}
";

const V2_GEOMETRY_ENTRY: &str = "
fn pack_vertex_stage(input: VertexOutput) -> VertexOutput {
    var v: PackVertex;
    v.world = input.world;
    v.normal = input.normal;
    v.layer = input.layer;
    let r = pack_vertex(v);
    var o = input;
    o.world = r.world;
    o.clip = camera.view_proj * vec4<f32>(r.world, 1.0);
    o.pack0 = r.v0;
    o.pack1 = r.v1;
    o.pack2 = r.v2;
    o.pack3 = r.v3;
    return o;
}

@fragment
fn fs_pack(in: VertexOutput) -> PackTargets {
    let n = in.normal;
    // Texture coordinates from the undisplaced cell grid would need the
    // original position; displaced vertices use the displaced one.
    var uv: vec2<f32>;
    if (n.y != 0.0) {
        uv = in.world.xz;
    } else if (n.x != 0.0) {
        uv = vec2<f32>(in.world.z, -in.world.y);
    } else {
        uv = vec2<f32>(in.world.x, -in.world.y);
    }
    let color = textureSample(block_textures, block_sampler, uv, in.layer);
    if (color.a < 0.5) {
        discard;
    }
    let light = light_at(vec3<i32>(floor(in.world + n * 0.5)));
    var f: PackFragment;
    f.albedo = color;
    f.normal = n;
    f.world = in.world;
    f.uv = fract(uv);
    f.block_light = light.x;
    f.sky_light = light.y;
    f.daylight = camera.position.w;
    f.v0 = in.pack0;
    f.v1 = in.pack1;
    f.v2 = in.pack2;
    f.v3 = in.pack3;
    return pack_fragment(f);
}
";

/// `struct PackTargets` with one `vec4<f32>` output per written target.
fn targets_struct(writes: &[&str]) -> String {
    let mut s = String::from("struct PackTargets {\n");
    for (i, w) in writes.iter().enumerate() {
        s.push_str(&format!("    @location({i}) {w}: vec4<f32>,\n"));
    }
    s.push_str("}\n");
    s
}

/// The 32-option uniform of hook version 2 (the v1 layout, extended).
fn v2_pack_params(common: &str) -> String {
    common
        .replace(
            "struct PackParams {\n    options: array<vec4<f32>, 2>,\n    time: vec4<f32>,\n}",
            "struct PackParams {\n    options: array<vec4<f32>, 2>,\n    time: vec4<f32>,\n    more: array<vec4<f32>, 6>,\n}",
        )
        .replace(
            "    return pack_params.options[i / 4u][i % 4u];",
            "    if (i < 8u) {\n        return pack_params.options[i / 4u][i % 4u];\n    }\n    let j = i - 8u;\n    return pack_params.more[j / 4u][j % 4u];",
        )
}

/// The full shader of a version 2 geometry pass on `path`: the engine's
/// vertex pulling with the pack's `pack_vertex` (identity if absent), its
/// `pack_fragment(f: PackFragment) -> PackTargets` writing `writes`, and the
/// version 1 hooks (the pack's or built-in) available to call.
#[must_use]
pub fn compose_geometry_v2(path: TerrainPath, pack: &str, writes: &[&str]) -> String {
    let common = v2_pack_params(include_str!("terrain-common.wgsl")).replace(
        "    @location(2) @interpolate(flat) layer: u32,\n}",
        "    @location(2) @interpolate(flat) layer: u32,\n    @location(3) pack0: vec4<f32>,\n    @location(4) pack1: vec4<f32>,\n    @location(5) pack2: vec4<f32>,\n    @location(6) pack3: vec4<f32>,\n}",
    );
    let entry = match path {
        TerrainPath::Pulling => include_str!("terrain-pulling.wgsl"),
        TerrainPath::PullingIndexed => include_str!("terrain-pulling-indexed.wgsl"),
        TerrainPath::Instanced => include_str!("terrain-instance.wgsl"),
    };
    // Every vertex goes through the pack's vertex hook.
    let entry = entry
        .lines()
        .map(
            |l| match l.trim_start().strip_prefix("return terrain_vertex(") {
                Some(rest) => format!(
                    "    return pack_vertex_stage(terrain_vertex({}",
                    rest.replacen(");", "));", 1)
                ),
                None => l.to_owned(),
            },
        )
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = String::with_capacity(common.len() + entry.len() + pack.len() + 4096);
    out.push_str(&common);
    out.push_str(V2_GEOMETRY_PRELUDE);
    out.push_str(&targets_struct(writes));
    out.push_str("\n// ---- shader pack ----\n");
    out.push_str(pack);
    out.push_str("\n// ---- built-in hooks ----\n");
    if !defines(pack, "pack_vertex") {
        out.push_str(V2_DEFAULT_VERTEX);
    }
    for (name, default) in HOOKS
        .iter()
        .zip([DEFAULT_LIGHT_CURVE, DEFAULT_SURFACE, DEFAULT_FOG])
    {
        if !defines(pack, name) {
            out.push_str(default);
        }
    }
    out.push_str(V2_GEOMETRY_ENTRY);
    out.push_str(&entry);
    out
}

/// What a full-screen pass samples: a color target or a depth target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadKind {
    /// `texture_2d<f32>`, sampled with `pack_sampler`.
    Color,
    /// `texture_depth_2d`, read with `textureLoad`.
    Depth,
}

/// The uniform block every full-screen pass gets (`render-graph.md`,
/// "Uniforms"; the subset implemented in T1-22 part 1).
pub const PACK_FRAME: &str = "
struct PackFrame {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
    // xyz camera position, w daylight 0..1
    camera: vec4<f32>,
    prev_camera: vec4<f32>,
    // width, height, 1 / width, 1 / height of this pass's targets
    viewport: vec4<f32>,
    // x frame index, y seconds since the world opened
    frame: vec4<f32>,
    // x fog start, y fog end (blocks)
    fog: vec4<f32>,
    sky: vec4<f32>,
    // xyz direction towards the sun
    sun: vec4<f32>,
    options: array<vec4<f32>, 8>,
}
";

/// The full shader of a version 2 full-screen pass: `PackFrame` at binding
/// 0, `pack_sampler` at 1, the read targets by name from binding 2 on, the
/// pack's `pack_fullscreen(uv: vec2<f32>) -> PackTargets` writing `writes`
/// (a `final` pass writes one field, `screen`).
#[must_use]
pub fn compose_fullscreen(pack: &str, reads: &[(&str, ReadKind)], writes: &[&str]) -> String {
    let mut out = String::from(PACK_FRAME);
    out.push_str("@group(0) @binding(0) var<uniform> pack_frame: PackFrame;\n");
    out.push_str("@group(0) @binding(1) var pack_sampler: sampler;\n");
    for (i, (name, kind)) in reads.iter().enumerate() {
        let ty = match kind {
            ReadKind::Color => "texture_2d<f32>",
            ReadKind::Depth => "texture_depth_2d",
        };
        out.push_str(&format!(
            "@group(0) @binding({}) var {name}: {ty};\n",
            i + 2
        ));
    }
    out.push_str(
        "
fn pack_option(i: u32) -> f32 {
    return pack_frame.options[i / 4u][i % 4u];
}
fn pack_time() -> f32 {
    return pack_frame.frame.y;
}
/// World position of the pixel at `uv` with depth `depth` (0..1).
fn pack_world_at(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let w = pack_frame.inv_view_proj * ndc;
    return w.xyz / w.w;
}
/// The pixel coordinate of `uv` in a target of this pass's size.
fn pack_pixel(uv: vec2<f32>) -> vec2<i32> {
    return vec2<i32>(uv * pack_frame.viewport.xy);
}
struct FullOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> FullOut {
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: FullOut;
    out.pos = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(xy.x, 1.0 - xy.y);
    return out;
}
",
    );
    out.push_str(&targets_struct(writes));
    out.push_str("\n// ---- shader pack ----\n");
    out.push_str(pack);
    out.push_str(
        "
@fragment
fn fs_full(in: FullOut) -> PackTargets {
    return pack_fullscreen(in.uv);
}
",
    );
    out
}

/// Parses and validates WGSL; the error text names the line.
pub fn validate(source: &str) -> Result<(), String> {
    let module = naga::front::wgsl::parse_str(source).map_err(|e| e.emit_to_string(source))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|e| e.emit_to_string(source))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATHS: [TerrainPath; 3] = [
        TerrainPath::Pulling,
        TerrainPath::PullingIndexed,
        TerrainPath::Instanced,
    ];

    #[test]
    fn built_in_hooks_validate_on_every_path() {
        for p in PATHS {
            validate(&compose(p, None)).unwrap_or_else(|e| panic!("{p:?}: {e}"));
        }
    }

    #[test]
    fn a_pack_replaces_some_hooks_and_keeps_the_rest() {
        let pack = "
fn pack_light_curve(level: f32) -> f32 { return level / 15.0 * pack_option(0u); }
fn pack_fog(rgb: vec3<f32>, world: vec3<f32>, eye: vec3<f32>, sky: vec3<f32>, start: f32, end: f32) -> vec3<f32> {
    return rgb * (0.5 + 0.5 * sin(pack_time()));
}
";
        for p in PATHS {
            let src = compose(p, Some(pack));
            validate(&src).unwrap_or_else(|e| panic!("{p:?}: {e}"));
            assert_eq!(src.matches("fn pack_light_curve(").count(), 1);
            assert_eq!(
                src.matches("fn pack_surface(").count(),
                1,
                "built-in surface kept"
            );
        }
    }

    #[test]
    fn broken_packs_are_rejected_with_a_message() {
        let wrong_type = "fn pack_light_curve(level: f32) -> f32 { return vec3<f32>(1.0); }";
        let syntax = "fn pack_surface(s: Surface -> vec3<f32> {";
        for pack in [wrong_type, syntax] {
            let e = validate(&compose(TerrainPath::Pulling, Some(pack))).expect_err("must fail");
            assert!(!e.is_empty());
        }
    }

    #[test]
    fn version_2_geometry_and_fullscreen_compose_and_validate() {
        let pack = "
fn pack_vertex(v: PackVertex) -> PackVaryings {
    var o: PackVaryings;
    o.world = v.world + vec3<f32>(0.0, 0.1 * sin(pack_time() + v.world.x), 0.0);
    o.v0 = vec4<f32>(v.normal, 1.0);
    return o;
}
fn pack_fragment(f: PackFragment) -> PackTargets {
    var t: PackTargets;
    t.colortex1 = f.albedo;
    t.colortex2 = vec4<f32>(f.v0.xyz * 0.5 + 0.5, f.sky_light / 15.0);
    t.colortex3 = vec4<f32>(pack_surface(Surface(f.albedo, f.normal, f.world, f.block_light, f.sky_light, f.daylight)), pack_option(20u));
    return t;
}
";
        for p in PATHS {
            let src = compose_geometry_v2(p, pack, &["colortex1", "colortex2", "colortex3"]);
            validate(&src).unwrap_or_else(|e| panic!("{p:?}: {e}"));
            assert!(
                src.contains("return pack_vertex_stage(terrain_vertex("),
                "{p:?}"
            );
        }
        // Without a vertex hook, the identity one is supplied.
        let frag_only = "fn pack_fragment(f: PackFragment) -> PackTargets { var t: PackTargets; t.colortex1 = f.albedo; return t; }";
        validate(&compose_geometry_v2(
            TerrainPath::Pulling,
            frag_only,
            &["colortex1"],
        ))
        .expect("valid");
        let full = "
fn pack_fullscreen(uv: vec2<f32>) -> PackTargets {
    let c = textureSample(colortex1, pack_sampler, uv).rgb;
    let d = textureLoad(depth0, pack_pixel(uv), 0);
    let w = pack_world_at(uv, d);
    var t: PackTargets;
    t.colortex0 = vec4<f32>(c * pack_option(3u) + w * 0.0, 1.0);
    return t;
}
";
        validate(&compose_fullscreen(
            full,
            &[("colortex1", ReadKind::Color), ("depth0", ReadKind::Depth)],
            &["colortex0"],
        ))
        .expect("valid");
        // A pack error names its line.
        let bad = compose_fullscreen(
            "fn pack_fullscreen(uv: vec2<f32>) -> PackTargets { return 1; }",
            &[],
            &["colortex0"],
        );
        assert!(validate(&bad).is_err());
    }
}
