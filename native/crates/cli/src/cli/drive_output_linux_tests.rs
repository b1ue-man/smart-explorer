//! Signal `drive_task_cli_linux_shapes` (DRV-M4): the text and JSON forms
//! of `se drive runtime`, `install-runtime`, `list` and `mount` under Linux,
//! the refusal of Windows-only options, and the new arguments.
use super::super::drive_options::MountArgs;
use super::super::drive_print::status_text;
use super::super::{snapshot_line, UnmountArgs};
use super::{
    folder_column, install_text, refuse_windows_options, runtime_text, LETTERS_ONLY_ON_WINDOWS,
};
use crate::mount::os::linux_os::fuse_runtime::{FuseDevice, FuseHelper, Sandbox};
use crate::mount::{
    BackendRoot, DriveSelection, FuseInstallOutcome, FuseRuntimeReport, MountConfig, MountId,
    MountMode, MountRecovery, MountSnapshot, MountSource, MountStatus,
};
use clap::Parser;
use std::path::PathBuf;

fn report(ready: bool) -> FuseRuntimeReport {
    FuseRuntimeReport {
        platform: "linux",
        ready,
        device: FuseDevice {
            path: "/dev/fuse".into(),
            accessible: true,
        },
        fusermount3: FuseHelper {
            path: Some("/usr/bin/fusermount3".into()),
            setuid: ready,
            version: Some("3.14.0".into()),
        },
        kernel_request_timeout_s: Some(0),
        kernel_max_request_timeout_s: Some(0),
        sandbox: Sandbox::None,
        wsl: false,
        problems: if ready {
            Vec::new()
        } else {
            vec!["/usr/bin/fusermount3 ist nicht setuid root".into()]
        },
    }
}

fn snapshot(status: MountStatus, folder: Option<&str>) -> MountSnapshot {
    let config = MountConfig::new(
        MountId::parse("0123456789abcdef").unwrap(),
        MountSource::SavedRemote {
            account: "sftp:prod".into(),
            root: BackendRoot::parse("/srv").unwrap(),
        },
        DriveSelection::Automatic,
        MountMode::ReadWrite,
        "prod",
    )
    .unwrap()
    .with_location_directory(folder.map(PathBuf::from))
    .unwrap();
    MountSnapshot {
        capacity: None,
        config,
        status,
        recovery: MountRecovery::Clean,
        recovery_required_compat: false,
    }
}

#[derive(Parser)]
struct MountCommand {
    #[command(flatten)]
    mount: MountArgs,
}

#[derive(Parser)]
struct UnmountCommand {
    #[command(flatten)]
    unmount: UnmountArgs,
}

#[test]
fn drive_task_cli_linux_shapes() {
    // runtime: the contract line, a not-ready report with its problems, hints.
    assert_eq!(
        runtime_text(&report(true)),
        "fuse\tready\tfusermount3=/usr/bin/fusermount3\tversion=3.14.0\tsetuid=yes\tdevice=/dev/fuse\tkernel_request_timeout=0"
    );
    let not_ready = runtime_text(&report(false));
    let lines: Vec<_> = not_ready.lines().collect();
    assert_eq!(
        lines[0],
        "fuse\tnot-ready\tfusermount3=/usr/bin/fusermount3\tsetuid=no\tdevice=/dev/fuse\taccessible=yes\tkernel_request_timeout=0"
    );
    assert_eq!(
        lines[1],
        "fuse\tproblem\t/usr/bin/fusermount3 ist nicht setuid root"
    );
    let mut limited = report(true);
    limited.kernel_request_timeout_s = Some(15);
    limited.wsl = true;
    let text = runtime_text(&limited);
    assert!(text.contains("kernel_request_timeout=15"), "{text}");
    assert!(text.contains("fuse\thint\tDer Kernel bricht ein Laufwerk ab, wenn eine Anfrage länger als 15 s dauert"), "{text}");
    assert!(text.contains("/mnt/wsl"), "{text}");
    let json = serde_json::to_value(report(true)).unwrap();
    for key in [
        "platform",
        "ready",
        "device",
        "fusermount3",
        "kernel_request_timeout_s",
        "kernel_max_request_timeout_s",
        "sandbox",
        "wsl",
        "problems",
    ] {
        assert!(json.get(key).is_some(), "missing {key}: {json}");
    }
    assert_eq!(json["platform"], "linux");
    assert_eq!(json["device"]["path"], "/dev/fuse");
    assert_eq!(json["fusermount3"]["setuid"], true);
    assert_eq!(json["sandbox"], "none");

    // install-runtime: text and JSON of both answers.
    let manual = FuseInstallOutcome::ManualRequired {
        command: "sudo apt-get install fuse3".into(),
        reason: "fusermount3 fehlt".into(),
    };
    assert_eq!(
        install_text(&manual),
        "fuse\tmanual-required\tsudo apt-get install fuse3\tfusermount3 fehlt"
    );
    assert_eq!(manual.exit_code(), 2);
    assert_eq!(
        install_text(&FuseInstallOutcome::AlreadyReady),
        "fuse\talready-ready"
    );
    assert_eq!(FuseInstallOutcome::AlreadyReady.exit_code(), 0);
    assert_eq!(
        serde_json::to_value(FuseInstallOutcome::AlreadyReady).unwrap()["status"],
        "already_ready"
    );

    // mount: letters and the system Dokany DLL exist only under Windows.
    assert_eq!(refuse_windows_options("auto", false), None);
    assert_eq!(refuse_windows_options(" AUTO ", false), None);
    assert_eq!(
        refuse_windows_options("M", false),
        Some(LETTERS_ONLY_ON_WINDOWS)
    );
    assert_eq!(
        refuse_windows_options("auto", true),
        Some(LETTERS_ONLY_ON_WINDOWS)
    );
    let parsed = MountCommand::try_parse_from([
        "mount",
        "@prod:/srv",
        "--mount-point",
        "/home/u/SmartExplorer/prod",
        "--read-write",
    ])
    .unwrap();
    assert_eq!(
        parsed.mount.mount_point.as_deref(),
        Some(std::path::Path::new("/home/u/SmartExplorer/prod"))
    );
    assert!(MountCommand::try_parse_from(["mount", "@prod:/srv"])
        .unwrap()
        .mount
        .mount_point
        .is_none());
    let unmount =
        UnmountCommand::try_parse_from(["unmount", "prod-id", "--force", "--json"]).unwrap();
    assert!(unmount.unmount.force && unmount.unmount.json);
    assert_eq!(unmount.unmount.selector, "prod-id");

    // list: column 3 is the folder, the status names it.
    let folder = "/home/u/SmartExplorer/prod";
    let mounted = snapshot(
        MountStatus::MountedAt {
            path: PathBuf::from(folder),
        },
        Some(folder),
    );
    assert_eq!(folder_column(&mounted), folder);
    assert_eq!(status_text(&mounted.status), format!("mounted:{folder}"));
    assert_eq!(
        snapshot_line(&mounted),
        format!("drive\t0123456789abcdef\t{folder}\tread-write\tmounted:{folder}\tprod")
    );
    let waiting = snapshot(MountStatus::Mounting, None);
    assert_eq!(folder_column(&waiting), "-");
    assert_eq!(
        snapshot_line(&waiting),
        "drive\t0123456789abcdef\t-\tread-write\tmounting\tprod"
    );
    let stored = snapshot(MountStatus::Failed { detail: "x".into() }, Some(folder));
    assert_eq!(folder_column(&stored), folder);
}
