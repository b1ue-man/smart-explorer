//! Progress and completion values every transfer worker reports.
use super::limiter::Limiter;

// Serializable for the worker's job service (Z-P2): progress, issues and
// roots of a job that runs in the background worker reach the app over IPC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TransferKind {
    Upload,
    Download,
    RemoteCopy,
    /// Local folder to local folder.
    Local,
    /// Local folder to local folder, removing the sources.
    Move,
}

impl TransferKind {
    pub fn label(self) -> &'static str {
        match self {
            TransferKind::Upload => "Upload",
            TransferKind::Download => "Download",
            TransferKind::RemoteCopy => "Remote-Kopie",
            TransferKind::Local => "Kopieren",
            TransferKind::Move => "Verschieben",
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TransferProgress {
    pub kind: TransferKind,
    pub label: String,
    pub current: String,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub elapsed_ms: u64,
    pub errors: u64,
    /// Entries left out as protected omissions (the app trash while it is
    /// active); never counted as errors. Always 0 on the desktop builds.
    pub omitted: u64,
    pub done: bool,
    /// Selected folders are still being walked; the totals are the entries
    /// found so far and keep growing until this turns false.
    pub discovering: bool,
    /// Entries left alone by the conflict policy "skip".
    pub skipped: u64,
    /// Current throughput over the last few seconds (bytes per second).
    pub rate_bps: u64,
    /// Up to three entries being transferred right now.
    pub active: Vec<String>,
    /// Operations running concurrently on the transfer's connection(s).
    pub parallel: u32,
    /// Where the transfer reads from and writes to, for displays.
    pub source: String,
    pub target: String,
    /// A short hint about the current state or a limit of the connection
    /// (for example "verbinde …" or a provider's rate for new files).
    pub note: Option<String>,
    /// The complete issue log of this transfer (one JSON object per line),
    /// once the first issue was written.
    pub log_path: Option<String>,
    /// Who sets the pace of the transfer (confirmed over three windows of
    /// about a second); `None` until the first verdict. The final progress
    /// carries the last verdict that named a resource.
    pub limiter: Limiter,
    /// The German text of `limiter`: drives and devices and rates only, never
    /// a file or folder name. `None` for no verdict.
    pub limiter_text: Option<String>,
    /// Where the time of the last window went, in percent: source, target,
    /// computing, waiting.
    pub limiter_shares: [u8; 4],
    /// Link rate of the connection in bytes per second; 0 = unknown.
    pub link_bps: u64,
    /// Ranges of large files transferred right now.
    pub segments: u32,
    /// Decisions of the file rule "neuere ersetzen ältere"
    /// (`Conflict::Update`): files whose older target was replaced by a
    /// newer source ...
    #[serde(default)]
    pub replaced: u64,
    /// ... equal files that were skipped (part of `skipped`) ...
    #[serde(default)]
    pub skipped_equal: u64,
    /// ... newer targets that were kept (part of `skipped`) ...
    #[serde(default)]
    pub kept_newer: u64,
    /// ... and files of the same time but another size, kept beside their
    /// counterpart under a numbered name.
    #[serde(default)]
    pub kept_both: u64,
}

impl TransferProgress {
    pub fn new(
        kind: TransferKind,
        label: impl Into<String>,
        files_total: u64,
        bytes_total: u64,
    ) -> Self {
        Self {
            kind,
            label: label.into(),
            current: String::new(),
            files_done: 0,
            files_total,
            bytes_done: 0,
            bytes_total,
            elapsed_ms: 0,
            errors: 0,
            omitted: 0,
            done: false,
            discovering: false,
            skipped: 0,
            rate_bps: 0,
            active: Vec::new(),
            parallel: 0,
            source: String::new(),
            target: String::new(),
            note: None,
            log_path: None,
            limiter: Limiter::None,
            limiter_text: None,
            limiter_shares: [0; 4],
            link_bps: 0,
            segments: 0,
            replaced: 0,
            skipped_equal: 0,
            kept_newer: 0,
            kept_both: 0,
        }
    }

    pub fn fraction(&self) -> f32 {
        if self.bytes_total > 0 {
            (self.bytes_done as f32 / self.bytes_total as f32).clamp(0.0, 1.0)
        } else if self.files_total > 0 {
            (self.files_done as f32 / self.files_total as f32).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

/// One entry that was not transferred, or a problem of the whole transfer
/// (empty `path`), for lists users can read and copy.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TransferIssue {
    pub path: String,
    pub message: String,
}

/// Where a selected entry landed below the target folder (possibly under a
/// numbered name). A later "transfer missing files" run of the same job
/// reuses these instead of creating new copies.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResolvedRoot {
    /// The selected source path.
    pub source: String,
    /// Its destination relative to the target folder.
    pub rel: String,
}

#[derive(Clone, Debug)]
pub enum TransferMsg {
    Progress(TransferProgress),
    Done {
        progress: TransferProgress,
        /// Display lines of the first issues (legacy consumers).
        errors: Vec<String>,
        canceled: bool,
        /// The first issues with their paths; the full list is in
        /// `progress.log_path`.
        issues: Vec<TransferIssue>,
        /// Destination roots of the selected entries, for "transfer missing
        /// files" after a cancel or failure.
        roots: Vec<ResolvedRoot>,
    },
}
