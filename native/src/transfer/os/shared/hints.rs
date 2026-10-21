//! What the flows learned, kept across restarts: the limit each connection
//! (or local volume) settled at, so the next run starts at half of it instead
//! of at two (`<app data>/transfer-hints.json`). A hint is only a starting
//! point; the first measurement replaces it.
//!
//! The file never holds a host, an account, a path or a name. A key is the
//! SHA-256 of a random 32-byte salt of this installation followed by the
//! connection's identity text (without the salt, equal keys would link users
//! of the same server across installations and could be computed in advance).
//! Reading drops everything that is not valid: unknown entries, limits outside
//! 1..=ceiling of the flow, times in the future. The file is private
//! (`0600` in a `0700` folder; the profile's ACL on Windows) and written
//! atomically. Every failure is ignored: the hint is a convenience.
use super::flow_control::RESOURCE_CEILING;
use super::flow_window::start_from_learned;
use ring::digest;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// File name below the app data folder.
#[cfg_attr(test, allow(dead_code))]
const FILE_NAME: &str = "transfer-hints.json";
/// Entries kept (about 100 KiB), the oldest dropped first: the file is
/// rewritten whole, so its size bounds the cost of a write.
const MAX_ENTRIES: usize = 512;
/// The file is written at most this often while limits change.
const WRITE_INTERVAL_MS: u64 = 60_000;
/// Larger files are not read (512 entries need about 60 KiB).
const MAX_FILE_BYTES: u64 = 256 * 1024;
const VERSION: u32 = 1;
const SALT_BYTES: usize = 32;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Entry {
    key: String,
    limit: usize,
    saved_ms: u64,
}

#[derive(Serialize, Deserialize)]
struct FileForm {
    version: u32,
    salt: String,
    flows: Vec<Entry>,
}

/// The learned limits of one installation.
pub(super) struct Hints {
    path: Option<PathBuf>,
    loaded: bool,
    salt: Option<[u8; SALT_BYTES]>,
    entries: Vec<Entry>,
    dirty: bool,
    last_write_ms: Option<u64>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Option<[u8; SALT_BYTES]> {
    if text.len() != SALT_BYTES * 2 || !text.is_ascii() {
        return None;
    }
    let mut bytes = [0u8; SALT_BYTES];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}

fn is_key(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Hints {
    /// Hints stored in `path`; nothing is read until the first use.
    pub(super) fn at(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            loaded: false,
            salt: None,
            entries: Vec::new(),
            dirty: false,
            last_write_ms: None,
        }
    }

    /// Hints that neither read nor write (tests of other parts).
    #[cfg(test)]
    pub(super) fn disabled() -> Self {
        Self {
            path: None,
            loaded: true,
            salt: None,
            entries: Vec::new(),
            dirty: false,
            last_write_ms: None,
        }
    }

    /// `SHA-256(salt ‖ identity)` in hex; `None` without a salt (the random
    /// source failed: no hint is kept rather than a predictable key).
    pub(super) fn identity_hash(&mut self, identity: &str, now_ms: u64) -> Option<String> {
        self.load(now_ms);
        let salt = self.salt?;
        let mut context = digest::Context::new(&digest::SHA256);
        context.update(&salt);
        context.update(identity.as_bytes());
        Some(hex(context.finish().as_ref()))
    }

    fn load(&mut self, now_ms: u64) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let parsed = self
            .path
            .as_deref()
            .and_then(|path| crate::support_dirs::read_private_text(path, MAX_FILE_BYTES).ok())
            .and_then(|text| serde_json::from_str::<FileForm>(&text).ok())
            .filter(|form| form.version == VERSION);
        if let Some((salt, form)) = parsed.and_then(|form| Some((unhex(&form.salt)?, form))) {
            self.salt = Some(salt);
            self.entries = form
                .flows
                .into_iter()
                .filter(|entry| {
                    is_key(&entry.key)
                        && (1..=RESOURCE_CEILING).contains(&entry.limit)
                        && entry.saved_ms <= now_ms
                })
                .collect();
            // Newest first: a key seen twice keeps its newest entry, and a
            // file with too many entries loses its oldest.
            self.entries
                .sort_by_key(|entry| std::cmp::Reverse(entry.saved_ms));
            let mut seen = std::collections::HashSet::new();
            self.entries.retain(|entry| seen.insert(entry.key.clone()));
            self.entries.truncate(MAX_ENTRIES);
            return;
        }
        // No usable file: a new salt, no entries (the old keys are void).
        let mut salt = [0u8; SALT_BYTES];
        self.salt = getrandom::getrandom(&mut salt).is_ok().then_some(salt);
        self.entries.clear();
    }

    /// The limit a new flow of `identity` starts at: half of the one it
    /// learned last time.
    pub(super) fn start_limit(&mut self, identity: &str, now_ms: u64) -> Option<usize> {
        let key = self.identity_hash(identity, now_ms)?;
        let entry = self.entries.iter().find(|entry| entry.key == key)?;
        Some(start_from_learned(entry.limit))
    }

    /// Remembers the limit `identity` settled at; writes the file when the
    /// last write is a minute old.
    pub(super) fn record(&mut self, identity: &str, limit: usize, now_ms: u64) {
        let Some(key) = self.identity_hash(identity, now_ms) else {
            return;
        };
        let limit = limit.clamp(1, RESOURCE_CEILING);
        match self.entries.iter_mut().find(|entry| entry.key == key) {
            Some(entry) if entry.limit == limit => return,
            Some(entry) => {
                entry.limit = limit;
                entry.saved_ms = now_ms;
            }
            None => {
                self.entries.push(Entry {
                    key,
                    limit,
                    saved_ms: now_ms,
                });
                if self.entries.len() > MAX_ENTRIES {
                    let oldest = (0..self.entries.len())
                        .min_by_key(|index| self.entries[*index].saved_ms)
                        .unwrap_or(0);
                    self.entries.remove(oldest);
                }
            }
        }
        self.dirty = true;
        let due = self
            .last_write_ms
            .is_none_or(|at| now_ms.saturating_sub(at) >= WRITE_INTERVAL_MS);
        if due {
            self.write(now_ms);
        }
    }

    /// Writes what changed since the last write.
    pub(super) fn flush(&mut self, now_ms: u64) {
        if self.dirty {
            self.write(now_ms);
        }
    }

    fn write(&mut self, now_ms: u64) {
        self.dirty = false;
        self.last_write_ms = Some(now_ms);
        let (Some(path), Some(salt)) = (&self.path, self.salt) else {
            return;
        };
        let form = FileForm {
            version: VERSION,
            salt: hex(&salt),
            flows: self.entries.clone(),
        };
        if let Ok(bytes) = serde_json::to_vec(&form) {
            // A failure is ignored: the hint is a convenience.
            let _ = crate::support_dirs::write_private_atomic(path, &bytes);
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

fn global() -> std::sync::MutexGuard<'static, Hints> {
    static HINTS: OnceLock<Mutex<Hints>> = OnceLock::new();
    HINTS
        .get_or_init(|| {
            // Tests of other parts never touch the user's file.
            #[cfg(test)]
            let hints = Hints::disabled();
            #[cfg(not(test))]
            let hints = Hints::at(crate::support_dirs::app_data_file(FILE_NAME));
            Mutex::new(hints)
        })
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The limit a flow of `identity` starts at, if one was learned.
pub(crate) fn start_limit(identity: &str) -> Option<usize> {
    global().start_limit(identity, now_ms())
}

/// Remembers the limit a flow of `identity` has learned.
pub(crate) fn record(identity: &str, limit: usize) {
    global().record(identity, limit, now_ms());
}

/// Writes pending changes (a job ended, the process ends).
pub(crate) fn flush() {
    global().flush(now_ms());
}

#[cfg(test)]
#[path = "hints_tests.rs"]
mod tests;
