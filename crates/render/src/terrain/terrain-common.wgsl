// Terrain shader shared by paths B (vertex pulling) and C (instancing).
// Quad: low word x 0-5, y 6-11, z 12-17, w 18-23, h 24-29 (batch coordinates
// modulo 64, a stored 0 in a max-corner field means 64); high word block
// state id 0-15, face direction 29-31 (Up, Down, +X, -X, +Z, -Z).

struct Camera {
    view_proj: mat4x4<f32>,
    // xyz camera position, w daylight 0..1
    position: vec4<f32>,
    // x fog start, y fog end (blocks)
    fog: vec4<f32>,
    sky: vec4<f32>,
}

@group(0) @binding(0) var<uniform> camera: Camera;
// Per render batch slot: world origin of the 64³ batch in blocks.
@group(0) @binding(1) var<uniform> batches: array<vec4<i32>, 4096>;
@group(0) @binding(2) var block_textures: texture_2d_array<f32>;
@group(0) @binding(3) var block_sampler: sampler;
// x = face direction, y = block state id -> texture layer.
@group(0) @binding(4) var layers: texture_2d<u32>;
// 16³ light bricks; texel = block | sky << 4. Brick axes: (x, z, y).
@group(0) @binding(5) var light_atlas: texture_3d<u32>;
// Per section (toroidal 64 x 32 x 64): bit 31 = brick slot, else uniform light.
@group(0) @binding(6) var light_index: texture_3d<u32>;

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) @interpolate(flat) normal: vec3<f32>,
    @location(2) @interpolate(flat) layer: u32,
}

const PLANE_AXIS = array<u32, 6>(1u, 1u, 0u, 0u, 2u, 2u);
const W_AXIS = array<u32, 6>(0u, 0u, 1u, 1u, 0u, 0u);
const H_AXIS = array<u32, 6>(2u, 2u, 2u, 2u, 1u, 1u);
// Bit per axis: the field stores a max corner (0 means 64).
const MAX_CORNER = array<u32, 6>(2u, 1u, 3u, 0u, 5u, 0u);
const W_MAX = array<bool, 6>(false, true, true, false, true, false);
const NORMALS = array<vec3<f32>, 6>(
    vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, -1.0, 0.0),
    vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(-1.0, 0.0, 0.0),
    vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 0.0, -1.0),
);

fn get_component(v: vec3<u32>, idx: u32) -> u32 {
    return select(select(v.z, v.y, idx == 1u), v.x, idx == 0u);
}

fn set_component_by_axes(pa: u32, p_val: u32, wa: u32, w_val: u32, h_val: u32, axis: u32) -> u32 {
    return select(select(h_val, w_val, wa == axis), p_val, pa == axis);
}

fn terrain_vertex(quad: vec2<u32>, corner: u32, slot: u32) -> VertexOutput {
    let dir = quad.y >> 29u;
    var a = vec3<u32>(quad.x & 63u, (quad.x >> 6u) & 63u, (quad.x >> 12u) & 63u);
    let wide = (quad.x >> 18u) & 63u;
    let high = (quad.x >> 24u) & 63u;
    let mc = MAX_CORNER[dir];
    if (a.x == 0u && (mc & 1u) != 0u) {
        a.x = 64u;
    }
    if (a.y == 0u && (mc & 2u) != 0u) {
        a.y = 64u;
    }
    if (a.z == 0u && (mc & 4u) != 0u) {
        a.z = 64u;
    }
    let pa = PLANE_AXIS[dir];
    let wa = W_AXIS[dir];
    let ha = H_AXIS[dir];
    let a_wa = get_component(a, wa);
    var u0 = a_wa;
    if (W_MAX[dir]) {
        u0 = a_wa - wide;
    }
    let plane_val = get_component(a, pa);
    let w_val = u0 + (corner & 1u) * wide;
    let h_val = get_component(a, ha) + (corner >> 1u) * high;
    let px = set_component_by_axes(pa, plane_val, wa, w_val, h_val, 0u);
    let py = set_component_by_axes(pa, plane_val, wa, w_val, h_val, 1u);
    let pz = set_component_by_axes(pa, plane_val, wa, w_val, h_val, 2u);
    let p = vec3<u32>(px, py, pz);
    let world = vec3<f32>(batches[slot].xyz) + vec3<f32>(p);
    var out: VertexOutput;
    out.clip = camera.view_proj * vec4<f32>(world, 1.0);
    out.world = world;
    out.normal = NORMALS[dir];
    out.layer = textureLoad(layers, vec2<u32>(dir, quad.y & 65535u), 0).r;
    return out;
}

fn light_at(cell: vec3<i32>) -> vec2<f32> {
    let section = cell >> vec3<u32>(4u);
    let dims = vec3<i32>(64, 32, 64);
    let idx = ((section % dims) + dims) % dims;
    let entry = textureLoad(light_index, idx, 0).r;
    var v = entry & 255u;
    if ((entry & 0x80000000u) != 0u) {
        let s = entry & 0x7fffffffu;
        let origin = vec3<u32>(s % 32u, (s / 32u) % 32u, s / 1024u) * 16u;
        let l = vec3<u32>(cell & vec3<i32>(15));
        v = textureLoad(light_atlas, origin + vec3<u32>(l.x, l.z, l.y), 0).r;
    }
    return vec2<f32>(f32(v & 15u), f32(v >> 4u));
}

// Pack uniform (shader packs, ADR-0014): 8 option values and the time.
struct PackParams {
    options: array<vec4<f32>, 2>,
    time: vec4<f32>,
}
@group(0) @binding(7) var<uniform> pack_params: PackParams;

/// Value of pack option `i` (0..7).
fn pack_option(i: u32) -> f32 {
    return pack_params.options[i / 4u][i % 4u];
}

/// Seconds since the world opened.
fn pack_time() -> f32 {
    return pack_params.time.x;
}

/// What the surface hook gets for one fragment.
struct Surface {
    albedo: vec4<f32>,
    normal: vec3<f32>,
    world: vec3<f32>,
    block_light: f32,
    sky_light: f32,
    daylight: f32,
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let n = in.normal;
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
    // The cell in front of the face holds the light that reaches it.
    let cell = vec3<i32>(floor(in.world + n * 0.5));
    let light = light_at(cell);
    var s: Surface;
    s.albedo = color;
    s.normal = n;
    s.world = in.world;
    s.block_light = light.x;
    s.sky_light = light.y;
    s.daylight = camera.position.w;
    var rgb = pack_surface(s);
    rgb = pack_fog(rgb, in.world, camera.position.xyz, camera.sky.rgb, camera.fog.x, camera.fog.y);
    return vec4<f32>(rgb, 1.0);
}
