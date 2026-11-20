//! Shared gameplay simulation: regions, entity ECS, player physics,
//! prediction/reconciliation hooks.
//!
//! Spec:
//! - `docs/architecture/server-regions.md`
//! - `docs/architecture/engine-core.md`
//! - ADR-0008 (regional multithreading)
//!
//! Status: stage-0 prototype (T0-09). A region is a `bevy_ecs::World` with a
//! rectangular territory of sections in a flat world; the node runs the tick
//! phases, ticks regions in parallel, exchanges sorted inter-region messages
//! (N → N+1) and merges/splits regions with the ≥ 1 section buffer. No
//! networking, no mods, no vertical partitioning yet — see the T0-09 report
//! for the prototype relaxations.

pub mod fixture;
pub mod hash;
pub mod ids;
pub mod message;
pub mod node;
pub mod region;
pub mod territory;

pub use engine_core::Tick;
pub use ids::{NetId, RegionId};
pub use message::{Message, Payload};
pub use node::{RegionNode, SimError, SplitOutcome};
pub use region::{Body, GROUND_Y, Region};
pub use territory::{Axis, MIN_SIDE, SectionRect};
