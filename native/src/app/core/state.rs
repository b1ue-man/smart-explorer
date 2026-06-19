use super::prelude::*;
use super::*;

pub struct App {
    pub(in crate::app) root_path: String,
    pub(in crate::app) scan_running: bool,
    pub(in crate::app) entries: Vec<FileEntry>,
    /// Visible rows: (entry index, display depth from current root).
    pub(in crate::app) view: Vec<(usize, u32)>,
    pub(in crate::app) selection: HashSet<Arc<str>>,
    pub(in crate::app) last_anchor: Option<Arc<str>>,
    /// Keyboard cursor (focused row), moved by arrow keys.
    pub(in crate::app) cursor: Option<Arc<str>>,
    pub(in crate::app) scan_rx: Option<Receiver<ScanMessage>>,
    pub(in crate::app) scan_handle: Option<ScanHandle>,
    pub(in crate::app) progress: ScanProgress,

    pub(in crate::app) filter: FilterDef,
    pub(in crate::app) sort_key: SortKey,
    pub(in crate::app) sort_dir: SortDir,

    pub(in crate::app) show_filters: bool,
    pub(in crate::app) show_summary: bool,
    /// Effective "directories first" sort for the CURRENT location (classic when
    /// true; false = files+folders mixed by the active key). Set on navigation
    /// from `dir_sort`; the toggle writes back the per-location override.
    pub(in crate::app) dirs_first: bool,
    /// Per-location overrides for `dirs_first` (path → bool), persisted. Folders
    /// not in here use the default (`DEFAULT_DIRS_FIRST`).
    pub(in crate::app) dir_sort: std::collections::HashMap<String, bool>,
    /// Storage-analytics overlay (treemap + breakdowns) is open.
    pub(in crate::app) show_analytics: bool,
    /// Dedicated low-memory size tree for analytics (own scan, not the view).
    pub(in crate::app) analytics_tree: Option<crate::analytics::SizeNode>,
    /// Path the tree was scanned for (`/`-normalised, no trailing slash).
    pub(in crate::app) analytics_root_path: String,
    /// In-memory drill position within the tree (segment names from the root).
    pub(in crate::app) analytics_focus: Vec<String>,
    /// A running background analytics scan, if any.
    pub(in crate::app) analytics_scan: Option<AnalyticsScan>,
    /// Backend the current analytics tree was scanned with (None = local fs).
    /// Set when analysing a remote, so rescans re-walk the same source.
    pub(in crate::app) analytics_backend: Option<crate::vfs::BackendHandle>,
    /// Cached nested-treemap cells for the current focus + the rect they were
    /// laid out for (recomputed on drill or resize).
    pub(in crate::app) analytics_cells: Vec<TmCell>,
    pub(in crate::app) analytics_cells_rect: egui::Rect,
    /// (files, dirs) under the current focus, cached.
    pub(in crate::app) analytics_counts: Option<(u64, u64)>,

    pub(in crate::app) recursive: bool,
    pub(in crate::app) history: Vec<String>,
    pub(in crate::app) forward: Vec<String>,

    // ─── Tabs ───────────────────────────────────────────────────────────
    pub(in crate::app) tabs: Vec<TabState>,
    pub(in crate::app) active_tab: usize,
    /// Split-screen: show two tabs side by side. `panes` are the tab indices
    /// in the left/right slots; the focused one equals `active_tab`.
    pub(in crate::app) split: bool,
    pub(in crate::app) panes: [usize; 2],
    /// Which split slot (0 = left, 1 = right) currently has focus. Selecting a
    /// tab in the top bar applies it to THIS pane (not always the left one).
    pub(in crate::app) focused_pane: usize,

    // dialog state
    pub(in crate::app) copy_open: bool,
    pub(in crate::app) copy_mode_pending: CopyMode,
    pub(in crate::app) copy_dest: String,
    pub(in crate::app) copy_preserve: bool,
    pub(in crate::app) copy_conflict: Conflict,
    pub(in crate::app) copy_rx: Option<Receiver<CopyMsg>>,
    pub(in crate::app) copy_handle: Option<CopyHandle>,
    pub(in crate::app) copy_progress: Option<CopyProgress>,
    pub(in crate::app) copy_errors: Vec<(String, String)>,
    /// Refresh the current directory when the running copy job finishes
    /// (set for paste operations into the current folder).
    pub(in crate::app) copy_refresh_after: bool,

    pub(in crate::app) error_msg: Option<String>,
    pub(in crate::app) notice: Option<(String, std::time::Instant)>,
    pub(in crate::app) failed_paths: Vec<(String, String)>,
    pub(in crate::app) app_errors: Vec<AppErrorEntry>,
    pub(in crate::app) last_logged_error: Option<String>,
    pub(in crate::app) show_errors_dialog: bool,

    // Filter input drafts
    pub(in crate::app) text_draft: String,
    pub(in crate::app) ext_draft: String,
    pub(in crate::app) size_min_draft: String,
    pub(in crate::app) size_max_draft: String,
    /// Debounce: text/ext filter commits this long after the last keystroke.
    pub(in crate::app) filter_pending_at: Option<Instant>,

    // Date filters (calendar pickers)
    pub(in crate::app) mtime_min_date: Option<chrono::NaiveDate>,
    pub(in crate::app) mtime_max_date: Option<chrono::NaiveDate>,
    pub(in crate::app) btime_min_date: Option<chrono::NaiveDate>,
    pub(in crate::app) btime_max_date: Option<chrono::NaiveDate>,

    pub(in crate::app) drives: Vec<String>,
    pub(in crate::app) drive_info: Vec<(String, u64, u64)>, // (root, free, total)
    pub(in crate::app) home: PathBuf,
    pub(in crate::app) recent: Vec<String>,
    /// Starred folders, persisted to favorites.txt. Saved on every mutation.
    pub(in crate::app) favorites: Vec<String>,

    /// Native file-type icon cache (extension-keyed, off-thread extraction).
    pub(in crate::app) icon_cache: crate::icons::IconCache,
    /// Whether the keyboard-shortcut cheat sheet overlay is open.
    pub(in crate::app) show_help: bool,
    /// First-run disclaimer / liability notice (shown until acknowledged).
    pub(in crate::app) show_disclaimer: bool,

    pub(in crate::app) last_view_recompute: Instant,
    /// Entries arrived but the view hasn't been rebuilt yet (throttled during
    /// scans so a 1M-entry stream doesn't trigger a full sort per frame).
    pub(in crate::app) view_dirty: bool,

    // Rubber-band selection
    pub(in crate::app) band_press: Option<(f32, f32)>, // (screen x, screen y) at press
    pub(in crate::app) band_active: bool,
    pub(in crate::app) band_base: HashSet<Arc<str>>,
    /// Set while rendering the NON-focused split pane so its `ui_table` ignores
    /// the rubber-band gesture (which belongs to the focused pane only) —
    /// otherwise one drag-box would select in both panes.
    pub(in crate::app) band_suppressed: bool,
    /// Last time a scroll input arrived — drives a short full-rate repaint tail
    /// so trackpad scrolling glides to a smooth stop (egui smooths the delta
    /// over frames but doesn't request those frames itself).
    pub(in crate::app) last_scroll_at: Option<Instant>,

    // ─── File drag (between tabs/panes; out to Explorer on Windows) ──────
    /// Absolute forward-slash source paths being dragged (empty = no drag).
    pub(in crate::app) drag_files: Vec<String>,
    pub(in crate::app) drag_active: bool,
    /// Backend the drag started from when the source view is remote (None =
    /// local). Lets a drop download/upload/cross-copy as needed.
    pub(in crate::app) drag_src: Option<crate::vfs::BackendHandle>,
    /// Tab the drag started from (drop onto the same tab is a no-op).
    pub(in crate::app) drag_source_tab: usize,
    /// Once we've handed an active drag to the OS (drag-out), don't re-trigger.
    pub(in crate::app) drag_out_started: bool,
    /// Per-frame: rect of each tab's header label, for drop routing.
    pub(in crate::app) tab_header_rects: Vec<(usize, egui::Rect)>,
    /// Per-frame: (tab index, rect) of each split pane, for drop routing.
    pub(in crate::app) pane_rects: Vec<(usize, egui::Rect)>,
    /// Tab index whose `ui_table` is rendering right now (focused tab, or the
    /// parked pane during its swapped render) — so a drag knows its source.
    pub(in crate::app) current_render_tab: usize,
    /// False until we've revealed the window (maximized) after the first paint.
    pub(in crate::app) shown: bool,

    pub(in crate::app) pending_scroll_row: Option<usize>,

    // Type-to-jump
    pub(in crate::app) type_jump: String,
    pub(in crate::app) type_jump_at: Instant,

    // Rename dialog: (path fwd-slashes, draft name)
    pub(in crate::app) rename_open: Option<(String, String)>,
    pub(in crate::app) rename_focus: bool,

    // Breadcrumb / path edit
    pub(in crate::app) path_edit_mode: bool,
    pub(in crate::app) path_edit_focus: bool,
    /// Request focus on the folder-search / name-filter fields next frame.
    pub(in crate::app) folder_search_focus: bool,
    pub(in crate::app) name_filter_focus: bool,
    /// Set when Enter in the name-filter pressed with >1 result moved keyboard
    /// focus into the result list. While true, opening a folder with Enter
    /// bounces focus back to the filter (cursorless drill-down). Cleared by a
    /// mouse click, a tab switch, or opening a file.
    pub(in crate::app) search_nav_from_filter: bool,
    /// Per-frame: the name-filter TextEdit just received Enter (set in
    /// `ui_filterbar`, consumed in `update`).
    pub(in crate::app) filter_enter: bool,
    /// Highlighted row in the omnibox dropdown (None = typing, no row picked).
    pub(in crate::app) omni_sel: Option<usize>,
    /// Set when Enter in the omnibox should activate the highlighted dropdown
    /// row (carried alongside `filter_enter` so `update` can dispatch it).
    pub(in crate::app) omni_activate: Option<OmniAction>,
    /// Alt key-overlay (classic accelerator badges) is showing.
    pub(in crate::app) accel_mode: bool,
    /// Alt modifier state last frame, and whether this Alt-hold was "used" as a
    /// modifier (another key/click) — a clean tap toggles the overlay.
    pub(in crate::app) alt_prev: bool,
    pub(in crate::app) alt_dirty: bool,
    /// Controls registered for the overlay this frame: (badge char, rect, act).
    pub(in crate::app) accel_targets: Vec<(char, egui::Rect, AccelAct)>,

    pub(in crate::app) summary_cache: Option<SummaryData>,
    /// (selection len, entries len, bytes) — cheap invalidation key.
    pub(in crate::app) sel_size_cache: (usize, usize, u64),

    // ─── Folder fuzzy search ────────────────────────────────────────────
    pub(in crate::app) folder_index: FolderIndex,
    pub(in crate::app) index_building: bool,
    pub(in crate::app) index_progress: u64,
    pub(in crate::app) index_progress_path: String,
    pub(in crate::app) index_rx: Option<Receiver<IndexMsg>>,
    pub(in crate::app) index_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub(in crate::app) folder_search_query: String,
    pub(in crate::app) folder_search_results: Vec<(String, i32)>,
    pub(in crate::app) folder_search_pending_at: Option<std::time::Instant>,
    /// Background mtime-ranking of search results: (sequence, ranked).
    pub(in crate::app) folder_search_rx: Option<Receiver<(u64, Vec<(String, i32)>)>>,
    pub(in crate::app) folder_search_seq: u64,

    // Background trash result
    pub(in crate::app) trash_rx: Option<Receiver<Option<String>>>,

    // ─── Self-update ────────────────────────────────────────────────────
    pub(in crate::app) update_rx: Option<Receiver<crate::updater::UpdateMsg>>,
    /// A downloaded update is swapped in and waiting for a restart: (version,
    /// new exe path). Shows the restart-now prompt; the new binary is already
    /// on disk, so "Später" just keeps running the old code until next launch.
    pub(in crate::app) update_ready: Option<(String, PathBuf)>,
    pub(in crate::app) update_feed_draft: String,
    /// Previously-released versions from the GitHub feed (#rollback). None = not
    /// fetched yet; `Some(vec)` = fetched (possibly empty).
    pub(in crate::app) remote_versions: Option<Vec<String>>,
    pub(in crate::app) remote_versions_rx: Option<Receiver<Vec<String>>>,
    /// Rollback-to-a-released-version download in flight: Ok((version, exe)).
    pub(in crate::app) rollback_rx: Option<Receiver<Result<(String, PathBuf), String>>>,
    /// True when the in-flight `rollback_rx` download is a FORWARD update
    /// (install a newer release) rather than a rollback — picks `install_version`
    /// vs `revert_to` when it lands.
    pub(in crate::app) rollback_forward: bool,
    /// Newest released version that is strictly newer than the running one, once
    /// the release list has been fetched. Drives the "⬆ Update verfügbar" banner
    /// + a one-shot notice, so a newer release is offered automatically (no need
    /// to press "Jetzt prüfen", and independent of the main-branch feed).
    pub(in crate::app) update_release_available: Option<String>,
    /// Set once we've shown the discovery notice, so it fires only once.
    pub(in crate::app) update_release_notified: bool,

    /// A folder path passed on the command line, scanned on the first frame.
    pub(in crate::app) pending_initial_path: Option<PathBuf>,

    // ─── Shell integration (Windows; mirrors actual registry state) ─────
    pub(in crate::app) integration_ctx_menu: bool,

    // Filter-aware clipboard (virtual files)
    #[cfg(windows)]
    pub(in crate::app) clip_prepare_rx:
        Option<Receiver<Vec<crate::virtual_clipboard::VirtualFile>>>,
    #[cfg(windows)]
    pub(in crate::app) virtual_clip: Option<(u32, Vec<(String, String)>)>, // (clipboard seq, (abs, rel))

    // Filesystem watcher state
    #[cfg(windows)]
    pub(in crate::app) watcher: Option<notify::RecommendedWatcher>,
    #[cfg(windows)]
    pub(in crate::app) watcher_rx:
        Option<crossbeam_channel::Receiver<notify::Result<notify::Event>>>,
    pub(in crate::app) index_dirty: bool,
    pub(in crate::app) index_last_saved: std::time::Instant,

    /// Background clipboard-key detection. egui swallows Ctrl+C/X/V and, for a
    /// file (non-text) clipboard, emits no paste event AND triggers no frame
    /// when idle — so the keypress is invisible to update(). A dedicated thread
    /// polls the OS key state and signals over this channel, waking the UI.
    pub(in crate::app) clip_key_rx: Option<crossbeam_channel::Receiver<ClipKey>>,
    pub(in crate::app) clip_key_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,

    // ─── Remote connections (SFTP/FTP/network shares) ───────────────────
    /// Active SFTP/FTP session; while set, navigation walks the backend via
    /// `rscan` instead of std::fs. `None` for local (incl. UNC shares).
    pub(in crate::app) remote: Option<crate::connect::RemoteState>,
    /// Live authenticated network-share connection, kept alive while browsing
    /// the UNC path (which is read locally through std::fs).
    pub(in crate::app) net_conn: Option<crate::net::NetConnection>,
    pub(in crate::app) show_connect: bool,
    pub(in crate::app) connecting: bool,
    pub(in crate::app) connect_form: crate::connect::ConnectForm,
    pub(in crate::app) connect_rx: Option<Receiver<crate::connect::ConnectResult>>,

    // One-way mirror of the current location to a chosen folder.
    pub(in crate::app) sync_rx: Option<Receiver<crate::sync::SyncMsg>>,
    pub(in crate::app) sync_running: bool,

    /// Cached saved-connection list (avoids reading connections.txt per frame).
    pub(in crate::app) saved_connections: Vec<crate::creds::SavedConnection>,

    // ─── Two-way sync (bisync) + conflict resolution ─────────────────────
    pub(in crate::app) bisync_rx: Option<Receiver<crate::bisync::Outcome>>,
    pub(in crate::app) bisync_running: bool,
    pub(in crate::app) bisync_ctx: Option<BisyncCtx>,
    pub(in crate::app) bisync_conflicts: Vec<crate::bisync::Conflict>,
    pub(in crate::app) show_bisync_conflicts: bool,
    /// Line-merge editor for one conflict (None = closed) + its async channels.
    pub(in crate::app) merge: Option<MergeUi>,
    pub(in crate::app) merge_load_rx:
        Option<Receiver<Result<(String, Vec<crate::linemerge::Row>), String>>>,
    pub(in crate::app) merge_apply_rx:
        Option<Receiver<Result<(String, crate::bisync::Sig, crate::bisync::Sig), String>>>,
    /// Compare ("ls-diff") view: a running preview + its result window.
    pub(in crate::app) preview_rx: Option<Receiver<crate::bisync::Preview>>,
    pub(in crate::app) preview_running: bool,
    pub(in crate::app) preview: Option<crate::bisync::Preview>,
    pub(in crate::app) preview_title: String,
    pub(in crate::app) preview_job_id: Option<String>,
    pub(in crate::app) preview_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub(in crate::app) show_preview: bool,
    /// Result channel for a single-file "sync this one" from the compare view.
    pub(in crate::app) apply_one_rx: Option<Receiver<String>>,
    /// Cancel flags so a running mirror / two-way sync can be stopped.
    pub(in crate::app) sync_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub(in crate::app) bisync_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,

    // ─── Saved sync setups (persistent jobs) ─────────────────────────────
    /// Loaded once at start, kept in sync with sync/jobs.tsv after edits/runs.
    pub(in crate::app) sync_jobs: Vec<crate::syncjobs::SyncJob>,
    pub(in crate::app) show_sync_jobs: bool,
    /// Background-daemon log viewer open?
    pub(in crate::app) show_daemon_log: bool,
    /// Open add/edit dialog (None = closed).
    pub(in crate::app) job_editor: Option<JobEditor>,
    /// Id of the job whose run is currently in flight (so its `last_run`
    /// gets stamped on completion). None = ad-hoc run, nothing to stamp.
    pub(in crate::app) running_job: Option<String>,

    // ─── In-app folder picker (local + saved remote connections) ─────────
    pub(in crate::app) picker: Option<PickerState>,
    /// Resolving a remote job's endpoints off the UI thread before a run.
    pub(in crate::app) job_connect_rx: Option<
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
    pub(in crate::app) job_connect_pending: Option<crate::syncjobs::SyncJob>,
    /// In-flight "download a remote file to temp, then open it" jobs (one per
    /// double-clicked remote file). Result is the local temp path to launch;
    /// `OpenMode` selects the default app vs. the native "Open with…" dialog.
    pub(in crate::app) file_open_rx:
        Vec<(Receiver<Result<(String, i64), String>>, OpenMode, PathBuf)>,
    /// How remote files are opened/edited (temp-watch vs CfAPI) — persisted.
    /// Temp-mode edit-watch: re-upload each temp copy to the remote on save.
    pub(in crate::app) remote_edits: Vec<RemoteEdit>,
    pub(in crate::app) edit_save_rx: Vec<Receiver<(PathBuf, SaveResult)>>,
    pub(in crate::app) last_edit_poll: Instant,
    /// In-flight upload of clipboard/dropped files into a remote folder.
    /// Result is (files uploaded, errors).
    pub(in crate::app) upload_rx: Option<Receiver<(u64, Vec<String>)>>,
    /// In-flight one-shot remote op (new folder, rename, download-to).
    /// Ok(notice)/Err(msg); the worker includes the op context in both.
    pub(in crate::app) remote_op_rx: Option<Receiver<Result<String, String>>>,
    /// In-flight runtime activation of the SSH agent on the live connection
    /// (#24): the deployed `AgentBackend` + its version, or an error message.
    pub(in crate::app) agent_activate_rx:
        Option<Receiver<Result<(crate::agent::AgentBackend, String), String>>>,
    /// The SFTP session the activation is running against (guards installing the
    /// result into the right connection if the user switched tabs meanwhile).
    pub(in crate::app) agent_activate_for: Option<Arc<crate::sftp::SftpBackend>>,
    /// Open egui context menu for a remote entry: (screen pos, entry index).
    pub(in crate::app) remote_ctx: Option<(egui::Pos2, usize)>,
    /// In-flight download of selected remote files to temp for a Ctrl+C →
    /// Explorer paste. Result is the local temp paths to put on the clipboard.
    pub(in crate::app) clip_download_rx: Option<Receiver<Vec<String>>>,

    // ─── Cloud (OAuth) — slice 1: connect Google Drive ───────────────────
    pub(in crate::app) cloud_client_id_draft: String,
    pub(in crate::app) cloud_secret_draft: String,
    pub(in crate::app) cloud_auth_rx: Option<Receiver<Result<(), String>>>,
    pub(in crate::app) cloud_authing: bool,

    // ─── Peer file sharing (#21) ─────────────────────────────────────────
    pub(in crate::app) share: Option<crate::share::ShareService>,
    pub(in crate::app) show_share: bool,
    /// Rendezvous server "host:port" (persisted) + device name + drafts.
    pub(in crate::app) share_server: String,
    pub(in crate::app) share_server_draft: String,
    pub(in crate::app) share_device_draft: String,
    /// Code typed to connect/join, and the code we generated to display.
    pub(in crate::app) share_code_input: String,
    pub(in crate::app) share_my_code: String,
    pub(in crate::app) share_room: bool,
    pub(in crate::app) share_roster: Vec<crate::share::RemoteDevice>,
    pub(in crate::app) share_incoming: Vec<(u64, String, Vec<(String, u64)>)>,
    pub(in crate::app) share_status: String,
    pub(in crate::app) share_progress: Option<(u64, u64)>,

    // Quick Share (Android) LAN discovery — started lazily when Teilen opens.
    pub(in crate::app) quickshare: Option<crate::quickshare::QuickShare>,
    pub(in crate::app) qs_devices: Vec<crate::quickshare::QsDevice>,
}
