// Post pass. pack_option(0) = warmth, pack_option(2) = vignette.
fn pack_post(color: vec3<f32>, uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let d = distance(uv, vec2<f32>(0.5));
    let vignette = 1.0 - pack_option(2u) * d * d;
    let grade = vec3<f32>(1.0 + 0.06 * pack_option(0u), 1.0, 1.0 - 0.06 * pack_option(0u));
    return color * grade * vignette;
}
