//! Owner-protected bounded history. Hashed pinned pairs, no addresses or secrets.
//!
//! The history lives in its own folder below the app data directory, which
//! the embedded worker's input fingerprint leaves out
//! (`daemon/os/shared/ipc_host_service.rs`, `NOT_INPUTS`): a diagnostic
//! write is no Share input and reloads nothing (A10 of plan
//! 2026-11-06-akku-rechte). Samples reach the in-memory history at once;
//! the file follows at most once per `WRITE_EVERY`.
use super::route_diagnostics::RouteMeasurement;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

const MAX_PAIRS: usize = 64;
const MAX_SAMPLES: usize = 32;
const MAX_BYTES: u64 = 512 * 1024;
const RETENTION_MS: u64 = 7 * 24 * 60 * 60 * 1000;
/// Folder below the app data directory; the same name stands in
/// `ipc_host_service.rs` (`NOT_INPUTS`).
const DIRECTORY: &str = "share-routes";
const FILE: &str = "route-history.json";
/// Where earlier versions kept it, directly in the app data directory; read
/// once when the new file is missing, removed after the first new write.
const LEGACY_FILE: &str = "share-route-history.json";
/// Shortest distance between two writes of the file. The route monitor
/// records a pair at most every 2 s while data flows, and a pair keeps 32
/// samples, about one minute of traffic: a crash loses at most that
/// window, while a busy connection writes the whole file 30 times less.
const WRITE_EVERY: Duration = Duration::from_secs(60);

#[derive(Default, Serialize, Deserialize)]
struct History {
    pairs: HashMap<String, VecDeque<RouteMeasurement>>,
}
static HISTORY: OnceLock<Mutex<History>> = OnceLock::new();

/// When the file was last written and whether a delayed write is waiting.
struct Writes {
    last: Option<Instant>,
    waiting: bool,
}
static WRITES: Mutex<Writes> = Mutex::new(Writes {
    last: None,
    waiting: false,
});
/// Serializes taking the snapshot and replacing the file, so a write never
/// replaces a newer snapshot with an older one.
static WRITING: Mutex<()> = Mutex::new(());

fn path() -> PathBuf {
    crate::support_dirs::app_data_dir()
        .join(DIRECTORY)
        .join(FILE)
}

pub(super) fn schedule_record(pair: String, sample: RouteMeasurement) {
    static WRITERS: OnceLock<std::sync::Arc<tokio::sync::Semaphore>> = OnceLock::new();
    let slots = WRITERS.get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(4)));
    let Ok(permit) = slots.clone().try_acquire_owned() else {
        return;
    };
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        record(&pair, sample);
    });
}

fn history() -> &'static Mutex<History> {
    HISTORY.get_or_init(|| {
        let read = |path: &std::path::Path| {
            crate::support_dirs::read_private_text(path, MAX_BYTES)
                .ok()
                .and_then(|text| serde_json::from_str::<History>(&text).ok())
        };
        let mut loaded = read(&path())
            .or_else(|| read(&crate::support_dirs::app_data_file(LEGACY_FILE)))
            .unwrap_or_default();
        for samples in loaded.pairs.values_mut() {
            while samples.len() > MAX_SAMPLES {
                samples.pop_front();
            }
        }
        while loaded.pairs.len() > MAX_PAIRS {
            remove_oldest(&mut loaded);
        }
        Mutex::new(loaded)
    })
}

fn remove_oldest(history: &mut History) {
    if let Some(oldest) = history
        .pairs
        .iter()
        .min_by_key(|(_, samples)| {
            samples
                .back()
                .map_or(0, |sample| sample.observed_at_unix_ms)
        })
        .map(|(pair, _)| pair.clone())
    {
        history.pairs.remove(&oldest);
    }
}

pub(super) fn pair_key(local_pin: &str, remote_pin: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"smart-explorer/route-history/v1\0");
    let mut pins = [local_pin, remote_pin];
    pins.sort();
    for pin in pins {
        hash.update((pin.len() as u64).to_be_bytes());
        hash.update(pin.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

pub(super) fn record(pair: &str, sample: RouteMeasurement) {
    let Ok(mut history) = history().lock() else {
        return;
    };
    let oldest = sample.observed_at_unix_ms.saturating_sub(RETENTION_MS);
    history.pairs.retain(|_, samples| {
        samples.retain(|sample| sample.observed_at_unix_ms >= oldest);
        while samples.len() > MAX_SAMPLES {
            samples.pop_front();
        }
        !samples.is_empty()
    });
    if !history.pairs.contains_key(pair) && history.pairs.len() >= MAX_PAIRS {
        remove_oldest(&mut history);
    }
    let samples = history.pairs.entry(pair.into()).or_default();
    while samples.len() >= MAX_SAMPLES {
        samples.pop_front();
    }
    samples.push_back(sample);
    drop(history);
    write_soon();
}

/// Writes now when the last write is `WRITE_EVERY` ago, otherwise once
/// when that time has passed; samples arriving meanwhile join that write.
fn write_soon() {
    let mut writes = WRITES.lock().unwrap_or_else(PoisonError::into_inner);
    if writes.waiting {
        return;
    }
    let wait = writes.last.map_or(Duration::ZERO, |last| {
        WRITE_EVERY.saturating_sub(last.elapsed())
    });
    if wait.is_zero() {
        writes.last = Some(Instant::now());
        drop(writes);
        write_now();
        return;
    }
    writes.waiting = true;
    drop(writes);
    let delayed = std::thread::Builder::new()
        .name("share-route-history".into())
        .spawn(move || {
            std::thread::sleep(wait);
            let mut writes = WRITES.lock().unwrap_or_else(PoisonError::into_inner);
            writes.waiting = false;
            writes.last = Some(Instant::now());
            drop(writes);
            write_now();
        });
    if delayed.is_err() {
        // No thread for the delay: write at once rather than lose samples.
        let mut writes = WRITES.lock().unwrap_or_else(PoisonError::into_inner);
        writes.waiting = false;
        writes.last = Some(Instant::now());
        drop(writes);
        write_now();
    }
}

fn write_now() {
    let _writing = WRITING.lock().unwrap_or_else(PoisonError::into_inner);
    let bytes = {
        let Ok(history) = history().lock() else {
            return;
        };
        match serde_json::to_vec(&*history) {
            Ok(bytes) => bytes,
            Err(_) => return,
        }
    };
    if bytes.len() as u64 > MAX_BYTES {
        return;
    }
    // A failed diagnostic write never changes route/identity authorization.
    if crate::support_dirs::write_private_atomic(&path(), &bytes).is_ok() {
        // The old file lies in the fingerprinted directory; removing it
        // costs one reload, once.
        let legacy = crate::support_dirs::app_data_file(LEGACY_FILE);
        if legacy.exists() {
            let _ = std::fs::remove_file(legacy);
        }
    }
}

pub(super) fn samples(pair: &str) -> Vec<RouteMeasurement> {
    history()
        .lock()
        .ok()
        .and_then(|history| history.pairs.get(pair).cloned())
        .map(|samples| samples.into_iter().collect())
        .unwrap_or_default()
}
