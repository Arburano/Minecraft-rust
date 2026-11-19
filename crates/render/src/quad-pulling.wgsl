@group(0) @binding(1) var<storage, read> quads: array<vec2<u32>>;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let corners = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
    return quad_vertex(quads[vertex_index / 6u], corners[vertex_index % 6u]);
}
