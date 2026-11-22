// Path B: six vertices per quad pulled from a storage buffer; the batch slot
// comes in as the instance index (first_instance).
@group(1) @binding(0) var<storage, read> quads: array<vec2<u32>>;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) slot: u32) -> VertexOutput {
    let corners = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
    return terrain_vertex(quads[vi / 6u], corners[vi % 6u], slot);
}
