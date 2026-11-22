//! Client library: the parts of the game client that are testable without a
//! window (the binary in `main.rs` is the window, menu and render loop).
//!
//! Spec: `docs/architecture/client-menu-and-updater.md`.

pub mod edit;
pub mod loading;
pub mod session;
pub mod settings;
pub mod textures;
pub mod worlds;
