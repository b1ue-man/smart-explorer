//! Host-facing parts of the text view: the app data locations of the
//! language folder and its cache, and the byte sources over local files
//! (positioned reads per platform) and over every `vfs::Backend`.
#[path = "shared/file_source.rs"]
mod file_source;
#[path = "shared/locations.rs"]
mod locations;
#[cfg(any(target_os = "linux", target_os = "android"))]
#[path = "linux_os.rs"]
mod platform;
#[cfg(windows)]
#[path = "windows.rs"]
mod platform;
#[path = "shared/vfs_source.rs"]
mod vfs_source;

pub use file_source::FileSource;
pub(crate) use locations::{languages_cache_folder, languages_folder};
pub use locations::{open_backend, open_local};
pub use vfs_source::VfsSource;
