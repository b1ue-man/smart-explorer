use super::api::FOLDER_MIME;
use super::sync_conflict_task_fixture::MIME;
use super::sync_reliability_task_fixture::{
    assert_complete, contains_bytes, options, DriveFixture,
};
use crate::bisync::{self, BisyncOptions, CompareMode, Direction};
use crate::vfs::Backend;
use serde_json::json;
use std::collections::HashMap;
use std::io::Write;

#[test]
fn sync_reliability_task_identity_first_migration_missing_id_never_selects_replacement() {
    for missing in ["deleted", "moved", "renamed", "deleted-without-mime"] {
        let mut f = DriveFixture::new("Notebook");
        let original = f.root_object_id();
        let note = f.write_file("note.md", b"last confirmed old bytes");
        f.write_local("note.md", b"last confirmed old bytes");
        let meta = f.local.stat(&format!("{}/note.md", f.local_root)).unwrap();
        let sig = bisync::Sig {
            size: meta.size,
            mtime_ms: meta.mtime_ms,
            hash: 0,
        };
        let old_remote = f.drive.object(&note).unwrap();
        let remote_sig = bisync::Sig {
            size: meta.size,
            mtime_ms: super::core::parse_rfc3339_ms(old_remote["modifiedTime"].as_str().unwrap())
                .unwrap(),
            hash: 0,
        };
        let baseline = bisync::Baseline::from([("note.md".into(), (Some(sig), Some(remote_sig)))]);
        let old_path = bisync::baseline_path(&bisync::pair_id_for(
            &f.local,
            &f.local_root,
            &f.backend,
            &f.root,
        ));
        bisync::save_baseline(&old_path, &baseline).unwrap();
        let old_bytes = std::fs::read(&old_path).unwrap();
        let legacy = f.directory.path().join("legacy-path-cache.json");
        let account = f.directory.path().join("account-path-cache.json");
        let mimes = if missing == "deleted-without-mime" {
            HashMap::new()
        } else {
            HashMap::from([("Notebook".into(), FOLDER_MIME.into())])
        };
        super::cache::save_to_path(
            &legacy,
            HashMap::from([("Notebook".into(), original.clone())]),
            mimes,
        )
        .unwrap();
        let legacy_bytes = std::fs::read(&legacy).unwrap();
        f.drive
            .insert("other-parent", "Elsewhere", "root", FOLDER_MIME, b"");
        match missing {
            "moved" => f
                .drive
                .change(&original, json!({"parents":["other-parent"]})),
            "renamed" => f.drive.change(&original, json!({"name":"Renamed"})),
            _ => f.drive.change(&original, json!({"trashed":true})),
        }
        f.drive
            .insert("replacement-folder", "Notebook", "root", FOLDER_MIME, b"");
        f.drive.insert(
            "replacement-note",
            "note.md",
            "replacement-folder",
            MIME,
            b"must not replace the old tree",
        );
        f.backend = f
            .fresh_backend("Notebook")
            .test_with_loaded_caches(account.clone(), &legacy);
        let opts = BisyncOptions {
            compare: CompareMode::MtimeSize,
            ..options(Direction::BtoA)
        };
        let records = f.registry_bytes();
        let blocked = f.run(opts); // the first new job run, before any new binding
        assert!(!blocked.errors.is_empty());
        assert_eq!(f.local_bytes("note.md"), b"last confirmed old bytes");
        assert_eq!(std::fs::read(&old_path).unwrap(), old_bytes);
        assert_eq!(std::fs::read(&legacy).unwrap(), legacy_bytes);
        assert_eq!(f.registry_bytes(), records);
        assert_eq!(
            f.backend.cached_id("Notebook").unwrap().as_deref(),
            Some(original.as_str())
        );
        assert_eq!(
            f.drive.bytes("replacement-note"),
            b"must not replace the old tree"
        );
        assert_eq!(f.mutations(), 0);

        let mut writer = f.backend.open_write("/Notebook/attempt.md").unwrap();
        writer
            .write_all(b"must not create in the replacement")
            .unwrap();
        assert!(writer.flush().is_err());
        drop(writer);
        assert_eq!(f.mutations(), 0);
        f.backend.persist_path_cache_checked().unwrap();
        f.backend = f
            .fresh_backend("Notebook")
            .test_with_loaded_caches(account.clone(), &legacy);
        let restarted = f.run(opts);
        assert!(!restarted.errors.is_empty());
        assert_eq!(std::fs::read(&old_path).unwrap(), old_bytes);
        assert_eq!(f.registry_bytes(), records);
        assert_eq!(f.mutations(), 0);

        // Restore the same proven object. Its competing single-name
        // replacement remains intact, so recovery cannot rely on uniqueness.
        f.drive.change(
            &original,
            json!({"name":"Notebook","parents":[f.drive.root_id],"trashed":false}),
        );
        f.drive.change(
            "replacement-folder",
            json!({"modifiedTime":"2035-01-01T00:00:00Z"}),
        );
        f.drive
            .insert(&note, "note.md", &original, MIME, b"restored previous tree");
        f.backend = f
            .fresh_backend("Notebook")
            .test_with_loaded_caches(account, &legacy);
        let recovered = f.run(opts);
        assert_complete(&recovered);
        assert_eq!(f.local_bytes("note.md"), b"restored previous tree");
        assert_eq!(
            f.backend.item_id(&f.root).unwrap().as_deref(),
            Some(original.as_str())
        );
        assert!(contains_bytes(
            &bisync::versions_dir(&recovered.state.as_ref().unwrap().pair_id),
            b"last confirmed old bytes"
        ));
        f.drive.insert(
            &note,
            "note.md",
            &original,
            MIME,
            b"changed restored exact ID",
        );
        let changed = f.run(opts);
        assert_complete(&changed);
        assert_eq!(changed.state, recovered.state);
        assert_eq!(f.local_bytes("note.md"), b"changed restored exact ID");
        assert_eq!(
            f.drive.bytes("replacement-note"),
            b"must not replace the old tree"
        );
        assert_eq!(std::fs::read(&legacy).unwrap(), legacy_bytes);
        assert_eq!(f.folder_posts(), 0);
        f.assert_noop(opts, &changed);
    }
}
