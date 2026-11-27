//! Block textures for the renderer: per face, from the player's own
//! installation when available (T1-14), otherwise our fallback textures.
//!
//! Block files name textures per face (`textures = { all | top | bottom |
//! side | north | south | east | west | side_overlay = "ns:path" }`) and
//! optional tints for gray textures (`tint = { <face> = "grass" | "foliage" |
//! "water" }`).

use std::collections::HashMap;
use std::path::Path;

use assets::local_install::{overlay, resize, tint};

use crate::packs::TextureSource;
use assets::{FALLBACK_KEYS, Rgba8Image, TextureSet, fallback_key, fallback_texture};
use world::{BlockStateId, Registry};

/// Smallest edge of block textures (fallback textures are 16×16).
pub const TEXTURE_SIZE: u32 = 16;

/// Fixed tint colors until biome colors exist.
fn tint_color(name: &str) -> Option<[u8; 3]> {
    match name {
        "grass" => Some([145, 189, 89]),
        "foliage" => Some([119, 171, 47]),
        "water" => Some([63, 118, 228]),
        _ => None,
    }
}

/// One face's texture.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FaceTexture {
    /// Texture reference (`namespace:path`).
    pub reference: Option<String>,
    /// Tint name.
    pub tint: Option<String>,
    /// Overlay drawn over side faces (grass), with its tint.
    pub overlay: Option<(String, Option<String>)>,
}

/// Faces in mesh order: Up, Down, +X (east), −X (west), +Z (south), −Z (north).
pub type BlockFaces = [FaceTexture; 6];

const FACE_NAMES: [[&str; 2]; 6] = [
    ["top", "up"],
    ["bottom", "down"],
    ["east", "side"],
    ["west", "side"],
    ["south", "side"],
    ["north", "side"],
];

/// [`read_block_faces`] of every mod of a build (ADR-0017).
#[must_use]
pub fn read_mod_faces(mods: &world::ModSet) -> HashMap<String, BlockFaces> {
    let mut out = HashMap::new();
    for (_, dir) in mods.block_dirs() {
        out.extend(read_block_faces(&dir));
    }
    out
}

/// Reads the `textures` and `tint` tables of every block file in `blocks_dir`.
#[must_use]
pub fn read_block_faces(blocks_dir: &Path) -> HashMap<String, BlockFaces> {
    let mut out = HashMap::new();
    for e in std::fs::read_dir(blocks_dir)
        .into_iter()
        .flatten()
        .flatten()
    {
        let Ok(text) = std::fs::read_to_string(e.path()) else {
            continue;
        };
        let Ok(t) = text.parse::<toml::Table>() else {
            continue;
        };
        let Some(id) = t.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let get = |table: &str, key: &str| {
            t.get(table)
                .and_then(|v| v.as_table())
                .and_then(|m| m.get(key))
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        };
        let pick = |table: &str, names: [&str; 2]| {
            get(table, names[0])
                .or_else(|| get(table, names[1]))
                .or_else(|| get(table, "all"))
        };
        let faces: BlockFaces = std::array::from_fn(|f| {
            let horizontal = f >= 2;
            FaceTexture {
                reference: pick("textures", FACE_NAMES[f]),
                tint: pick("tint", FACE_NAMES[f]),
                overlay: horizontal
                    .then(|| get("textures", "side_overlay"))
                    .flatten()
                    .map(|o| (o, get("tint", "side_overlay"))),
            }
        });
        out.insert(id.to_owned(), faces);
    }
    out
}

/// What the renderer gets.
pub struct BlockTextures {
    /// Texture arrays (one array; all layers `size`).
    pub set: TextureSet,
    /// `state * 6 + face` → layer.
    pub layer_table: Vec<u32>,
    /// Faces taken from each source, in the order given to [`build`].
    pub per_source: Vec<usize>,
    /// Faces drawn with our fallback textures.
    pub fallback: usize,
    /// Edge of every layer (the largest texture found, power of two, 16–256).
    pub size: u32,
}

/// Builds every state's six face textures. `sources` are searched in order
/// (resource packs top first, then the player's own installation); a face
/// none of them has uses the block's fallback texture. Tints and the side
/// overlay apply to whatever source the texture came from. All layers are
/// scaled (nearest) to the largest texture found, so HD packs keep detail.
#[must_use]
pub fn build(
    registry: &Registry,
    faces: &HashMap<String, BlockFaces>,
    sources: &mut [&mut dyn TextureSource],
) -> BlockTextures {
    let mut cache: HashMap<String, Option<(usize, Rgba8Image)>> = HashMap::new();
    let mut find = |sources: &mut [&mut dyn TextureSource], r: &str| {
        cache
            .entry(r.to_owned())
            .or_insert_with(|| {
                sources
                    .iter_mut()
                    .enumerate()
                    .find_map(|(i, s)| s.texture(r).map(|img| (i, img)))
            })
            .clone()
    };
    // Resolve every face first (to know the largest size).
    struct Resolved {
        key: String,
        base: Option<(usize, Rgba8Image)>,
        tint: Option<[u8; 3]>,
        overlay: Option<(Rgba8Image, Option<[u8; 3]>)>,
        fallback: &'static str,
    }
    let mut resolved = Vec::with_capacity(registry.len() * 6);
    for i in 0..registry.len() as u16 {
        let name = registry.name(BlockStateId(i)).unwrap_or("missing");
        let block = name.split('[').next().unwrap_or(name);
        let key = fallback_key(block);
        let fallback = FALLBACK_KEYS
            .iter()
            .copied()
            .find(|k| *k == key)
            .unwrap_or("missing");
        let default = BlockFaces::default();
        for face in faces.get(block).unwrap_or(&default) {
            let base = face.reference.as_deref().and_then(|r| find(sources, r));
            let overlay = face.overlay.as_ref().and_then(|(o, t)| {
                find(sources, o).map(|(_, img)| (img, t.as_deref().and_then(tint_color)))
            });
            resolved.push(Resolved {
                key: format!("{:?}|{:?}|{:?}", face.reference, face.tint, face.overlay),
                base,
                tint: face.tint.as_deref().and_then(tint_color),
                overlay,
                fallback,
            });
        }
    }
    let largest = resolved
        .iter()
        .filter_map(|r| r.base.as_ref().map(|(_, img)| img.width()))
        .max()
        .unwrap_or(TEXTURE_SIZE);
    let size = largest.clamp(TEXTURE_SIZE, 256).next_power_of_two();
    let mut set = TextureSet::new();
    let mut layer_table = Vec::with_capacity(resolved.len());
    let mut per_source = vec![0; sources.len()];
    let mut fallback = 0;
    for r in resolved {
        let loc = match r.base {
            Some((src, img)) => {
                per_source[src] += 1;
                set.require(&format!("{src}:{}", r.key), || {
                    let mut img = resize(&img, size);
                    if let Some(c) = r.tint {
                        img = tint(&img, c);
                    }
                    if let Some((top, t)) = &r.overlay {
                        let mut top = resize(top, size);
                        if let Some(c) = t {
                            top = tint(&top, *c);
                        }
                        img = overlay(&img, &top);
                    }
                    img
                })
            }
            None => {
                fallback += 1;
                set.require(&format!("fallback:{}", r.fallback), || {
                    resize(&fallback_texture(r.fallback), size)
                })
            }
        };
        layer_table.push(loc.map_or(0, |l| u32::from(l.layer)));
    }
    BlockTextures {
        set,
        layer_table,
        per_source,
        fallback,
        size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn data() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/data/blocks")
    }

    #[test]
    fn grass_has_top_side_and_bottom() {
        let f = read_block_faces(&data());
        let g = &f["core:grass_block"];
        assert_eq!(
            g[0].reference.as_deref(),
            Some("minecraft:block/grass_block_top")
        );
        assert_eq!(g[0].tint.as_deref(), Some("grass"));
        assert_eq!(g[1].reference.as_deref(), Some("minecraft:block/dirt"));
        assert_eq!(
            g[2].reference.as_deref(),
            Some("minecraft:block/grass_block_side")
        );
        assert!(g[2].overlay.is_some() && g[0].overlay.is_none());
        assert_eq!(
            f["core:stone"][4].reference.as_deref(),
            Some("minecraft:block/stone")
        );
    }

    #[test]
    fn without_an_installation_everything_falls_back() {
        let mods = world::modset::source_build(world::modset::DEFAULT_BUILD).expect("build");
        let b = mods.registry_builder().expect("blocks");
        let reg = b.build().expect("registry");
        let t = build(&reg, &read_mod_faces(&mods), &mut []);
        assert_eq!(t.per_source.len(), 0);
        assert_eq!(t.size, 16);
        assert_eq!(t.layer_table.len(), reg.len() * 6);
        assert_eq!(t.set.arrays().len(), 1, "one 16x16 array");
    }
}
