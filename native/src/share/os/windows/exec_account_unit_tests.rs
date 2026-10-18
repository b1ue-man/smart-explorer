//! Pure checks of the Windows account service (PRM-M7) that need neither
//! administrator rights nor the service: the configuration of D.2.8, the
//! words of the elevated setup (D.2.4), the exact job pipe names the
//! service opens and the access lists of the pipe and the service.
use std::ffi::OsString;
use std::path::Path;

use super::exec_account_config::{allowable_name, ServiceConfig};
use super::exec_account_pipes::{job_pipe_id, job_pipe_name, CLIENT_ACCESS};
use super::exec_account_scm::service_sddl;
use super::exec_account_security::{sid_text, SecurityDescriptor};
use super::exec_account_service::pipe_sddl;
use super::exec_account_setup::{
    admin_command, mode_arguments, parse_mode, parse_words, ExecAccountsAction,
};

const SID: &str = "S-1-5-21-1004336348-1177238915-682003330-1001";
const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn windows_account_config_reads_only_valid_files() {
    let text =
        format!(r#"{{"version":1,"users":[{{"sid":"{SID}","accounts":["se-remote","Deploy"]}}]}}"#);
    let config = ServiceConfig::parse(text.as_bytes()).unwrap();
    assert!(config.has_user(SID));
    assert!(
        config.allows(SID, "SE-REMOTE"),
        "names do not depend on case"
    );
    assert!(config.allows(SID, "deploy"));
    assert!(!config.allows(SID, "other"));
    assert!(!config.allows("S-1-5-21-1-2-3-1002", "se-remote"));
    assert_eq!(ServiceConfig::parse(&config.render()).unwrap(), config);
    for invalid in [
        r#"{"version":2,"users":[]}"#.to_string(),
        r#"{"version":1,"users":[{"sid":"S-1-x","accounts":[]}]}"#.to_string(),
        format!(r#"{{"version":1,"users":[{{"sid":"{SID}"}},{{"sid":"{SID}"}}]}}"#),
        format!(r#"{{"version":1,"users":[{{"sid":"{SID}","accounts":["Administrator"]}}]}}"#),
        format!(r#"{{"version":1,"users":[{{"sid":"{SID}","accounts":["a b"]}}]}}"#),
        "not json".to_string(),
    ] {
        assert!(
            ServiceConfig::parse(invalid.as_bytes()).is_err(),
            "{invalid}"
        );
    }
    let mut changed = ServiceConfig::new();
    assert!(changed.allow(SID, "se-remote").unwrap());
    assert!(!changed.allow(SID, "SE-Remote").unwrap());
    for never in ["Administrator", "SYSTEM", "Guest", "Network Service"] {
        assert!(!allowable_name(never), "{never}");
        assert!(changed.allow(SID, never).is_err(), "{never}");
    }
    assert!(changed.add_user("not-a-sid").is_err());
    assert!(changed.disallow(SID, "SE-REMOTE"));
    assert!(!changed.disallow(SID, "se-remote"));
    assert!(changed.has_user(SID));
    assert_eq!(changed.user_sids(), vec![SID]);
}

#[test]
fn windows_account_setup_words_round_trip() {
    let actions = [
        ExecAccountsAction::Setup {
            remote_account: true,
            allow: vec!["deploy".into()],
        },
        ExecAccountsAction::Allow {
            account: "deploy".into(),
        },
        ExecAccountsAction::Disallow {
            account: "deploy".into(),
        },
        ExecAccountsAction::Update,
        ExecAccountsAction::Remove {
            delete_remote_account: true,
        },
    ];
    for action in actions {
        let arguments: Vec<OsString> = mode_arguments(&action, SID, HASH)
            .into_iter()
            .map(OsString::from)
            .collect();
        let parsed = parse_mode(&arguments).unwrap();
        assert_eq!(parsed, (action.clone(), SID.to_string(), HASH.to_string()));
    }
    let setup = ExecAccountsAction::Setup {
        remote_account: true,
        allow: Vec::new(),
    };
    assert_eq!(
        admin_command(
            Path::new(r"C:\Users\o'neil\AppData\Local\Programs\Smart Explorer\se.exe"),
            &setup
        ),
        r"& 'C:\Users\o''neil\AppData\Local\Programs\Smart Explorer\se.exe' share exec accounts setup --remote-account --yes"
    );
    // `--yes` belongs to setup only (D.2.4).
    assert!(parse_words(&["allow", "deploy", "--yes"]).is_err());
    assert!(parse_words(&["update", "--yes"]).is_err());
    let refused: [&[&str]; 5] = [
        &[
            "--exec-accounts-setup",
            "--for-sid",
            SID,
            "--expect-sha256",
            HASH,
            "remove",
        ],
        &[
            "--exec-accounts-remove",
            "--for-sid",
            SID,
            "--expect-sha256",
            HASH,
            "--allow",
            "x",
        ],
        &[
            "--exec-accounts-setup",
            "--for-sid",
            "S-1-x",
            "--expect-sha256",
            HASH,
            "update",
        ],
        &[
            "--exec-accounts-setup",
            "--for-sid",
            SID,
            "--expect-sha256",
            "12",
            "update",
        ],
        &["--exec-accounts-setup", "--for-sid", SID, "update"],
    ];
    for words in refused {
        let arguments: Vec<OsString> = words.iter().copied().map(OsString::from).collect();
        assert!(parse_mode(&arguments).is_err(), "{arguments:?}");
    }
}

#[test]
fn windows_account_job_pipes_are_exact() {
    let id = "0123456789abcdef0123456789abcdef";
    let pipe_in = job_pipe_name(id, "in");
    assert_eq!(pipe_in, format!(r"\\.\pipe\SmartExplorerExec-{id}-in"));
    assert_eq!(job_pipe_id(&pipe_in, "in"), Some(id));
    assert_eq!(job_pipe_id(&job_pipe_name(id, "out"), "out"), Some(id));
    for name in [
        job_pipe_name(id, "out"),
        job_pipe_name(&id.to_uppercase(), "in"),
        job_pipe_name(&id[1..], "in"),
        format!(r"\\.\pipe\SmartExplorerExec-{id}-in-"),
        format!(r"\\server\pipe\SmartExplorerExec-{id}-in"),
        format!(r"C:\Windows\System32\config\SAM-{id}-in"),
        format!(r"\\.\pipe\SmartExplorerExec-..\{id}-in"),
    ] {
        assert_eq!(job_pipe_id(&name, "in"), None, "{name}");
    }
}

#[test]
fn windows_account_access_lists() {
    // Users read and write data but may never add a pipe instance
    // (`FILE_CREATE_PIPE_INSTANCE` = `FILE_APPEND_DATA` = 4).
    assert_eq!(CLIENT_ACCESS & 4, 0);
    let users = vec![SID.to_string()];
    let pipe = pipe_sddl(&users);
    assert!(pipe.starts_with("O:SYG:SYD:P(A;;GA;;;SY)"), "{pipe}");
    assert!(
        pipe.ends_with(&format!("(A;;{CLIENT_ACCESS:#x};;;{SID})")),
        "{pipe}"
    );
    assert!(SecurityDescriptor::from_sddl(&pipe).is_ok());
    let service = service_sddl(&[SID]);
    assert!(
        service.ends_with(&format!("(A;;LCRP;;;{SID})")),
        "{service}"
    );
    assert!(SecurityDescriptor::from_sddl(&service).is_ok());
    assert!(sid_text(SID));
    for text in ["S-1-", "S-1-5-", "S-1-5-21-x", "s-1-5-18", "S-1-5--18"] {
        assert!(!sid_text(text), "{text}");
    }
}
