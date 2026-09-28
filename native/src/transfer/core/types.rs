//! Progress and completion values every transfer worker reports.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug)]
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

#[derive(Clone, Debug)]
pub enum TransferMsg {
    Progress(TransferProgress),
    Done {
        progress: TransferProgress,
        errors: Vec<String>,
        canceled: bool,
    },
}
