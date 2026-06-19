// In-memory index of every folder under the chosen scan roots, used to power
// fuzzy folder search ("type 'dwnlds' to jump to Downloads").
//
// Why an index: a live filesystem walk would be far too slow to do on every
// keystroke. Pre-computing paths once gives us O(N) scoring against an
// in-memory array; for ~500k folders this is ~30-80 ms in release builds.
//
// Storage: plain UTF-8 paths, one per line, in %APPDATA%\smart_explorer\
// folder_index.txt. Loading is just split-on-newline.

#[path = "core/filters.rs"]
mod filters;
#[path = "core/model.rs"]
mod model;
#[path = "os/shared.rs"]
mod os;
#[cfg(windows)]
#[path = "os/windows.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "os/linux_os.rs"]
mod platform;
#[path = "core/search.rs"]
mod search;

#[allow(unused_imports)]
pub use filters::{is_generic_id, path_has_skipped_segment, should_skip, should_skip_meta};
pub use model::{FolderIndex, IndexMsg};
pub use os::stat_and_rank;

#[cfg(test)]
#[path = "core/tests.rs"]
mod tests;
