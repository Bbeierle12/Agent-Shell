use crate::error::{DbError, Result};

/// Hint file entry format:
/// | timestamp:u64 | key_sz:u32 | value_sz:u32 | value_pos:u64 | key:[u8] |
///   8 bytes         4 bytes      4 bytes        8 bytes          var
///
/// Total header: 24 bytes fixed + variable key.
/// Used on startup to rebuild keydir without reading values from data files.
pub const HINT_HEADER_SIZE: usize = 24;
pub const HINT_FILE_EXT: &str = "hint";

#[derive(Debug, Clone)]
pub struct HintEntry {
    pub timestamp: u64,
    pub key_sz: u32,
    pub value_sz: u32,
    pub value_pos: u64,
    pub key: Vec<u8>,
}

impl HintEntry {
    pub fn new(timestamp: u64, value_sz: u32, value_pos: u64, key: Vec<u8>) -> Self {
        Self {
            timestamp,
            key_sz: key.len() as u32,
            value_sz,
            value_pos,
            key,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let total = HINT_HEADER_SIZE + self.key.len();
        let mut buf = Vec::with_capacity(total);
        buf.extend_from_slice(&self.timestamp.to_le_bytes());
        buf.extend_from_slice(&self.key_sz.to_le_bytes());
        buf.extend_from_slice(&self.value_sz.to_le_bytes());
        buf.extend_from_slice(&self.value_pos.to_le_bytes());
        buf.extend_from_slice(&self.key);
        buf
    }

    pub fn decode(data: &[u8]) -> Result<(Self, usize)> {
        if data.len() < HINT_HEADER_SIZE {
            return Err(DbError::Corrupt(format!(
                "hint data too short: {} < {HINT_HEADER_SIZE}",
                data.len()
            )));
        }

        let timestamp = u64::from_le_bytes(data[0..8].try_into().unwrap());
        let key_sz = u32::from_le_bytes(data[8..12].try_into().unwrap()) as usize;
        let value_sz = u32::from_le_bytes(data[12..16].try_into().unwrap());
        let value_pos = u64::from_le_bytes(data[16..24].try_into().unwrap());

        let total = HINT_HEADER_SIZE + key_sz;
        if data.len() < total {
            return Err(DbError::Corrupt(format!(
                "hint data too short for key: {} < {total}",
                data.len()
            )));
        }

        let key = data[HINT_HEADER_SIZE..total].to_vec();

        Ok((
            Self {
                timestamp,
                key_sz: key_sz as u32,
                value_sz,
                value_pos,
                key,
            },
            total,
        ))
    }
}

pub fn encode_hint_file(entries: &[HintEntry]) -> Vec<u8> {
    let mut buf = Vec::new();
    for entry in entries {
        buf.extend_from_slice(&entry.encode());
    }
    buf
}

pub fn decode_hint_file(data: &[u8]) -> Result<Vec<HintEntry>> {
    let mut entries = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        let (entry, size) = HintEntry::decode(&data[offset..])?;
        entries.push(entry);
        offset += size;
    }
    Ok(entries)
}

pub fn hint_file_path(dir: &std::path::Path, file_id: u64) -> std::path::PathBuf {
    dir.join(format!("{:06}.{HINT_FILE_EXT}", file_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip() {
        let entry = HintEntry::new(12345, 100, 2048, b"mykey".to_vec());
        let encoded = entry.encode();
        let (decoded, size) = HintEntry::decode(&encoded).unwrap();

        assert_eq!(size, encoded.len());
        assert_eq!(decoded.timestamp, 12345);
        assert_eq!(decoded.value_sz, 100);
        assert_eq!(decoded.value_pos, 2048);
        assert_eq!(decoded.key, b"mykey");
    }

    #[test]
    fn file_write_read_roundtrip() {
        let entries = vec![
            HintEntry::new(100, 50, 0, b"key1".to_vec()),
            HintEntry::new(200, 75, 1024, b"key2".to_vec()),
            HintEntry::new(300, 120, 2048, b"key3".to_vec()),
        ];

        let data = encode_hint_file(&entries);
        let decoded = decode_hint_file(&data).unwrap();

        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded[0].key, b"key1");
        assert_eq!(decoded[1].value_pos, 1024);
        assert_eq!(decoded[2].timestamp, 300);
    }
}
