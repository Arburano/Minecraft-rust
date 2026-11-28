// Deferred lighting: sun from the normal, sky ambient, warm block light, fog
// from the depth buffer. pack_option(0) sun, (1) ambient.
fn pack_fullscreen(uv: vec2<f32>) -> PackTargets {
    var t: PackTargets;
    let px = pack_pixel(uv);
    let depth = textureLoad(depth0, px, 0);
    if (depth >= 1.0) {
        t.colortex0 = vec4<f32>(pack_frame.sky.rgb, 1.0);
        return t;
    }
    let albedo = textureLoad(colortex1, px, 0).rgb;
    let nl = textureLoad(colortex2, px, 0);
    let block = textureLoad(colortex3, px, 0).r;
    let sky = nl.w * pack_frame.camera.w;
    let direct = max(dot(nl.xyz, pack_frame.sun.xyz), 0.0) * sky * pack_option(0u);
    let ambient = pack_option(1u) * (0.25 + 0.75 * sky);
    let torch = block * block * vec3<f32>(1.0, 0.8, 0.55);
    var rgb = albedo * (vec3<f32>(ambient + 0.85 * direct) + torch);
    let d = distance(pack_world_at(uv, depth), pack_frame.camera.xyz);
    let f = clamp((d - pack_frame.fog.x) / max(pack_frame.fog.y - pack_frame.fog.x, 1.0), 0.0, 1.0);
    rgb = mix(rgb, pack_frame.sky.rgb, f * f);
    t.colortex0 = vec4<f32>(rgb, 1.0);
    return t;
}
