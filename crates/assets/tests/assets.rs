//! Golden and cache tests on fallback assets only (CI never sees Mojang files).

use std::path::PathBuf;

use assets::cache::{CONVERTER_VERSION, decode, encode};
use assets::{
    AssetCache, AssetError, FALLBACK_KEYS, Rgba8Image, TextureArray, TextureSet, fallback_key,
    fallback_set, fallback_texture,
};

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/fallback-16.vxta")
}

fn fallback_array() -> TextureArray {
    let set = fallback_set(FALLBACK_KEYS).expect("fallback set builds");
    assert_eq!(set.arrays().len(), 1, "all fallback textures are 16x16");
    set.arrays()[0].clone()
}

/// The converted fallback array is byte-identical to the committed golden file.
/// Regenerate after an intended change with `UPDATE_GOLDEN=1 cargo test -p assets`
/// and bump `CONVERTER_VERSION`.
#[test]
fn fallback_array_matches_golden_file() {
    let bytes = encode(&fallback_array());
    let path = golden_path();
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, &bytes).expect("write golden");
    }
    let golden = std::fs::read(&path).expect("golden file exists; run with UPDATE_GOLDEN=1");
    assert_eq!(
        bytes, golden,
        "fallback conversion changed: bump CONVERTER_VERSION"
    );
    let decoded = decode(&golden).expect("golden decodes");
    assert_eq!(decoded.names().len(), FALLBACK_KEYS.len());
    assert_eq!(decoded.levels().len(), 5, "16, 8, 4, 2, 1");
}

#[test]
fn every_core_block_has_its_own_fallback_texture() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/data/blocks");
    let missing = fallback_texture("missing");
    let mut count = 0;
    for entry in std::fs::read_dir(dir).expect("core blocks") {
        let path = entry.expect("entry").path();
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("utf-8 name");
        let key = fallback_key(stem);
        assert!(FALLBACK_KEYS.contains(&key), "no fallback look for {key}");
        assert_ne!(fallback_texture(key), missing, "{key}");
        count += 1;
    }
    assert!(count >= 15);
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("assets-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn cache_round_trip_and_hit() {
    let dir = temp_dir("roundtrip");
    let cache = AssetCache::new(&dir);
    let source = b"fallback set v1";
    let (first, hit) = cache
        .get_or_convert("core", source, || Ok(fallback_array()))
        .expect("convert");
    assert!(!hit);
    let (second, hit) = cache
        .get_or_convert("core", source, || panic!("must come from the cache"))
        .expect("load");
    assert!(hit);
    assert_eq!(first, second);
    assert!(
        cache
            .path("x")
            .starts_with(dir.join(CONVERTER_VERSION.to_string()))
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn other_converter_version_is_not_read() {
    let mut bytes = encode(&fallback_array());
    bytes[8..12].copy_from_slice(&(CONVERTER_VERSION + 1).to_le_bytes());
    assert_eq!(decode(&bytes), Err(AssetError::Stale));
    // A stale file in the current directory is treated as a miss and rewritten.
    let dir = temp_dir("stale");
    let cache = AssetCache::new(&dir);
    let key = assets::cache::cache_key("core", b"src");
    std::fs::create_dir_all(cache.path(&key).parent().expect("dir")).expect("mkdir");
    std::fs::write(cache.path(&key), &bytes).expect("write stale");
    let (_, hit) = cache
        .get_or_convert("core", b"src", || Ok(fallback_array()))
        .expect("reconvert");
    assert!(!hit);
    assert!(cache.load(&key).expect("readable").is_some());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn corrupt_files_are_rejected() {
    let bytes = encode(&fallback_array());
    for cut in [0, 3, 10, 30, bytes.len() - 1] {
        assert!(decode(&bytes[..cut]).is_err(), "cut at {cut}");
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(decode(&extra).is_err());
}

#[test]
fn lazy_set_loads_once_and_reports_new_layers() {
    let mut set = TextureSet::new();
    let mut loads = 0;
    let a = set
        .require("stone", || {
            loads += 1;
            fallback_texture("stone")
        })
        .expect("ok");
    let again = set
        .require("stone", || {
            loads += 1;
            fallback_texture("stone")
        })
        .expect("ok");
    assert_eq!((a, loads), (again, 1));
    let big = set
        .require("big", || Rgba8Image::filled(32, 32, [1, 2, 3, 255]))
        .expect("ok");
    assert_ne!(big.array, a.array, "one array per size");
    let mut added = Vec::new();
    set.take_added(&mut added);
    assert_eq!(added, vec![a, big]);
    set.take_added(&mut added);
    assert_eq!(added.len(), 2, "drained");
    assert!(matches!(
        set.require("odd", || Rgba8Image::filled(12, 12, [0; 4])),
        Err(AssetError::BadSize(12, 12))
    ));
}

#[test]
fn arrays_split_at_256_layers() {
    let mut set = TextureSet::new();
    for i in 0..300 {
        set.require(&format!("t{i}"), || {
            Rgba8Image::filled(4, 4, [i as u8, 0, 0, 255])
        })
        .expect("ok");
    }
    assert_eq!(set.arrays().len(), 2);
    assert_eq!(set.arrays()[0].layers(), 256);
    assert_eq!(
        set.location("t299").map(|l| (l.array, l.layer)),
        Some((1, 43))
    );
}
