//! Block and sky light: incremental add/remove BFS on section snapshots.
//!
//! Spec: `docs/architecture/lighting.md`, ADR-0011 (light is computed by the
//! client for rendering and by the server lazily for gameplay, with the same
//! code). Task T0-08.
//!
//! - [`Nibbles`] / [`SectionLight`]: per-section 4-bit storage with a single-value
//!   variant for uniform sections.
//! - [`LightNeighborhood`]: a 3x3x3 section snapshot with [`full_relight`],
//!   [`relight_add`] (BFS) and [`relight_remove`] (reverse BFS + refill); sky
//!   light starts from the heightmap's exposed run and spreads by the same BFS.
//! - [`reference::naive_light`]: an independent fixed-point reference.
//!
//! The engine does not know blocks: emission and opacity come from the caller
//! as [`LightProps`].
//!
//! [`full_relight`]: LightNeighborhood::full_relight
//! [`relight_add`]: LightNeighborhood::relight_add
//! [`relight_remove`]: LightNeighborhood::relight_remove

mod neighborhood;
mod nibbles;
pub mod reference;

pub use neighborhood::{COLUMNS, EDGE, LightError, LightNeighborhood, LightProps, VOLUME};
pub use nibbles::{MAX_LIGHT, Nibbles, PACKED_BYTES, SectionLight};
