//! Key bindings of the signaling server (FC4). A device id belongs to the
//! first key that proved it; a Direct lookup to the key that first published
//! it after a key login, together with the hash of its access proof. Keys
//! never change under a device id or lookup in the app (a new key comes with
//! a new device id and lookup), so a binding is permanent. Older clients keep
//! working on unbound entries but never take over bound ones.
//!
//! Bindings live in memory and, with `--state-file`, in a JSON file that a
//! restart reads back; without it the first proven key after a restart wins.
//!
//! Keys cost nothing, so one key binds at most [`MAX_DEVICES_PER_KEY`] device
//! ids and [`MAX_LOOKUPS_PER_KEY`] lookups, and new bindings per source are
//! rate limited by the caller (R-SERVER-02). An owner change is written at
//! once; a renewed access hash of an existing lookup at most once per
//! [`ACCESS_SAVE_INTERVAL_SECS`], because the owner sends its current hash
//! again with every publication after a restart (R-SERVER-03).

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Entries per kind; a full table refuses new bindings without forgetting owners.
pub(super) const MAX_BINDINGS: usize = 200_000;
/// Device ids one key may bind: the app keeps one id per key, and a lost
/// identity file with a kept keyring key or several profiles of one account
/// add a new id each (`native/crates/engine/src/share/core/identity.rs`).
pub(super) const MAX_DEVICES_PER_KEY: usize = 64;
/// Lookups one key may bind: one per Direct code renewal of a device;
/// 64 times the concurrent cap `MAX_PUBLISHED_DIRECTS_PER_CLIENT`.
pub(super) const MAX_LOOKUPS_PER_KEY: usize = 4_096;
/// Least interval between two saves caused only by renewed access hashes.
pub(super) const ACCESS_SAVE_INTERVAL_SECS: i64 = 60;
const MAX_STATE_BYTES: u64 = 256 * 1024 * 1024;
const STATE_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct Bindings {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    devices: HashMap<String, Binding>,
    #[serde(default)]
    lookups: HashMap<String, Binding>,
    /// An owner was added (written by the next save).
    #[serde(skip)]
    dirty: bool,
    /// Only access hashes changed (written at most every 60 s).
    #[serde(skip)]
    access_dirty: bool,
    /// Unix seconds of the last successful save of this process.
    #[serde(skip)]
    saved_at: Option<i64>,
    /// Bindings per key, rebuilt on load.
    #[serde(skip)]
    per_key: HashMap<String, KeyCounts>,
}

#[derive(Clone, Copy, Debug, Default)]
struct KeyCounts {
    devices: usize,
    lookups: usize,
}

#[derive(Clone, Copy)]
enum Kind {
    Device,
    Lookup,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Binding {
    /// Iroh public key as the app writes it (hex).
    key: String,
    /// SHA-256 (hex) of the owner's access proof (lookups only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    access: Option<String>,
    /// Unix seconds of the last use.
    seen: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BindOutcome {
    /// The key bound a new entry.
    Bound,
    /// The entry already belonged to this key.
    Confirmed,
    /// The entry belongs to another key.
    Conflict,
    /// The table holds [`MAX_BINDINGS`] entries.
    Full,
    /// The key already holds its maximum of entries of this kind.
    KeyFull,
}

impl Bindings {
    pub(super) fn device_key(&self, device_id: &str) -> Option<&str> {
        self.devices
            .get(device_id)
            .map(|binding| binding.key.as_str())
    }

    /// Whether binding `device_id` would add a new entry.
    pub(super) fn is_new_device(&self, device_id: &str) -> bool {
        !self.devices.contains_key(device_id)
    }

    pub(super) fn bind_device(&mut self, device_id: &str, key: &str, now: i64) -> BindOutcome {
        self.bind(Kind::Device, device_id, key, None, now)
    }

    /// Whether binding `lookup_id` would add a new entry.
    pub(super) fn is_new_lookup(&self, lookup_id: &str) -> bool {
        !self.lookups.contains_key(lookup_id)
    }

    pub(super) fn lookup_key(&self, lookup_id: &str) -> Option<&str> {
        self.lookups
            .get(lookup_id)
            .map(|binding| binding.key.as_str())
    }

    /// The access hash the owner left for a lookup.
    pub(super) fn lookup_access(&self, lookup_id: &str) -> Option<[u8; 32]> {
        self.lookups
            .get(lookup_id)
            .and_then(|binding| binding.access.as_deref())
            .and_then(super::access::parse_hash)
    }

    /// Binds a lookup to its publishing key; the owner may renew the hash.
    pub(super) fn bind_lookup(
        &mut self,
        lookup_id: &str,
        key: &str,
        access: Option<String>,
        now: i64,
    ) -> BindOutcome {
        self.bind(Kind::Lookup, lookup_id, key, access, now)
    }

    /// A copy to save when something changed since the last call.
    #[cfg(test)]
    pub(super) fn take_dirty(&mut self) -> Option<Self> {
        let owners = std::mem::take(&mut self.dirty);
        if !std::mem::take(&mut self.access_dirty) && !owners {
            return None;
        }
        let mut snapshot = self.clone();
        snapshot.version = STATE_VERSION;
        Some(snapshot)
    }

    /// Saves without a copy when an owner was added, or when only access
    /// hashes changed and the last save is [`ACCESS_SAVE_INTERVAL_SECS`] ago.
    /// A failed save keeps the pending changes for the next call.
    pub(super) fn save_pending(&mut self, path: &Path, now: i64) -> io::Result<()> {
        let access_due = self.access_dirty
            && self
                .saved_at
                .is_none_or(|at| now.saturating_sub(at) >= ACCESS_SAVE_INTERVAL_SECS);
        if !self.dirty && !access_due {
            return Ok(());
        }
        self.version = STATE_VERSION;
        self.save(path)?;
        self.dirty = false;
        self.access_dirty = false;
        self.saved_at = Some(now);
        Ok(())
    }

    fn bind(
        &mut self,
        kind: Kind,
        id: &str,
        key: &str,
        access: Option<String>,
        now: i64,
    ) -> BindOutcome {
        let (map, per_key_cap) = match kind {
            Kind::Device => (&mut self.devices, MAX_DEVICES_PER_KEY),
            Kind::Lookup => (&mut self.lookups, MAX_LOOKUPS_PER_KEY),
        };
        if let Some(binding) = map.get_mut(id) {
            if binding.key != key {
                return BindOutcome::Conflict;
            }
            if access.is_some() && binding.access != access {
                binding.access = access;
                self.access_dirty = true;
            }
            binding.seen = now;
            return BindOutcome::Confirmed;
        }
        if map.len() >= MAX_BINDINGS {
            return BindOutcome::Full;
        }
        let counts = self.per_key.get(key).copied().unwrap_or_default();
        if counts.of(kind) >= per_key_cap {
            return BindOutcome::KeyFull;
        }
        map.insert(
            id.to_string(),
            Binding {
                key: key.to_string(),
                access,
                seen: now,
            },
        );
        *self
            .per_key
            .entry(key.to_string())
            .or_default()
            .of_mut(kind) += 1;
        self.dirty = true;
        BindOutcome::Bound
    }

    fn rebuild_key_counts(&mut self) {
        let mut per_key: HashMap<String, KeyCounts> = HashMap::new();
        for (kind, map) in [(Kind::Device, &self.devices), (Kind::Lookup, &self.lookups)] {
            for binding in map.values() {
                *per_key.entry(binding.key.clone()).or_default().of_mut(kind) += 1;
            }
        }
        self.per_key = per_key;
    }

    /// Reads a state file; a missing file is an empty state.
    pub(super) fn load(path: &Path, now: i64) -> Result<Self, String> {
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
        };
        let mut text = String::new();
        file.take(MAX_STATE_BYTES + 1)
            .read_to_string(&mut text)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        if text.len() as u64 > MAX_STATE_BYTES {
            return Err("binding state file is too large".into());
        }
        let mut bindings: Self = serde_json::from_str(&text)
            .map_err(|error| format!("cannot parse {}: {error}", path.display()))?;
        if bindings.version > STATE_VERSION
            || bindings.devices.len() > MAX_BINDINGS
            || bindings.lookups.len() > MAX_BINDINGS
        {
            return Err("unsupported version or oversized binding state".into());
        }
        // Ownership never expires: ageing or pressure must not enable a takeover.
        // Entries above the per-key caps that an older server wrote stay bound;
        // they only block further new bindings of that key.
        let _ = now;
        bindings.dirty = false;
        bindings.access_dirty = false;
        bindings.rebuild_key_counts();
        Ok(bindings)
    }

    /// Writes the file atomically (temporary file, then rename).
    pub(super) fn save(&self, path: &Path) -> io::Result<()> {
        let text = serde_json::to_vec(self).map_err(io::Error::other)?;
        let nonce = super::login::nonce()
            .ok_or_else(|| io::Error::other("cannot create binding state nonce"))?;
        let temporary = path.with_extension(format!("{}.tmp", super::login::hex(&nonce)));
        let result = (|| {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            file.write_all(&text)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, path)?;
            #[cfg(unix)]
            {
                let parent = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."));
                std::fs::File::open(parent)?.sync_all()?;
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }
}

impl KeyCounts {
    fn of(self, kind: Kind) -> usize {
        match kind {
            Kind::Device => self.devices,
            Kind::Lookup => self.lookups,
        }
    }

    fn of_mut(&mut self, kind: Kind) -> &mut usize {
        match kind {
            Kind::Device => &mut self.devices,
            Kind::Lookup => &mut self.lookups,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BindOutcome, Bindings};

    #[test]
    fn review_task_first_proven_key_keeps_device_and_lookup() {
        let mut bindings = Bindings::default();
        assert_eq!(bindings.bind_device("d", "k1", 1), BindOutcome::Bound);
        assert_eq!(bindings.bind_device("d", "k1", 2), BindOutcome::Confirmed);
        assert_eq!(bindings.bind_device("d", "k2", 3), BindOutcome::Conflict);
        assert_eq!(bindings.device_key("d"), Some("k1"));
        let hash = "11".repeat(32);
        assert_eq!(
            bindings.bind_lookup("l", "k1", Some(hash.clone()), 4),
            BindOutcome::Bound
        );
        assert_eq!(bindings.lookup_access("l"), Some([0x11; 32]));
        assert_eq!(
            bindings.bind_lookup("l", "k2", None, 5),
            BindOutcome::Conflict
        );
        assert_eq!(bindings.lookup_key("l"), Some("k1"));
    }

    #[test]
    fn review_task_bindings_survive_a_restart_through_the_state_file() {
        let path = std::env::temp_dir().join(format!(
            "se-share-bindings-{}-{}.json",
            std::process::id(),
            line!()
        ));
        let mut bindings = Bindings::default();
        bindings.bind_device("device", "key", 100);
        bindings.bind_lookup("lookup", "key", Some("22".repeat(32)), 100);
        let snapshot = bindings.take_dirty().expect("changed");
        assert!(bindings.take_dirty().is_none());
        snapshot.save(&path).expect("save");
        let loaded = Bindings::load(&path, 200).expect("load");
        assert_eq!(loaded.device_key("device"), Some("key"));
        assert_eq!(loaded.lookup_key("lookup"), Some("key"));
        // Long inactivity must not give the binding to a stranger.
        let stale = Bindings::load(&path, 100 + 401 * 24 * 60 * 60).expect("load");
        assert_eq!(stale.device_key("device"), Some("key"));
        let _ = std::fs::remove_file(&path);
        assert!(Bindings::load(&path, 0)
            .expect("missing file")
            .device_key("device")
            .is_none());
    }
}

#[cfg(test)]
#[path = "bindings_tests.rs"]
mod bindings_tests;
