//! On-disk world storage (`world-storage.md`, "On-disk storage"), format v1.
//!
//! - `<world>/region/r.X.Z.bin`: 32×32 columns. Header = magic, format version
//!   and 1,024 entries `{offset u64, length u32, checksum u32}`. Column records
//!   are zstd (level 3) compressed and **appended**; the header entry is
//!   rewritten only after the record is on disk (copy-on-write). A crash before
//!   the entry update leaves the previous version readable; a torn record is
//!   detected by the checksum. `compact` rewrites a region without garbage.
//! - `<world>/world.redb`: metadata (format version) and the ID tables. Every
//!   column record names the ID table it was written with, so columns saved
//!   under different mod sets are remapped on load (`Registry::remap`).
//!
//! A `WorldStore` is owned by one thread (the background writer job); callers
//! hand it column snapshots.

use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

use crate::BlockStateId;
use crate::column::{Column, ColumnPos, WorldBounds};
use crate::registry::{IdTable, Registry};
use crate::section::{Section, SectionDecodeError};

/// Version of the column record and region file format.
pub const FORMAT_VERSION: u32 = 1;
/// Columns per region edge.
pub const REGION_SIZE: i32 = 32;
/// zstd level for saves (world-storage.md; backups use 9).
pub const ZSTD_LEVEL: i32 = 3;

const REGION_MAGIC: &[u8; 4] = b"VXRG";
const ENTRIES: usize = (REGION_SIZE * REGION_SIZE) as usize;
const ENTRY_BYTES: u64 = 16;
const HEADER_BYTES: u64 = 8 + ENTRIES as u64 * ENTRY_BYTES;

const META: TableDefinition<&str, u32> = TableDefinition::new("meta");
const ID_TABLES: TableDefinition<u32, &str> = TableDefinition::new("id_tables");

/// Storage errors.
#[derive(Debug)]
pub enum StorageError {
    /// File system error.
    Io(std::io::Error),
    /// Metadata database error.
    Meta(String),
    /// A file or record has another format version.
    Version(u32),
    /// A record fails its checksum or does not decode.
    Corrupt(String),
    /// A stored column has other world bounds than requested.
    Bounds,
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "world storage I/O: {e}"),
            Self::Meta(e) => write!(f, "world metadata: {e}"),
            Self::Version(v) => write!(f, "unsupported world format version {v}"),
            Self::Corrupt(e) => write!(f, "corrupt world data: {e}"),
            Self::Bounds => write!(f, "stored column has different world bounds"),
        }
    }
}

impl std::error::Error for StorageError {}

impl From<std::io::Error> for StorageError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<SectionDecodeError> for StorageError {
    fn from(e: SectionDecodeError) -> Self {
        Self::Corrupt(e.to_string())
    }
}

fn meta_err(e: impl fmt::Display) -> StorageError {
    StorageError::Meta(e.to_string())
}

/// FNV-1a 32 over a record, stored in the region header.
fn checksum(bytes: &[u8]) -> u32 {
    let mut h = 0x811c_9dc5u32;
    for &b in bytes {
        h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    h
}

/// A column as stored: blocks plus opaque per-column blobs owned by other
/// crates (light from `light`, entities from `sim`), so `world` keeps its
/// dependency direction.
#[derive(Clone)]
pub struct ColumnRecord {
    /// Position.
    pub pos: ColumnPos,
    /// Vertical bounds.
    pub bounds: WorldBounds,
    /// Sections bottom to top, in the IDs of `table`.
    pub sections: Vec<Section>,
    /// Encoded light of the column (empty if not lit yet).
    pub light: Vec<u8>,
    /// Encoded block entities and entities.
    pub entities: Vec<u8>,
}

/// Encodes an uncompressed column record.
#[must_use]
pub fn encode_column(record: &ColumnRecord, table: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(4096);
    for v in [FORMAT_VERSION, table] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for v in [record.pos.x, record.pos.z, record.bounds.min_section_y] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(record.sections.len() as u32).to_le_bytes());
    for s in &record.sections {
        s.encode(&mut out);
    }
    for blob in [&record.light, &record.entities] {
        out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        out.extend_from_slice(blob);
    }
    out
}

/// Decodes an uncompressed column record; returns it and its ID table number.
pub fn decode_column(bytes: &[u8]) -> Result<(ColumnRecord, u32), StorageError> {
    let mut at = 0usize;
    let mut word = |n: usize| -> Result<&[u8], StorageError> {
        let s = bytes
            .get(at..at + n)
            .ok_or_else(|| StorageError::Corrupt("truncated column record".into()))?;
        at += n;
        Ok(s)
    };
    let mut u32_ = || -> Result<u32, StorageError> {
        let b = word(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let version = u32_()?;
    if version != FORMAT_VERSION {
        return Err(StorageError::Version(version));
    }
    let table = u32_()?;
    let (x, z, min_section_y) = (u32_()? as i32, u32_()? as i32, u32_()? as i32);
    let count = u32_()?;
    if count > 4096 {
        return Err(StorageError::Corrupt("section count".into()));
    }
    let mut sections = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let (s, used) = Section::decode(&bytes[at..])?;
        at += used;
        sections.push(s);
    }
    let mut blob = || -> Result<Vec<u8>, StorageError> {
        let len = bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
            .ok_or_else(|| StorageError::Corrupt("truncated blob length".into()))?;
        let data = bytes
            .get(at + 4..at + 4 + len)
            .ok_or_else(|| StorageError::Corrupt("truncated blob".into()))?
            .to_vec();
        at += 4 + len;
        Ok(data)
    };
    let light = blob()?;
    let entities = blob()?;
    if at != bytes.len() {
        return Err(StorageError::Corrupt(
            "trailing bytes in column record".into(),
        ));
    }
    Ok((
        ColumnRecord {
            pos: ColumnPos { x, z },
            bounds: WorldBounds {
                min_section_y,
                section_count: count,
            },
            sections,
            light,
            entities,
        },
        table,
    ))
}

/// Region coordinates of a column.
#[must_use]
pub fn region_of(pos: ColumnPos) -> (i32, i32) {
    (pos.x.div_euclid(REGION_SIZE), pos.z.div_euclid(REGION_SIZE))
}

fn local_index(pos: ColumnPos) -> usize {
    (pos.z.rem_euclid(REGION_SIZE) * REGION_SIZE + pos.x.rem_euclid(REGION_SIZE)) as usize
}

/// One pending record write: local column index and uncompressed record.
type RegionWrite = (usize, Vec<u8>);

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct Entry {
    offset: u64,
    len: u32,
    sum: u32,
}

/// One open region file.
pub struct RegionFile {
    path: PathBuf,
    file: File,
    entries: Box<[Entry; ENTRIES]>,
    end: u64,
}

impl RegionFile {
    /// Opens or creates a region file.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let len = file.metadata()?.len();
        let mut entries = Box::new([Entry::default(); ENTRIES]);
        if len == 0 {
            let mut header = Vec::with_capacity(HEADER_BYTES as usize);
            header.extend_from_slice(REGION_MAGIC);
            header.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
            header.resize(HEADER_BYTES as usize, 0);
            file.write_all(&header)?;
            file.sync_all()?;
        } else {
            let mut header = vec![0u8; HEADER_BYTES as usize];
            file.seek(SeekFrom::Start(0))?;
            file.read_exact(&mut header)
                .map_err(|_| StorageError::Corrupt("region header truncated".into()))?;
            if &header[..4] != REGION_MAGIC {
                return Err(StorageError::Corrupt("region magic".into()));
            }
            let version = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
            if version != FORMAT_VERSION {
                return Err(StorageError::Version(version));
            }
            for (i, e) in entries.iter_mut().enumerate() {
                let b = &header[8 + i * 16..8 + i * 16 + 16];
                let mut o = [0u8; 8];
                o.copy_from_slice(&b[..8]);
                e.offset = u64::from_le_bytes(o);
                e.len = u32::from_le_bytes([b[8], b[9], b[10], b[11]]);
                e.sum = u32::from_le_bytes([b[12], b[13], b[14], b[15]]);
            }
        }
        let end = file.metadata()?.len().max(HEADER_BYTES);
        Ok(Self {
            path: path.to_owned(),
            file,
            entries,
            end,
        })
    }

    /// The decompressed record of a local column, if stored.
    pub fn read(&mut self, index: usize) -> Result<Option<Vec<u8>>, StorageError> {
        let e = self.entries[index];
        if e.len == 0 {
            return Ok(None);
        }
        let mut compressed = vec![0u8; e.len as usize];
        self.file.seek(SeekFrom::Start(e.offset))?;
        self.file
            .read_exact(&mut compressed)
            .map_err(|_| StorageError::Corrupt("record past the end of the region".into()))?;
        if checksum(&compressed) != e.sum {
            return Err(StorageError::Corrupt("record checksum mismatch".into()));
        }
        zstd::stream::decode_all(&compressed[..])
            .map(Some)
            .map_err(|e| StorageError::Corrupt(format!("zstd: {e}")))
    }

    /// Writes a record copy-on-write: compress, append, sync, then update the
    /// header entry and sync again.
    pub fn write(&mut self, index: usize, record: &[u8]) -> Result<(), StorageError> {
        let compressed = zstd::bulk::compress(record, ZSTD_LEVEL)?;
        let offset = self.append(&compressed)?;
        self.commit_entry(
            index,
            Entry {
                offset,
                len: compressed.len() as u32,
                sum: checksum(&compressed),
            },
        )
    }

    fn append(&mut self, bytes: &[u8]) -> Result<u64, StorageError> {
        let offset = self.end;
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write_all(bytes)?;
        self.file.sync_data()?;
        self.end += bytes.len() as u64;
        Ok(offset)
    }

    fn commit_entry(&mut self, index: usize, e: Entry) -> Result<(), StorageError> {
        self.write_entry(index, e)?;
        self.file.sync_data()?;
        Ok(())
    }

    fn write_entry(&mut self, index: usize, e: Entry) -> Result<(), StorageError> {
        let mut b = [0u8; 16];
        b[..8].copy_from_slice(&e.offset.to_le_bytes());
        b[8..12].copy_from_slice(&e.len.to_le_bytes());
        b[12..].copy_from_slice(&e.sum.to_le_bytes());
        self.file
            .seek(SeekFrom::Start(8 + index as u64 * ENTRY_BYTES))?;
        self.file.write_all(&b)?;
        self.entries[index] = e;
        Ok(())
    }

    /// Writes several records with two syncs in total: all records are
    /// appended and synced first, then all header entries are updated and
    /// synced. A crash leaves each column at its old or its new version.
    pub fn write_batch(&mut self, records: &[(usize, Vec<u8>)]) -> Result<(), StorageError> {
        let mut pending = Vec::with_capacity(records.len());
        self.file.seek(SeekFrom::Start(self.end))?;
        for (index, record) in records {
            let compressed = zstd::bulk::compress(record, ZSTD_LEVEL)?;
            self.file.write_all(&compressed)?;
            pending.push((
                *index,
                Entry {
                    offset: self.end,
                    len: compressed.len() as u32,
                    sum: checksum(&compressed),
                },
            ));
            self.end += compressed.len() as u64;
        }
        self.file.sync_data()?;
        for (index, e) in pending {
            self.write_entry(index, e)?;
        }
        self.file.sync_data()?;
        Ok(())
    }

    /// Bytes no live record uses (old versions left by copy-on-write).
    #[must_use]
    pub fn garbage_bytes(&self) -> u64 {
        let live: u64 = self.entries.iter().map(|e| u64::from(e.len)).sum();
        self.end - HEADER_BYTES - live
    }

    /// Rewrites the region without garbage (temporary file + rename).
    pub fn compact(&mut self) -> Result<(), StorageError> {
        let tmp = self.path.with_extension("bin.tmp");
        let _ = fs::remove_file(&tmp);
        let mut out = RegionFile::open(&tmp)?;
        for i in 0..ENTRIES {
            let e = self.entries[i];
            if e.len == 0 {
                continue;
            }
            let mut compressed = vec![0u8; e.len as usize];
            self.file.seek(SeekFrom::Start(e.offset))?;
            self.file.read_exact(&mut compressed)?;
            let offset = out.append(&compressed)?;
            out.commit_entry(i, Entry { offset, ..e })?;
        }
        drop(out);
        fs::rename(&tmp, &self.path)?;
        *self = RegionFile::open(&self.path)?;
        Ok(())
    }

    #[cfg(test)]
    fn append_without_commit(&mut self, record: &[u8]) -> Result<(), StorageError> {
        let compressed = zstd::bulk::compress(record, ZSTD_LEVEL)?;
        self.append(&compressed).map(|_| ())
    }
}

/// The save directory of one dimension: region files plus metadata.
pub struct WorldStore {
    dir: PathBuf,
    meta: Database,
    regions: HashMap<(i32, i32), RegionFile>,
    /// Table number of the current registry.
    current_table: u32,
    /// `remaps[table]` for tables other than the current one (lazily filled).
    remaps: HashMap<u32, Vec<BlockStateId>>,
    missing: BlockStateId,
    bounds: WorldBounds,
}

impl WorldStore {
    /// Opens or creates a world directory for `registry` and `bounds`. If the
    /// registry's ID table differs from every stored one, it is stored as a
    /// new table; older columns are remapped on load.
    pub fn open(
        dir: &Path,
        registry: &Registry,
        bounds: WorldBounds,
    ) -> Result<Self, StorageError> {
        fs::create_dir_all(dir.join("region"))?;
        let meta = Database::create(dir.join("world.redb")).map_err(meta_err)?;
        let names = registry.id_table().names.join("\n");
        let txn = meta.begin_write().map_err(meta_err)?;
        let current_table;
        {
            let mut m = txn.open_table(META).map_err(meta_err)?;
            let version = m
                .get("format_version")
                .map_err(meta_err)?
                .map(|v| v.value());
            match version {
                Some(v) if v != FORMAT_VERSION => return Err(StorageError::Version(v)),
                Some(_) => {}
                None => {
                    m.insert("format_version", FORMAT_VERSION)
                        .map_err(meta_err)?;
                }
            }
            let mut t = txn.open_table(ID_TABLES).map_err(meta_err)?;
            let mut found = None;
            let mut next = 0u32;
            for row in t.iter().map_err(meta_err)? {
                let (k, v) = row.map_err(meta_err)?;
                next = next.max(k.value() + 1);
                if v.value() == names {
                    found = Some(k.value());
                }
            }
            current_table = match found {
                Some(k) => k,
                None => {
                    t.insert(next, names.as_str()).map_err(meta_err)?;
                    next
                }
            };
        }
        txn.commit().map_err(meta_err)?;
        let mut remaps = HashMap::new();
        let read = meta.begin_read().map_err(meta_err)?;
        let t = read.open_table(ID_TABLES).map_err(meta_err)?;
        for row in t.iter().map_err(meta_err)? {
            let (k, v) = row.map_err(meta_err)?;
            if k.value() != current_table {
                let saved = IdTable {
                    names: v.value().split('\n').map(str::to_owned).collect(),
                };
                remaps.insert(k.value(), registry.remap(&saved));
            }
        }
        drop(t);
        drop(read);
        Ok(Self {
            dir: dir.to_owned(),
            meta,
            regions: HashMap::new(),
            current_table,
            remaps,
            missing: registry.missing(),
            bounds,
        })
    }

    fn region(&mut self, pos: ColumnPos) -> Result<&mut RegionFile, StorageError> {
        let key = region_of(pos);
        if !self.regions.contains_key(&key) {
            let path = self
                .dir
                .join("region")
                .join(format!("r.{}.{}.bin", key.0, key.1));
            self.regions.insert(key, RegionFile::open(&path)?);
        }
        self.regions
            .get_mut(&key)
            .ok_or_else(|| StorageError::Corrupt("region cache".into()))
    }

    /// Saves a column snapshot with the current registry's IDs.
    pub fn save(&mut self, record: &ColumnRecord) -> Result<(), StorageError> {
        if record.bounds != self.bounds {
            return Err(StorageError::Bounds);
        }
        let bytes = encode_column(record, self.current_table);
        let index = local_index(record.pos);
        self.region(record.pos)?.write(index, &bytes)
    }

    /// Saves many columns: grouped by region, two syncs per region (the
    /// autosave path of the background writer job).
    pub fn save_all(&mut self, records: &[ColumnRecord]) -> Result<(), StorageError> {
        let mut by_region: HashMap<(i32, i32), Vec<RegionWrite>> = HashMap::new();
        for r in records {
            if r.bounds != self.bounds {
                return Err(StorageError::Bounds);
            }
            by_region
                .entry(region_of(r.pos))
                .or_default()
                .push((local_index(r.pos), encode_column(r, self.current_table)));
        }
        let mut keys: Vec<_> = by_region.keys().copied().collect();
        keys.sort_unstable();
        for key in keys {
            let batch = by_region.remove(&key).unwrap_or_default();
            let pos = ColumnPos {
                x: key.0 * REGION_SIZE,
                z: key.1 * REGION_SIZE,
            };
            self.region(pos)?.write_batch(&batch)?;
        }
        Ok(())
    }

    /// Loads a column, remapping IDs written by another mod set; `None` if
    /// never saved.
    pub fn load(&mut self, pos: ColumnPos) -> Result<Option<ColumnRecord>, StorageError> {
        let index = local_index(pos);
        let Some(bytes) = self.region(pos)?.read(index)? else {
            return Ok(None);
        };
        let (mut record, table) = decode_column(&bytes)?;
        if record.pos != pos {
            return Err(StorageError::Corrupt(
                "column stored at the wrong position".into(),
            ));
        }
        if record.bounds != self.bounds {
            return Err(StorageError::Bounds);
        }
        if table != self.current_table {
            let map = self
                .remaps
                .get(&table)
                .ok_or_else(|| StorageError::Corrupt(format!("unknown ID table {table}")))?;
            for s in &mut record.sections {
                s.remap(map, self.missing);
            }
        }
        Ok(Some(record))
    }

    /// Loads a column straight into a [`Column`] (heightmap recomputed).
    pub fn load_column(
        &mut self,
        pos: ColumnPos,
        registry: &Registry,
    ) -> Result<Option<(Column, ColumnRecord)>, StorageError> {
        let Some(mut record) = self.load(pos)? else {
            return Ok(None);
        };
        let sections = std::mem::take(&mut record.sections);
        let column = Column::from_sections(pos, self.bounds, sections, registry)
            .ok_or(StorageError::Bounds)?;
        Ok(Some((column, record)))
    }

    /// Compacts every open region with garbage above `threshold` bytes.
    pub fn compact(&mut self, threshold: u64) -> Result<(), StorageError> {
        for region in self.regions.values_mut() {
            if region.garbage_bytes() > threshold {
                region.compact()?;
            }
        }
        Ok(())
    }

    /// The metadata database (world index, future world settings).
    #[must_use]
    pub fn metadata(&self) -> &Database {
        &self.meta
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("world-storage-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).expect("temp dir");
        d
    }

    fn record(x: i32, fill: u16) -> Vec<u8> {
        let bounds = WorldBounds {
            min_section_y: 0,
            section_count: 2,
        };
        encode_column(
            &ColumnRecord {
                pos: ColumnPos { x, z: 0 },
                bounds,
                sections: vec![Section::uniform(BlockStateId(fill)), Section::new()],
                light: vec![1, 2, 3],
                entities: Vec::new(),
            },
            0,
        )
    }

    #[test]
    fn interrupted_write_keeps_previous_version() {
        let dir = temp("crash");
        let path = dir.join("r.0.0.bin");
        let mut r = RegionFile::open(&path).expect("open");
        r.write(5, &record(5, 7)).expect("write v1");
        // Crash after the data append, before the header entry update.
        r.append_without_commit(&record(5, 8)).expect("append");
        drop(r);
        let mut r = RegionFile::open(&path).expect("reopen");
        let (rec, _) = decode_column(&r.read(5).expect("read").expect("present")).expect("decode");
        assert_eq!(
            rec.sections[0].get(0),
            BlockStateId(7),
            "old version survives"
        );
        // A torn record (header points past a truncated file) is an error, not garbage.
        r.write(6, &record(6, 9)).expect("write");
        let len = fs::metadata(&path).expect("meta").len();
        drop(r);
        OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open")
            .set_len(len - 3)
            .expect("truncate");
        let mut r = RegionFile::open(&path).expect("reopen");
        assert!(r.read(6).is_err());
        assert!(r.read(5).expect("intact").is_some());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn compaction_drops_garbage_and_keeps_data() {
        let dir = temp("compact");
        let path = dir.join("r.0.0.bin");
        let mut r = RegionFile::open(&path).expect("open");
        for v in 0..5 {
            r.write(1, &record(1, v + 1)).expect("write");
            r.write(2, &record(2, 100)).expect("write");
        }
        assert!(r.garbage_bytes() > 0);
        r.compact().expect("compact");
        assert_eq!(r.garbage_bytes(), 0);
        let (one, _) = decode_column(&r.read(1).expect("read").expect("present")).expect("decode");
        assert_eq!(one.sections[0].get(0), BlockStateId(5));
        assert!(r.read(3).expect("read").is_none());
        let _ = fs::remove_dir_all(dir);
    }
}
