//! World generation pipeline (terrain -> biomes -> caves -> structures -> decorations), heightmap pre-pass, LOD source data.
//!
//! Spec:
//! - `docs/architecture/worldgen.md`
//!
//! Status: the deterministic test corpus is implemented (task T0-12,
//! [`test_corpus`]). The mod-driven generation pipeline is a later stage.

pub mod test_corpus;

pub use test_corpus::{CorpusArea, TestColumn, test_column, test_corpus};
