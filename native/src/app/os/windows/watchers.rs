use super::super::prelude::*;
use super::super::*;

impl App {
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
                let mut prev = [false; 3];
                while !cancel_t.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(30));
                    let fg = app_is_foreground();
                    let ctrl = down(0x11);
                    let shift = down(0x10);
                    let cur = [down(0x43), down(0x58), down(0x56)];
                    for idx in 0..3 {
                        let just_pressed = cur[idx] && !prev[idx];
                        prev[idx] = cur[idx];
                        if !(just_pressed && ctrl && fg) {
                            continue;
                        }
                        let action = match idx {
                            0 if !shift => ClipKey::Copy,
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
                    RenameMode::Both if event.paths.len() == 2 => {
                        rename_subtrees
                            .push((normalize(&event.paths[0]), normalize(&event.paths[1])));
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
}
