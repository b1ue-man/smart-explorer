use super::prelude::*;

pub(in crate::app) const APP_ERROR_LOG_LIMIT: usize = 200;
pub(in crate::app) const DOWNLOAD_SPACE_MARGIN_BYTES: u64 = 32 * 1024 * 1024;
pub(in crate::app) const TEMP_SESSION_PID_FILE: &str = "session.pid";

// ─── Own context-menu command IDs (>= shell_menu::OWN_ID_BASE) ─────────────
pub(in crate::app) mod menu_ids {
    pub const COPY: u32 = 0x8000;
    pub const CUT: u32 = 0x8001;
    pub const COPY_PATH: u32 = 0x8002;
    pub const COPY_TO: u32 = 0x8003;
    pub const MOVE_TO: u32 = 0x8004;
    pub const RENAME: u32 = 0x8005;
    pub const TOGGLE_FAV: u32 = 0x8006;
    pub const EXTRACT_ZIP: u32 = 0x8007;
    // Background (empty space) menu
    pub const PASTE: u32 = 0x8010;
    pub const NEW_FOLDER: u32 = 0x8011;
    pub const REFRESH: u32 = 0x8012;
    pub const SELECT_ALL: u32 = 0x8013;
}

/// Everything that belongs to one tab. The ACTIVE tab's state lives directly
/// in the `App` fields (so the rest of the code stays unchanged); inactive
/// tabs park their state here. Switching tabs swaps the field sets.
pub(in crate::app) struct TabState {
    pub(in crate::app) root_path: String,
    pub(in crate::app) entries: Vec<FileEntry>,
    pub(in crate::app) view: Vec<(usize, u32)>,
    pub(in crate::app) selection: HashSet<Arc<str>>,
    pub(in crate::app) last_anchor: Option<Arc<str>>,
    pub(in crate::app) cursor: Option<Arc<str>>,
    pub(in crate::app) scan_rx: Option<Receiver<ScanMessage>>,
    pub(in crate::app) scan_handle: Option<ScanHandle>,
    pub(in crate::app) progress: ScanProgress,
    pub(in crate::app) scan_running: bool,
    pub(in crate::app) history: Vec<String>,
    pub(in crate::app) forward: Vec<String>,
    pub(in crate::app) failed_paths: Vec<(String, String)>,
    pub(in crate::app) view_dirty: bool,
    /// Per-tab remote session (SFTP/FTP/WebDAV). Lives here so opening/closing
    /// another tab can't touch this tab's connection.
    pub(in crate::app) remote: Option<crate::connect::RemoteState>,
    /// Per-tab authenticated network-share connection (kept alive while the tab
    /// browses the UNC path).
    pub(in crate::app) net_conn: Option<crate::net::NetConnection>,
    // ── Per-tab filter / search / sort (so each tab — and each split pane —
    //    filters independently) ──
    pub(in crate::app) filter: FilterDef,
    pub(in crate::app) sort_key: SortKey,
    pub(in crate::app) sort_dir: SortDir,
    pub(in crate::app) text_draft: String,
    pub(in crate::app) ext_draft: String,
    pub(in crate::app) size_min_draft: String,
    pub(in crate::app) size_max_draft: String,
    pub(in crate::app) filter_pending_at: Option<Instant>,
    pub(in crate::app) mtime_min_date: Option<chrono::NaiveDate>,
    pub(in crate::app) mtime_max_date: Option<chrono::NaiveDate>,
    pub(in crate::app) btime_min_date: Option<chrono::NaiveDate>,
    pub(in crate::app) btime_max_date: Option<chrono::NaiveDate>,
}

pub(in crate::app) fn empty_progress() -> ScanProgress {
    ScanProgress {
        scanned: 0,
        bytes: 0,
        errors: 0,
        elapsed_ms: 0,
        current_path: String::new(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum TransferKind {
    Upload,
    Download,
    RemoteCopy,
}

impl TransferKind {
    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            TransferKind::Upload => "Upload",
            TransferKind::Download => "Download",
            TransferKind::RemoteCopy => "Remote-Kopie",
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::app) struct TransferProgress {
    pub(in crate::app) kind: TransferKind,
    pub(in crate::app) label: String,
    pub(in crate::app) current: String,
    pub(in crate::app) files_done: u64,
    pub(in crate::app) files_total: u64,
    pub(in crate::app) bytes_done: u64,
    pub(in crate::app) bytes_total: u64,
    pub(in crate::app) elapsed_ms: u64,
    pub(in crate::app) errors: u64,
    pub(in crate::app) done: bool,
}

impl TransferProgress {
    pub(in crate::app) fn new(
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
            done: false,
        }
    }

    pub(in crate::app) fn fraction(&self) -> f32 {
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
pub(in crate::app) enum TransferMsg {
    Progress(TransferProgress),
    Done {
        progress: TransferProgress,
        errors: Vec<String>,
    },
}

impl Default for TabState {
    fn default() -> Self {
        Self {
            root_path: String::new(),
            entries: Vec::new(),
            view: Vec::new(),
            selection: HashSet::new(),
            last_anchor: None,
            cursor: None,
            scan_rx: None,
            scan_handle: None,
            progress: empty_progress(),
            scan_running: false,
            history: Vec::new(),
            forward: Vec::new(),
            failed_paths: Vec::new(),
            view_dirty: false,
            remote: None,
            net_conn: None,
            filter: FilterDef::new(),
            sort_key: SortKey::Path,
            sort_dir: SortDir::Asc,
            text_draft: String::new(),
            ext_draft: String::new(),
            size_min_draft: String::new(),
            size_max_draft: String::new(),
            filter_pending_at: None,
            mtime_min_date: None,
            mtime_max_date: None,
            btime_min_date: None,
            btime_max_date: None,
        }
    }
}

/// Cached aggregation for the summary panel — rebuilding this on every frame
/// over a million entries was measurable; now it's rebuilt only when the view
/// changes.
pub(in crate::app) struct SummaryData {
    pub(in crate::app) files: u64,
    pub(in crate::app) dirs: u64,
    pub(in crate::app) bytes: u64,
    pub(in crate::app) oldest: i64,
    pub(in crate::app) newest: i64,
    pub(in crate::app) by_ext: Vec<(String, u64, u64)>,
    pub(in crate::app) top: Vec<(String, String, u64)>,
}

#[derive(Clone)]
pub(in crate::app) struct AppErrorEntry {
    pub(in crate::app) ts: String,
    pub(in crate::app) context: String,
    pub(in crate::app) detail: String,
}

/// One painted cell of the nested treemap. Cached per focus + treemap size so
/// painting (every frame) is cheap; the layout walk only re-runs on drill or
/// resize. Containers (folders we recurse into) are drawn as a dark box with a
/// header; leaves (files, or folders too small to recurse) are filled solid.
#[derive(Clone)]
pub(in crate::app) struct TmCell {
    pub(in crate::app) rect: egui::Rect,
    pub(in crate::app) name: String,
    pub(in crate::app) path: String,
    pub(in crate::app) size: u64,
    pub(in crate::app) is_dir: bool,
    pub(in crate::app) container: bool,
    pub(in crate::app) color: Color32,
}

/// A running background analytics scan (dedicated low-memory size walk).
pub(in crate::app) struct AnalyticsScan {
    pub(in crate::app) rx: Receiver<crate::analytics::SizeNode>,
    pub(in crate::app) progress: crate::analytics::Progress,
    /// Root being scanned (`/`-normalised), for the progress label.
    pub(in crate::app) root: String,
    pub(in crate::app) started: Instant,
}

/// Keyboard actions are collected inside the input closure and executed
/// afterwards — calling back into egui (clipboard, repaint) from within
/// `input_mut` can deadlock the context lock.
pub(in crate::app) enum KbdAct {
    SelectAll,
    CopyPathsText,
    TrashSel,
    ClearSel,
    Rescan,
    Back,
    Forward,
    Up,
    ToggleRecursive,
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    NewFolder,
    RenameSel,
    PathEdit,
    Move(isize, bool),
    MoveToEnd(bool, bool), // (to_end, shift)
    Open,
    Properties,
    PermanentDelete,
    RevealInExplorer,
    InvertSelection,
    FocusSearch,
    FocusFilter,
    ToggleHelp,
    ToggleSplit,
    StarCurrent,
    /// Alt+1..9: jump to tab N (Alt+9 = last tab).
    SelectTab(usize),
}
