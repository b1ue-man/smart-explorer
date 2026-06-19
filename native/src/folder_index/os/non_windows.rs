#[cfg(not(windows))]
pub(super) fn file_attributes(_meta: &std::fs::Metadata) -> u32 {
    0
}
