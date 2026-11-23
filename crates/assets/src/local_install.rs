//! Textures from the player's **own** licensed installation, read locally
//! (`docs/architecture/auth-and-assets.md`, "Local installation";
//! `docs/legal.md` rule 2). Nothing read here is ever written into the
//! repository, CI or our builds: the files stay on the player's machine and
//! are read straight from the version's `.jar` each time a world opens.
//!
//! Layout of an installation folder: `versions/<id>/<id>.jar` (the client
//! archive holding `assets/<namespace>/textures/...png`) next to
//! `versions/<id>/<id>.json` (release type and time).

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::AssetError;
use crate::image::Rgba8Image;

/// Environment variable that points at an installation folder (development).
pub const ENV_VAR: &str = "ASSETS_LOCAL_INSTALL";

/// True if `dir` contains installed versions (vanilla, Prism Launcher, or direct archive).
#[must_use]
pub fn has_versions(dir: &Path) -> bool {
    dir.join("versions").is_dir()
        || dir.file_name().is_some_and(|n| n == "versions")
        || dir.join("libraries/com/mojang/minecraft").is_dir()
        || dir.ends_with("libraries/com/mojang/minecraft")
        || (dir.is_file() && dir.extension().is_some_and(|ext| ext == "jar"))
}

/// Usual installation folders for this OS, existing ones first.
#[must_use]
pub fn default_dirs() -> Vec<PathBuf> {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let mut out = Vec::new();
    if let Some(p) = env(ENV_VAR) {
        out.push(p);
    }
    if let Some(a) = env("APPDATA") {
        out.push(a.join(".minecraft"));
        out.push(a.join("PrismLauncher"));
    }
    for drive in ["D", "C", "E"] {
        let p = PathBuf::from(format!(r"{drive}:\Games\Prism Launcher"));
        if p.is_dir() {
            out.push(p);
        }
    }
    if let Some(h) = env("HOME") {
        out.push(h.join("Library/Application Support/minecraft"));
        out.push(h.join(".minecraft"));
        out.push(h.join(".local/share/PrismLauncher"));
    }
    out.sort_by_key(|p| !has_versions(p));
    out
}

/// One installed version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledVersion {
    /// Version id (folder name).
    pub id: String,
    /// The client archive.
    pub jar: PathBuf,
    /// `release`, `snapshot`, … (empty if unknown).
    pub kind: String,
    /// Release time as written in the version file (ISO 8601; sorts as text).
    pub released: String,
}

/// Versions with a client archive in `dir`, newest release first, then
/// other kinds by time. Supports standard vanilla launcher layout,
/// Prism Launcher / MultiMC layout, or direct JAR files.
pub fn versions(dir: &Path) -> Result<Vec<InstalledVersion>, AssetError> {
    if dir.is_file() {
        let name = dir.file_stem().unwrap_or_default().to_string_lossy();
        let id = name
            .strip_prefix("minecraft-")
            .unwrap_or(&name)
            .strip_suffix("-client")
            .unwrap_or(&name)
            .to_string();
        return Ok(vec![InstalledVersion {
            id,
            jar: dir.to_path_buf(),
            kind: "custom".into(),
            released: String::new(),
        }]);
    }

    let mut out = Vec::new();
    let mut searched_any = false;

    // 1. Standard vanilla launcher layout: <dir>/versions/<id>/<id>.jar
    let root = if dir.file_name().is_some_and(|n| n == "versions") {
        dir.to_path_buf()
    } else {
        dir.join("versions")
    };
    if let Ok(entries) = std::fs::read_dir(&root) {
        searched_any = true;
        for e in entries.flatten() {
            let id = e.file_name().to_string_lossy().into_owned();
            let jar = e.path().join(format!("{id}.jar"));
            if !jar.is_file() {
                continue;
            }
            let (kind, released) = std::fs::read_to_string(e.path().join(format!("{id}.json")))
                .ok()
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                .map(|v| {
                    let s = |k: &str| {
                        v.get(k)
                            .and_then(|x| x.as_str())
                            .unwrap_or_default()
                            .to_owned()
                    };
                    (s("type"), s("releaseTime"))
                })
                .unwrap_or_default();
            out.push(InstalledVersion {
                id,
                jar,
                kind,
                released,
            });
        }
    }

    // 2. Prism Launcher / MultiMC layout:
    // <dir>/libraries/com/mojang/minecraft/<id>/minecraft-<id>-client.jar
    let prism_root = if dir.ends_with("libraries/com/mojang/minecraft") {
        dir.to_path_buf()
    } else {
        dir.join("libraries/com/mojang/minecraft")
    };
    if let Ok(entries) = std::fs::read_dir(&prism_root) {
        searched_any = true;
        for e in entries.flatten() {
            let id = e.file_name().to_string_lossy().into_owned();
            let mut jar = e.path().join(format!("minecraft-{id}-client.jar"));
            if !jar.is_file() {
                jar = e.path().join(format!("{id}.jar"));
            }
            if !jar.is_file() {
                if let Ok(sub) = std::fs::read_dir(e.path()) {
                    if let Some(f) = sub
                        .flatten()
                        .find(|f| f.path().extension().is_some_and(|x| x == "jar"))
                    {
                        jar = f.path();
                    }
                }
            }
            if !jar.is_file() {
                continue;
            }

            let meta_path = dir.join("meta/net.minecraft").join(format!("{id}.json"));
            let (kind, released) = std::fs::read_to_string(meta_path)
                .ok()
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                .map(|v| {
                    let s = |k: &str| {
                        v.get(k)
                            .and_then(|x| x.as_str())
                            .unwrap_or_default()
                            .to_owned()
                    };
                    (s("type"), s("releaseTime"))
                })
                .unwrap_or_else(|| {
                    let kind = if id.contains('w') || id.contains("pre") || id.contains("rc") {
                        "snapshot"
                    } else {
                        "release"
                    };
                    (kind.to_string(), String::new())
                });

            out.push(InstalledVersion {
                id,
                jar,
                kind,
                released,
            });
        }
    }

    if !searched_any {
        return Err(AssetError::Io(format!(
            "{}: neither versions nor libraries/com/mojang/minecraft directory exists",
            dir.display()
        )));
    }

    out.sort_by(|a, b| {
        (b.kind == "release")
            .cmp(&(a.kind == "release"))
            .then(b.released.cmp(&a.released))
            .then(b.id.cmp(&a.id))
    });
    out.dedup_by(|a, b| a.id == b.id);
    Ok(out)
}

/// An opened client archive.
pub struct InstalledTextures {
    archive: zip::ZipArchive<File>,
    /// The version read.
    pub version: String,
}

impl InstalledTextures {
    /// Opens `version` (or the newest release) of the installation in `dir`.
    pub fn open(dir: &Path, version: Option<&str>) -> Result<Self, AssetError> {
        if dir.is_file() {
            let id = version
                .map(ToOwned::to_owned)
                .or_else(|| {
                    let stem = dir.file_stem()?.to_string_lossy();
                    let s = stem
                        .strip_prefix("minecraft-")
                        .unwrap_or(&stem)
                        .strip_suffix("-client")
                        .unwrap_or(&stem);
                    Some(s.to_string())
                })
                .unwrap_or_else(|| "custom".to_string());
            return Self::open_jar(dir, &id);
        }
        let all = versions(dir)?;
        let v = match version {
            Some(want) => all.into_iter().find(|v| v.id == want),
            None => all.into_iter().next(),
        }
        .ok_or_else(|| {
            AssetError::Io(format!("no installed version found in {}", dir.display()))
        })?;
        Self::open_jar(&v.jar, &v.id)
    }

    /// Opens a client archive directly.
    pub fn open_jar(jar: &Path, version: &str) -> Result<Self, AssetError> {
        let file =
            File::open(jar).map_err(|e| AssetError::Io(format!("{}: {e}", jar.display())))?;
        let archive = zip::ZipArchive::new(file)
            .map_err(|e| AssetError::Corrupt(format!("{}: {e}", jar.display())))?;
        Ok(Self {
            archive,
            version: version.to_owned(),
        })
    }

    /// A texture by reference (`namespace:path`, e.g. `minecraft:block/stone`)
    /// as RGBA8. Animated strips (taller than wide) give their first frame.
    /// `None` if the archive has no such texture or it does not decode.
    pub fn texture(&mut self, reference: &str) -> Option<Rgba8Image> {
        let (ns, path) = reference
            .split_once(':')
            .unwrap_or(("minecraft", reference));
        let name = format!("assets/{ns}/textures/{path}.png");
        let mut bytes = Vec::new();
        self.archive
            .by_name(&name)
            .ok()?
            .read_to_end(&mut bytes)
            .ok()?;
        decode_png(&bytes)
    }
}

/// Decodes a PNG into RGBA8 (any color type and bit depth); an animated strip
/// is cropped to its first square frame.
#[must_use]
pub fn decode_png(bytes: &[u8]) -> Option<Rgba8Image> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width, info.height);
    let channels = info.color_type.samples();
    let frame = h.min(w);
    let mut pixels = Vec::with_capacity((w * frame) as usize);
    for y in 0..frame {
        for x in 0..w {
            let i = ((y * w + x) as usize) * channels;
            let px = &buf[i..i + channels];
            pixels.push(match channels {
                1 => [px[0], px[0], px[0], 255],
                2 => [px[0], px[0], px[0], px[1]],
                3 => [px[0], px[1], px[2], 255],
                _ => [px[0], px[1], px[2], px[3]],
            });
        }
    }
    Rgba8Image::from_pixels(w, frame, pixels)
}

/// Multiplies an image by a color (biome tint of gray textures).
#[must_use]
pub fn tint(image: &Rgba8Image, color: [u8; 3]) -> Rgba8Image {
    let px = image
        .pixels()
        .iter()
        .map(|p| {
            let m = |c: u8, t: u8| ((u16::from(c) * u16::from(t)) / 255) as u8;
            [
                m(p[0], color[0]),
                m(p[1], color[1]),
                m(p[2], color[2]),
                p[3],
            ]
        })
        .collect();
    Rgba8Image::from_pixels(image.width(), image.height(), px).unwrap_or_else(|| image.clone())
}

/// Draws `top` over `base` with alpha (same size, else `base` unchanged).
#[must_use]
pub fn overlay(base: &Rgba8Image, top: &Rgba8Image) -> Rgba8Image {
    if base.width() != top.width() || base.height() != top.height() {
        return base.clone();
    }
    let px = base
        .pixels()
        .iter()
        .zip(top.pixels())
        .map(|(b, t)| {
            let a = u16::from(t[3]);
            let mix = |x: u8, y: u8| ((u16::from(y) * a + u16::from(x) * (255 - a)) / 255) as u8;
            [
                mix(b[0], t[0]),
                mix(b[1], t[1]),
                mix(b[2], t[2]),
                b[3].max(t[3]),
            ]
        })
        .collect();
    Rgba8Image::from_pixels(base.width(), base.height(), px).unwrap_or_else(|| base.clone())
}

/// Nearest-neighbour resize to a square of `size` (keeps one texture array).
#[must_use]
pub fn resize(image: &Rgba8Image, size: u32) -> Rgba8Image {
    if image.width() == size && image.height() == size {
        return image.clone();
    }
    let mut out = Rgba8Image::filled(size, size, [0; 4]);
    for y in 0..size {
        for x in 0..size {
            out.set(
                x,
                y,
                image.get(x * image.width() / size, y * image.height() / size),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PNG made by our own encoder from our own pixels (no third-party file).
    pub(crate) fn png_bytes(w: u32, h: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(color);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().expect("header");
        writer.write_image_data(data).expect("data");
        drop(writer);
        out
    }

    #[test]
    fn decodes_gray_rgba_and_crops_strips() {
        let gray = png_bytes(
            2,
            4,
            png::ColorType::Grayscale,
            &[10, 20, 30, 40, 50, 60, 70, 80],
        );
        let img = decode_png(&gray).expect("decode");
        assert_eq!(
            (img.width(), img.height()),
            (2, 2),
            "first frame of a strip"
        );
        assert_eq!(img.get(1, 1), [40, 40, 40, 255]);
        let rgba = png_bytes(1, 1, png::ColorType::Rgba, &[1, 2, 3, 4]);
        assert_eq!(decode_png(&rgba).expect("decode").get(0, 0), [1, 2, 3, 4]);
        assert!(decode_png(b"not a png").is_none());
    }

    #[test]
    fn tint_overlay_resize() {
        let white = Rgba8Image::filled(2, 2, [255, 255, 255, 255]);
        assert_eq!(tint(&white, [10, 20, 30]).get(0, 0), [10, 20, 30, 255]);
        let half = Rgba8Image::filled(2, 2, [0, 0, 0, 0]);
        assert_eq!(overlay(&white, &half).get(1, 1), [255, 255, 255, 255]);
        let big = resize(&white, 4);
        assert_eq!((big.width(), big.get(3, 3)), (4, [255, 255, 255, 255]));
    }

    #[test]
    fn reads_a_generated_installation() {
        let dir = std::env::temp_dir().join(format!("assets-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (id, kind, time) in [
            ("1.0", "release", "2020-01-01"),
            ("2.0", "release", "2021-01-01"),
            ("3.0-pre", "snapshot", "2022-01-01"),
        ] {
            let vd = dir.join("versions").join(id);
            std::fs::create_dir_all(&vd).expect("dir");
            std::fs::write(
                vd.join(format!("{id}.json")),
                format!(r#"{{"type":"{kind}","releaseTime":"{time}"}}"#),
            )
            .expect("json");
            let f = File::create(vd.join(format!("{id}.jar"))).expect("jar");
            let mut z = zip::ZipWriter::new(f);
            z.start_file(
                "assets/minecraft/textures/block/stone.png",
                zip::write::SimpleFileOptions::default(),
            )
            .expect("entry");
            std::io::Write::write_all(&mut z, &png_bytes(1, 1, png::ColorType::Rgb, &[9, 8, 7]))
                .expect("write");
            z.finish().expect("finish");
        }
        let v = versions(&dir).expect("versions");
        assert_eq!(
            v.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(),
            ["2.0", "1.0", "3.0-pre"]
        );
        let mut t = InstalledTextures::open(&dir, None).expect("open");
        assert_eq!(t.version, "2.0");
        assert_eq!(
            t.texture("minecraft:block/stone").expect("stone").get(0, 0),
            [9, 8, 7, 255]
        );
        assert!(t.texture("minecraft:block/missing").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reads_prism_launcher_installation() {
        let dir = std::env::temp_dir().join(format!("assets-prism-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let id = "1.21.4";
        let sub = dir.join("libraries/com/mojang/minecraft").join(id);
        std::fs::create_dir_all(&sub).expect("dir");
        let meta = dir.join("meta/net.minecraft");
        std::fs::create_dir_all(&meta).expect("meta");
        std::fs::write(
            meta.join(format!("{id}.json")),
            r#"{"type":"release","releaseTime":"2024-12-03"}"#,
        )
        .expect("json");
        let f = File::create(sub.join(format!("minecraft-{id}-client.jar"))).expect("jar");
        let mut z = zip::ZipWriter::new(f);
        z.start_file(
            "assets/minecraft/textures/block/stone.png",
            zip::write::SimpleFileOptions::default(),
        )
        .expect("entry");
        std::io::Write::write_all(&mut z, &png_bytes(1, 1, png::ColorType::Rgb, &[12, 34, 56]))
            .expect("write");
        z.finish().expect("finish");

        assert!(has_versions(&dir));
        let v = versions(&dir).expect("versions");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, id);
        assert_eq!(v[0].kind, "release");

        let mut t = InstalledTextures::open(&dir, Some("1.21.4")).expect("open");
        assert_eq!(t.version, "1.21.4");
        assert_eq!(
            t.texture("minecraft:block/stone").expect("stone").get(0, 0),
            [12, 34, 56, 255]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Opt-in: reads the developer's own installation when `ASSETS_LOCAL_INSTALL`
    /// is set (never in CI) and prints how long the textures take.
    #[test]
    fn own_installation_when_configured() {
        let Some(dir) = std::env::var_os(ENV_VAR) else {
            eprintln!("skipped: set {ENV_VAR} to your own installation folder to run this test");
            return;
        };
        let t0 = std::time::Instant::now();
        let mut t = InstalledTextures::open(Path::new(&dir), None).expect("open the installation");
        let names = [
            "stone",
            "dirt",
            "grass_block_top",
            "grass_block_side",
            "sand",
            "oak_log",
            "oak_leaves",
            "water_still",
        ];
        for n in names {
            let img = t
                .texture(&format!("minecraft:block/{n}"))
                .unwrap_or_else(|| panic!("{n}"));
            assert!(img.width() >= 16, "{n}");
        }
        eprintln!(
            "version {}: {} textures in {:?}",
            t.version,
            names.len(),
            t0.elapsed()
        );
    }
}
