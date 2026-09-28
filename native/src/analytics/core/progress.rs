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
            Self::Legacy => "Ältere Gegenstelle: bisheriger Analysepfad",
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
    state: Arc<Mutex<State>>,
    scope: Option<Arc<(String, String)>>,
    offset: (u64, u64, u64),
}

impl Progress {
    /// Counters and cancellation are shared; only path presentation is scoped.
    pub(crate) fn scoped(&self, physical: String, visible: String) -> Self {
        Self { scope: Some(Arc::new((physical, visible))), ..self.clone() }
    }

    pub(crate) fn remote_segment(&self) -> Self {
        let current = self.snapshot();
        Self { offset: (current.files, current.dirs, current.bytes), ..self.clone() }
    }

    pub(crate) fn visible_path(&self, path: &str) -> String {
        let Some(scope) = &self.scope else { return path.to_owned() };
        let normalized = path.replace('\\', "/");
        if normalized == scope.1 || normalized.starts_with(&format!("{}/", scope.1.trim_end_matches('/'))) {
            return normalized;
        }
        let base = scope.0.trim_end_matches('/');
        match normalized.strip_prefix(base) {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => {
                format!("{}{}", scope.1.trim_end_matches('/'), rest)
            }
            _ => scope.1.clone(),
        }
    }

    pub fn set_phase(&self, phase: ScanPhase, current: impl Into<String>) {
        let current = self.visible_path(&current.into());
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.snapshot.phase != phase || state.snapshot.current != current {
            state.changed = Instant::now();
            state.snapshot.unchanged_ms = 0;
        }
        state.snapshot.phase = phase;
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
        if counts != (state.snapshot.files, state.snapshot.dirs, state.snapshot.bytes) {
            (state.snapshot.files, state.snapshot.dirs, state.snapshot.bytes) = counts;
            state.changed = Instant::now();
            state.snapshot.unchanged_ms = 0;
        }
        let mut snapshot = state.snapshot.clone();
        snapshot.unchanged_ms = snapshot.unchanged_ms.saturating_add(
            state.changed.elapsed().as_millis().min(u64::MAX as u128) as u64,
        );
        if let Some(received) = state.remote_received {
            snapshot.source_age_ms = snapshot.source_age_ms.saturating_add(received.elapsed().as_millis() as u64);
        }
        snapshot
    }

    /// Called only for actual received peer evidence, never by cancellation polls.
    pub(crate) fn receive(&self, mut snapshot: ScanSnapshot) -> io::Result<()> {
        snapshot.files = snapshot.files.checked_add(self.offset.0).ok_or_else(counter_overflow)?;
        snapshot.dirs = snapshot.dirs.checked_add(self.offset.1).ok_or_else(counter_overflow)?;
        snapshot.bytes = snapshot.bytes.checked_add(self.offset.2).ok_or_else(counter_overflow)?;
        snapshot.current = self.visible_path(&snapshot.current);
        let previous = self.snapshot();
        if snapshot.files < previous.files || snapshot.dirs < previous.dirs
            || snapshot.bytes < previous.bytes || snapshot.current.len() > 32 * 1024
            || snapshot.transferred > snapshot.transfer_total
        {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Ungültiger Analyse-Fortschritt"));
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

    pub fn remote_report_age(&self) -> Option<Duration> {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.remote_received.map(|at| at.elapsed().saturating_add(Duration::from_millis(state.snapshot.source_age_ms)))
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

    pub fn check_cancel(&self) -> io::Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(io::Error::new(io::ErrorKind::Interrupted, "Speicheranalyse abgebrochen"))
        } else {
            Ok(())
        }
    }
}

fn counter_overflow() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Analyse-Zählerüberlauf")
}
