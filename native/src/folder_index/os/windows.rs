#[cfg(windows)]
pub(super) fn file_attributes(meta: &std::fs::Metadata) -> u32 {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes()
}
