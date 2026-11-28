use super::*;
use crate::share::exec_types::ExecId;
use crate::share::export_config::SharedRoot;

fn start(command: ExecCommand) -> ExecStart {
    ExecStart {
        exec_id: ExecId::parse("5e".repeat(16)).unwrap(),
        command,
        cwd: None,
        env: BTreeMap::new(),
        timeout_ms: None,
        max_output_bytes: None,
        terminal: None,
        shell: None,
        share_cwd: None,
        elevated: false,
    }
}

fn shell(command: &str) -> ExecStart {
    start(ExecCommand::Shell {
        command: command.into(),
    })
}

#[test]
fn shell_elevate_task_new_start_fields_keep_old_bytes_and_are_checked() {
    let plain = shell("true");
    let encoded = serde_json::to_string(&plain).unwrap();
    assert!(
        !encoded.contains("share_cwd") && !encoded.contains("elevated"),
        "{encoded}"
    );
    let mut both = shell("true");
    both.share_cwd = Some("/Docs/proj".into());
    both.elevated = true;
    let encoded = serde_json::to_string(&both).unwrap();
    assert!(
        encoded.contains("\"share_cwd\":\"/Docs/proj\"") && encoded.contains("\"elevated\":true")
    );
    let decoded: ExecStart = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, both);
    assert!(both.validate().is_ok());
    let mut twice = both.clone();
    twice.cwd = Some("/srv".into());
    assert!(twice.validate().is_err());
    for invalid in ["Docs", "/", "//"] {
        let mut wrong = shell("true");
        wrong.share_cwd = Some(invalid.into());
        assert!(wrong.validate().is_err(), "{invalid}");
    }
}

#[test]
fn shell_elevate_task_client_refuses_options_an_old_host_does_not_announce() {
    let mut with_cwd = shell("true");
    with_cwd.share_cwd = Some("/Docs".into());
    let old: Vec<String> = vec!["exec_shell_v1".into()];
    let refused = require_host_options(&with_cwd, &old).unwrap_err();
    assert_eq!(refused.code, OPTION_UNSUPPORTED);
    assert!(refused.message.contains("update it"), "{}", refused.message);
    let mut elevated = shell("true");
    elevated.elevated = true;
    assert!(require_host_options(&elevated, &old).is_err());
    let new = host_features();
    assert!(new.iter().any(|feature| feature == EXEC_SHARE_CWD_FEATURE));
    assert!(require_host_options(&with_cwd, &new).is_ok());
    assert!(require_host_options(&shell("true"), &old).is_ok());
    if matches!(std::env::consts::OS, "linux" | "windows") {
        assert!(require_host_options(&elevated, &new).is_ok());
    }
}

#[test]
fn shell_elevate_task_share_folder_becomes_the_disk_folder_inside_the_share() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("proj/src")).unwrap();
    std::fs::write(root.path().join("file.txt"), b"x").unwrap();
    let exports = || ShareExportConfig {
        roots: vec![SharedRoot::new("Docs", root.path().to_string_lossy())],
        ..Default::default()
    };
    let disk = share_folder_on_disk("/Docs/proj/src", exports()).unwrap();
    assert_eq!(
        std::fs::canonicalize(&disk).unwrap(),
        std::fs::canonicalize(root.path().join("proj/src")).unwrap()
    );
    for refused in [
        "/Docs/../..",
        "/Other/proj",
        "/Docs/file.txt",
        "/Docs/missing",
    ] {
        let error = share_folder_on_disk(refused, exports()).unwrap_err();
        assert_eq!(error.code, SHARE_CWD_INVALID, "{refused}");
    }
}

#[test]
fn shell_elevate_task_host_wraps_an_elevated_start_into_se_elevated_run() {
    let mut environment = BTreeMap::new();
    environment.insert("HOME".to_string(), "/home/peer".to_string());
    environment.insert("SHELL".to_string(), "/bin/bash".to_string());
    let mut request = shell("id -u");
    request.elevated = true;
    request.env.insert("MODE".into(), "x".into());
    wrap_elevated(&mut request, &environment, None, "linux").unwrap();
    let ExecCommand::Argv { program, args } = &request.command else {
        panic!("argv");
    };
    assert!(program.ends_with("se"), "{program}");
    assert_eq!(
        args,
        &[
            "elevated",
            "run",
            "--cwd",
            "/home/peer",
            "--env",
            "MODE=x",
            "--",
            "/bin/bash",
            "-c",
            "id -u"
        ]
        .map(String::from)
    );
    assert!(!request.elevated && request.shell.is_none());
    // The wrapper's arguments are the helper's own grammar.
    #[cfg(not(target_os = "android"))]
    {
        let call = crate::elevate::helper_args::parse(&args[2..]).unwrap();
        assert_eq!(call.spec.cwd.as_deref(), Some("/home/peer"));
    }

    let mut windows = shell("dir");
    windows.elevated = true;
    windows.cwd = Some(r"C:\Work".into());
    wrap_elevated(&mut windows, &BTreeMap::new(), None, "windows").unwrap();
    let ExecCommand::Argv { program, args } = &windows.command else {
        panic!("argv");
    };
    assert!(program.ends_with("se.exe"), "{program}");
    assert_eq!(
        args,
        &["elevated", "run", "--cwd", r"C:\Work", "--cmd-line", "dir"].map(String::from)
    );

    let mut untouched = shell("true");
    wrap_elevated(&mut untouched, &environment, None, "linux").unwrap();
    assert_eq!(untouched, shell("true"));
    let mut other_account = shell("true");
    other_account.elevated = true;
    let account = RunAsAccount {
        name: "builder".into(),
        uid: 1001,
        gid: 1001,
        groups: Vec::new(),
        home: "/home/builder".into(),
        shell: "/bin/sh".into(),
    };
    assert!(wrap_elevated(&mut other_account, &environment, Some(&account), "linux").is_err());
}
