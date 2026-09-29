//! Ctrl+C / Ctrl+X / Ctrl+V. Copying only remembers the selection (see
//! `transfer_clip`); where the platform has a file clipboard it is also
//! offered to other programs the way they expect it: local files as CF_HDROP
//! (a filtered selection as virtual files, prepared in the background) and
//! remote entries as virtual files that are listed and streamed only when
//! the other program pastes. Pasting starts the transfer at once from our own
//! entry while it is current, otherwise from the OS file clipboard.
use super::prelude::*;
use super::transfer_clip::{clip_marker_text, AppClip};
use super::transfer_selection::{os_paths_selection, SelectionIssue, ViewSelection};
use super::*;
use crate::app::shared_platform_helpers::ClipboardEffect;

const REMOTE_CUT_REFUSED: &str =
    "Remote-Ausschneiden wird nicht unterstützt. Bitte kopieren; die Quelldateien bleiben unverändert.";

impl App {
    pub(in crate::app) fn clipboard_copy_files(&mut self, cut: bool) {
        if cut && self.remote.is_some() {
            self.error_msg = Some(REMOTE_CUT_REFUSED.to_string());
            return;
        }
        let view = match self.view_selection(cut) {
            Ok(view) => view,
            Err(SelectionIssue::Empty(hint)) => {
                self.notice = Some((hint, Instant::now()));
                return;
            }
            Err(SelectionIssue::Invalid(error)) => {
                self.error_msg = Some(error);
                return;
            }
        };
        let hint = if cut && view.has_dir && self.filter_is_active() {
            " — Hinweis: Ausschneiden überträgt ganze Ordner, Filter gelten dabei nicht"
        } else {
            ""
        };
        self.cancel_clipboard_preparation();
        let clip = AppClip::new(view.selection.clone(), cut);
        let clip = if clipboard_file_ops_supported() {
            self.offer_to_other_programs(clip, view, cut)
        } else {
            clip.guarded_by_marker(clip_marker_text(&view.roots))
        };
        self.notice = Some((format!("{}{hint}", clip.copied_notice()), Instant::now()));
        self.clip = Some(clip);
    }

    /// The OS file clipboard gets what other programs understand; our own
    /// entry stays current under the sequence numbers this produces.
    fn offer_to_other_programs(
        &mut self,
        clip: AppClip,
        view: ViewSelection,
        cut: bool,
    ) -> AppClip {
        let mut clip = clip.guarded_by_sequence(virtual_clipboard_sequence());
        if let Some(backend) = clip.selection.source.backend().cloned() {
            if !remote_clipboard_supported() {
                // No virtual files here: the entry stays with the app.
                return clip;
            }
            let source = crate::transfer::SelectionSource {
                backend,
                paths: view.roots,
                filter: view.view_filter,
                label: clip.selection.source.label.clone(),
            };
            match set_remote_clipboard(source) {
                Ok(sequence) => clip.add_sequence(sequence),
                Err(error) => {
                    self.error_msg = Some(format!(
                        "Für andere Programme nicht bereitgestellt: {error} — Einfügen in Smart Explorer funktioniert"
                    ));
                }
            }
            return clip;
        }
        if !cut && (view.snapshot.is_some() || view.view_filter.is_some()) {
            self.start_filtered_preparation(view);
            return clip;
        }
        let paths: Vec<String> = view
            .roots
            .iter()
            .map(|path| path.replace('/', "\\"))
            .collect();
        let effect = if cut {
            ClipboardEffect::Move
        } else {
            ClipboardEffect::Copy
        };
        match write_clipboard_files(&paths, effect) {
            Ok(()) => {
                if let Some(sequence) = virtual_clipboard_sequence() {
                    clip.add_sequence(sequence);
                }
            }
            Err(error) => {
                self.error_msg = Some(format!(
                    "Zwischenablage für andere Programme: {error} — Einfügen in Smart Explorer funktioniert"
                ));
            }
        }
        clip
    }

    /// Menu "Einfügen" (no clipboard text involved).
    pub(in crate::app) fn clipboard_paste_files(&mut self) {
        self.clipboard_paste_with_text(None);
    }

    /// Ctrl+V; `pasted_text` is the text the keyboard paste brought, which
    /// tells on Linux whether another program copied since our copy.
    pub(in crate::app) fn clipboard_paste_with_text(&mut self, pasted_text: Option<&str>) {
        if self.root_path.is_empty() {
            self.notice = Some((
                "Strg+V: kein Zielordner geöffnet".to_string(),
                Instant::now(),
            ));
            return;
        }
        let Some((target, target_dir)) = self.tab_place(self.active_tab) else {
            return;
        };
        let own = self
            .clip
            .as_ref()
            .filter(|clip| clip.is_current(virtual_clipboard_sequence(), pasted_text))
            .map(|clip| (clip.selection.clone(), clip.cut));
        if let Some((selection, cut)) = own {
            let mode = if cut { CopyMode::Move } else { CopyMode::Copy };
            // Moved sources are gone; the entry cannot be pasted again.
            if self.submit_paste(&selection, &target, &target_dir, mode) && cut {
                self.clip = None;
            }
            return;
        }
        let replaced = self.clip.take().is_some();
        if !clipboard_file_ops_supported() {
            let hint = if replaced {
                "Die Zwischenablage wurde in einem anderen Programm geändert — bitte in Smart Explorer erneut kopieren"
            } else {
                "Strg+V: die Zwischenablage enthält keine in Smart Explorer kopierten Dateien"
            };
            self.notice = Some((hint.to_string(), Instant::now()));
            return;
        }
        let (paths, is_cut) = match read_clipboard_files() {
            Ok(Some(files)) => files,
            Ok(None) => {
                self.notice = Some((
                    "Strg+V erkannt — aber die Zwischenablage enthält keine Dateien".to_string(),
                    Instant::now(),
                ));
                return;
            }
            Err(error) => {
                self.error_msg = Some(format!(
                    "Zwischenablage konnte nicht gelesen werden: {error}"
                ));
                return;
            }
        };
        let Some(selection) = os_paths_selection(paths) else {
            self.notice = Some((
                "Strg+V erkannt — die Zwischenablage enthält keine Dateien".to_string(),
                Instant::now(),
            ));
            return;
        };
        let mode = if is_cut {
            CopyMode::Move
        } else {
            CopyMode::Copy
        };
        self.submit_paste(&selection, &target, &target_dir, mode);
    }
}
