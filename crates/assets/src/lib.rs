//! Asset pipeline: fallback assets, conversion to engine formats, content-addressed
//! cache; Mojang asset import from a local licensed installation (T1-14) and
//! download after the ownership check (after T0-10).
//!
//! Spec:
//! - `docs/architecture/auth-and-assets.md` ("Assets")
//! - `docs/architecture/meshing-and-rendering.md` ("Textures")
//! - `docs/legal.md` rules 2 and 3: the game and all CI tests run on our own
//!   fallback assets.
//!
//! Status (T1-07): procedural fallback textures, mip chains, texture arrays
//! with lazy loading, and the on-disk cache.

use std::fmt;

pub mod cache;
pub mod fallback;
pub mod image;
pub mod texture_array;

pub use cache::{AssetCache, CONVERTER_VERSION};
pub use fallback::{FALLBACK_KEYS, fallback_key, fallback_texture};
pub use image::Rgba8Image;
pub use texture_array::{TextureArray, TextureLocation, TextureSet};

/// Asset errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetError {
    /// A texture is not square with a power-of-two edge (width, height).
    BadSize(u32, u32),
    /// A texture array already holds 256 layers.
    ArrayFull,
    /// A cache file is damaged.
    Corrupt(String),
    /// A cache file was written by another format or converter version.
    Stale,
    /// File system error.
    Io(String),
}

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadSize(w, h) => write!(f, "texture {w}x{h} is not a square power of two"),
            Self::ArrayFull => write!(f, "texture array is full (256 layers)"),
            Self::Corrupt(what) => write!(f, "corrupt asset cache entry: {what}"),
            Self::Stale => write!(f, "asset cache entry from another converter version"),
            Self::Io(e) => write!(f, "asset cache I/O: {e}"),
        }
    }
}

impl std::error::Error for AssetError {}

/// The fallback texture set for the given keys, in order (lazy: callers pass
/// only what they need now and `require` more later).
pub fn fallback_set<'a>(keys: impl IntoIterator<Item = &'a str>) -> Result<TextureSet, AssetError> {
    let mut set = TextureSet::new();
    for key in keys {
        set.require(key, || fallback_texture(key))?;
    }
    Ok(set)
}
