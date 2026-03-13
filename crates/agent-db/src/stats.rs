use std::fs;

use crate::engine::{list_data_files, Engine};
use crate::error::Result;

#[derive(Debug, Clone)]
pub struct DbStats {
    pub live_keys: usize,
    pub data_files: usize,
    pub total_data_bytes: u64,
    pub live_data_bytes: u64,
    pub dead_bytes: u64,
    pub fragmentation: f64,
}

pub fn compute_stats(engine: &Engine) -> Result<DbStats> {
    let config = engine.config();
    let keydir = engine.keydir().read().unwrap();

    let data_files = list_data_files(&config.dir)?;
    let num_files = data_files.len();

    let mut total_bytes: u64 = 0;
    for &file_id in &data_files {
        let path = config.dir.join(format!("{:06}.data", file_id));
        if let Ok(meta) = fs::metadata(&path) {
            total_bytes += meta.len();
        }
    }

    let mut live_bytes: u64 = 0;
    for (key, meta) in keydir.iter() {
        // Each live entry takes: header(20) + key_len + value_sz
        live_bytes += 20 + key.len() as u64 + meta.value_sz as u64;
    }

    let live_keys = keydir.len();
    let dead_bytes = total_bytes.saturating_sub(live_bytes);
    let fragmentation = if total_bytes > 0 {
        dead_bytes as f64 / total_bytes as f64
    } else {
        0.0
    };

    Ok(DbStats {
        live_keys,
        data_files: num_files,
        total_data_bytes: total_bytes,
        live_data_bytes: live_bytes,
        dead_bytes,
        fragmentation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::DbConfig;
    use tempfile::TempDir;

    #[test]
    fn empty_db_stats() {
        let dir = TempDir::new().unwrap();
        let engine = Engine::open(DbConfig::new(dir.path())).unwrap();
        let stats = compute_stats(&engine).unwrap();

        assert_eq!(stats.live_keys, 0);
        assert_eq!(stats.dead_bytes, 0);
        assert_eq!(stats.fragmentation, 0.0);
    }

    #[test]
    fn stats_after_writes() {
        let dir = TempDir::new().unwrap();
        let engine = Engine::open(DbConfig::new(dir.path())).unwrap();

        engine.put(b"key1", b"value1").unwrap();
        engine.put(b"key2", b"value2").unwrap();
        engine.sync().unwrap();

        let stats = compute_stats(&engine).unwrap();
        assert_eq!(stats.live_keys, 2);
        assert!(stats.total_data_bytes > 0);
        assert!(stats.live_data_bytes > 0);
        assert_eq!(stats.dead_bytes, 0);
    }

    #[test]
    fn stats_after_overwrites() {
        let dir = TempDir::new().unwrap();
        let engine = Engine::open(DbConfig::new(dir.path())).unwrap();

        engine.put(b"key1", b"value1").unwrap();
        engine.put(b"key1", b"value1_updated").unwrap(); // overwrite → dead bytes
        engine.sync().unwrap();

        let stats = compute_stats(&engine).unwrap();
        assert_eq!(stats.live_keys, 1);
        assert!(
            stats.dead_bytes > 0,
            "should have dead bytes from overwrite"
        );
        assert!(stats.fragmentation > 0.0);
    }

    #[test]
    fn stats_after_merge() {
        let dir = TempDir::new().unwrap();
        let config = DbConfig {
            dir: dir.path().to_path_buf(),
            max_file_size: 100,
            sync_on_write: false,
        };
        let engine = Engine::open(config).unwrap();

        // Create overwrites across multiple files
        for i in 0..8u32 {
            engine
                .put(
                    format!("key{i}").as_bytes(),
                    format!("original_val_{i}").as_bytes(),
                )
                .unwrap();
        }
        // Overwrite some to create dead data
        engine.put(b"key0", b"updated_value_0_long").unwrap();
        engine.put(b"key1", b"updated_value_1_long").unwrap();
        engine.sync().unwrap();

        let before = compute_stats(&engine).unwrap();
        assert!(before.dead_bytes > 0);

        engine.merge().unwrap();

        let after = compute_stats(&engine).unwrap();
        assert_eq!(after.live_keys, 8);
        assert!(
            after.dead_bytes < before.dead_bytes,
            "merge should reclaim dead bytes: before={}, after={}",
            before.dead_bytes,
            after.dead_bytes
        );
    }
}
