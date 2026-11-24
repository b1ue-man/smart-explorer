//! Signals of the Linux runtime report (DRV-M4): a missing `fusermount3`
//! is reported and installed on use (package fuse3, 2026-11-24), and the
//! sandbox and WSL markers are detected. Both run on a fixture root, never
//! on the runner's own system files; the package step is a stand-in.
use super::{install_with, report, FuseInstallOutcome, RuntimeProbe, Sandbox};
use crate::login::os::shared::install_gate::Attempt;
use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::Path;

fn probe<'a>(root: &'a Path, env: &'a dyn Fn(&str) -> Option<OsString>) -> RuntimeProbe<'a> {
    RuntimeProbe {
        root,
        env,
        version: false,
    }
}

fn environment(values: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let values: HashMap<String, OsString> = values
        .iter()
        .map(|(name, value)| (name.to_string(), OsString::from(value)))
        .collect();
    move |name| values.get(name).cloned()
}

fn write(root: &Path, relative: &str, text: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn drive_task_fuse_runtime_report_missing_fusermount() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    write(root, "dev/fuse", "");
    write(
        root,
        "etc/os-release",
        "NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\n",
    );
    write(root, "proc/sys/fs/fuse/default_request_timeout", "0\n");
    fs::create_dir_all(root.join("opt/empty")).unwrap();
    let env = environment(&[("PATH", "/opt/empty")]);
    let report = report(&probe(root, &env));
    assert!(!report.ready);
    assert_eq!(report.platform, "linux");
    assert!(report.device.accessible, "{report:?}");
    assert_eq!(report.fusermount3.path, None);
    assert!(!report.fusermount3.setuid);
    assert_eq!(report.kernel_request_timeout_s, Some(0));
    assert_eq!(report.kernel_max_request_timeout_s, None);
    assert_eq!(report.sandbox, Sandbox::None);
    assert_eq!(report.problems, ["fusermount3 fehlt (Paket fuse3)"]);
    // Case "install on use": a missing helper runs the package step once;
    // a declined dialog is the answer, exit 2.
    let asked = Cell::new(0);
    let declined = "Die Installation von FUSE (Paket fuse3) wurde in der Systemabfrage abgelehnt.";
    let outcome = install_with(&probe(root, &env), || {
        asked.set(asked.get() + 1);
        Attempt::Declined(declined.into())
    });
    assert_eq!(
        outcome,
        FuseInstallOutcome::Declined {
            reason: declined.into()
        }
    );
    assert_eq!((outcome.exit_code(), asked.get()), (2, 1));
    let json = serde_json::to_value(&outcome).unwrap();
    assert_eq!(json["status"], "declined");
    assert_eq!(json["reason"], declined);
    // An install that leaves FUSE unready reports why, exit 1.
    let outcome = install_with(&probe(root, &env), || Attempt::Installed);
    assert_eq!(
        outcome,
        FuseInstallOutcome::Failed {
            reason: "FUSE ist nach der Installation nicht bereit: fusermount3 fehlt (Paket fuse3)"
                .into()
        }
    );
    assert_eq!(outcome.exit_code(), 1);

    // A helper that is not setuid root is no runtime either; it is found
    // through PATH below the fixture root.
    write(root, "opt/empty/fusermount3", "#!/bin/sh\n");
    let report = super::report(&probe(root, &env));
    assert_eq!(
        report.fusermount3.path.as_deref(),
        Some("/opt/empty/fusermount3")
    );
    assert!(!report.fusermount3.setuid);
    assert!(!report.ready);
    assert!(
        report.problems[0].contains("nicht setuid root"),
        "{report:?}"
    );
    // An installed helper without setuid is no case for the package step.
    let outcome = install_with(&probe(root, &env), || {
        asked.set(asked.get() + 1);
        Attempt::Installed
    });
    assert!(
        matches!(&outcome, FuseInstallOutcome::Failed { reason } if reason.contains("nicht setuid root")),
        "{outcome:?}"
    );
    assert_eq!(asked.get(), 1, "no second package step");

    // Without /dev/fuse the device is the first problem.
    let bare = tempfile::tempdir().unwrap();
    let report = super::report(&probe(bare.path(), &env));
    assert!(!report.device.accessible);
    assert!(
        report.problems[0].starts_with("/dev/fuse fehlt"),
        "{report:?}"
    );
    assert_eq!(report.problems[1], "fusermount3 fehlt (Paket fuse3)");
    // Case "sandbox and WSL markers": each marker names its sandbox, WSL is found by kernel, interop or environment.
    {
        let none = environment(&[]);
        let plain = tempfile::tempdir().unwrap();
        write(
            plain.path(),
            "proc/version",
            "Linux version 6.17.0-1 (buildd@ubuntu)\n",
        );
        let marked = super::report(&probe(plain.path(), &none));
        assert_eq!(marked.sandbox, Sandbox::None, "sandbox markers");
        assert!(!marked.wsl, "sandbox markers");

        let flatpak = tempfile::tempdir().unwrap();
        write(
            flatpak.path(),
            ".flatpak-info",
            "[Application]\nname=org.example\n",
        );
        let report = super::report(&probe(flatpak.path(), &none));
        assert_eq!(report.sandbox, Sandbox::Flatpak, "flatpak");
        assert!(
            report
                .problems
                .iter()
                .any(|problem| problem.starts_with("Flatpak-Sandkasten")),
            "flatpak"
        );
        // A sandbox forbids drives: no package step there.
        let outcome = install_with(&probe(flatpak.path(), &none), || {
            panic!("no package step in a sandbox")
        });
        assert!(
            matches!(&outcome, FuseInstallOutcome::Failed { reason } if reason.contains("Flatpak-Sandkasten")),
            "flatpak install"
        );

        let snap_env = environment(&[("SNAP", "/snap/smart-explorer/1")]);
        let report = super::report(&probe(plain.path(), &snap_env));
        assert_eq!(report.sandbox, Sandbox::Snap, "snap");
        assert!(
            report
                .problems
                .iter()
                .any(|problem| problem.starts_with("Snap-Sandkasten")),
            "snap"
        );

        let docker = tempfile::tempdir().unwrap();
        write(docker.path(), ".dockerenv", "");
        assert_eq!(
            super::report(&probe(docker.path(), &none)).sandbox,
            Sandbox::Container,
            "docker container"
        );
        let podman = tempfile::tempdir().unwrap();
        write(podman.path(), "run/.containerenv", "");
        assert_eq!(
            super::report(&probe(podman.path(), &none)).sandbox,
            Sandbox::Container,
            "podman container"
        );
        let container_env = environment(&[("container", "lxc")]);
        assert_eq!(
            super::report(&probe(plain.path(), &container_env)).sandbox,
            Sandbox::Container,
            "container environment"
        );

        // WSL through the kernel version, the interop file or the environment.
        let wsl = tempfile::tempdir().unwrap();
        write(
            wsl.path(),
            "proc/version",
            "Linux version 6.6.36.6-microsoft-standard-WSL2\n",
        );
        assert!(super::report(&probe(wsl.path(), &none)).wsl, "WSL kernel");
        let interop = tempfile::tempdir().unwrap();
        write(
            interop.path(),
            "proc/sys/fs/binfmt_misc/WSLInterop",
            "enabled\n",
        );
        assert!(
            super::report(&probe(interop.path(), &none)).wsl,
            "WSL interop"
        );
        let wsl_env = environment(&[("WSL_DISTRO_NAME", "Ubuntu")]);
        assert!(
            super::report(&probe(plain.path(), &wsl_env)).wsl,
            "WSL environment"
        );
        let json = serde_json::to_value(super::report(&probe(flatpak.path(), &none))).unwrap();
        assert_eq!(json["sandbox"], "flatpak", "json sandbox");
        assert_eq!(json["wsl"], false, "json wsl");
    }
}
