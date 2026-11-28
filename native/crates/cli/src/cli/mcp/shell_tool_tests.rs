//! The place of a `shell` command for every kind of working root, and its
//! description and parameters.
use super::exec_tool::RemoteHome;
use super::root_path::{RootKind, WorkRoot};
use super::shell_tool::{place_for_tests as place, properties, DESCRIPTION};
use crate::creds::Protocol;

fn root(spec: &str, kind: RootKind, path: &str) -> WorkRoot {
    WorkRoot {
        spec: spec.into(),
        kind,
        path: path.into(),
    }
}

#[test]
fn shell_elevate_task_without_root_the_command_runs_here() {
    assert_eq!(place(None, None).unwrap(), (None, None));
    assert_eq!(
        place(None, Some("/tmp".into())).unwrap(),
        (Some("/tmp".to_string()), None)
    );
}

#[test]
fn shell_elevate_task_local_root_is_the_folder_and_relative_cwd_stays_below() {
    let base = std::env::temp_dir();
    let spec = base.to_string_lossy().into_owned();
    let local = root(&spec, RootKind::Local, &spec);
    assert_eq!(
        place(Some(&local), None).unwrap(),
        (Some(spec.clone()), None)
    );
    let (cwd, _) = place(Some(&local), Some("src/./lib".into())).unwrap();
    assert_eq!(
        std::path::PathBuf::from(cwd.unwrap()),
        base.join("src").join("lib")
    );
    assert!(place(Some(&local), Some("../..".into())).is_err());
}

#[test]
fn shell_elevate_task_remote_roots_run_there_and_file_roots_refuse() {
    let ssh = root(
        "@prod:/srv/app",
        RootKind::Login(Protocol::Sftp),
        "/srv/app",
    );
    assert_eq!(
        place(Some(&ssh), Some("bin".into())).unwrap(),
        (
            None,
            Some(RemoteHome {
                selector: "@prod:/srv/app".into(),
                cwd: Some("/srv/app/bin".into()),
                share_cwd: None,
            })
        )
    );
    let peer = root("share://direct/c1/Docs/proj", RootKind::Peer, "/Docs/proj");
    assert_eq!(
        place(Some(&peer), None)
            .unwrap()
            .1
            .unwrap()
            .share_cwd
            .as_deref(),
        Some("/Docs/proj")
    );
    for (spec, kind) in [
        ("gdrive:///Docs", RootKind::Drive),
        ("vault://site/x", RootKind::Vault),
        ("@ftp:/pub", RootKind::Login(Protocol::Ftp)),
        ("@dav:/x", RootKind::Login(Protocol::Webdav)),
    ] {
        let refused = place(Some(&root(spec, kind, "/")), None).unwrap_err();
        assert!(refused.contains("no shell"), "{spec}: {refused}");
    }
}

#[test]
fn shell_elevate_task_tool_names_every_field_of_the_package() {
    let properties = properties();
    for field in [
        "command",
        "argv",
        "shell",
        "cwd",
        "env",
        "stdin",
        "keep_stdin_open",
        "timeout_seconds",
        "session",
        "attachable",
        "elevated",
        "wait_seconds",
        "max_bytes",
    ] {
        assert!(properties.get(field).is_some(), "{field}");
    }
    for word in [
        "elevated",
        "UAC",
        "polkit",
        "sudo",
        "working root",
        "placeholders",
    ] {
        assert!(DESCRIPTION.contains(word), "{word}");
    }
}
