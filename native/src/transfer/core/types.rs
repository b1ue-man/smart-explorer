//! Progress and completion values every transfer worker reports.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferKind {
    Upload,
    Download,
    RemoteCopy,
}

impl TransferKind {
    pub fn label(self) -> &'static str {
        match self {
            TransferKind::Upload => "Upload",
            TransferKind::Download => "Download",
            TransferKind::RemoteCopy => "Remote-Kopie",
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
