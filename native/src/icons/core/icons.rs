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

use super::os::IconWorker;

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
    worker: IconWorker,
}

impl IconCache {
    pub fn new() -> Self {
        Self {
            textures: HashMap::new(),
            requested: HashSet::new(),
            worker: IconWorker::new(),
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
        self.worker.request(key.clone(), key_to_kind(&key));
    }

    /// Drain finished extractions into textures. Returns true if anything new
    /// landed (so the caller can request a repaint).
    pub fn drain(&mut self, ctx: &egui::Context) -> bool {
        let results = self.worker.drain();
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
}

impl Default for IconCache {
    fn default() -> Self {
        Self::new()
    }
}
