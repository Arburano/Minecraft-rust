//! The local converted-asset cache: `<root>/<converter version>/<key>.vxta`,
//! key = SHA-256 of the source identity and bytes. A converter version change
//! moves to a new directory, so stale conversions are never read.
//!
//! File format (little endian, version 1):
//!
//! ```text
//! magic "VXTA" | u32 format version | u32 converter version | u32 size
//! u32 layer count | u32 mip count
//! per layer: u32 name length, UTF-8 name
//! per mip:   u64 byte length, RGBA8 bytes of all layers
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::AssetError;
use crate::texture_array::TextureArray;

/// Version of the conversion code. Bump on any change of conversion output.
pub const CONVERTER_VERSION: u32 = 1;
/// Version of the `.vxta` file format.
pub const FORMAT_VERSION: u32 = 1;
const MAGIC: &[u8; 4] = b"VXTA";

/// Encodes a texture array to the cache format.
#[must_use]
pub fn encode(array: &TextureArray) -> Vec<u8> {
    let mut out = Vec::with_capacity(array.byte_len() + 64);
    out.extend_from_slice(MAGIC);
    for v in [
        FORMAT_VERSION,
        CONVERTER_VERSION,
        array.size(),
        array.layers() as u32,
        array.levels().len() as u32,
    ] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for name in array.names() {
        out.extend_from_slice(&(name.len() as u32).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
    }
    for level in array.levels() {
        out.extend_from_slice(&(level.len() as u64).to_le_bytes());
        out.extend_from_slice(level);
    }
    out
}

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], AssetError> {
        if self.0.len() < n {
            return Err(AssetError::Corrupt("truncated".into()));
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(head)
    }
    fn u32(&mut self) -> Result<u32, AssetError> {
        let mut b = [0u8; 4];
        b.copy_from_slice(self.take(4)?);
        Ok(u32::from_le_bytes(b))
    }
    fn u64(&mut self) -> Result<u64, AssetError> {
        let mut b = [0u8; 8];
        b.copy_from_slice(self.take(8)?);
        Ok(u64::from_le_bytes(b))
    }
}

/// Decodes the cache format; rejects other format or converter versions.
pub fn decode(bytes: &[u8]) -> Result<TextureArray, AssetError> {
    let mut r = Reader(bytes);
    if r.take(4)? != MAGIC {
        return Err(AssetError::Corrupt("magic".into()));
    }
    let (format, converter) = (r.u32()?, r.u32()?);
    if format != FORMAT_VERSION || converter != CONVERTER_VERSION {
        return Err(AssetError::Stale);
    }
    let (size, layers, mips) = (r.u32()?, r.u32()? as usize, r.u32()? as usize);
    if layers > crate::texture_array::MAX_LAYERS || mips > 32 {
        return Err(AssetError::Corrupt("counts".into()));
    }
    let mut names = Vec::with_capacity(layers);
    for _ in 0..layers {
        let len = r.u32()? as usize;
        let name =
            std::str::from_utf8(r.take(len)?).map_err(|_| AssetError::Corrupt("name".into()))?;
        names.push(name.to_owned());
    }
    let mut levels = Vec::with_capacity(mips);
    for _ in 0..mips {
        let len = usize::try_from(r.u64()?).map_err(|_| AssetError::Corrupt("len".into()))?;
        levels.push(r.take(len)?.to_vec());
    }
    if !r.0.is_empty() {
        return Err(AssetError::Corrupt("trailing bytes".into()));
    }
    TextureArray::from_parts(size, names, levels)
}

/// Cache key: SHA-256 of the source identity (e.g. a version id or a mod id)
/// and the source bytes, as lowercase hex.
#[must_use]
pub fn cache_key(source_id: &str, source: &[u8]) -> String {
    let digest = Sha256::new()
        .chain_update((source_id.len() as u64).to_le_bytes())
        .chain_update(source_id.as_bytes())
        .chain_update(source)
        .finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// The on-disk cache of converted texture arrays.
#[derive(Clone, Debug)]
pub struct AssetCache {
    root: PathBuf,
}

impl AssetCache {
    /// A cache under `root` (e.g. `<data>/cache/assets`).
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Path of an entry for the current converter version.
    #[must_use]
    pub fn path(&self, key: &str) -> PathBuf {
        self.root
            .join(CONVERTER_VERSION.to_string())
            .join(format!("{key}.vxta"))
    }

    /// Reads an entry; `Ok(None)` if absent or written by another version.
    pub fn load(&self, key: &str) -> Result<Option<TextureArray>, AssetError> {
        match fs::read(self.path(key)) {
            Ok(bytes) => match decode(&bytes) {
                Ok(a) => Ok(Some(a)),
                Err(AssetError::Stale) => Ok(None),
                Err(e) => Err(e),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(AssetError::Io(e.to_string())),
        }
    }

    /// Writes an entry atomically (temporary file + rename).
    pub fn store(&self, key: &str, array: &TextureArray) -> Result<(), AssetError> {
        let path = self.path(key);
        let dir = path.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(dir).map_err(|e| AssetError::Io(e.to_string()))?;
        let tmp = path.with_extension("vxta.tmp");
        fs::write(&tmp, encode(array)).map_err(|e| AssetError::Io(e.to_string()))?;
        fs::rename(&tmp, &path).map_err(|e| AssetError::Io(e.to_string()))
    }

    /// The cached conversion of `source`, converting and storing on a miss.
    /// Returns the array and whether it came from the cache.
    pub fn get_or_convert(
        &self,
        source_id: &str,
        source: &[u8],
        convert: impl FnOnce() -> Result<TextureArray, AssetError>,
    ) -> Result<(TextureArray, bool), AssetError> {
        let key = cache_key(source_id, source);
        if let Some(hit) = self.load(&key)? {
            return Ok((hit, true));
        }
        let array = convert()?;
        self.store(&key, &array)?;
        Ok((array, false))
    }
}
