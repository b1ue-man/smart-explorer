use super::Source;
use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadMode {
    Waiting,
    Exclusive,
    Sequential,
    MetadataFallback,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Status {
    pub source: Source,
    pub active: bool,
    pub foreground_waiters: usize,
    pub background_waiters: usize,
    pub mode: ReadMode,
    pub block_bytes: Option<usize>,
}

fn registry() -> &'static Mutex<BTreeMap<String, Status>> {
    static REGISTRY: OnceLock<Mutex<BTreeMap<String, Status>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub fn snapshots() -> Vec<Status> {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .cloned()
        .collect()
}

pub(super) fn update(source: &Source, operation: impl FnOnce(&mut Status)) {
    let mut registry = registry().lock().unwrap_or_else(|e| e.into_inner());
    let status = registry
        .entry(source.device.clone())
        .or_insert_with(|| Status {
            source: source.clone(),
            active: false,
            foreground_waiters: 0,
            background_waiters: 0,
            mode: ReadMode::Waiting,
            block_bytes: None,
        });
    if !status.active {
        status.source = source.clone();
    }
    operation(status);
    if !status.active && status.foreground_waiters == 0 && status.background_waiters == 0 {
        registry.remove(&source.device);
    }
}
