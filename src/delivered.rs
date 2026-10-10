//! The payloads Apple already delivered, so a tap or a repeated arrival of the same push is dropped.
//!
//! The file starts with [`HEADER`] and holds up to [`CAPACITY`] little-endian `u64` FNV-1a hashes of payloads, oldest first.

use std::io;
use std::path::PathBuf;

/// Names the format and its version.
const HEADER: &[u8; 8] = b"PUSHDUP\x01";

/// How many payloads are remembered.
pub(crate) const CAPACITY: usize = 64;

/// The delivered payloads of this app, on disk.
#[derive(Debug)]
pub(crate) struct Delivered {
    path: PathBuf,
}

impl Delivered {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Records `payload` and returns `true` when it was not delivered before.
    pub(crate) fn first_delivery(&self, payload: &[u8]) -> io::Result<bool> {
        let hash = fnv1a(payload);
        let mut hashes = self.read();
        if hashes.contains(&hash) {
            return Ok(false);
        }
        hashes.push(hash);
        let excess = hashes.len().saturating_sub(CAPACITY);
        let mut bytes = HEADER.to_vec();
        for hash in &hashes[excess..] {
            bytes.extend_from_slice(&hash.to_le_bytes());
        }
        std::fs::write(&self.path, bytes)?;
        Ok(true)
    }

    /// The remembered hashes, or none when the file is missing or not in this format.
    fn read(&self) -> Vec<u64> {
        let Ok(bytes) = std::fs::read(&self.path) else {
            return Vec::new();
        };
        let Some(body) = bytes.strip_prefix(HEADER.as_slice()) else {
            return Vec::new();
        };
        body.chunks_exact(8)
            .map(|chunk| u64::from_le_bytes(chunk.try_into().expect("chunks of 8 bytes")))
            .collect()
    }
}

/// The 64-bit FNV-1a hash, fixed by its specification so the file reads the same in every build.
fn fnv1a(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    bytes.iter().fold(OFFSET, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(PRIME)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Delivered) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let store = Delivered::new(dir.path().join("delivered"));
        (dir, store)
    }

    #[test]
    fn a_payload_is_delivered_once() {
        let (_dir, store) = store();
        assert!(store.first_delivery(b"{\"seq\":1}").unwrap());
        assert!(!store.first_delivery(b"{\"seq\":1}").unwrap());
        assert!(store.first_delivery(b"{\"seq\":2}").unwrap());
    }

    #[test]
    fn the_record_survives_the_process() {
        let (dir, store) = store();
        assert!(store.first_delivery(b"a").unwrap());
        let again = Delivered::new(dir.path().join("delivered"));
        assert!(!again.first_delivery(b"a").unwrap());
    }

    #[test]
    fn the_oldest_payload_is_forgotten_past_the_capacity() {
        let (_dir, store) = store();
        for seq in 0..=CAPACITY {
            assert!(store.first_delivery(seq.to_string().as_bytes()).unwrap());
        }
        assert!(
            store.first_delivery(b"0").unwrap(),
            "the oldest of 65 is forgotten"
        );
        assert!(
            !store
                .first_delivery(CAPACITY.to_string().as_bytes())
                .unwrap()
        );
    }

    #[test]
    fn an_unreadable_file_starts_over() {
        let (dir, store) = store();
        std::fs::write(dir.path().join("delivered"), b"not the format").unwrap();
        assert!(store.first_delivery(b"a").unwrap());
        assert!(!store.first_delivery(b"a").unwrap());
    }
}
