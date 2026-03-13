use std::path::Path;

use crate::blob::BlobStore;
use crate::collection::Collection;
use crate::engine::{DbConfig, Engine};
use crate::error::Result;
use crate::merge::MergeStats;
use crate::stats::DbStats;

/// High-level database handle wrapping the low-level [`Engine`].
///
/// Provides typed collection access, blob storage, and convenience
/// constructors so that callers never need to touch `Engine` directly.
pub struct Database {
    engine: Engine,
}

impl Database {
    /// Open (or create) a database with the given configuration.
    pub fn open(config: DbConfig) -> Result<Self> {
        let engine = Engine::open(config)?;
        Ok(Self { engine })
    }

    /// Open (or create) a database at the given directory path with defaults.
    pub fn open_at(path: impl AsRef<Path>) -> Result<Self> {
        Self::open(DbConfig::new(path.as_ref()))
    }

    /// Return a typed collection scoped to `name`.
    pub fn collection<T>(&self, name: &str) -> Collection<'_, T>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Clone + 'static,
    {
        Collection::new(&self.engine, name)
    }

    /// Access the underlying engine (for advanced / raw operations).
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Compute database statistics.
    pub fn stats(&self) -> Result<DbStats> {
        self.engine.stats()
    }

    /// Run a compaction / merge pass.
    pub fn merge(&self) -> Result<MergeStats> {
        self.engine.merge()
    }

    /// Flush writes to disk.
    pub fn sync(&self) -> Result<()> {
        self.engine.sync()
    }

    /// Return a namespaced blob store for raw key-value access.
    pub fn blob_store(&self, namespace: &str) -> BlobStore<'_> {
        BlobStore::new(&self.engine, namespace)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use tempfile::TempDir;

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct Widget {
        label: String,
        count: u32,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct Gadget {
        name: String,
    }

    fn tmp_db() -> (Database, TempDir) {
        let dir = TempDir::new().unwrap();
        let db = Database::open_at(dir.path()).unwrap();
        (db, dir)
    }

    #[test]
    fn open_and_collection_access() {
        let (db, _dir) = tmp_db();
        let coll = db.collection::<Widget>("widgets");
        let id = coll
            .insert(&Widget {
                label: "sprocket".into(),
                count: 7,
            })
            .unwrap();
        let loaded = coll.get(id).unwrap().unwrap();
        assert_eq!(loaded.label, "sprocket");
        assert_eq!(loaded.count, 7);
    }

    #[test]
    fn multiple_typed_collections() {
        let (db, _dir) = tmp_db();

        let widgets = db.collection::<Widget>("widgets");
        let gadgets = db.collection::<Gadget>("gadgets");

        widgets
            .insert(&Widget {
                label: "a".into(),
                count: 1,
            })
            .unwrap();
        gadgets.insert(&Gadget { name: "b".into() }).unwrap();

        assert_eq!(widgets.count().unwrap(), 1);
        assert_eq!(gadgets.count().unwrap(), 1);
    }

    #[test]
    fn open_at_convenience() {
        let dir = TempDir::new().unwrap();
        let db = Database::open_at(dir.path()).unwrap();
        let coll = db.collection::<Widget>("w");
        coll.insert(&Widget {
            label: "x".into(),
            count: 0,
        })
        .unwrap();
        assert_eq!(coll.count().unwrap(), 1);
    }

    #[test]
    fn stats_and_sync_delegation() {
        let (db, _dir) = tmp_db();
        let coll = db.collection::<Widget>("w");
        coll.insert(&Widget {
            label: "y".into(),
            count: 5,
        })
        .unwrap();

        db.sync().unwrap();
        let stats = db.stats().unwrap();
        assert!(stats.live_keys > 0);
    }
}
