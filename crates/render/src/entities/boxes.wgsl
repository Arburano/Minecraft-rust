// Colored boxes for entities (T2-03): one instance per box, 36 vertices.
// Only vertex attributes and a uniform, so it runs on every render path.

struct Camera {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    fog: vec4<f32>,   // start, end
    sky: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;

struct Instance {
    @location(0) min: vec3<f32>,
    @location(1) size: vec3<f32>,
    @location(2) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) world: vec3<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: Instance) -> VsOut {
    // Two triangles per face: corners (u, v) of a unit square.
    var quad = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let face = vi / 6u;
    let uv = quad[vi % 6u];
    var p = vec3<f32>(0.0);
    var shade = 1.0;
    switch face {
        case 0u: { p = vec3<f32>(uv.x, 1.0, uv.y); shade = 1.0; }   // up
        case 1u: { p = vec3<f32>(uv.y, 0.0, uv.x); shade = 0.5; }   // down
        case 2u: { p = vec3<f32>(1.0, uv.x, uv.y); shade = 0.8; }   // +x
        case 3u: { p = vec3<f32>(0.0, uv.y, uv.x); shade = 0.8; }   // -x
        case 4u: { p = vec3<f32>(uv.y, uv.x, 1.0); shade = 0.65; }  // +z
        default: { p = vec3<f32>(uv.x, uv.y, 0.0); shade = 0.65; }  // -z
    }
    let world = inst.min + p * inst.size;
    var out: VsOut;
    out.clip = camera.view_proj * vec4<f32>(world, 1.0);
    out.color = inst.color.rgb * shade;
    out.world = world;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let d = distance(in.world, camera.eye.xyz);
    let f = clamp((d - camera.fog.x) / max(camera.fog.y - camera.fog.x, 1.0), 0.0, 1.0);
    return vec4<f32>(mix(in.color, camera.sky.rgb, f), 1.0);
}
