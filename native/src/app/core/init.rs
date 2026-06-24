use super::prelude::*;
use super::*;

impl App {
    pub fn new(just_updated: bool, initial_path: Option<PathBuf>) -> Self {
        // Clean up dead-session temp copies and mark this live session.
        init_temp_session();
        // Keep the background worker alive across startup and self-update. It
        // owns background sync and persistent Share sessions, so it is started
        // even when no sync job is currently due.
        let _ = crate::autostart::enable();
        if just_updated {
            // Hand off to a fresh daemon running the new exe: ask the old one to
            // stop and spawn a new one (which waits for the old to exit).
            crate::daemon::request_stop();
            crate::autostart::spawn_daemon_now();
        } else if !crate::daemon::is_running() {
            crate::daemon::clear_stop();
            crate::autostart::spawn_daemon_now();
        }
        let home = dirs_home();
        let default_share_path = home.to_string_lossy().replace('\\', "/");
        let share_identity = crate::share::ShareIdentity::load_or_create(default_device_name());
        let share_profiles = crate::share::ShareProfiles::load(Some(default_share_path.clone()));
        let room_draft_code = crate::share::ShareProfiles::new_room_code();
        let drives = list_drives();
        let drive_info = drive_info_list(&drives);
        let recent: Vec<String> = std::fs::read_to_string(settings_path())
            .ok()
            .map(|s| s.lines().map(|l| l.to_string()).collect())
            .unwrap_or_default();
        let favorites: Vec<String> = std::fs::read_to_string(favorites_path())
            .ok()
            .map(|s| {
                s.lines()
                    .filter(|l| !l.is_empty())
                    .map(|l| l.to_string())
                    .collect()
            })
            .unwrap_or_default();
        let ui_state = UiState::load();
        let startup_update_error =
            crate::updater::take_updater_error().map(|e| format!("Update-Helfer: {}", e));

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
            dirs_first: DEFAULT_DIRS_FIRST,
            dir_sort: load_dir_sort(),
            show_analytics: false,
            analytics_tree: None,
            analytics_root_path: String::new(),
            analytics_focus: Vec::new(),
            analytics_scan: None,
            analytics_backend: None,
            analytics_cells: Vec::new(),
            analytics_cells_rect: egui::Rect::ZERO,
            analytics_counts: None,
            recursive: false,
            history: Vec::new(),
            forward: Vec::new(),

            tabs: vec![TabState::default()],
            active_tab: 0,
            split: false,
            panes: [0, 1],
            focused_pane: 0,

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

            error_msg: startup_update_error,
            notice: if just_updated {
                Some((
                    format!(
                        "✓ Update installiert — Version {}",
                        env!("CARGO_PKG_VERSION")
                    ),
                    std::time::Instant::now(),
                ))
            } else {
                None
            },
            failed_paths: Vec::new(),
            app_errors: Vec::new(),
            last_logged_error: None,
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
            search_nav_from_filter: false,
            filter_enter: false,
            omni_sel: None,
            omni_activate: None,
            accel_mode: false,
            alt_prev: false,
            alt_dirty: false,
            accel_targets: Vec::new(),

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
            remote_versions: None,
            remote_versions_rx: None,
            rollback_forward: false,
            update_release_available: None,
            update_release_notified: false,
            rollback_rx: None,
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
            sync_progress: None,

            saved_connections: crate::creds::load_connections(),

            bisync_rx: None,
            bisync_running: false,
            bisync_ctx: None,
            bisync_conflicts: Vec::new(),
            show_bisync_conflicts: false,
            merge: None,
            merge_load_rx: None,
            merge_apply_rx: None,
            preview_rx: None,
            preview_running: false,
            preview: None,
            preview_title: String::new(),
            preview_job_id: None,
            preview_cancel: None,
            show_preview: false,
            apply_one_rx: None,
            sync_cancel: None,
            bisync_cancel: None,

            sync_jobs: crate::syncjobs::load(),
            show_sync_jobs: false,
            show_daemon_log: false,
            job_editor: None,
            running_job: None,

            picker: None,
            job_connect_rx: None,
            job_connect_pending: None,
            file_open_rx: Vec::new(),
            remote_edits: Vec::new(),
            edit_save_rx: Vec::new(),
            last_edit_poll: Instant::now(),
            upload_rx: None,
            transfer_progress: None,
            remote_op_rx: None,
            agent_activate_rx: None,
            agent_activate_for: None,
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
            share_device_draft: share_identity.device_name.clone(),
            share_identity,
            share_profiles,
            share_tab: 0,
            share_direct_code_input: String::new(),
            share_direct_name_input: String::new(),
            share_room_code_input: String::new(),
            share_room_name_input: String::new(),
            share_room_create_name_input: "Raum".to_string(),
            share_room_draft_code: room_draft_code,
            share_export_scope: 0,
            share_export_target_id: String::new(),
            share_export_path_draft: default_share_path,
            share_export_label_draft: "Home".to_string(),
            share_block_symlink_escape: true,
            share_regenerate_direct_confirm: false,
            share_direct_requests: Vec::new(),
            share_diag_log: String::new(),
            share_manual_stop: false,
            share_open_rx: None,
            share_opening: None,
            share_status: String::new(),
            quickshare: None,
            qs_devices: Vec::new(),
        }
    }
}
