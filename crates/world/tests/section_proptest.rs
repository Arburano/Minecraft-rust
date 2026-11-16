//! Property tests for `world::Section` against a naive reference `[u16; 4096]`.
//!
//! Task T0-03 acceptance: 10 000 random edit sequences must keep `get` equal to
//! the reference at every index. Also checks `unpack_into`, the `non_air`
//! counter, and that `compact` preserves content.

use proptest::prelude::*;
use proptest::test_runner::Config;
use world::{BlockStateId, Section};

const VOLUME: usize = 4096;

/// Applies edits to both the section and the reference, then checks every index.
fn check_against_reference(
    section: &Section,
    reference: &[u16; VOLUME],
) -> Result<(), TestCaseError> {
    let mut unpacked = [BlockStateId::AIR; VOLUME];
    section.unpack_into(&mut unpacked);
    let mut non_air = 0u16;
    for i in 0..VOLUME {
        let expected = reference[i];
        prop_assert_eq!(section.get(i).0, expected, "get mismatch at index {}", i);
        prop_assert_eq!(unpacked[i].0, expected, "unpack mismatch at index {}", i);
        if expected != 0 {
            non_air += 1;
        }
    }
    prop_assert_eq!(section.non_air(), non_air, "non_air counter mismatch");
    prop_assert_eq!(section.all_air(), non_air == 0);
    Ok(())
}

proptest! {
    #![proptest_config(Config::with_cases(10_000))]

    /// 10 000 random edit sequences: `get` always matches the reference.
    #[test]
    fn section_matches_reference(
        edits in prop::collection::vec((0..VOLUME, 0..=300u16), 1..256),
    ) {
        let mut section = Section::new();
        let mut reference = [0u16; VOLUME];
        for &(i, v) in &edits {
            section.set(i, BlockStateId(v));
            reference[i] = v;
        }
        check_against_reference(&section, &reference)?;
    }

    /// Editing an already-populated section (paletted and direct) stays correct.
    #[test]
    fn edits_on_populated_section(
        base in prop::collection::vec(0..=300u16, VOLUME),
        edits in prop::collection::vec((0..VOLUME, 0..=300u16), 1..128),
    ) {
        let mut arr = [BlockStateId::AIR; VOLUME];
        let mut reference = [0u16; VOLUME];
        for i in 0..VOLUME {
            arr[i] = BlockStateId(base[i]);
            reference[i] = base[i];
        }
        let mut section = Section::from_blocks(&arr);
        for &(i, v) in &edits {
            section.set(i, BlockStateId(v));
            reference[i] = v;
        }
        check_against_reference(&section, &reference)?;
    }
}

proptest! {
    #![proptest_config(Config::with_cases(2_000))]

    /// `compact` never changes the visible content.
    #[test]
    fn compact_preserves_content(
        edits in prop::collection::vec((0..VOLUME, 0..=300u16), 1..256),
    ) {
        let mut section = Section::new();
        let mut reference = [0u16; VOLUME];
        for &(i, v) in &edits {
            section.set(i, BlockStateId(v));
            reference[i] = v;
        }
        section.compact();
        check_against_reference(&section, &reference)?;
    }

    /// `from_blocks` round-trips arbitrary content.
    #[test]
    fn from_blocks_round_trips(base in prop::collection::vec(0..=600u16, VOLUME)) {
        let mut arr = [BlockStateId::AIR; VOLUME];
        let mut reference = [0u16; VOLUME];
        for i in 0..VOLUME {
            arr[i] = BlockStateId(base[i]);
            reference[i] = base[i];
        }
        let section = Section::from_blocks(&arr);
        check_against_reference(&section, &reference)?;
    }
}
