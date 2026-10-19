//! The file clipboard of a surface process (DSK-M7, U37-14). The view and its
//! actions (`desktop_surface_actions`) copy, cut and paste desktop files
//! through `app::platform_helpers::{write,read}_clipboard_files`; on Linux
//! those ask the provider installed here. The Wayland layer backend installs
//! one that speaks `wl_data_device` (`text/uri-list`,
//! `x-special/gnome-copied-files`); the X11 surface follows with DSK-M6. A
//! process without a provider keeps the earlier answer "not available", and
//! the surface then carries the paths as text with a marker (`clip_text`).
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

/// Copy, cut and paste of files between programs.
pub(crate) trait FileClipboard: Send + Sync {
    /// Puts `files` on the clipboard; `cut` marks a move.
    fn write(&self, files: &[PathBuf], cut: bool) -> Result<(), String>;
    /// The files another program (or this one) put on the clipboard and
    /// whether they were cut; `Ok(None)` when it holds no file list.
    fn read(&self) -> Result<Option<(Vec<PathBuf>, bool)>, String>;
}

static PROVIDER: OnceLock<Arc<dyn FileClipboard>> = OnceLock::new();

/// Installs the provider of this process; only the first call counts.
pub(crate) fn install(provider: Arc<dyn FileClipboard>) {
    let _ = PROVIDER.set(provider);
}

/// The provider of this process, if a surface host installed one.
pub(crate) fn installed() -> Option<&'static Arc<dyn FileClipboard>> {
    PROVIDER.get()
}
