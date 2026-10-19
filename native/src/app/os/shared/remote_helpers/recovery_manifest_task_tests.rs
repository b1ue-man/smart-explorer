use super::{preserve_marker_state, PreserveMarkerState, LEGACY_MANIFEST_HEADER, PRESERVE_MARKER};

fn write_marker(directory: &std::path::Path, contents: &str) {
    std::fs::write(directory.join(PRESERVE_MARKER), contents).unwrap();
}

#[test]
fn remote_drive_task_empty_complete_recovery_markers_are_cleanup_only() {
    let temporary = tempfile::tempdir().unwrap();
    assert_eq!(
        preserve_marker_state(temporary.path()),
        PreserveMarkerState::Missing
    );

    write_marker(temporary.path(), r#"{"schema":2,"entries":[]}"#);
    assert_eq!(
        preserve_marker_state(temporary.path()),
        PreserveMarkerState::KnownEmpty
    );

    std::fs::create_dir(temporary.path().join("allocated-but-empty")).unwrap();
    assert_eq!(
        preserve_marker_state(temporary.path()),
        PreserveMarkerState::KnownEmpty
    );

    write_marker(
        temporary.path(),
        &format!("{LEGACY_MANIFEST_HEADER}\nactive_transfer=0\n"),
    );
    assert_eq!(
        preserve_marker_state(temporary.path()),
        PreserveMarkerState::KnownEmpty
    );
}

#[test]
fn remote_drive_task_invalid_or_declared_recovery_fails_closed() {
    let temporary = tempfile::tempdir().unwrap();
    write_marker(temporary.path(), r#"{"schema":2,"entries":["#);
    assert_eq!(
        preserve_marker_state(temporary.path()),
        PreserveMarkerState::Recovery
    );

    write_marker(
        temporary.path(),
        r#"{
  "schema": 2,
  "entries": [{
    "name": "note.md",
    "local": "missing-during-atomic-save.md",
    "remote": "/notes/note.md",
    "dirty": true,
    "uploading": false
  }]
}"#,
    );
    assert_eq!(
        preserve_marker_state(temporary.path()),
        PreserveMarkerState::Recovery
    );
}

#[test]
fn remote_drive_task_empty_marker_with_real_payload_is_recovery() {
    let temporary = tempfile::tempdir().unwrap();
    write_marker(temporary.path(), r#"{"schema":2,"entries":[]}"#);
    std::fs::write(temporary.path().join("downloaded-note.md"), b"unsaved").unwrap();
    assert_eq!(
        preserve_marker_state(temporary.path()),
        PreserveMarkerState::Recovery
    );
}

mod vault_fields {
    use super::super::{seal_vault_fields, RecoveryManifestEntry};

    const SITE: &str = "0123456789abcdef0123456789abcdef";

    fn entry() -> RecoveryManifestEntry {
        RecoveryManifestEntry {
            name: "Geheim.txt".into(),
            local: "/tmp/se/open-temp/1/Geheim.txt".into(),
            remote: "/Akten/Geheim.txt".into(),
            dirty: true,
            uploading: false,
            vault: String::new(),
            withheld: false,
        }
    }

    fn sealer(site: &str, items: &[&str]) -> Result<Vec<String>, String> {
        Ok(items
            .iter()
            .map(|item| format!("sealed:{site}:{}", item.len()))
            .collect())
    }

    #[test]
    fn vault_task_recovery_manifest_seals_path_fields_of_vault_copies() {
        let mut sealed = entry();
        seal_vault_fields(&mut sealed, Some(SITE), &sealer);
        let text = serde_json::to_string(&sealed).unwrap();
        assert!(
            !text.contains("Geheim") && !text.contains("Akten"),
            "{text}"
        );
        assert_eq!(sealed.vault, SITE);
        assert!(!sealed.withheld);
        assert_eq!(sealed.name, format!("sealed:{SITE}:10"));
        // An unknown site or a failing sealer withholds the fields.
        let mut unknown = entry();
        seal_vault_fields(&mut unknown, None, &sealer);
        let mut failing = entry();
        seal_vault_fields(&mut failing, Some(SITE), &|_, _| Err("gesperrt".into()));
        for withheld in [&unknown, &failing] {
            let text = serde_json::to_string(withheld).unwrap();
            assert!(
                !text.contains("Geheim") && !text.contains("Akten"),
                "{text}"
            );
            assert!(withheld.withheld);
            assert!(withheld.name.is_empty() && withheld.local.is_empty());
        }
    }

    #[test]
    fn vault_task_recovery_manifest_plain_entries_unchanged() {
        // Copies of every other backend keep the exact form of schema 2.
        let text = serde_json::to_string(&entry()).unwrap();
        assert_eq!(
            text,
            r#"{"name":"Geheim.txt","local":"/tmp/se/open-temp/1/Geheim.txt","remote":"/Akten/Geheim.txt","dirty":true,"uploading":false}"#
        );
        let old: RecoveryManifestEntry = serde_json::from_str(&text).unwrap();
        assert!(old.vault.is_empty() && !old.withheld);
    }
}
