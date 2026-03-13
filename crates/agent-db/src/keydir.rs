use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct EntryMeta {
    pub file_id: u64,
    pub offset: u64,
    pub value_sz: u32,
    pub timestamp: u64,
}

#[derive(Debug)]
pub struct KeyDir {
    entries: HashMap<Vec<u8>, EntryMeta>,
}

impl KeyDir {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn get(&self, key: &[u8]) -> Option<&EntryMeta> {
        self.entries.get(key)
    }

    pub fn put(&mut self, key: Vec<u8>, meta: EntryMeta) {
        self.entries.insert(key, meta);
    }

    pub fn remove(&mut self, key: &[u8]) -> bool {
        self.entries.remove(key).is_some()
    }

    pub fn keys(&self) -> impl Iterator<Item = &Vec<u8>> {
        self.entries.keys()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Vec<u8>, &EntryMeta)> {
        self.entries.iter()
    }
}

impl Default for KeyDir {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(file_id: u64, offset: u64) -> EntryMeta {
        EntryMeta {
            file_id,
            offset,
            value_sz: 100,
            timestamp: 1000,
        }
    }

    #[test]
    fn put_and_get() {
        let mut kd = KeyDir::new();
        kd.put(b"key1".to_vec(), meta(1, 0));

        let m = kd.get(b"key1").unwrap();
        assert_eq!(m.file_id, 1);
        assert_eq!(m.offset, 0);
    }

    #[test]
    fn overwrite_replaces() {
        let mut kd = KeyDir::new();
        kd.put(b"key".to_vec(), meta(1, 0));
        kd.put(b"key".to_vec(), meta(2, 100));

        let m = kd.get(b"key").unwrap();
        assert_eq!(m.file_id, 2);
        assert_eq!(m.offset, 100);
    }

    #[test]
    fn remove_entry() {
        let mut kd = KeyDir::new();
        kd.put(b"key".to_vec(), meta(1, 0));
        assert!(kd.remove(b"key"));
        assert!(kd.get(b"key").is_none());
        assert!(!kd.remove(b"key")); // second remove returns false
    }

    #[test]
    fn keys_iteration() {
        let mut kd = KeyDir::new();
        kd.put(b"a".to_vec(), meta(1, 0));
        kd.put(b"b".to_vec(), meta(1, 100));
        kd.put(b"c".to_vec(), meta(1, 200));

        let mut keys: Vec<_> = kd.keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]);
    }

    #[test]
    fn len_tracking() {
        let mut kd = KeyDir::new();
        assert_eq!(kd.len(), 0);
        assert!(kd.is_empty());

        kd.put(b"a".to_vec(), meta(1, 0));
        kd.put(b"b".to_vec(), meta(1, 100));
        assert_eq!(kd.len(), 2);
        assert!(!kd.is_empty());

        kd.remove(b"a");
        assert_eq!(kd.len(), 1);
    }
}
