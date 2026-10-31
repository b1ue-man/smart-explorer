//! Synthetic device evidence exists only in test binaries. Production accepts
//! neither environment variables nor path names as optical-device evidence.
use super::Source;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

#[derive(Default)]
struct Profiles {
    paths: BTreeMap<PathBuf, Source>,
    readers: BTreeMap<String, (usize, usize, usize)>,
    /// Folders whose probes are counted, with the count so far.
    watched: BTreeMap<PathBuf, usize>,
}
fn profiles() -> &'static Mutex<Profiles> {
    static PROFILES: OnceLock<Mutex<Profiles>> = OnceLock::new();
    PROFILES.get_or_init(|| Mutex::new(Profiles::default()))
}

pub(crate) struct Profile(PathBuf);
pub(crate) fn register(root: &Path, source: Source) -> Profile {
    let root = crate::local_access::normalize_scan_root(root);
    profiles()
        .lock()
        .unwrap()
        .paths
        .insert(root.clone(), source);
    Profile(root)
}
impl Profile {
    pub(crate) fn replace(&self, source: Source) {
        profiles()
            .lock()
            .unwrap()
            .paths
            .insert(self.0.clone(), source);
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        profiles().lock().unwrap().paths.remove(&self.0);
    }
}
pub(super) fn probe(path: &Path) -> Option<Source> {
    let path = crate::local_access::normalize_scan_root(path);
    profiles()
        .lock()
        .unwrap()
        .paths
        .iter()
        .filter(|(root, _)| path.starts_with(root))
        .max_by_key(|(root, _)| root.components().count())
        .map(|(_, source)| source.clone())
}
pub(super) fn enter(device: &str) {
    let mut profiles = profiles().lock().unwrap();
    let (active, maximum, acquisitions) = profiles.readers.entry(device.into()).or_default();
    *acquisitions += 1;
    *active += 1;
    *maximum = (*maximum).max(*active);
}
pub(super) fn leave(device: &str) {
    let mut profiles = profiles().lock().unwrap();
    if let Some((active, _, _)) = profiles.readers.get_mut(device) {
        *active = active.saturating_sub(1);
    }
}
pub(crate) fn maximum(device: &str) -> usize {
    profiles()
        .lock()
        .unwrap()
        .readers
        .get(device)
        .map_or(0, |(_, max, _)| *max)
}

pub(crate) fn acquisitions(device: &str) -> usize {
    profiles()
        .lock()
        .unwrap()
        .readers
        .get(device)
        .map_or(0, |(_, _, count)| *count)
}

/// The registered root that gives `path` its synthetic device, if any.
pub(super) fn root_of(path: &Path) -> Option<PathBuf> {
    let path = crate::local_access::normalize_scan_root(path);
    profiles()
        .lock()
        .unwrap()
        .paths
        .keys()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.components().count())
        .cloned()
}

/// Counts the probes of paths below one folder while it lives.
pub(crate) struct ProbeWatch(PathBuf);
pub(crate) fn watch_probes(root: &Path) -> ProbeWatch {
    let root = crate::local_access::normalize_scan_root(root);
    profiles().lock().unwrap().watched.insert(root.clone(), 0);
    ProbeWatch(root)
}
impl ProbeWatch {
    pub(crate) fn count(&self) -> usize {
        profiles()
            .lock()
            .unwrap()
            .watched
            .get(&self.0)
            .copied()
            .unwrap_or(0)
    }
}
impl Drop for ProbeWatch {
    fn drop(&mut self) {
        profiles().lock().unwrap().watched.remove(&self.0);
    }
}
pub(super) fn count_probe(path: &Path) {
    let path = crate::local_access::normalize_scan_root(path);
    for (root, count) in profiles().lock().unwrap().watched.iter_mut() {
        if path.starts_with(root) {
            *count += 1;
        }
    }
}
