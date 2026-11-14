//! Saves of the key bindings in generation order (SEC-SERVER-BESITZER-60S Teil 3):
//! overlapping copies leave the newer one, a restart reads the newer one, and a
//! failed write keeps its change pending.

use std::path::PathBuf;

use super::{BindOutcome, Bindings, SaveOrder};

/// A private directory for one state file; removed when dropped.
struct Dir {
    path: PathBuf,
}

impl Dir {
    fn new(label: &str) -> Self {
        let nonce = crate::login::nonce().expect("random nonce");
        let path = std::env::temp_dir().join(format!(
            "se-share-bindings-task-{label}-{}-{}",
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

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn server_rand_task_overlapping_saves_leave_the_newer_copy() {
    let dir = Dir::new("overlap");
    let path = dir.file();
    let mut bindings = Bindings::default();
    assert_eq!(bindings.bind_device("first", "k", 1), BindOutcome::Bound);
    let older = bindings.take_snapshot(1).expect("a save is due");
    assert_eq!(bindings.bind_device("second", "k", 2), BindOutcome::Bound);
    let newer = bindings.take_snapshot(2).expect("a save is due");

    // The newer copy lands first; the older copy must not overwrite it.
    let order = SaveOrder::default();
    order.write(&path, &newer).expect("newer copy");
    order.write(&path, &older).expect("older copy is skipped");
    let loaded = Bindings::load(&path, 3).expect("load");
    assert_eq!(
        loaded.device_key("second"),
        Some("k"),
        "the newer copy stays"
    );
    assert_eq!(loaded.device_key("first"), Some("k"));
    assert_eq!(dir.entries(), 1, "only the state file exists");
}

#[test]
fn server_rand_task_restart_reads_the_newer_copy_in_either_order() {
    let dir = Dir::new("restart");
    let path = dir.file();
    let mut bindings = Bindings::default();
    assert_eq!(bindings.bind_device("first", "k", 1), BindOutcome::Bound);
    let older = bindings.take_snapshot(1).expect("a save is due");
    assert_eq!(bindings.bind_device("second", "k", 2), BindOutcome::Bound);
    let newer = bindings.take_snapshot(2).expect("a save is due");

    // The older copy lands first; the newer one replaces it on disk.
    let order = SaveOrder::default();
    order.write(&path, &older).expect("older copy");
    order.write(&path, &newer).expect("newer copy");
    // A restart reads the newer copy back.
    let restarted = Bindings::load(&path, 3).expect("load after restart");
    assert_eq!(restarted.device_key("second"), Some("k"));
    assert_eq!(restarted.device_key("first"), Some("k"));
}

#[test]
fn server_rand_task_a_copy_keeps_the_moment_it_was_taken() {
    let dir = Dir::new("copy");
    let path = dir.file();
    let mut bindings = Bindings::default();
    assert_eq!(bindings.bind_device("first", "k", 1), BindOutcome::Bound);
    let snapshot = bindings.take_snapshot(1).expect("a save is due");

    // A change after the copy is pending again and is not part of the copy.
    assert_eq!(bindings.bind_device("later", "k", 2), BindOutcome::Bound);
    assert!(bindings.save_due(2), "the later change is due");
    assert!(bindings.take_snapshot(2).is_some());
    SaveOrder::default()
        .write(&path, &snapshot)
        .expect("write the copy");
    let loaded = Bindings::load(&path, 3).expect("load");
    assert_eq!(loaded.device_key("first"), Some("k"));
    assert_eq!(
        loaded.device_key("later"),
        None,
        "the copy predates the change"
    );
}

#[test]
fn server_rand_task_a_failed_write_keeps_the_copy_pending() {
    let dir = Dir::new("failed");
    let missing = dir.path.join("missing").join("bindings.json");
    let path = dir.file();
    let mut bindings = Bindings::default();
    assert_eq!(bindings.bind_device("device", "k", 1), BindOutcome::Bound);
    let failed = bindings.take_snapshot(1).expect("a save is due");
    let order = SaveOrder::default();
    assert!(order.write(&missing, &failed).is_err());
    assert_eq!(dir.entries(), 0, "nothing was written");

    // The caller restores the pending change; the next copy writes it.
    bindings.restore_pending();
    let retry = bindings.take_snapshot(2).expect("pending again");
    order.write(&path, &retry).expect("retry");
    assert_eq!(
        Bindings::load(&path, 2).expect("load").device_key("device"),
        Some("k")
    );
}
