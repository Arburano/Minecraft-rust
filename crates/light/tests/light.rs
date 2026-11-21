//! Incremental lighting must equal a full recompute and the naive reference,
//! bit for bit (`docs/architecture/lighting.md`, "Tests").

use light::reference::naive_light;
use light::{COLUMNS, EDGE, LightError, LightNeighborhood, LightProps, Nibbles, VOLUME};
use proptest::prelude::*;
use world::coords::{BlockPos, SectionPos};
use world::{BlockStateId, Section};

const ORIGIN: BlockPos = BlockPos::new(-16, 0, -16);
const AIR: LightProps = LightProps::AIR;
const STONE: LightProps = LightProps::OPAQUE;
const TORCH: LightProps = LightProps::new(14, 0);
const GLOWSTONE: LightProps = LightProps::new(15, 15);
const WATER: LightProps = LightProps::new(0, 1);
const LEAVES: LightProps = LightProps::new(0, 3);
const LANTERN: LightProps = LightProps::new(10, 2);
const KINDS: [LightProps; 7] = [AIR, STONE, TORCH, GLOWSTONE, WATER, LEAVES, LANTERN];

struct World {
    cells: Vec<LightProps>,
    open_top: Vec<bool>,
}

impl World {
    fn new(fill: LightProps, open: bool) -> Self {
        Self {
            cells: vec![fill; VOLUME],
            open_top: vec![open; COLUMNS],
        }
    }

    fn set(&mut self, p: BlockPos, props: LightProps) {
        let (x, y, z) = local(p);
        self.cells[(y * EDGE + z) * EDGE + x] = props;
    }

    fn lit(&self) -> LightNeighborhood {
        let mut n = LightNeighborhood::from_props(ORIGIN, &self.cells, &self.open_top)
            .expect("valid snapshot");
        n.full_relight();
        n
    }
}

fn local(p: BlockPos) -> (usize, usize, usize) {
    (
        (p.x - ORIGIN.x) as usize,
        (p.y - ORIGIN.y) as usize,
        (p.z - ORIGIN.z) as usize,
    )
}

fn at(x: usize, y: usize, z: usize) -> BlockPos {
    BlockPos::new(
        ORIGIN.x + x as i32,
        ORIGIN.y + y as i32,
        ORIGIN.z + z as i32,
    )
}

/// Applies an edit incrementally and checks it against a full recompute.
fn edit_and_check(world: &mut World, lit: &mut LightNeighborhood, p: BlockPos, props: LightProps) {
    world.set(p, props);
    lit.set_block(p, props).expect("inside the snapshot");
    let full = world.lit();
    assert_eq!(
        lit.block_levels(),
        full.block_levels(),
        "block light after {p:?}"
    );
    assert_eq!(lit.sky_levels(), full.sky_levels(), "sky light after {p:?}");
}

#[test]
fn torch_levels_and_walls() {
    let mut world = World::new(AIR, false);
    let torch = at(24, 24, 24);
    world.set(torch, TORCH);
    world.set(at(26, 24, 24), STONE);
    let lit = world.lit();
    assert_eq!(lit.block_light(torch), Some(14));
    assert_eq!(lit.block_light(at(25, 24, 24)), Some(13));
    assert_eq!(lit.block_light(at(24, 29, 24)), Some(9));
    assert_eq!(
        lit.block_light(at(26, 24, 24)),
        Some(0),
        "opaque cell stays dark"
    );
    // Behind the wall light goes around it: 5 steps instead of 3.
    assert_eq!(lit.block_light(at(27, 24, 24)), Some(9));
    assert_eq!(lit.block_light(at(24, 24, 39)), Some(0), "15 blocks away");
    assert_eq!(lit.sky_light(torch), Some(0), "closed columns get no sky");
    assert_eq!(lit.block_light(BlockPos::new(100, 0, 0)), None);
}

#[test]
fn torch_on_section_boundary_matches_full_recompute() {
    let mut world = World::new(AIR, false);
    for (x, y, z) in [
        (16, 20, 20),
        (31, 31, 31),
        (32, 16, 16),
        (15, 32, 47),
        (0, 0, 0),
    ] {
        world.set(at(x, y, z), STONE);
    }
    let mut lit = world.lit();
    for p in [
        at(16, 16, 16),
        at(31, 32, 16),
        at(15, 15, 15),
        at(47, 47, 0),
    ] {
        edit_and_check(&mut world, &mut lit, p, TORCH);
    }
    for p in [at(16, 16, 16), at(31, 32, 16)] {
        edit_and_check(&mut world, &mut lit, p, AIR);
    }
}

#[test]
fn torch_removal_restores_other_sources() {
    let mut world = World::new(AIR, false);
    world.set(at(20, 24, 24), TORCH);
    world.set(at(28, 24, 24), TORCH);
    let mut lit = world.lit();
    edit_and_check(&mut world, &mut lit, at(28, 24, 24), AIR);
    assert_eq!(
        lit.block_light(at(28, 24, 24)),
        Some(6),
        "lit by the other torch"
    );
    edit_and_check(&mut world, &mut lit, at(20, 24, 24), AIR);
    assert!(lit.block_levels().iter().all(|&l| l == 0));
}

#[test]
fn broken_block_under_the_sky() {
    // Ground at y < 24, a stone roof over a cave, open sky above.
    let mut world = World::new(AIR, true);
    for z in 0..EDGE {
        for x in 0..EDGE {
            for y in 0..24 {
                world.set(at(x, y, z), STONE);
            }
        }
    }
    for z in 10..38 {
        for x in 10..38 {
            for y in 16..23 {
                world.set(at(x, y, z), AIR);
            }
        }
    }
    let mut lit = world.lit();
    assert_eq!(lit.sky_light(at(24, 30, 24)), Some(15));
    assert_eq!(lit.sky_light(at(24, 20, 24)), Some(0), "sealed cave");
    // Break the roof: direct sky falls into the cave down to its floor.
    edit_and_check(&mut world, &mut lit, at(24, 23, 24), AIR);
    assert_eq!(lit.sky_light(at(24, 16, 24)), Some(15));
    assert_eq!(lit.sky_light(at(25, 16, 24)), Some(14));
    // Put it back, then cover the hole with leaves (sky attenuated, not blocked).
    edit_and_check(&mut world, &mut lit, at(24, 23, 24), STONE);
    assert_eq!(lit.sky_light(at(24, 16, 24)), Some(0));
    edit_and_check(&mut world, &mut lit, at(24, 23, 24), AIR);
    edit_and_check(&mut world, &mut lit, at(24, 40, 24), LEAVES);
    assert_eq!(lit.sky_light(at(24, 40, 24)), Some(11));
    assert_eq!(
        lit.sky_light(at(24, 39, 24)),
        Some(14),
        "side light beats the leaves"
    );
    edit_and_check(&mut world, &mut lit, at(24, 40, 24), AIR);
    edit_and_check(&mut world, &mut lit, at(24, 47, 24), STONE);
    edit_and_check(&mut world, &mut lit, at(24, 47, 24), AIR);
}

#[test]
fn relight_add_rejects_darkening_changes() {
    let mut world = World::new(AIR, true);
    world.set(at(5, 5, 5), TORCH);
    let mut lit = world.lit();
    assert_eq!(
        lit.relight_add(at(5, 5, 5), AIR),
        Err(LightError::WouldDecrease)
    );
    assert_eq!(
        lit.relight_add(at(6, 5, 5), STONE),
        Err(LightError::WouldDecrease)
    );
    assert_eq!(lit.block_light(at(5, 5, 5)), Some(14), "unchanged on error");
    lit.relight_add(at(9, 9, 9), LightProps::new(GLOWSTONE.emission, 0))
        .expect("emission up, opacity same");
    assert!(matches!(
        lit.relight_remove(at(48, 0, 0), AIR),
        Err(LightError::OutOfBounds(_))
    ));
}

#[test]
fn section_export_is_compact() {
    let mut world = World::new(AIR, true);
    world.set(at(24, 24, 24), TORCH);
    let lit = world.lit();
    let top = lit.section_light(1, 2, 1);
    assert_eq!(top.sky, Nibbles::Single(15));
    let center = lit.section_light(1, 1, 1);
    assert_eq!(center.sky, Nibbles::Single(15));
    assert_eq!(center.block.get((8 * 16 + 8) * 16 + 8), 14);
    assert_eq!(center.block.storage_bytes(), 2048);
}

#[test]
fn section_constructor_matches_props_constructor() {
    // Center section stone floor at local y 0..4 (world 16..20), torch above.
    let mut sections: Vec<Section> = (0..27).map(|_| Section::new()).collect();
    let mut world = World::new(AIR, true);
    for i in 0..1024 {
        sections[13].set(i, BlockStateId(1));
        let (x, z, y) = (i % 16, i / 16 % 16, i / 256);
        world.set(at(16 + x, 16 + y, 16 + z), STONE);
    }
    sections[13].set((8 * 16 + 8) * 16 + 8, BlockStateId(2));
    world.set(at(24, 24, 24), TORCH);
    let refs: [&Section; 27] = std::array::from_fn(|i| &sections[i]);
    let heightmap = vec![i32::MIN; COLUMNS];
    let props = |id: BlockStateId| match id.0 {
        0 => AIR,
        2 => TORCH,
        _ => STONE,
    };
    let mut from_sections =
        LightNeighborhood::new(SectionPos { x: 0, y: 1, z: 0 }, refs, &heightmap, props)
            .expect("27 sections");
    from_sections.full_relight();
    assert_eq!(from_sections.origin(), ORIGIN);
    let expected = world.lit();
    assert_eq!(from_sections.block_levels(), expected.block_levels());
    assert_eq!(from_sections.sky_levels(), expected.sky_levels());
    let (block, sky) = naive_light(&world.cells, &world.open_top);
    assert_eq!(from_sections.block_levels(), block);
    assert_eq!(from_sections.sky_levels(), sky);
    assert!(matches!(
        LightNeighborhood::new(SectionPos::default(), refs, &[0; 3], props),
        Err(LightError::BadLength { .. })
    ));
}

/// Deterministic xorshift for world generation inside proptest cases.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Terrain-like random snapshot: blobs of stone/leaves/water, a few lights,
/// and a random set of columns open to the sky.
fn random_world(seed: u64, density: usize) -> World {
    let mut rng = Rng(seed | 1);
    let mut world = World::new(AIR, false);
    for flag in &mut world.open_top {
        *flag = rng.below(4) != 0;
    }
    let blobs = VOLUME * density / 100 / 27;
    for _ in 0..blobs {
        let kind = [STONE, STONE, STONE, LEAVES, WATER][rng.below(5)];
        let (x, y, z) = (
            rng.below(EDGE - 2),
            rng.below(EDGE - 2),
            rng.below(EDGE - 2),
        );
        for dy in 0..3 {
            for dz in 0..3 {
                for dx in 0..3 {
                    world.set(at(x + dx, y + dy, z + dz), kind);
                }
            }
        }
    }
    for _ in 0..rng.below(24) {
        let kind = [TORCH, GLOWSTONE, LANTERN][rng.below(3)];
        world.set(at(rng.below(EDGE), rng.below(EDGE), rng.below(EDGE)), kind);
    }
    world
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn incremental_equals_full_and_naive(
        seed in any::<u64>(),
        density in 0usize..60,
        edits in prop::collection::vec(
            (0usize..EDGE, 0usize..EDGE, 0usize..EDGE, 0usize..KINDS.len(), any::<bool>()),
            1..24,
        ),
    ) {
        let mut world = random_world(seed, density);
        let mut lit = world.lit();
        let (block, sky) = naive_light(&world.cells, &world.open_top);
        prop_assert_eq!(lit.block_levels(), block);
        prop_assert_eq!(lit.sky_levels(), sky);
        for (x, y, z, kind, near_center) in edits {
            // Half of the edits land in the center section, where light is exact.
            let (x, y, z) = if near_center { (16 + x / 3, 16 + y / 3, 16 + z / 3) } else { (x, y, z) };
            let p = at(x, y, z);
            let props = KINDS[kind];
            world.set(p, props);
            lit.set_block(p, props).expect("inside the snapshot");
            let full = world.lit();
            prop_assert_eq!(lit.block_levels(), full.block_levels(), "block light after {:?}", p);
            prop_assert_eq!(lit.sky_levels(), full.sky_levels(), "sky light after {:?}", p);
        }
        let (block, sky) = naive_light(&world.cells, &world.open_top);
        prop_assert_eq!(lit.block_levels(), block);
        prop_assert_eq!(lit.sky_levels(), sky);
    }
}
