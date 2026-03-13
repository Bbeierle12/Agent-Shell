use std::fs::{self, OpenOptions};
use std::io::Write;

use crate::engine::{data_file_path_pub, list_data_files, Engine};
use crate::error::{DbError, Result};
use crate::format::Entry;
use crate::hint::{encode_hint_file, hint_file_path, HintEntry};
use crate::keydir::EntryMeta;

#[derive(Debug, Clone)]
pub struct MergeStats {
    pub files_merged: usize,
    pub entries_before: usize,
    pub entries_after: usize,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub tombstones_removed: usize,
}

pub fn merge(engine: &Engine) -> Result<MergeStats> {
    let config = engine.config();
    let active_id = engine.active_file_id();

    // Find immutable (non-active) data files
    let all_files = list_data_files(&config.dir)?;
    let immutable: Vec<u64> = all_files
        .into_iter()
        .filter(|&id| id != active_id)
        .collect();

    if immutable.is_empty() {
        return Ok(MergeStats {
            files_merged: 0,
            entries_before: 0,
            entries_after: 0,
            bytes_before: 0,
            bytes_after: 0,
            tombstones_removed: 0,
        });
    }

    let mut entries_before = 0;
    let mut entries_after = 0;
    let mut bytes_before: u64 = 0;
    let mut tombstones_removed = 0;

    // Allocate a new file ID for merged output
    let merged_file_id = active_id + 1000; // use a high ID to avoid collision
    let merged_path = data_file_path_pub(&config.dir, merged_file_id);
    let mut merged_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&merged_path)
        .map_err(|e| DbError::io(&merged_path, e))?;

    let mut hint_entries = Vec::new();
    let mut merged_offset: u64 = 0;
    let mut keydir = engine.keydir().write().unwrap();

    // Process each immutable file
    for &file_id in &immutable {
        let path = data_file_path_pub(&config.dir, file_id);
        let data = match fs::read(&path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(DbError::io(&path, e)),
        };

        bytes_before += data.len() as u64;
        let mut offset = 0usize;

        while offset < data.len() {
            match Entry::decode(&data[offset..]) {
                Ok((entry, entry_size)) => {
                    entries_before += 1;

                    if entry.is_tombstone() {
                        tombstones_removed += 1;
                        offset += entry_size;
                        continue;
                    }

                    // Check if this entry is the current live version
                    let is_live = keydir
                        .get(&entry.key)
                        .map(|meta| meta.file_id == file_id && meta.offset == offset as u64)
                        .unwrap_or(false);

                    if is_live {
                        // Write to merged file
                        let encoded = entry.encode();
                        merged_file
                            .write_all(&encoded)
                            .map_err(|e| DbError::io(&merged_path, e))?;

                        // Update keydir to point to new location
                        keydir.put(
                            entry.key.clone(),
                            EntryMeta {
                                file_id: merged_file_id,
                                offset: merged_offset,
                                value_sz: entry.value.len() as u32,
                                timestamp: entry.timestamp,
                            },
                        );

                        // Record hint entry
                        hint_entries.push(HintEntry::new(
                            entry.timestamp,
                            entry.value.len() as u32,
                            merged_offset,
                            entry.key,
                        ));

                        merged_offset += encoded.len() as u64;
                        entries_after += 1;
                    }

                    offset += entry_size;
                }
                Err(_) => break,
            }
        }
    }

    merged_file
        .sync_all()
        .map_err(|e| DbError::io(&merged_path, e))?;
    drop(merged_file);

    // Write hint file
    let hint_path = hint_file_path(&config.dir, merged_file_id);
    let hint_data = encode_hint_file(&hint_entries);
    fs::write(&hint_path, &hint_data).map_err(|e| DbError::io(&hint_path, e))?;

    // Delete old immutable files and their hint files
    for &file_id in &immutable {
        let data_path = data_file_path_pub(&config.dir, file_id);
        let _ = fs::remove_file(&data_path);
        let hint = hint_file_path(&config.dir, file_id);
        let _ = fs::remove_file(&hint);
    }

    drop(keydir);

    let bytes_after = merged_offset;

    Ok(MergeStats {
        files_merged: immutable.len(),
        entries_before,
        entries_after,
        bytes_before,
        bytes_after,
        tombstones_removed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::DbConfig;
    use crate::stats::compute_stats;
    use tempfile::TempDir;

    fn small_engine(dir: &std::path::Path) -> Engine {
        let config = DbConfig {
            dir: dir.to_path_buf(),
            max_file_size: 100, // tiny — forces rotation
            sync_on_write: false,
        };
        Engine::open(config).unwrap()
    }

    #[test]
    fn merge_removes_overwritten_entries() {
        let dir = TempDir::new().unwrap();
        let engine = small_engine(dir.path());

        // Write + overwrite to create dead data
        engine.put(b"key1", b"original_value").unwrap();
        engine.put(b"key1", b"updated_value").unwrap();
        engine.put(b"key2", b"another_value").unwrap();
        engine.sync().unwrap();

        let stats_before = compute_stats(&engine).unwrap();
        assert!(stats_before.dead_bytes > 0);

        let result = merge(&engine).unwrap();
        assert!(result.entries_before >= result.entries_after);

        // Data still accessible
        assert_eq!(engine.get(b"key1").unwrap().unwrap(), b"updated_value");
        assert_eq!(engine.get(b"key2").unwrap().unwrap(), b"another_value");
    }

    #[test]
    fn merge_removes_tombstones() {
        let dir = TempDir::new().unwrap();
        let engine = small_engine(dir.path());

        // Write enough to force multiple file rotations
        engine.put(b"keep", b"yes_this_is_a_keeper_value").unwrap();
        engine.put(b"gone", b"delete_me_this_value_long").unwrap();
        engine.put(b"pad1", b"padding_to_force_rotation").unwrap();
        engine.delete(b"gone").unwrap();
        engine.put(b"pad2", b"more_padding_force_rotate").unwrap();
        engine.sync().unwrap();

        let result = merge(&engine).unwrap();
        assert!(
            result.tombstones_removed > 0,
            "expected tombstones removed, got {:?}",
            result
        );

        assert_eq!(
            engine.get(b"keep").unwrap().unwrap(),
            b"yes_this_is_a_keeper_value"
        );
        assert!(engine.get(b"gone").unwrap().is_none());
    }

    #[test]
    fn merge_preserves_live_data() {
        let dir = TempDir::new().unwrap();
        let engine = small_engine(dir.path());

        for i in 0..10u32 {
            engine
                .put(
                    format!("key{i:02}").as_bytes(),
                    format!("value_{i}_padded").as_bytes(),
                )
                .unwrap();
        }
        engine.sync().unwrap();

        merge(&engine).unwrap();

        for i in 0..10u32 {
            let val = engine
                .get(format!("key{i:02}").as_bytes())
                .unwrap()
                .unwrap();
            assert_eq!(val, format!("value_{i}_padded").as_bytes());
        }
        assert_eq!(engine.len(), 10);
    }

    #[test]
    fn merge_creates_hint_file() {
        let dir = TempDir::new().unwrap();
        let engine = small_engine(dir.path());

        // Write enough to create multiple files
        for i in 0..8u32 {
            engine
                .put(
                    format!("key{i}").as_bytes(),
                    format!("value_{i}_padding_data").as_bytes(),
                )
                .unwrap();
        }
        engine.sync().unwrap();

        merge(&engine).unwrap();

        // Check that at least one hint file exists
        let hint_files: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".hint"))
            .collect();
        assert!(!hint_files.is_empty(), "merge should create a hint file");
    }
}
