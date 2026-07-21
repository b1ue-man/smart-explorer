use super::prelude::*;
use super::*;

impl App {
    pub fn new(just_updated: bool, initial_path: Option<PathBuf>) -> Self {
        // Clean up dead-session temp copies and mark this live session.
        let recoverable_temp_sessions = init_temp_session();
        let mut startup_daemon_error = None;
        let post_update_startup_pending =
            just_updated && crate::updater::update_startup_ack_pending();
        // Background sync is opt-in. Share may start a session worker later,
        // without registering that worker for logon or enabling scheduled jobs.
        if crate::autostart::is_enabled() && !post_update_startup_pending {
            if just_updated {
                // Hand off to a fresh daemon running the new executable.
                match crate::daemon::request_daemon_replacement() {
                    Ok(()) => {}
                    Err(error) => {
                        startup_daemon_error = Some(format!(
                            "Hintergrunddienst konnte beim Update nicht sicher neu gestartet werden: {error}"
                        ));
                    }
                }
            } else if !crate::daemon::is_running() {
                match crate::daemon::request_daemon_replacement() {
                    Ok(()) => {}
                    Err(error) => {
                        startup_daemon_error = Some(format!(
                            "Hintergrunddienst konnte nicht sicher gestartet werden: {error}"
                        ));
                    }
                }
            }
        }
        let home = dirs_home();
        let default_share_path = home.to_string_lossy().replace('\\', "/");
        let default_share_device_name = default_device_name();
        let (share_identity, share_identity_error) =
            match crate::share::ShareIdentity::load_or_create(default_share_device_name.clone()) {
                Ok(identity) => (Some(identity), None),
                Err(error) => (None, Some(error)),
            };
        let share_device_draft = share_identity
            .as_ref()
            .map(|identity| identity.device_name.clone())
            .unwrap_or(default_share_device_name);
        let (mut share_profiles, mut share_profiles_error) =
            match crate::share::ShareProfiles::load_checked(Some(default_share_path.clone())) {
                Ok(profiles) => (profiles, None),
                Err(error) => (crate::share::ShareProfiles::default(), Some(error)),
            };
        if let Some(identity) = &share_identity {
            if !share_profiles.legacy_direct_requests.is_empty() {
                match crate::share::refresh_legacy_request_expiry(
                    Some(default_share_path.clone()),
                    identity,
                ) {
                    Ok(refreshed) => share_profiles = refreshed,
                    Err(error) => {
                        share_profiles = crate::share::ShareProfiles::default();
                        share_profiles_error = Some(format!(
                            "Persistierte Legacy-Anfragen konnten nicht authentifiziert werden: {error}"
                        ));
                    }
                }
            }
        }
        let room_draft_code = match crate::share::ShareProfiles::new_room_code() {
            Ok(code) => code,
            Err(error) => {
                share_profiles_error = Some(match share_profiles_error {
                    Some(existing) => format!("{existing}; Raumcode erzeugen: {error}"),
                    None => format!("Raumcode erzeugen: {error}"),
                });
                String::new()
            }
        };
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
        let recovery_notice = (recoverable_temp_sessions > 0).then(|| {
            format!(
                "⚠ {} wiederherstellbare Sitzung(en) mit möglicherweise ungespeicherten Remote-Dateien gefunden — Verwaltung in den Einstellungen; Ordner: {}",
                recoverable_temp_sessions,
                temp_root().display()
            )
        });
        let mut startup_update_error =
            crate::updater::take_updater_error().map(|e| format!("Update-Helfer: {e}"));
        let (staged_update, staging_load_failed) = if just_updated {
            (None, false)
        } else {
            match crate::updater::load_staged_update() {
                Ok(bundle) => (bundle, false),
                Err(error) => {
                    let detail = format!("Gestagtes Update konnte nicht geladen werden: {error}");
                    startup_update_error = Some(match startup_update_error.take() {
                        Some(existing) => format!("{existing}\n{detail}"),
                        None => detail,
                    });
                    (None, true)
                }
            }
        };
        let (sync_jobs, sync_jobs_error) = match crate::syncjobs::load() {
            Ok(jobs) => (jobs, None),
            Err(error) => (
                Vec::new(),
                Some(format!(
                    "Gespeicherte Sync-Jobs konnten nicht geladen werden: {error}"
                )),
            ),
        };
        for detail in startup_daemon_error.into_iter().chain(sync_jobs_error) {
            startup_update_error = Some(match startup_update_error.take() {
                Some(existing) => format!("{existing}\n{detail}"),
                None => detail,
            });
        }

        // A snoozed bundle remains the only candidate on the next launch.
        // Do not silently replace it with a new download before consent.
        let update_rx =
            if post_update_startup_pending || staged_update.is_some() || staging_load_failed {
                None
            } else {
                let (utx, urx) = unbounded();
                match crate::updater::check_async(utx, false) {
                    Ok(()) => Some(urx),
                    Err(error) => {
                        let detail =
                            format!("Automatische Update-Prüfung konnte nicht starten: {error}");
                        startup_update_error = Some(match startup_update_error {
                            Some(existing) => format!("{existing}\n{detail}"),
                            None => detail,
                        });
                        None
                    }
                }
            };
        let show_update_dialog = staged_update.is_some();
        let update_ready = staged_update.map(ReadyUpdate::Staged);

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
            scan_was_canceled: false,

            filter: FilterDef::new(),
            sort_key: SortKey::Path,
            sort_dir: SortDir::Asc,

            show_filters: ui_state.show_filters,
            show_summary: ui_state.show_summary,
            dirs_first: DEFAULT_DIRS_FIRST,
            dir_sort: load_dir_sort(),
            show_analytics: false,
            analytics_tree: None,
            analytics_source: None,
            analytics_state: StorageRunState::Idle,
            analytics_issues: Vec::new(),
            analytics_suppressed_issues: 0,
            analytics_focus: Vec::new(),
            analytics_scan: None,
            analytics_cells: Vec::new(),
            analytics_cells_rect: egui::Rect::ZERO,
            analytics_counts: None,
            analytics_panel: AnalyticsPanel::Treemap,
            reclaim_scan: None,
            reclaim_source: None,
            reclaim_state: StorageRunState::Idle,
            reclaim_issues: Vec::new(),
            reclaim_suppressed_issues: 0,
            reclaim_report: None,
            reclaim_selected: HashSet::new(),
            reclaim_large_min_gb: 1.0,
            reclaim_stale_days: 365,
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
            copy_active_mode: None,
            copy_refresh_after: false,

            error_msg: startup_update_error,
            notice: recovery_notice
                .or_else(|| {
                    just_updated.then(|| {
                        format!(
                            "✓ Update installiert — Version {}",
                            env!("CARGO_PKG_VERSION")
                        )
                    })
                })
                .map(|message| (message, std::time::Instant::now())),
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
            drag_filter: None,
            drag_source_tab: 0,
            drag_out_started: false,
            tab_header_rects: Vec::new(),
            pane_rects: Vec::new(),
            current_render_tab: 0,
            shown: false,
            post_update_startup_pending,
            post_update_daemon_at: None,
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
            trash_worker: None,
            trash_cancel: None,
            trash_progress: None,
            trash_origin: None,

            update_rx,
            update_ready,
            show_update_dialog,
            shutdown_prepared: false,
            remote_versions: None,
            remote_versions_rx: None,
            update_release_available: None,
            update_release_notified: false,
            rollback_rx: None,
            update_feed_draft: crate::updater::update_source_str().unwrap_or_default(),
            pending_initial_path: initial_path,
            integration_ctx_menu: Self::initial_context_menu_enabled(),

            clip_prepare_rx: None,
            virtual_clip: None,

            watcher: None,
            watcher_rx: None,
            index_dirty: false,
            index_last_saved: std::time::Instant::now(),
            index_save_rx: None,
            index_save_worker: None,

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
            mount_ui: MountUiState::default(),

            bisync_rx: None,
            bisync_running: false,
            bisync_ctx: None,
            bisync_conflicts: Vec::new(),
            show_bisync_conflicts: false,
            conflict_bulk: None,
            conflict_resolution: None,
            conflict_baseline_dirty: false,
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

            sync_jobs,
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
            transfer_cancel: None,
            transfer_worker: None,
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
            share_device_draft,
            share_identity,
            share_identity_error,
            share_profiles,
            share_profiles_error,
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
            share_diag_log: String::new(),
            share_manual_stop: false,
            share_poll_rx: None,
            share_next_poll_at: Instant::now(),
            share_last_op_log_at: Instant::now() - std::time::Duration::from_secs(60),
            share_open_rx: None,
            share_opening: None,
            share_opening_origin: None,
            share_status: String::new(),
            share_worker_running: false,
            share_worker_relay_url: String::new(),
            share_worker_candidates: Vec::new(),
            quickshare: None,
            qs_devices: Vec::new(),
            quickshare_error: None,
        }
    }
}
