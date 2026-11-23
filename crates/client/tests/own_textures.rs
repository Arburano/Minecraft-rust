//! T1-14: block textures come from an installation's archive when present
//! (here a generated one: our own pixels, never third-party files), tinted
//! and composited per face; missing ones fall back.

use std::io::Write;
use std::path::PathBuf;

use assets::local_install::InstalledTextures;
use client::textures::{build, read_block_faces};
use world::RegistryBuilder;
use world::registry::blocks_from_dir;

fn png(rgba: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, 16, 16);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().expect("header");
    w.write_image_data(&rgba.repeat(256)).expect("data");
    drop(w);
    out
}

#[test]
fn faces_come_from_the_installation_with_tints() {
    let dir = std::env::temp_dir().join(format!("client-own-textures-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let vd = dir.join("versions/test");
    std::fs::create_dir_all(&vd).expect("dir");
    std::fs::write(
        vd.join("test.json"),
        r#"{"type":"release","releaseTime":"2026"}"#,
    )
    .expect("json");
    let mut z = zip::ZipWriter::new(std::fs::File::create(vd.join("test.jar")).expect("jar"));
    for (name, px) in [
        ("stone", [100, 110, 120, 255]),
        ("grass_block_top", [255, 255, 255, 255]),
        ("dirt", [90, 60, 30, 255]),
    ] {
        z.start_file(
            format!("assets/minecraft/textures/block/{name}.png"),
            zip::write::SimpleFileOptions::default(),
        )
        .expect("entry");
        z.write_all(&png(px)).expect("write");
    }
    z.finish().expect("finish");

    let blocks = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/data/blocks");
    let mut b = RegistryBuilder::new();
    b.add_mod("core", blocks_from_dir(&blocks).expect("blocks"));
    let reg = b.build().expect("registry");
    let mut installed = InstalledTextures::open(&dir, None).expect("open");
    let t = build(&reg, &read_block_faces(&blocks), &mut [&mut installed]);
    assert!(
        t.per_source[0] > 0 && t.fallback > 0,
        "{:?} own, {} fallback",
        t.per_source,
        t.fallback
    );

    let layer = |block: &str, face: usize| {
        let id = reg.state_id(block).expect(block);
        t.layer_table[usize::from(id.0) * 6 + face] as usize
    };
    let pixel = |layer: usize| {
        let level0 = &t.set.arrays()[0].levels()[0];
        let i = layer * 16 * 16 * 4;
        [level0[i], level0[i + 1], level0[i + 2]]
    };
    assert_eq!(
        pixel(layer("core:stone", 0)),
        [100, 110, 120],
        "stone from the archive"
    );
    assert_eq!(
        pixel(layer("core:grass_block", 0)),
        [145, 189, 89],
        "white top tinted grass green"
    );
    assert_eq!(
        pixel(layer("core:grass_block", 1)),
        [90, 60, 30],
        "grass bottom is dirt"
    );
    let _ = std::fs::remove_dir_all(dir);
}

fn pack_with(
    dir: &std::path::Path,
    name: &str,
    size: u32,
    textures: &[(&str, [u8; 4])],
) -> client::packs::PackFiles {
    let p = dir.join(name);
    std::fs::create_dir_all(p.join("assets/minecraft/textures/block")).expect("dir");
    std::fs::write(p.join("pack.toml"), format!("name = \"{name}\"")).expect("toml");
    for (n, px) in textures {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, size, size);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().expect("header");
        w.write_image_data(&px.repeat((size * size) as usize))
            .expect("data");
        drop(w);
        std::fs::write(
            p.join(format!("assets/minecraft/textures/block/{n}.png")),
            out,
        )
        .expect("png");
    }
    client::packs::PackFiles::open(&p).expect("open")
}

#[test]
fn packs_stack_top_first_fall_through_and_keep_hd_detail() {
    let dir = std::env::temp_dir().join(format!("client-pack-stack-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut top = pack_with(&dir, "top", 64, &[("stone", [200, 0, 0, 255])]);
    let mut lower = pack_with(
        &dir,
        "lower",
        16,
        &[("stone", [0, 200, 0, 255]), ("dirt", [0, 0, 200, 255])],
    );
    let blocks = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/data/blocks");
    let mut b = RegistryBuilder::new();
    b.add_mod("core", blocks_from_dir(&blocks).expect("blocks"));
    let reg = b.build().expect("registry");
    let faces = read_block_faces(&blocks);
    let t = build(&reg, &faces, &mut [&mut top, &mut lower]);
    assert_eq!(t.size, 64, "the HD texture sets the layer size");
    let layer =
        |block: &str| t.layer_table[usize::from(reg.state_id(block).expect(block).0) * 6] as usize;
    let pixel = |layer: usize| {
        let level0 = &t.set.arrays()[0].levels()[0];
        let i = layer * 64 * 64 * 4;
        [level0[i], level0[i + 1], level0[i + 2]]
    };
    assert_eq!(pixel(layer("core:stone")), [200, 0, 0], "the top pack wins");
    assert_eq!(
        pixel(layer("core:dirt")),
        [0, 0, 200],
        "falls through to the lower pack"
    );
    assert!(t.per_source[0] > 0 && t.per_source[1] > 0 && t.fallback > 0);
    // Without packs: exactly the fallback look at 16 px.
    let none = build(&reg, &faces, &mut []);
    assert_eq!((none.size, none.per_source.len()), (16, 0));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn real_installation_when_configured() {
    let Some(dir) = std::env::var_os(assets::local_install::ENV_VAR) else {
        eprintln!("skipped: set ASSETS_LOCAL_INSTALL to your own installation folder");
        return;
    };
    let blocks = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/data/blocks");
    let mut b = RegistryBuilder::new();
    b.add_mod("core", blocks_from_dir(&blocks).expect("blocks"));
    let reg = b.build().expect("registry");
    let t0 = std::time::Instant::now();
    let mut installed = InstalledTextures::open(std::path::Path::new(&dir), None).expect("open");
    let t = build(&reg, &read_block_faces(&blocks), &mut [&mut installed]);
    let elapsed = t0.elapsed();
    eprintln!(
        "real installation (version {}): {} faces from installation (+{} fallback) in {:?}",
        installed.version, t.per_source[0], t.fallback, elapsed
    );
    assert!(t.per_source[0] > 0);
}
