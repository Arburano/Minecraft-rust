// Path B, indexed variant (open question 13): four vertices per quad pulled
// from storage, a shared index buffer 0 1 2 2 1 3 (+4 per quad) and
// base_vertex = first quad × 4 lets the post-transform cache reuse corners.
@group(1) @binding(0) var<storage, read> quads: array<vec2<u32>>;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) slot: u32) -> VertexOutput {
    return terrain_vertex(quads[vi >> 2u], vi & 3u, slot);
}
