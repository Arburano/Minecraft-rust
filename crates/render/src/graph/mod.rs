//! Render graph declarations (ADR-0015, `docs/architecture/render-graph.md`):
//! the version 2 shader-pack manifest, its validation and the allocation of
//! render targets with aliasing. Pure code: no GPU, fully unit-tested.
//!
//! A frame is a fixed list of stages. A pack declares passes in some stages
//! and the targets they read and write; stages it leaves out run the
//! engine's built-in passes. Validation checks the order (every read target
//! is written earlier in the frame, or has history, or is provided by the
//! engine at that point), formats, limits and memory. Any error names the
//! pass, and the caller falls back to the built-in look.
//!
//! Allocation works on target *versions*: each write of a target starts a
//! new version that lives until its last read. Versions of the same format
//! and scale whose lifetimes do not overlap share one physical texture, so a
//! pass that reads and writes the same target (ping-pong) gets two textures
//! and a long chain of passes needs only a few.
//!
//! [`exec::GraphRenderer`] runs a validated graph on the GPU.

pub mod exec;

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use serde::Deserialize;

/// The hook interface version of this module.
pub const GRAPH_HOOK_VERSION: u32 = 2;
/// Pack-declared color targets at most (`colortex0` … `colortex15`).
pub const MAX_TARGETS: usize = 16;
/// Pack options at most in version 2.
pub const MAX_OPTIONS_V2: usize = 32;
/// Largest vertex displacement a pack may declare, in blocks.
pub const MAX_DISPLACEMENT: f32 = 4.0;
/// Passes per pack at most.
pub const MAX_PASSES: usize = 32;

/// Frame stages in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Once per resize: the pack's lookup textures.
    Setup,
    /// Shadow maps from the sun.
    Shadow,
    /// Opaque geometry into the G-buffer (or the built-in forward pass).
    GbufferOpaque,
    /// Full-screen passes after the opaque geometry.
    Deferred,
    /// Water, glass, weather.
    GbufferTranslucent,
    /// Full-screen passes after all geometry.
    Composite,
    /// The last pass, into the swap chain.
    Final,
}

impl Stage {
    /// Whether passes of this stage draw geometry (vertex + fragment hooks).
    #[must_use]
    pub fn is_geometry(self) -> bool {
        matches!(
            self,
            Self::Shadow | Self::GbufferOpaque | Self::GbufferTranslucent
        )
    }

    /// The manifest name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Shadow => "shadow",
            Self::GbufferOpaque => "gbuffer_opaque",
            Self::Deferred => "deferred",
            Self::GbufferTranslucent => "gbuffer_translucent",
            Self::Composite => "composite",
            Self::Final => "final",
        }
    }
}

/// Allowed target formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetFormat {
    /// 8-bit RGBA, unsigned normalized.
    Rgba8,
    /// 16-bit float RGBA.
    Rgba16f,
    /// 16-bit float RG.
    Rg16f,
    /// Packed 11/11/10-bit float RGB.
    R11g11b10f,
    /// 32-bit float R.
    R32f,
    /// 32-bit float RGBA.
    Rgba32f,
}

impl TargetFormat {
    /// Bytes per pixel.
    #[must_use]
    pub fn bytes(self) -> u64 {
        match self {
            Self::Rgba8 | Self::Rg16f | Self::R11g11b10f | Self::R32f => 4,
            Self::Rgba16f => 8,
            Self::Rgba32f => 16,
        }
    }

    /// The wgpu format.
    #[must_use]
    pub fn wgpu(self) -> wgpu::TextureFormat {
        match self {
            Self::Rgba8 => wgpu::TextureFormat::Rgba8Unorm,
            Self::Rgba16f => wgpu::TextureFormat::Rgba16Float,
            Self::Rg16f => wgpu::TextureFormat::Rg16Float,
            Self::R11g11b10f => wgpu::TextureFormat::Rg11b10Ufloat,
            Self::R32f => wgpu::TextureFormat::R32Float,
            Self::Rgba32f => wgpu::TextureFormat::Rgba32Float,
        }
    }
}

/// A pack-declared target.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetDecl {
    /// Pixel format.
    pub format: TargetFormat,
    /// Size relative to the viewport (0 < scale ≤ 1).
    #[serde(default = "one")]
    pub scale: f32,
    /// The previous frame's content can be read (`colortexN` before it is
    /// written in the frame); written at most once per frame.
    #[serde(default)]
    pub history: bool,
}

fn one() -> f32 {
    1.0
}

/// The `[shadow]` table.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShadowDecl {
    /// Shadow map size.
    pub resolution: u32,
    /// Cascades.
    #[serde(default = "one_u32")]
    pub cascades: u32,
    /// A colored shadow map too (tinted glass).
    #[serde(default)]
    pub color: bool,
}

fn one_u32() -> u32 {
    1
}

/// One `[[pass]]`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PassDecl {
    /// The stage it runs in.
    pub stage: Stage,
    /// The WGSL file with its hooks.
    pub file: String,
    /// Targets it samples.
    #[serde(default)]
    pub reads: Vec<String>,
    /// Targets it writes (geometry passes: the color attachments, in order).
    #[serde(default)]
    pub writes: Vec<String>,
    /// Geometry it draws (geometry stages only).
    #[serde(default)]
    pub buckets: Vec<String>,
}

impl PassDecl {
    /// A short name for messages: `stage/file`.
    #[must_use]
    pub fn label(&self) -> String {
        format!("{}/{}", self.stage.name(), self.file)
    }
}

/// A pack option (slider).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptionDecl {
    /// Stable id.
    pub id: String,
    /// Menu label.
    #[serde(default)]
    pub label: String,
    /// Smallest value.
    pub min: f32,
    /// Largest value.
    pub max: f32,
    /// Initial value.
    pub default: f32,
}

/// `shaderpack.toml` of a version 2 pack.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphManifest {
    /// Display name.
    #[serde(default)]
    pub name: String,
    /// Description.
    #[serde(default)]
    pub description: String,
    /// Must be 2.
    pub hook_version: u32,
    /// How far the vertex hook may move a vertex (blocks); culling boxes grow
    /// by this.
    #[serde(default)]
    pub max_displacement: f32,
    /// Pack targets by name (`colortex0` … `colortex15`).
    #[serde(default)]
    pub targets: BTreeMap<String, TargetDecl>,
    /// Shadow maps, if the pack draws shadows.
    #[serde(default)]
    pub shadow: Option<ShadowDecl>,
    /// Passes in file order.
    #[serde(default, rename = "pass")]
    pub passes: Vec<PassDecl>,
    /// Sliders.
    #[serde(default, rename = "option")]
    pub options: Vec<OptionDecl>,
}

/// Why a graph was refused: the pass it concerns (if any) and what is wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphError {
    /// `stage/file` of the pass, if the error belongs to one.
    pub pass: Option<String>,
    /// What is wrong.
    pub message: String,
}

impl GraphError {
    fn new(pass: Option<&PassDecl>, message: impl Into<String>) -> Self {
        Self {
            pass: pass.map(PassDecl::label),
            message: message.into(),
        }
    }
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.pass {
            Some(p) => write!(f, "pass {p}: {}", self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for GraphError {}

impl GraphManifest {
    /// Parses `shaderpack.toml`.
    pub fn from_toml(source: &str) -> Result<Self, GraphError> {
        toml::from_str(source).map_err(|e| GraphError::new(None, format!("shaderpack.toml: {e}")))
    }
}

/// Targets the engine provides.
fn engine_target(name: &str) -> bool {
    matches!(name, "depth0" | "depth1")
        || name.starts_with("shadowtex")
        || name.starts_with("shadowcolor")
}

fn color_target_name(name: &str) -> bool {
    name.strip_prefix("colortex")
        .and_then(|n| n.parse::<usize>().ok())
        .is_some_and(|n| n < MAX_TARGETS && format!("colortex{n}") == name)
}

/// What the validator knows about the device and the engine version.
#[derive(Clone, Copy, Debug)]
pub struct GraphLimits {
    /// Viewport size in pixels.
    pub width: u32,
    /// Viewport size in pixels.
    pub height: u32,
    /// The most the pack's targets may take, in bytes.
    pub max_target_bytes: u64,
    /// Color attachments per pass the device supports.
    pub max_color_attachments: u32,
    /// Stages this engine version can run passes in.
    pub supported: &'static [Stage],
}

/// The engine's built-in Default graph: the built-in opaque pass into a
/// 16-bit float `colortex0`, presented by the built-in final pass.
pub const BUILTIN_DEFAULT: &str = "name = \"Built-in (Default)\"\nhook_version = 2\n[targets]\ncolortex0 = { format = \"rgba16f\" }\n";

/// The stages implemented by this engine version (T1-22 part 1). Shadow,
/// setup and translucent passes are refused with a clear message.
pub const SUPPORTED_STAGES: [Stage; 4] = [
    Stage::GbufferOpaque,
    Stage::Deferred,
    Stage::Composite,
    Stage::Final,
];

/// A physical texture of the allocation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    /// Format.
    pub format: TargetFormat,
    /// Scale of the viewport.
    pub scale: f32,
    /// Kept across frames (history targets: two slots, swapped each frame).
    pub persistent: bool,
}

/// Where one pass reads and writes, as slot indices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassIo {
    /// For each name in `reads`: the slot to sample, or `None` for an
    /// engine target (`depth0`, …).
    pub reads: Vec<Option<usize>>,
    /// For each name in `writes`: the slot to render into.
    pub writes: Vec<usize>,
}

/// A validated graph with its target allocation.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidGraph {
    /// Passes in frame order (stage order, then file order).
    pub passes: Vec<PassDecl>,
    /// I/O per pass, same order.
    pub io: Vec<PassIo>,
    /// Physical textures.
    pub slots: Vec<Slot>,
    /// For each history target: its two slots (current frame writes the
    /// first; the engine swaps them every frame).
    pub history: BTreeMap<String, [usize; 2]>,
    /// The slot holding `colortex0` at the end of the frame (what `final`
    /// presents when the pack has no final pass), if the pack writes it.
    pub output: Option<usize>,
    /// Bytes all slots take at the given viewport.
    pub bytes: u64,
    /// Whether the pack replaces the opaque geometry pass.
    pub custom_gbuffer: bool,
    /// Where the built-in opaque pass draws its lit color (the first
    /// version of `colortex0`), when the pack keeps the built-in pass and
    /// reads its result.
    pub builtin_color: Option<usize>,
}

/// One version of a target: written by `from` (None: before the frame, i.e.
/// history), last read by `until`.
struct Version {
    target: String,
    from: Option<usize>,
    until: usize,
}

impl GraphManifest {
    /// Checks the graph against the limits and allocates its targets.
    pub fn validate(&self, limits: &GraphLimits) -> Result<ValidGraph, GraphError> {
        if self.hook_version != GRAPH_HOOK_VERSION {
            return Err(GraphError::new(
                None,
                format!(
                    "hook_version {} (this module runs version {GRAPH_HOOK_VERSION})",
                    self.hook_version
                ),
            ));
        }
        if !(0.0..=MAX_DISPLACEMENT).contains(&self.max_displacement) {
            return Err(GraphError::new(
                None,
                format!("max_displacement must be 0..={MAX_DISPLACEMENT} blocks"),
            ));
        }
        if self.targets.len() > MAX_TARGETS {
            return Err(GraphError::new(
                None,
                format!("at most {MAX_TARGETS} targets"),
            ));
        }
        if self.options.len() > MAX_OPTIONS_V2 {
            return Err(GraphError::new(
                None,
                format!("at most {MAX_OPTIONS_V2} options"),
            ));
        }
        if self.passes.len() > MAX_PASSES {
            return Err(GraphError::new(
                None,
                format!("at most {MAX_PASSES} passes"),
            ));
        }
        for o in &self.options {
            if !(o.min <= o.default && o.default <= o.max) {
                return Err(GraphError::new(
                    None,
                    format!("option {}: min ≤ default ≤ max", o.id),
                ));
            }
        }
        for (name, t) in &self.targets {
            if !color_target_name(name) {
                return Err(GraphError::new(
                    None,
                    format!("target {name:?}: pack targets are colortex0 … colortex15"),
                ));
            }
            if !(t.scale > 0.0 && t.scale <= 1.0) {
                return Err(GraphError::new(
                    None,
                    format!("target {name}: scale must be in (0, 1]"),
                ));
            }
        }
        if let Some(s) = &self.shadow
            && !limits.supported.contains(&Stage::Shadow)
        {
            let _ = s;
            return Err(GraphError::new(
                None,
                "shadows ([shadow]) are not implemented in this engine version yet",
            ));
        }
        // Frame order: stage, then file order (stable sort).
        let mut passes = self.passes.clone();
        passes.sort_by_key(|p| p.stage);
        let custom_gbuffer = passes.iter().any(|p| p.stage == Stage::GbufferOpaque);
        // What exists before each pass.
        let mut versions: Vec<Version> = Vec::new();
        let mut current: HashMap<String, usize> = HashMap::new(); // target -> version index
        let mut written_once: HashMap<String, usize> = HashMap::new();
        // History targets start with the previous frame's version.
        for (name, t) in &self.targets {
            if t.history {
                current.insert(name.clone(), versions.len());
                versions.push(Version {
                    target: name.clone(),
                    from: None,
                    until: 0,
                });
            }
        }
        // The built-in opaque pass writes colortex0 (lit color) and depth0.
        let builtin_writes_color0 = !custom_gbuffer;
        let mut io_versions: Vec<(Vec<Option<usize>>, Vec<usize>)> = Vec::new();
        let mut color0_pending = builtin_writes_color0;
        let mut builtin_version: Option<usize> = None;
        let mut depth0 = false;
        for (i, p) in passes.iter().enumerate() {
            if !limits.supported.contains(&p.stage) {
                return Err(GraphError::new(
                    Some(p),
                    format!(
                        "the {} stage is not implemented in this engine version yet",
                        p.stage.name()
                    ),
                ));
            }
            if p.file.is_empty() || p.file.contains("..") || p.file.contains('\\') {
                return Err(GraphError::new(
                    Some(p),
                    "file must be a plain path inside the pack",
                ));
            }
            if p.stage.is_geometry() {
                if p.writes.is_empty() {
                    return Err(GraphError::new(
                        Some(p),
                        "a geometry pass writes at least one target",
                    ));
                }
                for b in &p.buckets {
                    if b != "voxel_quads" {
                        return Err(GraphError::new(
                            Some(p),
                            format!(
                                "bucket {b:?} is not drawn by this engine version (voxel_quads only)"
                            ),
                        ));
                    }
                }
            } else if !p.buckets.is_empty() {
                return Err(GraphError::new(
                    Some(p),
                    "only geometry passes draw buckets",
                ));
            }
            if p.stage == Stage::Final && !p.writes.is_empty() {
                return Err(GraphError::new(
                    Some(p),
                    "the final pass writes the screen, not targets",
                ));
            }
            if !p.stage.is_geometry() && p.stage != Stage::Final && p.writes.is_empty() {
                return Err(GraphError::new(
                    Some(p),
                    "a full-screen pass writes at least one target",
                ));
            }
            let scales: Vec<f32> = p
                .writes
                .iter()
                .filter_map(|w| self.targets.get(w).map(|t| t.scale))
                .collect();
            if scales.windows(2).any(|w| w[0] != w[1]) {
                return Err(GraphError::new(
                    Some(p),
                    "all targets a pass writes must have the same scale",
                ));
            }
            if p.stage.is_geometry() && scales.iter().any(|&s| s != 1.0) {
                return Err(GraphError::new(
                    Some(p),
                    "geometry passes write full-size targets (scale = 1)",
                ));
            }
            if p.writes.len() as u32 > limits.max_color_attachments {
                return Err(GraphError::new(
                    Some(p),
                    format!(
                        "at most {} targets per pass on this device",
                        limits.max_color_attachments
                    ),
                ));
            }
            // The built-in opaque pass comes right before the first pass
            // after it; mark its outputs once we reach that point.
            if p.stage > Stage::GbufferOpaque {
                depth0 = true;
                if color0_pending {
                    color0_pending = false;
                    if self.targets.contains_key("colortex0") {
                        // The built-in lit color goes into the pack's colortex0.
                        let v = versions.len();
                        versions.push(Version {
                            target: "colortex0".into(),
                            from: Some(i),
                            until: i,
                        });
                        current.insert("colortex0".into(), v);
                        builtin_version = Some(v);
                    }
                }
            }
            let mut reads = Vec::new();
            for r in &p.reads {
                if engine_target(r) {
                    let available = match r.as_str() {
                        "depth0" | "depth1" => depth0 || p.stage > Stage::GbufferOpaque,
                        _ => false,
                    };
                    if !available {
                        return Err(GraphError::new(
                            Some(p),
                            format!(
                                "reads {r}, which this engine version does not provide at this point"
                            ),
                        ));
                    }
                    reads.push(None);
                    continue;
                }
                if !self.targets.contains_key(r) {
                    return Err(GraphError::new(
                        Some(p),
                        format!("reads {r}, which is not declared in [targets]"),
                    ));
                }
                let Some(&v) = current.get(r) else {
                    return Err(GraphError::new(
                        Some(p),
                        format!(
                            "reads {r} before any pass writes it (declare history = true to read the previous frame)"
                        ),
                    ));
                };
                versions[v].until = versions[v].until.max(i);
                reads.push(Some(v));
            }
            let mut writes = Vec::new();
            for w in &p.writes {
                let Some(decl) = self.targets.get(w) else {
                    return Err(GraphError::new(
                        Some(p),
                        format!("writes {w}, which is not declared in [targets]"),
                    ));
                };
                if writes.iter().any(|&v: &usize| versions[v].target == *w) {
                    return Err(GraphError::new(Some(p), format!("writes {w} twice")));
                }
                if decl.history {
                    let n = written_once.entry(w.clone()).or_insert(0);
                    *n += 1;
                    if *n > 1 {
                        return Err(GraphError::new(
                            Some(p),
                            format!("history target {w} is written more than once per frame"),
                        ));
                    }
                }
                let v = versions.len();
                versions.push(Version {
                    target: w.clone(),
                    from: Some(i),
                    until: i,
                });
                current.insert(w.clone(), v);
                writes.push(v);
            }
            io_versions.push((reads, writes));
        }
        // No pass after the opaque stage: the built-in pass still draws into
        // colortex0 when it is declared (the built-in Default graph), and
        // the built-in final pass presents it.
        if color0_pending && self.targets.contains_key("colortex0") {
            let n = passes.len();
            let v = versions.len();
            versions.push(Version {
                target: "colortex0".into(),
                from: Some(n),
                until: n,
            });
            current.insert("colortex0".into(), v);
            builtin_version = Some(v);
        }
        if custom_gbuffer && !self.targets.contains_key("colortex0") {
            return Err(GraphError::new(
                None,
                "a pack with its own gbuffer_opaque pass must declare colortex0 (the lit color it presents)",
            ));
        }
        if custom_gbuffer && !current.contains_key("colortex0") {
            return Err(GraphError::new(
                None,
                "no pass writes colortex0 (the lit color the frame presents)",
            ));
        }
        // History versions written this frame must survive to the next one.
        let n = passes.len();
        for v in &mut versions {
            if self.targets.get(&v.target).is_some_and(|t| t.history) {
                v.until = n;
            }
        }
        // The final colortex0 version lives to the end (presented).
        if let Some(&v) = current.get("colortex0") {
            versions[v].until = n;
        }
        // Allocate: history targets get two persistent slots each.
        let mut slots: Vec<Slot> = Vec::new();
        let mut history: BTreeMap<String, [usize; 2]> = BTreeMap::new();
        let mut slot_of: Vec<usize> = vec![usize::MAX; versions.len()];
        for (name, t) in &self.targets {
            if t.history {
                let a = slots.len();
                slots.push(Slot {
                    format: t.format,
                    scale: t.scale,
                    persistent: true,
                });
                slots.push(Slot {
                    format: t.format,
                    scale: t.scale,
                    persistent: true,
                });
                history.insert(name.clone(), [a, a + 1]);
            }
        }
        for (vi, v) in versions.iter().enumerate() {
            if let Some(&[cur, prev]) = history.get(&v.target) {
                slot_of[vi] = if v.from.is_none() { prev } else { cur };
            }
        }
        // Others: greedy interval coloring by start.
        let start = |v: &Version| v.from.unwrap_or(0);
        let mut order: Vec<usize> = (0..versions.len())
            .filter(|&i| slot_of[i] == usize::MAX)
            .collect();
        order.sort_by_key(|&i| (start(&versions[i]), i));
        let mut busy_until: Vec<Option<usize>> = slots.iter().map(|_| None).collect();
        for vi in order {
            let v = &versions[vi];
            let t = &self.targets[&v.target];
            let s = start(v);
            let free = (0..slots.len()).find(|&k| {
                !slots[k].persistent
                    && slots[k].format == t.format
                    && slots[k].scale == t.scale
                    && busy_until[k].is_some_and(|u| u < s)
            });
            let k = match free {
                Some(k) => k,
                None => {
                    slots.push(Slot {
                        format: t.format,
                        scale: t.scale,
                        persistent: false,
                    });
                    busy_until.push(None);
                    slots.len() - 1
                }
            };
            busy_until[k] = Some(v.until.max(s));
            slot_of[vi] = k;
        }
        let io = io_versions
            .into_iter()
            .map(|(r, w)| PassIo {
                reads: r.into_iter().map(|v| v.map(|v| slot_of[v])).collect(),
                writes: w.into_iter().map(|v| slot_of[v]).collect(),
            })
            .collect();
        let bytes = slots
            .iter()
            .map(|s| {
                let w = (f64::from(limits.width) * f64::from(s.scale)).ceil() as u64;
                let h = (f64::from(limits.height) * f64::from(s.scale)).ceil() as u64;
                w * h * s.format.bytes()
            })
            .sum();
        if bytes > limits.max_target_bytes {
            return Err(GraphError::new(
                None,
                format!(
                    "the targets need {} MiB at {}×{}, more than the {} MiB allowed",
                    bytes >> 20,
                    limits.width,
                    limits.height,
                    limits.max_target_bytes >> 20
                ),
            ));
        }
        Ok(ValidGraph {
            output: current.get("colortex0").map(|&v| slot_of[v]),
            passes,
            io,
            slots,
            history,
            bytes,
            custom_gbuffer,
            builtin_color: builtin_version.map(|v| slot_of[v]),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: GraphLimits = GraphLimits {
        width: 1920,
        height: 1080,
        max_target_bytes: 256 << 20,
        max_color_attachments: 8,
        supported: &SUPPORTED_STAGES,
    };

    fn graph(toml: &str) -> Result<ValidGraph, GraphError> {
        GraphManifest::from_toml(toml)?.validate(&LIMITS)
    }

    const DEFERRED: &str = r#"
name = "Deferred"
hook_version = 2
max_displacement = 0.5
[targets]
colortex0 = { format = "rgba16f" }
colortex1 = { format = "rgba8" }
colortex2 = { format = "rg16f" }
[[pass]]
stage = "gbuffer_opaque"
file = "gbuffer.wgsl"
buckets = ["voxel_quads"]
writes = ["colortex1", "colortex2"]
[[pass]]
stage = "deferred"
file = "light.wgsl"
reads = ["colortex1", "colortex2", "depth0"]
writes = ["colortex0"]
[[pass]]
stage = "composite"
file = "tone.wgsl"
reads = ["colortex0"]
writes = ["colortex0"]
"#;

    #[test]
    fn a_deferred_graph_validates_and_ping_pongs() {
        let g = graph(DEFERRED).expect("valid");
        assert!(g.custom_gbuffer);
        assert_eq!(g.passes.len(), 3);
        // The composite pass reads and writes colortex0: two textures.
        let tone = &g.io[2];
        assert_ne!(tone.reads[0], Some(tone.writes[0]));
        assert_eq!(g.output, Some(tone.writes[0]));
        // depth0 is the engine's.
        assert_eq!(g.io[1].reads[2], None);
        assert!(g.bytes > 0);
    }

    #[test]
    fn versions_alias_when_lifetimes_do_not_overlap() {
        // A chain of full-screen passes over one target needs 2 textures,
        // not one per pass.
        let mut toml =
            String::from("hook_version = 2\n[targets]\ncolortex0 = { format = \"rgba16f\" }\n");
        for i in 0..6 {
            toml.push_str(&format!(
                "[[pass]]\nstage = \"composite\"\nfile = \"c{i}.wgsl\"\nreads = [\"colortex0\"]\nwrites = [\"colortex0\"]\n"
            ));
        }
        let g = graph(&toml).expect("valid");
        assert_eq!(g.slots.len(), 2, "{:?}", g.slots);
        // Every pass reads one slot and writes the other.
        for io in &g.io {
            assert_ne!(io.reads[0], Some(io.writes[0]));
        }
    }

    #[test]
    fn errors_name_the_pass() {
        let err = |t: &str| graph(t).expect_err("invalid").to_string();
        // Reading before writing.
        let e = err(&DEFERRED.replace(
            "reads = [\"colortex1\", \"colortex2\", \"depth0\"]",
            "reads = [\"colortex3\"]",
        ));
        assert!(
            e.contains("deferred/light.wgsl") && e.contains("colortex3"),
            "{e}"
        );
        // Unknown format.
        assert!(err(&DEFERRED.replace("rg16f", "rgb9e5")).contains("shaderpack.toml"));
        // Unimplemented stage.
        let e = err(&DEFERRED.replace("stage = \"deferred\"", "stage = \"gbuffer_translucent\""));
        assert!(
            e.contains("gbuffer_translucent") && e.contains("not implemented"),
            "{e}"
        );
        // Shadows not yet.
        assert!(err(&format!("{DEFERRED}[shadow]\nresolution = 2048\n")).contains("shadows"));
        // Bad target name.
        assert!(err(&DEFERRED.replace("colortex2 = ", "gnormal = ")).contains("gnormal"));
        // Displacement.
        assert!(err(&DEFERRED.replace("0.5", "9.0")).contains("max_displacement"));
        // A full-screen pass that writes nothing.
        assert!(
            err(&DEFERRED.replace(
                "writes = [\"colortex0\"]\n[[pass]]",
                "writes = []\n[[pass]]"
            ))
            .contains("full-screen")
        );
        // Path escaping the pack.
        assert!(err(&DEFERRED.replace("light.wgsl", "../light.wgsl")).contains("plain path"));
        // Version.
        assert!(
            err(&DEFERRED.replace("hook_version = 2", "hook_version = 1")).contains("hook_version")
        );
    }

    #[test]
    fn history_reads_the_previous_frame() {
        let toml = r#"
hook_version = 2
[targets]
colortex0 = { format = "rgba16f" }
colortex4 = { format = "rgba16f", history = true }
[[pass]]
stage = "composite"
file = "taa.wgsl"
reads = ["colortex0", "colortex4"]
writes = ["colortex4"]
"#;
        let g = graph(toml).expect("valid");
        let [cur, prev] = g.history["colortex4"];
        assert_eq!(g.io[0].reads[1], Some(prev));
        assert_eq!(g.io[0].writes[0], cur);
        assert!(g.slots[cur].persistent && g.slots[prev].persistent);
        // Written twice per frame: refused.
        let twice = format!(
            "{toml}[[pass]]\nstage = \"composite\"\nfile = \"b.wgsl\"\nreads = [\"colortex4\"]\nwrites = [\"colortex4\"]\n"
        );
        assert!(
            graph(&twice)
                .expect_err("twice")
                .to_string()
                .contains("more than once")
        );
    }

    #[test]
    fn memory_is_checked() {
        let toml = r#"
hook_version = 2
[targets]
colortex0 = { format = "rgba32f" }
colortex1 = { format = "rgba32f" }
[[pass]]
stage = "composite"
file = "a.wgsl"
reads = ["colortex0"]
writes = ["colortex1"]
"#;
        let small = GraphLimits {
            max_target_bytes: 8 << 20,
            ..LIMITS
        };
        let e = GraphManifest::from_toml(toml)
            .expect("toml")
            .validate(&small)
            .expect_err("too big")
            .to_string();
        assert!(e.contains("MiB"), "{e}");
    }

    #[test]
    fn built_in_color_feeds_a_post_only_pack() {
        // No gbuffer pass: the built-in forward pass writes colortex0.
        let toml = r#"
hook_version = 2
[targets]
colortex0 = { format = "rgba16f" }
colortex1 = { format = "rgba16f", scale = 0.5 }
[[pass]]
stage = "composite"
file = "half.wgsl"
reads = ["colortex0"]
writes = ["colortex1"]
[[pass]]
stage = "composite"
file = "merge.wgsl"
reads = ["colortex0", "colortex1"]
writes = ["colortex0"]
"#;
        let g = graph(toml).expect("valid");
        assert!(!g.custom_gbuffer);
        // merge reads colortex0 (built-in version) and writes a new one.
        assert_ne!(g.io[1].reads[0], Some(g.io[1].writes[0]));
        assert_eq!(g.slots[g.io[0].writes[0]].scale, 0.5);
    }

    #[test]
    fn the_built_in_default_graph_routes_through_colortex0() {
        let g = graph(BUILTIN_DEFAULT).expect("valid");
        assert!(g.passes.is_empty() && !g.custom_gbuffer);
        assert_eq!(g.slots.len(), 1);
        assert_eq!(g.builtin_color, Some(0));
        assert_eq!(g.output, Some(0));
    }
}
