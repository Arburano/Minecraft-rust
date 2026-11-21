//! Shared scenario helpers for the integration tests.

// Each integration test compiles this module separately and uses a subset.
#![allow(dead_code)]

use sim::fixture::spawn_bots;
use sim::{NetId, RegionNode, SectionRect};
use world::coords::BlockPos;

/// The property-test arena: 16 x 8 sections.
pub fn arena() -> SectionRect {
    SectionRect::new(0, 0, 15, 7).expect("valid")
}

/// Builds a node with `regions` equal vertical slices of `rect` and `bots`
/// fixture bodies. The slice width must be at least `MIN_SIDE`.
pub fn build(
    rect: SectionRect,
    regions: usize,
    bots: u64,
    threads: usize,
) -> (RegionNode, Vec<NetId>) {
    let mut node = RegionNode::new(rect, threads).expect("pool");
    let width = rect.width() / regions as i32;
    assert!(width >= sim::MIN_SIDE);
    assert_eq!(rect.width() % regions as i32, 0);
    for i in 0..regions as i32 {
        node.add_region(
            SectionRect::new(
                rect.min_x + i * width,
                rect.min_z,
                rect.min_x + (i + 1) * width - 1,
                rect.max_z,
            )
            .expect("valid"),
        )
        .expect("tiling");
    }
    let ids = spawn_bots(&mut node, bots).expect("spawn");
    (node, ids)
}

/// Resident plus in-flight ids must equal the initial set exactly: no
/// entity lost, none duplicated.
pub fn check_conservation(node: &RegionNode, initial: &[NetId]) {
    let mut present: Vec<NetId> = node
        .regions()
        .iter()
        .flat_map(|r| r.net_ids())
        .chain(node.in_flight_ids())
        .collect();
    present.sort_unstable();
    let mut expected = initial.to_vec();
    expected.sort_unstable();
    assert_eq!(present, expected, "entity set changed");
    assert_eq!(
        node.entity_count() + node.in_flight(),
        initial.len(),
        "conserved total"
    );
}

/// Every resident entity is inside its region's territory; territories are
/// disjoint, inside the arena, and together cover it.
pub fn check_partitioning(node: &RegionNode) {
    let arena = node.arena();
    let regions = node.regions();
    let mut covered = 0usize;
    for (i, r) in regions.iter().enumerate() {
        let t = r.territory();
        assert!(arena.contains(t.min_x, t.min_z) && arena.contains(t.max_x, t.max_z));
        covered += (t.width() * t.depth()) as usize;
        for other in &regions[i + 1..] {
            let o = other.territory();
            assert!(
                t.max_x < o.min_x || o.max_x < t.min_x || t.max_z < o.min_z || o.max_z < t.min_z,
                "territories overlap"
            );
        }
        for net_id in r.net_ids() {
            let body = r.body(net_id).expect("resident");
            let section =
                BlockPos::new(body.position.x as i32, 0, body.position.z as i32).section();
            assert!(t.contains_pos(section), "entity outside its territory");
            assert!(body.position.y >= sim::GROUND_Y, "entity below ground");
        }
    }
    assert_eq!(
        covered,
        (arena.width() * arena.depth()) as usize,
        "partitioning must tile the arena"
    );
}
