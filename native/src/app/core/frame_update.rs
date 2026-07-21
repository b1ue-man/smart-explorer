use super::prelude::*;
use super::*;

impl eframe::App for App {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.prepare_for_exit(false);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.shutdown_prepared {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        self.update_background(ctx);
        self.update_keyboard(ctx);
        self.update_layout(ctx);
        self.update_repaint(ctx);
        if self.post_update_startup_pending {
            if let Err(error) = crate::updater::acknowledge_update_startup() {
                panic!("{error}");
            }
            self.post_update_startup_pending = false;
            // Do not run abandoned-staging cleanup in the replacement process:
            // v0.5.119 can have another hash-verified worker queued behind the
            // updater lock. The next ordinary launch performs cleanup after
            // those compatibility workers have had a chance to finish.
            crate::updater::archive_current_version();
            if crate::autostart::is_enabled() {
                self.post_update_daemon_at =
                    Some(Instant::now() + std::time::Duration::from_secs(5));
                ctx.request_repaint_after(std::time::Duration::from_secs(5));
            }
        }
        if self
            .post_update_daemon_at
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.post_update_daemon_at = None;
            match crate::daemon::request_daemon_replacement() {
                Ok(()) => {}
                Err(error) => {
                    self.error_msg = Some(format!(
                        "Hintergrunddienst konnte nach dem Update nicht sicher neu gestartet werden: {error}"
                    ));
                }
            }
        }
    }
}

impl App {
    pub(in crate::app) fn update_background(&mut self, ctx: &egui::Context) {
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
        self.drain_index_save();
        self.drain_watcher();
        self.drain_folder_search();
        self.drain_trash();
        self.drain_clip_prepare();
        self.drain_update();
        self.drain_connect();
        self.drain_mount_ui(ctx);
        self.drain_sync();
        self.drain_bisync();
        self.drain_preview();
        self.poll_analytics_scan();
        self.poll_reclaim_scan();
        self.drain_apply_one();
        self.drain_merge();
        self.drain_job_connect();
        self.drain_picker_connect();
        self.drain_picker_list();
        self.drain_cloud_auth();
        self.drain_file_open();
        self.poll_remote_edits();
        self.drain_edit_saves();
        self.drain_upload();
        self.drain_remote_op();
        self.drain_agent_activate();
        // Fetch the released-versions list once, early, so a newer release is
        // discovered and offered automatically (independent of the feed check).
        self.fetch_remote_versions();
        self.drain_version_channels();
        self.drain_clip_download();
        self.drain_share();
        self.drain_quickshare();
        self.capture_current_error();
        if self.icon_cache.drain(ctx) {
            ctx.request_repaint();
        }
        self.maybe_save_index();

        // Files dropped onto the window from the OS (Explorer/desktop) → land
        // in the current folder. Processed once per frame.
        if !self.show_disclaimer {
            self.handle_os_drop(ctx);
        }

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
                self.flush_text_filter();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(150));
            }
        }

        // Lazy-start the filesystem watcher once the platform adapter can use it.
        if self.should_start_watcher() {
            self.start_watcher();
        }

        // Lazy-start the background clipboard-key poller when the platform has one.
        if self.should_start_clip_key_poller() {
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
    }

    pub(in crate::app) fn update_repaint(&mut self, ctx: &egui::Context) {
        // Repaint while background work is active
        if self.scan_running
            || self.tabs.iter().any(|t| t.scan_running)
            || matches!(&self.copy_progress, Some(p) if !p.done)
            || self.sync_running
            || self.bisync_running
            || self.preview_running
            || self.apply_one_rx.is_some()
            || self.merge_load_rx.is_some()
            || self.merge_apply_rx.is_some()
            || self.index_building
            || self.index_save_active()
            || self.band_active
            || !self.file_open_rx.is_empty()
            || self.upload_rx.is_some()
            || self.remote_op_rx.is_some()
            || self.clip_download_rx.is_some()
            || self.job_connect_rx.is_some()
            || self.connecting
            || self.agent_activate_rx.is_some()
            || self.remote_versions_rx.is_some()
            || self.rollback_rx.is_some()
            || self.trash_rx.is_some()
            || self.trash_worker.is_some()
            || self.clip_prepare_rx.is_some()
            || self.share_open_rx.is_some()
            || !self.edit_save_rx.is_empty()
            || self
                .picker
                .as_ref()
                .is_some_and(|picker| picker.connecting || picker.listing)
            || self.cloud_authing
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        } else if self.share_worker_running
            || self.share_profiles.auto_connect
            || !self.remote_edits.is_empty()
            || self.quickshare.is_some()
        {
            // Poll for incoming share offers / roster changes at a calm cadence.
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }
}
