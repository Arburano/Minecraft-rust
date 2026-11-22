//! On-disk world format (T1-02): round trips, remapping across mod sets,
//! golden region file, version checks and the background writer job.

use std::path::PathBuf;
use std::sync::Arc;

use engine_core::jobs::{CancelToken, JobPool, Priority};
use proptest::prelude::*;
use world::storage::{ColumnRecord, RegionFile, StorageError, WorldStore, decode_column};
use world::{
    BlockDef, BlockProps, BlockStateId, ColumnPos, Registry, RegistryBuilder, Section, WorldBounds,
};
use worldgen::test_corpus::{CorpusArea, test_corpus};

const BOUNDS: WorldBounds = WorldBounds {
    min_section_y: 0,
    section_count: 8,
};

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("world-store-it-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// A registry whose numeric IDs 0..=9 match the T0-12 corpus IDs.
fn corpus_registry(extra_first: bool, drop: Option<&str>) -> Registry {
    let mut blocks = Vec::new();
    if extra_first {
        blocks.push(BlockDef::simple("a:inserted", BlockProps::MISSING));
    }
    for i in 2..10 {
        let name = format!("t:b{i}");
        if Some(name.as_str()) != drop {
            blocks.push(BlockDef::simple(&name, BlockProps::MISSING));
        }
    }
    let mut b = RegistryBuilder::new();
    b.add_mod("t", blocks);
    b.build().expect("valid registry")
}

fn corpus_records() -> Vec<ColumnRecord> {
    let area = CorpusArea {
        x: 0..4,
        y: 0..8,
        z: 0..4,
    };
    test_corpus(0, &area)
        .into_iter()
        .map(|c| ColumnRecord {
            pos: ColumnPos { x: c.x, z: c.z },
            bounds: BOUNDS,
            sections: c.sections().to_vec(),
            light: vec![(c.x + c.z) as u8; 7],
            entities: Vec::new(),
        })
        .collect()
}

fn same_blocks(a: &[Section], b: &[Section]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| (0..4096).all(|i| x.get(i) == y.get(i)))
}

#[test]
fn save_all_load_reopen() {
    let dir = temp("reopen");
    let reg = corpus_registry(false, None);
    let records = corpus_records();
    {
        let mut store = WorldStore::open(&dir, &reg, BOUNDS).expect("open");
        store.save_all(&records).expect("save");
    }
    let mut store = WorldStore::open(&dir, &reg, BOUNDS).expect("reopen");
    for r in &records {
        let back = store.load(r.pos).expect("load").expect("saved");
        assert!(same_blocks(&back.sections, &r.sections), "{:?}", r.pos);
        assert_eq!(back.light, r.light);
    }
    assert!(
        store
            .load(ColumnPos { x: 99, z: -99 })
            .expect("load")
            .is_none()
    );
    let (column, _) = store
        .load_column(records[0].pos, &reg)
        .expect("load")
        .expect("saved");
    assert!(column.height(0, 0) > 0, "heightmap recomputed");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ids_are_remapped_when_the_mod_set_changes() {
    let dir = temp("remap");
    let old = corpus_registry(false, None);
    let records = corpus_records();
    WorldStore::open(&dir, &old, BOUNDS)
        .expect("open")
        .save_all(&records)
        .expect("save");
    // A new mod sorted first shifts every ID by one; t:b5 was removed.
    let new = corpus_registry(true, Some("t:b5"));
    let mut store = WorldStore::open(&dir, &new, BOUNDS).expect("reopen with new mods");
    let back = store.load(records[0].pos).expect("load").expect("saved");
    for (old_s, new_s) in records[0].sections.iter().zip(&back.sections) {
        for i in 0..4096 {
            let name = old.name(old_s.get(i)).expect("old id");
            let expected = new.state_id(name).unwrap_or(new.missing());
            assert_eq!(new_s.get(i), expected);
        }
    }
    // Saving under the new set and reading it back needs no remap.
    store.save(&back).expect("save new");
    let again = store.load(back.pos).expect("load").expect("saved");
    assert!(same_blocks(&again.sections, &back.sections));
    let _ = std::fs::remove_dir_all(dir);
}

/// The committed region file (written by format v1 from the T0-12 corpus)
/// still decodes to the corpus. Regenerate after an intended format change
/// (and a FORMAT_VERSION bump) with `UPDATE_GOLDEN=1`.
#[test]
fn golden_region_file_decodes() {
    let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/r.0.0.bin");
    let records = corpus_records();
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        let dir = temp("golden");
        let mut store =
            WorldStore::open(&dir, &corpus_registry(false, None), BOUNDS).expect("open");
        store.save_all(&records).expect("save");
        drop(store);
        std::fs::copy(dir.join("region/r.0.0.bin"), &golden).expect("copy golden");
        let _ = std::fs::remove_dir_all(dir);
    }
    let copy = temp("golden-read").with_extension("bin");
    std::fs::copy(&golden, &copy).expect("golden exists; run with UPDATE_GOLDEN=1");
    let mut region = RegionFile::open(&copy).expect("golden opens");
    for r in &records {
        let index = (r.pos.z * 32 + r.pos.x) as usize;
        let bytes = region.read(index).expect("read").expect("present");
        let (back, table) = decode_column(&bytes).expect("decodes");
        assert_eq!(table, 0);
        assert!(same_blocks(&back.sections, &r.sections));
    }
    let _ = std::fs::remove_file(copy);
}

#[test]
fn other_format_versions_are_rejected() {
    let mut bytes = world::storage::encode_column(&corpus_records()[0], 0);
    bytes[..4].copy_from_slice(&2u32.to_le_bytes());
    assert!(matches!(
        decode_column(&bytes),
        Err(StorageError::Version(2))
    ));
    let wrong = WorldBounds {
        min_section_y: -4,
        section_count: 24,
    };
    let dir = temp("bounds");
    let mut store = WorldStore::open(&dir, &corpus_registry(false, None), wrong).expect("open");
    assert!(matches!(
        store.save(&corpus_records()[0]),
        Err(StorageError::Bounds)
    ));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn saving_runs_as_a_background_job() {
    let dir = temp("job");
    let reg = Arc::new(corpus_registry(false, None));
    let store = WorldStore::open(&dir, &reg, BOUNDS).expect("open");
    let records = corpus_records();
    let pool = JobPool::new(2).expect("pool");
    let snapshot = records.clone();
    pool.submit(Priority::Background, CancelToken::new(), move |_| {
        let mut store = store;
        let result = store.save_all(&snapshot).map_err(|e| e.to_string());
        (store, result)
    });
    let (mut store, result) = loop {
        if let Some(done) = pool.drain_completed().next() {
            break done
                .downcast::<(WorldStore, Result<(), String>)>()
                .expect("job result type");
        }
        std::thread::yield_now();
    };
    result.expect("saved in the background");
    assert!(store.load(records[3].pos).expect("load").is_some());
    let _ = std::fs::remove_dir_all(dir);
}

fn make_section(distinct: usize, seed: u64) -> Section {
    let mut s = seed | 1;
    let mut arr = [BlockStateId::AIR; 4096];
    for v in arr.iter_mut() {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        *v = BlockStateId((s % distinct as u64) as u16);
    }
    Section::from_blocks(&arr)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn random_columns_round_trip(
        params in prop::collection::vec((1usize..400, any::<u64>()), 8),
        x in -100i32..100,
        z in -100i32..100,
        light in prop::collection::vec(any::<u8>(), 0..64),
    ) {
        let dir = temp(&format!("prop-{x}-{z}"));
        let reg = corpus_registry(false, None);
        let sections: Vec<Section> = params.iter().map(|&(d, seed)| make_section(d, seed)).collect();
        let rec = ColumnRecord { pos: ColumnPos { x, z }, bounds: BOUNDS, sections, light, entities: vec![1] };
        let mut store = WorldStore::open(&dir, &reg, BOUNDS).expect("open");
        store.save(&rec).expect("save");
        store.save(&rec).expect("save twice (copy-on-write)");
        let back = store.load(rec.pos).expect("load").expect("present");
        prop_assert!(same_blocks(&back.sections, &rec.sections));
        prop_assert_eq!(&back.light, &rec.light);
        prop_assert_eq!(&back.entities, &rec.entities);
        let _ = std::fs::remove_dir_all(dir);
    }
}
