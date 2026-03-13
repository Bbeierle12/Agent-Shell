use serde::{de::DeserializeOwned, Serialize};

use crate::engine::Engine;
use crate::error::{DbError, Result};
use crate::query::Query;

const NEXT_ID_SUFFIX: &str = "__next_id";

pub struct Collection<'a, T> {
    engine: &'a Engine,
    name: String,
    _marker: std::marker::PhantomData<T>,
}

impl<'a, T: Serialize + DeserializeOwned + Clone + 'static> Collection<'a, T> {
    pub fn new(engine: &'a Engine, name: &str) -> Self {
        Self {
            engine,
            name: name.to_string(),
            _marker: std::marker::PhantomData,
        }
    }

    /// Insert a record, returning the auto-incremented ID.
    pub fn insert(&self, record: &T) -> Result<u64> {
        let id = self.next_id()?;
        let key = self.record_key(id);
        let value = serde_json::to_vec(record)?;
        self.engine.put(key.as_bytes(), &value)?;
        Ok(id)
    }

    /// Get a record by ID.
    pub fn get(&self, id: u64) -> Result<Option<T>> {
        let key = self.record_key(id);
        match self.engine.get(key.as_bytes())? {
            Some(data) => {
                let record: T = serde_json::from_slice(&data)?;
                Ok(Some(record))
            }
            None => Ok(None),
        }
    }

    /// Update an existing record.
    pub fn update(&self, id: u64, record: &T) -> Result<()> {
        let key = self.record_key(id);
        // Check exists
        if self.engine.get(key.as_bytes())?.is_none() {
            return Err(DbError::NotFound {
                collection: self.name.clone(),
                id,
            });
        }
        let value = serde_json::to_vec(record)?;
        self.engine.put(key.as_bytes(), &value)?;
        Ok(())
    }

    /// Delete a record by ID.
    pub fn delete(&self, id: u64) -> Result<bool> {
        let key = self.record_key(id);
        self.engine.delete(key.as_bytes())
    }

    /// List all records as (id, record) pairs.
    pub fn list_all(&self) -> Result<Vec<(u64, T)>> {
        let prefix = format!("{}:", self.name);
        let next_id_key = format!("{}{}", prefix, NEXT_ID_SUFFIX);
        let keys = self.engine.keys();

        let mut results = Vec::new();
        for key in keys {
            let key_str = match std::str::from_utf8(&key) {
                Ok(s) => s,
                Err(_) => continue,
            };
            if !key_str.starts_with(&prefix) || key_str == next_id_key {
                continue;
            }
            let id_str = &key_str[prefix.len()..];
            let id: u64 = match id_str.parse() {
                Ok(id) => id,
                Err(_) => continue,
            };
            if let Some(data) = self.engine.get(&key)? {
                let record: T = serde_json::from_slice(&data)?;
                results.push((id, record));
            }
        }

        results.sort_by_key(|(id, _)| *id);
        Ok(results)
    }

    /// Query records using a predicate builder.
    pub fn query(&self, query: &Query<T>) -> Result<Vec<(u64, T)>> {
        let all = self.list_all()?;
        let records: Vec<T> = all.iter().map(|(_, r)| r).cloned().collect();
        let ids: Vec<u64> = all.iter().map(|(id, _)| *id).collect();

        let matched: Vec<&T> = query.execute(&records);

        // Map matched references back to (id, T)
        let mut results = Vec::new();
        for matched_ref in matched {
            let ptr = matched_ref as *const T;
            for (i, record) in records.iter().enumerate() {
                if std::ptr::eq(ptr, record) {
                    results.push((ids[i], records[i].clone()));
                    break;
                }
            }
        }

        Ok(results)
    }

    /// Count live records.
    pub fn count(&self) -> Result<usize> {
        Ok(self.list_all()?.len())
    }

    // --- Internal ---

    fn record_key(&self, id: u64) -> String {
        format!("{}:{}", self.name, id)
    }

    fn next_id_key(&self) -> String {
        format!("{}:{}", self.name, NEXT_ID_SUFFIX)
    }

    fn next_id(&self) -> Result<u64> {
        let key = self.next_id_key();
        let current = match self.engine.get(key.as_bytes())? {
            Some(data) => {
                let s = String::from_utf8(data)
                    .map_err(|_| DbError::Corrupt("next_id is not UTF-8".into()))?;
                s.parse::<u64>()
                    .map_err(|_| DbError::Corrupt("next_id is not a number".into()))?
            }
            None => 0,
        };

        let next = current + 1;
        self.engine
            .put(key.as_bytes(), next.to_string().as_bytes())?;
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::DbConfig;
    use serde::{Deserialize, Serialize};
    use tempfile::TempDir;

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct TestRecord {
        name: String,
        score: i32,
    }

    fn test_engine() -> (Engine, TempDir) {
        let dir = TempDir::new().unwrap();
        let config = DbConfig::new(dir.path());
        let engine = Engine::open(config).unwrap();
        (engine, dir)
    }

    #[test]
    fn insert_get_roundtrip() {
        let (engine, _dir) = test_engine();
        let coll = Collection::<TestRecord>::new(&engine, "items");

        let rec = TestRecord {
            name: "Alice".into(),
            score: 100,
        };
        let id = coll.insert(&rec).unwrap();
        assert_eq!(id, 1);

        let loaded = coll.get(id).unwrap().unwrap();
        assert_eq!(loaded, rec);
    }

    #[test]
    fn auto_increment_ids() {
        let (engine, _dir) = test_engine();
        let coll = Collection::<TestRecord>::new(&engine, "items");

        let id1 = coll
            .insert(&TestRecord {
                name: "A".into(),
                score: 1,
            })
            .unwrap();
        let id2 = coll
            .insert(&TestRecord {
                name: "B".into(),
                score: 2,
            })
            .unwrap();
        let id3 = coll
            .insert(&TestRecord {
                name: "C".into(),
                score: 3,
            })
            .unwrap();

        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(id3, 3);
    }

    #[test]
    fn update_record() {
        let (engine, _dir) = test_engine();
        let coll = Collection::<TestRecord>::new(&engine, "items");

        let id = coll
            .insert(&TestRecord {
                name: "Old".into(),
                score: 1,
            })
            .unwrap();

        coll.update(
            id,
            &TestRecord {
                name: "New".into(),
                score: 99,
            },
        )
        .unwrap();

        let loaded = coll.get(id).unwrap().unwrap();
        assert_eq!(loaded.name, "New");
        assert_eq!(loaded.score, 99);
    }

    #[test]
    fn delete_and_count() {
        let (engine, _dir) = test_engine();
        let coll = Collection::<TestRecord>::new(&engine, "items");

        let id1 = coll
            .insert(&TestRecord {
                name: "A".into(),
                score: 1,
            })
            .unwrap();
        coll.insert(&TestRecord {
            name: "B".into(),
            score: 2,
        })
        .unwrap();

        assert_eq!(coll.count().unwrap(), 2);

        assert!(coll.delete(id1).unwrap());
        assert_eq!(coll.count().unwrap(), 1);
        assert!(coll.get(id1).unwrap().is_none());
    }

    #[test]
    fn list_all_sorted() {
        let (engine, _dir) = test_engine();
        let coll = Collection::<TestRecord>::new(&engine, "items");

        coll.insert(&TestRecord {
            name: "C".into(),
            score: 3,
        })
        .unwrap();
        coll.insert(&TestRecord {
            name: "A".into(),
            score: 1,
        })
        .unwrap();
        coll.insert(&TestRecord {
            name: "B".into(),
            score: 2,
        })
        .unwrap();

        let all = coll.list_all().unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].0, 1); // sorted by ID
        assert_eq!(all[0].1.name, "C");
    }

    #[test]
    fn query_with_filter() {
        let (engine, _dir) = test_engine();
        let coll = Collection::<TestRecord>::new(&engine, "items");

        coll.insert(&TestRecord {
            name: "Low".into(),
            score: 10,
        })
        .unwrap();
        coll.insert(&TestRecord {
            name: "High".into(),
            score: 90,
        })
        .unwrap();
        coll.insert(&TestRecord {
            name: "Mid".into(),
            score: 50,
        })
        .unwrap();

        let q = Query::new().filter(|r: &TestRecord| r.score > 40);
        let results = coll.query(&q).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn persistence_across_reopen() {
        let dir = TempDir::new().unwrap();
        {
            let engine = Engine::open(DbConfig::new(dir.path())).unwrap();
            let coll = Collection::<TestRecord>::new(&engine, "items");
            coll.insert(&TestRecord {
                name: "Persistent".into(),
                score: 42,
            })
            .unwrap();
            engine.sync().unwrap();
        }

        let engine = Engine::open(DbConfig::new(dir.path())).unwrap();
        let coll = Collection::<TestRecord>::new(&engine, "items");
        let all = coll.list_all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].1.name, "Persistent");

        // New inserts continue from correct ID
        let id = coll
            .insert(&TestRecord {
                name: "Next".into(),
                score: 43,
            })
            .unwrap();
        assert_eq!(id, 2);
    }

    #[test]
    fn multiple_collections_isolated() {
        let (engine, _dir) = test_engine();
        let coll_a = Collection::<TestRecord>::new(&engine, "alpha");
        let coll_b = Collection::<TestRecord>::new(&engine, "beta");

        coll_a
            .insert(&TestRecord {
                name: "InA".into(),
                score: 1,
            })
            .unwrap();
        coll_b
            .insert(&TestRecord {
                name: "InB".into(),
                score: 2,
            })
            .unwrap();

        assert_eq!(coll_a.count().unwrap(), 1);
        assert_eq!(coll_b.count().unwrap(), 1);
        assert_eq!(coll_a.list_all().unwrap()[0].1.name, "InA");
        assert_eq!(coll_b.list_all().unwrap()[0].1.name, "InB");
    }
}
