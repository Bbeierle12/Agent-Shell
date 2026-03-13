use crate::error::{DbError, Result};

/// On-disk entry format:
/// | CRC:u32 | timestamp:u64 | key_sz:u32 | value_sz:u32 | key:[u8] | value:[u8] |
///   4 bytes    8 bytes         4 bytes      4 bytes        var        var
///
/// Header = 20 bytes fixed. Tombstone = value_sz == 0 and empty value.
pub const HEADER_SIZE: usize = 20;

#[derive(Debug, Clone)]
pub struct Entry {
    pub crc: u32,
    pub timestamp: u64,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

impl Entry {
    pub fn new(key: Vec<u8>, value: Vec<u8>) -> Self {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let mut entry = Self {
            crc: 0,
            timestamp,
            key,
            value,
        };
        entry.crc = entry.compute_crc();
        entry
    }

    pub fn tombstone(key: Vec<u8>) -> Self {
        Self::new(key, Vec::new())
    }

    pub fn is_tombstone(&self) -> bool {
        self.value.is_empty()
    }

    pub fn compute_crc(&self) -> u32 {
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&self.timestamp.to_le_bytes());
        hasher.update(&(self.key.len() as u32).to_le_bytes());
        hasher.update(&(self.value.len() as u32).to_le_bytes());
        hasher.update(&self.key);
        hasher.update(&self.value);
        hasher.finalize()
    }

    pub fn encoded_size(&self) -> usize {
        HEADER_SIZE + self.key.len() + self.value.len()
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.encoded_size());
        buf.extend_from_slice(&self.crc.to_le_bytes());
        buf.extend_from_slice(&self.timestamp.to_le_bytes());
        buf.extend_from_slice(&(self.key.len() as u32).to_le_bytes());
        buf.extend_from_slice(&(self.value.len() as u32).to_le_bytes());
        buf.extend_from_slice(&self.key);
        buf.extend_from_slice(&self.value);
        buf
    }

    pub fn decode(data: &[u8]) -> Result<(Self, usize)> {
        if data.len() < HEADER_SIZE {
            return Err(DbError::Corrupt(format!(
                "data too short for header: {} < {HEADER_SIZE}",
                data.len()
            )));
        }

        let crc = u32::from_le_bytes(data[0..4].try_into().unwrap());
        let timestamp = u64::from_le_bytes(data[4..12].try_into().unwrap());
        let key_sz = u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize;
        let value_sz = u32::from_le_bytes(data[16..20].try_into().unwrap()) as usize;

        let total = HEADER_SIZE + key_sz + value_sz;
        if data.len() < total {
            return Err(DbError::Corrupt(format!(
                "data too short for entry: {} < {total}",
                data.len()
            )));
        }

        let key = data[HEADER_SIZE..HEADER_SIZE + key_sz].to_vec();
        let value = data[HEADER_SIZE + key_sz..total].to_vec();

        let entry = Self {
            crc,
            timestamp,
            key,
            value,
        };

        let actual_crc = entry.compute_crc();
        if crc != actual_crc {
            return Err(DbError::CrcMismatch {
                expected: crc,
                actual: actual_crc,
            });
        }

        Ok((entry, total))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip() {
        let entry = Entry::new(b"hello".to_vec(), b"world".to_vec());
        let encoded = entry.encode();
        let (decoded, size) = Entry::decode(&encoded).unwrap();

        assert_eq!(size, encoded.len());
        assert_eq!(decoded.key, b"hello");
        assert_eq!(decoded.value, b"world");
        assert_eq!(decoded.timestamp, entry.timestamp);
        assert_eq!(decoded.crc, entry.crc);
    }

    #[test]
    fn tombstone_has_empty_value() {
        let entry = Entry::tombstone(b"gone".to_vec());
        assert!(entry.is_tombstone());
        assert!(entry.value.is_empty());

        let encoded = entry.encode();
        let (decoded, _) = Entry::decode(&encoded).unwrap();
        assert!(decoded.is_tombstone());
    }

    #[test]
    fn crc_corruption_detected() {
        let entry = Entry::new(b"key".to_vec(), b"value".to_vec());
        let mut encoded = entry.encode();
        // Corrupt a data byte
        let last = encoded.len() - 1;
        encoded[last] ^= 0xFF;

        let result = Entry::decode(&encoded);
        assert!(matches!(result, Err(DbError::CrcMismatch { .. })));
    }

    #[test]
    fn encoded_size_matches() {
        let entry = Entry::new(b"key123".to_vec(), b"value456789".to_vec());
        assert_eq!(entry.encoded_size(), HEADER_SIZE + 6 + 11);
        assert_eq!(entry.encode().len(), entry.encoded_size());
    }
}
