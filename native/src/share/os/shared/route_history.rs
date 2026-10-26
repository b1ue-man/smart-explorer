//! Owner-protected bounded history. Hashed pinned pairs, no addresses or secrets.
use super::route_diagnostics::RouteMeasurement;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

const MAX_PAIRS: usize = 64;
const MAX_SAMPLES: usize = 32;
const MAX_BYTES: u64 = 512 * 1024;
const RETENTION_MS: u64 = 7 * 24 * 60 * 60 * 1000;

#[derive(Default, Serialize, Deserialize)]
struct History {
    pairs: HashMap<String, VecDeque<RouteMeasurement>>,
}
static HISTORY: OnceLock<Mutex<History>> = OnceLock::new();

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
        let path = crate::support_dirs::app_data_file("share-route-history.json");
        let mut loaded = crate::support_dirs::read_private_text(&path, MAX_BYTES)
            .ok()
            .and_then(|text| serde_json::from_str::<History>(&text).ok())
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
    if let Ok(bytes) = serde_json::to_vec(&*history) {
        if bytes.len() as u64 <= MAX_BYTES {
            let path = crate::support_dirs::app_data_file("share-route-history.json");
            // A failed diagnostic write never changes route/identity authorization.
            let _ = crate::support_dirs::write_private_atomic(&path, &bytes);
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
