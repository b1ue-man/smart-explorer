//! Display state of the storage analysis: the measure („Belegt“/„Größe“),
//! the cached treemap (cells, labels and the fill mesh built once per layout
//! and theme), the shown tab and the overviews „Größte Dateien“ and „Nach
//! Dateityp“ of the shown folder.
use super::analytics_measure::{MeasureView, SizeMeasure};
use super::app_models::TmCell;
use crate::analytics_overview::Overview;
use eframe::egui::{self, Color32};
use std::cell::RefCell;

/// Tooltip of the NTFS system files node of the fast NTFS way (MFT).
pub(in crate::app) const SYSTEM_FILES_HINT: &str = "Vom Dateisystem belegt, nicht löschbar";

/// The node „NTFS-Systemdateien“ (`analytics::SYSTEM_FILES_NAME`): space the
/// file system itself holds ($MFT, journal, …). It has no place to open and
/// nothing to delete.
pub(in crate::app) fn is_system_files(name: &str, is_dir: bool) -> bool {
    !is_dir && name == crate::analytics::SYSTEM_FILES_NAME
}

/// Largest files the overview lists.
pub(in crate::app) const OVERVIEW_TOP: usize = 100;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) enum AnalyticsTab {
    #[default]
    Treemap,
    Largest,
    Types,
}

impl AnalyticsTab {
    pub(in crate::app) const ALL: [Self; 3] = [Self::Treemap, Self::Largest, Self::Types];

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::Treemap => "Kacheln",
            Self::Largest => "Größte Dateien",
            Self::Types => "Nach Dateityp",
        }
    }
}

/// The fills of all cells as one mesh, for the theme it was built in.
pub(in crate::app) struct CachedMesh {
    pub(in crate::app) dark: bool,
    pub(in crate::app) surface: Color32,
    pub(in crate::app) mesh: egui::Mesh,
}

pub(in crate::app) struct TreemapCache {
    pub(in crate::app) cells: Vec<TmCell>,
    pub(in crate::app) rect: egui::Rect,
    /// Built on the first paint after a layout or theme change; painting
    /// borrows the app immutably, hence the cell.
    pub(in crate::app) mesh: RefCell<Option<CachedMesh>>,
}

impl Default for TreemapCache {
    fn default() -> Self {
        Self {
            cells: Vec::new(),
            rect: egui::Rect::ZERO,
            mesh: RefCell::new(None),
        }
    }
}

pub(in crate::app) struct AnalyticsView {
    pub(in crate::app) measure: MeasureView,
    pub(in crate::app) treemap: TreemapCache,
    pub(in crate::app) tab: AnalyticsTab,
    /// Overview of the shown folder for the current measure; built when a
    /// list tab is shown.
    pub(in crate::app) overview: Option<Overview>,
}

impl AnalyticsView {
    pub(in crate::app) fn new(chosen: SizeMeasure) -> Self {
        Self {
            measure: MeasureView::new(chosen),
            treemap: TreemapCache::default(),
            tab: AnalyticsTab::default(),
            overview: None,
        }
    }

    /// The tree, the drill position, the measure or the layout changed.
    pub(in crate::app) fn invalidate(&mut self) {
        self.treemap = TreemapCache::default();
        self.overview = None;
    }

    pub(in crate::app) fn set_cells(&mut self, cells: Vec<TmCell>, rect: egui::Rect) {
        self.treemap = TreemapCache {
            cells,
            rect,
            mesh: RefCell::new(None),
        };
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn analyse_task_system_files_node_is_not_openable() {
        let name = crate::analytics::SYSTEM_FILES_NAME;
        assert!(super::is_system_files(name, false));
        assert!(!super::is_system_files(name, true));
        assert!(!super::is_system_files("pagefile.sys", false));
    }
}
