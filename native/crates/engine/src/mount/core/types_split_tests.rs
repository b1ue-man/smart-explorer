//! Acceptance signal `drive_task_types_split_serde_unchanged` of DRV-M1
//! (plan 2026-10-17 „Rechte, se Desktop, Tresore“): `MountSource`,
//! `PeerMountTarget`, `MountStatus` and the outcome enums moved out of
//! `types.rs` without changing one serde byte. Every JSON literal below is
//! the compact form the type definitions of commit `da2646d2` produce.
use super::{
    BackendRoot, Baseline, DriveLetter, DriveSelection, FlushOutcome, MountConfig, MountConflict,
    MountId, MountMode, MountRuntimeConfig, MountSource, MountStatus, PathDialect, PeerMountTarget,
};

#[test]
fn drive_task_types_split_serde_unchanged() {
    let root = BackendRoot::parse("/srv/data").unwrap();
    let sources = [
        (
            MountSource::SavedRemote {
                account: "sftp:prod".into(),
                root: root.clone(),
            },
            r#"{"SavedRemote":{"account":"sftp:prod","root":"/srv/data"}}"#,
        ),
        (
            MountSource::GoogleDrive {
                account: "cloud:gdrive".into(),
                root: BackendRoot::parse("/").unwrap(),
            },
            r#"{"GoogleDrive":{"account":"cloud:gdrive","root":"/"}}"#,
        ),
        (
            MountSource::Peer {
                target: PeerMountTarget::Direct {
                    contact_id: "contact-1".into(),
                },
                root: root.clone(),
            },
            r#"{"Peer":{"target":{"Direct":{"contact_id":"contact-1"}},"root":"/srv/data"}}"#,
        ),
        (
            MountSource::Peer {
                target: PeerMountTarget::RoomDevice {
                    room_id: "room-7".into(),
                    device_id: "device-9".into(),
                },
                root: BackendRoot::parse("//server/share").unwrap(),
            },
            r#"{"Peer":{"target":{"RoomDevice":{"room_id":"room-7","device_id":"device-9"}},"root":"//server/share"}}"#,
        ),
    ];
    for (source, json) in sources {
        round_trip(&source, json);
    }
    let z = DriveLetter::parse('z').unwrap();
    let statuses = [
        (MountStatus::Unmounted, r#""Unmounted""#),
        (MountStatus::Mounting, r#""Mounting""#),
        (
            MountStatus::Mounted { drive: z },
            r#"{"Mounted":{"drive":"Z"}}"#,
        ),
        (MountStatus::Unmounting, r#""Unmounting""#),
        (
            MountStatus::RuntimeUnavailable {
                detail: "dokany".into(),
            },
            r#"{"RuntimeUnavailable":{"detail":"dokany"}}"#,
        ),
        (
            MountStatus::Conflict {
                drive: z,
                path: "/a.txt".into(),
                detail: "changed".into(),
            },
            r#"{"Conflict":{"drive":"Z","path":"/a.txt","detail":"changed"}}"#,
        ),
        (
            MountStatus::Failed {
                detail: "Ä \"x\"".into(),
            },
            r#"{"Failed":{"detail":"Ä \"x\""}}"#,
        ),
    ];
    for (status, json) in statuses {
        round_trip(&status, json);
    }
    // A stored drive definition keeps its complete registry form.
    let config = MountConfig::new(
        MountId::parse("split-fixture").unwrap(),
        MountSource::Peer {
            target: PeerMountTarget::Direct {
                contact_id: "contact-1".into(),
            },
            root,
        },
        DriveSelection::Letter(z),
        MountMode::ReadOnly,
        "Fotos",
    )
    .unwrap();
    round_trip(
        &config,
        r#"{"id":"split-fixture","source":{"Peer":{"target":{"Direct":{"contact_id":"contact-1"}},"root":"/srv/data"}},"drive":{"Letter":"Z"},"mode":"ReadOnly","root_security":"enforced","metadata":{"preload_depth":2},"cache":{"retained_mib":500},"runtime_preference":"auto","label":"Fotos"}"#,
    );
    // The runtime configuration writes the dialect only when it is not the
    // Windows default, and reads the older form as Windows.
    let runtime = config.runtime();
    let legacy = r#"{"id":"split-fixture","mode":"ReadOnly","metadata":{"preload_depth":2},"cache":{"retained_mib":500},"runtime_preference":"auto"}"#;
    round_trip(&runtime, legacy);
    let posix = runtime.clone().with_path_dialect(PathDialect::Posix);
    let encoded = serde_json::to_string(&posix).unwrap();
    assert!(
        encoded.ends_with(r#","path_dialect":"posix"}"#),
        "{encoded}"
    );
    let decoded: MountRuntimeConfig = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, posix);
    // The outcome types moved as well; they stay plain Rust values.
    let conflict = MountConflict {
        path: "/a".into(),
        baseline: Baseline::Missing,
        current: None,
        detail: "d".into(),
    };
    assert_eq!(
        FlushOutcome::Conflict(conflict.clone()),
        FlushOutcome::Conflict(conflict)
    );
}

fn round_trip<T>(value: &T, json: &str)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    assert_eq!(serde_json::to_string(value).unwrap(), json);
    let decoded: T = serde_json::from_str(json).unwrap();
    assert_eq!(&decoded, value);
}
