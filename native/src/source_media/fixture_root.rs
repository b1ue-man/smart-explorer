//! Temporary test roots whose indexed paths survive the real load filters.
use std::io;

pub(super) fn new() -> io::Result<tempfile::TempDir> {
    let current = std::env::current_dir()?;
    let parent = current
        .ancestors()
        .find(|parent| {
            parent.to_str().is_some_and(|path| {
                !crate::folder_index::path_has_skipped_segment(&path.replace('\\', "/"))
            })
        })
        .ok_or_else(|| io::Error::other("optical index fixture has no indexable parent"))?;
    let fixture = tempfile::Builder::new()
        .prefix("se-optical-fixture-")
        .tempdir_in(parent)?;
    let path = fixture
        .path()
        .to_str()
        .ok_or_else(|| io::Error::other("optical index fixture path is not Unicode"))?;
    if crate::folder_index::path_has_skipped_segment(&path.replace('\\', "/")) {
        return Err(io::Error::other(
            "optical index fixture path is excluded by the index filters",
        ));
    }
    Ok(fixture)
}
