//! Measured storage-analysis state shared by local and peer scans.
use serde::{Deserialize, Serialize};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScanPhase {
    #[default]
    Preparing,
    Queued,
    Scanning,
    Assembling,
    Transferring,
    Verifying,
    Legacy,
}

impl ScanPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Preparing => "Verbindung / Quelle wird geöffnet",
            Self::Queued => "Wartet auf freien Analyse-Worker",
            Self::Scanning => "Verzeichnisse werden gelesen",
            Self::Assembling => "Ergebnis wird zusammengestellt",
            Self::Transferring => "Fertiges Ergebnis wird übertragen",
            Self::Verifying => "Empfangenes Ergebnis wird geprüft",
            Self::Legacy => "Bisheriger Analysepfad: nur Datei- und Bytezähler",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanSnapshot {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub phase: ScanPhase,
    pub current: String,
    pub transferred: u64,
    pub transfer_total: u64,
    pub unchanged_ms: u64,
    pub host_scan_ms: Option<u64>,
    pub source_age_ms: u64,
    pub directories_unreported: bool,
    /// Place in a host's analysis queue while `Queued` (1 = next); 0 = not
    /// known or not waiting. Older hosts omit it.
    #[serde(default)]
    pub queue_position: u32,
}

/// Longest `current` a snapshot carries over the wire; a deeper path keeps
/// its end (the folder being read) behind an ellipsis.
pub(crate) const MAX_CURRENT_BYTES: usize = 4 * 1024;

/// `text` with at most `max` bytes: the end, behind `…`, cut at a character
/// boundary.
pub(crate) fn shorten_tail(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let keep = max.saturating_sub('…'.len_utf8());
    let mut start = text.len() - keep;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &text[start..])
}

struct State {
    snapshot: ScanSnapshot,
    changed: Instant,
    remote_received: Option<Instant>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            snapshot: ScanSnapshot::default(),
            changed: Instant::now(),
            remote_received: None,
        }
    }
}

#[derive(Clone, Default)]
pub struct Progress {
    pub files: Arc<AtomicU64>,
    pub dirs: Arc<AtomicU64>,
    pub bytes: Arc<AtomicU64>,
    pub cancel: Arc<AtomicBool>,
    node_limit: Arc<AtomicU64>,
    state: Arc<Mutex<State>>,
    scope: Option<Arc<(String, String)>>,
    offset: (u64, u64, u64),
    #[cfg(test)]
    reports: Arc<AtomicU64>,
}

impl Progress {
    /// Receiver's retained-node budget, shared by all scoped remote segments.
    pub fn node_budget(&self) -> u64 {
        let explicit = self.node_limit.load(Ordering::Relaxed);
        if explicit != 0 {
            return explicit;
        }
        // Names, nodes and the vectors retaining their children. The report
        // also checks the actual encoded size before allocating the tree.
        (crate::transfer::memory_budget() as u64 / 128).clamp(2, super::tree_transfer::MAX_NODES)
    }

    pub fn set_node_budget(&self, nodes: u64) {
        self.node_limit.store(nodes.clamp(2, super::tree_transfer::MAX_NODES), Ordering::Relaxed);
    }

    /// Counters and cancellation are shared; only path presentation is scoped.
    pub(crate) fn scoped(&self, physical: String, visible: String) -> Self {
        Self {
            scope: Some(Arc::new((physical, visible))),
            ..self.clone()
        }
    }

    pub(crate) fn remote_segment(&self) -> Self {
        let current = self.snapshot();
        Self {
            offset: (current.files, current.dirs, current.bytes),
            ..self.clone()
        }
    }

    /// A path of the scoped walk as the peer sees it. Walk paths are physical
    /// paths, so the physical root is stripped first: a visible export name
    /// that equals the first physical component (`/home` for `/home/alice`)
    /// must not pass a physical path through unchanged.
    pub(crate) fn visible_path(&self, path: &str) -> String {
        let Some(scope) = &self.scope else {
            return path.to_owned();
        };
        let normalized = normalize_path(path);
        let visible = scope.1.trim_end_matches('/');
        let normalized_base = normalize_path(&scope.0);
        let base = normalized_base.trim_end_matches('/');
        if let Some(rest) = normalized.strip_prefix(base) {
            if rest.is_empty() || rest.starts_with('/') {
                return format!("{visible}{rest}");
            }
        }
        if normalized == scope.1 || normalized.starts_with(&format!("{visible}/")) {
            return normalized;
        }
        scope.1.clone()
    }

    pub fn set_phase(&self, phase: ScanPhase, current: impl Into<String>) {
        let current = shorten_tail(&self.visible_path(&current.into()), MAX_CURRENT_BYTES);
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.snapshot.phase != phase || state.snapshot.current != current {
            state.changed = Instant::now();
            state.snapshot.unchanged_ms = 0;
        }
        state.snapshot.phase = phase;
        if phase == ScanPhase::Legacy {
            state.snapshot.directories_unreported = true;
        } else if phase == ScanPhase::Scanning {
            state.snapshot.directories_unreported = false;
        }
        state.snapshot.current = current;
        if !matches!(phase, ScanPhase::Transferring | ScanPhase::Verifying) {
            state.snapshot.transferred = 0;
            state.snapshot.transfer_total = 0;
        }
    }

    pub(crate) fn enter_directory(&self, path: &str) {
        self.set_phase(ScanPhase::Scanning, self.visible_path(path));
    }

    pub fn snapshot(&self) -> ScanSnapshot {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let counts = (
            self.files.load(Ordering::Relaxed),
            self.dirs.load(Ordering::Relaxed),
            self.bytes.load(Ordering::Relaxed),
        );
        if counts
            != (
                state.snapshot.files,
                state.snapshot.dirs,
                state.snapshot.bytes,
            )
        {
            (
                state.snapshot.files,
                state.snapshot.dirs,
                state.snapshot.bytes,
            ) = counts;
            state.changed = Instant::now();
            state.snapshot.unchanged_ms = 0;
        }
        let mut snapshot = state.snapshot.clone();
        snapshot.unchanged_ms = snapshot
            .unchanged_ms
            .saturating_add(state.changed.elapsed().as_millis().min(u64::MAX as u128) as u64);
        if let Some(received) = state.remote_received {
            snapshot.source_age_ms = snapshot
                .source_age_ms
                .saturating_add(received.elapsed().as_millis() as u64);
        }
        snapshot
    }

    /// Called only for actual received peer evidence, never by cancellation polls.
    pub(crate) fn receive(&self, snapshot: ScanSnapshot) -> io::Result<()> {
        self.receive_snapshot(snapshot, false)
    }

    /// The verified result's counters replace live estimates that may include
    /// a child whose scan later failed. Transfer shape/hash stay authoritative.
    pub(crate) fn receive_result(&self, snapshot: ScanSnapshot) -> io::Result<()> {
        self.receive_snapshot(snapshot, true)
    }

    fn receive_snapshot(&self, mut snapshot: ScanSnapshot, final_result: bool) -> io::Result<()> {
        #[cfg(test)]
        self.reports.fetch_add(1, Ordering::Relaxed);
        snapshot.files = snapshot
            .files
            .checked_add(self.offset.0)
            .ok_or_else(counter_overflow)?;
        snapshot.dirs = snapshot
            .dirs
            .checked_add(self.offset.1)
            .ok_or_else(counter_overflow)?;
        snapshot.bytes = snapshot
            .bytes
            .checked_add(self.offset.2)
            .ok_or_else(counter_overflow)?;
        // A host may walk deeper than one frame should carry: keep the end.
        snapshot.current = shorten_tail(&self.visible_path(&snapshot.current), MAX_CURRENT_BYTES);
        let previous = self.snapshot();
        if (!final_result && (snapshot.files < previous.files
            || snapshot.dirs < previous.dirs
            || snapshot.bytes < previous.bytes))
            || snapshot.transferred > snapshot.transfer_total
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Ungültiger Analyse-Fortschritt",
            ));
        }
        self.files.store(snapshot.files, Ordering::Relaxed);
        self.dirs.store(snapshot.dirs, Ordering::Relaxed);
        self.bytes.store(snapshot.bytes, Ordering::Relaxed);
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.snapshot = snapshot;
        state.changed = Instant::now();
        state.remote_received = Some(Instant::now());
        Ok(())
    }

    /// Before a peer's request is sent again after a lost connection: the
    /// host may start over, so its counters may begin at zero again.
    pub(crate) fn restart_remote(&self) {
        self.files.store(self.offset.0, Ordering::Relaxed);
        self.dirs.store(self.offset.1, Ordering::Relaxed);
        self.bytes.store(self.offset.2, Ordering::Relaxed);
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        (
            state.snapshot.files,
            state.snapshot.dirs,
            state.snapshot.bytes,
        ) = self.offset;
        state.snapshot.transferred = 0;
        state.snapshot.transfer_total = 0;
        state.changed = Instant::now();
    }

    pub fn remote_report_age(&self) -> Option<Duration> {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.remote_received.map(|at| {
            at.elapsed()
                .saturating_add(Duration::from_millis(state.snapshot.source_age_ms))
        })
    }

    pub(crate) fn transfer(&self, received: u64, total: u64) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.snapshot.transferred != received {
            state.changed = Instant::now();
            state.snapshot.unchanged_ms = 0;
        }
        state.snapshot.phase = ScanPhase::Transferring;
        state.snapshot.transferred = received;
        state.snapshot.transfer_total = total;
        state.remote_received = Some(Instant::now());
    }

    #[cfg(test)]
    pub(crate) fn report_count(&self) -> u64 {
        self.reports.load(Ordering::Relaxed)
    }

    pub fn check_cancel(&self) -> io::Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Speicheranalyse abgebrochen",
            ))
        } else {
            Ok(())
        }
    }
}

pub(crate) fn normalize_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    if let Some(unc) = normalized.strip_prefix("//?/UNC/") {
        format!("//{unc}")
    } else if let Some(rest) = normalized.strip_prefix("//?/") {
        rest.to_owned()
    } else {
        normalized
    }
}

fn counter_overflow() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Analyse-Zählerüberlauf")
}
