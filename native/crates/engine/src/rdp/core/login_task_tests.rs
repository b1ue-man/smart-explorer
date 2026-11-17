//! Task tests of the sign-in with the account of the sharing device (plan
//! 2026-11-17 P4, TODO RDP-ZIELKONTO): account names, the viewer's plan
//! against old and new hosts, the mstsc start and profile, the FreeRDP
//! invocation and the redaction of every secret.
use super::*;
use crate::share::apps_wire::{AppsCommand, AppsResult};

fn listener() -> SocketAddr {
    "127.1.2.3:3390".parse().unwrap()
}

/// A user or domain part never carries a line break (it would end a `.rdp`
/// line or a FreeRDP argument line) or a separator of account names.
#[test]
fn rdp_konto_task_account_rejects_line_and_separator_characters() {
    for good in [
        "alice",
        "Max Mustermann",
        "alice@outlook.com",
        "DESKTOP-4711",
        "se-remote",
    ] {
        assert!(valid_account_part(good), "{good:?}");
    }
    let long = "a".repeat(MAX_ACCOUNT_PART + 1);
    for bad in [
        "",
        "a\nb",
        "a\rb",
        "a\0b",
        "a\\b",
        "a/b",
        "a:b",
        "a;b",
        "a|b",
        "a\"b",
        " alice",
        "alice ",
        long.as_str(),
    ] {
        assert!(!valid_account_part(bad), "{bad:?}");
        assert!(HostAccount::new(bad, None).is_err(), "{bad:?}");
        assert!(HostAccount::new("alice", Some(bad)).is_err(), "{bad:?}");
    }
    assert!(valid_password("pässwörd mit Leerzeichen!"));
    let long = "x".repeat(MAX_PASSWORD + 1);
    for bad in ["", "a\nb", "a\rb", "a\0b", long.as_str()] {
        assert!(!valid_password(bad), "{bad:?}");
    }
    // A reply naming such an account is refused as a whole.
    let reply = LoginInfo::from_wire("a\nb".into(), None, None, LoginState::EntryNeeded);
    assert!(reply.is_err());
}

/// The whole desktop starts as `mstsc /v:<address>` without a `.rdp` file,
/// so mstsc shows no warning with redirection checkboxes (CVE-2026-26151); a
/// RemoteApp keeps its profile.
#[test]
fn rdp_konto_task_mstsc_desktop_uses_v_without_profile() {
    assert_eq!(
        mstsc_start(listener(), None),
        MstscStart::Address("/v:127.1.2.3:3390".into())
    );
    assert_eq!(mstsc_start(listener(), Some("calc")), MstscStart::Profile);
}

/// The RemoteApp profile names the host's account (`username:s:`,
/// `domain:s:`), never a password.
#[test]
fn rdp_konto_task_remoteapp_profile_carries_host_account() {
    let account = HostAccount::new("alice", Some("PC-B")).unwrap();
    let body = mstsc_profile_body(listener(), Some("calc"), Some(&account)).unwrap();
    for line in [
        "full address:s:127.1.2.3:3390\r\n",
        "username:s:alice\r\n",
        "domain:s:PC-B\r\n",
        "remoteapplicationprogram:s:||calc\r\n",
        "prompt for credentials:i:0\r\n",
    ] {
        assert!(body.contains(line), "{line:?} in {body:?}");
    }
    assert!(!body.to_ascii_lowercase().contains("password"), "{body:?}");
    let microsoft = windows_account("PC-B\\alice", Some("alice@outlook.com")).unwrap();
    let body = mstsc_profile_body(listener(), Some("calc"), Some(&microsoft)).unwrap();
    assert!(
        body.contains("username:s:alice@outlook.com\r\n"),
        "{body:?}"
    );
    assert!(body.contains("domain:s:MicrosoftAccount\r\n"), "{body:?}");
    // Without an account (host up to 0.5.197) the profile is as before.
    let plain = mstsc_profile_body(listener(), None, None).unwrap();
    assert!(!plain.contains("username:s:") && plain.contains("remoteapplicationmode:i:0"));
    assert!(mstsc_profile_body(listener(), Some("bad\r\nalias"), None).is_err());
}

/// Assumption K2: a Microsoft account is `MicrosoftAccount\<principal>`,
/// otherwise `NameSamCompatible` (`DOMAIN\user`, a local account under the
/// computer name); Linux accounts have no domain.
#[test]
fn rdp_konto_task_account_name_forms() {
    let local = windows_account("PC-B\\alice", None).unwrap();
    assert_eq!((local.username(), local.domain()), ("alice", Some("PC-B")));
    assert_eq!(local.qualified(), "PC-B\\alice");
    let microsoft = windows_account("PC-B\\alice", Some(" alice@outlook.com ")).unwrap();
    assert_eq!(microsoft.qualified(), "MicrosoftAccount\\alice@outlook.com");
    let domain = windows_account("CORP\\bob", Some("")).unwrap();
    assert_eq!(domain.qualified(), "CORP\\bob");
    let bare = windows_account("alice", None).unwrap();
    assert_eq!(
        (bare.domain(), bare.qualified()),
        (None, "alice".to_string())
    );
    let remote = HostAccount::new("se-remote", None).unwrap();
    assert_eq!(remote.qualified(), "se-remote");
    let view = LoginInfo::new(Some(remote), None, LoginState::Automatic).view();
    assert!(view.text.contains(REMOTE_NOTICE), "{}", view.text);
    assert_eq!(view.state, "automatic");
    assert_eq!(
        freerdp_account_args(&local),
        vec!["/u:alice".to_string(), "/d:PC-B".to_string()]
    );
    for state in [
        "automatic",
        "stored",
        "entry_needed",
        "unsupported:krdp_later",
    ] {
        assert_eq!(LoginState::parse(state).unwrap().to_string(), state);
    }
    assert!(LoginState::parse("unsupported:").is_err());
    assert!(LoginState::parse("other").is_err());
}

/// The variants a host up to 0.5.197 reads (`deny_unknown_fields`); it
/// refuses `rdp_login`, and the viewer then starts as before.
#[allow(dead_code)]
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum HostCommandsUpTo197 {
    Catalog,
    Icon {
        app_id: String,
        icon: serde_json::Value,
        catalog_revision: String,
    },
    Launch {
        request: serde_json::Value,
    },
    Receipt {
        request: serde_json::Value,
    },
    RdpProbe,
    RdpProbeV2,
    RdpPrepare,
    RdpCredential {
        service_id: String,
    },
    LocalServices,
}

#[test]
fn rdp_konto_task_old_host_refuses_rdp_login_viewer_falls_back() {
    let login = AppsCommand::RdpLogin {
        service_id: "windows-rdp".into(),
        status_only: false,
    };
    let wire = serde_json::to_value(&login).unwrap();
    assert_eq!(
        wire,
        serde_json::json!({"kind": "rdp_login", "service_id": "windows-rdp"})
    );
    assert!(serde_json::from_value::<HostCommandsUpTo197>(wire.clone()).is_err());
    let credential = serde_json::json!({"kind": "rdp_credential", "service_id": "gnome-console"});
    assert!(serde_json::from_value::<HostCommandsUpTo197>(credential).is_ok());
    // A host of this version reads the reply without `status_only`.
    let read: AppsCommand = serde_json::from_value(wire).unwrap();
    assert!(matches!(
        read,
        AppsCommand::RdpLogin {
            status_only: false,
            ..
        }
    ));
    // The refusal reaches the viewer as an error: it starts as before.
    let plan = viewer_plan(Err("device command connection closed".into()), true).unwrap();
    assert!(plan.credentials.is_none() && plan.account.is_none());
    assert_eq!(plan.view.state, "unavailable");
    assert!(plan.view.text.contains("0.5.198"), "{}", plan.view.text);
    assert!(
        plan.view.text.contains("connection closed"),
        "{}",
        plan.view.text
    );
    // A new host that needs the password once: the GUI asks, others start.
    let account = HostAccount::new("alice", Some("PC-B")).unwrap();
    let needed = || LoginInfo::new(Some(account.clone()), None, LoginState::EntryNeeded);
    let error = viewer_plan(Ok(needed()), true).err().unwrap();
    assert_eq!(entry_needed_account(&error), Some("PC-B\\alice"));
    let plan = viewer_plan(Ok(needed()), false).unwrap();
    assert!(plan.credentials.is_none());
    assert_eq!(plan.account, Some(account.clone()));
    let stored = LoginInfo::new(
        Some(account.clone()),
        Some(ManagedSecret::new("Geheim-1".into())),
        LoginState::Stored,
    );
    let plan = viewer_plan(Ok(stored), true).unwrap();
    let (user, secret) = plan.credentials.unwrap();
    assert_eq!((user, secret.expose()), (account, "Geheim-1"));
}

/// No Debug form shows a password: the reply, the command that stores it,
/// the checked answer and the credentials a client receives.
#[test]
fn rdp_konto_task_login_info_debug_redacts_secret() {
    let secret = "Geheim-Kennwort-42";
    let info = LoginInfo::new(
        Some(HostAccount::new("alice", Some("PC-B")).unwrap()),
        Some(ManagedSecret::new(secret.into())),
        LoginState::Stored,
    );
    let reply = AppsResult::RdpLoginInfo {
        username: "alice".into(),
        domain: Some("PC-B".into()),
        secret: Some(ManagedSecret::new(secret.into())),
        state: LoginState::Stored,
    };
    let store = AppsCommand::RdpStorePassword {
        service_id: "windows-rdp".into(),
        password: Some(ManagedSecret::new(secret.into())),
    };
    let credentials = crate::rdp::Credentials {
        username: "alice".into(),
        domain: Some("PC-B".into()),
        password: ManagedSecret::new(secret.into()),
    };
    for shown in [
        format!("{info:?}"),
        format!("{reply:?}"),
        format!("{store:?}"),
        format!("{credentials:?}"),
    ] {
        assert!(!shown.contains(secret), "{shown}");
    }
    assert!(format!("{info:?}").contains("<redacted>"));
    // An entry_needed reply carries no password.
    let carried = LoginInfo::from_wire(
        "alice".into(),
        None,
        Some(ManagedSecret::new(secret.into())),
        LoginState::EntryNeeded,
    );
    assert!(carried.is_err());
}

/// Decision 9: FreeRDP receives user, domain and password only on standard
/// input (`/args-from:stdin`); argv names the switch alone.
#[test]
fn rdp_konto_task_freerdp_password_only_on_stdin() {
    let args = vec![
        "/v:127.1.2.3:3390".to_string(),
        "/cert:deny,fingerprint:sha256:00".to_string(),
    ];
    let password = "Geheim 1!";
    let (argv, input) = freerdp_stdin_invocation(&args, "alice", Some("PC-B"), password).unwrap();
    assert_eq!(argv, vec!["/args-from:stdin".to_string()]);
    assert!(argv.iter().all(|arg| !arg.contains(password)));
    let text = String::from_utf8(input.to_vec()).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines,
        vec![
            "/v:127.1.2.3:3390",
            "/cert:deny,fingerprint:sha256:00",
            "/u:alice",
            "/d:PC-B",
            "/p:Geheim 1!"
        ]
    );
    for bad in ["a\nb", "a\rb", "a\0b"] {
        assert!(freerdp_stdin_invocation(&args, "alice", None, bad).is_err());
        assert!(freerdp_stdin_invocation(&args, bad, None, password).is_err());
    }
    // Without a password only user and domain stand in argv.
    let account = HostAccount::new("alice", Some("PC-B")).unwrap();
    assert!(freerdp_account_args(&account)
        .iter()
        .all(|arg| !arg.starts_with("/p:")));
}
