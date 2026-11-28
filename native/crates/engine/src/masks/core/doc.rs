//! The store document (lists, entries, scopes, assigned placeholders) and
//! its sealing: `lists.sealed` is magic ‖ key id ‖ nonce ‖ ChaCha20-Poly1305
//! of the JSON document, with magic and key id as associated data. The key is
//! 32 random bytes in the se secret store. `state.json` beside it holds no
//! secret: generations and the lockdown flag for the agent hook.
use std::collections::BTreeMap;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, KeyInit, Nonce};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::model::{EntryId, EntryKind, ListId, Tolerance};
use super::scope::Scope;

pub(crate) const KEY_BYTES: usize = 32;
const MAGIC: &[u8; 8] = b"SEMASK1\0";
const KEY_ID_BYTES: usize = 16;
const NONCE_BYTES: usize = 12;
const HEADER_BYTES: usize = MAGIC.len() + KEY_ID_BYTES + NONCE_BYTES;
const KEY_ID_DOMAIN: &[u8] = b"smart-explorer/masks/key-id/v1\0";
/// Largest sealed store read back (a planted giant file is refused).
pub(crate) const MAX_SEALED_BYTES: u64 = 256 * 1024 * 1024;
/// Largest `state.json` read back.
pub(crate) const MAX_STATE_BYTES: u64 = 64 * 1024;
const FORMAT: u32 = 1;

pub(crate) type StoreKey = Zeroizing<[u8; KEY_BYTES]>;

/// Bytes stored as Base64 (a regex on bytes may match non-UTF-8 data).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct B64(pub Vec<u8>);

impl Serialize for B64 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for B64 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        STANDARD
            .decode(text.as_bytes())
            .map(B64)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct StoreDoc {
    pub format: u32,
    /// Raised by every write (lists and assignments).
    pub generation: u64,
    /// Raised by every change of lists, entries, scopes and flags; also kept
    /// in the secret store, so an older copy of the file is refused.
    pub config_generation: u64,
    pub next_list: u64,
    pub next_entry: u64,
    /// Last placeholder number per label over all lists, never reused.
    pub counters: BTreeMap<String, u64>,
    pub lists: Vec<ListDoc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct ListDoc {
    pub id: ListId,
    pub name: String,
    pub enabled: bool,
    pub locked: bool,
    pub scopes: Vec<Scope>,
    pub entries: Vec<EntryDoc>,
    pub assigned: Vec<AssignedDoc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct EntryDoc {
    pub id: EntryId,
    pub kind: EntryKind,
    pub value: String,
    pub label: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub tolerance: Tolerance,
}

/// One real value with its placeholder number; `variants[k]` is shown as
/// `⟦label-number.(k+1)⟧`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct AssignedDoc {
    pub label: String,
    pub number: u64,
    pub entry: EntryId,
    pub value: B64,
    pub variants: Vec<B64>,
}

/// `state.json`: what the agent hook reads without the key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StateDoc {
    pub format: u32,
    pub generation: u64,
    pub config_generation: u64,
    pub lockdown: bool,
}

impl StoreDoc {
    pub(crate) fn new() -> Self {
        Self {
            format: FORMAT,
            next_list: 1,
            next_entry: 1,
            ..Self::default()
        }
    }

    /// An enabled list has a protected path (plan S1: agent lockdown).
    pub(crate) fn lockdown(&self) -> bool {
        self.lists
            .iter()
            .any(|list| list.enabled && !list.scopes.is_empty())
    }

    pub(crate) fn state(&self) -> StateDoc {
        StateDoc {
            format: FORMAT,
            generation: self.generation,
            config_generation: self.config_generation,
            lockdown: self.lockdown(),
        }
    }

    pub(crate) fn list(&self, id: ListId) -> Option<&ListDoc> {
        self.lists.iter().find(|list| list.id == id)
    }

    pub(crate) fn list_mut(&mut self, id: ListId) -> Option<&mut ListDoc> {
        self.lists.iter_mut().find(|list| list.id == id)
    }
}

impl StateDoc {
    pub(crate) fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, String> {
        let state: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if state.format != FORMAT {
            return Err(format!("unbekanntes Format {}", state.format));
        }
        Ok(state)
    }
}

fn key_id(key: &[u8; KEY_BYTES]) -> [u8; KEY_ID_BYTES] {
    let digest = Sha256::new()
        .chain_update(KEY_ID_DOMAIN)
        .chain_update(key)
        .finalize();
    let mut id = [0u8; KEY_ID_BYTES];
    id.copy_from_slice(&digest[..KEY_ID_BYTES]);
    id
}

/// A new random store key.
pub(crate) fn generate_key() -> Result<StoreKey, String> {
    let mut key = Zeroizing::new([0u8; KEY_BYTES]);
    getrandom::getrandom(&mut key[..]).map_err(|e| format!("Schlüssel erzeugen: {e}"))?;
    Ok(key)
}

pub(crate) fn encode_key(key: &StoreKey) -> Zeroizing<String> {
    Zeroizing::new(STANDARD.encode(&key[..]))
}

pub(crate) fn decode_key(text: &str) -> Result<StoreKey, String> {
    let bytes = Zeroizing::new(
        STANDARD
            .decode(text.trim().as_bytes())
            .map_err(|_| "Schlüssel im Geheimnisspeicher beschädigt".to_string())?,
    );
    if bytes.len() != KEY_BYTES {
        return Err("Schlüssel im Geheimnisspeicher hat die falsche Länge".into());
    }
    let mut key = Zeroizing::new([0u8; KEY_BYTES]);
    key.copy_from_slice(&bytes);
    Ok(key)
}

/// Seals the document under `key`.
pub(crate) fn seal(doc: &StoreDoc, key: &StoreKey) -> Result<Vec<u8>, String> {
    let plain = Zeroizing::new(serde_json::to_vec(doc).map_err(|e| e.to_string())?);
    let mut nonce = [0u8; NONCE_BYTES];
    getrandom::getrandom(&mut nonce).map_err(|e| format!("Nonce erzeugen: {e}"))?;
    let mut out = Vec::with_capacity(HEADER_BYTES + plain.len() + 16);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&key_id(key));
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key[..]));
    let sealed = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &plain,
                aad: &out[..MAGIC.len() + KEY_ID_BYTES],
            },
        )
        .map_err(|_| "Maskenspeicher versiegeln".to_string())?;
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&sealed);
    Ok(out)
}

/// Opens a sealed document; any change of the bytes fails authentication.
pub(crate) fn open(bytes: &[u8], key: &StoreKey) -> Result<StoreDoc, String> {
    if bytes.len() < HEADER_BYTES + 16 || !bytes.starts_with(MAGIC) {
        return Err("kein Maskenspeicher (Kopf fehlt)".into());
    }
    let id_end = MAGIC.len() + KEY_ID_BYTES;
    if bytes[MAGIC.len()..id_end] != key_id(key) {
        return Err("mit einem anderen Schlüssel versiegelt".into());
    }
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key[..]));
    let plain = cipher
        .decrypt(
            Nonce::from_slice(&bytes[id_end..HEADER_BYTES]),
            Payload {
                msg: &bytes[HEADER_BYTES..],
                aad: &bytes[..id_end],
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| "Echtheitsprüfung fehlgeschlagen".to_string())?;
    let doc: StoreDoc = serde_json::from_slice(&plain).map_err(|e| e.to_string())?;
    if doc.format != FORMAT {
        return Err(format!("unbekanntes Format {}", doc.format));
    }
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_task_sealed_store_hides_values_and_refuses_changes() {
        let key = generate_key().unwrap();
        let mut doc = StoreDoc::new();
        doc.lists.push(ListDoc {
            id: ListId(1),
            name: "Klienten".into(),
            enabled: true,
            locked: false,
            scopes: vec![],
            entries: vec![EntryDoc {
                id: EntryId(1),
                kind: EntryKind::Literal,
                value: "Mustermann".into(),
                label: "Name".into(),
                case_sensitive: false,
                whole_word: false,
                tolerance: Tolerance::Off,
            }],
            assigned: vec![],
        });
        let sealed = seal(&doc, &key).unwrap();
        assert!(!sealed.windows(10).any(|w| w == b"Mustermann"));
        assert!(!sealed.windows(8).any(|w| w == b"Klienten"));
        assert_eq!(open(&sealed, &key).unwrap(), doc);
        let mut flipped = sealed.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 1;
        assert!(open(&flipped, &key).is_err());
        let other = generate_key().unwrap();
        assert!(open(&sealed, &other).is_err());
    }
}
