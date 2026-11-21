//! GPU-ready texture arrays (`meshing-and-rendering.md`, "Textures"): one array
//! per texture size, mipmaps generated at conversion time, at most 256 layers
//! per array (the default wgpu limit).

use std::collections::{BTreeMap, HashMap};

use crate::AssetError;
use crate::image::Rgba8Image;

/// Layers per array (default wgpu `max_texture_array_layers`).
pub const MAX_LAYERS: usize = 256;

/// One `texture_2d_array`: square layers of one size with a full mip chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureArray {
    size: u32,
    names: Vec<String>,
    /// `levels[mip]`: all layers of that mip level, RGBA8, layer after layer.
    levels: Vec<Vec<u8>>,
}

impl TextureArray {
    /// An empty array of square `size`×`size` layers (power of two).
    pub fn new(size: u32) -> Result<Self, AssetError> {
        if !size.is_power_of_two() {
            return Err(AssetError::BadSize(size, size));
        }
        let mips = size.trailing_zeros() as usize + 1;
        Ok(Self {
            size,
            names: Vec::new(),
            levels: vec![Vec::new(); mips],
        })
    }

    /// Rebuilds an array from its parts (used by the cache decoder).
    pub(crate) fn from_parts(
        size: u32,
        names: Vec<String>,
        levels: Vec<Vec<u8>>,
    ) -> Result<Self, AssetError> {
        let mut expect = Self::new(size)?;
        if levels.len() != expect.levels.len() || names.len() > MAX_LAYERS {
            return Err(AssetError::Corrupt("mip or layer count".into()));
        }
        for (mip, level) in levels.iter().enumerate() {
            let side = (size >> mip).max(1) as usize;
            if level.len() != side * side * 4 * names.len() {
                return Err(AssetError::Corrupt(format!("mip {mip} length")));
            }
        }
        expect.names = names;
        expect.levels = levels;
        Ok(expect)
    }

    /// Edge length of level 0.
    #[must_use]
    pub fn size(&self) -> u32 {
        self.size
    }

    /// Layer names in layer order.
    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Number of layers.
    #[must_use]
    pub fn layers(&self) -> usize {
        self.names.len()
    }

    /// Mip levels: `levels()[mip]` holds every layer of that level.
    #[must_use]
    pub fn levels(&self) -> &[Vec<u8>] {
        &self.levels
    }

    /// Whether another layer fits.
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.names.len() >= MAX_LAYERS
    }

    /// Appends a layer with its full mip chain; returns the layer index.
    pub fn push(&mut self, name: &str, image: &Rgba8Image) -> Result<u16, AssetError> {
        if image.width() != self.size || image.height() != self.size {
            return Err(AssetError::BadSize(image.width(), image.height()));
        }
        if self.is_full() {
            return Err(AssetError::ArrayFull);
        }
        for (level, mip) in self.levels.iter_mut().zip(image.mip_chain()) {
            level.extend(mip.to_bytes());
        }
        self.names.push(name.to_owned());
        Ok((self.names.len() - 1) as u16)
    }

    /// Bytes of all levels (for memory accounting).
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.levels.iter().map(Vec::len).sum()
    }
}

/// Where a texture lives: array index and layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureLocation {
    /// Index into [`TextureSet::arrays`].
    pub array: u16,
    /// Layer inside that array.
    pub layer: u16,
}

/// Texture arrays plus a name index. Supports lazy loading: textures are
/// added when first required, and newly added layers are reported for upload.
#[derive(Clone, Debug, Default)]
pub struct TextureSet {
    arrays: Vec<TextureArray>,
    index: HashMap<String, TextureLocation>,
    /// Current (not yet full) array per size.
    open: BTreeMap<u32, u16>,
    added: Vec<TextureLocation>,
}

impl TextureSet {
    /// An empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a set from images, in the given order.
    pub fn build<'a>(
        images: impl IntoIterator<Item = (&'a str, &'a Rgba8Image)>,
    ) -> Result<Self, AssetError> {
        let mut set = Self::new();
        for (name, image) in images {
            set.require(name, || image.clone())?;
        }
        set.added.clear();
        Ok(set)
    }

    /// The location of `name`, loading it with `load` on first use (lazy
    /// loading: only what the menu and nearby blocks ask for is converted).
    pub fn require(
        &mut self,
        name: &str,
        load: impl FnOnce() -> Rgba8Image,
    ) -> Result<TextureLocation, AssetError> {
        if let Some(&loc) = self.index.get(name) {
            return Ok(loc);
        }
        let image = load();
        if image.width() != image.height() || !image.width().is_power_of_two() {
            return Err(AssetError::BadSize(image.width(), image.height()));
        }
        let size = image.width();
        let array = match self.open.get(&size) {
            Some(&a) if !self.arrays[usize::from(a)].is_full() => a,
            _ => {
                let a = u16::try_from(self.arrays.len()).map_err(|_| AssetError::ArrayFull)?;
                self.arrays.push(TextureArray::new(size)?);
                self.open.insert(size, a);
                a
            }
        };
        let layer = self.arrays[usize::from(array)].push(name, &image)?;
        let loc = TextureLocation { array, layer };
        self.index.insert(name.to_owned(), loc);
        self.added.push(loc);
        Ok(loc)
    }

    /// Location of an already loaded texture.
    #[must_use]
    pub fn location(&self, name: &str) -> Option<TextureLocation> {
        self.index.get(name).copied()
    }

    /// All arrays.
    #[must_use]
    pub fn arrays(&self) -> &[TextureArray] {
        &self.arrays
    }

    /// Layers added since the last call (to upload to the GPU).
    pub fn take_added(&mut self, out: &mut Vec<TextureLocation>) {
        out.append(&mut self.added);
    }
}
