#[cfg(not(windows))]
pub(super) fn get_attrs(_meta: &std::fs::Metadata) -> (bool, bool) {
    (false, false)
}
