//! Signals of the Linux runtime report (DRV-M4): a missing `fusermount3`
//! is reported with the package command of the distribution, and the
//! sandbox and WSL markers are detected. Both run on a fixture root, never
//! on the runner's own system files.
use super::{install_outcome, package_command, report, FuseInstallOutcome, RuntimeProbe, Sandbox};
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
    assert_eq!(
        report.problems,
        ["fusermount3 fehlt; installieren mit: sudo apt-get install fuse3"]
    );
    let outcome = install_outcome(&probe(root, &env));
    assert_eq!(
        outcome,
        FuseInstallOutcome::ManualRequired {
            command: "sudo apt-get install fuse3".into(),
            reason: "fusermount3 fehlt; installieren mit: sudo apt-get install fuse3".into(),
        }
    );
    assert_eq!(outcome.exit_code(), 2);
    let json = serde_json::to_value(&outcome).unwrap();
    assert_eq!(json["status"], "manual_required");
    assert_eq!(json["command"], "sudo apt-get install fuse3");

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

    // Without /dev/fuse the device is the first problem.
    let bare = tempfile::tempdir().unwrap();
    let report = super::report(&probe(bare.path(), &env));
    assert!(!report.device.accessible);
    assert!(
        report.problems[0].starts_with("/dev/fuse fehlt"),
        "{report:?}"
    );
    assert_eq!(
        report.problems[1],
        "fusermount3 fehlt; installieren mit: das Paket fuse3 mit der Paketverwaltung installieren"
    );

    // Package commands of the distributions the plan names.
    for (release, command) in [
        ("ID=fedora\n", "sudo dnf install fuse3"),
        ("ID=arch\n", "sudo pacman -S fuse3"),
        (
            "ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n",
            "sudo zypper install fuse3",
        ),
        ("ID=debian\n", "sudo apt-get install fuse3"),
    ] {
        let distribution = tempfile::tempdir().unwrap();
        write(distribution.path(), "etc/os-release", release);
        assert_eq!(package_command(distribution.path()), command, "{release}");
    }
}

#[test]
fn drive_task_runtime_detects_sandbox_markers() {
    let none = environment(&[]);
    let plain = tempfile::tempdir().unwrap();
    write(
        plain.path(),
        "proc/version",
        "Linux version 6.17.0-1 (buildd@ubuntu)\n",
    );
    let report = report(&probe(plain.path(), &none));
    assert_eq!(report.sandbox, Sandbox::None);
    assert!(!report.wsl);

    let flatpak = tempfile::tempdir().unwrap();
    write(
        flatpak.path(),
        ".flatpak-info",
        "[Application]\nname=org.example\n",
    );
    let report = super::report(&probe(flatpak.path(), &none));
    assert_eq!(report.sandbox, Sandbox::Flatpak);
    assert!(report
        .problems
        .iter()
        .any(|problem| problem.starts_with("Flatpak-Sandkasten")));

    let snap_env = environment(&[("SNAP", "/snap/smart-explorer/1")]);
    let report = super::report(&probe(plain.path(), &snap_env));
    assert_eq!(report.sandbox, Sandbox::Snap);
    assert!(report
        .problems
        .iter()
        .any(|problem| problem.starts_with("Snap-Sandkasten")));

    let docker = tempfile::tempdir().unwrap();
    write(docker.path(), ".dockerenv", "");
    assert_eq!(
        super::report(&probe(docker.path(), &none)).sandbox,
        Sandbox::Container
    );
    let podman = tempfile::tempdir().unwrap();
    write(podman.path(), "run/.containerenv", "");
    assert_eq!(
        super::report(&probe(podman.path(), &none)).sandbox,
        Sandbox::Container
    );
    let container_env = environment(&[("container", "lxc")]);
    assert_eq!(
        super::report(&probe(plain.path(), &container_env)).sandbox,
        Sandbox::Container
    );

    // WSL through the kernel version, the interop file or the environment.
    let wsl = tempfile::tempdir().unwrap();
    write(
        wsl.path(),
        "proc/version",
        "Linux version 6.6.36.6-microsoft-standard-WSL2\n",
    );
    assert!(super::report(&probe(wsl.path(), &none)).wsl);
    let interop = tempfile::tempdir().unwrap();
    write(
        interop.path(),
        "proc/sys/fs/binfmt_misc/WSLInterop",
        "enabled\n",
    );
    assert!(super::report(&probe(interop.path(), &none)).wsl);
    let wsl_env = environment(&[("WSL_DISTRO_NAME", "Ubuntu")]);
    assert!(super::report(&probe(plain.path(), &wsl_env)).wsl);
    let json = serde_json::to_value(super::report(&probe(flatpak.path(), &none))).unwrap();
    assert_eq!(json["sandbox"], "flatpak");
    assert_eq!(json["wsl"], false);
}
