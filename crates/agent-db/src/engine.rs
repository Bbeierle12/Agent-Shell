use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};

use crate::error::{DbError, Result};
use crate::format::Entry;
use crate::keydir::{EntryMeta, KeyDir};

const DATA_FILE_EXT: &str = "data";

#[derive(Debug, Clone)]
pub struct DbConfig {
    pub dir: PathBuf,
    pub max_file_size: u64,
    pub sync_on_write: bool,
}

impl DbConfig {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            max_file_size: 256 * 1024 * 1024, // 256 MB
            sync_on_write: false,
        }
    }
}

struct ActiveWriter {
    file: File,
    file_id: u64,
    offset: u64,
}

pub struct Engine {
    config: DbConfig,
    keydir: RwLock<KeyDir>,
    active: Mutex<ActiveWriter>,
}

impl Engine {
    pub fn open(config: DbConfig) -> Result<Self> {
        fs::create_dir_all(&config.dir).map_err(|e| DbError::io(&config.dir, e))?;

        let mut keydir = KeyDir::new();
        let data_files = list_data_files(&config.dir)?;

        // Rebuild keydir — prefer hint files when available
        for &file_id in &data_files {
            let hint_path = crate::hint::hint_file_path(&config.dir, file_id);
            if hint_path.exists() {
                rebuild_from_hint_file(&config.dir, file_id, &mut keydir)?;
            } else {
                rebuild_from_data_file(&config.dir, file_id, &mut keydir)?;
            }
        }

        // Determine active file ID
        let active_id = data_files.last().copied().unwrap_or(1);
        let active_path = data_file_path(&config.dir, active_id);
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&active_path)
            .map_err(|e| DbError::io(&active_path, e))?;

        let offset = file
            .seek(SeekFrom::End(0))
            .map_err(|e| DbError::io(&active_path, e))?;

        let active = ActiveWriter {
            file,
            file_id: active_id,
            offset,
        };

        Ok(Self {
            config,
            keydir: RwLock::new(keydir),
            active: Mutex::new(active),
        })
    }

    pub fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
        let entry = Entry::new(key.to_vec(), value.to_vec());
        let encoded = entry.encode();

        let mut active = self.active.lock().unwrap();

        // Rotate if needed
        if active.offset + encoded.len() as u64 > self.config.max_file_size {
            self.rotate_active_file(&mut active)?;
        }

        let offset = active.offset;
        let file_id = active.file_id;

        active
            .file
            .write_all(&encoded)
            .map_err(|e| DbError::io(data_file_path(&self.config.dir, file_id), e))?;

        if self.config.sync_on_write {
            active
                .file
                .sync_data()
                .map_err(|e| DbError::io(data_file_path(&self.config.dir, file_id), e))?;
        }

        active.offset += encoded.len() as u64;

        let meta = EntryMeta {
            file_id,
            offset,
            value_sz: value.len() as u32,
            timestamp: entry.timestamp,
        };

        self.keydir.write().unwrap().put(key.to_vec(), meta);
        Ok(())
    }

    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let keydir = self.keydir.read().unwrap();
        let meta = match keydir.get(key) {
            Some(m) => m,
            None => return Ok(None),
        };

        let value = self.read_value(meta)?;
        Ok(Some(value))
    }

    pub fn delete(&self, key: &[u8]) -> Result<bool> {
        // Check if key exists
        {
            let keydir = self.keydir.read().unwrap();
            if keydir.get(key).is_none() {
                return Ok(false);
            }
        }

        // Write tombstone
        let entry = Entry::tombstone(key.to_vec());
        let encoded = entry.encode();

        let mut active = self.active.lock().unwrap();

        if active.offset + encoded.len() as u64 > self.config.max_file_size {
            self.rotate_active_file(&mut active)?;
        }

        active
            .file
            .write_all(&encoded)
            .map_err(|e| DbError::io(data_file_path(&self.config.dir, active.file_id), e))?;

        active.offset += encoded.len() as u64;

        self.keydir.write().unwrap().remove(key);
        Ok(true)
    }

    pub fn keys(&self) -> Vec<Vec<u8>> {
        let keydir = self.keydir.read().unwrap();
        keydir.keys().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.keydir.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.keydir.read().unwrap().is_empty()
    }

    pub fn sync(&self) -> Result<()> {
        let active = self.active.lock().unwrap();
        active
            .file
            .sync_all()
            .map_err(|e| DbError::io(data_file_path(&self.config.dir, active.file_id), e))
    }

    // --- Internal ---

    fn read_value(&self, meta: &EntryMeta) -> Result<Vec<u8>> {
        let path = data_file_path(&self.config.dir, meta.file_id);
        let mut file = File::open(&path).map_err(|e| DbError::io(&path, e))?;
        file.seek(SeekFrom::Start(meta.offset))
            .map_err(|e| DbError::io(&path, e))?;

        // Read full entry to validate CRC — need header first for key_sz
        let mut header_buf = [0u8; crate::format::HEADER_SIZE];
        file.read_exact(&mut header_buf)
            .map_err(|e| DbError::io(&path, e))?;

        let key_sz = u32::from_le_bytes(header_buf[12..16].try_into().unwrap()) as usize;
        let value_sz = u32::from_le_bytes(header_buf[16..20].try_into().unwrap()) as usize;
        let total_size = crate::format::HEADER_SIZE + key_sz + value_sz;

        let mut full_buf = vec![0u8; total_size];
        full_buf[..crate::format::HEADER_SIZE].copy_from_slice(&header_buf);
        file.read_exact(&mut full_buf[crate::format::HEADER_SIZE..])
            .map_err(|e| DbError::io(&path, e))?;

        let (entry, _) = Entry::decode(&full_buf)?;
        Ok(entry.value)
    }

    fn rotate_active_file(&self, active: &mut ActiveWriter) -> Result<()> {
        active
            .file
            .sync_all()
            .map_err(|e| DbError::io(data_file_path(&self.config.dir, active.file_id), e))?;

        let new_id = active.file_id + 1;
        let new_path = data_file_path(&self.config.dir, new_id);
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&new_path)
            .map_err(|e| DbError::io(&new_path, e))?;

        active.file = file;
        active.file_id = new_id;
        active.offset = 0;
        Ok(())
    }

    // --- Accessors for merge/stats ---

    pub(crate) fn config(&self) -> &DbConfig {
        &self.config
    }

    pub(crate) fn keydir(&self) -> &RwLock<KeyDir> {
        &self.keydir
    }

    pub(crate) fn active_file_id(&self) -> u64 {
        self.active.lock().unwrap().file_id
    }

    /// Run compaction on immutable data files.
    pub fn merge(&self) -> Result<crate::merge::MergeStats> {
        crate::merge::merge(self)
    }

    /// Get database statistics.
    pub fn stats(&self) -> Result<crate::stats::DbStats> {
        crate::stats::compute_stats(self)
    }
}

// --- File utilities ---

fn data_file_path(dir: &Path, file_id: u64) -> PathBuf {
    dir.join(format!("{:06}.{DATA_FILE_EXT}", file_id))
}

pub(crate) fn list_data_files(dir: &Path) -> Result<Vec<u64>> {
    let mut ids = Vec::new();

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ids),
        Err(e) => return Err(DbError::io(dir, e)),
    };

    for entry in entries {
        let entry = entry.map_err(|e| DbError::io(dir, e))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(stem) = name.strip_suffix(&format!(".{DATA_FILE_EXT}")) {
            if let Ok(id) = stem.parse::<u64>() {
                ids.push(id);
            }
        }
    }

    ids.sort();
    Ok(ids)
}

pub(crate) fn rebuild_from_data_file(dir: &Path, file_id: u64, keydir: &mut KeyDir) -> Result<()> {
    let path = data_file_path(dir, file_id);
    let data = match fs::read(&path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(DbError::io(&path, e)),
    };

    let mut offset = 0usize;
    while offset < data.len() {
        match Entry::decode(&data[offset..]) {
            Ok((entry, entry_size)) => {
                if entry.is_tombstone() {
                    keydir.remove(&entry.key);
                } else {
                    keydir.put(
                        entry.key,
                        EntryMeta {
                            file_id,
                            offset: offset as u64,
                            value_sz: entry.value.len() as u32,
                            timestamp: entry.timestamp,
                        },
                    );
                }
                offset += entry_size;
            }
            Err(_) => break, // truncated tail from crash — stop here
        }
    }

    Ok(())
}

fn rebuild_from_hint_file(dir: &Path, file_id: u64, keydir: &mut KeyDir) -> Result<()> {
    let path = crate::hint::hint_file_path(dir, file_id);
    let data = match fs::read(&path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return rebuild_from_data_file(dir, file_id, keydir);
        }
        Err(e) => return Err(DbError::io(&path, e)),
    };

    let entries = crate::hint::decode_hint_file(&data)?;
    for entry in entries {
        keydir.put(
            entry.key,
            EntryMeta {
                file_id,
                offset: entry.value_pos,
                value_sz: entry.value_sz,
                timestamp: entry.timestamp,
            },
        );
    }

    Ok(())
}

// Re-export for merge module
pub(crate) fn data_file_path_pub(dir: &Path, file_id: u64) -> PathBuf {
    data_file_path(dir, file_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_engine() -> (Engine, TempDir) {
        let dir = TempDir::new().unwrap();
        let config = DbConfig::new(dir.path());
        let engine = Engine::open(config).unwrap();
        (engine, dir)
    }

    #[test]
    fn put_get_roundtrip() {
        let (engine, _dir) = test_engine();
        engine.put(b"name", b"Alice").unwrap();

        let val = engine.get(b"name").unwrap().unwrap();
        assert_eq!(val, b"Alice");
    }

    #[test]
    fn get_missing_returns_none() {
        let (engine, _dir) = test_engine();
        assert!(engine.get(b"nonexistent").unwrap().is_none());
    }

    #[test]
    fn delete_key() {
        let (engine, _dir) = test_engine();
        engine.put(b"key", b"value").unwrap();
        assert!(engine.delete(b"key").unwrap());
        assert!(engine.get(b"key").unwrap().is_none());
        assert!(!engine.delete(b"key").unwrap()); // already gone
    }

    #[test]
    fn overwrite_key() {
        let (engine, _dir) = test_engine();
        engine.put(b"key", b"v1").unwrap();
        engine.put(b"key", b"v2").unwrap();

        let val = engine.get(b"key").unwrap().unwrap();
        assert_eq!(val, b"v2");
        assert_eq!(engine.len(), 1);
    }

    #[test]
    fn keys_list() {
        let (engine, _dir) = test_engine();
        engine.put(b"a", b"1").unwrap();
        engine.put(b"b", b"2").unwrap();
        engine.put(b"c", b"3").unwrap();

        let mut keys = engine.keys();
        keys.sort();
        assert_eq!(keys, vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]);
    }

    #[test]
    fn persistence_across_reopen() {
        let dir = TempDir::new().unwrap();
        {
            let config = DbConfig::new(dir.path());
            let engine = Engine::open(config).unwrap();
            engine.put(b"persist", b"yes").unwrap();
            engine.put(b"temp", b"data").unwrap();
            engine.delete(b"temp").unwrap();
            engine.sync().unwrap();
        }

        // Reopen
        let config = DbConfig::new(dir.path());
        let engine = Engine::open(config).unwrap();
        assert_eq!(engine.get(b"persist").unwrap().unwrap(), b"yes");
        assert!(engine.get(b"temp").unwrap().is_none());
        assert_eq!(engine.len(), 1);
    }

    #[test]
    fn empty_db() {
        let (engine, _dir) = test_engine();
        assert!(engine.is_empty());
        assert_eq!(engine.len(), 0);
        assert!(engine.keys().is_empty());
    }

    #[test]
    fn large_value() {
        let (engine, _dir) = test_engine();
        let big = vec![0xAB; 1024 * 1024]; // 1 MB
        engine.put(b"big", &big).unwrap();

        let val = engine.get(b"big").unwrap().unwrap();
        assert_eq!(val.len(), big.len());
        assert_eq!(val, big);
    }

    #[test]
    fn file_rotation() {
        let dir = TempDir::new().unwrap();
        let config = DbConfig {
            dir: dir.path().to_path_buf(),
            max_file_size: 100, // tiny — forces rotation
            sync_on_write: false,
        };
        let engine = Engine::open(config).unwrap();

        // Write enough to trigger rotation
        for i in 0..10u32 {
            engine
                .put(format!("key{i}").as_bytes(), b"some value data here")
                .unwrap();
        }

        // Should have multiple data files
        let files = list_data_files(dir.path()).unwrap();
        assert!(files.len() > 1);

        // All keys still readable
        for i in 0..10u32 {
            let val = engine.get(format!("key{i}").as_bytes()).unwrap();
            assert!(val.is_some());
        }
    }
}
