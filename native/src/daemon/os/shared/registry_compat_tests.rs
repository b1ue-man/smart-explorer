//! Acceptance signals of DRV-M3 for the drive registry (plan 2026-10-17,
//! strand S2 C4): stored drives of v0.5.182 keep their bytes and meaning,
//! v0.5.182 keeps reading `registry.json`, and the kinds it cannot read
//! never enter that file.
use serde::Deserialize;
use std::path::{Path, PathBuf};

use super::ext::{known_sources, SECOND_FILE};
use super::{load_in, remove_in, upsert_in, REGISTRY_FILE};
use crate::mount::{
    BackendRoot, DriveSelection, MountConfig, MountId, MountMode, MountSource, PeerMountTarget,
};

/// `registry.json` as v0.5.182 (commit `da2646d2`) wrote it: compact, sorted
/// by id, every field of that version present.
const FIXTURE_V0_5_182: &str = concat!(
    r#"{"version":1,"mounts":["#,
    r#"{"id":"a-saved","source":{"SavedRemote":{"account":"sftp:prod","root":"/srv/data"}},"#,
    r#""drive":"Automatic","mode":"ReadWrite","root_security":"trusted","#,
    r#""metadata":{"preload_depth":0},"cache":{"retained_mib":0},"#,
    r#""runtime_preference":"system","label":"Projekte"},"#,
    r#"{"id":"b-gdrive","source":{"GoogleDrive":{"account":"cloud:gdrive","root":"/"}},"#,
    r#""drive":{"Letter":"G"},"mode":"ReadOnly","root_security":"enforced","#,
    r#""metadata":{"preload_depth":2},"cache":{"retained_mib":500},"#,
    r#""runtime_preference":"auto","label":"Google Drive"},"#,
    r#"{"id":"c-peer","source":{"Peer":{"target":{"Direct":{"contact_id":"contact-1"}},"#,
    r#""root":"/Fotos"}},"drive":"Automatic","mode":"ReadOnly","root_security":"enforced","#,
    r#""metadata":{"preload_depth":4},"cache":{"retained_mib":65536},"#,
    r#""runtime_preference":"auto","label":"Fotos \"Ä\""},"#,
    r#"{"id":"d-room","source":{"Peer":{"target":{"RoomDevice":{"room_id":"room-7","#,
    r#""device_id":"device-9"}},"root":"//server/share"}},"drive":{"Letter":"Z"},"#,
    r#""mode":"ReadOnly","root_security":"enforced","metadata":{"preload_depth":1},"#,
    r#""cache":{"retained_mib":100},"runtime_preference":"auto","label":"Raum"}"#,
    r#"]}"#
);

/// The registry types of v0.5.182, variant by variant: unknown fields are
/// ignored there too (no `deny_unknown_fields`), unknown kinds are errors.
#[derive(Debug, Deserialize)]
struct OldRegistry {
    version: u32,
    mounts: Vec<OldMountConfig>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OldMountConfig {
    id: String,
    source: OldMountSource,
    drive: OldDriveSelection,
    mode: OldMountMode,
    #[serde(default)]
    root_security: Option<String>,
    #[serde(default)]
    metadata: Option<OldMetadata>,
    #[serde(default)]
    cache: Option<OldCache>,
    #[serde(default)]
    runtime_preference: Option<String>,
    label: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
enum OldMountSource {
    SavedRemote { account: String, root: String },
    GoogleDrive { account: String, root: String },
    Peer { target: OldPeerTarget, root: String },
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
enum OldPeerTarget {
    Direct { contact_id: String },
    RoomDevice { room_id: String, device_id: String },
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
enum OldDriveSelection {
    Automatic,
    Letter(String),
}

#[derive(Debug, Deserialize)]
enum OldMountMode {
    ReadOnly,
    ReadWrite,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OldMetadata {
    preload_depth: u8,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OldCache {
    retained_mib: u32,
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "se_drive_registry_{tag}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn read(dir: &Path, file: &str) -> Option<Vec<u8>> {
    std::fs::read(dir.join(file)).ok()
}

fn old_parse(dir: &Path) -> OldRegistry {
    let registry: OldRegistry = serde_json::from_slice(&read(dir, REGISTRY_FILE).unwrap()).unwrap();
    assert_eq!(registry.version, 1);
    registry
}

fn drive(id: &str, source: MountSource) -> MountConfig {
    MountConfig::new(
        MountId::parse(id).unwrap(),
        source,
        DriveSelection::Automatic,
        MountMode::ReadOnly,
        id,
    )
    .unwrap()
}

fn saved(id: &str) -> MountConfig {
    drive(
        id,
        MountSource::SavedRemote {
            account: "sftp:prod".into(),
            root: BackendRoot::parse("/srv/data").unwrap(),
        },
    )
}

/// An absolute folder of this platform.
fn folder() -> PathBuf {
    std::env::temp_dir().join("SmartExplorer").join("Projekte")
}

#[test]
fn drive_task_registry_v0_5_182_fixture_roundtrip_identical() {
    let dir = temp_dir("fixture");
    std::fs::write(dir.join(REGISTRY_FILE), FIXTURE_V0_5_182).unwrap();
    let configs = load_in(&dir).unwrap();
    let ids: Vec<_> = configs.iter().map(|config| config.id.as_str()).collect();
    assert_eq!(ids, ["a-saved", "b-gdrive", "c-peer", "d-room"]);
    // Stored drives keep their meaning: no option of this batch is set.
    assert!(configs
        .iter()
        .all(|config| !config.auto_remount && config.location.is_none()));
    assert!(configs.iter().all(|config| !config.restores_at_login()));
    assert_eq!(configs[0].label, "Projekte");
    assert_eq!(configs[2].label, "Fotos \"Ä\"");
    assert_eq!(configs[3].source.root().as_str(), "//server/share");
    // Storing an unchanged drive rewrites the file with the same bytes.
    upsert_in(&dir, &configs[1]).unwrap();
    assert_eq!(
        read(&dir, REGISTRY_FILE).unwrap(),
        FIXTURE_V0_5_182.as_bytes()
    );
    // A drive with the options of this batch comes and goes without
    // touching the bytes of the others.
    let added = saved("e-login")
        .with_auto_remount(true)
        .with_location_directory(Some(folder()))
        .unwrap();
    upsert_in(&dir, &added).unwrap();
    let with_added = String::from_utf8(read(&dir, REGISTRY_FILE).unwrap()).unwrap();
    let untouched = &FIXTURE_V0_5_182[..FIXTURE_V0_5_182.len() - 2];
    assert!(with_added.starts_with(untouched), "{with_added}");
    remove_in(&dir, &added.id).unwrap();
    assert_eq!(
        read(&dir, REGISTRY_FILE).unwrap(),
        FIXTURE_V0_5_182.as_bytes()
    );
    // Removing an unknown drive writes nothing; no second file appears.
    remove_in(&dir, &MountId::parse("unknown").unwrap()).unwrap();
    assert_eq!(
        read(&dir, REGISTRY_FILE).unwrap(),
        FIXTURE_V0_5_182.as_bytes()
    );
    assert!(read(&dir, SECOND_FILE).is_none());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn drive_task_registry_old_parser_ignores_new_fields() {
    let dir = temp_dir("old_parser");
    let login = saved("login")
        .with_auto_remount(true)
        .with_location_directory(Some(folder()))
        .unwrap();
    let share = drive(
        "share",
        MountSource::Peer {
            target: PeerMountTarget::RoomDevice {
                room_id: "room-7".into(),
                device_id: "device-9".into(),
            },
            root: BackendRoot::parse("/").unwrap(),
        },
    )
    .with_auto_remount(true);
    let plain = saved("plain");
    for config in [&login, &share, &plain] {
        upsert_in(&dir, config).unwrap();
    }
    // The new fields are written, and only where they are set.
    let body = String::from_utf8(read(&dir, REGISTRY_FILE).unwrap()).unwrap();
    let location = serde_json::to_string(&login.location).unwrap();
    assert!(body.contains(&format!(
        r#""label":"login","location":{location},"auto_remount":true}}"#
    )));
    assert!(body.contains(r#""label":"share","auto_remount":true}"#));
    assert!(body.contains(r#""label":"plain"}"#));
    // v0.5.182 reads the file and sees its own fields unchanged.
    let old = old_parse(&dir);
    let labels: Vec<_> = old
        .mounts
        .iter()
        .map(|mount| mount.label.as_str())
        .collect();
    assert_eq!(labels, ["login", "plain", "share"]);
    assert!(matches!(
        &old.mounts[0].source,
        OldMountSource::SavedRemote { root, .. } if root == "/srv/data"
    ));
    assert!(matches!(old.mounts[2].mode, OldMountMode::ReadOnly));
    // This version reads the options back.
    let configs = load_in(&dir).unwrap();
    assert_eq!(configs, [login.clone(), plain, share]);
    assert_eq!(configs[0].location_directory(), Some(folder().as_path()));
    // A relative or dotted folder is refused before it is stored.
    assert!(saved("bad")
        .with_location_directory(Some(PathBuf::from("SmartExplorer")))
        .is_err());
    assert!(saved("bad")
        .with_location_directory(Some(folder().join("..").join("x")))
        .is_err());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn drive_task_registry_ext_kinds_never_in_v1_file() {
    let dir = temp_dir("ext_kinds");
    let plain = saved("plain");
    let devices = drive("devices", MountSource::Devices);
    let vault = drive(
        "vault",
        MountSource::Vault {
            site_id: "site-1234".into(),
        },
    )
    .with_auto_remount(true);
    for config in [&plain, &devices, &vault] {
        upsert_in(&dir, config).unwrap();
    }
    let first = String::from_utf8(read(&dir, REGISTRY_FILE).unwrap()).unwrap();
    assert!(
        !first.contains("Devices") && !first.contains("Vault"),
        "{first}"
    );
    assert_eq!(old_parse(&dir).mounts.len(), 1);
    let second = String::from_utf8(read(&dir, SECOND_FILE).unwrap()).unwrap();
    assert!(second.starts_with(r#"{"version":1,"mounts":["#), "{second}");
    assert!(second.contains(r#""source":"Devices""#), "{second}");
    assert!(second.contains(r#""source":{"Vault":{"site_id":"site-1234"}}"#));
    assert_eq!(
        load_in(&dir).unwrap(),
        [devices.clone(), plain.clone(), vault.clone()]
    );
    // A change of a later kind leaves `registry.json` as it is.
    let first_bytes = read(&dir, REGISTRY_FILE);
    remove_in(&dir, &vault.id).unwrap();
    assert_eq!(read(&dir, REGISTRY_FILE), first_bytes);
    assert_eq!(load_in(&dir).unwrap(), [devices.clone(), plain.clone()]);

    // An entry of a kind that only a later version knows stays in place.
    let foreign = r#"{"id":"later","source":{"Tunnel":{"host":"x"}},"label":"later"}"#;
    std::fs::write(
        dir.join(SECOND_FILE),
        format!(r#"{{"version":1,"mounts":[{foreign}]}}"#),
    )
    .unwrap();
    assert_eq!(load_in(&dir).unwrap(), [plain.clone()]);
    upsert_in(&dir, &devices).unwrap();
    let second = String::from_utf8(read(&dir, SECOND_FILE).unwrap()).unwrap();
    assert!(second.contains(r#""source":"Devices""#), "{second}");
    assert!(second.contains(r#""Tunnel""#), "{second}");
    assert_eq!(load_in(&dir).unwrap(), [devices.clone(), plain.clone()]);

    // A later kind written into `registry.json` by hand moves out of it at
    // the next change, so v0.5.182 reads that file again.
    let stray = serde_json::to_string(&vault).unwrap();
    let edited = first.replacen(r#""mounts":["#, &format!(r#""mounts":[{stray},"#), 1);
    std::fs::write(dir.join(REGISTRY_FILE), edited).unwrap();
    assert!(serde_json::from_slice::<OldRegistry>(&read(&dir, REGISTRY_FILE).unwrap()).is_err());
    upsert_in(&dir, &saved("other")).unwrap();
    assert_eq!(old_parse(&dir).mounts.len(), 2);
    let second = String::from_utf8(read(&dir, SECOND_FILE).unwrap()).unwrap();
    assert!(second.contains("site-1234"), "{second}");
    assert_eq!(load_in(&dir).unwrap().len(), 4);

    // Every kind of this version is known to the second file.
    for source in [
        plain.source.clone(),
        MountSource::GoogleDrive {
            account: "cloud:gdrive".into(),
            root: BackendRoot::parse("/").unwrap(),
        },
        MountSource::Peer {
            target: PeerMountTarget::Direct {
                contact_id: "c".into(),
            },
            root: BackendRoot::parse("/").unwrap(),
        },
        devices.source.clone(),
        vault.source.clone(),
    ] {
        let value = serde_json::to_value(&source).unwrap();
        let kind = value
            .as_str()
            .map(str::to_string)
            .or_else(|| {
                value
                    .as_object()
                    .and_then(|object| object.keys().next().cloned())
            })
            .unwrap();
        assert!(known_sources().contains(&kind.as_str()), "{kind}");
        assert_eq!(
            source.stored_in_first_registry(),
            !matches!(source, MountSource::Devices | MountSource::Vault { .. })
        );
    }
    std::fs::remove_dir_all(dir).ok();
}
