use super::*;

#[test]
fn review_task_host_private_names_hide_quarantine_and_versions_but_allow_transfer_stages() {
    for name in [
        ".se-versions",
        ".SE-VERSIONS. ",
        ".se-versions:stream",
        ".held.se-recycle-0123456789abcdef",
        ".HELD.SE-RECYCLE-0123456789ABCDEF. ",
    ] {
        assert!(private_name(name), "{name}");
        assert!(private_path(&format!("/export/a/{name}/content")));
    }
    for name in [
        ".held.se-recycle-not-a-nonce",
        ".held.se-recycle-0123456789abcde",
        ".held.se-recycle-0123456789abcdef0",
        ".file.smart-explorer-deadbeef.part",
        "file.se-copy-0123456789abcdef",
        "node_modules",
        ".se-versions-other",
    ] {
        assert!(!private_name(name), "{name}");
    }
}

#[test]
fn review_task_host_system_write_classification_covers_startup_shell_and_keys() {
    for path in [
        "/home/user/.ssh/id_ed25519",
        "/home/user/.profile",
        "/home/user/.config/autostart/app.desktop",
        "/home/user/.config/systemd/user/x.service",
        "/etc/passwd",
        "C:/Windows/System32/x",
        "C:/Users/user/AppData/Roaming/Microsoft/Windows/Start Menu/Programs/Startup/x.lnk",
        "C:/Users/user/Documents/WindowsPowerShell/profile.ps1",
        "//?/C:/Windows/x",
    ] {
        assert!(system_write(path), "{path}");
    }
    for path in [
        "/home/user/documents/file",
        "/shared/node_modules/package.json",
        "/shared/.profile.txt",
        "/shared/.file.smart-explorer-deadbeef.part",
        "C:/Users/user/Documents/readme.txt",
    ] {
        assert!(!system_write(path), "{path}");
    }
}
