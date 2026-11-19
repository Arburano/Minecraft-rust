@vertex
fn vs_main(
    @location(0) quad: vec2<u32>,
    @builtin(vertex_index) vertex_index: u32,
) -> VertexOutput {
    return quad_vertex(quad, vertex_index);
}
