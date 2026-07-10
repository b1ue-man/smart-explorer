use super::should_cleanup_missing_temp;

#[test]
fn finished_launcher_does_not_cleanup_existing_clean_temp() {
    assert!(!should_cleanup_missing_temp(123, true, false));
}

#[test]
fn missing_clean_temp_can_be_untracked_after_launcher_finishes() {
    assert!(should_cleanup_missing_temp(0, true, false));
    assert!(!should_cleanup_missing_temp(0, false, false));
    assert!(!should_cleanup_missing_temp(0, true, true));
}
