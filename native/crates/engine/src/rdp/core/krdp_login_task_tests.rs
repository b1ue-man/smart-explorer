//! Task tests of the KRdp sign-in with se's own KRdp user (RDP-ZIELKONTO,
//! second step): who may use it, the `Users` list and the steering end.
use super::*;

/// KRdp shows the session of the account it runs as: only that account's
/// grant signs in, never through a root worker.
#[test]
fn rdp_konto_task_krdp_only_for_the_session_account() {
    assert!(account_allowed("anna", "anna").is_ok());
    let other = account_allowed("anna", "ben").unwrap_err();
    assert!(other.contains("anna") && other.contains("ben"), "{other}");
    assert!(account_allowed("root", "root").is_err());
    assert!(account_allowed("root", "anna").is_err());
}

/// se's user joins the `Users` list once and leaves only its own entry;
/// other entries keep their text.
#[test]
fn rdp_konto_task_krdp_users_list_keeps_other_entries() {
    assert_eq!(users_with("", KRDP_USER).as_deref(), Some(KRDP_USER));
    assert_eq!(
        users_with("anna,b\\,c", KRDP_USER).as_deref(),
        Some("anna,b\\,c,se-krdp")
    );
    assert_eq!(users_with("anna, se-krdp", KRDP_USER), None);
    assert_eq!(
        users_without("anna,se-krdp,b\\,c", KRDP_USER).as_deref(),
        Some("anna,b\\,c")
    );
    assert_eq!(users_without("se-krdp", KRDP_USER).as_deref(), Some(""));
    assert_eq!(users_without("anna", KRDP_USER), None);
    let password = new_password().unwrap();
    assert_eq!(password.len(), 43);
    assert!(password
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
    assert_ne!(password, new_password().unwrap());
}

/// A steering end replaces the password only when a device that received
/// it lost its right; after the last steering device the user goes.
#[test]
fn rdp_konto_task_krdp_password_replaced_when_a_recipient_ends() {
    use CoverageChange::*;
    assert_eq!(coverage_change(false, true, false), Keep);
    assert_eq!(coverage_change(true, false, true), Keep);
    assert_eq!(coverage_change(true, true, true), Rotate);
    assert_eq!(coverage_change(true, false, false), Remove);
    assert_eq!(coverage_change(true, true, false), Remove);
}
