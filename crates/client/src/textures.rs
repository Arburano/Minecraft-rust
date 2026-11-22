//! Block textures for the renderer: per face, from the player's own
//! installation when available (T1-14), otherwise our fallback textures.
//!
//! Block files name textures per face (`textures = { all | top | bottom |
//! side | north | south | east | west | side_overlay = "ns:path" }`) and
//! optional tints for gray textures (`tint = { <face> = "grass" | "foliage" |
//! "water" }`).

use std::collections::HashMap;
use std::path::Path;

use assets::local_install::{InstalledTextures, overlay, resize, tint};
use assets::{FALLBACK_KEYS, Rgba8Image, TextureSet, fallback_key, fallback_texture};
use world::{BlockStateId, Registry};

/// Edge of every block texture (one texture array; larger textures are scaled).
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
    /// Texture arrays (all layers `TEXTURE_SIZE`).
    pub set: TextureSet,
    /// `state * 6 + face` → layer.
    pub layer_table: Vec<u32>,
    /// Faces drawn with the player's own textures.
    pub from_installation: usize,
    /// Faces drawn with our fallback textures.
    pub fallback: usize,
}

/// Builds every state's six face textures. With `installed`, each face is read
/// from it (tinted, overlay applied); a face it lacks uses the block's
/// fallback texture.
#[must_use]
pub fn build(
    registry: &Registry,
    faces: &HashMap<String, BlockFaces>,
    mut installed: Option<&mut InstalledTextures>,
) -> BlockTextures {
    let mut set = TextureSet::new();
    let mut layer_table = Vec::with_capacity(registry.len() * 6);
    let (mut from_installation, mut fallback) = (0, 0);
    let mut cache: HashMap<String, Option<Rgba8Image>> = HashMap::new();
    let mut read =
        |installed: &mut Option<&mut InstalledTextures>, r: &str| -> Option<Rgba8Image> {
            cache
                .entry(r.to_owned())
                .or_insert_with(|| installed.as_mut().and_then(|i| i.texture(r)))
                .clone()
        };
    for i in 0..registry.len() as u16 {
        let name = registry.name(BlockStateId(i)).unwrap_or("missing");
        let block = name.split('[').next().unwrap_or(name);
        let key = fallback_key(block);
        let key = if FALLBACK_KEYS.contains(&key) {
            key
        } else {
            "missing"
        };
        let default = BlockFaces::default();
        let block_faces = faces.get(block).unwrap_or(&default);
        for face in block_faces {
            let own = face.reference.as_deref().and_then(|r| {
                let mut img = resize(&read(&mut installed, r)?, TEXTURE_SIZE);
                if let Some(c) = face.tint.as_deref().and_then(tint_color) {
                    img = tint(&img, c);
                }
                if let Some((o, t)) = &face.overlay
                    && let Some(top) = read(&mut installed, o)
                {
                    let mut top = resize(&top, TEXTURE_SIZE);
                    if let Some(c) = t.as_deref().and_then(tint_color) {
                        top = tint(&top, c);
                    }
                    img = overlay(&img, &top);
                }
                Some((format!("own:{r}|{:?}|{:?}", face.tint, face.overlay), img))
            });
            let loc = match own {
                Some((id, img)) => {
                    from_installation += 1;
                    set.require(&id, || img)
                }
                None => {
                    fallback += 1;
                    set.require(&format!("fallback:{key}"), || fallback_texture(key))
                }
            };
            layer_table.push(loc.map_or(0, |l| u32::from(l.layer)));
        }
    }
    BlockTextures {
        set,
        layer_table,
        from_installation,
        fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use world::RegistryBuilder;
    use world::registry::blocks_from_dir;

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
        let mut b = RegistryBuilder::new();
        b.add_mod("core", blocks_from_dir(&data()).expect("blocks"));
        let reg = b.build().expect("registry");
        let t = build(&reg, &read_block_faces(&data()), None);
        assert_eq!(t.from_installation, 0);
        assert_eq!(t.layer_table.len(), reg.len() * 6);
        assert_eq!(t.set.arrays().len(), 1, "one 16x16 array");
    }
}
