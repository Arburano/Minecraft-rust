//! World generation pipeline (terrain -> biomes -> caves -> structures -> decorations), heightmap pre-pass, LOD source data.
//!
//! Spec:
//! - `docs/architecture/worldgen.md`
//!
//! Status: the deterministic test corpus (task T0-12, [`test_corpus`]) and the
//! overworld pipeline (task T1-03, [`Pipeline`]) configured by mod data
//! (`mods/overworld/data/worldgen/overworld.toml`). Mod-registered stages through the
//! WIT API come in stage 3.

pub mod config;
pub mod pipeline;
pub mod stages;
pub mod test_corpus;

pub use config::{ConfigError, WorldgenConfig};
pub use pipeline::{GenContext, Pipeline, ProtoColumn, Stage, Surface};

pub use test_corpus::{CorpusArea, TestColumn, test_column, test_corpus};
