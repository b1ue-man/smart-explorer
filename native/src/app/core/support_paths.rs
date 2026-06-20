use super::prelude::*;
use super::*;

/// First-run liability notice (single source: the repo's DISCLAIMER.txt, also
/// used by the installer's accept page).
pub(in crate::app) const DISCLAIMER_TEXT: &str = include_str!("../../../../DISCLAIMER.txt");

/// How many saved (set-up-once) remote connections stay pinned on the sidebar.
/// The freshest are shown there; any older ones overflow into the "Verbindung"
/// menu so the sidebar can't grow without bound.
pub(in crate::app) const SIDEBAR_CONN_CAP: usize = 10;

/// Format a unix-millis timestamp as local "YYYY-MM-DD HH:MM".
pub(in crate::app) fn now_secs_i64() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub(in crate::app) fn fmt_ms(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

/// Live context for a two-way sync, kept so its conflicts can be resolved
/// against the same backends afterwards.
pub(in crate::app) struct BisyncCtx {
    pub(in crate::app) a: crate::vfs::BackendHandle,
    pub(in crate::app) root_a: String,
    pub(in crate::app) b: crate::vfs::BackendHandle,
    pub(in crate::app) root_b: String,
    pub(in crate::app) pair: String,
    pub(in crate::app) baseline: crate::bisync::Baseline,
}

/// Whether a forward-slash path is a LOCAL path (drive letter `X:/…` or a UNC
/// `//server/…`). Remote SFTP/FTP roots are rooted POSIX paths (`/…`) with no
/// drive prefix, so this distinguishes "stay on the remote backend" from
/// "switch back to the local std::fs scanner".
pub(in crate::app) fn app_data_file(name: &str) -> PathBuf {
    crate::support_dirs::app_data_file(name)
}

pub(in crate::app) fn share_server_path() -> PathBuf {
    app_data_file("share_server.txt")
}

pub(in crate::app) fn load_share_server() -> String {
    std::fs::read_to_string(share_server_path())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

pub(in crate::app) fn default_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Mein Gerät".to_string())
}

/// Stream a remote file to a temp copy and return its local path (for opening
/// remote files in their associated app). Overwrites a prior copy of the same
/// name so re-opening picks up fresh content.
pub(in crate::app) fn download_to_temp(
    be: &dyn crate::vfs::Backend,
    path: &str,
    name: &str,
) -> Result<String, String> {
    download_to(be, path, &open_temp_path(name))
}

/// Stream a remote file to an explicit local `dest` (creating parents). Returns
/// the local path string for launching.
/// How an opened file is launched once it's local: the OS default app, or the
/// native Windows "Open with…" chooser (the `openas` shell verb).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::app) enum OpenMode {
    Default,
    With,
}

// ─── Omnibox (the combo-field) ───────────────────────────────────────────────
// The name-filter doubles as an address bar + command palette. What the input
// means is decided by its content (no mode switch): a leading `>` is a command,
// path-like text navigates, everything else filters the current list as before.

/// What the omnibox input currently means.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::app) enum OmniMode {
    Filter,
    Path,
    Command,
    /// Leading `/` → fuzzy global folder-jump search (dropdown owns the keyboard).
    FolderSearch,
}

pub(in crate::app) fn omni_mode(input: &str) -> OmniMode {
    let t = input.trim_start();
    if t.starts_with('>') {
        OmniMode::Command
    } else if t.starts_with('/') {
        OmniMode::FolderSearch
    } else if omni_is_path(input) {
        OmniMode::Path
    } else {
        OmniMode::Filter
    }
}

/// True when the input should be read as a filesystem path rather than a filter:
/// a drive (`C:`), a root (`\`, `~`), a UNC (`\\srv`), up-navigation (`..`),
/// or anything containing a path separator. (A leading `/` is handled earlier as
/// folder-search, so it's intentionally not a path trigger here.)
pub(in crate::app) fn omni_is_path(input: &str) -> bool {
    let t = input.trim();
    if t.is_empty() {
        return false;
    }
    if t.starts_with('\\') || t.starts_with('~') {
        return true;
    }
    if t.starts_with("..") {
        return true;
    }
    // Pure dots (".." = up 1, "..." = up 2, …).
    if t.len() >= 2 && t.bytes().all(|b| b == b'.') {
        return true;
    }
    let b = t.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return true; // drive-qualified
    }
    t.contains('/') || t.contains('\\')
}

/// If `input` is an up-navigation (`..`, `../..`, `..\..`, or `...`/`....`),
/// return how many levels to go up. Dot-runs use n-dots → n-1 levels.
pub(in crate::app) fn omni_up_levels(input: &str) -> Option<usize> {
    let t = input.trim();
    if t.len() >= 2 && t.bytes().all(|b| b == b'.') {
        return Some(t.len() - 1);
    }
    let segs: Vec<&str> = t.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    if !segs.is_empty() && segs.iter().all(|s| *s == "..") {
        return Some(segs.len());
    }
    None
}

/// Resolve omnibox path-mode input to a concrete path to scan: expand `~`,
/// complete a bare drive (`C:` → `C:\`), and resolve a relative `..`-path
/// against the current root (the OS normalises `..` segments when scanning).
pub(in crate::app) fn expand_omni_path(raw: &str, home: &std::path::Path, root: &str) -> String {
    let t = raw.trim();
    let sep = std::path::MAIN_SEPARATOR_STR;
    if t == "~" {
        return home.to_string_lossy().to_string();
    }
    if let Some(rest) = t.strip_prefix("~/").or_else(|| t.strip_prefix("~\\")) {
        return home
            .join(rest.replace(['/', '\\'], sep))
            .to_string_lossy()
            .to_string();
    }
    let b = t.as_bytes();
    if b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return format!("{}{}", t, sep); // C: → C:\
    }
    // Relative path that starts with `..` (mixed, e.g. ../sibling): resolve
    // against the current root.
    if t.starts_with("..") && !root.is_empty() {
        return PathBuf::from(root.replace('/', sep))
            .join(t.replace(['/', '\\'], sep))
            .to_string_lossy()
            .to_string();
    }
    t.replace('/', sep)
}

/// A one-shot command available via `>` in the omnibox (folder actions only —
/// navigation/roots are expressed as `Go`/`Up`/`Connect` actions).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum OmniCmd {
    NewFolder,
    Reveal,
    Terminal,
    CopyPath,
    StarToggle,
    Refresh,
    Analytics,
}

/// What activating a dropdown row does.
#[derive(Clone, Debug)]
pub(in crate::app) enum OmniAction {
    /// Navigate to (scan) this local path.
    Go(String),
    /// Open a saved remote connection by index into `saved_connections`.
    Connect(usize),
    /// Run a one-shot folder command.
    Cmd(OmniCmd),
}

/// One row in the omnibox dropdown.
pub(in crate::app) struct OmniItem {
    pub(in crate::app) icon: &'static str,
    pub(in crate::app) label: String,
    pub(in crate::app) sub: String,
    pub(in crate::app) action: OmniAction,
}

/// A control reachable from the Alt key-overlay (classic accelerator badges).
#[derive(Clone, Copy, Debug)]
pub(in crate::app) enum AccelAct {
    Back,
    Forward,
    Up,
    PickFolder,
    Split,
    NewTab,
    Tab(usize),
}

/// Map an accelerator character to its egui logical key (letters + digits).
pub(in crate::app) fn accel_key(c: char) -> Option<egui::Key> {
    use egui::Key::*;
    Some(match c.to_ascii_uppercase() {
        'A' => A,
        'B' => B,
        'C' => C,
        'D' => D,
        'E' => E,
        'F' => F,
        'G' => G,
        'H' => H,
        'I' => I,
        'J' => J,
        'K' => K,
        'L' => L,
        'M' => M,
        'N' => N,
        'O' => O,
        'P' => P,
        'Q' => Q,
        'R' => R,
        'S' => S,
        'T' => T,
        'U' => U,
        'V' => V,
        'W' => W,
        'X' => X,
        'Y' => Y,
        'Z' => Z,
        '1' => Num1,
        '2' => Num2,
        '3' => Num3,
        '4' => Num4,
        '5' => Num5,
        '6' => Num6,
        '7' => Num7,
        '8' => Num8,
        '9' => Num9,
        _ => return None,
    })
}

/// Draw one accelerator badge (boxed letter) at the top-left of `rect`.
pub(in crate::app) fn draw_accel_badge(painter: &egui::Painter, rect: egui::Rect, c: char) {
    let sz = egui::vec2(16.0, 16.0);
    let at = egui::Rect::from_min_size(rect.left_top() + egui::vec2(1.0, 1.0), sz);
    painter.rect_filled(at, 3.0, Color32::from_rgb(250, 240, 200));
    painter.rect_stroke(
        at,
        3.0,
        egui::Stroke::new(1.0, Color32::from_rgb(120, 90, 30)),
    );
    painter.text(
        at.center(),
        egui::Align2::CENTER_CENTER,
        c,
        egui::FontId::proportional(12.0),
        Color32::from_rgb(40, 30, 10),
    );
}

// ─── Storage analytics ───────────────────────────────────────────────────────

/// Squarified treemap layout (Bruls/Huizing/van Wijk). Returns a rect per input
/// weight, in the SAME order, with area proportional to the weight; zero/negative
/// weights get an empty rect.
pub(in crate::app) fn treemap_layout(weights: &[f64], rect: egui::Rect) -> Vec<egui::Rect> {
    let mut out = vec![egui::Rect::ZERO; weights.len()];
    let total: f64 = weights.iter().filter(|w| **w > 0.0).sum();
    if total <= 0.0 || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return out;
    }
    let area = rect.width() as f64 * rect.height() as f64;
    let mut idx: Vec<usize> = (0..weights.len()).filter(|&i| weights[i] > 0.0).collect();
    idx.sort_by(|&a, &b| {
        weights[b]
            .partial_cmp(&weights[a])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let scaled: Vec<f64> = idx.iter().map(|&i| weights[i] / total * area).collect();
    let laid = squarify_sorted(&scaled, rect);
    for (k, &i) in idx.iter().enumerate() {
        out[i] = laid[k];
    }
    out
}

pub(in crate::app) fn squarify_worst(row: &[f64], sum: f64, side: f32) -> f64 {
    let w2 = (side as f64) * (side as f64);
    let s2 = sum * sum;
    let (mut rmax, mut rmin) = (f64::MIN, f64::MAX);
    for &a in row {
        rmax = rmax.max(a);
        rmin = rmin.min(a);
    }
    (w2 * rmax / s2).max(s2 / (w2 * rmin))
}

/// Lay scaled areas (sum == rect area, sorted desc) into `rect` row by row.
pub(in crate::app) fn squarify_sorted(scaled: &[f64], mut rect: egui::Rect) -> Vec<egui::Rect> {
    let n = scaled.len();
    let mut out = vec![egui::Rect::ZERO; n];
    let mut i = 0;
    while i < n {
        let side = rect.width().min(rect.height());
        let mut j = i;
        let mut sum = scaled[i];
        while j + 1 < n {
            let new_sum = sum + scaled[j + 1];
            let cur = squarify_worst(&scaled[i..=j], sum, side);
            let nxt = squarify_worst(&scaled[i..=j + 1], new_sum, side);
            if nxt > cur {
                break;
            }
            j += 1;
            sum = new_sum;
        }
        let row = &scaled[i..=j];
        if rect.width() >= rect.height() {
            let strip_w = (sum / rect.height() as f64) as f32;
            let mut yy = rect.min.y;
            for (k, a) in row.iter().enumerate() {
                let ch = (*a / sum * rect.height() as f64) as f32;
                out[i + k] =
                    egui::Rect::from_min_size(egui::pos2(rect.min.x, yy), egui::vec2(strip_w, ch));
                yy += ch;
            }
            rect.min.x += strip_w;
        } else {
            let strip_h = (sum / rect.width() as f64) as f32;
            let mut xx = rect.min.x;
            for (k, a) in row.iter().enumerate() {
                let cw = (*a / sum * rect.width() as f64) as f32;
                out[i + k] =
                    egui::Rect::from_min_size(egui::pos2(xx, rect.min.y), egui::vec2(cw, strip_h));
                xx += cw;
            }
            rect.min.y += strip_h;
        }
        i = j + 1;
    }
    out
}

/// Wrap a REMOTE backend with the interactive browsing cache; local backends
/// pass through (their `std::fs` listing is already instant).
pub(in crate::app) fn cache_remote(b: crate::vfs::BackendHandle) -> crate::vfs::BackendHandle {
    if b.is_local() {
        b
    } else {
        Arc::new(crate::vfs::CachingBackend::new(b))
    }
}

/// Recursive (files, dirs) count under `node`.
pub(in crate::app) fn count_subtree(node: &crate::analytics::SizeNode) -> (u64, u64) {
    let mut files = 0u64;
    let mut dirs = 0u64;
    for c in &node.children {
        if c.is_dir {
            dirs += 1;
            let (f, d) = count_subtree(c);
            files += f;
            dirs += d;
        } else {
            files += 1;
        }
    }
    (files, dirs)
}

// Nested-treemap layout tuning.
