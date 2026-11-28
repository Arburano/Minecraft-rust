//! Network game logic shared by the client and the server (T2-11).
//!
//! Both ends of a connection must compute exactly the same things: how a
//! wire intent becomes physics input, the state hash that detects a
//! diverged prediction, what a correction carries, and how entity deltas are
//! built and applied. Keeping that here lets the client be built without the
//! server crate (no `singleplayer` feature) while both still agree bit for
//! bit.
//!
//! Spec: `docs/architecture/networking-protocol.md`,
//! `docs/architecture/simulation-and-ai.md` (reconciliation).

pub mod input;
pub mod replication;
