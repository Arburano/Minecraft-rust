//! Shader-pack hooks for the terrain shader (ADR-0014, version 1). A pack's
//! `shade.wgsl` may define any of [`HOOKS`]; the engine supplies the rest.
//! Geometry (quad decoding, culling, draw submission) stays the engine's.
//!
//! What pack code can use (all documented in `docs/modding/packs.md`):
//! `struct Surface { albedo, normal, world, block_light, sky_light, daylight }`,
//! `camera` (view-projection, eye position, fog, sky color) and
//! `pack_option(i) -> f32` / `pack_time() -> f32` from the pack uniform.

use crate::terrain::renderer::TerrainPath;

/// Hook interface version. Packs state the version they target.
pub const HOOK_VERSION: u32 = 1;

/// Function names a pack may provide.
pub const HOOKS: [&str; 3] = ["pack_light_curve", "pack_surface", "pack_fog"];

/// Number of float options a pack can expose.
pub const MAX_OPTIONS: usize = 8;

const DEFAULT_LIGHT_CURVE: &str = "
fn pack_light_curve(level: f32) -> f32 {
    return 0.05 + 0.95 * pow(0.8, 15.0 - level);
}
";

const DEFAULT_SURFACE: &str = "
fn pack_surface(s: Surface) -> vec3<f32> {
    let level = max(s.block_light, s.sky_light * s.daylight);
    let brightness = pack_light_curve(level);
    // Fixed per-direction shade, the classic look.
    let n = s.normal;
    let shade = select(select(0.8, 0.6, n.x != 0.0), select(0.5, 1.0, n.y > 0.0), n.y != 0.0);
    return s.albedo.rgb * brightness * shade;
}
";

const DEFAULT_FOG: &str = "
fn pack_fog(rgb: vec3<f32>, world: vec3<f32>, eye: vec3<f32>, sky: vec3<f32>, start: f32, end: f32) -> vec3<f32> {
    let d = distance(world, eye);
    let fog = clamp((d - start) / max(end - start, 1.0), 0.0, 1.0);
    return mix(rgb, sky, fog);
}
";

/// Whether `code` defines function `name`.
#[must_use]
pub fn defines(code: &str, name: &str) -> bool {
    code.contains(&format!("fn {name}(")) || code.contains(&format!("fn {name} ("))
}

/// The full terrain shader for `path`, with the pack's code (if any) and the
/// built-in version of every hook the pack does not define.
#[must_use]
pub fn compose(path: TerrainPath, pack: Option<&str>) -> String {
    let common = include_str!("terrain-common.wgsl");
    let entry = match path {
        TerrainPath::Pulling => include_str!("terrain-pulling.wgsl"),
        TerrainPath::PullingIndexed => include_str!("terrain-pulling-indexed.wgsl"),
        TerrainPath::Instanced => include_str!("terrain-instance.wgsl"),
    };
    let pack = pack.unwrap_or("");
    let mut out = String::with_capacity(common.len() + entry.len() + pack.len() + 1024);
    out.push_str(common);
    out.push_str("\n// ---- shader pack ----\n");
    out.push_str(pack);
    out.push_str("\n// ---- built-in hooks ----\n");
    for (name, default) in HOOKS
        .iter()
        .zip([DEFAULT_LIGHT_CURVE, DEFAULT_SURFACE, DEFAULT_FOG])
    {
        if !defines(pack, name) {
            out.push_str(default);
        }
    }
    out.push_str(entry);
    out
}

/// Parses and validates WGSL; the error text names the line.
pub fn validate(source: &str) -> Result<(), String> {
    let module = naga::front::wgsl::parse_str(source).map_err(|e| e.emit_to_string(source))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|e| e.emit_to_string(source))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATHS: [TerrainPath; 3] = [
        TerrainPath::Pulling,
        TerrainPath::PullingIndexed,
        TerrainPath::Instanced,
    ];

    #[test]
    fn built_in_hooks_validate_on_every_path() {
        for p in PATHS {
            validate(&compose(p, None)).unwrap_or_else(|e| panic!("{p:?}: {e}"));
        }
    }

    #[test]
    fn a_pack_replaces_some_hooks_and_keeps_the_rest() {
        let pack = "
fn pack_light_curve(level: f32) -> f32 { return level / 15.0 * pack_option(0u); }
fn pack_fog(rgb: vec3<f32>, world: vec3<f32>, eye: vec3<f32>, sky: vec3<f32>, start: f32, end: f32) -> vec3<f32> {
    return rgb * (0.5 + 0.5 * sin(pack_time()));
}
";
        for p in PATHS {
            let src = compose(p, Some(pack));
            validate(&src).unwrap_or_else(|e| panic!("{p:?}: {e}"));
            assert_eq!(src.matches("fn pack_light_curve(").count(), 1);
            assert_eq!(
                src.matches("fn pack_surface(").count(),
                1,
                "built-in surface kept"
            );
        }
    }

    #[test]
    fn broken_packs_are_rejected_with_a_message() {
        let wrong_type = "fn pack_light_curve(level: f32) -> f32 { return vec3<f32>(1.0); }";
        let syntax = "fn pack_surface(s: Surface -> vec3<f32> {";
        for pack in [wrong_type, syntax] {
            let e = validate(&compose(TerrainPath::Pulling, Some(pack))).expect_err("must fail");
            assert!(!e.is_empty());
        }
    }
}
