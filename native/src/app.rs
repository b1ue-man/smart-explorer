use crate::copy::{start_copy_expanded, start_copy_from_paths, CopyHandle, CopyMsg};
use crate::filter::{parse_size_input, CompiledFilter};
use crate::folder_index::{FolderIndex, IndexMsg};
use crate::format::{compare_entries, format_bytes, format_date};
use crate::scanner::{start_scan, ScanHandle, ScanMessage};
use crate::types::*;
use crossbeam_channel::{unbounded, Receiver};
use eframe::egui::{self, Color32, RichText};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

// ─── Own context-menu command IDs (>= shell_menu::OWN_ID_BASE) ─────────────
#[cfg(windows)]
mod menu_ids {
    pub const COPY: u32 = 0x8000;
    pub const CUT: u32 = 0x8001;
    pub const COPY_PATH: u32 = 0x8002;
    pub const COPY_TO: u32 = 0x8003;
    pub const MOVE_TO: u32 = 0x8004;
    pub const RENAME: u32 = 0x8005;
    pub const TOGGLE_FAV: u32 = 0x8006;
    // Background (empty space) menu
    pub const PASTE: u32 = 0x8010;
    pub const NEW_FOLDER: u32 = 0x8011;
    pub const REFRESH: u32 = 0x8012;
    pub const SELECT_ALL: u32 = 0x8013;
}

/// Everything that belongs to one tab. The ACTIVE tab's state lives directly
/// in the `App` fields (so the rest of the code stays unchanged); inactive
/// tabs park their state here. Switching tabs swaps the field sets.
struct TabState {
    root_path: String,
    entries: Vec<FileEntry>,
    view: Vec<(usize, u32)>,
    selection: HashSet<Arc<str>>,
    last_anchor: Option<Arc<str>>,
    cursor: Option<Arc<str>>,
    scan_rx: Option<Receiver<ScanMessage>>,
    scan_handle: Option<ScanHandle>,
    progress: ScanProgress,
    scan_running: bool,
    history: Vec<String>,
    forward: Vec<String>,
    failed_paths: Vec<(String, String)>,
    view_dirty: bool,
    /// Per-tab remote session (SFTP/FTP/WebDAV). Lives here so opening/closing
    /// another tab can't touch this tab's connection.
    remote: Option<crate::connect::RemoteState>,
    /// Per-tab authenticated network-share connection (kept alive while the tab
    /// browses the UNC path).
    net_conn: Option<crate::net::NetConnection>,
    // ── Per-tab filter / search / sort (so each tab — and each split pane —
    //    filters independently) ──
    filter: FilterDef,
    sort_key: SortKey,
    sort_dir: SortDir,
    text_draft: String,
    ext_draft: String,
    size_min_draft: String,
    size_max_draft: String,
    filter_pending_at: Option<Instant>,
    mtime_min_date: Option<chrono::NaiveDate>,
    mtime_max_date: Option<chrono::NaiveDate>,
    btime_min_date: Option<chrono::NaiveDate>,
    btime_max_date: Option<chrono::NaiveDate>,
}

fn empty_progress() -> ScanProgress {
    ScanProgress {
        scanned: 0,
        bytes: 0,
        errors: 0,
        elapsed_ms: 0,
        current_path: String::new(),
        done: false,
    }
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
struct SummaryData {
    files: u64,
    dirs: u64,
    bytes: u64,
    oldest: i64,
    newest: i64,
    by_ext: Vec<(String, u64, u64)>,
    top: Vec<(String, String, u64)>,
}

/// Keyboard actions are collected inside the input closure and executed
/// afterwards — calling back into egui (clipboard, repaint) from within
/// `input_mut` can deadlock the context lock.
enum KbdAct {
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
}

pub struct App {
    root_path: String,
    scan_running: bool,
    entries: Vec<FileEntry>,
    /// Visible rows: (entry index, display depth from current root).
    view: Vec<(usize, u32)>,
    selection: HashSet<Arc<str>>,
    last_anchor: Option<Arc<str>>,
    /// Keyboard cursor (focused row), moved by arrow keys.
    cursor: Option<Arc<str>>,
    scan_rx: Option<Receiver<ScanMessage>>,
    scan_handle: Option<ScanHandle>,
    progress: ScanProgress,

    filter: FilterDef,
    sort_key: SortKey,
    sort_dir: SortDir,

    show_filters: bool,
    show_summary: bool,

    recursive: bool,
    history: Vec<String>,
    forward: Vec<String>,

    // ─── Tabs ───────────────────────────────────────────────────────────
    tabs: Vec<TabState>,
    active_tab: usize,
    /// Split-screen: show two tabs side by side. `panes` are the tab indices
    /// in the left/right slots; the focused one equals `active_tab`.
    split: bool,
    panes: [usize; 2],

    // dialog state
    copy_open: bool,
    copy_mode_pending: CopyMode,
    copy_dest: String,
    copy_preserve: bool,
    copy_conflict: Conflict,
    copy_rx: Option<Receiver<CopyMsg>>,
    copy_handle: Option<CopyHandle>,
    copy_progress: Option<CopyProgress>,
    copy_errors: Vec<(String, String)>,
    /// Refresh the current directory when the running copy job finishes
    /// (set for paste operations into the current folder).
    copy_refresh_after: bool,

    error_msg: Option<String>,
    notice: Option<(String, std::time::Instant)>,
    failed_paths: Vec<(String, String)>,
    show_errors_dialog: bool,

    // Filter input drafts
    text_draft: String,
    ext_draft: String,
    size_min_draft: String,
    size_max_draft: String,
    /// Debounce: text/ext filter commits this long after the last keystroke.
    filter_pending_at: Option<Instant>,

    // Date filters (calendar pickers)
    mtime_min_date: Option<chrono::NaiveDate>,
    mtime_max_date: Option<chrono::NaiveDate>,
    btime_min_date: Option<chrono::NaiveDate>,
    btime_max_date: Option<chrono::NaiveDate>,

    drives: Vec<String>,
    drive_info: Vec<(String, u64, u64)>, // (root, free, total)
    home: PathBuf,
    recent: Vec<String>,
    /// Starred folders, persisted to favorites.txt. Saved on every mutation.
    favorites: Vec<String>,

    /// Native file-type icon cache (extension-keyed, off-thread extraction).
    icon_cache: crate::icons::IconCache,
    /// Whether the keyboard-shortcut cheat sheet overlay is open.
    show_help: bool,
    /// First-run disclaimer / liability notice (shown until acknowledged).
    show_disclaimer: bool,

    last_view_recompute: Instant,
    /// Entries arrived but the view hasn't been rebuilt yet (throttled during
    /// scans so a 1M-entry stream doesn't trigger a full sort per frame).
    view_dirty: bool,

    // Rubber-band selection
    band_press: Option<(f32, f32)>, // (screen x, screen y) at press
    band_active: bool,
    band_base: HashSet<Arc<str>>,
    /// Set while rendering the NON-focused split pane so its `ui_table` ignores
    /// the rubber-band gesture (which belongs to the focused pane only) —
    /// otherwise one drag-box would select in both panes.
    band_suppressed: bool,
    /// Last time a scroll input arrived — drives a short full-rate repaint tail
    /// so trackpad scrolling glides to a smooth stop (egui smooths the delta
    /// over frames but doesn't request those frames itself).
    last_scroll_at: Option<Instant>,

    // ─── File drag (between tabs/panes; out to Explorer on Windows) ──────
    /// Absolute forward-slash source paths being dragged (empty = no drag).
    drag_files: Vec<String>,
    drag_active: bool,
    /// Backend the drag started from when the source view is remote (None =
    /// local). Lets a drop download/upload/cross-copy as needed.
    drag_src: Option<crate::vfs::BackendHandle>,
    /// Tab the drag started from (drop onto the same tab is a no-op).
    drag_source_tab: usize,
    /// Once we've handed an active drag to the OS (drag-out), don't re-trigger.
    drag_out_started: bool,
    /// Per-frame: rect of each tab's header label, for drop routing.
    tab_header_rects: Vec<(usize, egui::Rect)>,
    /// Per-frame: (tab index, rect) of each split pane, for drop routing.
    pane_rects: Vec<(usize, egui::Rect)>,
    /// Tab index whose `ui_table` is rendering right now (focused tab, or the
    /// parked pane during its swapped render) — so a drag knows its source.
    current_render_tab: usize,
    /// False until we've revealed the window (maximized) after the first paint.
    shown: bool,

    pending_scroll_row: Option<usize>,

    // Type-to-jump
    type_jump: String,
    type_jump_at: Instant,

    // Rename dialog: (path fwd-slashes, draft name)
    rename_open: Option<(String, String)>,
    rename_focus: bool,

    // Breadcrumb / path edit
    path_edit_mode: bool,
    path_edit_focus: bool,
    /// Request focus on the folder-search / name-filter fields next frame.
    folder_search_focus: bool,
    name_filter_focus: bool,

    summary_cache: Option<SummaryData>,
    /// (selection len, entries len, bytes) — cheap invalidation key.
    sel_size_cache: (usize, usize, u64),

    // ─── Folder fuzzy search ────────────────────────────────────────────
    folder_index: FolderIndex,
    index_building: bool,
    index_progress: u64,
    index_progress_path: String,
    index_rx: Option<Receiver<IndexMsg>>,
    index_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    folder_search_query: String,
    folder_search_results: Vec<(String, i32)>,
    folder_search_pending_at: Option<std::time::Instant>,
    /// Background mtime-ranking of search results: (sequence, ranked).
    folder_search_rx: Option<Receiver<(u64, Vec<(String, i32)>)>>,
    folder_search_seq: u64,

    // Background trash result
    trash_rx: Option<Receiver<Option<String>>>,

    // ─── Self-update ────────────────────────────────────────────────────
    update_rx: Option<Receiver<crate::updater::UpdateMsg>>,
    /// A downloaded update is swapped in and waiting for a restart: (version,
    /// new exe path). Shows the restart-now prompt; the new binary is already
    /// on disk, so "Später" just keeps running the old code until next launch.
    update_ready: Option<(String, PathBuf)>,
    update_feed_draft: String,

    /// A folder path passed on the command line, scanned on the first frame.
    pending_initial_path: Option<PathBuf>,

    // ─── Shell integration (Windows; mirrors actual registry state) ─────
    integration_ctx_menu: bool,

    // Filter-aware clipboard (virtual files)
    #[cfg(windows)]
    clip_prepare_rx: Option<Receiver<Vec<crate::virtual_clipboard::VirtualFile>>>,
    #[cfg(windows)]
    virtual_clip: Option<(u32, Vec<(String, String)>)>, // (clipboard seq, (abs, rel))

    // Filesystem watcher state
    #[cfg(windows)]
    watcher: Option<notify::RecommendedWatcher>,
    #[cfg(windows)]
    watcher_rx: Option<crossbeam_channel::Receiver<notify::Result<notify::Event>>>,
    index_dirty: bool,
    index_last_saved: std::time::Instant,

    /// Background clipboard-key detection. egui swallows Ctrl+C/X/V and, for a
    /// file (non-text) clipboard, emits no paste event AND triggers no frame
    /// when idle — so the keypress is invisible to update(). A dedicated thread
    /// polls the OS key state and signals over this channel, waking the UI.
    clip_key_rx: Option<crossbeam_channel::Receiver<ClipKey>>,
    clip_key_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,

    // ─── Remote connections (SFTP/FTP/network shares) ───────────────────
    /// Active SFTP/FTP session; while set, navigation walks the backend via
    /// `rscan` instead of std::fs. `None` for local (incl. UNC shares).
    remote: Option<crate::connect::RemoteState>,
    /// Live authenticated network-share connection, kept alive while browsing
    /// the UNC path (which is read locally through std::fs).
    net_conn: Option<crate::net::NetConnection>,
    show_connect: bool,
    connecting: bool,
    connect_form: crate::connect::ConnectForm,
    connect_rx: Option<Receiver<crate::connect::ConnectResult>>,

    // One-way mirror of the current location to a chosen folder.
    sync_rx: Option<Receiver<crate::sync::SyncMsg>>,
    sync_running: bool,

    /// Cached saved-connection list (avoids reading connections.txt per frame).
    saved_connections: Vec<crate::creds::SavedConnection>,

    // ─── Two-way sync (bisync) + conflict resolution ─────────────────────
    bisync_rx: Option<Receiver<crate::bisync::Outcome>>,
    bisync_running: bool,
    bisync_ctx: Option<BisyncCtx>,
    bisync_conflicts: Vec<crate::bisync::Conflict>,
    show_bisync_conflicts: bool,
    /// Cancel flags so a running mirror / two-way sync can be stopped.
    sync_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    bisync_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,

    // ─── Saved sync setups (persistent jobs) ─────────────────────────────
    /// Loaded once at start, kept in sync with sync/jobs.tsv after edits/runs.
    sync_jobs: Vec<crate::syncjobs::SyncJob>,
    show_sync_jobs: bool,
    /// Open add/edit dialog (None = closed).
    job_editor: Option<JobEditor>,
    /// Id of the job whose run is currently in flight (so its `last_run`
    /// gets stamped on completion). None = ad-hoc run, nothing to stamp.
    running_job: Option<String>,

    // ─── In-app folder picker (local + saved remote connections) ─────────
    picker: Option<PickerState>,
    /// Resolving a remote job's endpoints off the UI thread before a run.
    job_connect_rx: Option<
        Receiver<
            Result<
                (
                    (crate::vfs::BackendHandle, String),
                    (crate::vfs::BackendHandle, String),
                ),
                String,
            >,
        >,
    >,
    job_connect_pending: Option<crate::syncjobs::SyncJob>,
    /// In-flight "download a remote file to temp, then open it" jobs (one per
    /// double-clicked remote file). Result is the local temp path to launch.
    file_open_rx: Vec<Receiver<Result<String, String>>>,
    /// How remote files are opened/edited (temp-watch vs CfAPI) — persisted.
    remote_open_mode: RemoteOpenMode,
    /// Temp-mode edit-watch: re-upload each temp copy to the remote on save.
    remote_edits: Vec<RemoteEdit>,
    edit_save_rx: Vec<Receiver<(PathBuf, Result<(), String>)>>,
    last_edit_poll: Instant,
    /// In-flight upload of clipboard/dropped files into a remote folder.
    /// Result is (files uploaded, errors).
    upload_rx: Option<Receiver<(u64, Vec<String>)>>,
    /// In-flight one-shot remote op (new folder, rename, download-to).
    /// Ok(notice)/Err(msg); the worker includes the op context in both.
    remote_op_rx: Option<Receiver<Result<String, String>>>,
    /// Open egui context menu for a remote entry: (screen pos, entry index).
    remote_ctx: Option<(egui::Pos2, usize)>,
    /// In-flight download of selected remote files to temp for a Ctrl+C →
    /// Explorer paste. Result is the local temp paths to put on the clipboard.
    clip_download_rx: Option<Receiver<Vec<String>>>,

    // ─── Cloud (OAuth) — slice 1: connect Google Drive ───────────────────
    cloud_client_id_draft: String,
    cloud_secret_draft: String,
    cloud_auth_rx: Option<Receiver<Result<(), String>>>,
    cloud_authing: bool,

    // ─── Peer file sharing (#21) ─────────────────────────────────────────
    share: Option<crate::share::ShareService>,
    show_share: bool,
    /// Rendezvous server "host:port" (persisted) + device name + drafts.
    share_server: String,
    share_server_draft: String,
    share_device_draft: String,
    /// Code typed to connect/join, and the code we generated to display.
    share_code_input: String,
    share_my_code: String,
    share_room: bool,
    share_roster: Vec<crate::share::RemoteDevice>,
    share_incoming: Vec<(u64, String, Vec<(String, u64)>)>,
    share_status: String,
    share_progress: Option<(u64, u64)>,

    // Quick Share (Android) LAN discovery — started lazily when Teilen opens.
    quickshare: Option<crate::quickshare::QuickShare>,
    qs_devices: Vec<crate::quickshare::QsDevice>,
}

/// Draft state for the add/edit sync-setup dialog. Number fields are kept as
/// strings so a half-typed value doesn't snap back.
struct JobEditor {
    /// Some(id) when editing an existing job, None for a new one.
    id: Option<String>,
    name: String,
    source: String,
    target: String,
    direction: crate::bisync::Direction,
    conflict: crate::bisync::ConflictMode,
    retain_days: String,
    interval_min: String,
    include_hidden: bool,
    /// One glob per line.
    ignore: String,
    enabled: bool,
}

impl JobEditor {
    fn blank(source: String, target: String) -> Self {
        JobEditor {
            id: None,
            name: String::new(),
            source,
            target,
            direction: crate::bisync::Direction::Both,
            conflict: crate::bisync::ConflictMode::FileLevel,
            retain_days: "30".into(),
            interval_min: "0".into(),
            include_hidden: true,
            ignore: String::new(),
            enabled: true,
        }
    }

    fn from_job(j: &crate::syncjobs::SyncJob) -> Self {
        JobEditor {
            id: Some(j.id.clone()),
            name: j.name.clone(),
            source: j.source.clone(),
            target: j.target.clone(),
            direction: j.direction,
            conflict: j.conflict,
            retain_days: j.retain_days.to_string(),
            interval_min: j.interval_min.to_string(),
            include_hidden: j.include_hidden,
            ignore: j.ignore.join("\n"),
            enabled: j.enabled,
        }
    }
}

/// Which sync-setup field the in-app folder picker fills in.
#[derive(Clone, Copy, PartialEq)]
enum PickerField {
    Source,
    Target,
}

/// In-app folder picker (#17): browse local drives AND saved remote
/// connections through the same `Backend` abstraction and choose a folder —
/// so a sync setup's source/target can point at a saved connection's remote
/// location without typing it. The chosen value is a local path or a
/// `proto://user@host:port/path` endpoint the sync runner re-opens.
struct PickerState {
    field: PickerField,
    /// Live backend for the current location (None = root list / not connected).
    backend: Option<crate::vfs::BackendHandle>,
    is_remote: bool,
    /// "" for local; "proto://user@host:port" for remote, to build the endpoint.
    endpoint_prefix: String,
    conn_label: String,
    /// Absolute forward-slash directory currently shown.
    cwd: String,
    /// Sub-folders of `cwd` (name only), sorted.
    entries: Vec<String>,
    error: Option<String>,
    /// Async connect for a saved connection.
    connect_rx: Option<Receiver<crate::connect::ConnectResult>>,
    connecting: bool,
}

#[cfg(windows)]
#[derive(Clone, Copy)]
enum ClipKey {
    Copy,
    Cut,
    Paste,
}
#[cfg(not(windows))]
#[derive(Clone, Copy)]
enum ClipKey {}

/// First-run liability notice (single source: the repo's DISCLAIMER.txt, also
/// used by the installer's accept page).
const DISCLAIMER_TEXT: &str = include_str!("../../DISCLAIMER.txt");

/// How many saved (set-up-once) remote connections stay pinned on the sidebar.
/// The freshest are shown there; any older ones overflow into the "Verbindung"
/// menu so the sidebar can't grow without bound.
const SIDEBAR_CONN_CAP: usize = 10;

/// Format a unix-millis timestamp as local "YYYY-MM-DD HH:MM".
fn fmt_ms(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

/// Live context for a two-way sync, kept so its conflicts can be resolved
/// against the same backends afterwards.
struct BisyncCtx {
    a: crate::vfs::BackendHandle,
    root_a: String,
    b: crate::vfs::BackendHandle,
    root_b: String,
    pair: String,
    baseline: crate::bisync::Baseline,
}

/// Whether a forward-slash path is a LOCAL path (drive letter `X:/…` or a UNC
/// `//server/…`). Remote SFTP/FTP roots are rooted POSIX paths (`/…`) with no
/// drive prefix, so this distinguishes "stay on the remote backend" from
/// "switch back to the local std::fs scanner".
/// How a remote file is opened/edited: a temp copy watched for save-back, or a
/// native Windows Cloud-Files placeholder (CfAPI). User-toggleable.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RemoteOpenMode {
    Temp,
    CfApi,
}

fn app_data_file(name: &str) -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let d = base.join("smart_explorer");
    let _ = std::fs::create_dir_all(&d);
    d.join(name)
}

fn load_remote_open_mode() -> RemoteOpenMode {
    match std::fs::read_to_string(app_data_file("remote_open_mode.txt"))
        .map(|s| s.trim().to_string())
        .as_deref()
    {
        Ok("cfapi") => RemoteOpenMode::CfApi,
        _ => RemoteOpenMode::Temp,
    }
}

fn save_remote_open_mode(m: RemoteOpenMode) {
    let v = match m {
        RemoteOpenMode::Temp => "temp",
        RemoteOpenMode::CfApi => "cfapi",
    };
    let _ = std::fs::write(app_data_file("remote_open_mode.txt"), v);
}

fn share_server_path() -> PathBuf {
    app_data_file("share_server.txt")
}

fn load_share_server() -> String {
    std::fs::read_to_string(share_server_path())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn default_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Mein Gerät".to_string())
}

/// Stream a remote file to a temp copy and return its local path (for opening
/// remote files in their associated app). Overwrites a prior copy of the same
/// name so re-opening picks up fresh content.
fn download_to_temp(
    be: &dyn crate::vfs::Backend,
    path: &str,
    name: &str,
) -> Result<String, String> {
    download_to(be, path, &open_temp_path(name))
}

/// Stream a remote file to an explicit local `dest` (creating parents). Returns
/// the local path string for launching.
fn download_to(
    be: &dyn crate::vfs::Backend,
    path: &str,
    dest: &std::path::Path,
) -> Result<String, String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut r = be.open_read(path).map_err(|e| e.to_string())?;
    let mut f = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    std::io::copy(&mut r, &mut f).map_err(|e| e.to_string())?;
    Ok(dest.to_string_lossy().to_string())
}

/// Local temp path a remote file is downloaded to for opening/editing.
fn open_temp_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("smart_explorer_open");
    let _ = std::fs::create_dir_all(&dir);
    let safe = name.replace(['/', '\\', ':'], "_");
    dir.join(if safe.trim().is_empty() { "datei".to_string() } else { safe })
}

fn file_mtime_ms(p: &std::path::Path) -> i64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// A remote file opened for editing in **temp mode**: the temp copy is watched
/// and re-uploaded to `remote_path` on the backend whenever it's saved.
struct RemoteEdit {
    temp: PathBuf,
    backend: crate::vfs::BackendHandle,
    remote_path: String,
    name: String,
    /// Last mtime uploaded/downloaded — a change above this is a save.
    baseline_mtime: i64,
    /// mtime seen last poll (1-cycle debounce so we don't upload mid-write).
    seen_mtime: i64,
    uploading: bool,
}

fn rjoin(root: &str, name: &str) -> String {
    format!("{}/{}", root.trim_end_matches('/'), name)
}

/// Stream one local file to `dest` on the backend (creating parent dirs). The
/// `flush()` is essential — the Drive backend uploads on flush.
fn upload_file(be: &dyn crate::vfs::Backend, src: &std::path::Path, dest: &str) -> Result<(), String> {
    use std::io::Write;
    if let Some((parent, _)) = dest.rsplit_once('/') {
        let _ = be.mkdir_all(parent);
    }
    let mut r = std::fs::File::open(src).map_err(|e| e.to_string())?;
    let mut w = be.open_write(dest).map_err(|e| e.to_string())?;
    std::io::copy(&mut r, &mut w).map_err(|e| e.to_string())?;
    w.flush().map_err(|e| e.to_string())?;
    Ok(())
}

fn upload_dir(
    be: &dyn crate::vfs::Backend,
    dir: &std::path::Path,
    dest: &str,
    copied: &mut u64,
    errors: &mut Vec<String>,
) {
    if let Err(e) = be.mkdir_all(dest) {
        errors.push(format!("{}: {}", dest, e));
        return;
    }
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            errors.push(format!("{}: {}", dir.display(), e));
            return;
        }
    };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let child = rjoin(dest, &name);
        let path = entry.path();
        if path.is_dir() {
            upload_dir(be, &path, &child, copied, errors);
        } else {
            match upload_file(be, &path, &child) {
                Ok(_) => *copied += 1,
                Err(e) => errors.push(format!("{}: {}", name, e)),
            }
        }
    }
}

/// Upload a set of local paths (files/folders) into `dest_root` on the backend.
/// Returns (files uploaded, error messages). Conflicts overwrite by name.
fn upload_paths(
    be: &dyn crate::vfs::Backend,
    paths: &[String],
    dest_root: &str,
) -> (u64, Vec<String>) {
    let mut copied = 0u64;
    let mut errors = Vec::new();
    for p in paths {
        let src = std::path::PathBuf::from(p);
        let base = src
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if base.is_empty() {
            continue;
        }
        let dest = rjoin(dest_root, &base);
        if src.is_dir() {
            upload_dir(be, &src, &dest, &mut copied, &mut errors);
        } else {
            match upload_file(be, &src, &dest) {
                Ok(_) => copied += 1,
                Err(e) => errors.push(format!("{}: {}", base, e)),
            }
        }
    }
    (copied, errors)
}

/// A bare drive letter like `C:` is **drive-relative** on Windows (it means
/// "current dir on C:"), so `read_dir("C:")` lists the wrong folder. Normalize
/// it to the drive root `C:/`.
fn ensure_dir_root(p: &str) -> String {
    let t = p.trim();
    let b = t.as_bytes();
    if b.len() == 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        format!("{}/", t)
    } else {
        t.to_string()
    }
}

pub(crate) fn is_local_style(path: &str) -> bool {
    let p = path.trim_start();
    let b = p.as_bytes();
    let has_drive = b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic();
    has_drive || p.starts_with("//") || p.starts_with("\\\\")
}

impl App {
    pub fn new(just_updated: bool, initial_path: Option<PathBuf>) -> Self {
        let home = dirs_home();
        let drives = list_drives();
        let drive_info = drive_info_list(&drives);
        let recent: Vec<String> = std::fs::read_to_string(settings_path())
            .ok()
            .map(|s| s.lines().map(|l| l.to_string()).collect())
            .unwrap_or_default();
        let favorites: Vec<String> = std::fs::read_to_string(favorites_path())
            .ok()
            .map(|s| s.lines().filter(|l| !l.is_empty()).map(|l| l.to_string()).collect())
            .unwrap_or_default();
        let ui_state = UiState::load();

        // Kick off the automatic update check (silent unless an update is
        // found and applied).
        let (utx, urx) = unbounded();
        crate::updater::check_async(utx, false);

        Self {
            root_path: String::new(),
            scan_running: false,
            entries: Vec::new(),
            view: Vec::new(),
            selection: HashSet::new(),
            last_anchor: None,
            cursor: None,
            scan_rx: None,
            scan_handle: None,
            progress: empty_progress(),

            filter: FilterDef::new(),
            sort_key: SortKey::Path,
            sort_dir: SortDir::Asc,

            show_filters: ui_state.show_filters,
            show_summary: ui_state.show_summary,
            recursive: false,
            history: Vec::new(),
            forward: Vec::new(),

            tabs: vec![TabState::default()],
            active_tab: 0,
            split: false,
            panes: [0, 1],

            copy_open: false,
            copy_mode_pending: CopyMode::Copy,
            copy_dest: String::new(),
            copy_preserve: true,
            copy_conflict: Conflict::Rename,
            copy_rx: None,
            copy_handle: None,
            copy_progress: None,
            copy_errors: Vec::new(),
            copy_refresh_after: false,

            error_msg: None,
            notice: if just_updated {
                Some((
                    format!("✓ Update installiert — Version {}", env!("CARGO_PKG_VERSION")),
                    std::time::Instant::now(),
                ))
            } else {
                None
            },
            failed_paths: Vec::new(),
            show_errors_dialog: false,

            text_draft: String::new(),
            ext_draft: String::new(),
            size_min_draft: String::new(),
            size_max_draft: String::new(),
            filter_pending_at: None,

            mtime_min_date: None,
            mtime_max_date: None,
            btime_min_date: None,
            btime_max_date: None,

            drives,
            drive_info,
            home,
            recent,
            favorites,
            icon_cache: crate::icons::IconCache::new(),
            show_help: false,
            show_disclaimer: !appdata_file("disclaimer_ack.txt").exists(),
            last_view_recompute: Instant::now(),
            view_dirty: false,

            band_press: None,
            band_active: false,
            last_scroll_at: None,
            drag_files: Vec::new(),
            drag_active: false,
            drag_src: None,
            drag_source_tab: 0,
            drag_out_started: false,
            tab_header_rects: Vec::new(),
            pane_rects: Vec::new(),
            current_render_tab: 0,
            shown: false,
            band_base: HashSet::new(),
            band_suppressed: false,
            pending_scroll_row: None,

            type_jump: String::new(),
            type_jump_at: Instant::now(),

            rename_open: None,
            rename_focus: false,

            path_edit_mode: false,
            path_edit_focus: false,
            folder_search_focus: false,
            name_filter_focus: false,

            summary_cache: None,
            sel_size_cache: (usize::MAX, usize::MAX, 0),

            folder_index: load_folder_index_or_empty(),
            index_building: false,
            index_progress: 0,
            index_progress_path: String::new(),
            index_rx: None,
            index_cancel: None,
            folder_search_query: String::new(),
            folder_search_results: Vec::new(),
            folder_search_pending_at: None,
            folder_search_rx: None,
            folder_search_seq: 0,

            trash_rx: None,

            update_rx: Some(urx),
            update_ready: None,
            update_feed_draft: crate::updater::update_source_str().unwrap_or_default(),
            pending_initial_path: initial_path,
            #[cfg(windows)]
            integration_ctx_menu: crate::shell_register::context_menu_enabled(),
            #[cfg(not(windows))]
            integration_ctx_menu: false,

            #[cfg(windows)]
            clip_prepare_rx: None,
            #[cfg(windows)]
            virtual_clip: None,

            #[cfg(windows)]
            watcher: None,
            #[cfg(windows)]
            watcher_rx: None,
            index_dirty: false,
            index_last_saved: std::time::Instant::now(),

            clip_key_rx: None,
            clip_key_cancel: None,

            remote: None,
            net_conn: None,
            show_connect: false,
            connecting: false,
            connect_form: crate::connect::ConnectForm::default(),
            connect_rx: None,

            sync_rx: None,
            sync_running: false,

            saved_connections: crate::creds::load_connections(),

            bisync_rx: None,
            bisync_running: false,
            bisync_ctx: None,
            bisync_conflicts: Vec::new(),
            show_bisync_conflicts: false,
            sync_cancel: None,
            bisync_cancel: None,

            sync_jobs: crate::syncjobs::load(),
            show_sync_jobs: false,
            job_editor: None,
            running_job: None,

            picker: None,
            job_connect_rx: None,
            job_connect_pending: None,
            file_open_rx: Vec::new(),
            remote_open_mode: load_remote_open_mode(),
            remote_edits: Vec::new(),
            edit_save_rx: Vec::new(),
            last_edit_poll: Instant::now(),
            upload_rx: None,
            remote_op_rx: None,
            remote_ctx: None,
            clip_download_rx: None,

            cloud_client_id_draft: crate::cloud::load_config(crate::cloud::Provider::GDrive)
                .client_id,
            cloud_secret_draft: crate::cloud::load_config(crate::cloud::Provider::GDrive)
                .client_secret,
            cloud_auth_rx: None,
            cloud_authing: false,

            share: None,
            show_share: false,
            share_server: load_share_server(),
            share_server_draft: load_share_server(),
            share_device_draft: default_device_name(),
            share_code_input: String::new(),
            share_my_code: String::new(),
            share_room: false,
            share_roster: Vec::new(),
            share_incoming: Vec::new(),
            share_status: String::new(),
            share_progress: None,
            quickshare: None,
            qs_devices: Vec::new(),
        }
    }

    fn save_recent(&self) {
        let txt = self.recent.join("\n");
        let _ = std::fs::write(settings_path(), txt);
    }

    fn add_recent(&mut self, p: &str) {
        self.recent.retain(|x| x != p);
        self.recent.insert(0, p.to_string());
        self.recent.truncate(10);
        self.save_recent();
    }

    // ─── Favorites (starred folders) ────────────────────────────────────
    fn save_favorites(&self) {
        let _ = std::fs::write(favorites_path(), self.favorites.join("\n"));
    }

    fn is_favorite(&self, p: &str) -> bool {
        self.favorites.iter().any(|x| x == p)
    }

    /// Toggle a folder's starred state. Saves immediately — never deferred to
    /// on_exit (which clears state before any save could run).
    fn toggle_favorite(&mut self, p: &str) {
        if let Some(i) = self.favorites.iter().position(|x| x == p) {
            self.favorites.remove(i);
            self.notice = Some(("☆ Aus Favoriten entfernt".to_string(), std::time::Instant::now()));
        } else {
            self.favorites.insert(0, p.to_string());
            self.notice = Some(("★ Zu Favoriten hinzugefügt".to_string(), std::time::Instant::now()));
        }
        self.save_favorites();
    }

    fn save_ui_state(&self) {
        UiState {
            show_filters: self.show_filters,
            show_summary: self.show_summary,
        }
        .save();
    }

    fn root_prefix(&self) -> String {
        self.root_path.replace('\\', "/").trim_end_matches('/').to_string()
    }

    fn filter_is_active(&self) -> bool {
        let f = &self.filter;
        !f.text.is_empty()
            || !f.extensions.is_empty()
            || f.size.min.is_some()
            || f.size.max.is_some()
            || f.mtime.min.is_some()
            || f.mtime.max.is_some()
            || f.btime.min.is_some()
            || f.btime.max.is_some()
            || !f.include_files
            || !f.include_dirs
            || !f.include_hidden
            || !f.include_system
    }

    // ─── Tabs ────────────────────────────────────────────────────────────

    /// Exchange the App's working fields with the state parked in `tabs[i]`.
    fn swap_with_tab(&mut self, i: usize) {
        let mut t = std::mem::take(&mut self.tabs[i]);
        std::mem::swap(&mut t.root_path, &mut self.root_path);
        std::mem::swap(&mut t.entries, &mut self.entries);
        std::mem::swap(&mut t.view, &mut self.view);
        std::mem::swap(&mut t.selection, &mut self.selection);
        std::mem::swap(&mut t.last_anchor, &mut self.last_anchor);
        std::mem::swap(&mut t.cursor, &mut self.cursor);
        std::mem::swap(&mut t.scan_rx, &mut self.scan_rx);
        std::mem::swap(&mut t.scan_handle, &mut self.scan_handle);
        std::mem::swap(&mut t.progress, &mut self.progress);
        std::mem::swap(&mut t.scan_running, &mut self.scan_running);
        std::mem::swap(&mut t.history, &mut self.history);
        std::mem::swap(&mut t.forward, &mut self.forward);
        std::mem::swap(&mut t.failed_paths, &mut self.failed_paths);
        std::mem::swap(&mut t.view_dirty, &mut self.view_dirty);
        std::mem::swap(&mut t.remote, &mut self.remote);
        std::mem::swap(&mut t.net_conn, &mut self.net_conn);
        std::mem::swap(&mut t.filter, &mut self.filter);
        std::mem::swap(&mut t.sort_key, &mut self.sort_key);
        std::mem::swap(&mut t.sort_dir, &mut self.sort_dir);
        std::mem::swap(&mut t.text_draft, &mut self.text_draft);
        std::mem::swap(&mut t.ext_draft, &mut self.ext_draft);
        std::mem::swap(&mut t.size_min_draft, &mut self.size_min_draft);
        std::mem::swap(&mut t.size_max_draft, &mut self.size_max_draft);
        std::mem::swap(&mut t.filter_pending_at, &mut self.filter_pending_at);
        std::mem::swap(&mut t.mtime_min_date, &mut self.mtime_min_date);
        std::mem::swap(&mut t.mtime_max_date, &mut self.mtime_max_date);
        std::mem::swap(&mut t.btime_min_date, &mut self.btime_min_date);
        std::mem::swap(&mut t.btime_max_date, &mut self.btime_max_date);
        self.tabs[i] = t;
    }

    fn switch_tab(&mut self, to: usize) {
        if to == self.active_tab || to >= self.tabs.len() {
            return;
        }
        let from = self.active_tab;
        self.swap_with_tab(from);
        self.swap_with_tab(to);
        self.active_tab = to;
        self.band_press = None;
        self.band_active = false;
        self.summary_cache = None;
        self.sel_size_cache = (usize::MAX, usize::MAX, 0);
        if self.view_dirty {
            self.recompute_view();
        }
    }

    /// Toggle split-screen (two tabs side by side). Enabling guarantees a
    /// second tab exists (cloning the current location) without moving focus.
    fn toggle_split(&mut self) {
        if self.split {
            self.split = false;
            return;
        }
        if self.tabs.len() < 2 {
            let cur = self.root_path.clone();
            self.tabs.push(TabState::default());
            let new_idx = self.tabs.len() - 1;
            let prev = self.active_tab;
            self.switch_tab(new_idx);
            let target = if cur.is_empty() {
                self.home.clone()
            } else {
                PathBuf::from(cur.replace('/', std::path::MAIN_SEPARATOR_STR))
            };
            self.start_scan_navigated(target, false);
            self.switch_tab(prev);
        }
        let other = (0..self.tabs.len())
            .find(|&i| i != self.active_tab)
            .unwrap_or(self.active_tab);
        self.panes = [self.active_tab, other];
        self.split = true;
    }

    /// Render the central area: a single table, or two side-by-side panes in
    /// split mode. Each pane renders via `ui_table`; the non-focused pane's
    /// tab state is swapped into the working fields just for its render.
    fn ui_central(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if !self.split || self.tabs.len() < 2 {
                self.split = self.split && self.tabs.len() >= 2;
                self.pane_rects.clear();
                self.current_render_tab = self.active_tab;
                self.ui_table(ui);
                return;
            }
            let n = self.tabs.len();
            // Keep pane indices valid and ensure the focused tab is shown.
            for p in self.panes.iter_mut() {
                if *p >= n {
                    *p = 0;
                }
            }
            if self.panes[0] != self.active_tab && self.panes[1] != self.active_tab {
                self.panes[0] = self.active_tab;
            }
            if self.panes[0] == self.panes[1] {
                self.panes[1] = (0..n).find(|&i| i != self.panes[0]).unwrap_or(self.panes[0]);
            }
            let panes = self.panes;
            let mut focus_to: Option<usize> = None;
            // Set by either pane's header right-click → run after the loop to
            // avoid borrowing self while rendering.
            let mut sync_panes_req = false;
            let mut save_setup_req = false;

            // Manual two-pane split with hard clipping per pane — egui's
            // `columns` doesn't clip, so the wide table bled into the other
            // pane. Each pane gets its own rect, a clip rect, and there's a
            // visible vertical divider between them.
            let full = ui.available_rect_before_wrap();
            let gap = 9.0;
            let half = ((full.width() - gap) / 2.0).max(80.0);
            let rects = [
                egui::Rect::from_min_size(full.min, egui::vec2(half, full.height())),
                egui::Rect::from_min_size(
                    egui::pos2(full.min.x + half + gap, full.min.y),
                    egui::vec2(half, full.height()),
                ),
            ];
            let sep_x = full.min.x + half + gap / 2.0;
            ui.painter().vline(
                sep_x,
                full.min.y..=full.max.y,
                egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.fg_stroke.color),
            );
            // Remember each pane's rect (+ its tab) so a drag can drop onto the
            // other pane, not just the tab header.
            self.pane_rects = vec![(panes[0], rects[0]), (panes[1], rects[1])];

            for (slot, &rect) in rects.iter().enumerate() {
                let tab_idx = panes[slot];
                let focused = tab_idx == self.active_tab;
                ui.allocate_ui_at_rect(rect, |ui| {
                    ui.set_clip_rect(rect); // <- prevents the table from overflowing the pane
                    ui.push_id(("pane", tab_idx), |ui| {
                        let title = self.tab_title(tab_idx);
                        ui.horizontal(|ui| {
                            let resp = if focused {
                                ui.label(RichText::new(format!("● {}", title)).strong())
                            } else {
                                ui.label(
                                    RichText::new(format!("○ {}", title))
                                        .color(Color32::from_gray(150)),
                                )
                            };
                            // Right-click either pane header → sync the two open
                            // folders (the split-view sync the user asked for).
                            resp.context_menu(|ui| {
                                if ui.button("⇄ Diese beiden Ordner synchronisieren").clicked() {
                                    sync_panes_req = true;
                                    ui.close_menu();
                                }
                                if ui.button("＋ Als Sync-Setup speichern…").clicked() {
                                    save_setup_req = true;
                                    ui.close_menu();
                                }
                            });
                        });
                        ui.separator();
                        if focused {
                            self.current_render_tab = tab_idx;
                            self.ui_pane_search(ui);
                            self.ui_table(ui);
                        } else {
                            self.swap_with_tab(tab_idx);
                            self.current_render_tab = tab_idx;
                            self.ui_pane_search(ui);
                            self.band_suppressed = true; // band belongs to the focused pane
                            self.ui_table(ui);
                            self.band_suppressed = false;
                            self.swap_with_tab(tab_idx);
                            // Click anywhere in this pane focuses it.
                            let pressed = ui.input(|i| i.pointer.any_pressed());
                            if pressed {
                                if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                                    if rect.contains(pos) {
                                        focus_to = Some(tab_idx);
                                    }
                                }
                            }
                        }
                    });
                });
            }
            if let Some(t) = focus_to {
                self.switch_tab(t);
            }
            if sync_panes_req {
                self.sync_split_panes();
            }
            if save_setup_req {
                let (_, root_a) = self.pane_backend(panes[0]);
                let (_, root_b) = self.pane_backend(panes[1]);
                self.job_editor = Some(JobEditor::blank(root_a, root_b));
                self.show_sync_jobs = true;
            }
        });
    }

    fn new_tab(&mut self) {
        let cur = self.root_path.clone();
        // A fresh tab has no backend; if the current tab is remote, open the new
        // one at a LOCAL default instead of the (unreachable-without-backend)
        // remote path. The current tab's connection is parked with its TabState
        // by switch_tab and is unaffected.
        let cur_is_remote = self.remote.is_some();
        self.tabs.push(TabState::default());
        let idx = self.tabs.len() - 1;
        self.switch_tab(idx);
        let target = if cur.is_empty() || cur_is_remote {
            self.home.clone()
        } else {
            PathBuf::from(cur.replace('/', std::path::MAIN_SEPARATOR_STR))
        };
        self.start_scan_navigated(target, false);
    }

    fn close_tab(&mut self, i: usize) {
        if self.tabs.len() <= 1 || i >= self.tabs.len() {
            return;
        }
        if i == self.active_tab {
            let to = if i + 1 < self.tabs.len() { i + 1 } else { i - 1 };
            self.switch_tab(to);
        }
        let t = self.tabs.remove(i);
        if let Some(h) = t.scan_handle {
            h.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if self.active_tab > i {
            self.active_tab -= 1;
        }
    }

    /// Compact per-pane name filter/search, shown at the top of each split pane
    /// so the two panes filter independently. Operates on the currently
    /// swapped-in tab's filter (each pane is rendered inside its own swap), and
    /// commits + recomputes immediately for that pane.
    fn ui_pane_search(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("🔍");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.text_draft)
                    .hint_text("Filtern (Name/Regex/Glob)…")
                    .desired_width(f32::INFINITY),
            );
            if resp.changed() {
                self.filter.text = self.text_draft.clone();
                self.recompute_view();
            }
            // Cycle the match mode (substring → regex → glob) so each pane can
            // choose its own.
            let mode_label = match self.filter.text_mode {
                crate::types::TextMode::Substring => "abc",
                crate::types::TextMode::Regex => ".*",
                crate::types::TextMode::Glob => "*?",
            };
            if ui.small_button(mode_label).on_hover_text("Modus: Text / Regex / Glob").clicked() {
                self.filter.text_mode = match self.filter.text_mode {
                    crate::types::TextMode::Substring => crate::types::TextMode::Regex,
                    crate::types::TextMode::Regex => crate::types::TextMode::Glob,
                    crate::types::TextMode::Glob => crate::types::TextMode::Substring,
                };
                self.recompute_view();
            }
            if !self.text_draft.is_empty() && ui.small_button("×").on_hover_text("Filter löschen").clicked() {
                self.text_draft.clear();
                self.filter.text.clear();
                self.recompute_view();
            }
        });
    }

    fn tab_title(&self, i: usize) -> String {
        // Per-tab path + connection (active tab's live in the App fields).
        let (p, remote_label, is_share) = if i == self.active_tab {
            (
                &self.root_path,
                self.remote.as_ref().map(|r| r.label.as_str()),
                self.net_conn.is_some(),
            )
        } else {
            let t = &self.tabs[i];
            (
                &t.root_path,
                t.remote.as_ref().map(|r| r.label.as_str()),
                t.net_conn.is_some(),
            )
        };
        if p.is_empty() && remote_label.is_none() {
            return "Neuer Tab".to_string();
        }
        let t = p.trim_end_matches('/');
        let base = t.rsplit('/').next().unwrap_or(t);
        let base = if base.is_empty() { t } else { base };

        // Remote/share tabs get a marker + the connection name, so they're
        // identifiable (the bare folder name isn't enough).
        let title = if let Some(label) = remote_label {
            // "sftp://user@host:port" -> "user@host:port"
            let host = label.split("://").nth(1).unwrap_or(label);
            format!("🌐 {host} · {base}")
        } else if is_share {
            format!("🖧 {base}")
        } else {
            base.to_string()
        };

        if title.chars().count() > 24 {
            let mut out: String = title.chars().take(23).collect();
            out.push('…');
            out
        } else {
            title
        }
    }

    fn ui_tabbar(&mut self, ui: &mut egui::Ui) {
        enum TabAction {
            Switch(usize),
            Close(usize),
            New,
        }
        let mut action: Option<TabAction> = None;
        let dragging = self.drag_active;
        let mut header_rects: Vec<(usize, egui::Rect)> = Vec::new();
        ui.horizontal(|ui| {
            for i in 0..self.tabs.len() {
                let selected = i == self.active_tab;
                let title = self.tab_title(i);
                let resp = ui.selectable_label(selected, title);
                header_rects.push((i, resp.rect));
                // Highlight a tab as a drop target while files are being dragged
                // from another tab.
                if dragging && i != self.drag_source_tab && resp.hovered() {
                    ui.painter().rect_stroke(
                        resp.rect.expand(1.0),
                        3.0,
                        egui::Stroke::new(2.0, Color32::from_rgb(120, 200, 255)),
                    );
                }
                if resp.clicked() && !selected {
                    action = Some(TabAction::Switch(i));
                }
                if resp.middle_clicked() {
                    action = Some(TabAction::Close(i));
                }
                if selected && self.tabs.len() > 1 {
                    if ui
                        .small_button("✕")
                        .on_hover_text("Tab schließen (Ctrl+W)")
                        .clicked()
                    {
                        action = Some(TabAction::Close(i));
                    }
                }
            }
            if ui
                .button("＋")
                .on_hover_text("Neuer Tab (Ctrl+T)")
                .clicked()
            {
                action = Some(TabAction::New);
            }
        });
        self.tab_header_rects = header_rects;
        match action {
            Some(TabAction::Switch(i)) => self.switch_tab(i),
            Some(TabAction::Close(i)) => self.close_tab(i),
            Some(TabAction::New) => self.new_tab(),
            None => {}
        }
    }

    // ─── Scanning / navigation ──────────────────────────────────────────

    fn start_scan(&mut self, root: PathBuf) {
        self.start_scan_navigated(root, true);
    }

    fn start_scan_navigated(&mut self, root: PathBuf, record_history: bool) {
        if let Some(h) = self.scan_handle.take() {
            h.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }

        if record_history && !self.root_path.is_empty() {
            self.history.push(self.root_path.clone());
            self.forward.clear();
            if self.history.len() > 100 {
                self.history.remove(0);
            }
        }

        // Replace (not clear) to actually release the backing allocation.
        self.entries = Vec::new();
        self.view = Vec::new();
        self.selection = HashSet::new();
        self.last_anchor = None;
        self.cursor = None;
        self.progress = empty_progress();
        self.error_msg = None;
        self.failed_paths = Vec::new();
        self.summary_cache = None;
        self.view_dirty = false;
        self.band_press = None;
        self.band_active = false;
        // Opening a folder clears the NAME search so the new folder is fully
        // visible; other filters (type/size/date/ext) are kept on purpose.
        self.filter.text.clear();
        self.text_draft.clear();
        self.root_path = root.to_string_lossy().replace('\\', "/");

        let (tx, rx) = unbounded();
        let max_depth = if self.recursive { None } else { Some(1) };
        // Route remote roots through the backend walk; local roots (incl. drive
        // letters and UNC) keep the fast std::fs path. Decided centrally here by
        // path style, so every navigation entry point is handled without edits:
        // an active remote session stays remote as long as the target isn't a
        // local-style path; otherwise we drop back to local.
        let stay_remote = self.remote.is_some() && !is_local_style(&self.root_path);
        if !stay_remote {
            self.remote = None;
            // "Recent" is for local quick-access; remote locations live in the
            // saved-connections list instead (a remote path would fail a later
            // local scan).
            self.add_recent(&self.root_path.clone());
        }
        let handle = match self.remote.as_ref() {
            Some(rs) => crate::rscan::start_scan_backend(
                rs.backend.clone(),
                self.root_path.clone(),
                max_depth,
                tx,
            ),
            None => start_scan(root, false, max_depth, tx),
        };
        self.scan_rx = Some(rx);
        self.scan_handle = Some(handle);
        self.scan_running = true;
    }

    fn navigate_up(&mut self) {
        if self.root_path.is_empty() {
            return;
        }
        let p = PathBuf::from(self.root_path.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(parent) = p.parent() {
            let parent_buf = parent.to_path_buf();
            if !parent_buf.as_os_str().is_empty() {
                self.start_scan(parent_buf);
            }
        }
    }

    fn navigate_back(&mut self) {
        if let Some(prev) = self.history.pop() {
            self.forward.push(self.root_path.clone());
            let p = PathBuf::from(prev.replace('/', std::path::MAIN_SEPARATOR_STR));
            self.start_scan_navigated(p, false);
        }
    }

    fn navigate_forward(&mut self) {
        if let Some(next) = self.forward.pop() {
            self.history.push(self.root_path.clone());
            let p = PathBuf::from(next.replace('/', std::path::MAIN_SEPARATOR_STR));
            self.start_scan_navigated(p, false);
        }
    }

    fn rescan(&mut self) {
        if self.root_path.is_empty() {
            return;
        }
        let p = PathBuf::from(self.root_path.replace('/', std::path::MAIN_SEPARATOR_STR));
        self.start_scan_navigated(p, false);
    }

    fn cancel_scan(&mut self) {
        if let Some(h) = self.scan_handle.take() {
            h.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.scan_running = false;
    }

    // ─── Folder index lifecycle ─────────────────────────────────────────
    fn start_index_build(&mut self) {
        if self.index_building {
            return;
        }
        if let Some(c) = self.index_cancel.take() {
            c.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.index_building = true;
        self.index_progress = 0;
        self.index_progress_path = String::new();
        let roots: Vec<PathBuf> = if self.drives.is_empty() {
            vec![self.home.clone()]
        } else {
            self.drives.iter().map(PathBuf::from).collect()
        };
        let (tx, rx) = crossbeam_channel::unbounded();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        FolderIndex::build_async(roots, tx, cancel.clone());
        self.index_rx = Some(rx);
        self.index_cancel = Some(cancel);
    }

    fn cancel_index_build(&mut self) {
        if let Some(c) = self.index_cancel.take() {
            c.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.index_building = false;
        self.index_rx = None;
    }

    fn drain_index(&mut self) {
        let rx = match self.index_rx.as_ref() {
            Some(r) => r,
            None => return,
        };
        for _ in 0..16 {
            match rx.try_recv() {
                Ok(IndexMsg::Progress { count, current }) => {
                    self.index_progress = count;
                    self.index_progress_path = current;
                }
                Ok(IndexMsg::Done(idx)) => {
                    let _ = idx.save(&folder_index_path());
                    self.folder_index = idx;
                    self.index_building = false;
                    self.index_rx = None;
                    self.index_cancel = None;
                    if !self.folder_search_query.is_empty() {
                        self.run_folder_search();
                    }
                    self.notice = Some((
                        format!("✓ Index gebaut: {} Ordner", self.folder_index.len()),
                        std::time::Instant::now(),
                    ));
                    break;
                }
                Ok(IndexMsg::Error(e)) => {
                    self.error_msg = Some(e);
                    self.index_building = false;
                    self.index_rx = None;
                    break;
                }
                Err(_) => break,
            }
        }
    }

    /// Two-stage search: fuzzy scoring runs synchronously (pure CPU, fast),
    /// then a background thread stats the candidates and re-ranks by mtime —
    /// disk I/O never blocks the UI thread.
    fn run_folder_search(&mut self) {
        if self.folder_search_query.is_empty() || self.folder_index.is_empty() {
            self.folder_search_results.clear();
            self.folder_search_rx = None;
            return;
        }
        let scored = self.folder_index.search_scored(&self.folder_search_query, 90);
        // Provisional, score-only results shown immediately
        self.folder_search_results = scored.iter().take(30).cloned().collect();
        self.folder_search_seq += 1;
        let seq = self.folder_search_seq;
        let (tx, rx) = unbounded();
        self.folder_search_rx = Some(rx);
        std::thread::Builder::new()
            .name("search-rank".into())
            .spawn(move || {
                let ranked = crate::folder_index::stat_and_rank(scored, 30);
                let _ = tx.send((seq, ranked));
            })
            .ok();
    }

    fn drain_folder_search(&mut self) {
        let mut done = false;
        if let Some(rx) = self.folder_search_rx.as_ref() {
            while let Ok((seq, ranked)) = rx.try_recv() {
                if seq == self.folder_search_seq {
                    self.folder_search_results = ranked;
                    done = true;
                }
            }
        }
        if done {
            self.folder_search_rx = None;
        }
    }

    // ─── Background clipboard-key poller ────────────────────────────────
    // egui consumes Ctrl+C/X/V for its own text clipboard and, for a file
    // (CF_HDROP, no text) clipboard, emits NO paste event and triggers NO
    // repaint when idle — so update() never runs on the keypress and any
    // in-frame key poll is dead. A separate thread polls the real OS key
    // state ~30×/s, fires only when OUR window is the foreground window, and
    // wakes the UI via ctx.request_repaint().
    #[cfg(windows)]
    fn start_clip_key_poller(&mut self, ctx: &egui::Context) {
        use std::sync::atomic::{AtomicBool, Ordering};
        let (tx, rx) = crossbeam_channel::unbounded::<ClipKey>();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_t = cancel.clone();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("clip-keys".into())
            .spawn(move || {
                use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
                let down = |vk: i32| -> bool {
                    (unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000) != 0
                };
                let mut prev = [false; 3]; // C, X, V
                while !cancel_t.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(30));
                    let fg = app_is_foreground();
                    let ctrl = down(0x11); // VK_CONTROL
                    let shift = down(0x10); // VK_SHIFT
                    let cur = [down(0x43), down(0x58), down(0x56)]; // 'C','X','V'
                    for idx in 0..3 {
                        let just_pressed = cur[idx] && !prev[idx];
                        prev[idx] = cur[idx];
                        if !(just_pressed && ctrl && fg) {
                            continue;
                        }
                        let action = match idx {
                            0 if !shift => ClipKey::Copy, // Ctrl+Shift+C handled in-frame
                            0 => continue,
                            1 => ClipKey::Cut,
                            _ => ClipKey::Paste,
                        };
                        if tx.send(action).is_err() {
                            return;
                        }
                        ctx.request_repaint();
                    }
                }
            })
            .ok();
        self.clip_key_rx = Some(rx);
        self.clip_key_cancel = Some(cancel);
    }

    #[cfg(not(windows))]
    fn start_clip_key_poller(&mut self, _ctx: &egui::Context) {}

    // ─── Filesystem watcher for live index updates ──────────────────────
    #[cfg(windows)]
    fn start_watcher(&mut self) {
        use notify::{RecursiveMode, Watcher};
        self.watcher = None;
        self.watcher_rx = None;

        let (tx, rx) = crossbeam_channel::unbounded();
        let mut watcher = match notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        }) {
            Ok(w) => w,
            Err(e) => {
                self.error_msg = Some(format!("Watcher: {}", e));
                return;
            }
        };
        let roots: Vec<PathBuf> = if self.drives.is_empty() {
            vec![self.home.clone()]
        } else {
            self.drives.iter().map(PathBuf::from).collect()
        };
        for root in &roots {
            if let Err(e) = watcher.watch(root, RecursiveMode::Recursive) {
                eprintln!("watch failed for {}: {}", root.display(), e);
            }
        }
        self.watcher = Some(watcher);
        self.watcher_rx = Some(rx);
    }

    /// Drain pending watcher events in a single pass. Coalesces removes and
    /// renames so the worst case is O(N + K) over the index instead of
    /// O(N · K).
    #[cfg(windows)]
    fn drain_watcher(&mut self) {
        use notify::event::{CreateKind, EventKind, ModifyKind, RemoveKind, RenameMode};

        let mut events: Vec<notify::Event> = Vec::new();
        if let Some(rx) = self.watcher_rx.as_ref() {
            for _ in 0..8000 {
                match rx.try_recv() {
                    Ok(Ok(e)) => events.push(e),
                    Ok(Err(_)) | Err(_) => break,
                }
            }
        }
        if events.is_empty() {
            return;
        }

        let normalize = |p: &std::path::Path| -> String { p.to_string_lossy().replace('\\', "/") };
        let allowed = |path: &std::path::Path| -> bool {
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if crate::folder_index::should_skip(&name) {
                return false;
            }
            let s = path.to_string_lossy().replace('\\', "/");
            !crate::folder_index::path_has_skipped_segment(&s)
        };

        let mut additions: Vec<String> = Vec::new();
        let mut remove_subtrees: Vec<String> = Vec::new();
        let mut rename_subtrees: Vec<(String, String)> = Vec::new();

        for event in events {
            match event.kind {
                EventKind::Create(kind) => {
                    let assume_folder = matches!(kind, CreateKind::Folder);
                    let want_stat = matches!(kind, CreateKind::Any);
                    for p in &event.paths {
                        if !allowed(p) {
                            continue;
                        }
                        let is_dir = if assume_folder {
                            true
                        } else if want_stat {
                            std::fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false)
                        } else {
                            false
                        };
                        if is_dir {
                            additions.push(normalize(p));
                        }
                    }
                }
                EventKind::Remove(kind) => {
                    let assume_or_unknown = matches!(kind, RemoveKind::Folder | RemoveKind::Any);
                    if assume_or_unknown {
                        for p in &event.paths {
                            remove_subtrees.push(normalize(p));
                        }
                    }
                }
                EventKind::Modify(ModifyKind::Name(mode)) => match mode {
                    RenameMode::Both => {
                        if event.paths.len() == 2 {
                            rename_subtrees
                                .push((normalize(&event.paths[0]), normalize(&event.paths[1])));
                        }
                    }
                    RenameMode::From => {
                        for p in &event.paths {
                            remove_subtrees.push(normalize(p));
                        }
                    }
                    RenameMode::To => {
                        for p in &event.paths {
                            if !allowed(p) {
                                continue;
                            }
                            if std::fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false) {
                                additions.push(normalize(p));
                            }
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        let dirty = self.apply_batched_changes(&additions, &remove_subtrees, &rename_subtrees);
        if dirty {
            self.index_dirty = true;
            if !self.folder_search_query.is_empty() {
                self.run_folder_search();
            }
        }
    }

    /// One-pass batched mutation. Collects only the affected paths instead of
    /// cloning the whole index (the previous version cloned every path on any
    /// remove/rename burst).
    #[cfg(windows)]
    fn apply_batched_changes(
        &mut self,
        additions: &[String],
        remove_subtrees: &[String],
        rename_subtrees: &[(String, String)],
    ) -> bool {
        if additions.is_empty() && remove_subtrees.is_empty() && rename_subtrees.is_empty() {
            return false;
        }

        let mut dirty = false;

        if !remove_subtrees.is_empty() || !rename_subtrees.is_empty() {
            let remove_prefixes: Vec<String> =
                remove_subtrees.iter().map(|p| format!("{}/", p)).collect();
            let rename_prefixes: Vec<(String, String)> = rename_subtrees
                .iter()
                .map(|(old, new)| (format!("{}/", old), format!("{}/", new)))
                .collect();
            let remove_exact: std::collections::HashSet<&str> =
                remove_subtrees.iter().map(|s| s.as_str()).collect();

            let mut removes_to_apply: Vec<String> = Vec::new();
            let mut renames_to_apply: Vec<(String, String)> = Vec::new();

            for p in self.folder_index.iter() {
                if remove_exact.contains(p.as_str())
                    || remove_prefixes.iter().any(|pref| p.starts_with(pref.as_str()))
                {
                    removes_to_apply.push(p.clone());
                    continue;
                }
                let mut renamed: Option<String> = None;
                for (old, new) in rename_subtrees {
                    if p == old {
                        renamed = Some(new.clone());
                        break;
                    }
                }
                if renamed.is_none() {
                    for (old_pref, new_pref) in &rename_prefixes {
                        if p.starts_with(old_pref.as_str()) {
                            renamed = Some(format!("{}{}", new_pref, &p[old_pref.len()..]));
                            break;
                        }
                    }
                }
                if let Some(r) = renamed {
                    renames_to_apply.push((p.clone(), r));
                }
            }

            for r in &removes_to_apply {
                if self.folder_index.remove(r) {
                    dirty = true;
                }
            }
            for (old, new) in &renames_to_apply {
                self.folder_index.remove(old);
                dirty = true;
                if !crate::folder_index::path_has_skipped_segment(new) {
                    self.folder_index.insert(new.clone());
                }
            }
        }

        for p in additions {
            if self.folder_index.insert(p.clone()) {
                dirty = true;
            }
        }
        dirty
    }

    #[cfg(not(windows))]
    fn start_watcher(&mut self) {}
    #[cfg(not(windows))]
    fn drain_watcher(&mut self) {}

    fn maybe_save_index(&mut self) {
        if !self.index_dirty || self.index_last_saved.elapsed().as_secs() < 30 {
            return;
        }
        let mut buf = String::with_capacity(self.folder_index.len() * 50);
        for p in self.folder_index.iter() {
            buf.push_str(p);
            buf.push('\n');
        }
        let target = folder_index_path();
        std::thread::Builder::new()
            .name("index-save".into())
            .spawn(move || {
                let tmp = target.with_extension("txt.tmp");
                if std::fs::write(&tmp, buf).is_ok() {
                    let _ = std::fs::rename(&tmp, &target);
                }
            })
            .ok();
        self.index_dirty = false;
        self.index_last_saved = std::time::Instant::now();
    }

    // ─── Channel drains ─────────────────────────────────────────────────

    fn drain_scan(&mut self) {
        let rx = match self.scan_rx.take() {
            Some(r) => r,
            None => return,
        };
        let (got_entries, got_done) = drain_scan_channel(
            &rx,
            &mut self.entries,
            &mut self.progress,
            &mut self.failed_paths,
            &mut self.error_msg,
        );
        if got_done {
            self.scan_handle = None;
            self.scan_running = false;
            self.recompute_view();
        } else {
            self.scan_rx = Some(rx);
            if got_entries {
                self.view_dirty = true;
            }
        }
    }

    /// Keep background tabs' scans flowing so their channels don't pile up
    /// unboundedly; their views are rebuilt lazily on activation.
    fn drain_inactive_tabs(&mut self) {
        let active = self.active_tab;
        for (i, t) in self.tabs.iter_mut().enumerate() {
            if i == active {
                continue;
            }
            if let Some(rx) = t.scan_rx.take() {
                let mut err = None;
                let (got_entries, got_done) = drain_scan_channel(
                    &rx,
                    &mut t.entries,
                    &mut t.progress,
                    &mut t.failed_paths,
                    &mut err,
                );
                if got_done {
                    t.scan_handle = None;
                    t.scan_running = false;
                    t.view_dirty = true;
                } else {
                    t.scan_rx = Some(rx);
                    if got_entries {
                        t.view_dirty = true;
                    }
                }
            }
        }
    }

    fn drain_copy(&mut self) {
        let rx = match self.copy_rx.as_ref() {
            Some(r) => r,
            None => return,
        };
        let mut done = false;
        for _ in 0..16 {
            match rx.try_recv() {
                Ok(CopyMsg::Progress(p)) => self.copy_progress = Some(p),
                Ok(CopyMsg::Done { progress, errors }) => {
                    self.copy_progress = Some(progress);
                    self.copy_errors = errors;
                    done = true;
                    break;
                }
                Err(_) => break,
            }
        }
        if done {
            self.copy_rx = None;
            self.copy_handle = None;
            if !self.copy_errors.is_empty() {
                self.error_msg = Some(format!(
                    "{} Fehler beim Kopieren — erste: {}",
                    self.copy_errors.len(),
                    self.copy_errors
                        .first()
                        .map(|(p, m)| format!("{} ({})", p, m))
                        .unwrap_or_default()
                ));
            }
            if self.copy_refresh_after {
                self.copy_refresh_after = false;
                self.rescan();
            }
        }
    }

    fn drain_trash(&mut self) {
        let mut msg: Option<Option<String>> = None;
        if let Some(rx) = self.trash_rx.as_ref() {
            if let Ok(m) = rx.try_recv() {
                msg = Some(m);
            }
        }
        if let Some(m) = msg {
            self.trash_rx = None;
            match m {
                None => {
                    self.notice = Some((
                        "✓ In Papierkorb verschoben".to_string(),
                        std::time::Instant::now(),
                    ));
                }
                Some(e) => {
                    self.error_msg = Some(format!("Papierkorb: {}", e));
                    // State may be out of sync with disk — refresh.
                    self.rescan();
                }
            }
        }
    }

    #[cfg(windows)]
    fn drain_clip_prepare(&mut self) {
        let mut files = None;
        if let Some(rx) = self.clip_prepare_rx.as_ref() {
            if let Ok(f) = rx.try_recv() {
                files = Some(f);
            }
        }
        let files = match files {
            Some(f) => f,
            None => return,
        };
        self.clip_prepare_rx = None;
        if files.is_empty() {
            self.notice = Some((
                "Keine Dateien entsprechen dem aktiven Filter".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        let pairs: Vec<(String, String)> =
            files.iter().map(|f| (f.abs.clone(), f.rel.clone())).collect();
        let n = files.len();
        match crate::virtual_clipboard::set_clipboard(files) {
            Ok(seq) => {
                self.virtual_clip = Some((seq, pairs));
                self.notice = Some((
                    format!(
                        "✓ {} gefilterte Datei(en) kopiert — Einfügen (auch im Explorer) erhält die Ordnerstruktur",
                        n
                    ),
                    std::time::Instant::now(),
                ));
            }
            Err(e) => {
                self.error_msg = Some(format!("Zwischenablage: {}", e));
            }
        }
    }

    #[cfg(not(windows))]
    fn drain_clip_prepare(&mut self) {}

    fn drain_update(&mut self) {
        use crate::updater::UpdateMsg;
        let mut msg = None;
        if let Some(rx) = self.update_rx.as_ref() {
            if let Ok(m) = rx.try_recv() {
                msg = Some(m);
            }
        }
        let msg = match msg {
            Some(m) => m,
            None => return,
        };
        self.update_rx = None;
        match msg {
            UpdateMsg::Applied { version, exe } => {
                self.notice = Some((
                    format!("⬆ Update auf v{} bereit", version),
                    std::time::Instant::now(),
                ));
                self.update_ready = Some((version, exe));
            }
            UpdateMsg::UpToDate { feed_version } => {
                self.notice = Some((
                    format!(
                        "✓ Aktuell: v{} (Feed: v{})",
                        env!("CARGO_PKG_VERSION"),
                        feed_version
                    ),
                    std::time::Instant::now(),
                ));
            }
            UpdateMsg::NoFeed => {
                self.notice = Some((
                    "Kein Update-Feed konfiguriert (Pfad unten eintragen)".to_string(),
                    std::time::Instant::now(),
                ));
            }
            UpdateMsg::Error(e) => {
                self.error_msg = Some(format!("Update: {}", e));
            }
        }
    }

    fn check_updates_manual(&mut self) {
        let (tx, rx) = unbounded();
        self.update_rx = Some(rx);
        crate::updater::check_async(tx, true);
    }

    // ─── Remote connections ─────────────────────────────────────────────

    /// Start connecting with the current form (off the UI thread).
    fn begin_connect(&mut self, form: crate::connect::ConnectForm, secret: Option<String>) {
        self.connecting = true;
        self.error_msg = None;
        self.connect_rx = Some(crate::connect::spawn_connect(form, secret));
    }

    /// Connect to a saved connection: pre-fill from metadata + load its secret.
    fn connect_saved(&mut self, c: &crate::creds::SavedConnection) {
        let form = crate::connect::ConnectForm::from_saved(c);
        let secret = crate::creds::get_secret(&c.account());
        // Bump to most-recent so the sidebar keeps the freshest connections up
        // front and overflows the stale ones into the menu.
        crate::creds::touch_connection(&c.account());
        self.saved_connections = crate::creds::load_connections();
        self.begin_connect(form, secret);
    }

    fn drain_connect(&mut self) {
        let msg = match self.connect_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            Some(m) => m,
            None => return,
        };
        self.connect_rx = None;
        self.connecting = false;
        match msg {
            crate::connect::ConnectResult::Ok(c) => {
                // SFTP/FTP set a remote backend; a share clears it (browsed
                // locally) but keeps the auth connection alive.
                self.remote = c.remote;
                if let Some(nc) = c.net {
                    self.net_conn = Some(nc);
                }
                self.show_connect = false;
                // A "save" during connect wrote connections.txt on the worker
                // thread; refresh the cached list so it shows immediately.
                self.saved_connections = crate::creds::load_connections();
                self.notice = Some((
                    format!("✓ Verbunden: {}", c.label),
                    std::time::Instant::now(),
                ));
                let pb = PathBuf::from(c.target.replace('/', std::path::MAIN_SEPARATOR_STR));
                self.start_scan(pb);
            }
            crate::connect::ConnectResult::Err(e) => {
                self.error_msg = Some(format!("Verbindung fehlgeschlagen: {}", e));
            }
        }
    }

    /// One-way mirror the current location (local or remote) into `dest_local`.
    fn start_mirror(&mut self, dest_local: String) {
        if self.root_path.is_empty() || self.sync_running {
            return;
        }
        let src: crate::vfs::BackendHandle = match &self.remote {
            Some(rs) => rs.backend.clone(),
            None => Arc::new(crate::vfs::LocalBackend::new(&self.root_path)),
        };
        let dst: crate::vfs::BackendHandle = Arc::new(crate::vfs::LocalBackend::new(&dest_local));
        let (tx, rx) = unbounded();
        let h = crate::sync::start_sync(
            src,
            self.root_path.clone(),
            dst,
            dest_local,
            crate::sync::SyncOptions {
                delete_extra: false,
                dry_run: false,
            },
            tx,
        );
        self.sync_cancel = Some(h.cancel);
        self.sync_rx = Some(rx);
        self.sync_running = true;
        self.notice = Some(("⇅ Spiegelung gestartet…".to_string(), std::time::Instant::now()));
    }

    fn drain_sync(&mut self) {
        let msg = match self.sync_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            Some(m) => m,
            None => return,
        };
        match msg {
            crate::sync::SyncMsg::Progress(_) => {}
            crate::sync::SyncMsg::Done(r) => {
                self.sync_rx = None;
                self.sync_running = false;
                self.sync_cancel = None;
                if r.stats.errors > 0 {
                    self.error_msg = Some(format!(
                        "Spiegelung: {} kopiert, {} Fehler",
                        r.stats.copied, r.stats.errors
                    ));
                }
                self.notice = Some((
                    format!(
                        "✓ Spiegelung fertig: {} kopiert, {} übersprungen ({} MB)",
                        r.stats.copied,
                        r.stats.skipped,
                        r.stats.bytes / 1_048_576
                    ),
                    std::time::Instant::now(),
                ));
            }
        }
    }

    /// Two-way sync the current location with `dest_local` (safe defaults: both
    /// directions, strict file-level conflicts, reversible, 30-day version
    /// retention). Conflicts come back for resolution.
    fn start_bisync(&mut self, dest_local: String) {
        if self.root_path.is_empty() {
            return;
        }
        let a: crate::vfs::BackendHandle = match &self.remote {
            Some(rs) => rs.backend.clone(),
            None => Arc::new(crate::vfs::LocalBackend::new(&self.root_path)),
        };
        let root_a = self.root_path.clone();
        let b: crate::vfs::BackendHandle = Arc::new(crate::vfs::LocalBackend::new(&dest_local));
        self.launch_bisync(
            a,
            root_a,
            b,
            dest_local,
            crate::bisync::BisyncOptions::default(),
            30,
            true,
            Vec::new(),
            None,
        );
    }

    /// The single two-way-sync launcher used by the ad-hoc button, saved jobs,
    /// and the split-view "sync these two folders" action. Builds the ignore
    /// globset inside the worker (GlobSet isn't `Send`-cheap to pass), runs
    /// `bisync::run`, and stamps `running_job` so completion can mark the job.
    #[allow(clippy::too_many_arguments)]
    fn launch_bisync(
        &mut self,
        a: crate::vfs::BackendHandle,
        root_a: String,
        b: crate::vfs::BackendHandle,
        root_b: String,
        opts: crate::bisync::BisyncOptions,
        retain_days: u64,
        include_hidden: bool,
        ignore: Vec<String>,
        job_id: Option<String>,
    ) {
        if self.bisync_running {
            self.notice = Some((
                "Es läuft bereits ein Sync — bitte warten.".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        let pair = crate::bisync::pair_id(&root_a, &root_b);
        self.bisync_ctx = Some(BisyncCtx {
            a: a.clone(),
            root_a: root_a.clone(),
            b: b.clone(),
            root_b: root_b.clone(),
            pair,
            baseline: crate::bisync::Baseline::new(),
        });
        let (tx, rx) = unbounded();
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel_t = cancel.clone();
        std::thread::Builder::new()
            .name("bisync".into())
            .spawn(move || {
                let mut gb = globset::GlobSetBuilder::new();
                for pat in &ignore {
                    let pat = pat.trim();
                    if pat.is_empty() {
                        continue;
                    }
                    if let Ok(g) = globset::Glob::new(pat) {
                        gb.add(g);
                    }
                }
                let gs = gb.build().unwrap_or_else(|_| crate::bisync::empty_globset());
                let f = crate::bisync::WalkFilter {
                    include_hidden,
                    ignore: &gs,
                };
                let _ = tx.send(crate::bisync::run(
                    &*a, &root_a, &*b, &root_b, opts, retain_days, &cancel_t, &f,
                ));
            })
            .ok();
        self.bisync_cancel = Some(cancel);
        self.bisync_rx = Some(rx);
        self.bisync_running = true;
        self.running_job = job_id;
        self.notice = Some((
            "⇄ 2-Wege-Sync läuft…".to_string(),
            std::time::Instant::now(),
        ));
    }

    /// Run a saved sync setup now. Local↔local resolves instantly; if either
    /// endpoint is a saved-connection remote URL it's re-opened off the UI
    /// thread first (so the window doesn't freeze), then launched.
    fn run_job(&mut self, id: &str) {
        if self.bisync_running || self.job_connect_rx.is_some() {
            self.notice = Some((
                "Es läuft bereits ein Sync — bitte warten.".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        let job = match self.sync_jobs.iter().find(|j| j.id == id) {
            Some(j) => j.clone(),
            None => return,
        };
        let opts = crate::bisync::BisyncOptions {
            direction: job.direction,
            conflict: job.conflict,
            reversible: true,
            dry_run: false,
        };
        // Pure local: resolve inline (no network) and launch immediately.
        if !crate::connect::is_remote_url(&job.source)
            && !crate::connect::is_remote_url(&job.target)
        {
            let a: crate::vfs::BackendHandle =
                Arc::new(crate::vfs::LocalBackend::new(&job.source));
            let b: crate::vfs::BackendHandle =
                Arc::new(crate::vfs::LocalBackend::new(&job.target));
            self.launch_bisync(
                a,
                job.source.clone(),
                b,
                job.target.clone(),
                opts,
                job.retain_days,
                job.include_hidden,
                job.ignore.clone(),
                Some(job.id.clone()),
            );
            return;
        }
        // Remote endpoint(s): re-open the saved connection(s) off-thread.
        let (src, tgt) = (job.source.clone(), job.target.clone());
        let (tx, rx) = unbounded();
        std::thread::Builder::new()
            .name("job-connect".into())
            .spawn(move || {
                let res = (|| {
                    let a = crate::connect::resolve_endpoint(&src)?;
                    let b = crate::connect::resolve_endpoint(&tgt)?;
                    Ok::<_, String>((a, b))
                })();
                let _ = tx.send(res);
            })
            .ok();
        self.job_connect_rx = Some(rx);
        self.job_connect_pending = Some(job);
        self.notice = Some((
            "Verbinde mit Remote-Ziel…".to_string(),
            std::time::Instant::now(),
        ));
    }

    /// Once a remote job's endpoints are open, launch the sync (UI thread).
    fn drain_job_connect(&mut self) {
        let res = match self.job_connect_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            Some(r) => r,
            None => return,
        };
        self.job_connect_rx = None;
        let job = match self.job_connect_pending.take() {
            Some(j) => j,
            None => return,
        };
        match res {
            Ok(((a, root_a), (b, root_b))) => {
                let opts = crate::bisync::BisyncOptions {
                    direction: job.direction,
                    conflict: job.conflict,
                    reversible: true,
                    dry_run: false,
                };
                self.launch_bisync(
                    a,
                    root_a,
                    b,
                    root_b,
                    opts,
                    job.retain_days,
                    job.include_hidden,
                    job.ignore.clone(),
                    Some(job.id.clone()),
                );
            }
            Err(e) => {
                self.error_msg = Some(format!("Remote-Sync: {}", e));
            }
        }
    }

    /// Result of an interactive cloud authorize (#19, slice 1).
    fn drain_cloud_auth(&mut self) {
        let res = match self.cloud_auth_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            Some(r) => r,
            None => return,
        };
        self.cloud_auth_rx = None;
        self.cloud_authing = false;
        match res {
            Ok(()) => {
                self.notice = Some((
                    "✓ Google Drive verbunden".to_string(),
                    std::time::Instant::now(),
                ));
            }
            Err(e) => {
                self.error_msg = Some(format!("Cloud-Anmeldung: {}", e));
            }
        }
    }

    /// Open Google Drive as the active remote and browse it (reuses the normal
    /// connect drain → sidebar/scan path). Connects off the UI thread.
    fn open_gdrive_browse(&mut self) {
        if !crate::cloud::is_connected(crate::cloud::Provider::GDrive) {
            self.error_msg = Some("Google Drive ist nicht verbunden.".to_string());
            return;
        }
        let (tx, rx) = unbounded();
        self.connect_rx = Some(rx);
        self.connecting = true;
        std::thread::Builder::new()
            .name("gdrive-open".into())
            .spawn(move || {
                let res = match crate::connect::open_gdrive("/") {
                    Ok((be, root)) => crate::connect::ConnectResult::Ok(crate::connect::Connected {
                        remote: Some(crate::connect::RemoteState {
                            backend: be,
                            label: "Google Drive".to_string(),
                        }),
                        net: None,
                        target: root,
                        label: "Google Drive".to_string(),
                    }),
                    Err(e) => crate::connect::ConnectResult::Err(e),
                };
                let _ = tx.send(res);
            })
            .ok();
        self.notice = Some(("Verbinde mit Google Drive…".to_string(), std::time::Instant::now()));
    }

    /// Open Google Drive inside the picker (so a sync folder can be chosen on
    /// Drive). Connects off the UI thread via the picker's connect channel.
    fn picker_open_gdrive(&mut self) {
        let (tx, rx) = unbounded();
        std::thread::Builder::new()
            .name("gdrive-pick".into())
            .spawn(move || {
                let res = match crate::connect::open_gdrive("/") {
                    Ok((be, root)) => crate::connect::ConnectResult::Ok(crate::connect::Connected {
                        remote: Some(crate::connect::RemoteState {
                            backend: be,
                            label: "Google Drive".to_string(),
                        }),
                        net: None,
                        target: root,
                        label: "Google Drive".to_string(),
                    }),
                    Err(e) => crate::connect::ConnectResult::Err(e),
                };
                let _ = tx.send(res);
            })
            .ok();
        if let Some(p) = self.picker.as_mut() {
            p.connect_rx = Some(rx);
            p.connecting = true;
            p.is_remote = true;
            p.endpoint_prefix = "gdrive://".to_string();
            p.conn_label = "Google Drive".to_string();
        }
    }

    /// Backend + root for a tab index, honouring whether it's the focused tab
    /// (state in the App fields) or a parked split pane (state in `self.tabs`),
    /// and local vs. remote. Used by the split-view "sync these folders" action.
    fn pane_backend(&self, tab_idx: usize) -> (crate::vfs::BackendHandle, String) {
        if tab_idx == self.active_tab {
            let root = self.root_path.clone();
            let be: crate::vfs::BackendHandle = match &self.remote {
                Some(rs) => rs.backend.clone(),
                None => Arc::new(crate::vfs::LocalBackend::new(&root)),
            };
            (be, root)
        } else {
            let t = &self.tabs[tab_idx];
            let root = t.root_path.clone();
            let be: crate::vfs::BackendHandle = match &t.remote {
                Some(rs) => rs.backend.clone(),
                None => Arc::new(crate::vfs::LocalBackend::new(&root)),
            };
            (be, root)
        }
    }

    /// Two-way sync the two split panes' folders (right-click action). Safe
    /// defaults; works across local/remote since each pane's live backend is
    /// reused directly.
    fn sync_split_panes(&mut self) {
        if !self.split {
            return;
        }
        let (a_idx, b_idx) = (self.panes[0], self.panes[1]);
        let (a, root_a) = self.pane_backend(a_idx);
        let (b, root_b) = self.pane_backend(b_idx);
        if root_a.is_empty() || root_b.is_empty() {
            self.error_msg =
                Some("Beide Fenster müssen einen Ordner geöffnet haben.".to_string());
            return;
        }
        if root_a == root_b {
            self.error_msg = Some("Beide Fenster zeigen denselben Ordner.".to_string());
            return;
        }
        self.launch_bisync(
            a,
            root_a,
            b,
            root_b,
            crate::bisync::BisyncOptions::default(),
            30,
            true,
            Vec::new(),
            None,
        );
    }

    fn drain_bisync(&mut self) {
        let out = match self.bisync_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            Some(o) => o,
            None => return,
        };
        self.bisync_rx = None;
        self.bisync_running = false;
        self.bisync_cancel = None;
        // Stamp the saved job (if this run came from one) so its schedule and
        // "last run" reflect reality, then refresh the cached list.
        if let Some(id) = self.running_job.take() {
            crate::syncjobs::mark_run(&id);
            self.sync_jobs = crate::syncjobs::load();
        }
        if let Some(ctx) = self.bisync_ctx.as_mut() {
            ctx.baseline = out.baseline;
        }
        self.bisync_conflicts = out.conflicts;
        let s = out.stats;
        if !out.errors.is_empty() {
            self.error_msg = Some(format!("Sync: {} Fehler", out.errors.len()));
        }
        self.notice = Some((
            format!(
                "⇄ Sync: {} →, {} ←, {} gelöscht, {} Konflikte ({} MB)",
                s.a_to_b,
                s.b_to_a,
                s.deleted,
                self.bisync_conflicts.len(),
                s.bytes / 1_048_576
            ),
            std::time::Instant::now(),
        ));
        if !self.bisync_conflicts.is_empty() {
            self.show_bisync_conflicts = true;
        }
        // The current view may have changed on disk.
        if !self.root_path.is_empty() {
            self.rescan();
        }
    }

    /// Resolve conflict `idx` by keeping side A (→ overwrites B) or side B.
    fn resolve_conflict(&mut self, idx: usize, keep_a: bool) {
        if idx >= self.bisync_conflicts.len() {
            return;
        }
        let rel = self.bisync_conflicts[idx].rel.clone();
        let ctx = match self.bisync_ctx.as_mut() {
            Some(c) => c,
            None => return,
        };
        match crate::bisync::resolve(
            &*ctx.a, &ctx.root_a, &*ctx.b, &ctx.root_b, &rel, keep_a, &ctx.pair,
        ) {
            Ok((sa, sb)) => {
                ctx.baseline.insert(rel, (sa, sb));
            }
            Err(e) => {
                self.error_msg = Some(format!("Konfliktlösung: {}", e));
                return;
            }
        }
        self.bisync_conflicts.remove(idx);
        if self.bisync_conflicts.is_empty() {
            self.finish_bisync_conflicts();
        }
    }

    /// Persist the updated baseline once all conflicts are handled.
    fn finish_bisync_conflicts(&mut self) {
        if let Some(ctx) = &self.bisync_ctx {
            let path = crate::bisync::baseline_path(&ctx.pair);
            let _ = crate::bisync::save_baseline(&path, &ctx.baseline);
        }
        self.show_bisync_conflicts = false;
        if !self.root_path.is_empty() {
            self.rescan();
        }
    }

    fn ui_bisync_conflicts(&mut self, ctx: &egui::Context) {
        if !self.show_bisync_conflicts {
            return;
        }
        if self.bisync_conflicts.is_empty() {
            self.finish_bisync_conflicts();
            return;
        }
        let mut keep_a: Option<usize> = None;
        let mut keep_b: Option<usize> = None;
        let mut skip: Option<usize> = None;
        let mut close = false;
        let mut all_a = false;
        let mut all_b = false;
        let conflicts = self.bisync_conflicts.clone();
        egui::Window::new(format!("⚠ Sync-Konflikte ({})", conflicts.len()))
            .collapsible(false)
            .resizable(true)
            .default_size([620.0, 420.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("Beide Seiten wurden geändert. Wähle, welche Version gilt — die andere wird vorher reversibel gesichert.");
                ui.horizontal(|ui| {
                    if ui.button("Alle: ← A behalten").clicked() { all_a = true; }
                    if ui.button("Alle: B behalten →").clicked() { all_b = true; }
                });
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, c) in conflicts.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let a = c.a.map(|s| format!("{} B, {}", s.size, fmt_ms(s.mtime_ms))).unwrap_or_else(|| "—".into());
                            let b = c.b.map(|s| format!("{} B, {}", s.size, fmt_ms(s.mtime_ms))).unwrap_or_else(|| "—".into());
                            if ui.small_button("← A").on_hover_text(format!("A: {a}")).clicked() { keep_a = Some(i); }
                            if ui.small_button("B →").on_hover_text(format!("B: {b}")).clicked() { keep_b = Some(i); }
                            if ui.small_button("⏭").on_hover_text("Vorerst überspringen").clicked() { skip = Some(i); }
                            ui.label(&c.rel);
                        });
                    }
                });
                ui.add_space(6.0);
                if ui.button("Schließen (Rest später)").clicked() { close = true; }
            });
        if all_a || all_b {
            // resolve all in index order; removals shrink the vec, so resolve 0 repeatedly
            while !self.bisync_conflicts.is_empty() {
                self.resolve_conflict(0, all_a);
            }
        } else if let Some(i) = keep_a {
            self.resolve_conflict(i, true);
        } else if let Some(i) = keep_b {
            self.resolve_conflict(i, false);
        } else if let Some(i) = skip {
            if i < self.bisync_conflicts.len() {
                self.bisync_conflicts.remove(i);
            }
            if self.bisync_conflicts.is_empty() {
                self.finish_bisync_conflicts();
            }
        }
        if close {
            self.finish_bisync_conflicts();
        }
    }

    // ─── View ───────────────────────────────────────────────────────────

    fn recompute_view(&mut self) {
        let prefix = self.root_prefix();
        let cf = CompiledFilter::compile(&self.filter);
        let key = self.sort_key;
        let dir = self.sort_dir;
        self.summary_cache = None;
        self.sel_size_cache = (usize::MAX, usize::MAX, 0);
        self.view_dirty = false;

        // ─── Flat mode: contents of current dir only ──────────────────────
        if !self.recursive {
            let mut rows: Vec<(usize, u32)> = (0..self.entries.len())
                .filter(|&i| {
                    let e = &self.entries[i];
                    e.depth > 0 && cf.matches(e, &prefix)
                })
                .map(|i| (i, 0u32))
                .collect();
            let entries = &self.entries;
            rows.sort_unstable_by(|&(a, _), &(b, _)| {
                compare_entries(&entries[a], &entries[b], key, dir)
            });
            self.view = rows;
            self.last_view_recompute = Instant::now();
            return;
        }

        // ─── Tree mode: recursive view preserving folder structure ─────────
        let mut children_map: std::collections::HashMap<&str, Vec<usize>> =
            std::collections::HashMap::with_capacity(self.entries.len() / 4 + 16);
        for (i, e) in self.entries.iter().enumerate() {
            children_map.entry(e.parent.as_ref()).or_default().push(i);
        }

        let root_idx = match self
            .entries
            .iter()
            .position(|e| e.path.as_ref() == prefix.as_str())
        {
            Some(i) => i,
            None => {
                self.view = Vec::new();
                self.last_view_recompute = Instant::now();
                return;
            }
        };

        let mut file_matches = vec![false; self.entries.len()];
        for (i, e) in self.entries.iter().enumerate() {
            if !e.is_dir {
                file_matches[i] = cf.matches(e, &prefix);
            }
        }

        let mut has_match = vec![false; self.entries.len()];
        let mut stack: Vec<(usize, bool)> = vec![(root_idx, false)];
        while let Some((idx, expanded)) = stack.pop() {
            let e = &self.entries[idx];
            if !expanded {
                stack.push((idx, true));
                if let Some(children) = children_map.get(e.path.as_ref()) {
                    for &c in children {
                        if self.entries[c].is_dir {
                            stack.push((c, false));
                        }
                    }
                }
            } else {
                let mut any = false;
                if let Some(children) = children_map.get(e.path.as_ref()) {
                    for &c in children {
                        let ce = &self.entries[c];
                        if ce.is_dir {
                            if has_match[c] {
                                any = true;
                                break;
                            }
                        } else if file_matches[c] {
                            any = true;
                            break;
                        }
                    }
                }
                has_match[idx] = any;
            }
        }

        let dir_passes_view_filter = |idx: usize| -> bool {
            let e = &self.entries[idx];
            if !self.filter.include_dirs {
                return false;
            }
            if e.hidden && !self.filter.include_hidden {
                return false;
            }
            if e.system && !self.filter.include_system {
                return false;
            }
            true
        };

        let entries = &self.entries;
        let root_depth = entries[root_idx].depth;
        let mut visible: Vec<(usize, u32)> = Vec::new();

        struct Frame {
            children_remaining: std::vec::IntoIter<usize>,
        }
        let mut frames: Vec<Frame> = Vec::new();

        let make_sorted_children =
            |parent_idx: usize,
             children_map: &std::collections::HashMap<&str, Vec<usize>>,
             entries: &[FileEntry]|
             -> Vec<usize> {
                let parent_e = &entries[parent_idx];
                let mut out: Vec<usize> = match children_map.get(parent_e.path.as_ref()) {
                    Some(v) => v.clone(),
                    None => return Vec::new(),
                };
                out.retain(|&c| {
                    let ce = &entries[c];
                    if ce.is_dir {
                        has_match[c] && dir_passes_view_filter(c)
                    } else {
                        file_matches[c]
                    }
                });
                out.sort_unstable_by(|&a, &b| compare_entries(&entries[a], &entries[b], key, dir));
                out
            };

        frames.push(Frame {
            children_remaining: make_sorted_children(root_idx, &children_map, entries).into_iter(),
        });

        while let Some(frame) = frames.last_mut() {
            if let Some(idx) = frame.children_remaining.next() {
                let e = &entries[idx];
                let display_d = e.depth.saturating_sub(root_depth + 1);
                visible.push((idx, display_d));
                if e.is_dir {
                    let kids = make_sorted_children(idx, &children_map, entries);
                    frames.push(Frame {
                        children_remaining: kids.into_iter(),
                    });
                }
            } else {
                frames.pop();
            }
        }

        self.view = visible;
        self.last_view_recompute = Instant::now();
    }

    // ─── Selection / actions ────────────────────────────────────────────

    fn select_all(&mut self) {
        self.selection = self
            .view
            .iter()
            .map(|&(i, _)| self.entries[i].path.clone())
            .collect();
    }

    fn copy_paths_to_clipboard(&self, ctx: &egui::Context) {
        let lines: Vec<String> = self.selection.iter().map(|p| p.replace('/', "\\")).collect();
        ctx.copy_text(lines.join("\r\n"));
    }

    /// Move selection to the recycle bin on a background thread (a big
    /// selection can take seconds in the shell — that used to freeze the UI).
    fn trash_selected(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        // Remote view → delete via the backend (SFTP/FTP/WebDAV unlink; Drive
        // moves to its trash). std::fs/the recycle bin can't touch remote paths.
        if let Some(rs) = &self.remote {
            let backend = rs.backend.clone();
            let items: Vec<(String, bool)> = self
                .entries
                .iter()
                .filter(|e| self.selection.contains(&e.path))
                .map(|e| (e.path.to_string(), e.is_dir))
                .collect();
            let removed: HashSet<Arc<str>> = self.selection.drain().collect();
            self.entries.retain(|e| !removed.contains(&e.path));
            self.cursor = None;
            self.recompute_view();
            let (tx, rx) = unbounded();
            self.trash_rx = Some(rx);
            std::thread::Builder::new()
                .name("remote-delete".into())
                .spawn(move || {
                    let mut first_err: Option<String> = None;
                    for (p, is_dir) in &items {
                        let r = if *is_dir {
                            backend.remove_dir(p)
                        } else {
                            backend.remove_file(p)
                        };
                        if let Err(e) = r {
                            if first_err.is_none() {
                                first_err = Some(e.to_string());
                            }
                        }
                    }
                    let _ = tx.send(first_err);
                })
                .ok();
            return;
        }
        let paths: Vec<PathBuf> = self
            .selection
            .iter()
            .map(|p| PathBuf::from(p.replace('/', std::path::MAIN_SEPARATOR_STR)))
            .collect();
        // Optimistic UI update; on failure drain_trash() rescans.
        let removed: HashSet<Arc<str>> = self.selection.drain().collect();
        self.entries.retain(|e| !removed.contains(&e.path));
        self.cursor = None;
        self.recompute_view();

        let (tx, rx) = unbounded();
        self.trash_rx = Some(rx);
        std::thread::Builder::new()
            .name("trash".into())
            .spawn(move || {
                let res = trash::delete_all(&paths);
                let _ = tx.send(res.err().map(|e| e.to_string()));
            })
            .ok();
    }

    fn open_in_explorer(&self, path: &str) {
        let p = path.replace('/', "\\");
        let _ = std::process::Command::new("explorer.exe")
            .arg(format!("/select,{}", p))
            .spawn();
    }

    /// Open a file with its associated application. Uses ShellExecuteW —
    /// the previous `cmd /C start` spawned a visible console window.
    #[cfg(windows)]
    fn open_path(&self, path: &str) {
        let p = path.replace('/', "\\");
        let wide: Vec<u16> = p.encode_utf16().chain(Some(0)).collect();
        unsafe {
            windows_sys::Win32::UI::Shell::ShellExecuteW(
                std::ptr::null_mut(),
                std::ptr::null(),
                wide.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1, // SW_SHOWNORMAL
            );
        }
    }

    #[cfg(not(windows))]
    fn open_path(&self, path: &str) {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }

    fn open_selection(&mut self) {
        let targets: Vec<(String, String, bool)> = self
            .entries
            .iter()
            .filter(|e| self.selection.contains(&e.path))
            .map(|e| (e.path.to_string(), e.name.to_string(), e.is_dir))
            .collect();
        if targets.len() == 1 && targets[0].2 {
            let p = PathBuf::from(targets[0].0.replace('/', std::path::MAIN_SEPARATOR_STR));
            self.start_scan(p);
            return;
        }
        for (p, name, _) in targets.into_iter().filter(|(_, _, d)| !*d).take(10) {
            self.open_file(p, name);
        }
    }

    /// Open one entry by index: navigate into a folder, or open a file.
    fn activate_entry(&mut self, idx: usize) {
        if idx >= self.entries.len() {
            return;
        }
        let e = &self.entries[idx];
        if e.is_dir {
            let p = PathBuf::from(e.path.replace('/', std::path::MAIN_SEPARATOR_STR));
            self.start_scan(p);
            return;
        }
        let (path, name) = (e.path.to_string(), e.name.to_string());
        self.open_file(path, name);
    }

    /// Open a file in its associated app. Local files launch directly; a remote
    /// file is downloaded to a temp copy off the UI thread, then launched when
    /// ready (so double-click "just works" on SFTP/FTP/WebDAV/Drive too).
    fn open_file(&mut self, path: String, name: String) {
        let rs = match &self.remote {
            Some(rs) => rs,
            None => {
                self.open_path(&path);
                return;
            }
        };
        let backend = rs.backend.clone();
        let label = rs.label.clone();

        // The local destination depends on the open mode: an ephemeral temp copy,
        // or a stable per-connection sync-folder path that mirrors the remote
        // layout (CfAPI mode — see cfsync.rs). Both then download + watch + launch.
        let dest = match self.remote_open_mode {
            RemoteOpenMode::Temp => open_temp_path(&name),
            RemoteOpenMode::CfApi => {
                // Persistent per-connection sync folder mirroring the remote. (We
                // do NOT register it as a native CfAPI sync root: doing so without
                // a connected provider makes Windows' cloud filter reject normal
                // file creation — "invalid name request". Native on-demand
                // placeholders need a full CfConnectSyncRoot provider, see #30.)
                crate::cfsync::local_path(&label, &self.root_path, &path)
            }
        };
        self.remote_edits.retain(|e| e.temp != dest);
        if self.remote_edits.len() < 50 {
            self.remote_edits.push(RemoteEdit {
                temp: dest.clone(),
                backend: backend.clone(),
                remote_path: path.clone(),
                name: name.clone(),
                baseline_mtime: i64::MAX, // real value set once downloaded
                seen_mtime: 0,
                uploading: false,
            });
        }
        let (tx, rx) = unbounded();
        self.notice = Some((
            format!("⬇ Öffne „{}“ (Speichern landet auf dem Remote)…", name),
            std::time::Instant::now(),
        ));
        let dest_t = dest.clone();
        std::thread::Builder::new()
            .name("remote-open".into())
            .spawn(move || {
                let _ = tx.send(download_to(&*backend, &path, &dest_t));
            })
            .ok();
        self.file_open_rx.push(rx);
    }

    /// Launch any remote files that finished downloading to temp.
    fn drain_file_open(&mut self) {
        if self.file_open_rx.is_empty() {
            return;
        }
        let mut pending = Vec::new();
        let mut to_open = Vec::new();
        for rx in std::mem::take(&mut self.file_open_rx) {
            match rx.try_recv() {
                Ok(Ok(p)) => to_open.push(p),
                Ok(Err(e)) => self.error_msg = Some(format!("Datei öffnen: {}", e)),
                Err(crossbeam_channel::TryRecvError::Empty) => pending.push(rx),
                Err(_) => {}
            }
        }
        self.file_open_rx = pending;
        for p in to_open {
            // Baseline the edit-watch to the freshly downloaded content so we
            // don't immediately re-upload it; only the user's saves count.
            let pb = PathBuf::from(p.replace('/', std::path::MAIN_SEPARATOR_STR));
            let m = file_mtime_ms(&pb);
            if let Some(e) = self.remote_edits.iter_mut().find(|e| e.temp == pb) {
                e.baseline_mtime = m;
                e.seen_mtime = m;
            }
            self.open_path(&p);
        }
    }

    /// Poll temp-mode edit copies; re-upload to the remote when one is saved
    /// (mtime advances and is stable for one ~1.5s cycle = a completed write).
    fn poll_remote_edits(&mut self) {
        if self.remote_edits.is_empty() {
            return;
        }
        if self.last_edit_poll.elapsed() < std::time::Duration::from_millis(1500) {
            return;
        }
        self.last_edit_poll = std::time::Instant::now();
        let mut launch: Vec<(PathBuf, crate::vfs::BackendHandle, String, String)> = Vec::new();
        for e in self.remote_edits.iter_mut().filter(|e| !e.uploading) {
            let m = file_mtime_ms(&e.temp);
            if m == 0 || m == e.baseline_mtime {
                continue;
            }
            if m == e.seen_mtime {
                e.uploading = true;
                e.baseline_mtime = m;
                launch.push((e.temp.clone(), e.backend.clone(), e.remote_path.clone(), e.name.clone()));
            } else {
                e.seen_mtime = m;
            }
        }
        for (temp, be, remote, name) in launch {
            let (tx, rx) = unbounded();
            self.edit_save_rx.push(rx);
            self.notice = Some((
                format!("↑ Speichere „{}“ auf dem Remote…", name),
                std::time::Instant::now(),
            ));
            std::thread::Builder::new()
                .name("remote-edit-save".into())
                .spawn(move || {
                    let r = upload_file(&*be, &temp, &remote);
                    let _ = tx.send((temp, r));
                })
                .ok();
        }
    }

    fn drain_edit_saves(&mut self) {
        if self.edit_save_rx.is_empty() {
            return;
        }
        let mut pending = Vec::new();
        for rx in std::mem::take(&mut self.edit_save_rx) {
            match rx.try_recv() {
                Ok((temp, res)) => {
                    if let Some(e) = self.remote_edits.iter_mut().find(|e| e.temp == temp) {
                        e.uploading = false;
                        match res {
                            Ok(()) => {
                                self.notice = Some((
                                    format!("✓ „{}“ auf dem Remote gespeichert", e.name),
                                    std::time::Instant::now(),
                                ));
                            }
                            Err(err) => {
                                e.baseline_mtime = 0; // let a later save retry
                                self.error_msg =
                                    Some(format!("Remote speichern „{}“: {}", e.name, err));
                            }
                        }
                    }
                }
                Err(crossbeam_channel::TryRecvError::Empty) => pending.push(rx),
                Err(_) => {}
            }
        }
        self.edit_save_rx = pending;
    }

    /// Upload local `paths` (files and/or folders, recursively) into the remote
    /// folder `dest_root` via `backend`, off the UI thread. Used by Ctrl+V and
    /// drag-drop into a remote view.
    fn start_remote_upload(
        &mut self,
        paths: Vec<String>,
        backend: crate::vfs::BackendHandle,
        dest_root: String,
    ) {
        if self.upload_rx.is_some() {
            self.notice = Some((
                "Es läuft bereits ein Upload — bitte warten.".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        let n = paths.len();
        let (tx, rx) = unbounded();
        std::thread::Builder::new()
            .name("remote-upload".into())
            .spawn(move || {
                let r = upload_paths(&*backend, &paths, &dest_root);
                let _ = tx.send(r);
            })
            .ok();
        self.upload_rx = Some(rx);
        self.notice = Some((
            format!("⬆ Lade {} Element(e) hoch…", n),
            std::time::Instant::now(),
        ));
    }

    /// Once selected remote files have downloaded to temp, put them on the
    /// Windows clipboard as CF_HDROP so they paste into Explorer.
    fn drain_clip_download(&mut self) {
        let local = match self.clip_download_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            Some(v) => v,
            None => return,
        };
        self.clip_download_rx = None;
        if local.is_empty() {
            self.error_msg = Some("Zwischenablage: Download fehlgeschlagen".to_string());
            return;
        }
        #[cfg(windows)]
        match crate::shell_clipboard::write_files(&local, crate::shell_clipboard::DROPEFFECT_COPY) {
            Ok(_) => {
                self.virtual_clip = None;
                self.notice = Some((
                    format!(
                        "✓ {} Datei(en) kopiert — in Explorer einfügbar (Ctrl+V)",
                        local.len()
                    ),
                    std::time::Instant::now(),
                ));
            }
            Err(e) => self.error_msg = Some(format!("Zwischenablage: {}", e)),
        }
        #[cfg(not(windows))]
        let _ = local;
    }

    // ─── Peer file sharing (#21) ─────────────────────────────────────────

    /// Start the share service if a rendezvous server is configured. Returns
    /// whether a service is available.
    fn ensure_share(&mut self) -> bool {
        if self.share.is_some() {
            return true;
        }
        let server = self.share_server.trim().to_string();
        if server.is_empty() {
            self.share_status = "Kein Server eingetragen (Einstellungen → TEILEN)".to_string();
            return false;
        }
        let device = if self.share_device_draft.trim().is_empty() {
            default_device_name()
        } else {
            self.share_device_draft.trim().to_string()
        };
        match crate::share::ShareService::start(server, device) {
            Ok(svc) => {
                self.share = Some(svc);
                true
            }
            Err(e) => {
                self.error_msg = Some(format!("Teilen-Dienst: {}", e));
                false
            }
        }
    }

    fn share_cmd(&mut self, c: crate::share::ShareCmd) {
        if self.ensure_share() {
            if let Some(svc) = &self.share {
                svc.cmd(c);
            }
        }
    }

    /// Lazily start Quick Share LAN discovery while the Teilen view is open, and
    /// drain discovered devices.
    fn drain_quickshare(&mut self) {
        if self.show_share && self.quickshare.is_none() {
            let name = if self.share_device_draft.trim().is_empty() {
                default_device_name()
            } else {
                self.share_device_draft.trim().to_string()
            };
            self.quickshare = crate::quickshare::QuickShare::start(&name);
        }
        if let Some(qs) = &self.quickshare {
            for list in qs.events.try_iter() {
                self.qs_devices = list;
            }
        }
    }

    fn drain_share(&mut self) {
        let events: Vec<crate::share::ShareEvent> = match &self.share {
            Some(svc) => svc.events.try_iter().collect(),
            None => return,
        };
        for ev in events {
            use crate::share::ShareEvent as E;
            match ev {
                E::Status(s) => self.share_status = s,
                E::Error(e) => {
                    self.share_status = format!("Fehler: {}", e);
                    self.error_msg = Some(format!("Teilen: {}", e));
                }
                E::Roster(r) => self.share_roster = r,
                E::Incoming { id, from, files } => {
                    self.share_incoming.push((id, from, files));
                    self.show_share = true;
                }
                E::Progress { done, total } => self.share_progress = Some((done, total)),
                E::Received { count, dir } => {
                    self.share_progress = None;
                    self.share_status = format!("✓ {} empfangen → {}", count, dir);
                    self.notice = Some((
                        format!("📥 {} Datei(en) empfangen", count),
                        std::time::Instant::now(),
                    ));
                }
                E::Sent { count } => {
                    self.share_progress = None;
                    self.share_status = format!("✓ {} gesendet", count);
                }
            }
        }
    }

    /// Local file paths in the current selection (sharing sends local files;
    /// remote selections aren't supported yet).
    fn selected_local_files(&self) -> Vec<String> {
        if self.remote.is_some() {
            return Vec::new();
        }
        self.entries
            .iter()
            .filter(|e| !e.is_dir && self.selection.contains(&e.path))
            .map(|e| e.path.replace('/', std::path::MAIN_SEPARATOR_STR))
            .collect()
    }

    fn ui_share(&mut self, ctx: &egui::Context) {
        let mut open = self.show_share;
        let mut pair_show = false;
        let mut pair_connect = false;
        let mut room_join = false;
        let mut leave = false;
        let mut send = false;
        let mut answer: Option<(u64, bool)> = None;

        let roster = self.share_roster.clone();
        let incoming = self.share_incoming.clone();
        let status = self.share_status.clone();
        let progress = self.share_progress;
        let my_code = self.share_my_code.clone();
        let fingerprint = self.share.as_ref().map(|s| s.fingerprint.clone()).unwrap_or_default();
        let sel = self.selected_local_files().len();
        let qs_devices = self.qs_devices.clone();

        egui::Window::new("📡 Teilen — Geräte & Räume")
            .open(&mut open)
            .resizable(true)
            .default_size([460.0, 520.0])
            .show(ctx, |ui| {
                if self.share_server.trim().is_empty() {
                    ui.colored_label(
                        Color32::from_rgb(255, 185, 120),
                        "Kein Rendezvous-Server eingetragen.",
                    );
                    ui.label("Einstellungen → TEILEN: Server-Adresse (host:port) setzen.");
                    return;
                }
                if !fingerprint.is_empty() {
                    ui.label(
                        RichText::new(format!("Dieses Gerät: {}", fingerprint))
                            .small()
                            .color(Color32::from_gray(140)),
                    );
                }

                ui.add_space(6.0);
                ui.label(RichText::new("DIREKT KOPPELN").small().color(Color32::from_gray(140)));
                ui.horizontal(|ui| {
                    if ui.button("Code anzeigen").on_hover_text("Erzeugt einen Code; das andere Gerät gibt ihn ein").clicked() {
                        pair_show = true;
                    }
                    if !my_code.is_empty() {
                        ui.label(RichText::new(&my_code).monospace().strong().size(18.0));
                    }
                });
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.share_code_input).hint_text("Code vom anderen Gerät").desired_width(160.0));
                    if ui.button("Verbinden").clicked() {
                        pair_connect = true;
                    }
                });

                ui.add_space(8.0);
                ui.label(RichText::new("RAUM").small().color(Color32::from_gray(140)));
                ui.horizontal(|ui| {
                    if ui.button("Raum erstellen").clicked() {
                        room_join = true; // generates a code below
                    }
                    if ui.button("Beitreten").clicked() {
                        room_join = true;
                    }
                });

                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Verbundene Geräte ({})", roster.len())).strong());
                    if !roster.is_empty() && ui.small_button("Verlassen").clicked() {
                        leave = true;
                    }
                });
                if roster.is_empty() {
                    ui.colored_label(Color32::from_gray(140), "(noch keine — Code teilen oder Raum beitreten)");
                }
                for d in &roster {
                    ui.label(format!("● {}  ({})", d.device, d.fingerprint));
                }

                ui.add_space(6.0);
                if ui
                    .add_enabled(sel > 0 && !roster.is_empty(), egui::Button::new(format!("⮝ {} ausgewählte Datei(en) senden", sel)))
                    .on_hover_text("Sendet die in der Liste markierten lokalen Dateien an alle verbundenen Geräte")
                    .clicked()
                {
                    send = true;
                }
                if sel == 0 {
                    ui.label(RichText::new("Markiere lokale Dateien in der Liste, um sie zu senden.").small().color(Color32::from_gray(120)));
                }

                if let Some((done, total)) = progress {
                    let frac = if total > 0 { done as f32 / total as f32 } else { 0.0 };
                    ui.add(egui::ProgressBar::new(frac).show_percentage());
                }
                if !status.is_empty() {
                    ui.label(RichText::new(&status).small().color(Color32::from_gray(150)));
                }

                // Quick Share (Android) devices seen on the LAN.
                ui.separator();
                egui::CollapsingHeader::new(format!("📱 Quick Share (LAN) — {} gefunden", qs_devices.len()))
                    .id_salt("qs_devices")
                    .show(ui, |ui| {
                        if qs_devices.is_empty() {
                            ui.colored_label(Color32::from_gray(140), "(Suche… Android: Quick Share auf „Alle“ stellen)");
                        }
                        for d in &qs_devices {
                            ui.label(format!("📱 {}  {}", d.name, d.addr));
                        }
                        ui.label(
                            RichText::new(
                                "Übertragung zu/von Quick Share ist in Arbeit (UKEY2/Protobuf, \
                                 siehe docs/QUICKSHARE.md). Für Geräte mit Smart Explorer nutze \
                                 oben Direkt koppeln / Raum.",
                            )
                            .small()
                            .color(Color32::from_gray(120)),
                        );
                    });

                if !incoming.is_empty() {
                    ui.separator();
                    ui.label(RichText::new("EINGEHEND").small().color(Color32::from_gray(140)));
                    for (id, from, files) in &incoming {
                        let total: u64 = files.iter().map(|(_, s)| *s).sum();
                        ui.label(format!("{} möchte {} Datei(en) senden ({})", from, files.len(), format_bytes(total)));
                        ui.horizontal(|ui| {
                            if ui.button("Annehmen").clicked() {
                                answer = Some((*id, true));
                            }
                            if ui.button("Ablehnen").clicked() {
                                answer = Some((*id, false));
                            }
                        });
                    }
                }
            });
        self.show_share = open;

        if pair_show {
            let code = crate::share::gen_code();
            self.share_my_code = code.clone();
            self.share_room = false;
            self.share_cmd(crate::share::ShareCmd::Pair(code));
        }
        if pair_connect {
            let code = self.share_code_input.trim().to_string();
            if !code.is_empty() {
                self.share_my_code.clear();
                self.share_cmd(crate::share::ShareCmd::Pair(code));
            }
        }
        if room_join {
            let code = if self.share_code_input.trim().is_empty() {
                let c = crate::share::gen_code();
                self.share_my_code = c.clone();
                c
            } else {
                self.share_code_input.trim().to_string()
            };
            self.share_room = true;
            self.share_cmd(crate::share::ShareCmd::JoinRoom(code));
        }
        if leave {
            self.share_roster.clear();
            self.share_my_code.clear();
            self.share_cmd(crate::share::ShareCmd::Leave);
        }
        if send {
            let files = self.selected_local_files();
            if files.is_empty() {
                self.error_msg = Some("Keine lokalen Dateien ausgewählt.".to_string());
            } else {
                self.share_cmd(crate::share::ShareCmd::Send(files));
            }
        }
        if let Some((id, accept)) = answer {
            self.share_incoming.retain(|(i, _, _)| *i != id);
            self.share_cmd(crate::share::ShareCmd::Answer { id, accept });
        }
    }

    fn drain_remote_op(&mut self) {
        let res = match self.remote_op_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            Some(r) => r,
            None => return,
        };
        self.remote_op_rx = None;
        match res {
            Ok(msg) => {
                self.notice = Some((msg, std::time::Instant::now()));
                self.rescan();
            }
            // The worker already includes the operation context in the message.
            Err(e) => self.error_msg = Some(e),
        }
    }

    /// Our own right-click menu for a remote entry (the Windows shell menu can't
    /// act on remote paths). Each action routes through the backend.
    fn ui_remote_ctx(&mut self, ctx: &egui::Context) {
        let (pos, idx) = match self.remote_ctx {
            Some(v) => v,
            None => return,
        };
        if idx >= self.entries.len() {
            self.remote_ctx = None;
            return;
        }
        let e = &self.entries[idx];
        let path = e.path.to_string();
        let name = e.name.to_string();
        let is_dir = e.is_dir;

        #[derive(Clone, Copy)]
        enum A { Open, DownloadTo, CopyClip, Rename, Delete, NewFolder, CopyPath, Refresh }
        let mut act: Option<A> = None;
        let area = egui::Area::new(egui::Id::new("remote_ctx_menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(200.0);
                    if ui.button(if is_dir { "📂 Öffnen" } else { "📄 Öffnen" }).clicked() {
                        act = Some(A::Open);
                    }
                    if !is_dir {
                        if ui.button("⬇ Herunterladen nach…").clicked() {
                            act = Some(A::DownloadTo);
                        }
                        if ui.button("📋 In Zwischenablage kopieren").clicked() {
                            act = Some(A::CopyClip);
                        }
                    }
                    ui.separator();
                    if ui.button("✎ Umbenennen").clicked() {
                        act = Some(A::Rename);
                    }
                    if ui.button("🗑 Löschen").clicked() {
                        act = Some(A::Delete);
                    }
                    ui.separator();
                    if ui.button("＋ Neuer Ordner").clicked() {
                        act = Some(A::NewFolder);
                    }
                    if ui.button("⧉ Pfad kopieren").clicked() {
                        act = Some(A::CopyPath);
                    }
                    if ui.button("⟳ Aktualisieren").clicked() {
                        act = Some(A::Refresh);
                    }
                });
            });
        let dismiss = ctx.input(|i| i.key_pressed(egui::Key::Escape))
            || (ctx.input(|i| i.pointer.any_pressed())
                && ctx
                    .input(|i| i.pointer.interact_pos())
                    .map(|p| !area.response.rect.contains(p))
                    .unwrap_or(false));
        let act = match act {
            Some(a) => {
                self.remote_ctx = None;
                a
            }
            None => {
                if dismiss {
                    self.remote_ctx = None;
                }
                return;
            }
        };
        match act {
            A::Open => self.activate_entry(idx),
            A::Refresh => self.rescan(),
            A::NewFolder => self.create_new_folder(),
            A::Delete => self.trash_selected(),
            A::CopyClip => self.clipboard_copy_files(false),
            A::CopyPath => ctx.copy_text(path),
            A::Rename => {
                self.rename_open = Some((path, name));
                self.rename_focus = true;
            }
            A::DownloadTo => {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    if let Some(rs) = &self.remote {
                        if self.remote_op_rx.is_none() {
                            let backend = rs.backend.clone();
                            let dest = dir.join(&name);
                            let (tx, rx) = unbounded();
                            self.remote_op_rx = Some(rx);
                            self.notice = Some((
                                format!("⬇ Lade „{}“ herunter…", name),
                                std::time::Instant::now(),
                            ));
                            std::thread::Builder::new()
                                .name("remote-download".into())
                                .spawn(move || {
                                    let r = (|| -> Result<(), String> {
                                        let mut rd =
                                            backend.open_read(&path).map_err(|e| e.to_string())?;
                                        let mut f = std::fs::File::create(&dest)
                                            .map_err(|e| e.to_string())?;
                                        std::io::copy(&mut rd, &mut f).map_err(|e| e.to_string())?;
                                        Ok(())
                                    })();
                                    let _ = tx.send(
                                        r.map(|_| format!("✓ Heruntergeladen: {}", name))
                                            .map_err(|e| format!("Herunterladen: {}", e)),
                                    );
                                })
                                .ok();
                        }
                    }
                }
            }
        }
    }

    fn drain_upload(&mut self) {
        let res = match self.upload_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            Some(r) => r,
            None => return,
        };
        self.upload_rx = None;
        let (copied, errors) = res;
        if !errors.is_empty() {
            self.error_msg = Some(format!("Übertragung: {} Fehler (z. B. {})", errors.len(), errors[0]));
        }
        self.notice = Some((
            format!("✓ {} übertragen", copied),
            std::time::Instant::now(),
        ));
        // Show the newly uploaded files.
        if self.remote.is_some() && !self.root_path.is_empty() {
            self.rescan();
        }
    }

    /// The path the keyboard actions should act on: cursor first, else the
    /// first selected entry.
    fn focus_path(&self) -> Option<String> {
        self.cursor
            .as_ref()
            .map(|p| p.to_string())
            .or_else(|| self.selection.iter().next().map(|p| p.to_string()))
    }

    /// Open the native file Properties sheet for the focused item.
    #[cfg(windows)]
    fn show_properties(&mut self) {
        let p = match self.focus_path() {
            Some(p) => p.replace('/', "\\"),
            None => return,
        };
        use windows_sys::Win32::UI::Shell::{
            ShellExecuteExW, SEE_MASK_INVOKEIDLIST, SHELLEXECUTEINFOW,
        };
        let verb: Vec<u16> = "properties".encode_utf16().chain(Some(0)).collect();
        let file: Vec<u16> = p.encode_utf16().chain(Some(0)).collect();
        let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_INVOKEIDLIST;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.nShow = 1; // SW_SHOWNORMAL
        unsafe {
            ShellExecuteExW(&mut info);
        }
    }

    #[cfg(not(windows))]
    fn show_properties(&mut self) {}

    /// Invert the selection within the current view.
    fn invert_selection(&mut self) {
        let mut new: HashSet<Arc<str>> = HashSet::new();
        for &(i, _) in &self.view {
            let p = self.entries[i].path.clone();
            if !self.selection.contains(&p) {
                new.insert(p);
            }
        }
        self.selection = new;
        self.cursor = None;
    }

    /// Permanently delete the selection (bypassing the recycle bin), after an
    /// explicit confirmation. Runs the deletes on a worker thread.
    fn delete_permanent(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        let n = self.selection.len();
        if !confirm_yes_no(
            "Endgültig löschen",
            &format!(
                "{} Eintrag/Einträge UNWIDERRUFLICH löschen (nicht in den Papierkorb)?",
                n
            ),
        ) {
            return;
        }
        let paths: Vec<PathBuf> = self
            .selection
            .iter()
            .map(|p| PathBuf::from(p.replace('/', std::path::MAIN_SEPARATOR_STR)))
            .collect();
        let removed: HashSet<Arc<str>> = self.selection.drain().collect();
        self.entries.retain(|e| !removed.contains(&e.path));
        self.cursor = None;
        self.recompute_view();

        let (tx, rx) = unbounded();
        self.trash_rx = Some(rx); // reuse the trash result channel/drain
        std::thread::Builder::new()
            .name("delete-permanent".into())
            .spawn(move || {
                let mut first_err: Option<String> = None;
                for p in &paths {
                    let res = if p.is_dir() {
                        std::fs::remove_dir_all(p)
                    } else {
                        std::fs::remove_file(p)
                    };
                    if let Err(e) = res {
                        if first_err.is_none() {
                            first_err = Some(e.to_string());
                        }
                    }
                }
                let _ = tx.send(first_err);
            })
            .ok();
    }

    fn star_current_folder(&mut self) {
        if self.root_path.is_empty() {
            return;
        }
        let p = self.root_prefix();
        self.toggle_favorite(&p);
    }

    fn open_rename(&mut self) {
        if self.selection.len() != 1 {
            self.notice = Some((
                "Zum Umbenennen genau einen Eintrag auswählen".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        let p = self.selection.iter().next().unwrap().to_string();
        let name = p.rsplit('/').next().unwrap_or("").to_string();
        self.rename_open = Some((p, name));
        self.rename_focus = true;
    }

    fn create_new_folder(&mut self) {
        if self.root_path.is_empty() {
            return;
        }
        // Remote view → create via the backend (off the UI thread).
        if let Some(rs) = &self.remote {
            if self.remote_op_rx.is_some() {
                return;
            }
            let backend = rs.backend.clone();
            let base = self.root_path.trim_end_matches('/').to_string();
            let (tx, rx) = unbounded();
            self.remote_op_rx = Some(rx);
            std::thread::Builder::new()
                .name("remote-mkdir".into())
                .spawn(move || {
                    let mut name = "Neuer Ordner".to_string();
                    let mut i = 2;
                    while backend.exists(&format!("{}/{}", base, name)) && i < 1000 {
                        name = format!("Neuer Ordner ({})", i);
                        i += 1;
                    }
                    let path = format!("{}/{}", base, name);
                    let _ = tx.send(
                        backend
                            .mkdir_all(&path)
                            .map(|_| format!("✓ Ordner erstellt: {}", name))
                            .map_err(|e| format!("Ordner erstellen: {}", e)),
                    );
                })
                .ok();
            self.notice = Some(("Ordner wird erstellt…".to_string(), std::time::Instant::now()));
            return;
        }
        let base = PathBuf::from(self.root_path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let mut target = base.join("Neuer Ordner");
        let mut i = 2;
        while target.exists() {
            target = base.join(format!("Neuer Ordner ({})", i));
            i += 1;
        }
        match std::fs::create_dir(&target) {
            Ok(_) => {
                self.rescan();
                self.notice = Some((
                    format!("✓ Ordner erstellt: {}", target.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()),
                    std::time::Instant::now(),
                ));
            }
            Err(e) => self.error_msg = Some(format!("Ordner erstellen: {}", e)),
        }
    }

    /// Create a new empty editable file (`base.ext`) in the current folder, with
    /// a unique name. Local: created + opened for editing. Remote: created via
    /// the backend off-thread (open it afterwards by double-click).
    fn create_new_file(&mut self, base: &str, ext: &str) {
        if self.root_path.is_empty() {
            return;
        }
        // Remote view → create via the backend (threaded).
        if let Some(rs) = &self.remote {
            if self.remote_op_rx.is_some() {
                return;
            }
            let backend = rs.backend.clone();
            let root = self.root_path.trim_end_matches('/').to_string();
            let (base, ext) = (base.to_string(), ext.to_string());
            let (tx, rx) = unbounded();
            self.remote_op_rx = Some(rx);
            std::thread::Builder::new()
                .name("remote-newfile".into())
                .spawn(move || {
                    use std::io::Write;
                    let mut name = format!("{}.{}", base, ext);
                    let mut i = 2;
                    while backend.exists(&format!("{}/{}", root, name)) && i < 1000 {
                        name = format!("{} ({}).{}", base, i, ext);
                        i += 1;
                    }
                    let path = format!("{}/{}", root, name);
                    let r = (|| -> Result<(), String> {
                        let mut w = backend.open_write(&path).map_err(|e| e.to_string())?;
                        w.flush().map_err(|e| e.to_string())?;
                        Ok(())
                    })();
                    let _ = tx.send(
                        r.map(|_| format!("✓ Datei erstellt: {}", name))
                            .map_err(|e| format!("Datei erstellen: {}", e)),
                    );
                })
                .ok();
            self.notice = Some(("Datei wird erstellt…".to_string(), std::time::Instant::now()));
            return;
        }
        // Local view.
        let base_dir = PathBuf::from(self.root_path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let mut target = base_dir.join(format!("{}.{}", base, ext));
        let mut i = 2;
        while target.exists() {
            target = base_dir.join(format!("{} ({}).{}", base, i, ext));
            i += 1;
        }
        match std::fs::File::create(&target) {
            Ok(_) => {
                self.rescan();
                let nm = target.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                self.notice = Some((format!("✓ Datei erstellt: {}", nm), std::time::Instant::now()));
                self.open_path(&target.to_string_lossy().replace('\\', "/"));
            }
            Err(e) => self.error_msg = Some(format!("Datei erstellen: {}", e)),
        }
    }

    fn move_cursor_to(&mut self, pos: usize, shift: bool) {
        if self.view.is_empty() {
            return;
        }
        let pos = pos.min(self.view.len() - 1);
        let path = self.entries[self.view[pos].0].path.clone();
        if shift {
            if let Some(anchor) = self.last_anchor.clone() {
                if let Some(a) = self
                    .view
                    .iter()
                    .position(|&(i, _)| self.entries[i].path == anchor)
                {
                    let (lo, hi) = if a < pos { (a, pos) } else { (pos, a) };
                    self.selection.clear();
                    for r in lo..=hi {
                        self.selection
                            .insert(self.entries[self.view[r].0].path.clone());
                    }
                } else {
                    self.selection.clear();
                    self.selection.insert(path.clone());
                    self.last_anchor = Some(path.clone());
                }
            } else {
                self.selection.clear();
                self.selection.insert(path.clone());
                self.last_anchor = Some(path.clone());
            }
        } else {
            self.selection.clear();
            self.selection.insert(path.clone());
            self.last_anchor = Some(path.clone());
        }
        self.cursor = Some(path);
        self.pending_scroll_row = Some(pos);
    }

    fn cursor_pos_in_view(&self) -> Option<usize> {
        let c = self.cursor.as_ref()?;
        self.view
            .iter()
            .position(|&(i, _)| self.entries[i].path == *c)
    }

    fn move_cursor(&mut self, delta: isize, shift: bool) {
        if self.view.is_empty() {
            return;
        }
        let next = match self.cursor_pos_in_view() {
            Some(c) => (c as isize + delta).clamp(0, self.view.len() as isize - 1) as usize,
            None => {
                if delta >= 0 {
                    0
                } else {
                    self.view.len() - 1
                }
            }
        };
        self.move_cursor_to(next, shift);
    }

    fn type_to_jump(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.type_jump_at.elapsed().as_millis() > 800 {
            self.type_jump.clear();
        }
        self.type_jump.push_str(&text.to_lowercase());
        self.type_jump_at = Instant::now();
        let needle = self.type_jump.clone();
        if let Some(pos) = self
            .view
            .iter()
            .position(|&(i, _)| self.entries[i].name.to_lowercase().starts_with(&needle))
        {
            self.move_cursor_to(pos, false);
        }
    }

    fn confirm_rename(&mut self) {
        let (path, draft) = match self.rename_open.take() {
            Some(v) => v,
            None => return,
        };
        let draft = draft.trim().to_string();
        if draft.is_empty() {
            return;
        }
        // Remote view → rename via the backend (off the UI thread).
        if let Some(rs) = &self.remote {
            if draft.contains('/') || draft.contains('\\') {
                self.error_msg = Some("Name darf keine Schrägstriche enthalten.".to_string());
                return;
            }
            let old_fwd = path.clone();
            let parent = old_fwd.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
            let new_fwd = if parent.is_empty() {
                draft.clone()
            } else {
                format!("{}/{}", parent, draft)
            };
            if new_fwd == old_fwd || self.remote_op_rx.is_some() {
                return;
            }
            let backend = rs.backend.clone();
            let (tx, rx) = unbounded();
            self.remote_op_rx = Some(rx);
            std::thread::Builder::new()
                .name("remote-rename".into())
                .spawn(move || {
                    let _ = tx.send(
                        backend
                            .rename(&old_fwd, &new_fwd)
                            .map(|_| format!("✓ Umbenannt: {}", draft))
                            .map_err(|e| format!("Umbenennen: {}", e)),
                    );
                })
                .ok();
            self.selection.clear();
            return;
        }
        let old = PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let new = match old.parent() {
            Some(p) => p.join(&draft),
            None => return,
        };
        if new == old {
            return;
        }
        if new.exists() {
            self.error_msg = Some(format!("Ziel existiert bereits: {}", draft));
            return;
        }
        match std::fs::rename(&old, &new) {
            Ok(_) => {
                self.selection.clear();
                self.rescan();
            }
            Err(e) => self.error_msg = Some(format!("Umbenennen: {}", e)),
        }
    }

    fn confirm_copy(&mut self) {
        // Selection seeds; the worker thread expands directories recursively
        // and applies the current filter (no UI freeze on big subtrees).
        let seeds: Vec<FileEntry> = self
            .entries
            .iter()
            .filter(|e| self.selection.contains(&e.path))
            .cloned()
            .collect();
        if seeds.is_empty() || self.copy_dest.is_empty() {
            return;
        }
        let opts = CopyOptions {
            root: PathBuf::from(self.root_path.replace('/', std::path::MAIN_SEPARATOR_STR)),
            dest: PathBuf::from(&self.copy_dest),
            preserve_structure: self.copy_preserve,
            conflict: self.copy_conflict,
            mode: self.copy_mode_pending,
        };
        let (tx, rx) = unbounded();
        let h = start_copy_expanded(
            seeds,
            Some((self.filter.clone(), self.root_prefix())),
            opts,
            tx,
        );
        self.copy_handle = Some(h);
        self.copy_rx = Some(rx);
        self.copy_progress = Some(CopyProgress {
            files_done: 0,
            files_total: 0,
            bytes_done: 0,
            bytes_total: 0,
            elapsed_ms: 0,
            current_path: String::new(),
            errors: 0,
            done: false,
        });
        self.copy_errors.clear();
    }

    // ─── Clipboard ──────────────────────────────────────────────────────

    #[cfg(windows)]
    fn clipboard_copy_files(&mut self, cut: bool) {
        if self.selection.is_empty() {
            self.notice = Some((
                "Nichts ausgewählt — bitte erst Dateien markieren".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        // Remote selection → download the files to temp, then put those local
        // paths on the clipboard so they paste into Explorer (or back into us).
        if let Some(rs) = &self.remote {
            let files: Vec<(String, String)> = self
                .entries
                .iter()
                .filter(|e| !e.is_dir && self.selection.contains(&e.path))
                .map(|e| (e.path.to_string(), e.name.to_string()))
                .collect();
            if files.is_empty() {
                self.notice = Some((
                    "Remote: nur Dateien können in die Zwischenablage kopiert werden (keine Ordner).".to_string(),
                    std::time::Instant::now(),
                ));
                return;
            }
            let backend = rs.backend.clone();
            let n = files.len();
            let (tx, rx) = unbounded();
            self.clip_download_rx = Some(rx);
            self.notice = Some((
                format!("⬇ Bereite {} Datei(en) für die Zwischenablage vor…", n),
                std::time::Instant::now(),
            ));
            std::thread::Builder::new()
                .name("clip-download".into())
                .spawn(move || {
                    let mut local = Vec::new();
                    for (path, name) in &files {
                        if let Ok(p) = download_to_temp(&*backend, path, name) {
                            local.push(p);
                        }
                    }
                    let _ = tx.send(local);
                })
                .ok();
            return;
        }
        let has_dir = self
            .entries
            .iter()
            .any(|e| e.is_dir && self.selection.contains(&e.path));

        // Filter-aware copy: when a filter is active and folders are selected,
        // build a virtual-file data object so pasting (anywhere) recreates
        // only the matching files with their folder structure.
        if !cut && has_dir && self.filter_is_active() {
            let seeds: Vec<FileEntry> = self
                .entries
                .iter()
                .filter(|e| self.selection.contains(&e.path))
                .cloned()
                .collect();
            let filter = self.filter.clone();
            let prefix = self.root_prefix();
            let (tx, rx) = unbounded();
            self.clip_prepare_rx = Some(rx);
            self.notice = Some((
                "Sammle gefilterte Dateien…".to_string(),
                std::time::Instant::now(),
            ));
            std::thread::Builder::new()
                .name("clip-prepare".into())
                .spawn(move || {
                    let cf = CompiledFilter::compile(&filter);
                    let mut out: Vec<crate::virtual_clipboard::VirtualFile> = Vec::new();
                    for e in &seeds {
                        if e.is_dir {
                            let parent_norm = e.parent.trim_end_matches('/');
                            let base = format!("{}/", parent_norm);
                            let sub = crate::scanner::collect_recursive(
                                &PathBuf::from(
                                    e.path.replace('/', std::path::MAIN_SEPARATOR_STR),
                                ),
                                false,
                                e.depth + 1,
                            );
                            for s in sub {
                                if !s.is_dir && cf.matches(&s, &prefix) {
                                    let rel = s
                                        .path
                                        .strip_prefix(base.as_str())
                                        .unwrap_or(s.name.as_ref())
                                        .to_string();
                                    out.push(crate::virtual_clipboard::VirtualFile {
                                        abs: s.path.replace('/', "\\"),
                                        rel,
                                        size: s.size,
                                        mtime_ms: s.mtime_ms,
                                    });
                                }
                            }
                        } else {
                            // Explicitly selected files always go along.
                            out.push(crate::virtual_clipboard::VirtualFile {
                                abs: e.path.replace('/', "\\"),
                                rel: e.name.to_string(),
                                size: e.size,
                                mtime_ms: e.mtime_ms,
                            });
                        }
                    }
                    let _ = tx.send(out);
                })
                .ok();
            return;
        }

        // Plain CF_HDROP path (no filter, or cut, or files only).
        let paths: Vec<String> = self.selection.iter().map(|p| p.replace('/', "\\")).collect();
        let effect = if cut {
            crate::shell_clipboard::DROPEFFECT_MOVE
        } else {
            crate::shell_clipboard::DROPEFFECT_COPY
        };
        match crate::shell_clipboard::write_files(&paths, effect) {
            Ok(_) => {
                self.virtual_clip = None;
                let hint = if cut && has_dir && self.filter_is_active() {
                    " — Hinweis: Ausschneiden überträgt ganze Ordner, Filter gelten dabei nicht"
                } else {
                    ""
                };
                self.notice = Some((
                    format!(
                        "✓ {} Datei(en) {} — in Explorer einfügbar mit Ctrl+V{}",
                        paths.len(),
                        if cut { "ausgeschnitten" } else { "kopiert" },
                        hint
                    ),
                    std::time::Instant::now(),
                ));
            }
            Err(e) => {
                self.error_msg = Some(format!("Zwischenablage: {}", e));
            }
        }
    }

    #[cfg(windows)]
    fn clipboard_paste_files(&mut self) {
        if self.root_path.is_empty() {
            self.notice = Some((
                "Ctrl+V: kein Zielordner geöffnet".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        // Remote view → upload the clipboard's files into the current remote
        // folder via the backend (instead of a local std::fs copy).
        if let Some(rs) = &self.remote {
            let paths = match crate::shell_clipboard::read_files() {
                Some((p, _)) if !p.is_empty() => p,
                _ => {
                    self.notice = Some((
                        "Ctrl+V: Zwischenablage enthält keine Dateien".to_string(),
                        std::time::Instant::now(),
                    ));
                    return;
                }
            };
            self.start_remote_upload(paths, rs.backend.clone(), self.root_path.clone());
            return;
        }

        let dest = PathBuf::from(self.root_path.replace('/', std::path::MAIN_SEPARATOR_STR));

        // Fast path: the clipboard still holds OUR filtered virtual files —
        // copy them directly without the COM stream round-trip.
        if let Some((seq, pairs)) = self.virtual_clip.clone() {
            if crate::virtual_clipboard::clipboard_sequence() == seq {
                self.notice = Some((
                    format!("📥 Einfügen (gefiltert): {} Datei(en)", pairs.len()),
                    std::time::Instant::now(),
                ));
                let (tx, rx) = unbounded();
                let h = crate::copy::start_copy_pairs(pairs, dest, Conflict::Rename, tx);
                self.copy_handle = Some(h);
                self.copy_rx = Some(rx);
                self.copy_progress = Some(CopyProgress {
                    files_done: 0,
                    files_total: 0,
                    bytes_done: 0,
                    bytes_total: 0,
                    elapsed_ms: 0,
                    current_path: String::new(),
                    errors: 0,
                    done: false,
                });
                self.copy_refresh_after = true;
                return;
            } else {
                self.virtual_clip = None;
            }
        }

        let (paths, is_cut) = match crate::shell_clipboard::read_files() {
            Some(v) => v,
            None => {
                self.notice = Some((
                    "Ctrl+V erkannt — aber Zwischenablage enthält keine Dateien".to_string(),
                    std::time::Instant::now(),
                ));
                return;
            }
        };
        if paths.is_empty() {
            self.notice = Some((
                "Ctrl+V erkannt — Zwischenablage enthält keine Dateien".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        self.notice = Some((
            format!(
                "📥 Füge {} {} ein…",
                paths.len(),
                if is_cut { "Datei(en) (verschieben)" } else { "Datei(en)" }
            ),
            std::time::Instant::now(),
        ));
        let common_parent = PathBuf::from(&paths[0])
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let opts = CopyOptions {
            root: common_parent,
            dest,
            preserve_structure: true,
            conflict: Conflict::Rename,
            mode: if is_cut { CopyMode::Move } else { CopyMode::Copy },
        };
        let (tx, rx) = crossbeam_channel::unbounded();
        let h = start_copy_from_paths(paths, opts, tx);
        self.copy_handle = Some(h);
        self.copy_rx = Some(rx);
        self.copy_progress = Some(CopyProgress {
            files_done: 0,
            files_total: 0,
            bytes_done: 0,
            bytes_total: 0,
            elapsed_ms: 0,
            current_path: String::new(),
            errors: 0,
            done: false,
        });
        self.copy_refresh_after = true;
    }

    #[cfg(not(windows))]
    fn clipboard_copy_files(&mut self, _cut: bool) {}
    #[cfg(not(windows))]
    fn clipboard_paste_files(&mut self) {}

    // ─── Drag-and-drop into the app ─────────────────────────────────────

    /// Copy (or move) OS paths into `dest`, on the copy worker. Conflicts
    /// auto-rename so a drop never overwrites. Shared by the OS drop handler.
    fn copy_paths_into(&mut self, paths: Vec<String>, dest: PathBuf, move_files: bool) {
        if paths.is_empty() {
            return;
        }
        if self.copy_progress.as_ref().map(|p| !p.done).unwrap_or(false) {
            self.error_msg = Some("Es läuft bereits ein Kopiervorgang.".to_string());
            return;
        }
        let common_parent = PathBuf::from(&paths[0])
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let opts = CopyOptions {
            root: common_parent,
            dest,
            preserve_structure: true,
            conflict: Conflict::Rename,
            mode: if move_files { CopyMode::Move } else { CopyMode::Copy },
        };
        let (tx, rx) = crossbeam_channel::unbounded();
        let h = start_copy_from_paths(paths, opts, tx);
        self.copy_handle = Some(h);
        self.copy_rx = Some(rx);
        self.copy_progress = Some(CopyProgress {
            files_done: 0,
            files_total: 0,
            bytes_done: 0,
            bytes_total: 0,
            elapsed_ms: 0,
            current_path: String::new(),
            errors: 0,
            done: false,
        });
        self.copy_refresh_after = true;
    }

    /// Whether the current view can accept dropped files — a local folder, or a
    /// remote folder (files are uploaded via the backend).
    fn drop_target(&self) -> Option<String> {
        if self.root_path.is_empty() {
            None
        } else if self.remote.is_some() || is_local_style(&self.root_path) {
            Some(self.root_path.clone())
        } else {
            None
        }
    }

    /// Handle files dropped onto the window from the OS (Explorer, desktop, …).
    /// They land in the current folder — copy by default, move with Shift held.
    fn handle_os_drop(&mut self, ctx: &egui::Context) {
        let (paths, shift) = ctx.input(|i| {
            let p: Vec<String> = i
                .raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.as_ref())
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            (p, i.modifiers.shift)
        });
        if paths.is_empty() {
            return;
        }
        // Remote view → upload the dropped files into the current remote folder.
        if let Some(rs) = &self.remote {
            self.start_remote_upload(paths, rs.backend.clone(), self.root_path.clone());
            return;
        }
        let dest = match self.drop_target() {
            Some(p) => PathBuf::from(p.replace('/', std::path::MAIN_SEPARATOR_STR)),
            None => {
                self.error_msg =
                    Some("Ablegen nur in einem lokalen Ordner möglich.".to_string());
                return;
            }
        };
        let n = paths.len();
        self.copy_paths_into(paths, dest, shift);
        self.notice = Some((
            format!(
                "📥 {} Element(e) werden {}…",
                n,
                if shift { "verschoben" } else { "kopiert" }
            ),
            std::time::Instant::now(),
        ));
    }

    /// Which tab a screen point drops onto — a tab header, or (in split) a
    /// pane. None if over neither.
    fn drop_target_tab(&self, p: egui::Pos2) -> Option<usize> {
        if let Some((i, _)) = self.tab_header_rects.iter().find(|(_, r)| r.contains(p)) {
            return Some(*i);
        }
        if let Some((i, _)) = self.pane_rects.iter().find(|(_, r)| r.contains(p)) {
            return Some(*i);
        }
        None
    }

    /// Drop the dragged files into tab `t`'s folder. Handles every combination
    /// of local/remote source and target: local→local copy/move, local→remote
    /// upload, remote→local download. Remote→remote isn't supported yet.
    fn drop_files_into_tab(&mut self, t: usize, move_files: bool) {
        // Target backend: Some(handle) if the target tab is a remote view.
        let (dest_str, tgt_backend) = if t == self.active_tab {
            (self.root_path.clone(), self.remote.as_ref().map(|rs| rs.backend.clone()))
        } else {
            match self.tabs.get(t) {
                Some(x) => (x.root_path.clone(), x.remote.as_ref().map(|rs| rs.backend.clone())),
                None => return,
            }
        };
        if dest_str.is_empty() {
            return;
        }
        let dest_fwd = dest_str.trim_end_matches('/').to_string();
        let files: Vec<String> = std::mem::take(&mut self.drag_files)
            .into_iter()
            .filter(|p| p.rsplit_once('/').map(|(par, _)| par) != Some(dest_fwd.as_str()))
            .collect();
        let src_backend = self.drag_src.take();
        if files.is_empty() {
            self.notice = Some((
                "Dateien sind bereits im Ziel-Ordner.".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        let n = files.len();
        match (src_backend, tgt_backend) {
            // local → local
            (None, None) => {
                if !is_local_style(&dest_fwd) {
                    self.error_msg = Some("Ziel ist kein lokaler Ordner.".to_string());
                    return;
                }
                let dest = PathBuf::from(dest_fwd.replace('/', std::path::MAIN_SEPARATOR_STR));
                self.copy_paths_into(files, dest, move_files);
                self.notice = Some((format!("{} Element(e) werden kopiert…", n), std::time::Instant::now()));
            }
            // local → remote (upload)
            (None, Some(be)) => {
                self.start_remote_upload(files, be, dest_fwd);
            }
            // remote → local (download)
            (Some(be), None) => {
                if !is_local_style(&dest_fwd) {
                    self.error_msg = Some("Ziel ist kein lokaler Ordner.".to_string());
                    return;
                }
                self.start_remote_download(be, files, dest_fwd);
            }
            // remote → remote
            // remote → remote (cross-backend: download to temp, then upload)
            (Some(src), Some(tgt)) => {
                self.start_remote_to_remote(src, files, tgt, dest_fwd);
            }
        }
    }

    /// Copy remote `files` into another remote folder by streaming each through a
    /// temp file (download from `src`, upload to `tgt`/dest). Off the UI thread;
    /// reuses the transfer result channel.
    fn start_remote_to_remote(
        &mut self,
        src: crate::vfs::BackendHandle,
        files: Vec<String>,
        tgt: crate::vfs::BackendHandle,
        dest_root: String,
    ) {
        if self.upload_rx.is_some() {
            self.notice = Some(("Es läuft bereits eine Übertragung…".to_string(), std::time::Instant::now()));
            return;
        }
        let n = files.len();
        let (tx, rx) = unbounded();
        std::thread::Builder::new()
            .name("remote-to-remote".into())
            .spawn(move || {
                let mut copied = 0u64;
                let mut errors = Vec::new();
                for p in &files {
                    let name = p.trim_end_matches('/').rsplit('/').next().unwrap_or("datei");
                    let tmp = open_temp_path(name);
                    let dest = format!("{}/{}", dest_root.trim_end_matches('/'), name);
                    let r = download_to(&*src, p, &tmp)
                        .and_then(|_| upload_file(&*tgt, &tmp, &dest));
                    match r {
                        Ok(_) => copied += 1,
                        Err(e) => errors.push(format!("{}: {}", name, e)),
                    }
                    let _ = std::fs::remove_file(&tmp);
                }
                let _ = tx.send((copied, errors));
            })
            .ok();
        self.upload_rx = Some(rx);
        self.notice = Some((format!("⇄ Übertrage {} Element(e) (Remote→Remote)…", n), std::time::Instant::now()));
    }

    /// Download remote `files` into a local folder, off the UI thread (reuses
    /// the upload result channel for the completion notice).
    fn start_remote_download(
        &mut self,
        backend: crate::vfs::BackendHandle,
        files: Vec<String>,
        dest_local: String,
    ) {
        if self.upload_rx.is_some() {
            self.notice = Some(("Es läuft bereits eine Übertragung…".to_string(), std::time::Instant::now()));
            return;
        }
        let n = files.len();
        let (tx, rx) = unbounded();
        std::thread::Builder::new()
            .name("remote-download-multi".into())
            .spawn(move || {
                let mut copied = 0u64;
                let mut errors = Vec::new();
                for p in &files {
                    let name = p.trim_end_matches('/').rsplit('/').next().unwrap_or("datei");
                    let dest = std::path::Path::new(&dest_local).join(name);
                    match download_to(&*backend, p, &dest) {
                        Ok(_) => copied += 1,
                        Err(e) => errors.push(format!("{}: {}", name, e)),
                    }
                }
                let _ = tx.send((copied, errors));
            })
            .ok();
        self.upload_rx = Some(rx);
        self.notice = Some((format!("⬇ Lade {} Element(e) herunter…", n), std::time::Instant::now()));
    }

    /// Drive an active internal file drag each frame: paint a cursor chip,
    /// route a drop onto another tab/pane, and (Windows) hand the drag off to
    /// Explorer once the pointer leaves the window.
    fn handle_file_drag(&mut self, ctx: &egui::Context) {
        if !self.drag_active {
            return;
        }
        let (down, released, pos, shift) = ctx.input(|i| {
            (
                i.pointer.primary_down(),
                i.pointer.any_released(),
                i.pointer.latest_pos(),
                i.modifiers.shift,
            )
        });

        // Drag OUT to Explorer (Windows): once the pointer leaves the window
        // while still dragging, hand the files to the OS drag loop (blocks until
        // the drop completes), then refresh in case it was a move.
        #[cfg(windows)]
        if down && !self.drag_out_started {
            if let Some(p) = pos {
                if !ctx.screen_rect().contains(p) {
                    self.drag_out_started = true;
                    self.drag_active = false;
                    let files = std::mem::take(&mut self.drag_files);
                    // Remote source → materialize to temp copies first (Explorer
                    // needs real local paths). May briefly block on the download.
                    let files = if let Some(be) = self.drag_src.take() {
                        files
                            .iter()
                            .filter_map(|p| {
                                let name = p.trim_end_matches('/').rsplit('/').next().unwrap_or("datei");
                                download_to(&*be, p, &open_temp_path(name)).ok()
                            })
                            .collect()
                    } else {
                        files
                    };
                    crate::dragout::drag_out(&files);
                    self.rescan();
                    return;
                }
            }
        }

        if down {
            // Floating chip near the cursor.
            if let Some(p) = pos {
                let n = self.drag_files.len();
                let painter = ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Tooltip,
                    egui::Id::new("file_drag_chip"),
                ));
                let text = format!(
                    "📄 {} Element(e){}",
                    n,
                    if shift { " — verschieben" } else { "" }
                );
                let galley =
                    painter.layout_no_wrap(text, egui::FontId::proportional(13.0), Color32::WHITE);
                let pad = egui::vec2(8.0, 4.0);
                let origin = p + egui::vec2(14.0, 8.0);
                let rect = egui::Rect::from_min_size(origin, galley.size() + pad * 2.0);
                painter.rect_filled(rect, 4.0, Color32::from_rgb(40, 90, 140));
                painter.galley(origin + pad, galley, Color32::WHITE);
            }
            ctx.request_repaint();
            return;
        }

        // Released inside the window → route to a target tab/pane.
        if released {
            if let Some(t) = pos.and_then(|p| self.drop_target_tab(p)) {
                if t != self.drag_source_tab {
                    self.drop_files_into_tab(t, shift);
                }
            }
            self.drag_active = false;
            self.drag_files.clear();
            self.drag_src = None;
        }
    }

    // ─── In-app folder picker (#17) ─────────────────────────────────────

    /// Open the picker to fill a sync-setup field, starting from `initial`
    /// (local path → browse there; remote URL or empty → start at the roots).
    fn open_picker(&mut self, field: PickerField, initial: &str) {
        let mut st = PickerState {
            field,
            backend: None,
            is_remote: false,
            endpoint_prefix: String::new(),
            conn_label: String::new(),
            cwd: String::new(),
            entries: Vec::new(),
            error: None,
            connect_rx: None,
            connecting: false,
        };
        // A local starting folder opens directly; remote/empty starts at roots.
        if !initial.trim().is_empty()
            && !crate::connect::is_remote_url(initial)
            && is_local_style(initial)
        {
            st.backend = Some(Arc::new(crate::vfs::LocalBackend::new("/")));
            st.cwd = initial.replace('\\', "/").trim_end_matches('/').to_string();
            if st.cwd.is_empty() {
                st.cwd = "/".into();
            }
        }
        self.picker = Some(st);
        if self.picker.as_ref().map(|s| s.backend.is_some()).unwrap_or(false) {
            self.picker_list();
        }
    }

    /// (Re)list the current picker folder via its backend (folders only).
    fn picker_list(&mut self) {
        let (backend, cwd) = match &self.picker {
            Some(p) => match &p.backend {
                Some(b) => (b.clone(), ensure_dir_root(&p.cwd)),
                None => return,
            },
            None => return,
        };
        let res = backend.list_dir(&cwd);
        if let Some(p) = self.picker.as_mut() {
            match res {
                Ok(metas) => {
                    let mut dirs: Vec<String> = metas
                        .into_iter()
                        .filter(|m| m.is_dir)
                        .map(|m| m.name)
                        .collect();
                    dirs.sort_by_key(|n| n.to_lowercase());
                    p.entries = dirs;
                    p.error = None;
                }
                Err(e) => {
                    p.entries.clear();
                    p.error = Some(e.to_string());
                }
            }
        }
    }

    /// Open a local drive / folder root in the picker.
    fn picker_open_local(&mut self, root: &str) {
        if let Some(p) = self.picker.as_mut() {
            p.backend = Some(Arc::new(crate::vfs::LocalBackend::new("/")));
            p.is_remote = false;
            p.endpoint_prefix = String::new();
            p.conn_label = String::new();
            let c = root.replace('\\', "/");
            let c = c.trim_end_matches('/');
            p.cwd = if c.is_empty() { "/".into() } else { ensure_dir_root(c) };
            p.connecting = false;
            p.connect_rx = None;
        }
        self.picker_list();
    }

    /// Open a saved connection in the picker (async connect; keeps creds).
    fn picker_open_connection(&mut self, c: &crate::creds::SavedConnection) {
        let form = crate::connect::ConnectForm::from_saved(c);
        let secret = crate::creds::get_secret(&c.account());
        let rx = crate::connect::spawn_connect(form, secret);
        if let Some(p) = self.picker.as_mut() {
            p.connect_rx = Some(rx);
            p.connecting = true;
            p.error = None;
            p.conn_label = c.display();
            p.is_remote = c.protocol.is_url();
            p.endpoint_prefix = if c.protocol.is_url() {
                format!("{}://{}@{}:{}", c.protocol.as_str(), c.user, c.host, c.port)
            } else {
                String::new()
            };
        }
    }

    fn drain_picker_connect(&mut self) {
        let msg = match self
            .picker
            .as_ref()
            .and_then(|p| p.connect_rx.as_ref())
            .and_then(|rx| rx.try_recv().ok())
        {
            Some(m) => m,
            None => return,
        };
        let mut do_list = false;
        if let Some(p) = self.picker.as_mut() {
            p.connect_rx = None;
            p.connecting = false;
            match msg {
                crate::connect::ConnectResult::Ok(c) => {
                    // SFTP/FTP/WebDAV → remote backend; share → browse the UNC
                    // locally once authenticated.
                    if let Some(rs) = c.remote {
                        p.backend = Some(rs.backend);
                        p.is_remote = true;
                    } else {
                        p.backend = Some(Arc::new(crate::vfs::LocalBackend::new(&c.target)));
                        p.is_remote = false;
                        p.endpoint_prefix = String::new();
                    }
                    p.cwd = c.target;
                    do_list = true;
                }
                crate::connect::ConnectResult::Err(e) => {
                    p.error = Some(format!("Verbindung fehlgeschlagen: {}", e));
                }
            }
        }
        if do_list {
            self.picker_list();
        }
    }

    /// Parent of a picker directory (None at a drive/remote root).
    fn picker_parent(p: &str) -> Option<String> {
        let t = p.trim_end_matches('/');
        if t.is_empty() || t == "/" {
            return None;
        }
        if t.len() == 2 && t.ends_with(':') {
            return None; // drive root "C:"
        }
        match t.rsplit_once('/') {
            Some((par, _)) => {
                if par.is_empty() {
                    Some("/".into())
                } else if par.len() == 2 && par.ends_with(':') {
                    Some(format!("{}/", par))
                } else {
                    Some(par.to_string())
                }
            }
            None => None,
        }
    }

    /// The value the picker would return for the current folder.
    fn picker_value(p: &PickerState) -> String {
        if p.is_remote {
            format!("{}{}", p.endpoint_prefix, p.cwd)
        } else {
            p.cwd.clone()
        }
    }

    fn ui_picker(&mut self, ctx: &egui::Context) {
        if self.picker.is_none() {
            return;
        }
        let mut open = true;
        let mut close = false;
        let mut choose = false;
        let mut enter: Option<String> = None;
        let mut go_up = false;
        let mut open_local: Option<String> = None;
        let mut open_conn: Option<crate::creds::SavedConnection> = None;

        let st = self.picker.as_ref().unwrap();
        let title = match st.field {
            PickerField::Source => "📂 Quelle wählen",
            PickerField::Target => "📂 Ziel wählen",
        };
        let home = self.home.to_string_lossy().replace('\\', "/");
        let drives = self.drive_info.clone();
        let conns = self.saved_connections.clone();
        let connecting = st.connecting;
        let error = st.error.clone();
        let cwd = st.cwd.clone();
        let entries = st.entries.clone();
        let conn_label = st.conn_label.clone();
        let value_preview = Self::picker_value(st);
        let has_loc = st.backend.is_some();
        let gdrive_connected = crate::cloud::is_connected(crate::cloud::Provider::GDrive);
        let mut open_gdrive = false;

        egui::Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([720.0, 460.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // ── Left: places ──
                    ui.vertical(|ui| {
                        ui.set_min_width(200.0);
                        ui.label(RichText::new("ORTE").small().color(Color32::from_gray(140)));
                        if ui.selectable_label(false, "🏠 Home").clicked() {
                            open_local = Some(home.clone());
                        }
                        for (d, _f, _t) in &drives {
                            if ui.selectable_label(false, format!("💽 {}", d)).clicked() {
                                open_local = Some(d.clone());
                            }
                        }
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("VERBINDUNGEN")
                                .small()
                                .color(Color32::from_gray(140)),
                        );
                        if conns.is_empty() && !gdrive_connected {
                            ui.colored_label(Color32::from_gray(120), "(keine)");
                        }
                        if gdrive_connected
                            && ui.selectable_label(false, "☁ Google Drive").clicked()
                        {
                            open_gdrive = true;
                        }
                        for c in &conns {
                            if ui
                                .selectable_label(false, format!("🖧 {}", c.display()))
                                .on_hover_text(c.to_target())
                                .clicked()
                            {
                                open_conn = Some(c.clone());
                            }
                        }
                    });

                    ui.separator();

                    // ── Right: current folder ──
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            if ui.button("⬆ Hoch").clicked() {
                                go_up = true;
                            }
                            if !conn_label.is_empty() {
                                ui.colored_label(Color32::from_rgb(120, 200, 255), format!("● {}", conn_label));
                            }
                        });
                        ui.label(
                            RichText::new(if cwd.is_empty() { "—".to_string() } else { cwd.clone() })
                                .monospace()
                                .color(Color32::from_gray(180)),
                        );
                        ui.separator();
                        if connecting {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label("Verbinde…");
                            });
                        } else if let Some(e) = &error {
                            ui.colored_label(Color32::from_rgb(255, 140, 120), e);
                        } else if !has_loc {
                            ui.colored_label(
                                Color32::from_gray(140),
                                "Links einen Ort oder eine Verbindung wählen.",
                            );
                        }
                        egui::ScrollArea::vertical()
                            .id_salt("picker_list")
                            .max_height(300.0)
                            .show(ui, |ui| {
                                for name in &entries {
                                    if ui
                                        .selectable_label(false, format!("📁 {}", name))
                                        .double_clicked()
                                    {
                                        enter = Some(name.clone());
                                    }
                                }
                                if has_loc && entries.is_empty() && error.is_none() && !connecting {
                                    ui.colored_label(Color32::from_gray(120), "(keine Unterordner)");
                                }
                            });
                    });
                });

                ui.separator();
                ui.horizontal(|ui| {
                    let can_choose = has_loc && !connecting && !cwd.is_empty();
                    if ui
                        .add_enabled(can_choose, egui::Button::new("✔ Diesen Ordner wählen"))
                        .clicked()
                    {
                        choose = true;
                    }
                    if ui.button("Abbrechen").clicked() {
                        close = true;
                    }
                    if can_choose {
                        ui.colored_label(Color32::from_gray(140), value_preview.clone());
                    }
                });
            });

        // Apply deferred actions (outside the borrow of self.picker).
        if let Some(name) = enter {
            if let Some(p) = self.picker.as_mut() {
                p.cwd = format!("{}/{}", p.cwd.trim_end_matches('/'), name);
            }
            self.picker_list();
        }
        if go_up {
            let parent = self.picker.as_ref().and_then(|p| Self::picker_parent(&p.cwd));
            if let Some(par) = parent {
                if let Some(p) = self.picker.as_mut() {
                    p.cwd = par;
                }
                self.picker_list();
            }
        }
        if let Some(root) = open_local {
            self.picker_open_local(&root);
        }
        if let Some(c) = open_conn {
            self.picker_open_connection(&c);
        }
        if open_gdrive {
            self.picker_open_gdrive();
        }
        if choose {
            if let Some(p) = self.picker.take() {
                let value = Self::picker_value(&p);
                if let Some(ed) = self.job_editor.as_mut() {
                    match p.field {
                        PickerField::Source => ed.source = value,
                        PickerField::Target => ed.target = value,
                    }
                }
            }
        } else if close || !open {
            self.picker = None;
        }
    }

    /// Full-window hint shown while files are dragged over the app.
    fn ui_drop_overlay(&self, ctx: &egui::Context) {
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("drop_overlay"),
        ));
        let rect = ctx.screen_rect();
        painter.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(0, 0, 0, 110));
        let (text, color) = match self.drop_target() {
            Some(p) => (
                format!("📥 Hier ablegen → {}\n(Umschalt = verschieben)", p),
                Color32::from_rgb(150, 220, 255),
            ),
            None => (
                "Ablegen nur in einem lokalen Ordner möglich".to_string(),
                Color32::from_rgb(255, 185, 120),
            ),
        };
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(22.0),
            color,
        );
    }

    // ─── Context menus ──────────────────────────────────────────────────

    #[cfg(windows)]
    fn show_shell_menu_for(&mut self, clicked_path: &str, ctx: &egui::Context) {
        use crate::shell_menu::{MenuResult, OwnMenuItem};

        let clicked_arc: Arc<str> = Arc::from(clicked_path);
        let paths: Vec<String> = if self.selection.contains(&clicked_arc) && self.selection.len() > 1
        {
            self.selection.iter().map(|p| p.replace('/', "\\")).collect()
        } else {
            vec![clicked_path.replace('/', "\\")]
        };

        let filter_active = self.filter_is_active();
        let own = vec![
            OwnMenuItem {
                id: menu_ids::COPY,
                label: if filter_active {
                    "Kopieren (mit Filter)".to_string()
                } else {
                    "Kopieren".to_string()
                },
            },
            OwnMenuItem {
                id: menu_ids::CUT,
                label: "Ausschneiden".to_string(),
            },
            OwnMenuItem {
                id: menu_ids::COPY_PATH,
                label: "Pfad kopieren".to_string(),
            },
            OwnMenuItem {
                id: menu_ids::COPY_TO,
                label: "Kopieren nach… (Filter + Struktur)".to_string(),
            },
            OwnMenuItem {
                id: menu_ids::MOVE_TO,
                label: "Verschieben nach…".to_string(),
            },
            OwnMenuItem {
                id: menu_ids::RENAME,
                label: "Umbenennen (F2)".to_string(),
            },
        ];

        // Offer a favorite toggle when the clicked entry is a folder.
        let clicked_fwd = clicked_path.replace('\\', "/");
        let clicked_is_dir = self
            .entries
            .iter()
            .any(|e| e.is_dir && e.path.as_ref() == clicked_fwd);
        let mut own = own;
        if clicked_is_dir {
            own.push(OwnMenuItem {
                id: menu_ids::TOGGLE_FAV,
                label: if self.is_favorite(&clicked_fwd) {
                    "☆ Aus Favoriten entfernen".to_string()
                } else {
                    "★ Zu Favoriten".to_string()
                },
            });
        }

        match crate::shell_menu::show_for_paths(&paths, None, None, &own) {
            Ok(MenuResult::Own(id)) => match id {
                menu_ids::COPY => self.clipboard_copy_files(false),
                menu_ids::CUT => self.clipboard_copy_files(true),
                menu_ids::COPY_PATH => self.copy_paths_to_clipboard(ctx),
                menu_ids::COPY_TO => {
                    self.copy_mode_pending = CopyMode::Copy;
                    self.copy_open = true;
                }
                menu_ids::MOVE_TO => {
                    self.copy_mode_pending = CopyMode::Move;
                    self.copy_open = true;
                }
                menu_ids::RENAME => self.open_rename(),
                menu_ids::TOGGLE_FAV => self.toggle_favorite(&clicked_fwd),
                _ => {}
            },
            Ok(MenuResult::Shell) => {
                // The shell verb may have changed the directory (delete,
                // rename, …) — refresh.
                self.rescan();
            }
            _ => {}
        }
    }

    #[cfg(not(windows))]
    fn show_shell_menu_for(&mut self, clicked_path: &str, _ctx: &egui::Context) {
        self.open_in_explorer(clicked_path);
    }

    #[cfg(windows)]
    fn show_background_menu(&mut self) {
        use crate::shell_menu::{MenuResult, OwnMenuItem};
        if self.root_path.is_empty() {
            return;
        }
        let own = vec![
            OwnMenuItem {
                id: menu_ids::PASTE,
                label: "Einfügen".to_string(),
            },
            OwnMenuItem {
                id: menu_ids::NEW_FOLDER,
                label: "Neuer Ordner (Ctrl+Shift+N)".to_string(),
            },
            OwnMenuItem {
                id: menu_ids::SELECT_ALL,
                label: "Alles auswählen (Ctrl+A)".to_string(),
            },
            OwnMenuItem {
                id: menu_ids::REFRESH,
                label: "Aktualisieren (F5)".to_string(),
            },
        ];
        let folder = self.root_path.replace('/', "\\");
        match crate::shell_menu::show_background_menu(&folder, &own) {
            Ok(MenuResult::Own(id)) => match id {
                menu_ids::PASTE => self.clipboard_paste_files(),
                menu_ids::NEW_FOLDER => self.create_new_folder(),
                menu_ids::SELECT_ALL => self.select_all(),
                menu_ids::REFRESH => self.rescan(),
                _ => {}
            },
            Ok(MenuResult::Shell) => self.rescan(),
            _ => {}
        }
    }

    #[cfg(not(windows))]
    fn show_background_menu(&mut self) {}

    // ── UI ────────────────────────────────────────────────────────────────

    fn ui_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.history.is_empty(), egui::Button::new("◀"))
                .on_hover_text("Zurück (Alt+←)")
                .clicked()
            {
                self.navigate_back();
            }
            if ui
                .add_enabled(!self.forward.is_empty(), egui::Button::new("▶"))
                .on_hover_text("Vor (Alt+→)")
                .clicked()
            {
                self.navigate_forward();
            }
            if ui
                .add_enabled(!self.root_path.is_empty(), egui::Button::new("↑"))
                .on_hover_text("Eine Ebene hoch (Alt+↑ / Backspace)")
                .clicked()
            {
                self.navigate_up();
            }

            if ui.button("📂").on_hover_text("Ordner auswählen").clicked() {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    self.start_scan(p);
                }
            }

            // ─── Breadcrumbs / editable path ───────────────────────────
            let crumb_w = (ui.available_width() - 660.0).max(160.0);
            if self.path_edit_mode {
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.root_path)
                        .desired_width(crumb_w)
                        .hint_text("Pfad eingeben…"),
                );
                if self.path_edit_focus {
                    resp.request_focus();
                    self.path_edit_focus = false;
                }
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if enter && !self.root_path.is_empty() {
                    self.path_edit_mode = false;
                    let p = PathBuf::from(
                        self.root_path.replace('/', std::path::MAIN_SEPARATOR_STR),
                    );
                    self.start_scan(p);
                } else if resp.lost_focus() {
                    self.path_edit_mode = false;
                }
            } else {
                let mut nav_to: Option<String> = None;
                ui.allocate_ui(egui::vec2(crumb_w, 22.0), |ui| {
                    egui::ScrollArea::horizontal()
                        .id_salt("crumbs")
                        .max_width(crumb_w)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let prefix = self.root_prefix();
                                if prefix.is_empty() {
                                    ui.colored_label(
                                        Color32::from_gray(120),
                                        "Ordner wählen oder Pfad eingeben (Ctrl+L)",
                                    );
                                } else {
                                    // Keep the leading separator(s) so absolute
                                    // remote paths ("/home/…") and UNC ("//srv/…")
                                    // stay absolute when a crumb is clicked —
                                    // otherwise the root became relative and
                                    // failed with "Wurzel kann nicht gelesen werden".
                                    let lead: String =
                                        prefix.chars().take_while(|&c| c == '/').collect();
                                    let mut acc = lead;
                                    let segs: Vec<&str> =
                                        prefix.split('/').filter(|s| !s.is_empty()).collect();
                                    for (i, seg) in segs.iter().enumerate() {
                                        if i > 0 {
                                            ui.label(RichText::new("›").color(Color32::from_gray(110)));
                                        }
                                        acc.push_str(seg);
                                        acc.push('/');
                                        let full = acc.clone();
                                        if ui.small_button(*seg).clicked() {
                                            nav_to = Some(full);
                                        }
                                    }
                                }
                            });
                        });
                });
                if ui
                    .small_button("✏")
                    .on_hover_text("Pfad bearbeiten (Ctrl+L)")
                    .clicked()
                {
                    self.path_edit_mode = true;
                    self.path_edit_focus = true;
                }
                if let Some(p) = nav_to {
                    self.start_scan(PathBuf::from(
                        p.trim_end_matches('/')
                            .replace('/', std::path::MAIN_SEPARATOR_STR),
                    ));
                }
            }

            if self.scan_running {
                if ui.button("⏹ Stop").clicked() {
                    self.cancel_scan();
                }
            } else if ui.button("⟳").on_hover_text("Aktualisieren (F5)").clicked() {
                self.rescan();
            }

            let was_recursive = self.recursive;
            ui.toggle_value(&mut self.recursive, "🔁 Rekursiv")
                .on_hover_text("Inkl. Unterordner durchsuchen (Ctrl+R)");
            if was_recursive != self.recursive && !self.root_path.is_empty() {
                self.rescan();
            }

            ui.separator();

            let has_sel = !self.selection.is_empty();
            // Grouped feature menus (moved off the sidebar). Copy/cut/paste stay
            // on Ctrl+C/X/V and the right-click menu — out of the nav bar.
            ui.menu_button("🔌 Verbindung", |ui| {
                ui.set_min_width(330.0);
                self.ui_menu_connect(ui);
            });
            ui.menu_button("⇄ Sync", |ui| {
                ui.set_min_width(330.0);
                self.ui_menu_sync(ui);
            });
            ui.menu_button("⚙ Einstellungen", |ui| {
                ui.set_min_width(350.0);
                self.ui_menu_settings(ui);
            });
            if ui
                .selectable_label(self.show_share, "📡 Teilen")
                .on_hover_text("Dateien direkt an gekoppelte Geräte / in Räume senden (P2P, verschlüsselt)")
                .clicked()
            {
                self.show_share = !self.show_share;
            }
            ui.separator();
            if ui
                .add_enabled(has_sel, egui::Button::new("🗑").small())
                .on_hover_text("Entf — in Papierkorb")
                .clicked()
            {
                self.trash_selected();
            }
            // "Neu" dropdown: folder + various editable file types.
            enum NewKind {
                Folder,
                File(&'static str, &'static str),
            }
            let mut new_kind: Option<NewKind> = None;
            ui.add_enabled_ui(!self.root_path.is_empty(), |ui| {
                ui.menu_button("➕ Neu", |ui| {
                    if ui.button("📁 Ordner").clicked() {
                        new_kind = Some(NewKind::Folder);
                        ui.close_menu();
                    }
                    ui.separator();
                    for (label, base, ext) in [
                        ("📄 Textdatei (.txt)", "Neue Textdatei", "txt"),
                        ("📝 Markdown (.md)", "Neue Notiz", "md"),
                        ("📊 CSV (.csv)", "Neue Tabelle", "csv"),
                        ("🔧 JSON (.json)", "Neue Datei", "json"),
                        ("🌐 HTML (.html)", "Neue Seite", "html"),
                        ("</> Code (.rs)", "Neue Datei", "rs"),
                    ] {
                        if ui.button(label).clicked() {
                            new_kind = Some(NewKind::File(base, ext));
                            ui.close_menu();
                        }
                    }
                })
                .response
                .on_hover_text("Neu: Ordner oder Datei (Ctrl+Shift+N = Ordner)");
            });
            match new_kind {
                Some(NewKind::Folder) => self.create_new_folder(),
                Some(NewKind::File(base, ext)) => self.create_new_file(base, ext),
                None => {}
            }
            // Star the current folder
            let starred = !self.root_path.is_empty() && self.is_favorite(&self.root_prefix());
            let star_glyph = if starred { "★" } else { "☆" };
            if ui
                .add_enabled(!self.root_path.is_empty(), egui::Button::new(star_glyph).small())
                .on_hover_text("Aktuellen Ordner zu Favoriten (Ctrl+B)")
                .clicked()
            {
                self.star_current_folder();
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.toggle_value(&mut self.show_summary, "Σ").changed() {
                    self.save_ui_state();
                }
                if ui
                    .toggle_value(&mut self.show_filters, "🔍 Filter")
                    .on_hover_text("Filterleiste ein-/ausklappen")
                    .changed()
                {
                    self.save_ui_state();
                }
                if ui.button("？").on_hover_text("Tastenkürzel (F1)").clicked() {
                    self.show_help = !self.show_help;
                }
                if ui
                    .selectable_label(self.split, "⊟ Split")
                    .on_hover_text("Zwei Tabs nebeneinander (F6)")
                    .clicked()
                {
                    self.toggle_split();
                }
            });
        });
    }

    fn ui_filterbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("textmode")
                .selected_text(match self.filter.text_mode {
                    TextMode::Substring => "enthält",
                    TextMode::Regex => "RegExp",
                    TextMode::Glob => "Glob",
                })
                .show_ui(ui, |ui| {
                    let mut changed = false;
                    changed |= ui
                        .selectable_value(&mut self.filter.text_mode, TextMode::Substring, "enthält")
                        .clicked();
                    changed |= ui
                        .selectable_value(&mut self.filter.text_mode, TextMode::Regex, "RegExp")
                        .clicked();
                    changed |= ui
                        .selectable_value(&mut self.filter.text_mode, TextMode::Glob, "Glob")
                        .clicked();
                    if changed {
                        self.recompute_view();
                    }
                });

            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.text_draft)
                    .hint_text(match self.filter.text_mode {
                        TextMode::Substring => "Suche im Namen…",
                        TextMode::Regex => "Regex z.B. \\.log$",
                        TextMode::Glob => "Glob z.B. **/build/**",
                    })
                    .desired_width(240.0),
            );
            if self.name_filter_focus {
                resp.request_focus();
                self.name_filter_focus = false;
            }
            if resp.changed() {
                self.filter_pending_at = Some(Instant::now());
            }

            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.ext_draft)
                    .hint_text("Endungen z.B. jpg,png")
                    .desired_width(180.0),
            );
            if resp.changed() {
                self.filter_pending_at = Some(Instant::now());
            }

            ui.label("Größe:");
            self.size_input(ui, "size_min", "≥ 10 MB", true);
            self.size_input(ui, "size_max", "≤ 1 GB", false);

            ui.label("Geändert:");
            self.date_filter_ui(ui, true);

            ui.label("Erstellt:");
            self.date_filter_ui(ui, false);

            // Quick presets for the modified-date range
            let mut preset: Option<(Option<chrono::NaiveDate>, Option<chrono::NaiveDate>)> = None;
            egui::ComboBox::from_id_salt("date_preset")
                .selected_text("⏱ Zeitraum")
                .width(110.0)
                .show_ui(ui, |ui| {
                    let today = chrono::Local::now().date_naive();
                    if ui.button("Heute").clicked() {
                        preset = Some((Some(today), None));
                    }
                    if ui.button("Letzte 7 Tage").clicked() {
                        preset = Some((Some(today - chrono::Duration::days(7)), None));
                    }
                    if ui.button("Letzte 30 Tage").clicked() {
                        preset = Some((Some(today - chrono::Duration::days(30)), None));
                    }
                    if ui.button("Dieses Jahr").clicked() {
                        preset = Some((
                            chrono::NaiveDate::from_ymd_opt(
                                chrono::Datelike::year(&today),
                                1,
                                1,
                            ),
                            None,
                        ));
                    }
                    if ui.button("Alle Daten löschen").clicked() {
                        preset = Some((None, None));
                    }
                });
            if let Some((min, max)) = preset {
                self.mtime_min_date = min;
                self.mtime_max_date = max;
                if min.is_none() && max.is_none() {
                    self.btime_min_date = None;
                    self.btime_max_date = None;
                }
                self.apply_date_filters();
                self.recompute_view();
            }
        });

        ui.horizontal(|ui| {
            let mut changed = false;
            changed |= ui.checkbox(&mut self.filter.include_files, "Dateien").changed();
            changed |= ui.checkbox(&mut self.filter.include_dirs, "Ordner").changed();
            changed |= ui.checkbox(&mut self.filter.include_hidden, "versteckt").changed();
            changed |= ui.checkbox(&mut self.filter.include_system, "System").changed();
            if changed {
                self.recompute_view();
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Reset").clicked() {
                    self.filter = FilterDef::new();
                    self.text_draft.clear();
                    self.ext_draft.clear();
                    self.size_min_draft.clear();
                    self.size_max_draft.clear();
                    self.mtime_min_date = None;
                    self.mtime_max_date = None;
                    self.btime_min_date = None;
                    self.btime_max_date = None;
                    self.filter_pending_at = None;
                    self.recompute_view();
                }
                if self.filter_is_active() {
                    ui.colored_label(Color32::from_rgb(255, 190, 90), "● Filter aktiv");
                }
                ui.label(
                    RichText::new(format!(
                        "{} / {} Einträge",
                        self.view.len(),
                        self.entries.len()
                    ))
                    .color(Color32::from_gray(140)),
                );
            });
        });
    }

    fn size_input(&mut self, ui: &mut egui::Ui, id: &str, hint: &str, is_min: bool) {
        let draft = if is_min {
            &mut self.size_min_draft
        } else {
            &mut self.size_max_draft
        };
        let resp = ui.add(
            egui::TextEdit::singleline(draft)
                .id(egui::Id::new(id))
                .hint_text(hint)
                .desired_width(90.0),
        );
        if resp.lost_focus() {
            let parsed = parse_size_input(draft);
            if is_min {
                self.filter.size.min = parsed;
            } else {
                self.filter.size.max = parsed;
            }
            self.recompute_view();
        }
    }

    /// Calendar-based date range input: a "von 📅"/"bis 📅" button that turns
    /// into a date-picker button + clear once set.
    fn date_filter_ui(&mut self, ui: &mut egui::Ui, is_mtime: bool) {
        let mut changed = false;
        for is_min in [true, false] {
            let id = format!(
                "dp_{}_{}",
                if is_mtime { "m" } else { "b" },
                if is_min { "min" } else { "max" }
            );
            let field = match (is_mtime, is_min) {
                (true, true) => &mut self.mtime_min_date,
                (true, false) => &mut self.mtime_max_date,
                (false, true) => &mut self.btime_min_date,
                (false, false) => &mut self.btime_max_date,
            };
            match field {
                Some(d) => {
                    let resp = ui.add(
                        egui_extras::DatePickerButton::new(d)
                            .id_salt(id.as_str())
                            .show_icon(false),
                    );
                    if resp.changed() {
                        changed = true;
                    }
                    if ui.small_button("✕").clicked() {
                        *field = None;
                        changed = true;
                    }
                }
                None => {
                    let label = if is_min { "von 📅" } else { "bis 📅" };
                    if ui.small_button(label).clicked() {
                        *field = Some(chrono::Local::now().date_naive());
                        changed = true;
                    }
                }
            }
        }
        if changed {
            self.apply_date_filters();
            self.recompute_view();
        }
    }

    fn apply_date_filters(&mut self) {
        self.filter.mtime.min = self.mtime_min_date.map(date_to_ms_start);
        self.filter.mtime.max = self.mtime_max_date.map(date_to_ms_end);
        self.filter.btime.min = self.btime_min_date.map(date_to_ms_start);
        self.filter.btime.max = self.btime_max_date.map(date_to_ms_end);
    }

    fn ui_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.heading("Smart Explorer");
        ui.add_space(4.0);

        // ─── Folder fuzzy search ──────────────────────────────────────────
        ui.label(RichText::new("ORDNER SUCHEN").small().color(Color32::from_gray(140)));
        let search_resp = ui.add(
            egui::TextEdit::singleline(&mut self.folder_search_query)
                .hint_text("z.B. dwnlds, projekt-x …  (Ctrl+F)")
                .desired_width(f32::INFINITY),
        );
        if self.folder_search_focus {
            search_resp.request_focus();
            self.folder_search_focus = false;
        }
        if search_resp.changed() {
            if self.folder_search_query.is_empty() {
                self.folder_search_results.clear();
                self.folder_search_pending_at = None;
            } else {
                self.folder_search_pending_at = Some(std::time::Instant::now());
            }
        }
        let mut clicked_path: Option<String> = None;
        if !self.folder_index.is_empty() && !self.folder_search_query.is_empty() {
            egui::ScrollArea::vertical()
                .id_salt("folder_search_results")
                .max_height(220.0)
                .show(ui, |ui| {
                    if self.folder_search_results.is_empty() {
                        ui.colored_label(Color32::from_gray(140), "keine Treffer");
                    }
                    for (p, _score) in &self.folder_search_results {
                        let base = p.rsplit('/').next().unwrap_or(p);
                        let parent = p.rsplit_once('/').map(|(par, _)| par).unwrap_or("");
                        let label = format!("{}\n{}", base, parent);
                        let resp = ui
                            .add(egui::Button::new(label).wrap().min_size(egui::vec2(0.0, 28.0)))
                            .on_hover_text(p.clone());
                        if resp.clicked() {
                            clicked_path = Some(p.clone());
                        }
                    }
                });
        }
        if let Some(p) = clicked_path {
            self.folder_search_query.clear();
            self.folder_search_results.clear();
            self.start_scan(PathBuf::from(p.replace('/', std::path::MAIN_SEPARATOR_STR)));
        }

        ui.horizontal(|ui| {
            if self.index_building {
                ui.colored_label(
                    Color32::from_gray(140),
                    format!("⟳ Indizieren… {} Ordner", self.index_progress),
                );
                if ui.small_button("Stop").clicked() {
                    self.cancel_index_build();
                }
            } else if self.folder_index.is_empty() {
                ui.colored_label(Color32::from_gray(140), "Kein Index");
                if ui
                    .small_button("Bauen")
                    .on_hover_text("Scannt alle Laufwerke einmalig nach Ordnern (etwa 30-90s)")
                    .clicked()
                {
                    self.start_index_build();
                }
            } else {
                let count = self.folder_index.len();
                ui.colored_label(
                    Color32::from_gray(140),
                    format!(
                        "Index: {} Ordner",
                        count.to_string().chars().rev().enumerate().fold(
                            String::new(),
                            |acc, (i, c)| {
                                if i > 0 && i % 3 == 0 {
                                    format!("{}.{}", c, acc)
                                } else {
                                    format!("{}{}", c, acc)
                                }
                            }
                        )
                    ),
                );
                if ui.small_button("⟳").on_hover_text("Index aktualisieren").clicked() {
                    self.start_index_build();
                }
            }
        });

        ui.add_space(8.0);

        // ─── Favorites (starred folders) ───────────────────────────────
        if !self.favorites.is_empty() {
            ui.label(RichText::new("★ FAVORITEN").small().color(Color32::from_gray(140)));
            let favs = self.favorites.clone();
            let mut nav: Option<String> = None;
            let mut unstar: Option<String> = None;
            for f in &favs {
                ui.horizontal(|ui| {
                    let label = {
                        let base = f.trim_end_matches('/').rsplit('/').next().unwrap_or(f);
                        if base.is_empty() { f.as_str() } else { base }
                    };
                    if ui
                        .selectable_label(self.root_prefix() == *f, label)
                        .on_hover_text(f)
                        .clicked()
                    {
                        nav = Some(f.clone());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").on_hover_text("Aus Favoriten entfernen").clicked() {
                            unstar = Some(f.clone());
                        }
                    });
                });
            }
            if let Some(p) = nav {
                self.start_scan(PathBuf::from(p.replace('/', std::path::MAIN_SEPARATOR_STR)));
            }
            if let Some(p) = unstar {
                self.toggle_favorite(&p);
            }
            ui.add_space(8.0);
        }

        ui.label(RichText::new("SCHNELLZUGRIFF").small().color(Color32::from_gray(140)));
        let home = self.home.clone();
        for (label, sub) in [
            ("Home", ""),
            ("Desktop", "Desktop"),
            ("Documents", "Documents"),
            ("Downloads", "Downloads"),
            ("Pictures", "Pictures"),
            ("Music", "Music"),
            ("Videos", "Videos"),
        ] {
            let p = if sub.is_empty() { home.clone() } else { home.join(sub) };
            if ui
                .selectable_label(self.root_path == p.to_string_lossy().replace('\\', "/"), label)
                .on_hover_text(p.to_string_lossy())
                .clicked()
            {
                self.start_scan(p);
            }
        }

        if !self.drive_info.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new("LAUFWERKE").small().color(Color32::from_gray(140)));
            let infos = self.drive_info.clone();
            for (d, free, total) in infos {
                if ui
                    .selectable_label(self.root_path == d.replace('\\', "/"), &d)
                    .clicked()
                {
                    self.start_scan(PathBuf::from(&d));
                }
                if total > 0 {
                    let used = total.saturating_sub(free);
                    let frac = used as f32 / total as f32;
                    ui.add(
                        egui::ProgressBar::new(frac)
                            .desired_width(150.0)
                            .desired_height(6.0),
                    )
                    .on_hover_text(format!(
                        "{} frei von {}",
                        format_bytes(free),
                        format_bytes(total)
                    ));
                }
            }
        }

        if !self.recent.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new("ZULETZT").small().color(Color32::from_gray(140)));
            let recent = self.recent.clone();
            for r in recent {
                let label = r.rsplit('/').next().unwrap_or(&r).to_string();
                let label = if label.is_empty() { r.clone() } else { label };
                if ui
                    .selectable_label(self.root_path == r, &label)
                    .on_hover_text(&r)
                    .clicked()
                {
                    self.start_scan(PathBuf::from(r.replace('/', std::path::MAIN_SEPARATOR_STR)));
                }
            }
        }

        // ─── Remote connections (set-up-once; freshest pinned here) ─────
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("VERBINDUNGEN").small().color(Color32::from_gray(140)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("＋")
                    .on_hover_text("Neue Verbindung (SFTP / FTP / FTPS / Netzlaufwerk)")
                    .clicked()
                {
                    self.connect_form = crate::connect::ConnectForm::default();
                    self.show_connect = true;
                }
            });
        });

        let mut disconnect = false;
        let mut to_connect: Option<crate::creds::SavedConnection> = None;
        let mut to_remove: Option<String> = None;
        let mut open_gdrive = false;
        let mut disc_gdrive = false;

        // Active connection indicator + one-click disconnect.
        if let Some(rs) = &self.remote {
            ui.horizontal(|ui| {
                ui.colored_label(Color32::from_rgb(120, 200, 255), format!("● {}", rs.label));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("⏏").on_hover_text("Verbindung trennen").clicked() {
                        disconnect = true;
                    }
                });
            });
        } else if self.net_conn.is_some() {
            ui.horizontal(|ui| {
                ui.colored_label(Color32::from_rgb(120, 200, 255), "● Netzlaufwerk");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("⏏").on_hover_text("Verbindung trennen").clicked() {
                        disconnect = true;
                    }
                });
            });
        }

        // Pinned Google Drive — stays here whenever Drive is connected, even
        // with no tab open on it (click to browse, × to disconnect).
        let gdrive_active = self
            .remote
            .as_ref()
            .map(|rs| rs.backend.scheme() == crate::vfs::Scheme::GDrive)
            .unwrap_or(false);
        if crate::cloud::is_connected(crate::cloud::Provider::GDrive) {
            ui.horizontal(|ui| {
                let txt = RichText::new("☁ Google Drive").small();
                let txt = if gdrive_active {
                    txt.color(Color32::from_rgb(120, 200, 255))
                } else {
                    txt
                };
                if ui
                    .add(egui::Button::new(txt).frame(false))
                    .on_hover_text("Google Drive durchsuchen")
                    .clicked()
                {
                    open_gdrive = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("×").on_hover_text("Google Drive trennen").clicked() {
                        disc_gdrive = true;
                    }
                });
            });
        }

        // Saved connections, newest first, capped — click to connect, × forget.
        let conns: Vec<crate::creds::SavedConnection> =
            self.saved_connections.iter().rev().cloned().collect();
        if conns.is_empty() {
            ui.colored_label(Color32::from_gray(120), "(noch keine gespeichert)");
        }
        for c in conns.iter().take(SIDEBAR_CONN_CAP) {
            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::Button::new(RichText::new(format!("🖧 {}", c.display())).small())
                            .frame(false),
                    )
                    .on_hover_text(c.to_target())
                    .clicked()
                {
                    to_connect = Some(c.clone());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("×").on_hover_text("Entfernen").clicked() {
                        to_remove = Some(c.account());
                    }
                });
            });
        }
        if conns.len() > SIDEBAR_CONN_CAP {
            ui.colored_label(
                Color32::from_gray(120),
                format!(
                    "+{} ältere im Menü „Verbindung“",
                    conns.len() - SIDEBAR_CONN_CAP
                ),
            );
        }

        if disconnect {
            self.remote = None;
            self.net_conn = None;
            self.notice = Some(("Verbindung getrennt".to_string(), std::time::Instant::now()));
        }
        if let Some(acc) = to_remove {
            let _ = crate::creds::remove_connection(&acc);
            self.saved_connections = crate::creds::load_connections();
        }
        if let Some(c) = to_connect {
            self.connect_saved(&c);
        }
        if open_gdrive {
            self.open_gdrive_browse();
        }
        if disc_gdrive {
            crate::cloud::disconnect(crate::cloud::Provider::GDrive);
            if gdrive_active {
                self.remote = None;
            }
            self.notice = Some(("Google Drive getrennt".to_string(), std::time::Instant::now()));
        }
    }

    fn ui_menu_connect(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("VERBINDEN").small().color(Color32::from_gray(140)));
            if self.remote.is_some() || self.net_conn.is_some() {
                if ui.small_button("⏏").on_hover_text("Verbindung trennen").clicked() {
                    self.remote = None;
                    self.net_conn = None;
                    self.notice = Some(("Verbindung getrennt".to_string(), std::time::Instant::now()));
                }
            }
        });
        if let Some(rs) = &self.remote {
            ui.colored_label(Color32::from_rgb(120, 200, 255), format!("● {}", rs.label));
        }
        if ui
            .small_button("＋ Neue Verbindung")
            .on_hover_text("SFTP / FTP / FTPS / Netzlaufwerk")
            .clicked()
        {
            self.connect_form = crate::connect::ConnectForm::default();
            self.show_connect = true;
        }
        // Established connections live on the sidebar (most recent first). Only
        // the overflow — older ones beyond the sidebar cap — appears here, so
        // the menu stays uncluttered but no saved connection is ever hidden.
        let mut to_remove: Option<String> = None;
        let mut to_connect: Option<crate::creds::SavedConnection> = None;
        let conns: Vec<crate::creds::SavedConnection> =
            self.saved_connections.iter().rev().cloned().collect();
        if conns.len() > SIDEBAR_CONN_CAP {
            ui.add_space(4.0);
            ui.label(
                RichText::new("WEITERE (ältere)")
                    .small()
                    .color(Color32::from_gray(140)),
            );
            for c in conns.iter().skip(SIDEBAR_CONN_CAP) {
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new(format!("🖧 {}", c.display())).small())
                                .frame(false),
                        )
                        .on_hover_text(c.to_target())
                        .clicked()
                    {
                        to_connect = Some(c.clone());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("×").on_hover_text("Entfernen").clicked() {
                            to_remove = Some(c.account());
                        }
                    });
                });
            }
        } else if !conns.is_empty() {
            ui.colored_label(
                Color32::from_gray(120),
                "Gespeicherte Verbindungen: in der Sidebar links.",
            );
        }
        if let Some(acc) = to_remove {
            let _ = crate::creds::remove_connection(&acc);
            self.saved_connections = crate::creds::load_connections();
        }
        if let Some(c) = to_connect {
            self.connect_saved(&c);
        }
    }

    fn ui_menu_sync(&mut self, ui: &mut egui::Ui) {
        // One-way mirror of the current location to a local folder (backup).
        if !self.root_path.is_empty() {
            if self.sync_running {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Spiegelung läuft…");
                    if ui.button("⏹ Stop").clicked() {
                        if let Some(c) = &self.sync_cancel {
                            c.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                });
            } else if self.bisync_running {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("2-Wege-Sync läuft…");
                    if ui.button("⏹ Stop").clicked() {
                        if let Some(c) = &self.bisync_cancel {
                            c.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                });
            } else {
                if ui
                    .small_button("⇅ Spiegeln nach…")
                    .on_hover_text("Aktuellen Ordner (lokal oder remote) EINSEITIG in einen lokalen Zielordner spiegeln (Backup)")
                    .clicked()
                {
                    if let Some(dest) = rfd::FileDialog::new().pick_folder() {
                        self.start_mirror(dest.to_string_lossy().replace('\\', "/"));
                    }
                }
                if ui
                    .small_button("⇄ 2-Wege-Sync…")
                    .on_hover_text("Sicher in BEIDE Richtungen abgleichen: nur tatsächlich geänderte Dateien werden übertragen, beidseitige Änderungen werden als Konflikt gemeldet (nichts wird stillschweigend überschrieben), Änderungen sind reversibel.")
                    .clicked()
                {
                    if let Some(dest) = rfd::FileDialog::new().pick_folder() {
                        self.start_bisync(dest.to_string_lossy().replace('\\', "/"));
                    }
                }
            }
        }
        // ─── Saved sync setups (persist across restarts) ──────────────────
        ui.separator();
        ui.horizontal(|ui| {
            if ui
                .small_button("⚙ Sync-Setups…")
                .on_hover_text("Gespeicherte Sync-Aufträge verwalten (Quelle, Ziel, Methode, Zeitplan) — bleiben nach Neustart erhalten")
                .clicked()
            {
                self.show_sync_jobs = true;
            }
            let n = self.sync_jobs.len();
            if n > 0 {
                ui.colored_label(Color32::from_gray(140), format!("({n})"));
            }
        });
        // Quick-create from the current location.
        if !self.root_path.is_empty()
            && ui
                .small_button("＋ Setup aus aktuellem Ordner…")
                .on_hover_text("Neues Sync-Setup mit dem aktuellen Ordner als Quelle anlegen")
                .clicked()
        {
            let src = if is_local_style(&self.root_path) {
                self.root_path.clone()
            } else {
                String::new()
            };
            self.job_editor = Some(JobEditor::blank(src, String::new()));
            self.show_sync_jobs = true;
        }

        // ─── Background sync (runs setups on their schedule, app closed) ──
        ui.separator();
        ui.label(RichText::new("HINTERGRUND").small().color(Color32::from_gray(140)));
        let mut bg = crate::autostart::is_enabled();
        if ui
            .checkbox(&mut bg, "Beim Anmelden im Hintergrund synchronisieren")
            .on_hover_text(
                "Startet einen unsichtbaren Dienst (dieselbe App via Autostart), der \
                 gespeicherte Setups mit Zeitplan automatisch ausführt — auch wenn das \
                 Fenster geschlossen ist. Updates erfassen den Dienst automatisch.",
            )
            .changed()
        {
            if bg {
                match crate::autostart::enable() {
                    Ok(_) => {
                        crate::daemon::clear_stop();
                        crate::autostart::spawn_daemon_now();
                        self.notice = Some((
                            "✓ Hintergrund-Sync aktiviert".to_string(),
                            std::time::Instant::now(),
                        ));
                    }
                    Err(e) => self.error_msg = Some(format!("Autostart: {}", e)),
                }
            } else {
                let _ = crate::autostart::disable();
                crate::daemon::request_stop();
                self.notice = Some((
                    "Hintergrund-Sync deaktiviert".to_string(),
                    std::time::Instant::now(),
                ));
            }
        }
        if crate::daemon::is_running() {
            let age = crate::daemon::last_heartbeat_age().unwrap_or(0);
            ui.colored_label(
                Color32::from_rgb(120, 200, 255),
                format!("● Dienst aktiv (vor {age}s)"),
            );
        } else if bg {
            ui.colored_label(
                Color32::from_gray(150),
                "Dienst startet beim nächsten Anmelden.",
            );
        }
        ui.label(
            RichText::new("Hintergrund-Setups: lokale Pfade, Zeitplan > 0 min.")
                .small()
                .color(Color32::from_gray(120)),
        );
    }

    /// Saved-setups manager: list jobs with run / edit / delete / enable, plus
    /// "new". This is the rich overview the user asked for (source → target,
    /// method, schedule). Persists to sync/jobs.tsv on every change.
    fn ui_sync_jobs(&mut self, ctx: &egui::Context) {
        let mut open = self.show_sync_jobs;
        let mut run_id: Option<String> = None;
        let mut edit_id: Option<String> = None;
        let mut del_id: Option<String> = None;
        let mut toggle_id: Option<String> = None;
        let mut new_blank = false;
        let jobs = self.sync_jobs.clone();
        egui::Window::new("⚙ Sync-Setups")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([640.0, 440.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("＋ Neues Setup").clicked() {
                        new_blank = true;
                    }
                    ui.label(
                        RichText::new("Quelle ⇄ Ziel, Methode, Zeitplan — bleibt nach Neustart erhalten.")
                            .small()
                            .color(Color32::from_gray(140)),
                    );
                });
                ui.separator();
                if jobs.is_empty() {
                    ui.add_space(8.0);
                    ui.colored_label(
                        Color32::from_gray(140),
                        "Noch keine Setups. „＋ Neues Setup“ anlegen oder im Split-View zwei Ordner per Rechtsklick verbinden.",
                    );
                    return;
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for j in &jobs {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(if j.name.is_empty() { "(ohne Name)" } else { &j.name }).strong());
                                if !j.enabled {
                                    ui.colored_label(Color32::from_gray(130), "⏸ deaktiviert");
                                }
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button("✕").on_hover_text("Setup löschen").clicked() {
                                        del_id = Some(j.id.clone());
                                    }
                                    if ui.small_button("✎ Bearbeiten").clicked() {
                                        edit_id = Some(j.id.clone());
                                    }
                                    let enable_label = if j.enabled { "⏸ Aus" } else { "▶ Ein" };
                                    if ui.small_button(enable_label).on_hover_text("Zeitplan aktivieren/deaktivieren").clicked() {
                                        toggle_id = Some(j.id.clone());
                                    }
                                    if !self.bisync_running
                                        && ui.button("▶ Jetzt").on_hover_text("Diesen Sync jetzt ausführen").clicked()
                                    {
                                        run_id = Some(j.id.clone());
                                    }
                                });
                            });
                            ui.label(
                                RichText::new(format!("{}  →  {}", j.source, j.target))
                                    .small()
                                    .color(Color32::from_gray(170)),
                            );
                            let sched = if j.interval_min == 0 {
                                "manuell".to_string()
                            } else {
                                format!("alle {} min", j.interval_min)
                            };
                            let last = if j.last_run == 0 {
                                "nie".to_string()
                            } else {
                                fmt_ms(j.last_run * 1000)
                            };
                            ui.label(
                                RichText::new(format!(
                                    "{} · {} · {} Tage Verlauf · {} · zuletzt: {}",
                                    j.direction.label(),
                                    j.conflict.label(),
                                    j.retain_days,
                                    sched,
                                    last
                                ))
                                .small()
                                .color(Color32::from_gray(140)),
                            );
                        });
                    }
                });
            });
        self.show_sync_jobs = open;
        if new_blank {
            self.job_editor = Some(JobEditor::blank(String::new(), String::new()));
        }
        if let Some(id) = edit_id {
            if let Some(j) = self.sync_jobs.iter().find(|j| j.id == id) {
                self.job_editor = Some(JobEditor::from_job(j));
            }
        }
        if let Some(id) = toggle_id {
            if let Some(j) = self.sync_jobs.iter_mut().find(|j| j.id == id) {
                j.enabled = !j.enabled;
                let job = j.clone();
                let _ = crate::syncjobs::upsert(&job);
                self.sync_jobs = crate::syncjobs::load();
            }
        }
        if let Some(id) = del_id {
            let _ = crate::syncjobs::remove(&id);
            self.sync_jobs = crate::syncjobs::load();
        }
        if let Some(id) = run_id {
            self.run_job(&id);
        }
    }

    /// Add/edit dialog for a single sync setup (the "rich" setup menu: source,
    /// target, method = direction + conflict handling, retention, schedule,
    /// hidden-file handling, ignore globs).
    fn ui_job_editor(&mut self, ctx: &egui::Context) {
        let mut ed = match self.job_editor.take() {
            Some(e) => e,
            None => return,
        };
        let mut open = true;
        let mut save = false;
        let mut cancel = false;
        // Set when a "Durchsuchen" button is clicked → open the in-app picker
        // after `ed` is restored to self.job_editor (so the picker can write
        // back into it). Carries the field + its current value as a start point.
        let mut pick: Option<(PickerField, String)> = None;
        let title = if ed.id.is_some() { "✎ Sync-Setup bearbeiten" } else { "＋ Neues Sync-Setup" };
        egui::Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([560.0, 0.0])
            .show(ctx, |ui| {
                egui::Grid::new("job_editor_grid")
                    .num_columns(2)
                    .spacing([10.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Name");
                        ui.add(egui::TextEdit::singleline(&mut ed.name).hint_text("z. B. Dokumente sichern").desired_width(360.0));
                        ui.end_row();

                        ui.label("Quelle (A)");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut ed.source).hint_text("lokaler Ordner / Netzlaufwerk / Verbindung").desired_width(280.0));
                            if ui
                                .button("📂")
                                .on_hover_text("Im Explorer wählen — lokale Laufwerke oder gespeicherte Verbindungen")
                                .clicked()
                            {
                                pick = Some((PickerField::Source, ed.source.clone()));
                            }
                        });
                        ui.end_row();

                        ui.label("Ziel (B)");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut ed.target).hint_text("lokaler Ordner / Netzlaufwerk / Verbindung").desired_width(280.0));
                            if ui
                                .button("📂")
                                .on_hover_text("Im Explorer wählen — lokale Laufwerke oder gespeicherte Verbindungen")
                                .clicked()
                            {
                                pick = Some((PickerField::Target, ed.target.clone()));
                            }
                        });
                        ui.end_row();

                        ui.label("Richtung").on_hover_text("Methode: in welche Richtung abgeglichen wird");
                        egui::ComboBox::from_id_salt("job_dir")
                            .selected_text(ed.direction.label())
                            .show_ui(ui, |ui| {
                                for d in [
                                    crate::bisync::Direction::Both,
                                    crate::bisync::Direction::AtoB,
                                    crate::bisync::Direction::BtoA,
                                ] {
                                    ui.selectable_value(&mut ed.direction, d, d.label());
                                }
                            });
                        ui.end_row();

                        ui.label("Konflikte").on_hover_text("Was passiert, wenn beide Seiten geändert wurden");
                        egui::ComboBox::from_id_salt("job_conf")
                            .selected_text(ed.conflict.label())
                            .show_ui(ui, |ui| {
                                for c in [
                                    crate::bisync::ConflictMode::FileLevel,
                                    crate::bisync::ConflictMode::NewerWins,
                                ] {
                                    ui.selectable_value(&mut ed.conflict, c, c.label());
                                }
                            });
                        ui.end_row();

                        ui.label("Verlauf (Tage)").on_hover_text("Wie lange reversible Sicherungen überschriebener Dateien aufbewahrt werden");
                        ui.add(egui::TextEdit::singleline(&mut ed.retain_days).desired_width(80.0));
                        ui.end_row();

                        ui.label("Zeitplan (min)").on_hover_text("Automatisch alle N Minuten ausführen (0 = nur manuell)");
                        ui.add(egui::TextEdit::singleline(&mut ed.interval_min).desired_width(80.0));
                        ui.end_row();

                        ui.label("Versteckte Dateien");
                        ui.checkbox(&mut ed.include_hidden, "einbeziehen");
                        ui.end_row();

                        ui.label("Ignorieren").on_hover_text("Glob-Muster, eines pro Zeile (z. B. **/*.tmp, node_modules/**)");
                        ui.add(egui::TextEdit::multiline(&mut ed.ignore).hint_text("**/*.tmp\nnode_modules/**").desired_rows(3).desired_width(360.0));
                        ui.end_row();

                        ui.label("Aktiv");
                        ui.checkbox(&mut ed.enabled, "Zeitplan aktiv");
                        ui.end_row();
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("✔ Speichern").clicked() {
                        save = true;
                    }
                    if ui.button("Abbrechen").clicked() {
                        cancel = true;
                    }
                });
            });
        if cancel || !open {
            // Dropped (taken at top) — leaving job_editor as None closes it.
            return;
        }
        if save {
            if ed.source.trim().is_empty() || ed.target.trim().is_empty() {
                self.error_msg = Some("Quelle und Ziel dürfen nicht leer sein.".to_string());
                self.job_editor = Some(ed); // keep the dialog open
                return;
            }
            let name = if ed.name.trim().is_empty() {
                let base = ed.source.trim_end_matches('/').rsplit('/').next().unwrap_or("Sync");
                base.to_string()
            } else {
                ed.name.trim().to_string()
            };
            let ignore: Vec<String> = ed
                .ignore
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
            let mut job = match &ed.id {
                // Editing: preserve id + last_run from the stored job.
                Some(id) => {
                    let mut j = self
                        .sync_jobs
                        .iter()
                        .find(|j| &j.id == id)
                        .cloned()
                        .unwrap_or_else(|| crate::syncjobs::SyncJob::new(name.clone(), ed.source.clone(), ed.target.clone()));
                    j.name = name.clone();
                    j.source = ed.source.trim().to_string();
                    j.target = ed.target.trim().to_string();
                    j
                }
                None => crate::syncjobs::SyncJob::new(name.clone(), ed.source.trim().to_string(), ed.target.trim().to_string()),
            };
            job.direction = ed.direction;
            job.conflict = ed.conflict;
            job.retain_days = ed.retain_days.trim().parse().unwrap_or(30);
            job.interval_min = ed.interval_min.trim().parse().unwrap_or(0);
            job.include_hidden = ed.include_hidden;
            job.ignore = ignore;
            job.enabled = ed.enabled;
            match crate::syncjobs::upsert(&job) {
                Ok(_) => {
                    self.sync_jobs = crate::syncjobs::load();
                    self.notice = Some((format!("✓ Setup „{}“ gespeichert", job.name), std::time::Instant::now()));
                }
                Err(e) => {
                    self.error_msg = Some(format!("Setup speichern: {}", e));
                    self.job_editor = Some(ed);
                }
            }
            return;
        }
        // Still open, nothing pressed — keep the editor for the next frame.
        self.job_editor = Some(ed);
        // Now that job_editor is restored, the picker can write back into it.
        if let Some((field, initial)) = pick {
            self.open_picker(field, &initial);
        }
    }

    /// Cloud (Google Drive) connect (#19): configure your OWN Google OAuth
    /// client ID and run the authorize flow. Smart Explorer is not a hosted
    /// service — each user supplies their own client (see docs/CLOUD_SETUP.md).
    fn ui_menu_cloud(&mut self, ui: &mut egui::Ui) {
        use crate::cloud::Provider;
        let p = Provider::GDrive;
        ui.add_space(12.0);
        ui.label(RichText::new("CLOUD (GOOGLE DRIVE)").small().color(Color32::from_gray(140)));
        if crate::cloud::is_connected(p) {
            ui.horizontal(|ui| {
                ui.colored_label(Color32::from_rgb(120, 200, 255), "● Verbunden");
                if ui.small_button("☁ Drive öffnen").on_hover_text("Google Drive durchsuchen").clicked() {
                    self.open_gdrive_browse();
                }
                if ui.small_button("Trennen").clicked() {
                    crate::cloud::disconnect(p);
                    self.notice = Some(("Google Drive getrennt".to_string(), std::time::Instant::now()));
                }
            });
        }
        ui.add(
            egui::TextEdit::singleline(&mut self.cloud_client_id_draft)
                .hint_text("OAuth Client-ID (…apps.googleusercontent.com)")
                .desired_width(f32::INFINITY),
        )
        .on_hover_text(
            "Aus DEINEM eigenen Google-Cloud-Projekt (Desktop-OAuth-Client). \
             Diese App ist kein Dienst — siehe Anleitung unten / docs/CLOUD_SETUP.md.",
        );
        ui.add(
            egui::TextEdit::singleline(&mut self.cloud_secret_draft)
                .hint_text("Client-Secret (von Google, falls vergeben)")
                .password(true)
                .desired_width(f32::INFINITY),
        );
        ui.horizontal(|ui| {
            if self.cloud_authing {
                ui.spinner();
                ui.label("Browser-Anmeldung läuft…");
            } else if ui
                .small_button("Mit Google verbinden")
                .on_hover_text("Speichert die Client-ID und öffnet den Browser zur Anmeldung")
                .clicked()
            {
                let cfg = crate::cloud::ClientConfig {
                    client_id: self.cloud_client_id_draft.trim().to_string(),
                    client_secret: self.cloud_secret_draft.trim().to_string(),
                };
                if cfg.client_id.is_empty() {
                    self.error_msg = Some("Bitte zuerst die Client-ID eintragen.".to_string());
                } else {
                    let _ = crate::cloud::save_config(p, &cfg);
                    let (tx, rx) = unbounded();
                    self.cloud_auth_rx = Some(rx);
                    self.cloud_authing = true;
                    std::thread::Builder::new()
                        .name("cloud-auth".into())
                        .spawn(move || {
                            let _ = tx.send(crate::cloud::authorize(p).map(|_| ()));
                        })
                        .ok();
                    self.notice = Some((
                        "Browser zur Google-Anmeldung geöffnet…".to_string(),
                        std::time::Instant::now(),
                    ));
                }
            }
        });
        // Inline setup guide — the user runs their own Google project; this app
        // hosts nothing. Full version: docs/CLOUD_SETUP.md.
        egui::CollapsingHeader::new("ℹ Einrichtung (eigenes Google-Projekt)")
            .id_salt("cloud_setup_help")
            .show(ui, |ui| {
                ui.label(
                    RichText::new(
                        "Smart Explorer ist kein Cloud-Dienst — du nutzt dein eigenes \
                         Google-Konto. Einmalig (~5 min):",
                    )
                    .small(),
                );
                for line in [
                    "1. Google Cloud Console → Projekt anlegen.",
                    "2. APIs & Dienste → Bibliothek → „Google Drive API“ aktivieren.",
                    "3. OAuth-Zustimmungsbildschirm → Extern; dich als Testnutzer hinzufügen.",
                    "4. Anmeldedaten → OAuth-Client-ID → Typ „Desktop-App“ (keine Redirect-URI nötig).",
                    "5. Client-ID (+ ggf. Secret) oben einfügen → „Mit Google verbinden“.",
                ] {
                    ui.label(RichText::new(line).small().color(Color32::from_gray(180)));
                }
                ui.hyperlink_to("→ Google Cloud Console öffnen", "https://console.cloud.google.com");
                ui.label(
                    RichText::new(
                        "Hinweis: Im „Testing“-Modus laufen die Tokens nach ~7 Tagen ab — \
                         dann einfach erneut verbinden. Details: docs/CLOUD_SETUP.md.",
                    )
                    .small()
                    .color(Color32::from_gray(140)),
                );
            });
        ui.separator();
    }

    fn ui_menu_settings(&mut self, ui: &mut egui::Ui) {
        self.ui_menu_cloud(ui);

        // ─── Remote-Dateien öffnen (temp vs CfAPI) ────────────────────
        ui.add_space(12.0);
        ui.label(RichText::new("REMOTE-DATEIEN ÖFFNEN").small().color(Color32::from_gray(140)));
        let mut mode = self.remote_open_mode;
        let changed = ui
            .radio_value(&mut mode, RemoteOpenMode::Temp, "Temp-Kopie (überall)")
            .on_hover_text(
                "Lädt die Datei in eine temporäre Kopie, öffnet sie in der zugehörigen \
                 App und lädt Änderungen beim Speichern automatisch auf das Remote zurück. \
                 Funktioniert mit jeder App und jedem Backend.",
            )
            .changed()
            | ui
                .radio_value(&mut mode, RemoteOpenMode::CfApi, "Persistenter Sync-Ordner")
                .on_hover_text(
                    "Spiegelt Remote-Dateien an einem festen lokalen Pfad \
                     (%USERPROFILE%\\Smart Explorer\\<Verbindung>\\…), der die Remote-Struktur \
                     abbildet; Änderungen werden beim Speichern automatisch zurückgeladen. \
                     (Native OneDrive-Platzhalter mit Hydrierung-auf-Abruf folgen, #30.)",
                )
                .changed();
        if changed {
            self.remote_open_mode = mode;
            save_remote_open_mode(mode);
            self.notice = Some(("✓ Remote-Öffnen-Modus gespeichert".to_string(), std::time::Instant::now()));
        }

        // ─── Teilen (peer file sharing) ───────────────────────────────
        ui.add_space(12.0);
        ui.label(RichText::new("TEILEN (P2P)").small().color(Color32::from_gray(140)));
        ui.add(
            egui::TextEdit::singleline(&mut self.share_server_draft)
                .hint_text("Rendezvous-Server  host:port")
                .desired_width(f32::INFINITY),
        )
        .on_hover_text(
            "Adresse deines eigenen Routing-Servers (se-share-server). Er vermittelt \
             nur die Verbindung — die Dateien gehen direkt zwischen den Geräten, \
             Ende-zu-Ende-verschlüsselt.",
        );
        ui.add(
            egui::TextEdit::singleline(&mut self.share_device_draft)
                .hint_text("Gerätename")
                .desired_width(f32::INFINITY),
        );
        if ui.small_button("Speichern").clicked() {
            self.share_server = self.share_server_draft.trim().to_string();
            let _ = std::fs::write(share_server_path(), &self.share_server);
            // Restart the service so the new server/name take effect.
            self.share = None;
            self.notice = Some(("✓ Teilen-Einstellungen gespeichert".to_string(), std::time::Instant::now()));
        }

        // ─── Update ───────────────────────────────────────────────────
        ui.add_space(12.0);
        ui.label(RichText::new("UPDATE").small().color(Color32::from_gray(140)));
        ui.colored_label(
            Color32::from_gray(140),
            format!("Version {}", env!("CARGO_PKG_VERSION")),
        );
        ui.add(
            egui::TextEdit::singleline(&mut self.update_feed_draft)
                .hint_text("Feed-Ordner oder Git/HTTPS-URL…")
                .desired_width(f32::INFINITY),
        )
        .on_hover_text(
            "Quelle mit version.txt und smart_explorer.exe. Entweder ein Ordner \
             (lokal/Netzlaufwerk) ODER eine https-URL bzw. ein GitHub-Repo-Link \
             (z. B. https://github.com/b1ue-man/smart-explorer) — dann updatet \
             sich die App direkt aus dem Git. Beim Start wird automatisch geprüft.",
        );
        ui.horizontal(|ui| {
            if ui.small_button("Speichern").clicked() {
                match crate::updater::set_update_source(&self.update_feed_draft) {
                    Ok(_) => {
                        self.notice = Some((
                            "✓ Update-Feed gespeichert".to_string(),
                            std::time::Instant::now(),
                        ));
                    }
                    Err(e) => self.error_msg = Some(format!("Feed speichern: {}", e)),
                }
            }
            if ui.small_button("Jetzt prüfen").clicked() {
                self.check_updates_manual();
            }
        });

        // Rollback to a previously-installed version + pause/resume auto-update.
        if let Some(pinned) = crate::updater::pinned_version() {
            ui.colored_label(
                Color32::from_rgb(255, 190, 90),
                format!("⏸ Auto-Update pausiert (zurückgerollt auf v{})", pinned),
            );
            if ui.small_button("Auf neueste aktualisieren").clicked() {
                let (tx, rx) = unbounded();
                self.update_rx = Some(rx);
                crate::updater::update_to_latest_async(tx);
                self.notice =
                    Some(("Suche neueste Version…".to_string(), std::time::Instant::now()));
            }
        }
        // Rollback section — always shown so the feature is discoverable. The
        // currently-running version is filtered out (you can't roll back to it).
        ui.add_space(2.0);
        ui.label(
            RichText::new("Frühere Versionen")
                .small()
                .color(Color32::from_gray(140)),
        );
        let current = env!("CARGO_PKG_VERSION");
        let archived: Vec<(String, PathBuf)> = crate::updater::list_archived_versions()
            .into_iter()
            .filter(|(v, _)| v != current)
            .collect();
        if archived.is_empty() {
            ui.colored_label(
                Color32::from_gray(110),
                "(keine — werden nach jedem Update gesichert)",
            );
        } else {
            let mut revert: Option<(String, PathBuf)> = None;
            for (ver, path) in &archived {
                ui.horizontal(|ui| {
                    ui.label(format!("v{}", ver));
                    if ui
                        .small_button("↩ Zurück")
                        .on_hover_text("Auf diese Version zurückrollen (Neustart)")
                        .clicked()
                    {
                        revert = Some((ver.clone(), path.clone()));
                    }
                });
            }
            if let Some((ver, path)) = revert {
                match crate::updater::revert_to(&path, &ver) {
                    Ok(exe) => {
                        // Reuse the restart-prompt flow.
                        self.update_ready = Some((ver, exe));
                    }
                    Err(e) => self.error_msg = Some(format!("Zurückrollen: {}", e)),
                }
            }
        }

        // ─── Shell integration (Windows) ───────────────────────────────
        #[cfg(windows)]
        {
            ui.add_space(12.0);
            ui.label(RichText::new("INTEGRATION").small().color(Color32::from_gray(140)));

            let resp = ui
                .checkbox(
                    &mut self.integration_ctx_menu,
                    "„In Smart Explorer öffnen“ im Rechtsklick",
                )
                .on_hover_text(
                    "Fügt einen Rechtsklick-Eintrag bei Ordnern, Laufwerken und im leeren Bereich hinzu. Jederzeit hier abschaltbar.",
                );
            if resp.changed() {
                let on = self.integration_ctx_menu;
                match crate::shell_register::set_context_menu(on) {
                    Ok(()) => {
                        self.notice = Some((
                            if on {
                                "✓ Rechtsklick-Eintrag hinzugefügt".to_string()
                            } else {
                                "✓ Rechtsklick-Eintrag entfernt".to_string()
                            },
                            std::time::Instant::now(),
                        ));
                    }
                    Err(e) => {
                        self.integration_ctx_menu = !on; // revert UI to real state
                        self.error_msg = Some(format!("Registry: {}", e));
                    }
                }
            }

            ui.colored_label(
                Color32::from_gray(110),
                "Hinweis: Der Eintrag liegt unter „Weitere Optionen anzeigen“ (Win11).",
            );
        }
    }


    fn ui_table(&mut self, ui: &mut egui::Ui) {
        use egui_extras::{Column, TableBuilder};

        let prefix = self.root_prefix();
        let total_rows = self.view.len();
        let row_h = 22.0;

        let mut row_click: Option<(usize, bool, bool)> = None; // (idx, ctrl, shift)
        let mut row_dblclick: Option<usize> = None;
        let mut row_rclick: Option<usize> = None;
        let mut sort_clicked: Option<SortKey> = None;
        // Entry index of a row whose drag just started this frame (file drag to
        // another tab/pane or out to Explorer). Resolved after the table.
        let mut drag_start: Option<usize> = None;
        // (row index, name-cell rect) of rendered rows — used for rubber-band
        // geometry below.
        let mut visible_rows: Vec<(usize, egui::Rect)> = Vec::new();
        // Icon keys seen this frame that aren't cached yet (requested after the
        // table, since we can't mutably borrow self.icon_cache inside the body).
        let mut needed_icons: Vec<String> = Vec::new();

        let header_def: &[(SortKey, &str)] = &[
            (SortKey::Name, "Name"),
            (SortKey::Path, "Pfad"),
            (SortKey::Size, "Größe"),
            (SortKey::Mtime, "Geändert"),
            (SortKey::Btime, "Erstellt"),
            (SortKey::Ext, "Typ"),
            (SortKey::Depth, "Tiefe"),
        ];

        let mut builder = TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(240.0).at_least(120.0).resizable(true).clip(true)) // name
            .column(Column::initial(360.0).at_least(120.0).resizable(true).clip(true)) // path
            .column(Column::initial(90.0).at_least(60.0).resizable(true)) // size
            .column(Column::initial(130.0).at_least(80.0).resizable(true)) // mtime
            .column(Column::initial(130.0).at_least(80.0).resizable(true)) // btime
            .column(Column::initial(60.0).at_least(40.0).resizable(true)) // ext
            .column(Column::remainder().at_least(40.0)); // depth

        if let Some(r) = self.pending_scroll_row.take() {
            builder = builder.scroll_to_row(r, Some(egui::Align::Center));
        }

        builder
            .header(22.0, |mut header| {
                for (key, label) in header_def {
                    header.col(|ui| {
                        let arrow = if self.sort_key == *key {
                            if self.sort_dir == SortDir::Asc {
                                " ▲"
                            } else {
                                " ▼"
                            }
                        } else {
                            ""
                        };
                        let txt = RichText::new(format!("{}{}", label, arrow)).strong();
                        if ui.selectable_label(self.sort_key == *key, txt).clicked() {
                            sort_clicked = Some(*key);
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(row_h, total_rows, |mut row| {
                    let row_index = row.index();
                    let (entry_idx, display_depth) = self.view[row_index];
                    let e = &self.entries[entry_idx];
                    let selected = self.selection.contains(&e.path);
                    row.set_selected(selected);

                    let mut handle_resp = |resp: egui::Response, ui: &egui::Ui| {
                        if resp.clicked() {
                            let m = ui.input(|i| {
                                (i.modifiers.ctrl || i.modifiers.command, i.modifiers.shift)
                            });
                            row_click = Some((entry_idx, m.0, m.1));
                        }
                        if resp.double_clicked() {
                            row_dblclick = Some(entry_idx);
                        }
                        if resp.secondary_clicked() {
                            row_rclick = Some(entry_idx);
                        }
                        // Dragging a row begins a file drag (resolved after the
                        // table). The rubber-band bails when a drag is active, so
                        // these don't fight.
                        if resp.drag_started() {
                            drag_start = Some(entry_idx);
                        }
                    };

                    let handle_cell = |ui: &mut egui::Ui, content: &str, right_align: bool| {
                        let cell_w = ui.available_width();
                        let (rect, resp) = ui.allocate_exact_size(
                            egui::vec2(cell_w, row_h),
                            egui::Sense::click_and_drag(),
                        );
                        let color = if selected {
                            ui.visuals().selection.stroke.color
                        } else {
                            ui.visuals().text_color()
                        };
                        paint_cell_text(ui, rect, content, right_align, color, 0.0);
                        resp
                    };

                    // ─── Name (with indent + native icon) ──────────────
                    row.col(|ui| {
                        let cell_w = ui.available_width();
                        let (rect, resp) = ui.allocate_exact_size(
                            egui::vec2(cell_w, row_h),
                            egui::Sense::click_and_drag(),
                        );
                        visible_rows.push((row_index, rect));
                        let indent = display_depth.min(32) as f32 * 14.0;
                        let color = if selected {
                            ui.visuals().selection.stroke.color
                        } else {
                            ui.visuals().text_color()
                        };
                        // 16px icon slot at the left of the cell (after indent);
                        // the name always sits at indent+20 so it never shifts
                        // when the real icon replaces the emoji placeholder.
                        let icon_center = egui::pos2(
                            rect.left() + 4.0 + indent + 8.0,
                            rect.center().y,
                        );
                        let key = crate::icons::icon_key(e.is_dir, e.ext.as_ref());
                        if let Some(tex) = self.icon_cache.get(&key) {
                            let icon_rect = egui::Rect::from_center_size(
                                icon_center,
                                egui::vec2(16.0, 16.0),
                            );
                            egui::Image::from_texture(egui::load::SizedTexture::new(
                                tex.id(),
                                egui::vec2(16.0, 16.0),
                            ))
                            .paint_at(ui, icon_rect);
                        } else {
                            needed_icons.push(key);
                            let emoji = if e.is_dir { "📁" } else { "📄" };
                            ui.painter().text(
                                icon_center,
                                egui::Align2::CENTER_CENTER,
                                emoji,
                                egui::TextStyle::Body.resolve(ui.style()),
                                color,
                            );
                        }
                        paint_cell_text(ui, rect, e.name.as_ref(), false, color, indent + 20.0);
                        handle_resp(resp, ui);
                    });

                    // ─── Path (relative) ───────────────────────────────
                    row.col(|ui| {
                        let rel = if e.path.starts_with(&prefix) {
                            let r = e
                                .path
                                .as_ref()
                                .trim_start_matches(prefix.as_str())
                                .trim_start_matches('/');
                            if r.is_empty() {
                                "/".to_string()
                            } else {
                                r.to_string()
                            }
                        } else {
                            e.path.to_string()
                        };
                        let resp = handle_cell(ui, &rel, false);
                        handle_resp(resp, ui);
                    });

                    // ─── Size ──────────────────────────────────────────
                    row.col(|ui| {
                        let txt = if e.is_dir { String::new() } else { format_bytes(e.size) };
                        let resp = handle_cell(ui, &txt, true);
                        handle_resp(resp, ui);
                    });

                    // ─── Dates ─────────────────────────────────────────
                    row.col(|ui| {
                        let resp = handle_cell(ui, &format_date(e.mtime_ms), false);
                        handle_resp(resp, ui);
                    });
                    row.col(|ui| {
                        let resp = handle_cell(ui, &format_date(e.btime_ms), false);
                        handle_resp(resp, ui);
                    });

                    // ─── Ext ───────────────────────────────────────────
                    row.col(|ui| {
                        let resp = handle_cell(ui, e.ext.as_ref(), false);
                        handle_resp(resp, ui);
                    });

                    // ─── Depth ─────────────────────────────────────────
                    row.col(|ui| {
                        let resp = handle_cell(ui, &format!("{}", e.depth), true);
                        handle_resp(resp, ui);
                    });
                });
            });

        // A row drag started → capture the files (the whole selection if the
        // dragged row is part of it, otherwise just that row). Local files only
        // (remote items would need a download to drop elsewhere).
        if let Some(idx) = drag_start {
            if !self.drag_active {
                let dragged = self.entries[idx].path.clone();
                let mut files: Vec<String> = if self.selection.contains(&dragged) {
                    self.selection.iter().map(|p| p.to_string()).collect()
                } else {
                    vec![dragged.to_string()]
                };
                // From a local view we only carry local paths; from a remote view
                // the paths are remote and `drag_src` is the source backend.
                if self.remote.is_none() {
                    files.retain(|p| is_local_style(p));
                }
                if !files.is_empty() {
                    self.drag_files = files;
                    self.drag_active = true;
                    self.drag_src = self.remote.as_ref().map(|rs| rs.backend.clone());
                    self.drag_source_tab = self.current_render_tab;
                    self.drag_out_started = false;
                }
            }
        }

        if let Some(k) = sort_clicked {
            if self.sort_key == k {
                self.sort_dir = if self.sort_dir == SortDir::Asc {
                    SortDir::Desc
                } else {
                    SortDir::Asc
                };
            } else {
                self.sort_key = k;
                self.sort_dir = SortDir::Asc;
            }
            self.recompute_view();
        }

        if let Some((idx, ctrl, shift)) = row_click {
            let path = self.entries[idx].path.clone();
            if shift && !ctrl {
                // Explorer semantics: Shift+Click replaces the selection with
                // the anchor→clicked range.
                if let Some(anchor) = self.last_anchor.clone() {
                    let a = self
                        .view
                        .iter()
                        .position(|&(i, _)| self.entries[i].path == anchor);
                    let b = self.view.iter().position(|&(i, _)| self.entries[i].path == path);
                    if let (Some(a), Some(b)) = (a, b) {
                        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                        self.selection.clear();
                        for i in lo..=hi {
                            self.selection
                                .insert(self.entries[self.view[i].0].path.clone());
                        }
                    } else {
                        self.selection.insert(path.clone());
                    }
                } else {
                    self.selection.insert(path.clone());
                    self.last_anchor = Some(path.clone());
                }
            } else if shift && ctrl {
                // Ctrl+Shift+Click: add range to existing selection
                if let Some(anchor) = self.last_anchor.clone() {
                    let a = self
                        .view
                        .iter()
                        .position(|&(i, _)| self.entries[i].path == anchor);
                    let b = self.view.iter().position(|&(i, _)| self.entries[i].path == path);
                    if let (Some(a), Some(b)) = (a, b) {
                        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                        for i in lo..=hi {
                            self.selection
                                .insert(self.entries[self.view[i].0].path.clone());
                        }
                    }
                }
            } else if ctrl {
                if !self.selection.insert(path.clone()) {
                    self.selection.remove(&path);
                }
                self.last_anchor = Some(path.clone());
            } else {
                self.selection.clear();
                self.selection.insert(path.clone());
                self.last_anchor = Some(path.clone());
            }
            self.cursor = Some(path);
        }

        if let Some(idx) = row_dblclick {
            self.activate_entry(idx);
        }

        if let Some(idx) = row_rclick {
            let path_arc = self.entries[idx].path.clone();
            if !self.selection.contains(&path_arc) {
                self.selection.clear();
                self.selection.insert(path_arc.clone());
                self.last_anchor = Some(path_arc.clone());
            }
            // Remotes have no Windows shell menu (those paths aren't local) — show
            // our own egui context menu instead.
            if self.remote.is_some() {
                let pos = ui
                    .ctx()
                    .input(|i| i.pointer.interact_pos())
                    .unwrap_or_else(|| ui.min_rect().center());
                self.remote_ctx = Some((pos, idx));
            } else {
                let path = path_arc.to_string();
                let ctx = ui.ctx().clone();
                self.show_shell_menu_for(&path, &ctx);
            }
        }

        // ─── Rubber-band selection + empty-space interactions ─────────────
        let table_rect = ui.min_rect();
        let body_viewport = egui::Rect::from_min_max(
            egui::pos2(table_rect.left(), table_rect.top() + 24.0),
            table_rect.max,
        );

        let (primary_pressed, primary_down, primary_released, ptr_pos, ctrl_now, secondary_clicked) =
            ui.input(|i| {
                (
                    i.pointer.primary_pressed(),
                    i.pointer.primary_down(),
                    i.pointer.primary_released(),
                    i.pointer.latest_pos(),
                    i.modifiers.ctrl || i.modifiers.command,
                    i.pointer.secondary_clicked(),
                )
            });

        // base_y maps content row i to screen y: row_top(i) = base_y + i*row_h
        let base_y = visible_rows
            .first()
            .map(|&(idx, rect)| rect.top() - idx as f32 * row_h);

        let anything_dragged = ui.ctx().dragged_id().is_some();

        // A row was interacted with this frame? Then the pointer is over a row,
        // not empty space — the rubber-band / empty-space-clear logic must not
        // touch the selection that the row handlers just set.
        let row_hit = row_click.is_some() || row_dblclick.is_some() || row_rclick.is_some();

        if primary_pressed && !anything_dragged && !self.band_suppressed {
            if let Some(p) = ptr_pos {
                if body_viewport.contains(p) {
                    // Store the press in SCREEN coordinates so the drag-distance
                    // test is stable even if the table's base-Y shifts a pixel
                    // when layout settles (which previously could both spuriously
                    // start a band and mis-clear the bottom row's selection).
                    self.band_press = Some((p.x, p.y));
                    self.band_base = if ctrl_now {
                        self.selection.clone()
                    } else {
                        HashSet::new()
                    };
                }
            }
        }

        if let Some((press_x, press_y)) = self.band_press.filter(|_| !self.band_suppressed) {
            if anything_dragged {
                // A column-resize (or other) drag claimed the pointer.
                self.band_press = None;
                self.band_active = false;
            } else if primary_down {
                if let (Some(p), Some(by)) = (ptr_pos, base_y) {
                    if self.band_active
                        || (p.y - press_y).abs() > 4.0
                        || (p.x - press_x).abs() > 4.0
                    {
                        self.band_active = true;
                        let (lo_y, hi_y) = if press_y < p.y { (press_y, p.y) } else { (p.y, press_y) };
                        // Map both screen endpoints to rows via the current base-Y.
                        let lo_off = lo_y - by;
                        let hi_off = hi_y - by;
                        let mut sel = self.band_base.clone();
                        if total_rows > 0 && hi_off >= 0.0 {
                            let lo_row = (lo_off / row_h).floor().max(0.0) as usize;
                            let hi_row =
                                ((hi_off / row_h).floor() as isize).min(total_rows as isize - 1);
                            if hi_row >= 0 && lo_row < total_rows {
                                for r in lo_row..=(hi_row as usize) {
                                    sel.insert(self.entries[self.view[r].0].path.clone());
                                }
                            }
                        }
                        self.selection = sel;

                        // Draw the band (screen coords, clamped to the viewport)
                        let y0 = lo_y.max(body_viewport.top());
                        let y1 = hi_y.min(body_viewport.bottom());
                        let x0 = press_x.min(p.x).max(body_viewport.left());
                        let x1 = press_x.max(p.x).min(body_viewport.right());
                        if y1 > y0 && x1 > x0 {
                            let rect = egui::Rect::from_min_max(
                                egui::pos2(x0, y0),
                                egui::pos2(x1, y1),
                            );
                            let painter = ui.painter();
                            painter.rect_filled(
                                rect,
                                0.0,
                                Color32::from_rgba_unmultiplied(90, 140, 255, 36),
                            );
                            painter.rect_stroke(
                                rect,
                                0.0,
                                egui::Stroke::new(1.0, Color32::from_rgb(90, 140, 255)),
                            );
                        }

                        // Auto-scroll when the pointer leaves the viewport
                        if p.y > body_viewport.bottom() - 4.0 {
                            let bottom_row =
                                (((body_viewport.bottom() - by) / row_h) as usize + 2)
                                    .min(total_rows.saturating_sub(1));
                            self.pending_scroll_row = Some(bottom_row);
                        } else if p.y < body_viewport.top() + 4.0 {
                            let top_row =
                                (((body_viewport.top() - by) / row_h).max(0.0) as usize)
                                    .saturating_sub(2);
                            self.pending_scroll_row = Some(top_row);
                        }
                        ui.ctx().request_repaint();
                    }
                }
            }
            if primary_released {
                // Click (no drag) on empty space below the rows clears the
                // selection, like Explorer — but ONLY if the click didn't land
                // on a row (otherwise we'd wipe the just-made selection).
                if !self.band_active && !row_hit {
                    if let (Some(p), Some(by)) = (ptr_pos, base_y) {
                        let last_bottom = by + total_rows as f32 * row_h;
                        if p.y > last_bottom + 2.0 && body_viewport.contains(p) {
                            self.selection.clear();
                            self.cursor = None;
                        }
                    }
                }
                self.band_press = None;
                self.band_active = false;
            }
        }

        // Right-click on empty space → folder background menu
        if secondary_clicked && row_rclick.is_none() {
            if let Some(p) = ptr_pos {
                let on_empty = match base_y {
                    Some(by) => p.y > by + total_rows as f32 * row_h,
                    None => true,
                };
                if body_viewport.contains(p) && on_empty {
                    self.show_background_menu();
                }
            }
        }

        // Queue icon extraction for any type seen this frame but not cached.
        for key in needed_icons {
            self.icon_cache.request(key);
        }
    }

    fn build_summary(&self) -> SummaryData {
        let mut files = 0u64;
        let mut dirs = 0u64;
        let mut bytes = 0u64;
        let mut by_ext: std::collections::HashMap<&str, (u64, u64)> =
            std::collections::HashMap::new();
        let mut oldest = i64::MAX;
        let mut newest = 0i64;
        let mut top: Vec<&FileEntry> = Vec::new();

        for &(i, _) in &self.view {
            let e = &self.entries[i];
            if e.is_dir {
                dirs += 1;
            } else {
                files += 1;
                bytes += e.size;
                let k = if e.ext.is_empty() { "(none)" } else { e.ext.as_ref() };
                let entry = by_ext.entry(k).or_insert((0, 0));
                entry.0 += 1;
                entry.1 += e.size;
                if e.mtime_ms != 0 && e.mtime_ms < oldest {
                    oldest = e.mtime_ms;
                }
                if e.mtime_ms > newest {
                    newest = e.mtime_ms;
                }
                if top.len() < 10 {
                    top.push(e);
                    top.sort_by(|a, b| b.size.cmp(&a.size));
                } else if e.size > top.last().unwrap().size {
                    *top.last_mut().unwrap() = e;
                    top.sort_by(|a, b| b.size.cmp(&a.size));
                }
            }
        }

        let mut by_ext_v: Vec<(String, u64, u64)> = by_ext
            .into_iter()
            .map(|(k, (c, b))| (k.to_string(), c, b))
            .collect();
        by_ext_v.sort_by(|a, b| b.2.cmp(&a.2));
        by_ext_v.truncate(15);

        SummaryData {
            files,
            dirs,
            bytes,
            oldest,
            newest,
            by_ext: by_ext_v,
            top: top
                .into_iter()
                .map(|e| (e.name.to_string(), e.path.to_string(), e.size))
                .collect(),
        }
    }

    fn ui_summary(&mut self, ui: &mut egui::Ui) {
        if self.summary_cache.is_none() {
            self.summary_cache = Some(self.build_summary());
        }
        let s = self.summary_cache.as_ref().unwrap();

        ui.heading("Zusammenfassung");
        ui.add_space(4.0);
        egui::Grid::new("summary_kv").num_columns(2).striped(false).show(ui, |ui| {
            ui.label("Dateien");
            ui.label(format!("{}", s.files));
            ui.end_row();
            ui.label("Ordner");
            ui.label(format!("{}", s.dirs));
            ui.end_row();
            ui.label("Gesamtgröße");
            ui.label(format_bytes(s.bytes));
            ui.end_row();
            if s.oldest != i64::MAX {
                ui.label("Älteste");
                ui.label(format_date(s.oldest));
                ui.end_row();
            }
            if s.newest > 0 {
                ui.label("Neueste");
                ui.label(format_date(s.newest));
                ui.end_row();
            }
        });

        ui.add_space(8.0);
        ui.label(RichText::new("TOP-DATEITYPEN").small().color(Color32::from_gray(140)));
        for (k, count, bytes) in &s.by_ext {
            ui.horizontal(|ui| {
                ui.colored_label(Color32::from_rgb(80, 140, 255), RichText::new(k).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format_bytes(*bytes));
                    ui.label(format!("{} ×", count));
                });
            });
        }

        ui.add_space(8.0);
        ui.label(RichText::new("GRÖSSTE DATEIEN").small().color(Color32::from_gray(140)));
        for (name, path, size) in s.top.iter().take(10) {
            ui.horizontal(|ui| {
                ui.colored_label(Color32::from_rgb(80, 140, 255), format_bytes(*size));
                ui.add(egui::Label::new(name).truncate()).on_hover_text(path);
            });
        }
    }

    fn selection_bytes(&mut self) -> u64 {
        if self.sel_size_cache.0 == self.selection.len()
            && self.sel_size_cache.1 == self.entries.len()
        {
            return self.sel_size_cache.2;
        }
        let b: u64 = self
            .entries
            .iter()
            .filter(|e| !e.is_dir && self.selection.contains(&e.path))
            .map(|e| e.size)
            .sum();
        self.sel_size_cache = (self.selection.len(), self.entries.len(), b);
        b
    }

    fn ui_status(&mut self, ui: &mut egui::Ui) {
        let sel_bytes = self.selection_bytes();
        ui.horizontal(|ui| {
            if self.scan_running {
                ui.label("⟳ Scan läuft…");
            } else if !self.entries.is_empty() {
                ui.label("✓ Bereit");
            }
            let p = &self.progress;
            let rate = if p.elapsed_ms > 0 {
                (p.scanned as f64 / p.elapsed_ms as f64) * 1000.0
            } else {
                0.0
            };
            let rate_s = if rate >= 1000.0 {
                format!("{:.1}k/s", rate / 1000.0)
            } else {
                format!("{:.0}/s", rate)
            };
            ui.colored_label(
                Color32::from_gray(140),
                format!(
                    "{} gescannt · {} · {:.1}s · {}{}",
                    p.scanned,
                    format_bytes(p.bytes),
                    p.elapsed_ms as f64 / 1000.0,
                    rate_s,
                    if p.errors > 0 {
                        format!(" · {} Fehler", p.errors)
                    } else {
                        String::new()
                    },
                ),
            );
            if !p.current_path.is_empty() && self.scan_running {
                ui.colored_label(
                    Color32::from_gray(110),
                    egui::RichText::new(&p.current_path).monospace().small(),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.colored_label(Color32::from_gray(140), format!("v{}", env!("CARGO_PKG_VERSION")));
                if let Some((ref msg, ts)) = self.notice {
                    if ts.elapsed().as_secs() < 6 {
                        ui.colored_label(Color32::from_rgb(120, 200, 130), msg.clone());
                    }
                }
                if let Some(ref e) = self.error_msg {
                    ui.colored_label(Color32::from_rgb(220, 100, 80), format!("⚠ {}", e));
                }
                if !self.failed_paths.is_empty() || p.errors > 0 {
                    let label = format!(
                        "⚠ {} Fehler",
                        p.errors.max(self.failed_paths.len() as u64)
                    );
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(label).color(Color32::from_rgb(220, 100, 80)),
                            )
                            .small(),
                        )
                        .on_hover_text("Pfade anzeigen, die nicht gelesen werden konnten")
                        .clicked()
                    {
                        self.show_errors_dialog = true;
                    }
                }
                if self.selection.is_empty() {
                    ui.colored_label(Color32::from_gray(140), "Auswahl: 0");
                } else {
                    ui.colored_label(
                        Color32::from_gray(160),
                        format!(
                            "Auswahl: {} ({})",
                            self.selection.len(),
                            format_bytes(sel_bytes)
                        ),
                    );
                }
            });
        });
    }

    fn ui_errors_dialog(&mut self, ctx: &egui::Context) {
        let mut close = false;
        egui::Window::new(format!("Nicht lesbare Pfade ({})", self.failed_paths.len()))
            .resizable(true)
            .default_size([700.0, 480.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("Diese Pfade konnten nicht gelistet werden (Berechtigung, Reparse-Point, etc.):");
                ui.add_space(6.0);
                egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                    egui::Grid::new("errs").num_columns(2).striped(true).show(ui, |ui| {
                        for (p, msg) in &self.failed_paths {
                            ui.add(egui::Label::new(p).truncate()).on_hover_text(p);
                            ui.colored_label(Color32::from_gray(140), msg);
                            ui.end_row();
                        }
                    });
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Liste in Zwischenablage").clicked() {
                        let txt: String = self
                            .failed_paths
                            .iter()
                            .map(|(p, m)| format!("{}\t{}", p, m))
                            .collect::<Vec<_>>()
                            .join("\r\n");
                        ctx.copy_text(txt);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Schließen").clicked() {
                            close = true;
                        }
                    });
                });
            });
        if close {
            self.show_errors_dialog = false;
        }
    }

    fn ui_rename_dialog(&mut self, ctx: &egui::Context) {
        let mut confirm = false;
        let mut cancel = false;
        let mut focus = self.rename_focus;
        if let Some((path, draft)) = self.rename_open.as_mut() {
            let title = path.rsplit('/').next().unwrap_or("").to_string();
            egui::Window::new(format!("Umbenennen: {}", title))
                .fixed_size([420.0, 80.0])
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    let resp = ui.add(
                        egui::TextEdit::singleline(draft).desired_width(f32::INFINITY),
                    );
                    if focus {
                        resp.request_focus();
                        focus = false;
                    }
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        confirm = true;
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        cancel = true;
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(RichText::new("Umbenennen").strong()).clicked() {
                                confirm = true;
                            }
                            if ui.button("Abbrechen").clicked() {
                                cancel = true;
                            }
                        });
                    });
                });
        }
        self.rename_focus = focus;
        if confirm {
            self.confirm_rename();
        } else if cancel {
            self.rename_open = None;
        }
    }

    fn ui_update_dialog(&mut self, ctx: &egui::Context) {
        let (version, exe) = match self.update_ready.clone() {
            Some(v) => v,
            None => return,
        };
        let mut restart = false;
        let mut later = false;
        egui::Window::new("Update bereit")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "Version {} wurde installiert. Zum Übernehmen ist ein Neustart nötig.",
                    version
                ));
                ui.colored_label(
                    Color32::from_gray(150),
                    "„Später“ behält die laufende Version bei; das Update greift beim nächsten Start.",
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Jetzt neu starten").strong()).clicked() {
                        restart = true;
                    }
                    if ui.button("Später").clicked() {
                        later = true;
                    }
                });
            });
        if restart {
            let _ = std::process::Command::new(&exe).arg("--updated").spawn();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if later {
            self.update_ready = None;
            self.notice = Some((
                format!("Update v{} greift beim nächsten Start", version),
                std::time::Instant::now(),
            ));
        }
    }

    fn ui_connect_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_connect {
            return;
        }
        use crate::creds::Protocol;
        let mut do_connect = false;
        let mut close = false;
        let mut open = true;
        egui::Window::new("Verbinden (SFTP / FTP / Netzlaufwerk)")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .fixed_size([440.0, 0.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                let f = &mut self.connect_form;
                egui::ComboBox::from_label("Protokoll")
                    .selected_text(match f.protocol {
                        Protocol::Sftp => "SFTP",
                        Protocol::Ftp => "FTP",
                        Protocol::Ftps => "FTPS",
                        Protocol::Webdav => "WebDAV (HTTPS)",
                        Protocol::Share => "Netzlaufwerk (UNC)",
                    })
                    .show_ui(ui, |ui| {
                        for (p, lbl) in [
                            (Protocol::Sftp, "SFTP"),
                            (Protocol::Ftp, "FTP"),
                            (Protocol::Ftps, "FTPS"),
                            (Protocol::Webdav, "WebDAV (HTTPS)"),
                            (Protocol::Share, "Netzlaufwerk (UNC)"),
                        ] {
                            if ui.selectable_label(f.protocol == p, lbl).clicked() {
                                f.protocol = p;
                                if p != Protocol::Share && f.port.trim().is_empty() {
                                    f.port = p.default_port().to_string();
                                }
                            }
                        }
                    });
                ui.add_space(4.0);

                egui::Grid::new("connect_grid")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        if f.protocol == Protocol::Share {
                            ui.label("Freigabe (UNC)");
                            ui.add(
                                egui::TextEdit::singleline(&mut f.unc)
                                    .hint_text(r"\\server\share")
                                    .desired_width(f32::INFINITY),
                            );
                            ui.end_row();
                            ui.label("Benutzer");
                            ui.add(egui::TextEdit::singleline(&mut f.user).desired_width(f32::INFINITY));
                            ui.end_row();
                            ui.label("Passwort");
                            ui.add(egui::TextEdit::singleline(&mut f.password).password(true).desired_width(f32::INFINITY));
                            ui.end_row();
                        } else {
                            ui.label("Host");
                            ui.add(egui::TextEdit::singleline(&mut f.host).hint_text("host.example.com").desired_width(f32::INFINITY));
                            ui.end_row();
                            ui.label("Port");
                            ui.add(egui::TextEdit::singleline(&mut f.port).desired_width(70.0));
                            ui.end_row();
                            ui.label("Benutzer");
                            ui.add(egui::TextEdit::singleline(&mut f.user).desired_width(f32::INFINITY));
                            ui.end_row();
                            ui.label("Startpfad");
                            ui.add(egui::TextEdit::singleline(&mut f.root).hint_text("/").desired_width(f32::INFINITY));
                            ui.end_row();
                        }
                    });

                if f.protocol == Protocol::Sftp {
                    ui.checkbox(&mut f.use_key, "Mit Schlüsseldatei anmelden");
                }
                if f.protocol == Protocol::Sftp && f.use_key {
                    ui.horizontal(|ui| {
                        ui.label("Schlüssel");
                        ui.add(egui::TextEdit::singleline(&mut f.keyfile).desired_width(220.0));
                        if ui.button("…").clicked() {
                            if let Some(p) = rfd::FileDialog::new().pick_file() {
                                f.keyfile = p.to_string_lossy().replace('\\', "/");
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Passphrase");
                        ui.add(egui::TextEdit::singleline(&mut f.passphrase).password(true).desired_width(220.0));
                    });
                } else if f.protocol != Protocol::Share {
                    ui.horizontal(|ui| {
                        ui.label("Passwort");
                        ui.add(egui::TextEdit::singleline(&mut f.password).password(true).desired_width(f32::INFINITY));
                    });
                }

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.checkbox(&mut f.save, "Speichern");
                    ui.add(egui::TextEdit::singleline(&mut f.label).hint_text("Bezeichnung (optional)").desired_width(f32::INFINITY));
                });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if self.connecting {
                        ui.spinner();
                        ui.label("Verbinde…");
                    } else {
                        if ui.button(RichText::new("Verbinden").strong()).clicked() {
                            do_connect = true;
                        }
                        if ui.button("Abbrechen").clicked() {
                            close = true;
                        }
                    }
                });
            });
        if !open {
            close = true;
        }
        if do_connect {
            let form = self.connect_form.clone();
            self.begin_connect(form, None);
        } else if close && !self.connecting {
            self.show_connect = false;
        }
    }

    /// First-run liability notice. Modal-ish (foreground, dimmed backdrop);
    /// must be acknowledged once. The acceptance is recorded in appdata so it
    /// doesn't reappear.
    fn ui_disclaimer(&mut self, ctx: &egui::Context) {
        if !self.show_disclaimer {
            return;
        }
        // Dim everything behind the notice.
        egui::Area::new(egui::Id::new("disclaimer_backdrop"))
            .order(egui::Order::Background)
            .show(ctx, |ui| {
                let r = ui.ctx().screen_rect();
                ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha(200));
            });
        let mut accept = false;
        egui::Window::new("Hinweis & Haftungsausschluss")
            .order(egui::Order::Foreground)
            .collapsible(false)
            .resizable(false)
            .fixed_size([560.0, 0.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                    ui.label(DISCLAIMER_TEXT);
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(RichText::new("Verstanden — auf eigenes Risiko fortfahren").strong())
                        .clicked()
                    {
                        accept = true;
                    }
                    if ui.button("Beenden").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
        if accept {
            let _ = std::fs::write(appdata_file("disclaimer_ack.txt"), "1");
            self.show_disclaimer = false;
        }
    }

    fn ui_help_dialog(&mut self, ctx: &egui::Context) {
        let mut open = self.show_help;
        egui::Window::new("Tastenkürzel")
            .open(&mut open)
            .resizable(true)
            .default_size([520.0, 560.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let groups: &[(&str, &[(&str, &str)])] = &[
                        (
                            "Navigation",
                            &[
                                ("Alt+←/→", "Zurück / Vor"),
                                ("Alt+↑  ·  Backspace", "Eine Ebene hoch"),
                                ("Enter", "Öffnen (Ordner betreten / Datei öffnen)"),
                                ("F5", "Aktualisieren"),
                                ("Ctrl+L", "Pfad bearbeiten"),
                                ("Ctrl+R", "Rekursiv umschalten"),
                                ("Ctrl+F", "Ordnersuche fokussieren"),
                                ("F3", "Namensfilter fokussieren"),
                            ],
                        ),
                        (
                            "Tabs",
                            &[
                                ("Ctrl+T", "Neuer Tab"),
                                ("Ctrl+W", "Tab schließen"),
                                ("Ctrl+Tab / Ctrl+Shift+Tab", "Nächster / vorheriger Tab"),
                            ],
                        ),
                        (
                            "Auswahl",
                            &[
                                ("Klick / Ziehen", "Auswählen / Rechteck-Auswahl"),
                                ("Ctrl+Klick", "Einzeln hinzufügen/entfernen"),
                                ("Shift+Klick / Shift+Pfeile", "Bereich auswählen"),
                                ("Ctrl+A", "Alles auswählen"),
                                ("Ctrl+I", "Auswahl umkehren"),
                                ("Esc", "Auswahl aufheben"),
                                ("↑/↓ · PageUp/Down · Home/End", "Cursor bewegen"),
                                ("Tippen", "Zum Eintrag springen"),
                            ],
                        ),
                        (
                            "Dateiaktionen",
                            &[
                                ("Ctrl+C / Ctrl+X / Ctrl+V", "Kopieren / Ausschneiden / Einfügen"),
                                ("Ctrl+Shift+C", "Pfade als Text kopieren"),
                                ("Entf", "In den Papierkorb"),
                                ("Shift+Entf", "Endgültig löschen"),
                                ("F2", "Umbenennen"),
                                ("Ctrl+Shift+N", "Neuer Ordner"),
                                ("Alt+Enter", "Eigenschaften"),
                                ("Ctrl+Shift+E", "Im Explorer anzeigen"),
                                ("Ctrl+B", "Aktuellen Ordner zu Favoriten"),
                            ],
                        ),
                        ("Sonstiges", &[("F1", "Diese Hilfe")]),
                    ];
                    for (title, rows) in groups {
                        ui.add_space(4.0);
                        ui.label(RichText::new(*title).strong().color(Color32::from_rgb(120, 170, 255)));
                        egui::Grid::new(*title)
                            .num_columns(2)
                            .striped(true)
                            .spacing([16.0, 2.0])
                            .show(ui, |ui| {
                                for (k, d) in *rows {
                                    ui.label(RichText::new(*k).monospace());
                                    ui.label(*d);
                                    ui.end_row();
                                }
                            });
                    }
                });
            });
        self.show_help = open;
    }

    fn ui_copy_dialog(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let title = if self.copy_mode_pending == CopyMode::Copy {
            "Kopieren"
        } else {
            "Verschieben"
        };
        let running = matches!(&self.copy_progress, Some(p) if !p.done);
        let done = matches!(&self.copy_progress, Some(p) if p.done);

        egui::Window::new(title)
            .fixed_size([560.0, 280.0])
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!("{} Einträge ausgewählt", self.selection.len()));
                ui.horizontal(|ui| {
                    ui.label("Modus:");
                    ui.radio_value(&mut self.copy_mode_pending, CopyMode::Copy, "kopieren");
                    ui.radio_value(&mut self.copy_mode_pending, CopyMode::Move, "verschieben");
                });
                ui.colored_label(
                    egui::Color32::from_gray(160),
                    "Ordner werden rekursiv expandiert; nur Dateien die dem aktuellen Filter entsprechen werden kopiert. Ordnerstruktur wird erhalten, leere Ordner weggelassen.",
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Ziel:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.copy_dest)
                            .desired_width(360.0)
                            .hint_text("Zielordner…"),
                    );
                    if ui.add_enabled(!running, egui::Button::new("Wählen…")).clicked() {
                        if let Some(p) = rfd::FileDialog::new().pick_folder() {
                            self.copy_dest = p.to_string_lossy().to_string();
                        }
                    }
                });
                ui.checkbox(
                    &mut self.copy_preserve,
                    "Ordnerstruktur erhalten (leere Ordner werden weggelassen)",
                );
                ui.horizontal(|ui| {
                    ui.label("Bei Konflikt:");
                    ui.radio_value(&mut self.copy_conflict, Conflict::Rename, "umbenennen");
                    ui.radio_value(&mut self.copy_conflict, Conflict::Overwrite, "überschreiben");
                    ui.radio_value(&mut self.copy_conflict, Conflict::Skip, "überspringen");
                });

                if let Some(ref p) = self.copy_progress {
                    let frac = if p.bytes_total > 0 {
                        p.bytes_done as f32 / p.bytes_total as f32
                    } else if p.files_total > 0 {
                        p.files_done as f32 / p.files_total as f32
                    } else {
                        0.0
                    };
                    ui.add(egui::ProgressBar::new(frac).show_percentage());
                    ui.label(format!(
                        "{}/{} Dateien · {} / {} · {:.1}s{}",
                        p.files_done,
                        p.files_total,
                        format_bytes(p.bytes_done),
                        format_bytes(p.bytes_total),
                        p.elapsed_ms as f64 / 1000.0,
                        if p.errors > 0 {
                            format!(" · {} Fehler", p.errors)
                        } else {
                            String::new()
                        },
                    ));
                }

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                !self.copy_dest.is_empty() && !running,
                                egui::Button::new(RichText::new("Start").strong()),
                            )
                            .clicked()
                        {
                            self.confirm_copy();
                        }
                        if ui.add_enabled(!running, egui::Button::new("Abbrechen")).clicked() {
                            close = true;
                        }
                    });
                });
            });

        if close || done {
            self.copy_open = false;
            if done {
                if self.copy_mode_pending == CopyMode::Move {
                    let removed: HashSet<Arc<str>> = self.selection.drain().collect();
                    self.entries.retain(|e| !removed.contains(&e.path));
                    self.recompute_view();
                }
            }
        }
    }
}

impl eframe::App for App {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(h) = self.scan_handle.take() {
            h.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        for t in &mut self.tabs {
            if let Some(h) = t.scan_handle.take() {
                h.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        if let Some(h) = self.copy_handle.take() {
            h.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some(c) = self.index_cancel.take() {
            c.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some(c) = self.clip_key_cancel.take() {
            c.store(true, std::sync::atomic::Ordering::Relaxed);
        }

        if self.index_dirty {
            let _ = self.folder_index.save(&folder_index_path());
        }
        #[cfg(windows)]
        {
            self.watcher = None;
            self.watcher_rx = None;
        }

        self.entries = Vec::new();
        self.view = Vec::new();
        self.selection = HashSet::new();
        self.recent = Vec::new();
        self.tabs = Vec::new();
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Pump background channels
        self.drain_scan();

        // Maximize once, after the first frame is laid out, so the app opens as
        // a proper maximized window without the builder-`maximized` flashbang
        // (see main.rs).
        if !self.shown {
            self.shown = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            ctx.request_repaint();
        }

        self.drain_inactive_tabs();
        self.drain_copy();
        self.drain_index();
        self.drain_watcher();
        self.drain_folder_search();
        self.drain_trash();
        self.drain_clip_prepare();
        self.drain_update();
        self.drain_connect();
        self.drain_sync();
        self.drain_bisync();
        self.drain_job_connect();
        self.drain_picker_connect();
        self.drain_cloud_auth();
        self.drain_file_open();
        self.poll_remote_edits();
        self.drain_edit_saves();
        self.drain_upload();
        self.drain_remote_op();
        self.drain_clip_download();
        self.drain_share();
        self.drain_quickshare();
        if self.icon_cache.drain(ctx) {
            ctx.request_repaint();
        }
        self.maybe_save_index();

        // Files dropped onto the window from the OS (Explorer/desktop) → land
        // in the current folder. Processed once per frame.
        self.handle_os_drop(ctx);

        // Open the command-line path on the first frame (folder double-click /
        // "Open in Smart Explorer" / default-manager handoff). A file path
        // opens its parent folder.
        if let Some(p) = self.pending_initial_path.take() {
            let target = if p.is_dir() {
                Some(p)
            } else {
                p.parent().map(|q| q.to_path_buf())
            };
            if let Some(t) = target {
                if t.exists() {
                    self.start_scan(t);
                }
            }
        }

        // Throttled view rebuild while a scan streams entries in
        if self.view_dirty
            && (!self.scan_running || self.last_view_recompute.elapsed().as_millis() >= 150)
        {
            self.recompute_view();
        }
        if self.view_dirty {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }

        // Debounced folder search (80 ms after last keystroke)
        if let Some(ts) = self.folder_search_pending_at {
            if ts.elapsed().as_millis() >= 80 {
                self.run_folder_search();
                self.folder_search_pending_at = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(80));
            }
        }

        // Debounced name/extension filter (150 ms after last keystroke)
        if let Some(ts) = self.filter_pending_at {
            if ts.elapsed().as_millis() >= 150 {
                self.filter.text = self.text_draft.clone();
                self.filter.extensions = self
                    .ext_draft
                    .split(|c: char| c == ',' || c.is_whitespace())
                    .map(|s| s.trim().trim_start_matches('.').to_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect();
                self.filter_pending_at = None;
                self.recompute_view();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(150));
            }
        }

        // Lazy-start the filesystem watcher once we have an index.
        #[cfg(windows)]
        if self.watcher.is_none() && !self.folder_index.is_empty() {
            self.start_watcher();
        }

        // Lazy-start the background clipboard-key poller (needs the egui ctx
        // so it can wake the UI on detection).
        #[cfg(windows)]
        if self.clip_key_rx.is_none() {
            self.start_clip_key_poller(ctx);
        }

        // Auto-clear transient notice
        if let Some((_, ts)) = &self.notice {
            if ts.elapsed().as_secs() >= 6 {
                self.notice = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(500));
            }
        }

        // ─── Global keyboard shortcuts ─────────────────────────────────
        // `wants_keyboard_input` = a text field has focus; table shortcuts
        // and type-to-jump must not fire then.
        let typing = ctx.wants_keyboard_input();
        let renaming = self.rename_open.is_some();
        let mut acts: Vec<KbdAct> = Vec::new();
        let mut jump_text = String::new();
        // Clipboard ops are driven by egui's semantic Copy/Cut/Paste events
        // (and key-combos as a fallback) — see the event scan below.
        let mut do_copy = false;
        let mut do_cut = false;
        let mut do_paste = false;

        ctx.input_mut(|i| {
            use egui::{Key, Modifiers};

            // Tab management & global navigation (work even while typing)
            if i.consume_key(Modifiers::COMMAND, Key::T) {
                acts.push(KbdAct::NewTab);
            }
            if i.consume_key(Modifiers::COMMAND, Key::W) {
                acts.push(KbdAct::CloseTab);
            }
            if i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab) {
                acts.push(KbdAct::PrevTab);
            }
            if i.consume_key(Modifiers::CTRL, Key::Tab) {
                acts.push(KbdAct::NextTab);
            }
            if i.consume_key(Modifiers::COMMAND, Key::L) {
                acts.push(KbdAct::PathEdit);
            }
            if i.consume_key(Modifiers::NONE, Key::F5) {
                acts.push(KbdAct::Rescan);
            }
            if i.consume_key(Modifiers::ALT, Key::ArrowLeft) {
                acts.push(KbdAct::Back);
            }
            if i.consume_key(Modifiers::ALT, Key::ArrowRight) {
                acts.push(KbdAct::Forward);
            }
            if i.consume_key(Modifiers::ALT, Key::ArrowUp) {
                acts.push(KbdAct::Up);
            }
            // Focus jumps + help work even while a field is focused.
            if i.consume_key(Modifiers::NONE, Key::F1) {
                acts.push(KbdAct::ToggleHelp);
            }
            if i.consume_key(Modifiers::NONE, Key::F6) {
                acts.push(KbdAct::ToggleSplit);
            }
            if i.consume_key(Modifiers::COMMAND, Key::F) {
                acts.push(KbdAct::FocusSearch);
            }
            if i.consume_key(Modifiers::NONE, Key::F3) {
                acts.push(KbdAct::FocusFilter);
            }

            if !typing && !renaming {
                if i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::N) {
                    acts.push(KbdAct::NewFolder);
                }
                let copy_paths = i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::C);
                if copy_paths {
                    acts.push(KbdAct::CopyPathsText);
                }
                if i.consume_key(Modifiers::COMMAND, Key::A) {
                    acts.push(KbdAct::SelectAll);
                }
                // Ctrl+C / Ctrl+X / Ctrl+V do NOT arrive as Key events — the
                // winit backend turns them into semantic Copy/Cut/Paste events
                // (so text widgets work). consume_key on V/C/X therefore never
                // matches; we read the semantic events instead. The key-combo
                // checks below are kept only as a belt-and-braces fallback for
                // backends that DO emit them.
                for ev in &i.events {
                    match ev {
                        egui::Event::Copy => do_copy = true,
                        egui::Event::Cut => do_cut = true,
                        egui::Event::Paste(_) => do_paste = true,
                        _ => {}
                    }
                }
                if i.consume_key(Modifiers::COMMAND, Key::C) {
                    do_copy = true;
                }
                if i.consume_key(Modifiers::COMMAND, Key::X) {
                    do_cut = true;
                }
                if i.consume_key(Modifiers::COMMAND, Key::V) {
                    do_paste = true;
                }
                // Ctrl+Shift+C means "copy paths as text" — don't also fire the
                // file copy from the Event::Copy the backend emits for it.
                if copy_paths {
                    do_copy = false;
                }
                if i.consume_key(Modifiers::COMMAND, Key::R) {
                    acts.push(KbdAct::ToggleRecursive);
                }
                if i.consume_key(Modifiers::SHIFT, Key::Delete) {
                    acts.push(KbdAct::PermanentDelete);
                }
                if i.consume_key(Modifiers::NONE, Key::Delete) {
                    acts.push(KbdAct::TrashSel);
                }
                if i.consume_key(Modifiers::NONE, Key::Escape) {
                    acts.push(KbdAct::ClearSel);
                }
                if i.consume_key(Modifiers::NONE, Key::F2) {
                    acts.push(KbdAct::RenameSel);
                }
                if i.consume_key(Modifiers::ALT, Key::Enter) {
                    acts.push(KbdAct::Properties);
                }
                if i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::E) {
                    acts.push(KbdAct::RevealInExplorer);
                }
                if i.consume_key(Modifiers::COMMAND, Key::I) {
                    acts.push(KbdAct::InvertSelection);
                }
                if i.consume_key(Modifiers::COMMAND, Key::B) {
                    acts.push(KbdAct::StarCurrent);
                }
                if i.consume_key(Modifiers::NONE, Key::Backspace) {
                    acts.push(KbdAct::Up);
                }
                if i.consume_key(Modifiers::NONE, Key::Enter) {
                    acts.push(KbdAct::Open);
                }
                for shift in [false, true] {
                    let m = if shift { Modifiers::SHIFT } else { Modifiers::NONE };
                    if i.consume_key(m, Key::ArrowDown) {
                        acts.push(KbdAct::Move(1, shift));
                    }
                    if i.consume_key(m, Key::ArrowUp) {
                        acts.push(KbdAct::Move(-1, shift));
                    }
                    if i.consume_key(m, Key::PageDown) {
                        acts.push(KbdAct::Move(15, shift));
                    }
                    if i.consume_key(m, Key::PageUp) {
                        acts.push(KbdAct::Move(-15, shift));
                    }
                    if i.consume_key(m, Key::Home) {
                        acts.push(KbdAct::MoveToEnd(false, shift));
                    }
                    if i.consume_key(m, Key::End) {
                        acts.push(KbdAct::MoveToEnd(true, shift));
                    }
                }
                // Type-to-jump: collect plain text events
                for ev in &i.events {
                    if let egui::Event::Text(t) = ev {
                        jump_text.push_str(t);
                    }
                }
            }
        });

        for act in acts {
            match act {
                KbdAct::SelectAll => self.select_all(),
                KbdAct::CopyPathsText => self.copy_paths_to_clipboard(ctx),
                KbdAct::TrashSel => self.trash_selected(),
                KbdAct::ClearSel => self.selection.clear(),
                KbdAct::Rescan => self.rescan(),
                KbdAct::Back => self.navigate_back(),
                KbdAct::Forward => self.navigate_forward(),
                KbdAct::Up => self.navigate_up(),
                KbdAct::ToggleRecursive => {
                    self.recursive = !self.recursive;
                    self.rescan();
                }
                KbdAct::NewTab => self.new_tab(),
                KbdAct::CloseTab => self.close_tab(self.active_tab),
                KbdAct::NextTab => {
                    let n = self.tabs.len();
                    if n > 1 {
                        self.switch_tab((self.active_tab + 1) % n);
                    }
                }
                KbdAct::PrevTab => {
                    let n = self.tabs.len();
                    if n > 1 {
                        self.switch_tab((self.active_tab + n - 1) % n);
                    }
                }
                KbdAct::NewFolder => self.create_new_folder(),
                KbdAct::RenameSel => self.open_rename(),
                KbdAct::PathEdit => {
                    self.path_edit_mode = true;
                    self.path_edit_focus = true;
                }
                KbdAct::Move(d, shift) => self.move_cursor(d, shift),
                KbdAct::MoveToEnd(to_end, shift) => {
                    if !self.view.is_empty() {
                        let pos = if to_end { self.view.len() - 1 } else { 0 };
                        self.move_cursor_to(pos, shift);
                    }
                }
                KbdAct::Open => self.open_selection(),
                KbdAct::Properties => self.show_properties(),
                KbdAct::PermanentDelete => self.delete_permanent(),
                KbdAct::RevealInExplorer => {
                    if let Some(p) = self.focus_path() {
                        self.open_in_explorer(&p);
                    }
                }
                KbdAct::InvertSelection => self.invert_selection(),
                KbdAct::FocusSearch => self.folder_search_focus = true,
                KbdAct::FocusFilter => {
                    self.show_filters = true;
                    self.name_filter_focus = true;
                }
                KbdAct::ToggleHelp => self.show_help = !self.show_help,
                KbdAct::ToggleSplit => self.toggle_split(),
                KbdAct::StarCurrent => self.star_current_folder(),
            }
        }
        if !jump_text.is_empty() {
            self.type_to_jump(&jump_text);
        }

        // Drain the background clipboard-key poller (Windows). This is what
        // actually makes Ctrl+V work for a file clipboard — see clip_key_rx.
        #[cfg(windows)]
        if !typing && !renaming {
            if let Some(rx) = self.clip_key_rx.as_ref() {
                while let Ok(k) = rx.try_recv() {
                    match k {
                        ClipKey::Copy => do_copy = true,
                        ClipKey::Cut => do_cut = true,
                        ClipKey::Paste => do_paste = true,
                    }
                }
            }
        }

        // File-clipboard ops, triggered by egui's semantic Copy/Cut/Paste
        // events, the OS-level key poller above, or the key-combo fallback.
        if do_copy {
            self.clipboard_copy_files(false);
        }
        if do_cut {
            self.clipboard_copy_files(true);
        }
        if do_paste {
            self.clipboard_paste_files();
        }

        // ─── Layout ────────────────────────────────────────────────────
        egui::TopBottomPanel::top("tabbar")
            .min_height(26.0)
            .show(ctx, |ui| self.ui_tabbar(ui));

        egui::TopBottomPanel::top("toolbar")
            .min_height(32.0)
            .show(ctx, |ui| self.ui_toolbar(ui));

        // Collapsible filter section: the header is always present (so the
        // panel can be re-opened from there), the body folds away.
        egui::TopBottomPanel::top("filterbar").show(ctx, |ui| {
            let active = self.filter_is_active();
            let title = if active {
                RichText::new("🔍 Filter & Suche  ●").strong().color(Color32::from_rgb(255, 190, 90))
            } else {
                RichText::new("🔍 Filter & Suche").strong()
            };
            let header = egui::CollapsingHeader::new(title)
                .id_salt("filter_collapse")
                .open(Some(self.show_filters))
                .show(ui, |ui| self.ui_filterbar(ui));
            if header.header_response.clicked() {
                self.show_filters = !self.show_filters;
                self.save_ui_state();
            }
        });

        egui::TopBottomPanel::bottom("status")
            .min_height(22.0)
            .show(ctx, |ui| self.ui_status(ui));

        egui::SidePanel::left("sidebar")
            .resizable(true)
            .default_width(190.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.ui_sidebar(ui));
            });

        if self.show_summary {
            egui::SidePanel::right("summary")
                .resizable(true)
                .default_width(280.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.ui_summary(ui));
                });
        }

        self.ui_central(ctx);

        if self.copy_open {
            self.ui_copy_dialog(ctx);
        }
        if self.show_errors_dialog {
            self.ui_errors_dialog(ctx);
        }
        if self.rename_open.is_some() {
            self.ui_rename_dialog(ctx);
        }
        if self.show_help {
            self.ui_help_dialog(ctx);
        }
        if self.update_ready.is_some() {
            self.ui_update_dialog(ctx);
        }
        if self.show_connect {
            self.ui_connect_dialog(ctx);
        }
        self.ui_bisync_conflicts(ctx);
        if self.show_sync_jobs {
            self.ui_sync_jobs(ctx);
        }
        if self.job_editor.is_some() {
            self.ui_job_editor(ctx);
        }
        if self.picker.is_some() {
            self.ui_picker(ctx);
        }
        if self.show_share {
            self.ui_share(ctx);
        }
        if self.remote_ctx.is_some() {
            self.ui_remote_ctx(ctx);
        }
        // Liability notice on top of everything, on first run.
        self.ui_disclaimer(ctx);

        // Drag-over hint while the OS is dragging files onto the window.
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            self.ui_drop_overlay(ctx);
            ctx.request_repaint();
        }

        // Internal file drag (between tabs/panes; out to Explorer on Windows).
        self.handle_file_drag(ctx);

        // Trackpad scrolling: egui spreads each scroll delta over several frames
        // (exponential smoothing) but does NOT request those frames itself, so a
        // reactive app only repaints on the discrete OS events → the glide
        // stalls and stutters. Keep painting at full rate during scrolling and
        // for a short tail afterwards, so the smoothing runs to a clean stop.
        if ctx.input(|i| i.raw_scroll_delta != egui::Vec2::ZERO || i.smooth_scroll_delta != egui::Vec2::ZERO) {
            self.last_scroll_at = Some(std::time::Instant::now());
        }
        if let Some(t) = self.last_scroll_at {
            if t.elapsed() < std::time::Duration::from_millis(900) {
                ctx.request_repaint();
            } else {
                self.last_scroll_at = None;
            }
        }

        // Repaint while background work is active
        if self.scan_running
            || self.tabs.iter().any(|t| t.scan_running)
            || matches!(&self.copy_progress, Some(p) if !p.done)
            || self.index_building
            || self.band_active
            || !self.file_open_rx.is_empty()
            || self.upload_rx.is_some()
            || self.remote_op_rx.is_some()
            || self.clip_download_rx.is_some()
            || self.job_connect_rx.is_some()
            || self.cloud_authing
            || self.share_progress.is_some()
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        } else if self.share.is_some() || !self.remote_edits.is_empty() || self.quickshare.is_some() {
            // Poll for incoming share offers / roster changes at a calm cadence.
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }
}

/// Drain up to 64 messages from a scan channel into the given state slices.
/// Returns (got_entries, got_done). Shared between the active tab and
/// background tabs.
fn drain_scan_channel(
    rx: &Receiver<ScanMessage>,
    entries: &mut Vec<FileEntry>,
    progress: &mut ScanProgress,
    failed_paths: &mut Vec<(String, String)>,
    error_msg: &mut Option<String>,
) -> (bool, bool) {
    let mut new_entries: Vec<FileEntry> = Vec::new();
    let mut got_done = false;
    for _ in 0..64 {
        match rx.try_recv() {
            Ok(ScanMessage::Entries(mut chunk)) => new_entries.append(&mut chunk),
            Ok(ScanMessage::Progress(p)) => *progress = p,
            Ok(ScanMessage::Error(e)) => *error_msg = Some(e),
            Ok(ScanMessage::FailedPaths(mut paths)) => {
                let remaining = 500usize.saturating_sub(failed_paths.len());
                if remaining < paths.len() {
                    paths.truncate(remaining);
                }
                failed_paths.append(&mut paths);
            }
            Ok(ScanMessage::Done(p)) => {
                *progress = p;
                got_done = true;
                break;
            }
            Err(_) => break,
        }
    }
    let got_entries = !new_entries.is_empty();
    if got_entries {
        entries.extend(new_entries);
    }
    (got_entries, got_done)
}

/// Single-layout text painting with ellipsis truncation. The previous
/// implementation re-laid-out the string once per removed character —
/// O(len²) galley builds per overflowing cell per frame.
fn paint_cell_text(
    ui: &egui::Ui,
    rect: egui::Rect,
    content: &str,
    right_align: bool,
    color: Color32,
    indent: f32,
) {
    if content.is_empty() {
        return;
    }
    use egui::text::{LayoutJob, TextWrapping};
    let font_id = egui::TextStyle::Body.resolve(ui.style());
    let max_w = (rect.width() - 10.0 - indent).max(8.0);
    let mut job = LayoutJob::simple_singleline(content.to_string(), font_id, color);
    job.wrap = TextWrapping::truncate_at_width(max_w);
    let galley = ui.fonts(|f| f.layout_job(job));
    let size = galley.size();
    let pos = if right_align {
        egui::pos2(rect.right() - 6.0 - size.x, rect.center().y - size.y * 0.5)
    } else {
        egui::pos2(rect.left() + 4.0 + indent, rect.center().y - size.y * 0.5)
    };
    ui.painter().galley(pos, galley, color);
}

fn date_to_ms_start(d: chrono::NaiveDate) -> i64 {
    use chrono::TimeZone;
    let dt = match d.and_hms_opt(0, 0, 0) {
        Some(t) => t,
        None => return 0,
    };
    chrono::Local
        .from_local_datetime(&dt)
        .single()
        .or_else(|| chrono::Local.from_local_datetime(&dt).earliest())
        .map(|t| t.timestamp_millis())
        .unwrap_or(0)
}

fn date_to_ms_end(d: chrono::NaiveDate) -> i64 {
    date_to_ms_start(d) + 24 * 3600 * 1000 - 1
}

/// Native Yes/No confirmation via MessageBoxW. Deliberately NOT rfd's
/// MessageDialog, which uses comctl32 v6 TaskDialogIndirect — that import is
/// unresolved without an embedded v6 manifest and crashes the process at load
/// (STATUS_ENTRYPOINT_NOT_FOUND). MessageBoxW is in user32 on every Windows.
#[cfg(windows)]
fn confirm_yes_no(title: &str, msg: &str) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDYES, MB_ICONWARNING, MB_YESNO,
    };
    let t: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    let m: Vec<u16> = msg.encode_utf16().chain(Some(0)).collect();
    let r = unsafe {
        MessageBoxW(
            None,
            PCWSTR(m.as_ptr()),
            PCWSTR(t.as_ptr()),
            MB_YESNO | MB_ICONWARNING,
        )
    };
    r == IDYES
}

#[cfg(not(windows))]
fn confirm_yes_no(_title: &str, _msg: &str) -> bool {
    true
}

/// True if our process owns the current foreground window. Used to gate the
/// global clipboard-key poll so Ctrl+V in another app never pastes into ours.
#[cfg(windows)]
fn app_is_foreground() -> bool {
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId,
    };
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return false;
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        pid != 0 && pid == GetCurrentProcessId()
    }
}

fn dirs_home() -> PathBuf {
    if let Some(h) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(h);
    }
    if let Some(h) = std::env::var_os("HOME") {
        return PathBuf::from(h);
    }
    PathBuf::from(".")
}

#[cfg(windows)]
fn list_drives() -> Vec<String> {
    use windows_sys::Win32::Storage::FileSystem::GetLogicalDrives;
    let bits = unsafe { GetLogicalDrives() };
    (0u32..26)
        .filter(|i| bits & (1 << i) != 0)
        .map(|i| format!("{}:\\", char::from(b'A' + i as u8)))
        .collect()
}

#[cfg(not(windows))]
fn list_drives() -> Vec<String> {
    vec!["/".to_string()]
}

#[cfg(windows)]
fn drive_info_list(drives: &[String]) -> Vec<(String, u64, u64)> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    drives
        .iter()
        .map(|d| {
            let wide: Vec<u16> = d.encode_utf16().chain(Some(0)).collect();
            let mut free = 0u64;
            let mut total = 0u64;
            let mut total_free = 0u64;
            unsafe {
                GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, &mut total_free);
            }
            (d.clone(), free, total)
        })
        .collect()
}

#[cfg(not(windows))]
fn drive_info_list(_drives: &[String]) -> Vec<(String, u64, u64)> {
    Vec::new()
}

fn settings_path() -> PathBuf {
    let dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs_home().join(".config"));
    let app = dir.join("smart_explorer");
    let _ = std::fs::create_dir_all(&app);
    app.join("recent.txt")
}

fn folder_index_path() -> PathBuf {
    let dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs_home().join(".config"));
    let app = dir.join("smart_explorer");
    let _ = std::fs::create_dir_all(&app);
    app.join("folder_index.txt")
}

fn load_folder_index_or_empty() -> FolderIndex {
    FolderIndex::load(&folder_index_path()).unwrap_or_else(|_| FolderIndex::new())
}

fn appdata_file(name: &str) -> PathBuf {
    let dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs_home().join(".config"));
    let app = dir.join("smart_explorer");
    let _ = std::fs::create_dir_all(&app);
    app.join(name)
}

fn favorites_path() -> PathBuf {
    appdata_file("favorites.txt")
}

/// Small persisted UI preference set (panel visibility). One `key=value` per
/// line, following the project's one-file-per-concern convention.
struct UiState {
    show_filters: bool,
    show_summary: bool,
}

impl UiState {
    fn load() -> Self {
        let mut s = UiState {
            show_filters: true,
            show_summary: false,
        };
        if let Ok(txt) = std::fs::read_to_string(appdata_file("ui_state.txt")) {
            for line in txt.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    let on = v.trim() == "1" || v.trim().eq_ignore_ascii_case("true");
                    match k.trim() {
                        "show_filters" => s.show_filters = on,
                        "show_summary" => s.show_summary = on,
                        _ => {}
                    }
                }
            }
        }
        s
    }

    fn save(&self) {
        let txt = format!(
            "show_filters={}\nshow_summary={}\n",
            self.show_filters as u8, self.show_summary as u8
        );
        let _ = std::fs::write(appdata_file("ui_state.txt"), txt);
    }
}
