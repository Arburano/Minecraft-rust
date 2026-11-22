//! Terrain renderer (T1-11): render batches uploaded into one quad buffer,
//! paths B and C, texture arrays, light from a 3D brick atlas, CPU culling.

pub mod arena;
pub mod camera;
pub mod culling;
pub mod hooks;
pub mod light_atlas;
pub mod post;
pub mod profiles;
pub mod renderer;
