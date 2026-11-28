//! Resumable terminal jobs on filesystems without stable folder ids (U4):
//! records written before the weaker identity existed load unchanged, and a
//! record without any identity of the destination root is still refused.
use super::*;
use crate::vfs::{BackendHandle, LocalBackend};
use std::sync::Arc;

fn endpoint_of(local: bool) -> StoredEndpoint {
    StoredEndpoint {
        local,
        namespace: "local".to_string(),
        state: "local".to_string(),
    }
}

fn record(parent: &str) -> SavedCommand {
    SavedCommand {
        version: 1,
        id: "0".repeat(64),
        source: endpoint_of(true),
        target: endpoint_of(true),
        source_path: "/media/u/STICK/Fotos".to_string(),
        vault_locations: Vec::new(),
        source_local_root: Some("/".to_string()),
        target_argument: parent.to_string(),
        target_local_root: Some("/".to_string()),
        destination: format!("{parent}/Fotos"),
        parent: parent.to_string(),
        parent_object: None,
        parent_location: None,
        recursive: true,
        force: false,
        moving: false,
        folders: FolderConflict::Merge,
        stamp: SourceStamp {
            name: "Fotos".to_string(),
            id: None,
            directory: true,
            link: false,
            size: 0,
            time: 0,
            md5: None,
            object: None,
            location: None,
        },
        complete: false,
        mutation_started: false,
    }
}

fn local(path: &str) -> Target {
    let backend: BackendHandle = Arc::new(LocalBackend::new("/"));
    Target::with_backend_key(backend, path.to_string(), "local")
}

#[test]
fn wechselmedium_task_records_without_location_identity_load_unchanged() {
    let old = serde_json::to_value(record("/media/u/STICK")).expect("serialize");
    assert!(
        old.get("parent_location").is_none(),
        "no new key when absent"
    );
    assert!(old["stamp"].get("location").is_none());
    let loaded: SavedCommand = serde_json::from_value(old).expect("an old record loads");
    assert_eq!(loaded.parent_location, None);
    assert_eq!(loaded.stamp.location, None);

    let mut weak = record("/media/u/STICK");
    weak.parent_location = Some("volume-location:v1:abcd-1234:-:/".to_string());
    weak.stamp.location = Some("volume-location:v1:abcd-1234:-:/Fotos".to_string());
    let text = serde_json::to_string(&weak).expect("serialize");
    let loaded: SavedCommand = serde_json::from_str(&text).expect("load");
    assert_eq!(loaded.parent_location, weak.parent_location);
    assert_eq!(loaded.stamp.location, weak.stamp.location);
}

#[test]
fn wechselmedium_task_destination_root_needs_some_identity() {
    let scratch = tempfile::tempdir().expect("scratch");
    let parent = scratch.path().to_string_lossy().replace('\\', "/");
    let target = local(&parent);
    let unconfirmed = record(&parent);
    assert_eq!(
        confirm_parent(&unconfirmed, &target).unwrap_err(),
        "Gespeicherter Zielroot ist geändert oder unbestätigt"
    );
    let mut other_place = record(&parent);
    other_place.parent_location = Some("volume-location:v1:0000-0000:-:/elsewhere".to_string());
    assert!(confirm_parent(&other_place, &target).is_err());
    if let Some(location) = location_identity(&*target.backend, &parent) {
        assert!(location.starts_with("volume-location:v1:"), "{location}");
        let mut same_place = record(&parent);
        same_place.parent_location = Some(location);
        assert!(confirm_parent(&same_place, &target).is_ok());
    }
    if let Some(object) = crate::vfs::directory_identity(&*target.backend, &parent)
        .expect("identity of a local folder")
    {
        let mut stable = record(&parent);
        stable.parent_object = Some(object);
        stable.parent_location = Some("volume-location:v1:ignored".to_string());
        assert!(
            confirm_parent(&stable, &target).is_ok(),
            "a stable identity decides over a recorded location"
        );
    }
}
