// In-memory index of every folder under the chosen scan roots, used to power
// fuzzy folder search ("type 'dwnlds' to jump to Downloads").
//
// Why an index: a live filesystem walk would be far too slow to do on every
// keystroke. Pre-computing paths once gives us O(N) scoring against an
// in-memory array; for ~500k folders this is ~30-80 ms in release builds.
//
// Storage: plain UTF-8 paths, one per line, in %APPDATA%\smart_explorer\
// folder_index.txt. Loading is just split-on-newline.

#[path = "core_oslocked/index_build.rs"]
mod core_oslocked;
#[path = "core/filters.rs"]
mod filters;
#[path = "core/model.rs"]
mod model;
#[path = "core_oslocked/persistence.rs"]
mod persistence;
#[path = "core/search.rs"]
mod search;

#[allow(unused_imports)]
pub use filters::{
    is_generic_id, path_has_skipped_segment, should_skip, should_skip_meta,
};
pub use core_oslocked::stat_and_rank;
pub use model::{FolderIndex, IndexMsg};

#[cfg(test)]
#[path = "core/tests.rs"]
mod tests;
