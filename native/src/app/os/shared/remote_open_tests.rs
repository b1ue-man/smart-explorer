use super::missing_temp_requires_recovery;

#[test]
fn existing_temp_does_not_enter_missing_recovery_state() {
    assert!(!missing_temp_requires_recovery(123));
}

#[test]
fn missing_temp_is_retained_for_atomic_editor_save_recovery() {
    assert!(missing_temp_requires_recovery(0));
}
