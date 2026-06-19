use super::prelude::*;
use super::*;

impl App {
    #[cfg(windows)]
    pub(in crate::app) fn start_clip_key_poller(&mut self, ctx: &egui::Context) {
        use std::sync::atomic::{AtomicBool, Ordering};
        let (tx, rx) = crossbeam_channel::unbounded::<ClipKey>();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_t = cancel.clone();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("clip-keys".into())
            .spawn(move || {
                use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
                let down =
                    |vk: i32| -> bool { (unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000) != 0 };
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
    pub(in crate::app) fn start_clip_key_poller(&mut self, _ctx: &egui::Context) {}

    // ─── Filesystem watcher for live index updates ──────────────────────
    #[cfg(windows)]
    pub(in crate::app) fn start_watcher(&mut self) {
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
    pub(in crate::app) fn drain_watcher(&mut self) {
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
    pub(in crate::app) fn apply_batched_changes(
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
                    || remove_prefixes
                        .iter()
                        .any(|pref| p.starts_with(pref.as_str()))
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
    pub(in crate::app) fn start_watcher(&mut self) {}
    #[cfg(not(windows))]
    pub(in crate::app) fn drain_watcher(&mut self) {}
}
