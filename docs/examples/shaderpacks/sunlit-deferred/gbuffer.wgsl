// G-buffer pass: what the deferred pass needs per pixel.
fn pack_fragment(f: PackFragment) -> PackTargets {
    var t: PackTargets;
    t.colortex1 = vec4<f32>(f.albedo.rgb, 1.0);
    t.colortex2 = vec4<f32>(f.normal, f.sky_light / 15.0);
    t.colortex3 = vec4<f32>(f.block_light / 15.0, 0.0, 0.0, 1.0);
    return t;
}
