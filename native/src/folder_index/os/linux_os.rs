#[cfg(not(windows))]
pub(super) fn should_skip_meta(name: &str, _meta: &std::fs::Metadata) -> bool {
    super::filters::should_skip(name)
}
