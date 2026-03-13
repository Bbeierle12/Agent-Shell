use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::engine::Engine;
use crate::error::Result;

/// Generic, namespaced key-value blob store backed by the raw [`Engine`].
///
/// Each key is prefixed with `namespace:` to avoid collisions.
pub struct BlobStore<'a> {
    engine: &'a Engine,
    namespace: String,
}

impl<'a> BlobStore<'a> {
    /// Create a blob store scoped to `namespace`.
    pub fn new(engine: &'a Engine, namespace: &str) -> Self {
        Self {
            engine,
            namespace: namespace.to_string(),
        }
    }

    fn prefixed_key(&self, key: &str) -> Vec<u8> {
        format!("{}:{}", self.namespace, key).into_bytes()
    }

    /// Store a serializable value under `key`.
    pub fn put<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        let data = serde_json::to_vec(value)?;
        self.engine.put(&self.prefixed_key(key), &data)
    }

    /// Retrieve and deserialize a value by `key`, or `None` if absent.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        match self.engine.get(&self.prefixed_key(key))? {
            Some(data) => Ok(Some(serde_json::from_slice(&data)?)),
            None => Ok(None),
        }
    }

    /// Delete a key. Returns `true` if the key existed.
    pub fn delete(&self, key: &str) -> Result<bool> {
        let k = self.prefixed_key(key);
        if self.engine.get(&k)?.is_some() {
            self.engine.delete(&k)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Check whether a key exists.
    pub fn exists(&self, key: &str) -> Result<bool> {
        Ok(self.engine.get(&self.prefixed_key(key))?.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::DbConfig;
    use serde::{Deserialize, Serialize};
    use tempfile::TempDir;

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct Artifact {
        hash: String,
        size: u64,
    }

    fn tmp_engine() -> (Engine, TempDir) {
        let dir = TempDir::new().unwrap();
        let engine = Engine::open(DbConfig::new(dir.path())).unwrap();
        (engine, dir)
    }

    #[test]
    fn put_get_roundtrip() {
        let (engine, _dir) = tmp_engine();
        let store = BlobStore::new(&engine, "blobs");

        let art = Artifact {
            hash: "abc123".into(),
            size: 4096,
        };
        store.put("abc123", &art).unwrap();

        let loaded: Artifact = store.get("abc123").unwrap().unwrap();
        assert_eq!(loaded, art);
    }

    #[test]
    fn missing_key_returns_none() {
        let (engine, _dir) = tmp_engine();
        let store = BlobStore::new(&engine, "blobs");
        let result: Option<Artifact> = store.get("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn delete_key() {
        let (engine, _dir) = tmp_engine();
        let store = BlobStore::new(&engine, "blobs");

        let art = Artifact {
            hash: "del".into(),
            size: 100,
        };
        store.put("del", &art).unwrap();
        assert!(store.delete("del").unwrap());
        assert!(!store.delete("del").unwrap());
        assert!(store.get::<Artifact>("del").unwrap().is_none());
    }

    #[test]
    fn exists_check() {
        let (engine, _dir) = tmp_engine();
        let store = BlobStore::new(&engine, "blobs");

        assert!(!store.exists("key").unwrap());
        store
            .put(
                "key",
                &Artifact {
                    hash: "k".into(),
                    size: 1,
                },
            )
            .unwrap();
        assert!(store.exists("key").unwrap());
    }
}
