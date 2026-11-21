//! Stable identifiers that survive transfers between regions and worlds.

use bevy_ecs::prelude::Component;

/// Stable entity identity (ADR-0008, engine-core spec). Survives transfer
/// between `World`s, region merges and splits; never reused by a node.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NetId(pub u64);

/// Stable region identity. Assigned by the node, never reused: a retired
/// region's id is not handed to a new region (message sort keys stay honest).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RegionId(pub u64);
