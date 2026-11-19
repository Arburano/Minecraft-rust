@group(0) @binding(0) var<uniform> transform: vec4<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) color: vec3<f32>,
}

fn quad_vertex(quad: vec2<u32>, corner: u32) -> VertexOutput {
    let x = f32(quad.x & 63u);
    let y = f32((quad.x >> 6u) & 63u);
    let z = f32((quad.x >> 12u) & 63u);
    let width = f32(((quad.x >> 18u) & 63u) + 1u);
    let height = f32(((quad.x >> 24u) & 63u) + 1u);
    let uv = vec2<f32>(f32(corner & 1u), f32(corner >> 1u));
    let xy = (vec2<f32>(x, y) + uv * vec2<f32>(width, height)) * transform.zw + transform.xy;
    let material = quad.y & 65535u;
    var output: VertexOutput;
    output.position = vec4<f32>(xy, z / 64.0, 1.0);
    output.color = vec3<f32>(0.25 + f32(material & 255u) / 340.0, 0.5, 0.75);
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(input.color, 1.0);
}
