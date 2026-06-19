use super::schedule::{drive_matches, wildcard_ci};

#[test]
fn wildcard_matches() {
    assert!(wildcard_ci("*", "anything"));
    assert!(wildcard_ci("backup*", "BACKUP_DRIVE"));
    assert!(wildcard_ci("E:", "e:"));
    assert!(wildcard_ci("????", "ABCD"));
    assert!(!wildcard_ci("backup?", "backup"));
    assert!(!wildcard_ci("x*", "yz"));
}

#[test]
fn drive_matching() {
    assert!(drive_matches("", "E:|STICK|1A2B")); // empty = any
    assert!(drive_matches("STICK", "E:|STICK|1A2B")); // by label
    assert!(drive_matches("E:", "E:|STICK|1A2B")); // by letter
    assert!(drive_matches("1A2B", "E:|STICK|1A2B")); // by serial
    assert!(drive_matches("back*", "F:|Backup|99")); // wildcard label
    assert!(!drive_matches("nope", "E:|STICK|1A2B"));
}
