//! Acceptance signal `drive_task_plain_spool_bytes_unchanged` of DRV-M2
//! (contract V1): with the plain storage every spool, journal and stage
//! ledger byte stays as before the storage boundary existed. The second
//! signal shows that the path hook reaches every path field and nothing
//! else, so a vault drive (DRV-M7) can seal them.
use super::journal::{Journal, PersistedDelete, PersistedEntry};
use super::journal_record::DeletePhase;
use super::optimization_fixture::{FixtureDirectory, OptimizationBackend};
use super::path_sealer::{PathSealer, PlainPathSealer};
use super::spool_storage::{PlainSpoolStorage, SpoolHandle, SpoolStorage};
use super::stage_ledger::StageLedger;
use super::types::{Baseline, EntryCondition, MountConflict};
use super::{
    MountEngine, MountId, MountMode, MountRuntimeConfig, OpenDisposition, OpenFileOptions,
};
use crate::vfs::Backend;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[test]
fn drive_task_plain_spool_bytes_unchanged() {
    let directory = FixtureDirectory::new();
    // The plain spool file holds exactly the content, at every offset.
    let path = directory
        .path()
        .join("0123456789abcdef0123456789abcdef.spool");
    let mut file = PlainSpoolStorage.create_new(&path).unwrap();
    file.write_at(0, b"hello").unwrap();
    file.write_at(5, b" world").unwrap();
    file.write_at(0, b"J").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"Jello world");
    let mut output = [0u8; 5];
    assert_eq!(file.read_at(6, &mut output).unwrap(), 5);
    assert_eq!(&output, b"world");
    assert_eq!(file.read_at(11, &mut output).unwrap(), 0);
    file.set_len(5).unwrap();
    file.sync_data().unwrap();
    assert_eq!(file.len().unwrap(), 5);
    assert!(!file.is_empty().unwrap());
    assert_eq!(fs::read(&path).unwrap(), b"Jello");
    let error = PlainSpoolStorage.create_new(&path).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    drop(file);

    // The positioned handle the engine uses behaves like `File`.
    let mut handle = SpoolHandle::new(PlainSpoolStorage.open(&path, true).unwrap());
    handle.seek(SeekFrom::End(0)).unwrap();
    handle.write_all(b"!").unwrap();
    handle.seek(SeekFrom::Start(1)).unwrap();
    let mut rest = String::new();
    handle.read_to_string(&mut rest).unwrap();
    assert_eq!(rest, "ello!");
    assert_eq!(handle.metadata().unwrap().len(), 6);
    assert!(handle.seek(SeekFrom::Current(-100)).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"Jello!");

    // Through the engine: the spool of a written file and its journal line
    // keep the form of v0.5.182.
    let backend = OptimizationBackend::new();
    backend.put("/note", b"old");
    let baseline = backend.stat("/note").unwrap();
    let id = MountId::parse("plain-spool-bytes").unwrap();
    let engine = MountEngine::open_host_cache(
        MountRuntimeConfig::new(id.clone(), MountMode::ReadWrite),
        backend.clone(),
        directory.path(),
    )
    .unwrap();
    let handle = engine
        .open_file(
            "\\note",
            OpenFileOptions {
                writable: true,
                disposition: OpenDisposition::OpenExisting,
            },
        )
        .unwrap();
    assert_eq!(engine.write(handle, 0, b"new bytes").unwrap(), 9);
    let mount = directory.path().join(id.as_str());
    let spools = spool_files(&mount.join("files"));
    assert_eq!(spools.len(), 1);
    assert_eq!(fs::read(&spools[0]).unwrap(), b"new bytes");
    let spool_name = spools[0].file_name().unwrap().to_str().unwrap().to_string();
    let expected = format!(
        concat!(
            r#"{{"version":1,"action":{{"UpsertEntry":{{"remote_path":"/note","#,
            r#""spool_name":"{}","baseline":{{"Present":{{"id":"{}","size":3,"#,
            r#""mtime_ms":{},"content_md5":null}}}},"condition":"Dirty","#,
            r#""delete_token":null}}}}}}"#,
            "\n"
        ),
        spool_name,
        baseline.id.as_deref().unwrap(),
        baseline.mtime_ms
    );
    assert_eq!(
        fs::read_to_string(mount.join("journal.jsonl")).unwrap(),
        expected
    );
    engine.close(handle).unwrap();
    drop(engine);

    // The stage ledger keeps its plain form.
    let ledger_root = directory.path().join("ledger");
    fs::create_dir(&ledger_root).unwrap();
    let ledger = StageLedger::open(&ledger_root, Arc::new(PlainPathSealer));
    drop(ledger.begin("/a.pdf.se-mount-0000000000000001", 10));
    assert_eq!(
        fs::read_to_string(ledger_root.join("stages.json")).unwrap(),
        r#"{"stages":{"/a.pdf.se-mount-0000000000000001":10}}"#
    );
}

#[test]
fn drive_task_path_sealer_reaches_every_journal_path_field() {
    let directory = FixtureDirectory::new();
    let journal_path = directory.path().join("journal.jsonl");
    let sealer: Arc<dyn PathSealer> = Arc::new(MarkingSealer);
    let (mut journal, _) = Journal::open(&journal_path, Arc::clone(&sealer)).unwrap();
    let conflict = MountConflict {
        path: "/docs/a.txt".into(),
        baseline: Baseline::Missing,
        current: None,
        detail: "changed".into(),
    };
    let entry = PersistedEntry {
        remote_path: "/docs/a.txt".into(),
        spool_name: "0123456789abcdef0123456789abcdef.spool".into(),
        baseline: Baseline::Missing,
        condition: EntryCondition::Conflict(conflict),
        delete_token: None,
    };
    journal.upsert_entry(&entry).unwrap();
    journal.move_entry("/docs/old.txt", &entry).unwrap();
    journal
        .upsert_delete(&PersistedDelete {
            token: 7,
            original_path: "/docs/gone.txt".into(),
            quarantine_path: "/docs/gone.txt.se-mount-delete-0000000000000007".into(),
            id: Some("object-7".into()),
            is_directory: false,
            phase: DeletePhase::Moved,
        })
        .unwrap();
    journal.forget_entry("/docs/old.txt").unwrap();
    drop(journal);
    let written = fs::read_to_string(&journal_path).unwrap();
    // Every path field is sealed; names of spool files, ids and texts are not.
    for plain in [
        "\"/docs/a.txt\"",
        "\"/docs/old.txt\"",
        "\"/docs/gone.txt\"",
        "\"/docs/gone.txt.se-mount-delete-0000000000000007\"",
    ] {
        assert!(!written.contains(plain), "{plain} stayed plain: {written}");
    }
    for kept in [
        "0123456789abcdef0123456789abcdef.spool",
        "object-7",
        "\"changed\"",
    ] {
        assert!(written.contains(kept), "{kept} was altered: {written}");
    }
    // Replay unseals: the recovered state holds the plain paths.
    let (_, recovered) = Journal::open(&journal_path, sealer).unwrap();
    let restored = &recovered.entries["/docs/a.txt"];
    assert!(matches!(
        &restored.condition,
        EntryCondition::Conflict(conflict) if conflict.path == "/docs/a.txt"
    ));
    assert!(!recovered.entries.contains_key("/docs/old.txt"));
    assert_eq!(recovered.deletes[&7].original_path, "/docs/gone.txt");
    // A sealer that cannot open the records refuses the journal.
    let refused = Journal::open(&journal_path, Arc::new(PlainPathSealer));
    assert!(refused.is_ok(), "the plain sealer reads any text");
    let error = Journal::open(&journal_path, Arc::new(RefusingSealer))
        .err()
        .unwrap();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

fn spool_files(files: &Path) -> Vec<PathBuf> {
    let mut found = fs::read_dir(files)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "spool")
        })
        .collect::<Vec<_>>();
    found.sort();
    found
}

/// Test sealer: a reversible text form that cannot be mistaken for a path.
struct MarkingSealer;

impl PathSealer for MarkingSealer {
    fn seal(&self, path: &str) -> io::Result<String> {
        Ok(format!("sealed:{}", path.chars().rev().collect::<String>()))
    }

    fn unseal(&self, sealed: &str) -> io::Result<String> {
        sealed
            .strip_prefix("sealed:")
            .map(|reversed| reversed.chars().rev().collect())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not sealed by this test"))
    }
}

/// A sealer without the key of the stored records.
struct RefusingSealer;

impl PathSealer for RefusingSealer {
    fn seal(&self, path: &str) -> io::Result<String> {
        Ok(path.to_string())
    }

    fn unseal(&self, _sealed: &str) -> io::Result<String> {
        Err(io::Error::new(io::ErrorKind::InvalidData, "wrong key"))
    }
}
