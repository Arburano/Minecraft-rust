// Composite: soft tone mapping, a warm tint and a vignette.
// pack_option(2) vignette, (3) warmth.
fn pack_fullscreen(uv: vec2<f32>) -> PackTargets {
    var t: PackTargets;
    let c = textureSample(colortex0, pack_sampler, uv).rgb;
    let warm = mix(vec3<f32>(1.0), vec3<f32>(1.06, 1.0, 0.92), pack_option(3u));
    var rgb = c * warm;
    rgb = rgb / (vec3<f32>(1.0) + rgb * 0.25) * 1.15;
    let o = uv - vec2<f32>(0.5);
    rgb = rgb * (1.0 - pack_option(2u) * dot(o, o));
    t.colortex0 = vec4<f32>(rgb, 1.0);
    return t;
}
