// Path C: one 8-byte instance per quad, a 4-vertex strip; the batch slot
// comes in as first_vertex / 4.
@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @location(0) quad: vec2<u32>) -> VertexOutput {
    return terrain_vertex(quad, vi & 3u, vi >> 2u);
}
