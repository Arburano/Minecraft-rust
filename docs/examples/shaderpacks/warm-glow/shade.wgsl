// Terrain hooks. pack_option(0) = warmth, pack_option(1) = saturation.

// A brighter, softer light curve than the built-in one.
fn pack_light_curve(level: f32) -> f32 {
    let x = level / 15.0;
    return 0.04 + 0.96 * x * x * (3.0 - 2.0 * x);
}

fn pack_surface(s: Surface) -> vec3<f32> {
    let level = max(s.block_light, s.sky_light * s.daylight);
    let brightness = pack_light_curve(level);
    let n = s.normal;
    let shade = select(select(0.85, 0.7, n.x != 0.0), select(0.55, 1.0, n.y > 0.0), n.y != 0.0);
    var rgb = s.albedo.rgb * brightness * shade;
    // Block light (torches) gets an orange tint, sky light stays neutral.
    let torch = s.block_light / 15.0;
    let warm = mix(vec3<f32>(1.0), vec3<f32>(1.25, 1.0, 0.7), torch * pack_option(0u));
    rgb = rgb * warm;
    let grey = dot(rgb, vec3<f32>(0.299, 0.587, 0.114));
    return mix(vec3<f32>(grey), rgb, pack_option(1u));
}

// Fog that starts later and fades smoothly.
fn pack_fog(rgb: vec3<f32>, world: vec3<f32>, eye: vec3<f32>, sky: vec3<f32>, start: f32, end: f32) -> vec3<f32> {
    let d = distance(world, eye);
    let t = clamp((d - start) / max(end - start, 1.0), 0.0, 1.0);
    return mix(rgb, sky, t * t);
}
