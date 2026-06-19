use super::prelude::*;

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
/// implementation re-laid-out the string once per removed character —
/// O(len²) galley builds per overflowing cell per frame.
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

/// Native Yes/No confirmation via MessageBoxW. Deliberately NOT rfd's
/// MessageDialog, which uses comctl32 v6 TaskDialogIndirect — that import is
/// unresolved without an embedded v6 manifest and crashes the process at load
/// (STATUS_ENTRYPOINT_NOT_FOUND). MessageBoxW is in user32 on every Windows.
#[cfg(windows)]
pub(in crate::app) fn confirm_yes_no(title: &str, msg: &str) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONWARNING, MB_YESNO};
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
pub(in crate::app) fn confirm_yes_no(_title: &str, _msg: &str) -> bool {
    true
}

/// True if our process owns the current foreground window. Used to gate the
/// global clipboard-key poll so Ctrl+V in another app never pastes into ours.
#[cfg(windows)]
pub(in crate::app) fn app_is_foreground() -> bool {
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

pub(in crate::app) fn dirs_home() -> PathBuf {
    if let Some(h) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(h);
    }
    if let Some(h) = std::env::var_os("HOME") {
        return PathBuf::from(h);
    }
    PathBuf::from(".")
}

#[cfg(windows)]
pub(in crate::app) fn list_drives() -> Vec<String> {
    use windows_sys::Win32::Storage::FileSystem::GetLogicalDrives;
    let bits = unsafe { GetLogicalDrives() };
    (0u32..26)
        .filter(|i| bits & (1 << i) != 0)
        .map(|i| format!("{}:\\", char::from(b'A' + i as u8)))
        .collect()
}

#[cfg(not(windows))]
pub(in crate::app) fn list_drives() -> Vec<String> {
    vec!["/".to_string()]
}

#[cfg(windows)]
pub(in crate::app) fn drive_info_list(drives: &[String]) -> Vec<(String, u64, u64)> {
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
pub(in crate::app) fn drive_info_list(_drives: &[String]) -> Vec<(String, u64, u64)> {
    Vec::new()
}

pub(in crate::app) fn settings_path() -> PathBuf {
    let dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs_home().join(".config"));
    let app = dir.join("smart_explorer");
    let _ = std::fs::create_dir_all(&app);
    app.join("recent.txt")
}

pub(in crate::app) fn folder_index_path() -> PathBuf {
    let dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs_home().join(".config"));
    let app = dir.join("smart_explorer");
    let _ = std::fs::create_dir_all(&app);
    app.join("folder_index.txt")
}

pub(in crate::app) fn load_folder_index_or_empty() -> FolderIndex {
    FolderIndex::load(&folder_index_path()).unwrap_or_else(|_| FolderIndex::new())
}

pub(in crate::app) fn appdata_file(name: &str) -> PathBuf {
    let dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs_home().join(".config"));
    let app = dir.join("smart_explorer");
    let _ = std::fs::create_dir_all(&app);
    app.join(name)
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
