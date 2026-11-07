//! Per-key caps and the saving rules of the key bindings (R-SERVER-02, R-SERVER-03).

use std::path::PathBuf;

use super::{BindOutcome, Bindings, MAX_DEVICES_PER_KEY, MAX_LOOKUPS_PER_KEY, STATE_VERSION};

/// A private directory for the state file; removed when dropped.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let nonce = crate::login::nonce().expect("random nonce");
        let path = std::env::temp_dir().join(format!(
            "se-share-bindings-{label}-{}-{}",
            std::process::id(),
            crate::login::hex(&nonce)
        ));
        std::fs::create_dir(&path).expect("temporary directory");
        Self { path }
    }

    fn file(&self) -> PathBuf {
        self.path.join("bindings.json")
    }

    /// Everything in the directory, state file and leftovers alike.
    fn entries(&self) -> usize {
        std::fs::read_dir(&self.path)
            .expect("read directory")
            .count()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn hash(byte: &str) -> Option<String> {
    Some(byte.repeat(32))
}

#[test]
fn key_binding_caps_hold_per_key_and_per_kind() {
    // The documented values: a change of a cap is a decision, not an accident.
    assert_eq!(MAX_LOOKUPS_PER_KEY, 4_096);
    assert_eq!(MAX_DEVICES_PER_KEY, 64);

    let mut bindings = Bindings::default();
    for index in 0..MAX_LOOKUPS_PER_KEY {
        assert_eq!(
            bindings.bind_lookup(&format!("lookup-{index}"), "k1", None, 1),
            BindOutcome::Bound,
            "lookup {index}"
        );
    }
    let over = format!("lookup-{MAX_LOOKUPS_PER_KEY}");
    assert_eq!(
        bindings.bind_lookup(&over, "k1", None, 2),
        BindOutcome::KeyFull
    );
    assert!(bindings.is_new_lookup(&over), "a refusal binds nothing");
    // Another key is not affected, and the full key keeps what it holds.
    assert_eq!(
        bindings.bind_lookup("lookup-x", "k2", None, 2),
        BindOutcome::Bound
    );
    assert_eq!(
        bindings.bind_lookup("lookup-0", "k1", None, 3),
        BindOutcome::Confirmed
    );
    assert_eq!(
        bindings.bind_lookup("lookup-0", "k2", None, 3),
        BindOutcome::Conflict
    );

    // Devices count apart from lookups: the key with all lookups bound
    // still binds its device ids up to their own cap.
    for index in 0..MAX_DEVICES_PER_KEY {
        assert_eq!(
            bindings.bind_device(&format!("device-{index}"), "k1", 4),
            BindOutcome::Bound,
            "device {index}"
        );
    }
    let over = format!("device-{MAX_DEVICES_PER_KEY}");
    assert_eq!(bindings.bind_device(&over, "k1", 5), BindOutcome::KeyFull);
    assert!(bindings.is_new_device(&over), "a refusal binds nothing");
    assert_eq!(
        bindings.bind_device("device-0", "k1", 5),
        BindOutcome::Confirmed
    );
    assert_eq!(
        bindings.bind_device("device-x", "k2", 5),
        BindOutcome::Bound
    );
}

#[test]
fn key_counts_are_rebuilt_from_the_state_file() {
    let directory = TempDir::new("counts");
    let path = directory.file();
    let mut bindings = Bindings::default();
    for index in 0..3 {
        assert_eq!(
            bindings.bind_lookup(&format!("saved-{index}"), "k1", hash("11"), 1),
            BindOutcome::Bound
        );
    }
    bindings.save_pending(&path, 0).expect("save");

    let mut loaded = Bindings::load(&path, 0).expect("load");
    assert_eq!(loaded.lookup_key("saved-2"), Some("k1"));
    for index in 0..MAX_LOOKUPS_PER_KEY - 3 {
        assert_eq!(
            loaded.bind_lookup(&format!("more-{index}"), "k1", None, 2),
            BindOutcome::Bound,
            "lookup {index} after the restart"
        );
    }
    assert_eq!(
        loaded.bind_lookup("one-too-many", "k1", None, 2),
        BindOutcome::KeyFull
    );
    assert_eq!(
        loaded.bind_device("device", "k1", 2),
        BindOutcome::Bound,
        "the loaded lookups do not count as devices"
    );
}

#[test]
fn renewed_access_hashes_are_written_at_most_every_sixty_seconds() {
    let directory = TempDir::new("hash");
    let path = directory.file();
    let mut bindings = Bindings::default();
    assert_eq!(
        bindings.bind_lookup("l", "k", hash("11"), 0),
        BindOutcome::Bound
    );
    bindings.save_pending(&path, 0).expect("first save");
    let first = std::fs::read(&path).expect("state file");

    // The same hash again changes nothing: no write at all.
    assert_eq!(
        bindings.bind_lookup("l", "k", hash("11"), 1),
        BindOutcome::Confirmed
    );
    bindings.save_pending(&path, 1).expect("nothing due");
    assert_eq!(std::fs::read(&path).expect("state file"), first);

    // A renewed hash is valid at once in memory but not yet on disk.
    assert_eq!(
        bindings.bind_lookup("l", "k", hash("22"), 2),
        BindOutcome::Confirmed
    );
    bindings.save_pending(&path, 2).expect("not due");
    assert_eq!(bindings.lookup_access("l"), Some([0x22; 32]));
    assert_eq!(
        Bindings::load(&path, 2).expect("load").lookup_access("l"),
        Some([0x11; 32])
    );
    assert_eq!(directory.entries(), 1, "no temporary file is left");
    bindings.save_pending(&path, 59).expect("not due yet");
    assert_eq!(
        Bindings::load(&path, 59).expect("load").lookup_access("l"),
        Some([0x11; 32])
    );

    // Sixty seconds after the last save the pending hash is written.
    bindings.save_pending(&path, 60).expect("due");
    assert_eq!(
        Bindings::load(&path, 60).expect("load").lookup_access("l"),
        Some([0x22; 32])
    );
    assert_eq!(directory.entries(), 1, "no temporary file is left");
    assert_eq!(STATE_VERSION, 1, "the file format keeps its version");
}

#[test]
fn a_new_owner_is_written_at_once_inside_the_hash_interval() {
    let directory = TempDir::new("owner");
    let path = directory.file();
    let mut bindings = Bindings::default();
    assert_eq!(
        bindings.bind_lookup("l", "k", hash("11"), 0),
        BindOutcome::Bound
    );
    bindings.save_pending(&path, 0).expect("first save");

    assert_eq!(bindings.bind_device("d2", "k", 3), BindOutcome::Bound);
    bindings.save_pending(&path, 3).expect("owner save");
    let loaded = Bindings::load(&path, 3).expect("load");
    assert_eq!(loaded.device_key("d2"), Some("k"));
    assert_eq!(loaded.lookup_key("l"), Some("k"));
}

#[test]
fn a_failed_save_keeps_the_change_pending() {
    let directory = TempDir::new("failure");
    let missing = directory.path.join("missing").join("bindings.json");
    let path = directory.file();
    let mut bindings = Bindings::default();
    assert_eq!(bindings.bind_device("d", "k", 1), BindOutcome::Bound);

    assert!(bindings.save_pending(&missing, 1).is_err());
    assert_eq!(directory.entries(), 0, "nothing was written");

    bindings
        .save_pending(&path, 2)
        .expect("save after the failure");
    assert_eq!(
        Bindings::load(&path, 2).expect("load").device_key("d"),
        Some("k")
    );
    assert_eq!(directory.entries(), 1, "only the state file exists");
}
