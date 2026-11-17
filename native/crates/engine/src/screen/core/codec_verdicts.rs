//! Verdicts of the run-time codec check (plan 2026-11-17
//! „Windows-Bild“ P3, TODO `VERB-WIN-KODIERER`): an encoder or decoder
//! counts as checked once a round trip of the test picture succeeded
//! (`codec_verify.rs`). A verdict is kept per process in memory and, for
//! the codec register of a program, in `<app data>/screen/codec_verdicts.json`
//! (`codec_inventory`), keyed by program version, factory, adapter (PCI
//! vendor, device, revision and user-mode driver version where the
//! platform reads them), codec, profile, size class and role, so a new
//! version or driver checks again.
//!
//! Before a check its key is written as running (`begin`). A process that
//! crashes inside the check leaves that mark: a mark older than
//! `CRASH_AFTER_SECS` counts as one failed attempt, and the second such
//! attempt is `Failed(Crashed)` (decision of this batch: a host process
//! killed while it checked, e.g. by a closed session, is not taken for a
//! driver crash at once; see the report).
//!
//! Nothing here reads the platform: the file path and the clock come from
//! the caller, the tests use `VerdictStore::memory` and their own clock.
//! Which codecs keep their rank before H.264 (D6) follows from the
//! summaries of a register (`CodecRegistry::codec_trust`), not from state
//! of this module.
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::negotiate::{CodecKind, H264Profile};
use super::pixel_convert::FrameSize;

/// The program version every key carries.
pub const PROGRAM_VERSION: &str = env!("CARGO_PKG_VERSION");
/// A running mark older than this was left by a process that ended inside
/// the check (the check itself has a hard limit of 3 s).
pub const CRASH_AFTER_SECS: u64 = 60;
/// Attempts that ended inside the check before the verdict is `Crashed`.
pub const CRASH_ATTEMPTS: u32 = 2;
/// Most entries the file keeps (the oldest go first).
const MAX_ENTRIES: usize = 512;
/// Largest verdict file read.
const MAX_FILE_BYTES: u64 = 1 << 20;

/// Encoder or decoder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Encoder,
    Decoder,
}

/// Why a check failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailReason {
    /// A decoded picture does not carry its barcode, or the stream does not
    /// decode.
    Garbled,
    /// Encoding took longer than the limit (p95).
    Slow,
    /// A picture came out of a later call than its own.
    Late,
    /// A requested key frame is none.
    MissingKeyframe,
    /// The key-frame flag contradicts the stream.
    KeyframeMismatch,
    /// The first key frame lacks its parameter sets.
    MissingParameterSets,
    /// One call gave more than one access unit.
    NotOneUnit,
    /// The encoder refused a picture.
    EncodeFailed,
    /// The round trip did not end within its hard limit.
    Deadline,
    /// The process ended inside the check (`CRASH_ATTEMPTS` times).
    Crashed,
}

impl FailReason {
    pub fn key(self) -> &'static str {
        match self {
            Self::Garbled => "garbled",
            Self::Slow => "slow",
            Self::Late => "late",
            Self::MissingKeyframe => "missing_keyframe",
            Self::KeyframeMismatch => "keyframe_mismatch",
            Self::MissingParameterSets => "missing_parameter_sets",
            Self::NotOneUnit => "not_one_unit",
            Self::EncodeFailed => "encode_failed",
            Self::Deadline => "deadline",
            Self::Crashed => "crashed",
        }
    }
}

/// The result of one check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Round trip with a local decoder: every picture intact, in time.
    Verified,
    /// No local reference decoder: stream structure and time checked.
    Structural,
    Failed(FailReason),
}

impl Verdict {
    /// Verified or Structural.
    pub fn passes(self) -> bool {
        !matches!(self, Self::Failed(_))
    }

    pub fn state_key(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Structural => "structural",
            Self::Failed(_) => "failed",
        }
    }

    pub fn reason(self) -> Option<FailReason> {
        match self {
            Self::Failed(reason) => Some(reason),
            _ => None,
        }
    }
}

/// The class of a picture size in a key: a check at 1280 × 720 says
/// little about 3840 × 2160.
pub fn size_class(size: FrameSize) -> &'static str {
    match size.pixels() {
        pixels if pixels <= 1280 * 720 => "720p",
        pixels if pixels <= 1920 * 1080 => "1080p",
        pixels if pixels <= 2560 * 1600 => "1600p",
        _ => "2160p",
    }
}

/// What a verdict belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct VerdictKey {
    pub version: String,
    pub factory: String,
    /// `vendor:device:revision@driver` of the adapter; empty for software.
    pub device: String,
    pub codec: String,
    /// The standard's profile name (`high`, `cb`, `main`, …); empty for a
    /// decoder (one verdict per codec).
    pub profile: String,
    pub size_class: String,
    pub role: Role,
}

impl VerdictKey {
    pub fn new(
        factory: &str,
        device: &str,
        codec: CodecKind,
        profile: Option<H264Profile>,
        size: FrameSize,
        role: Role,
    ) -> Self {
        let profile = match role {
            Role::Encoder => codec.profile_name(profile).to_string(),
            Role::Decoder => String::new(),
        };
        Self {
            version: PROGRAM_VERSION.into(),
            factory: factory.into(),
            device: device.into(),
            codec: codec.key().into(),
            profile,
            size_class: size_class(size).into(),
            role,
        }
    }

    /// Same version, factory, adapter, codec and role (any profile and
    /// size class when `profile` is `None`).
    fn matches(&self, other: &Self, profile: Option<&str>) -> bool {
        self.version == other.version
            && self.factory == other.factory
            && self.device == other.device
            && self.codec == other.codec
            && self.role == other.role
            && profile.is_none_or(|profile| self.profile == profile)
    }
}

/// One entry of the store and of the file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    key: VerdictKey,
    /// `verified`, `structural`, `failed` or `running`.
    state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<FailReason>,
    /// Seconds since 1970 of the verdict or of the running mark.
    at: u64,
    /// Running marks that ended without a verdict, this one included.
    #[serde(default)]
    attempts: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    detail: String,
}

#[derive(Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    entries: Vec<Entry>,
}

/// What a store holds for a factory and codec over all size classes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub verified: bool,
    pub structural: bool,
    pub failed: Option<FailReason>,
}

impl Summary {
    pub fn passes(&self) -> bool {
        self.verified || self.structural
    }

    pub fn checked(&self) -> bool {
        self.passes() || self.failed.is_some()
    }
}

/// `se screen codec --json`: the verdict of one row (`CodecEntry`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerdictView {
    /// `verified`, `structural`, `failed` or `unchecked`.
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl VerdictView {
    pub fn of(summary: Summary) -> Self {
        let (state, reason) = if summary.verified {
            ("verified", None)
        } else if summary.structural {
            ("structural", None)
        } else if let Some(reason) = summary.failed {
            ("failed", Some(reason.key().to_string()))
        } else {
            ("unchecked", None)
        };
        Self {
            state: state.into(),
            reason,
        }
    }
}

type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// The verdicts of this process, optionally backed by a file.
pub struct VerdictStore {
    path: Option<PathBuf>,
    clock: Clock,
    entries: Mutex<Vec<Entry>>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

impl VerdictStore {
    /// Verdicts of this process only (task tests, platforms without a file).
    pub fn memory() -> Self {
        Self {
            path: None,
            clock: Arc::new(now_secs),
            entries: Mutex::new(Vec::new()),
        }
    }

    /// Verdicts kept in `path` as well.
    pub fn at(path: PathBuf) -> Self {
        let store = Self {
            path: Some(path),
            ..Self::memory()
        };
        let loaded = store.read_file();
        *store.lock() = loaded;
        store
    }

    /// The same store with another clock (seconds since 1970; task tests).
    pub fn with_clock(mut self, clock: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn read_file(&self) -> Vec<Entry> {
        let Some(path) = &self.path else {
            return Vec::new();
        };
        crate::support_dirs::read_private_text(path, MAX_FILE_BYTES)
            .ok()
            .and_then(|text| serde_json::from_str::<File>(&text).ok())
            .map(|file| file.entries)
            .unwrap_or_default()
    }

    /// Runs `change` on the newest entries (the file of other processes
    /// first) and writes them back.
    fn update(&self, change: impl FnOnce(&mut Vec<Entry>)) {
        let mut entries = self.lock();
        if self.path.is_some() {
            *entries = self.read_file();
        }
        change(&mut entries);
        if entries.len() > MAX_ENTRIES {
            entries.sort_by_key(|entry| entry.at);
            let excess = entries.len() - MAX_ENTRIES;
            entries.drain(..excess);
        }
        if let Err(error) = self.write_file(&entries) {
            eprintln!("screen codec verdicts: {error}");
        }
    }

    fn write_file(&self, entries: &[Entry]) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = File {
            entries: entries.to_vec(),
        };
        let bytes = serde_json::to_vec_pretty(&file).map_err(io::Error::other)?;
        crate::support_dirs::write_private_atomic(path, &bytes)
    }

    /// A stale running mark: its attempts, else `None`.
    fn stale(&self, entry: &Entry) -> Option<u32> {
        let old = (self.clock)().saturating_sub(entry.at) > CRASH_AFTER_SECS;
        (entry.state == "running" && old).then_some(entry.attempts.max(1))
    }

    fn verdict_of(&self, entry: &Entry) -> Option<Verdict> {
        match entry.state.as_str() {
            "verified" => Some(Verdict::Verified),
            "structural" => Some(Verdict::Structural),
            "failed" => Some(Verdict::Failed(entry.reason.unwrap_or(FailReason::Crashed))),
            "running" => self
                .stale(entry)
                .filter(|attempts| *attempts >= CRASH_ATTEMPTS)
                .map(|_| Verdict::Failed(FailReason::Crashed)),
            _ => None,
        }
    }

    /// The verdict of `key`; `None` while it is unchecked (or a check runs).
    pub fn get(&self, key: &VerdictKey) -> Option<Verdict> {
        let mut entries = self.lock();
        if self.path.is_some() {
            *entries = self.read_file();
        }
        let entry = entries.iter().find(|entry| entry.key == *key)?;
        self.verdict_of(entry)
    }

    /// Marks `key` as running before its check; `false` when the attempts
    /// that ended inside the check make it `Crashed` (then it is recorded).
    pub fn begin(&self, key: &VerdictKey) -> bool {
        let now = (self.clock)();
        let mut checks = true;
        self.update(|entries| {
            let previous = entries.iter().position(|entry| entry.key == *key);
            let attempts = previous
                .and_then(|index| self.stale(&entries[index]))
                .map_or(1, |attempts| attempts + 1);
            if let Some(index) = previous {
                entries.remove(index);
            }
            let crashed = attempts > CRASH_ATTEMPTS;
            checks = !crashed;
            entries.push(Entry {
                key: key.clone(),
                state: if crashed { "failed" } else { "running" }.into(),
                reason: crashed.then_some(FailReason::Crashed),
                at: now,
                attempts: attempts.min(CRASH_ATTEMPTS),
                detail: String::new(),
            });
        });
        checks
    }

    /// The check of `key` ended without a verdict (e.g. it could not open).
    pub fn cancel(&self, key: &VerdictKey) {
        self.update(|entries| entries.retain(|entry| entry.key != *key));
    }

    /// Records `verdict` for `key`.
    pub fn record(&self, key: &VerdictKey, verdict: Verdict, detail: &str) {
        let now = (self.clock)();
        self.update(|entries| {
            entries.retain(|entry| entry.key != *key);
            entries.push(Entry {
                key: key.clone(),
                state: verdict.state_key().into(),
                reason: verdict.reason(),
                at: now,
                attempts: 0,
                detail: detail.chars().take(240).collect(),
            });
        });
    }

    /// Forgets every verdict of this version (`se screen codec verify`
    /// checks again).
    pub fn forget(&self) {
        self.update(|entries| entries.retain(|entry| entry.key.version != PROGRAM_VERSION));
    }

    /// The verdicts of `factory` on `device` for `codec` and `role` over
    /// all size classes (and every profile when `profile` is `None`).
    pub fn summary(
        &self,
        factory: &str,
        device: &str,
        codec: CodecKind,
        profile: Option<&str>,
        role: Role,
    ) -> Summary {
        let probe = VerdictKey {
            version: PROGRAM_VERSION.into(),
            factory: factory.into(),
            device: device.into(),
            codec: codec.key().into(),
            profile: String::new(),
            size_class: String::new(),
            role,
        };
        let mut entries = self.lock();
        if self.path.is_some() {
            *entries = self.read_file();
        }
        let mut summary = Summary::default();
        for entry in entries
            .iter()
            .filter(|entry| entry.key.matches(&probe, profile))
        {
            match self.verdict_of(entry) {
                Some(Verdict::Verified) => summary.verified = true,
                Some(Verdict::Structural) => summary.structural = true,
                Some(Verdict::Failed(reason)) => summary.failed = Some(reason),
                None => {}
            }
        }
        summary
    }
}
