//! Live phase of a duplicate search for the status line of its task.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use super::types::ReclaimProgress;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReclaimPhase {
    /// Directories are read and candidates collected.
    #[default]
    Walking,
    /// First and last bytes of same-size candidates are compared.
    Fingerprinting,
    /// Whole contents of candidates with equal ends are compared.
    Hashing,
    /// Groups are formed.
    Grouping,
}

impl ReclaimPhase {
    const ALL: [Self; 4] = [
        Self::Walking,
        Self::Fingerprinting,
        Self::Hashing,
        Self::Grouping,
    ];
}

#[derive(Default)]
struct StageState {
    phase: AtomicU8,
    files_total: AtomicU64,
    bytes_total: AtomicU64,
    bytes_done: AtomicU64,
    current: Mutex<PathBuf>,
    visible_roots: Mutex<Vec<(String, String)>>,
}

/// Shared like the other progress counters. Walks that report no phases
/// (the desktop's) leave it at `Walking` without a current directory.
#[derive(Clone, Default)]
pub struct ReclaimStage(Arc<StageState>);

impl ReclaimStage {
    pub fn phase(&self) -> ReclaimPhase {
        let index = usize::from(self.0.phase.load(Ordering::Relaxed));
        ReclaimPhase::ALL.get(index).copied().unwrap_or_default()
    }

    /// Best effort: a directory entered while another thread updates the
    /// line is skipped, the walk never waits for the status.
    pub(super) fn enter_directory(&self, dir: &Path) {
        if let Ok(mut current) = self.0.current.try_lock() {
            let text = current.as_mut_os_string();
            text.clear();
            text.push(dir.as_os_str());
        }
    }

    pub(super) fn begin(&self, phase: ReclaimPhase, files: u64, bytes: u64) {
        self.0.files_total.store(files, Ordering::Relaxed);
        self.0.bytes_total.store(bytes, Ordering::Relaxed);
        self.0.bytes_done.store(0, Ordering::Relaxed);
        let index = ReclaimPhase::ALL
            .iter()
            .position(|known| *known == phase)
            .unwrap_or_default();
        self.0.phase.store(index as u8, Ordering::Relaxed);
    }

    pub(crate) fn add_bytes(&self, bytes: u64) {
        self.0.bytes_done.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Files and bytes the current phase works through and the bytes done
    /// (a host reports them to the peer that asked for the search).
    pub(crate) fn totals(&self) -> (u64, u64, u64) {
        (
            self.0.files_total.load(Ordering::Relaxed),
            self.0.bytes_total.load(Ordering::Relaxed),
            self.0.bytes_done.load(Ordering::Relaxed),
        )
    }

    /// The state of a search another device runs: its phase, the totals of
    /// that phase (`totals`) and the folder it reads.
    pub(crate) fn mirror(&self, phase: ReclaimPhase, totals: (u64, u64, u64), current: &str) {
        let (files, bytes, done) = totals;
        self.0.files_total.store(files, Ordering::Relaxed);
        self.0.bytes_total.store(bytes, Ordering::Relaxed);
        self.0.bytes_done.store(done, Ordering::Relaxed);
        let index = ReclaimPhase::ALL
            .iter()
            .position(|known| *known == phase)
            .unwrap_or_default();
        self.0.phase.store(index as u8, Ordering::Relaxed);
        let mut shown = self.0.current.lock().unwrap_or_else(|p| p.into_inner());
        *shown = PathBuf::from(current);
    }

    /// The folder being read, as the status line shows it.
    pub(crate) fn current_text(&self) -> String {
        self.current()
    }

    pub(crate) fn map_directories(&self, mut roots: Vec<(String, String)>) {
        for (physical, _) in &mut roots { *physical = crate::analytics::progress::normalize_path(physical); }
        roots.sort_by_key(|(physical, _)| std::cmp::Reverse(physical.len()));
        *self.0.visible_roots.lock().unwrap_or_else(|p| p.into_inner()) = roots;
    }

    fn current(&self) -> String {
        let current = self.0.current.lock().unwrap_or_else(|p| p.into_inner());
        let raw = super::util::to_fwd(&current);
        let roots = self.0.visible_roots.lock().unwrap_or_else(|p| p.into_inner());
        if roots.is_empty() { return raw; }
        let normalized = crate::analytics::progress::normalize_path(&raw);
        for (physical, visible) in roots.iter() {
            if let Some(rest) = normalized.strip_prefix(physical.trim_end_matches('/')) {
                if rest.is_empty() || rest.starts_with('/') { return format!("{}{rest}", visible.trim_end_matches('/')); }
            }
        }
        String::new()
    }
}

impl ReclaimProgress {
    /// German status line of the current phase, e.g. `1.234 Ordner · /pfad`
    /// or `Inhalt vergleichen: 3 von 12 Dateien · 40 %`.
    pub fn status_line(&self) -> String {
        let state = &self.stage.0;
        let total = state.files_total.load(Ordering::Relaxed);
        match self.stage.phase() {
            ReclaimPhase::Walking => crate::analytics::walk_status(
                self.dirs.load(Ordering::Relaxed),
                &self.stage.current(),
            ),
            ReclaimPhase::Fingerprinting => crate::analytics::compare_status(
                "Anfang und Ende vergleichen",
                self.fingerprinted.load(Ordering::Relaxed),
                total,
                None,
            ),
            ReclaimPhase::Hashing => crate::analytics::compare_status(
                "Inhalt vergleichen",
                self.hashed.load(Ordering::Relaxed),
                total,
                Some((
                    state.bytes_done.load(Ordering::Relaxed),
                    state.bytes_total.load(Ordering::Relaxed),
                )),
            ),
            ReclaimPhase::Grouping => "Gruppen werden gebildet …".to_string(),
        }
    }
}
