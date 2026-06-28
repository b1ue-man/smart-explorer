use super::prelude::*;

/// What the in-app folder picker is being opened for — decides the title, where
/// the chosen folder is routed, and whether remote connections are offered.
pub(in crate::app) enum PickerPurpose {
    /// Sync-setup editor source/target (remote endpoints allowed).
    SyncSource,
    SyncTarget,
    /// Open a folder in the explorer (local).
    ScanFolder,
    /// Pick a folder to run storage-analytics on (local).
    AnalyticsFolder,
    /// Pick a local folder for find-and-reclaim.
    ReclaimFolder,
    /// One-way mirror the current folder into a local destination.
    MirrorDest,
    /// Two-way sync the current folder with a local destination.
    BisyncDest,
    /// Copy-dialog destination folder (local).
    CopyDest,
    /// Download a remote file (at remote path `src`) into the picked dir.
    DownloadTo {
        src: String,
    },
}

impl PickerPurpose {
    pub(in crate::app) fn title(&self) -> &'static str {
        match self {
            PickerPurpose::SyncSource => "📂 Quelle wählen",
            PickerPurpose::SyncTarget => "📂 Ziel wählen",
            PickerPurpose::ScanFolder => "📂 Ordner öffnen",
            PickerPurpose::AnalyticsFolder => "📂 Ordner für Analyse",
            PickerPurpose::ReclaimFolder => "Ordner fuer Aufraeumen",
            PickerPurpose::MirrorDest => "📂 Ziel zum Spiegeln",
            PickerPurpose::BisyncDest => "📂 Ziel für 2-Wege-Sync",
            PickerPurpose::CopyDest => "📂 Zielordner wählen",
            PickerPurpose::DownloadTo { .. } => "📂 Speichern unter…",
        }
    }
    /// Whether to offer remote connections too. Sync source/target and the
    /// storage-analysis target can point at a remote; the rest are local-only.
    pub(in crate::app) fn local_only(&self) -> bool {
        !matches!(
            self,
            PickerPurpose::SyncSource | PickerPurpose::SyncTarget | PickerPurpose::AnalyticsFolder
        )
    }
}

/// In-app folder picker (#17): browse local drives AND saved remote
/// connections through the same `Backend` abstraction and choose a folder —
/// so a sync setup's source/target can point at a saved connection's remote
/// location without typing it. The chosen value is a local path or a
/// `proto://user@host:port/path` endpoint the sync runner re-opens.
pub(in crate::app) struct PickerState {
    pub(in crate::app) purpose: PickerPurpose,
    /// Live backend for the current location (None = root list / not connected).
    pub(in crate::app) backend: Option<crate::vfs::BackendHandle>,
    pub(in crate::app) is_remote: bool,
    /// "" for local; "proto://user@host:port" for remote, to build the endpoint.
    pub(in crate::app) endpoint_prefix: String,
    pub(in crate::app) conn_label: String,
    /// Absolute forward-slash directory currently shown.
    pub(in crate::app) cwd: String,
    /// Sub-folders of `cwd` (name only), sorted.
    pub(in crate::app) entries: Vec<String>,
    pub(in crate::app) error: Option<String>,
    /// Async connect for a saved connection.
    pub(in crate::app) connect_rx: Option<Receiver<crate::connect::ConnectResult>>,
    pub(in crate::app) connecting: bool,
}

#[derive(Clone, Copy)]
pub(in crate::app) enum ClipKey {
    Copy,
    Cut,
    Paste,
}
