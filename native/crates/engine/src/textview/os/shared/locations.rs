//! Where the text view keeps its files: the language folder for added packs
//! and the cache of compiled syntax sets, both in the app data folder
//! (desktop: `<data home>/smart_explorer`, Android: the folder the app hands
//! in through `support_dirs::set_host`).
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::textview::{TextDocument, TextViewResult};
use crate::vfs::BackendHandle;

use super::{FileSource, VfsSource};

/// `<app data>/languages`: one `.sublime-syntax` file or one package folder
/// per added language pack.
pub(crate) fn languages_folder() -> PathBuf {
    crate::support_dirs::app_data_dir().join("languages")
}

/// `<app data>/languages-cache`: the compiled syntax set of the added packs.
pub(crate) fn languages_cache_folder() -> PathBuf {
    crate::support_dirs::app_data_dir().join("languages-cache")
}

/// Opens a local file as a text document (positioned reads, no full load).
pub fn open_local(path: &Path) -> TextViewResult<TextDocument> {
    TextDocument::open(Arc::new(FileSource::open(path)?))
}

/// Opens a file of any backend (local, SFTP, FTP, WebDAV, SMB, Google
/// Drive, Share peers, vaults) as a text document.
pub fn open_backend(backend: BackendHandle, path: &str) -> TextViewResult<TextDocument> {
    TextDocument::open(Arc::new(VfsSource::open(backend, path)?))
}
