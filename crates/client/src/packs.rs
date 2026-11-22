//! Player packs (ADR-0014): resource and texture packs in
//! `<data>/resourcepacks/`, shader packs in `<data>/shaderpacks/`. Each is a
//! folder or a `.zip`. Nothing here ships with the game; packs are what the
//! player puts there.
//!
//! - Resource pack: `pack.toml` (`name`, `description`) — or the common
//!   community `pack.mcmeta` — and `assets/<namespace>/textures/<path>.png`.
//! - Shader pack: `shaderpack.toml` (`name`, `description`, `[[option]]`
//!   with `id`, `label`, `min`, `max`, `default`), `shade.wgsl` (terrain
//!   hooks) and/or `post.wgsl` (`pack_post`).

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use assets::Rgba8Image;
use assets::local_install::{InstalledTextures, decode_png};
use serde::Deserialize;

/// Where a pack's files are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackInfo {
    /// Folder or file name (the id stored in settings).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Folder or `.zip`.
    pub path: PathBuf,
}

/// Reads files from a folder or a zip.
pub enum PackFiles {
    /// A folder.
    Folder(PathBuf),
    /// A zip archive.
    Zip(Box<zip::ZipArchive<File>>),
}

impl PackFiles {
    /// Opens a pack folder or `.zip`.
    pub fn open(path: &Path) -> Result<Self, String> {
        if path.is_dir() {
            return Ok(Self::Folder(path.to_owned()));
        }
        let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        zip::ZipArchive::new(file)
            .map(|z| Self::Zip(Box::new(z)))
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    /// A file's bytes by pack-relative path (`/` separators).
    pub fn read(&mut self, name: &str) -> Option<Vec<u8>> {
        match self {
            Self::Folder(dir) => std::fs::read(dir.join(name)).ok(),
            Self::Zip(z) => {
                let mut f = z.by_name(name).ok()?;
                let mut out = Vec::new();
                f.read_to_end(&mut out).ok()?;
                Some(out)
            }
        }
    }

    fn read_text(&mut self, name: &str) -> Option<String> {
        self.read(name).and_then(|b| String::from_utf8(b).ok())
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PackToml {
    name: String,
    description: String,
}

fn pack_meta(files: &mut PackFiles, manifest: &str) -> Option<(String, String)> {
    if let Some(t) = files.read_text(manifest) {
        let m: PackToml = toml::from_str(&t).ok()?;
        return Some((m.name, m.description));
    }
    if manifest == "pack.toml"
        && let Some(t) = files.read_text("pack.mcmeta")
    {
        let v: serde_json::Value = serde_json::from_str(&t).ok()?;
        let desc = &v["pack"]["description"];
        let d = desc
            .as_str()
            .map(str::to_owned)
            .or_else(|| desc.get("text").and_then(|x| x.as_str()).map(str::to_owned))
            .unwrap_or_default();
        return Some((String::new(), d));
    }
    None
}

fn list(dir: &Path, manifest: &str) -> Vec<PackInfo> {
    let mut out: Vec<PackInfo> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let id = e.file_name().to_string_lossy().into_owned();
            let is_zip = path
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("zip"));
            if !path.is_dir() && !is_zip {
                return None;
            }
            let mut files = PackFiles::open(&path).ok()?;
            let (name, description) = pack_meta(&mut files, manifest)?;
            let name = if name.is_empty() {
                id.trim_end_matches(".zip").to_owned()
            } else {
                name
            };
            Some(PackInfo {
                id,
                name,
                description,
                path,
            })
        })
        .collect();
    out.sort_by_key(|p| p.name.to_lowercase());
    out
}

/// Resource packs in `dir` (folders or zips with `pack.toml` or `pack.mcmeta`).
#[must_use]
pub fn list_resource_packs(dir: &Path) -> Vec<PackInfo> {
    list(dir, "pack.toml")
}

/// Shader packs in `dir` (folders or zips with `shaderpack.toml`).
#[must_use]
pub fn list_shader_packs(dir: &Path) -> Vec<PackInfo> {
    list(dir, "shaderpack.toml")
}

/// Anything that can provide a texture by reference (`namespace:path`).
pub trait TextureSource {
    /// The texture, if this source has it.
    fn texture(&mut self, reference: &str) -> Option<Rgba8Image>;
}

impl TextureSource for InstalledTextures {
    fn texture(&mut self, reference: &str) -> Option<Rgba8Image> {
        InstalledTextures::texture(self, reference)
    }
}

impl TextureSource for PackFiles {
    fn texture(&mut self, reference: &str) -> Option<Rgba8Image> {
        let (ns, path) = reference
            .split_once(':')
            .unwrap_or(("minecraft", reference));
        decode_png(&self.read(&format!("assets/{ns}/textures/{path}.png"))?)
    }
}

/// One option a shader pack exposes as a slider.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct PackOption {
    /// Stable id (stored in settings).
    pub id: String,
    /// Slider label.
    #[serde(default)]
    pub label: String,
    /// Lowest value.
    #[serde(default)]
    pub min: f32,
    /// Highest value.
    #[serde(default = "one")]
    pub max: f32,
    /// Default value.
    #[serde(default)]
    pub default: f32,
}

fn one() -> f32 {
    1.0
}

#[derive(Deserialize)]
struct ShaderPackToml {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    hook_version: Option<u32>,
    #[serde(default, rename = "option")]
    options: Vec<PackOption>,
}

/// A loaded shader pack.
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderPack {
    /// Display name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Sliders (at most 8; `pack_option(i)` in WGSL is the i-th).
    pub options: Vec<PackOption>,
    /// Terrain hooks (`shade.wgsl`).
    pub shade: Option<String>,
    /// Post pass (`post.wgsl`).
    pub post: Option<String>,
}

/// Loads a shader pack's files (validation happens when it is applied).
pub fn load_shader_pack(info: &PackInfo) -> Result<ShaderPack, String> {
    let mut files = PackFiles::open(&info.path)?;
    let text = files
        .read_text("shaderpack.toml")
        .ok_or_else(|| format!("{}: no shaderpack.toml", info.id))?;
    let t: ShaderPackToml = toml::from_str(&text).map_err(|e| format!("{}: {e}", info.id))?;
    if let Some(v) = t.hook_version
        && v != render::terrain::hooks::HOOK_VERSION
    {
        return Err(format!(
            "{} targets shader hook version {v}; this game has version {}",
            info.id,
            render::terrain::hooks::HOOK_VERSION
        ));
    }
    if t.options.len() > render::terrain::hooks::MAX_OPTIONS {
        return Err(format!(
            "{}: at most {} options",
            info.id,
            render::terrain::hooks::MAX_OPTIONS
        ));
    }
    let shade = files.read_text("shade.wgsl");
    let post = files.read_text("post.wgsl");
    if shade.is_none() && post.is_none() {
        return Err(format!("{}: neither shade.wgsl nor post.wgsl", info.id));
    }
    Ok(ShaderPack {
        name: if t.name.is_empty() {
            info.name.clone()
        } else {
            t.name
        },
        description: t.description,
        options: t.options,
        shade,
        post,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_zips_and_community_manifests() {
        let dir = std::env::temp_dir().join(format!("client-packs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = dir.join("a_pack");
        std::fs::create_dir_all(a.join("assets/minecraft/textures/block")).expect("dir");
        std::fs::write(
            a.join("pack.toml"),
            "name = \"Alpha\"\ndescription = \"ours\"",
        )
        .expect("toml");
        let b = dir.join("community.zip");
        let mut z = zip::ZipWriter::new(File::create(&b).expect("zip"));
        z.start_file("pack.mcmeta", zip::write::SimpleFileOptions::default())
            .expect("entry");
        std::io::Write::write_all(
            &mut z,
            br#"{"pack":{"pack_format":34,"description":"from a zip"}}"#,
        )
        .expect("write");
        z.finish().expect("finish");
        std::fs::create_dir_all(dir.join("not_a_pack")).expect("dir");
        let l = list_resource_packs(&dir);
        assert_eq!(l.len(), 2);
        assert_eq!(
            (l[0].name.as_str(), l[0].description.as_str()),
            ("Alpha", "ours")
        );
        assert_eq!(
            (l[1].id.as_str(), l[1].name.as_str()),
            ("community.zip", "community")
        );
        assert_eq!(l[1].description, "from a zip");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn shader_pack_options_and_errors() {
        let dir = std::env::temp_dir().join(format!("client-shaderpacks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = dir.join("glow");
        std::fs::create_dir_all(&p).expect("dir");
        std::fs::write(
            p.join("shaderpack.toml"),
            "name = \"Glow\"\nhook_version = 1\n[[option]]\nid = \"strength\"\nlabel = \"Strength\"\nmin = 0.0\nmax = 2.0\ndefault = 1.0\n",
        )
        .expect("toml");
        std::fs::write(
            p.join("post.wgsl"),
            "fn pack_post(c: vec3<f32>, uv: vec2<f32>, d: f32) -> vec3<f32> { return c; }",
        )
        .expect("wgsl");
        let info = &list_shader_packs(&dir)[0];
        let pack = load_shader_pack(info).expect("load");
        assert_eq!(pack.name, "Glow");
        assert_eq!(pack.options[0].default, 1.0);
        assert!(pack.shade.is_none() && pack.post.is_some());
        std::fs::write(p.join("shaderpack.toml"), "hook_version = 9").expect("toml");
        assert!(load_shader_pack(info).is_err(), "wrong hook version");
        let _ = std::fs::remove_dir_all(dir);
    }
}
