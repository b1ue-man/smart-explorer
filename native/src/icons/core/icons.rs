// Native Windows file-type icons for the table.
//
// We show the same per-extension icons Explorer shows (folder, .pdf, .docx,
// .exe, …). Strategy:
//   - Icons are keyed by EXTENSION (or a "<dir>"/"<file>" sentinel), so a
//     million-row listing only ever creates a few dozen textures, not one per
//     row.
//   - Extraction runs on a dedicated COM-initialized worker thread via
//     SHGetFileInfoW with SHGFI_USEFILEATTRIBUTES (never touches disk — it
//     resolves the icon for a *type*, given a fake name + attributes).
//   - HICON -> RGBA8 via GetIconInfo + a top-down 32bpp GetDIBits, with the
//     classic-icon AND-mask fallback when the colour bitmap carries no alpha.
//   - The UI thread uploads the RGBA into an egui texture and caches it.
//
// The egui/UI side never blocks: when an icon isn't cached yet the cell draws
// the old emoji glyph, and swaps to the real icon a frame later.

use eframe::egui;
use std::collections::{HashMap, HashSet};

pub enum IconKind {
    Dir,
    GenericFile,
    Ext(String),
}

pub struct IconResult {
    pub key: String,
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

/// Stable cache key for an entry's icon.
pub fn icon_key(is_dir: bool, ext: &str) -> String {
    if is_dir {
        "<dir>".to_string()
    } else if ext.is_empty() {
        "<file>".to_string()
    } else {
        ext.to_string()
    }
}

fn key_to_kind(key: &str) -> IconKind {
    match key {
        "<dir>" => IconKind::Dir,
        "<file>" => IconKind::GenericFile,
        other => IconKind::Ext(other.to_string()),
    }
}

pub struct IconCache {
    textures: HashMap<String, egui::TextureHandle>,
    requested: HashSet<String>,
    #[cfg(windows)]
    req_tx: Option<crossbeam_channel::Sender<(String, IconKind)>>,
    #[cfg(windows)]
    res_rx: Option<crossbeam_channel::Receiver<IconResult>>,
}

impl IconCache {
    pub fn new() -> Self {
        #[cfg(windows)]
        {
            let (req_tx, req_rx) = crossbeam_channel::unbounded::<(String, IconKind)>();
            let (res_tx, res_rx) = crossbeam_channel::unbounded::<IconResult>();
            std::thread::Builder::new()
                .name("icon-extract".into())
                .spawn(move || win::worker(req_rx, res_tx))
                .ok();
            return Self {
                textures: HashMap::new(),
                requested: HashSet::new(),
                req_tx: Some(req_tx),
                res_rx: Some(res_rx),
            };
        }
        #[cfg(not(windows))]
        {
            Self {
                textures: HashMap::new(),
                requested: HashSet::new(),
            }
        }
    }

    pub fn get(&self, key: &str) -> Option<&egui::TextureHandle> {
        self.textures.get(key)
    }

    /// Ask the worker for an icon if we haven't already. Cheap & deduplicated.
    pub fn request(&mut self, key: String) {
        if self.textures.contains_key(&key) || !self.requested.insert(key.clone()) {
            return;
        }
        #[cfg(windows)]
        if let Some(tx) = self.req_tx.as_ref() {
            let _ = tx.send((key.clone(), key_to_kind(&key)));
        }
        #[cfg(not(windows))]
        {
            let _ = &key; // no-op: emoji fallback stays
        }
    }

    /// Drain finished extractions into textures. Returns true if anything new
    /// landed (so the caller can request a repaint).
    pub fn drain(&mut self, ctx: &egui::Context) -> bool {
        #[cfg(windows)]
        {
            let mut results: Vec<IconResult> = Vec::new();
            if let Some(rx) = self.res_rx.as_ref() {
                while let Ok(r) = rx.try_recv() {
                    results.push(r);
                }
            }
            if results.is_empty() {
                return false;
            }
            for r in results {
                let (w, h) = (r.w.max(1), r.h.max(1));
                let pixels = if r.rgba.len() == w * h * 4 {
                    r.rgba
                } else {
                    vec![0u8; w * h * 4]
                };
                let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &pixels);
                let tex = ctx.load_texture(
                    format!("ficon:{}", r.key),
                    img,
                    egui::TextureOptions::LINEAR,
                );
                self.textures.insert(r.key, tex);
            }
            true
        }
        #[cfg(not(windows))]
        {
            let _ = ctx;
            false
        }
    }
}

impl Default for IconCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(windows)]
#[path = "../os/windows.rs"]
mod win;

#[cfg(all(test, windows))]
mod tests {
    use super::{IconKind, IconResult};

    /// Exercises the real SHGetFileInfo + GDI extraction path end-to-end on
    /// Windows and asserts we get visible (non-transparent) pixels — guards
    /// the alpha/mask handling and the worker plumbing against regressions.
    #[test]
    fn extracts_real_icons() {
        let (req_tx, req_rx) = crossbeam_channel::unbounded::<(String, IconKind)>();
        let (res_tx, res_rx) = crossbeam_channel::unbounded::<IconResult>();
        std::thread::spawn(move || super::win::worker(req_rx, res_tx));

        req_tx.send(("<dir>".into(), IconKind::Dir)).unwrap();
        req_tx
            .send(("txt".into(), IconKind::Ext("txt".into())))
            .unwrap();

        let mut visible = 0;
        for _ in 0..2 {
            let r = res_rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .expect("icon result");
            assert!(r.w >= 1 && r.h >= 1);
            assert_eq!(r.rgba.len(), r.w * r.h * 4);
            if r.rgba.chunks_exact(4).any(|p| p[3] != 0) {
                visible += 1;
            }
        }
        assert!(visible >= 1, "no visible icon pixels were extracted");
        drop(req_tx);
    }
}
