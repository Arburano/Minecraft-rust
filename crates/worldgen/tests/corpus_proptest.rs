//! Property tests for the test corpus: for random seeds and areas, a column
//! generated alone equals the same column of a full corpus generation, and a
//! shuffled regeneration gives the same hashes.

use std::collections::HashMap;

use proptest::prelude::*;
use worldgen::test_corpus::{CorpusArea, test_column, test_corpus};

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn corpus_is_order_and_padding_independent(
        seed in any::<u64>(),
        x0 in -4i32..4,
        z0 in -4i32..4,
        y0 in -2i32..2,
        h in 1i32..5,
    ) {
        let area = CorpusArea {
            x: x0..x0 + 2,
            y: y0..y0 + h,
            z: z0..z0 + 2,
        };
        let columns = test_corpus(seed, &area);
        let reference: HashMap<(i32, i32), u64> =
            columns.iter().map(|c| ((c.x, c.z), c.hash())).collect();

        // Arbitrary (HashMap) order, one column at a time.
        let mut scattered = HashMap::new();
        for &(x, z) in reference.keys() {
            let column = test_column(seed, x, z, area.y.clone());
            scattered.insert((x, z), column.hash());
        }
        prop_assert_eq!(scattered, reference);
    }
}
