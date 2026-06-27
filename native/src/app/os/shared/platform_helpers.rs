use super::super::prelude::*;

#[derive(Clone, Debug)]
pub(in crate::app) struct ClipboardVirtualFile {
    pub(in crate::app) abs: String,
    pub(in crate::app) rel: String,
    pub(in crate::app) size: u64,
    pub(in crate::app) mtime_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum ClipboardEffect {
    Copy,
    Move,
}

pub(in crate::app) fn drain_scan_channel(
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
/// implementation re-laid-out the string once per removed character, which was
/// quadratic in overflowing table cells.
pub(in crate::app) fn paint_cell_text(
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

pub(in crate::app) fn date_to_ms_start(d: chrono::NaiveDate) -> i64 {
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

pub(in crate::app) fn date_to_ms_end(d: chrono::NaiveDate) -> i64 {
    date_to_ms_start(d) + 24 * 3600 * 1000 - 1
}

pub(in crate::app) fn dirs_home() -> PathBuf {
    if let Some(h) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(h);
    }
    if let Some(h) = std::env::var_os("HOME") {
        return PathBuf::from(h);
    }
    PathBuf::from(".")
}

pub(in crate::app) fn settings_path() -> PathBuf {
    crate::support_dirs::app_data_file("recent.txt")
}

pub(in crate::app) fn folder_index_path() -> PathBuf {
    crate::support_dirs::app_data_file("folder_index.txt")
}

pub(in crate::app) fn load_folder_index_or_empty() -> FolderIndex {
    FolderIndex::load(&folder_index_path()).unwrap_or_else(|_| FolderIndex::new())
}

pub(in crate::app) fn appdata_file(name: &str) -> PathBuf {
    crate::support_dirs::app_data_file(name)
}

pub(in crate::app) fn favorites_path() -> PathBuf {
    appdata_file("favorites.txt")
}

/// Small persisted UI preference set (panel visibility). One `key=value` per
/// line, following the project's one-file-per-concern convention.
pub(in crate::app) struct UiState {
    pub(in crate::app) show_filters: bool,
    pub(in crate::app) show_summary: bool,
}

impl UiState {
    pub(in crate::app) fn load() -> Self {
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

    pub(in crate::app) fn save(&self) {
        let txt = format!(
            "show_filters={}\nshow_summary={}\n",
            self.show_filters as u8, self.show_summary as u8
        );
        let _ = std::fs::write(appdata_file("ui_state.txt"), txt);
    }
}

/// Default "directories first" when a location has no saved preference.
pub(in crate::app) const DEFAULT_DIRS_FIRST: bool = true;

/// Load the per-location `dirs_first` overrides (`path\t0|1` per line).
pub(in crate::app) fn load_dir_sort() -> std::collections::HashMap<String, bool> {
    let mut m = std::collections::HashMap::new();
    if let Ok(txt) = std::fs::read_to_string(appdata_file("dir_sort.tsv")) {
        for line in txt.lines() {
            if let Some((path, v)) = line.rsplit_once('\t') {
                if !path.is_empty() {
                    m.insert(path.to_string(), v.trim() == "1");
                }
            }
        }
    }
    m
}

pub(in crate::app) fn save_dir_sort(map: &std::collections::HashMap<String, bool>) {
    let mut lines: Vec<String> = map
        .iter()
        .map(|(p, v)| format!("{}\t{}", p, *v as u8))
        .collect();
    lines.sort();
    let _ = std::fs::write(appdata_file("dir_sort.tsv"), lines.join("\n"));
}
