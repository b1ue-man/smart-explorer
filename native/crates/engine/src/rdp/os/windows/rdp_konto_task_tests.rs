//! Windows task test of the sign-in with the device's account (plan
//! 2026-11-17 P4): mstsc proposes the device's account.
use crate::rdp::os::platform::clients::trust::{
    read_username_hint, server_names, write_username_hint,
};

/// `UsernameHint` stands under both names mstsc may use for the device
/// address (as `CertHash`), and a later start replaces it.
#[test]
fn rdp_konto_task_username_hint_written_for_both_names() {
    let listener: std::net::SocketAddr = "127.203.9.11:51236".parse().unwrap();
    let names = server_names(listener);
    for account in ["PC-B\\alice", "MicrosoftAccount\\alice@outlook.com"] {
        write_username_hint(listener, account).unwrap();
        for name in &names {
            assert_eq!(read_username_hint(name).as_deref(), Some(account), "{name}");
        }
    }
    for name in names {
        let path = format!(r"Software\Microsoft\Terminal Server Client\Servers\{name}");
        let _ = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).delete_subkey(&path);
    }
}
