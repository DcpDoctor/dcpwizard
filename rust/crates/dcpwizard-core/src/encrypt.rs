//! Content-key generation and persistence for encrypted DCPs.
//!
//! Keys are minted with the OS CSPRNG and the essence is encrypted at wrap time
//! (see [`crate::mxf_wrap`]). This module only generates keys and reads/writes
//! the sidecar keys file the `kdm` step consumes to bind a KDM to the actual
//! content keys. The keys file holds the plaintext AES keys: it is as sensitive
//! as a master key and must never ship inside the DCP or be committed anywhere.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Default sidecar filename written next to a DCP when `--encrypt` is used.
pub const KEYS_FILE_NAME: &str = "KEYS.json";

/// SMPTE ST 430-1 content key type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyType {
    /// Image essence key (MDIK).
    Mdik,
    /// Audio essence key (MDAK).
    Mdak,
    /// Timed-text essence key (MDSK), for subtitle and closed-caption tracks.
    Mdsk,
    /// Auxiliary-data essence key (MDEK), for the ST 429-18 Atmos track.
    Mdek,
}

impl KeyType {
    pub fn as_smpte(self) -> [u8; 4] {
        match self {
            KeyType::Mdik => *b"MDIK",
            KeyType::Mdak => *b"MDAK",
            KeyType::Mdsk => *b"MDSK",
            KeyType::Mdek => *b"MDEK",
        }
    }
}

/// One content key: which essence it encrypts and the raw key material (hex).
#[derive(Clone, Serialize, Deserialize)]
pub struct ContentKey {
    pub key_type: KeyType,
    /// KeyId (UUID) recorded in the MXF header and the CPL/KDM.
    pub key_id: String,
    /// Bare UUID of the encrypted track file this key belongs to.
    pub asset_uuid: String,
    /// 32-char lowercase hex of the 16-byte AES-128 key.
    pub content_key_hex: String,
}

/// Redact the key so it cannot reach a log through a stray debug print.
impl std::fmt::Debug for ContentKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContentKey")
            .field("key_type", &self.key_type)
            .field("key_id", &self.key_id)
            .field("asset_uuid", &self.asset_uuid)
            .field("content_key_hex", &"<redacted>")
            .finish()
    }
}

impl ContentKey {
    /// Raw (key_type, key_id, content_key) for the KDM key block.
    pub fn to_raw(&self) -> Result<([u8; 4], uuid::Uuid, [u8; 16]), String> {
        let key_id = uuid::Uuid::parse_str(self.key_id.trim_start_matches("urn:uuid:"))
            .map_err(|e| format!("bad key_id '{}': {e}", self.key_id))?;
        let key = hex_decode_16(&self.content_key_hex)?;
        Ok((self.key_type.as_smpte(), key_id, key))
    }
}

/// All content keys for one DCP, bound to the CPL they decrypt.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct KeyBundle {
    pub cpl_id: String,
    pub keys: Vec<ContentKey>,
}

impl KeyBundle {
    pub fn write(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("cannot serialize keys: {e}"))?;
        // holds raw content keys: atomic_write's tempfile gives it mode 0600
        crate::store::atomic_write(path, json.as_bytes())
    }

    pub fn read(path: &Path) -> Result<Self, String> {
        let json = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read keys file {}: {e}", path.display()))?;
        serde_json::from_str(&json).map_err(|e| format!("bad keys file {}: {e}", path.display()))
    }
}

/// A freshly minted key: the record for the keys file plus the raw material the
/// MXF writer needs.
pub struct GeneratedKey {
    pub info: ContentKey,
    pub key: [u8; 16],
    pub key_id: [u8; 16],
}

/// Generate a random AES-128 content key and KeyId using the OS CSPRNG.
pub fn generate_content_key(key_type: KeyType, asset_uuid: &str) -> Result<GeneratedKey, String> {
    let mut key = [0u8; 16];
    getrandom::getrandom(&mut key).map_err(|e| format!("CSPRNG failed: {e}"))?;
    let key_id = uuid::Uuid::new_v4();
    Ok(GeneratedKey {
        info: ContentKey {
            key_type,
            key_id: key_id.to_string(),
            asset_uuid: asset_uuid.to_string(),
            content_key_hex: hex_encode(&key),
        },
        key,
        key_id: *key_id.as_bytes(),
    })
}

fn hex_encode(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_decode_16(s: &str) -> Result<[u8; 16], String> {
    if s.len() != 32 {
        return Err(format!("content key must be 32 hex chars, got {}", s.len()));
    }
    let mut out = [0u8; 16];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
            .map_err(|e| format!("content key is not hex: {e}"))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_keys_are_random_and_roundtrip() {
        let a = generate_content_key(KeyType::Mdik, "asset-a").unwrap();
        let b = generate_content_key(KeyType::Mdik, "asset-a").unwrap();
        assert_ne!(a.key, b.key, "CSPRNG must not repeat keys");
        assert_ne!(a.key_id, b.key_id);

        let (kt, kid, key) = a.info.to_raw().unwrap();
        assert_eq!(kt, *b"MDIK");
        assert_eq!(key, a.key);
        assert_eq!(*kid.as_bytes(), a.key_id);
    }

    #[test]
    fn every_essence_has_its_st_430_1_key_type() {
        assert_eq!(KeyType::Mdik.as_smpte(), *b"MDIK");
        assert_eq!(KeyType::Mdak.as_smpte(), *b"MDAK");
        assert_eq!(KeyType::Mdsk.as_smpte(), *b"MDSK");
        assert_eq!(KeyType::Mdek.as_smpte(), *b"MDEK");
    }

    #[test]
    fn key_bundle_file_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(KEYS_FILE_NAME);
        let bundle = KeyBundle {
            cpl_id: "cpl-1".into(),
            keys: vec![
                generate_content_key(KeyType::Mdik, "pic").unwrap().info,
                generate_content_key(KeyType::Mdak, "snd").unwrap().info,
            ],
        };
        bundle.write(&path).unwrap();
        let back = KeyBundle::read(&path).unwrap();
        assert_eq!(back.cpl_id, "cpl-1");
        assert_eq!(back.keys.len(), 2);
        assert_eq!(back.keys[1].key_type, KeyType::Mdak);
    }
}
