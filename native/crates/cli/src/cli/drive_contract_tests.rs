//! Preservation contracts of the drive commands (DRV-M10, "Bestandsverträge"
//! of strand S2): the spelling of the stage names a mounted save creates and
//! the JSON form of `MountSnapshot`. Other code reads both (the drive hides
//! the stage names, sync walks skip them; `se drive … --json`, the daemon and
//! an older GUI exchange the JSON), so every expectation below is the form of
//! the current release: a change needs a decision about each reader.
//!
//! - `drive_task_stage_names_se_mount_prefix_unchanged` runs the real mount
//!   engine over the mutable test remote and looks at the names it makes the
//!   remote hold.
//! - `drive_task_mount_snapshot_json_unchanged` compares the compact JSON of
//!   every state of a drive with a fixed text and reads the text back.
use std::path::PathBuf;
use std::sync::Arc;

use crate::mount::optimization_fixture::{FixtureDirectory, OptimizationBackend};
use crate::mount::{
    BackendRoot, DriveLetter, DriveSelection, FlushOutcome, HandleId, MountCachePolicy,
    MountConfig, MountEngine, MountId, MountMode, MountRecovery, MountRuntimeConfig, MountSnapshot,
    MountSource, MountStatus, OpenDisposition, OpenFileOptions,
};
use crate::vfs::{is_staging_name, Backend};

fn new_engine(directory: &FixtureDirectory, backend: &Arc<OptimizationBackend>) -> MountEngine {
    let config =
        MountRuntimeConfig::new(MountId::parse("stage-names").unwrap(), MountMode::ReadWrite)
            .with_cache_policy(MountCachePolicy::new(0).unwrap());
    let engine = MountEngine::open_host_cache(config, backend.clone(), directory.path()).unwrap();
    engine.prepare_host_remote().unwrap();
    engine
}

fn open_writable(engine: &MountEngine, path: &str) -> HandleId {
    engine
        .open_file(
            path,
            OpenFileOptions {
                writable: true,
                disposition: OpenDisposition::OpenExisting,
            },
        )
        .unwrap()
}

/// The names in the root of the remote, in the order of its paths.
fn names(backend: &OptimizationBackend) -> Vec<String> {
    backend
        .list_dir("/")
        .unwrap()
        .into_iter()
        .map(|meta| meta.name)
        .collect()
}

/// `<name>.se-<purpose>-<16 lower-case hex digits>`.
fn is_stage_of(candidate: &str, name: &str, purpose: &str) -> bool {
    candidate
        .strip_prefix(&format!("{name}.se-{purpose}-"))
        .is_some_and(|nonce| {
            nonce.len() == 16
                && nonce
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        })
}

#[test]
fn drive_task_stage_names_se_mount_prefix_unchanged() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/Scan.pdf", b"old");
    let engine = Arc::new(new_engine(&directory, &backend));

    // A save. Its stage exists from the exclusive creation until the
    // promotion; the gate holds the conflict check that follows the upload
    // (the check before the upload is the first stat of the file, so one is
    // skipped).
    let handle = open_writable(&engine, "\\Scan.pdf");
    engine.write(handle, 0, b"new content").unwrap();
    let gate = backend.gate_stat("/Scan.pdf", 1);
    let flushing = Arc::clone(&engine);
    let flusher = std::thread::spawn(move || flushing.flush(handle));
    gate.wait();
    let while_uploaded = names(&backend);
    gate.release();
    let outcome = flusher.join().expect("flush thread").expect("flush");
    assert_eq!(outcome, FlushOutcome::Committed);
    assert_eq!(
        while_uploaded.len(),
        2,
        "the stage of the save is missing while it is uploaded: {while_uploaded:?}"
    );
    assert_eq!(while_uploaded[0], "Scan.pdf");
    let stage = while_uploaded[1].clone();
    assert!(
        is_stage_of(&stage, "Scan.pdf", "mount"),
        "the stage of a save is `<name>.se-mount-<16 hex>`: {stage}"
    );
    assert!(is_staging_name(&stage), "sync walks skip {stage}");
    // The save ends under the user's name and leaves nothing under the stage
    // spelling.
    assert_eq!(names(&backend), ["Scan.pdf"]);
    assert_eq!(backend.bytes("/Scan.pdf"), b"new content");
    engine.close(handle).unwrap();

    // A delete that is not committed yet keeps the file in a quarantine.
    backend.put("/Alt.txt", b"alt");
    let token = engine.begin_delete("\\Alt.txt", false).unwrap();
    let pending = names(&backend);
    assert_eq!(pending.len(), 2, "{pending:?}");
    let quarantine = pending[0].clone();
    assert!(
        is_stage_of(&quarantine, "Alt.txt", "mount-delete"),
        "the quarantine of a delete is `<name>.se-mount-delete-<16 hex>`: {pending:?}"
    );
    assert_eq!(pending[1], "Scan.pdf");
    assert!(is_staging_name(&quarantine), "sync walks skip {quarantine}");

    // The reading side knows exactly these spellings: a drive over the same
    // remote lists neither a stage a crash left behind nor the quarantine.
    backend.put(&format!("/{stage}"), b"left behind by a crash");
    assert_eq!(names(&backend).len(), 3, "{:?}", names(&backend));
    let reader_directory = FixtureDirectory::new();
    let reader = new_engine(&reader_directory, &backend);
    let listed: Vec<String> = reader
        .list_dir("\\")
        .unwrap()
        .into_iter()
        .map(|meta| meta.name)
        .collect();
    assert_eq!(
        listed,
        ["Scan.pdf"],
        "the drive lists a stage or a quarantine"
    );

    // Cancelling the delete gives the file its name back and takes the
    // quarantine spelling off the remote.
    engine.cancel_delete(token).unwrap();
    let restored = names(&backend);
    assert!(
        restored.iter().any(|name| name == "Alt.txt"),
        "{restored:?}"
    );
    assert!(
        restored.iter().all(|name| *name != quarantine),
        "{restored:?}"
    );
}

const SAVED_REMOTE: &str = concat!(
    r#"{"id":"snapshot-fixture","#,
    r#""source":{"SavedRemote":{"account":"sftp:prod","root":"/srv/data"}},"#,
    r#""drive":{"Letter":"M"},"mode":"ReadWrite","root_security":"enforced","#,
    r#""metadata":{"preload_depth":2},"cache":{"retained_mib":500},"#,
    r#""runtime_preference":"auto","label":"Projekte"}"#
);

#[cfg(windows)]
const FOLDER: &str = "C:/Users/u/SmartExplorer/Geräte";
#[cfg(not(windows))]
const FOLDER: &str = "/home/u/SmartExplorer/Geräte";

fn saved_remote() -> MountConfig {
    MountConfig::new(
        MountId::parse("snapshot-fixture").unwrap(),
        MountSource::SavedRemote {
            account: "sftp:prod".into(),
            root: BackendRoot::parse("/srv/data").unwrap(),
        },
        DriveSelection::Letter(DriveLetter::parse('m').unwrap()),
        MountMode::ReadWrite,
        "Projekte",
    )
    .unwrap()
}

/// The device tree as a folder drive: the two optional keys of the
/// configuration, `location` and `auto_remount`, are present.
fn device_tree() -> MountConfig {
    MountConfig::new(
        MountId::parse("se-devices").unwrap(),
        MountSource::Devices,
        DriveSelection::Automatic,
        MountMode::ReadWrite,
        "Geräte",
    )
    .unwrap()
    .with_auto_remount(true)
    .with_location_directory(Some(PathBuf::from(FOLDER)))
    .unwrap()
}

fn device_tree_json() -> String {
    concat!(
        r#"{"id":"se-devices","source":"Devices","drive":"Automatic","mode":"ReadWrite","#,
        r#""root_security":"enforced","metadata":{"preload_depth":2},"#,
        r#""cache":{"retained_mib":500},"runtime_preference":"auto","label":"Geräte","#,
        r#""location":{"Directory":{"path":"@FOLDER@"}},"auto_remount":true}"#
    )
    .replace("@FOLDER@", FOLDER)
}

fn drive_snapshot(
    config: &MountConfig,
    status: MountStatus,
    recovery: MountRecovery,
) -> MountSnapshot {
    MountSnapshot {
        capacity: None,
        config: config.clone(),
        status,
        recovery,
        recovery_required_compat: recovery.requires_retention(),
    }
}

/// One snapshot against its fixed compact text, and the text read back.
/// `capacity` follows the status since 7442afc0 (`MountSnapshot::capacity`,
/// docs/oberflaechen/laufwerke.md: capacity in the mount snapshot JSON); it is
/// `null` until the provider reported one.
fn check(snapshot: &MountSnapshot, config: &str, status: &str, recovery: &str, retained: bool) {
    let wanted = format!(
        "{{\"config\":{config},\"status\":{status},\"capacity\":null,\"recovery\":{recovery},\"recovery_required\":{retained}}}"
    );
    assert_eq!(serde_json::to_string(snapshot).unwrap(), wanted);
    let decoded: MountSnapshot = serde_json::from_str(&wanted).unwrap();
    assert_eq!(&decoded, snapshot);
}

#[test]
fn drive_task_mount_snapshot_json_unchanged() {
    let config = saved_remote();
    let m = DriveLetter::parse('M').unwrap();

    // Every state of a drive; the keys are the names of the variants and
    // their fields, in the order of the declaration.
    let states = [
        (MountStatus::Unmounted, r#""Unmounted""#),
        (MountStatus::Mounting, r#""Mounting""#),
        (
            MountStatus::Mounted { drive: m },
            r#"{"Mounted":{"drive":"M"}}"#,
        ),
        (
            MountStatus::MountedAt {
                path: PathBuf::from("/home/u/SmartExplorer/Projekte"),
            },
            r#"{"MountedAt":{"path":"/home/u/SmartExplorer/Projekte"}}"#,
        ),
        (MountStatus::Unmounting, r#""Unmounting""#),
        (
            MountStatus::RuntimeUnavailable {
                detail: "fusermount3 fehlt".into(),
            },
            r#"{"RuntimeUnavailable":{"detail":"fusermount3 fehlt"}}"#,
        ),
        (
            MountStatus::Conflict {
                drive: m,
                path: "/a.txt".into(),
                detail: "changed".into(),
            },
            r#"{"Conflict":{"drive":"M","path":"/a.txt","detail":"changed"}}"#,
        ),
        (
            MountStatus::Failed {
                detail: "vault_locked: Tresor Privat ist gesperrt".into(),
            },
            r#"{"Failed":{"detail":"vault_locked: Tresor Privat ist gesperrt"}}"#,
        ),
    ];
    for (status, json) in states {
        let snapshot = drive_snapshot(&config, status, MountRecovery::Clean);
        check(&snapshot, SAVED_REMOTE, json, r#""Clean""#, false);
    }

    // Every state of the recovery; `recovery_required` is the key an older
    // GUI or daemon reads, true for everything but `Clean`.
    let recoveries = [
        (MountRecovery::Clean, r#""Clean""#, false),
        (MountRecovery::Required, r#""Required""#, true),
        (MountRecovery::Unknown, r#""Unknown""#, true),
        (
            MountRecovery::RequiresUnlock { pending: 3 },
            r#"{"requires_unlock":{"pending":3}}"#,
            true,
        ),
    ];
    for (recovery, json, retained) in recoveries {
        let snapshot = drive_snapshot(&config, MountStatus::Mounted { drive: m }, recovery);
        check(
            &snapshot,
            SAVED_REMOTE,
            r#"{"Mounted":{"drive":"M"}}"#,
            json,
            retained,
        );
    }

    // The two optional keys of the configuration appear only when set, and
    // the folder drive's status carries its path.
    let tree = drive_snapshot(
        &device_tree(),
        MountStatus::MountedAt {
            path: PathBuf::from(FOLDER),
        },
        MountRecovery::Required,
    );
    let mounted_at = format!(r#"{{"MountedAt":{{"path":"{FOLDER}"}}}}"#);
    check(
        &tree,
        &device_tree_json(),
        &mounted_at,
        r#""Required""#,
        true,
    );

    // A snapshot of an older release has no `recovery`; its
    // `recovery_required` decides, and without both the state is unknown.
    let older =
        |tail: &str| format!("{{\"config\":{SAVED_REMOTE},\"status\":\"Unmounted\"{tail}}}");
    let read = |text: String| {
        serde_json::from_str::<MountSnapshot>(&text)
            .unwrap()
            .recovery
    };
    assert_eq!(
        read(older(r#","recovery_required":true"#)),
        MountRecovery::Required
    );
    assert_eq!(
        read(older(r#","recovery_required":false"#)),
        MountRecovery::Clean
    );
    assert_eq!(read(older("")), MountRecovery::Unknown);

    // `se drive list --json` prints the array of the snapshots.
    let none: Vec<MountSnapshot> = Vec::new();
    assert_eq!(serde_json::to_string_pretty(&none).unwrap(), "[]");
}
