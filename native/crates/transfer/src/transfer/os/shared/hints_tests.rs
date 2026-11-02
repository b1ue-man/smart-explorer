//! The hint file: hashed salted keys, private atomic writes, a distrustful
//! reader.
use super::*;

const NOW: u64 = 1_800_000_000_000;

fn hints_in(dir: &std::path::Path) -> Hints {
    Hints::at(dir.join("hints").join(FILE_NAME))
}

fn text_of(hints: &Hints) -> String {
    std::fs::read_to_string(hints.path.as_deref().expect("a path")).expect("the hint file")
}

#[test]
fn speed_task_hints_roundtrip_with_hashed_keys() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut hints = hints_in(dir.path());
    assert_eq!(hints.start_limit("Sftp|alice@files.example.org", NOW), None);
    hints.record("Sftp|alice@files.example.org", 16, NOW);
    let text = text_of(&hints);
    // neither the host nor the account is in the file
    assert!(
        !text.contains("alice") && !text.contains("example.org"),
        "{text}"
    );
    let form: FileForm = serde_json::from_str(&text).expect("valid json");
    assert_eq!(form.version, 1);
    assert_eq!(form.salt.len(), 64);
    assert_eq!(form.flows.len(), 1);
    assert!(is_key(&form.flows[0].key), "{}", form.flows[0].key);
    assert_eq!((form.flows[0].limit, form.flows[0].saved_ms), (16, NOW));
    // a new process reads it back: it starts at half the learned limit
    let mut again = hints_in(dir.path());
    assert_eq!(
        again.start_limit("Sftp|alice@files.example.org", NOW + 1),
        Some(8)
    );
    assert_eq!(
        again.start_limit("Sftp|bob@files.example.org", NOW + 1),
        None
    );
}

#[test]
fn speed_task_hints_ignore_corrupt_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("hints").join(FILE_NAME);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("folder");
    for garbage in [
        "",
        "not json",
        "{\"version\":2,\"salt\":\"\",\"flows\":[]}",
        "[1,2]",
    ] {
        std::fs::write(&path, garbage).expect("garbage");
        let mut hints = Hints::at(path.clone());
        assert_eq!(hints.start_limit("x", NOW), None, "{garbage:?}");
        // and writing replaces it
        hints.record("x", 4, NOW);
        assert_eq!(Hints::at(path.clone()).start_limit("x", NOW), Some(2));
    }
}

#[test]
fn speed_task_hints_reader_drops_invalid_values() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut hints = hints_in(dir.path());
    let good = hints.identity_hash("good", NOW).expect("hash");
    let zero = hints.identity_hash("zero", NOW).expect("hash");
    let huge = hints.identity_hash("huge", NOW).expect("hash");
    let future = hints.identity_hash("future", NOW).expect("hash");
    let salt = hex(&hints.salt.expect("salt"));
    let entry = |key: &str, limit: usize, saved_ms: u64| Entry {
        key: key.to_string(),
        limit,
        saved_ms,
    };
    let form = FileForm {
        version: 1,
        salt,
        flows: vec![
            entry(&good, 32, NOW - 1),
            entry(&zero, 0, NOW - 1),
            entry(&huge, RESOURCE_CEILING + 1, NOW - 1),
            entry(&future, 8, NOW + 60_000),
            entry("not a key", 8, NOW - 1),
            entry(&"A".repeat(64), 8, NOW - 1),
        ],
    };
    let path = hints.path.clone().expect("path");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("folder");
    std::fs::write(&path, serde_json::to_vec(&form).expect("json")).expect("write");
    let mut reader = Hints::at(path);
    assert_eq!(reader.start_limit("good", NOW), Some(16));
    assert_eq!(reader.start_limit("zero", NOW), None, "limit 0");
    assert_eq!(
        reader.start_limit("huge", NOW),
        None,
        "above the flow bound"
    );
    assert_eq!(
        reader.start_limit("future", NOW),
        None,
        "saved in the future"
    );
    assert_eq!(reader.entries.len(), 1);
}

#[test]
fn speed_task_hints_cap_is_512_oldest_first() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut hints = hints_in(dir.path());
    for index in 0..600u64 {
        hints.record(&format!("identity-{index}"), 4, NOW + index);
    }
    assert_eq!(hints.entries.len(), 512);
    assert_eq!(
        hints.start_limit("identity-0", NOW + 600),
        None,
        "oldest dropped"
    );
    assert_eq!(hints.start_limit("identity-87", NOW + 600), None);
    assert_eq!(hints.start_limit("identity-88", NOW + 600), Some(2));
    assert_eq!(hints.start_limit("identity-599", NOW + 600), Some(2));
    hints.flush(NOW + 600);
    assert_eq!(
        Hints::at(hints.path.clone().expect("path")).entries_len(NOW + 600),
        512
    );
}

impl Hints {
    fn entries_len(&mut self, now_ms: u64) -> usize {
        self.load(now_ms);
        self.entries.len()
    }
}

#[test]
fn speed_task_hints_key_includes_backend_identity() {
    // two connections whose paths read the same: the identity tells them apart
    let dir = tempfile::tempdir().expect("temp dir");
    let mut hints = hints_in(dir.path());
    hints.record("Sftp|alice@one.example", 32, NOW);
    hints.record("Sftp|alice@two.example", 4, NOW);
    assert_eq!(hints.entries.len(), 2);
    assert_eq!(hints.start_limit("Sftp|alice@one.example", NOW), Some(16));
    assert_eq!(hints.start_limit("Sftp|alice@two.example", NOW), Some(2));
    assert_ne!(
        hints.identity_hash("Sftp|alice@one.example", NOW),
        hints.identity_hash("Sftp|alice@two.example", NOW)
    );
    // and the path text alone is no identity
    assert_eq!(hints.start_limit("/data", NOW), None);
}

#[test]
fn speed_task_hints_salt_is_per_installation() {
    let one = tempfile::tempdir().expect("temp dir");
    let two = tempfile::tempdir().expect("temp dir");
    let mut first = hints_in(one.path());
    let mut second = hints_in(two.path());
    let key = |hints: &mut Hints| hints.identity_hash("Sftp|alice@files.example", NOW);
    assert_ne!(
        key(&mut first),
        key(&mut second),
        "equal keys would link installations"
    );
    assert_eq!(key(&mut first), key(&mut first), "stable within one");
}

#[test]
fn speed_task_hints_file_is_written_atomically() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut hints = hints_in(dir.path());
    hints.record("one", 8, NOW);
    let folder = hints
        .path
        .clone()
        .expect("path")
        .parent()
        .expect("parent")
        .to_path_buf();
    let names = |folder: &std::path::Path| -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(folder)
            .expect("folder")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    };
    // only the final file: the stage was renamed over it
    assert_eq!(names(&folder), vec![FILE_NAME.to_string()]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let file = std::fs::metadata(folder.join(FILE_NAME)).expect("file");
        assert_eq!(file.permissions().mode() & 0o777, 0o600);
        let parent = std::fs::metadata(&folder).expect("folder");
        assert_eq!(parent.permissions().mode() & 0o777, 0o700);
    }
    // a second change within the minute waits for the flush
    let before = text_of(&hints);
    hints.record("two", 8, NOW + 1_000);
    assert_eq!(text_of(&hints), before);
    hints.flush(NOW + 1_000);
    assert_ne!(text_of(&hints), before);
    assert_eq!(names(&folder), vec![FILE_NAME.to_string()]);
    // after the minute a change is written at once
    hints.record("three", 8, NOW + 62_000);
    let mut reader = Hints::at(hints.path.clone().expect("path"));
    assert_eq!(reader.start_limit("three", NOW + 62_001), Some(4));
}
