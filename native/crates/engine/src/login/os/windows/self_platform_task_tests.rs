//! E21 Windows grant channel: only user account SIDs set grants.
use super::*;

#[test]
fn login_selbst_task_windows_foreign_principals_refused() {
    for sid in [
        "S-1-5-21-1004336348-1177238915-682003330-1001",
        "S-1-12-1-1234567890-1234567890-1234567890-1234567890",
    ] {
        assert!(valid_windows_account(sid), "{sid}");
    }
    // LocalSystem, LocalService (the gateway), NetworkService, a service
    // SID, Administrators, an AppContainer and anything malformed.
    for sid in [
        "S-1-5-18",
        "S-1-5-19",
        "S-1-5-20",
        "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464",
        "S-1-5-32-544",
        "S-1-15-2-1",
        "S-1-5-21-1 /x",
        "",
    ] {
        assert!(!valid_windows_account(sid), "{sid}");
    }
    assert!(
        SCRIPT.contains("SelfInstall"),
        "embedded installer knows the self mode"
    );
    assert_eq!(
        crate::login::os::shared::self_install::app_image(Path::new(r"C:\Apps\SE\se.exe")),
        Path::new(r"C:\Apps\SE").join(APP_FILE)
    );
}

#[test]
#[ignore = "Administratorrechte; Suite crates-task"]
fn login_selbst_task_windows_grant_pipe_names_the_client_token() {
    static STOP: AtomicBool = AtomicBool::new(false);
    assert!(privileged(), "needs an elevated administrator");
    start_grant_pipe(&STOP).expect("grant pipe");
    // The test process is not LocalSystem: the worker side refuses it as server.
    assert!(connect_grants().is_err());
    STOP.store(true, Ordering::SeqCst);
}
