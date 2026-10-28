//! The folder register keeps a refusal of a busy peer only briefly: askers
//! right after it get the failure (counted once for the breaker), later ones
//! create the folder again.
use super::super::test_backend::{fwd, unique, Fake};
use super::{lock, FolderError, FolderRegister, Slot};
use crate::transfer::Side;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[test]
fn transfer_engine_task_folder_refusals_are_not_kept() {
    let busy = FolderError::of(
        "/ziel/busy",
        &crate::vfs::congestion_error("zu viele", None),
    );
    assert!(busy.congestion && busy.connection, "a refusal is marked");
    let plain = FolderError::of("/ziel/x", &io::Error::from(io::ErrorKind::NotFound));
    assert!(!plain.congestion && !plain.connection);

    let root = tempfile::tempdir().expect("temp dir");
    let target = fwd(root.path());
    let flow = crate::transfer::flow(unique("refused-folder"), None);
    let register = FolderRegister::new(Side::Local, &target, flow);
    let cancel = AtomicBool::new(false);
    lock(&register.slots).insert(
        "busy".to_string(),
        Slot::Refused {
            error: busy.clone(),
            until: Instant::now() + Duration::from_secs(60),
        },
    );
    let early = register
        .ensure("busy", &cancel)
        .expect_err("askers right after the refusal get it");
    assert!(early.congestion, "{early:?}");
    assert!(!early.connection, "the breaker counted it once already");
    assert!(!root.path().join("busy").exists());

    lock(&register.slots).insert(
        "busy".to_string(),
        Slot::Refused {
            error: busy,
            until: Instant::now(),
        },
    );
    assert!(register
        .ensure("busy", &cancel)
        .expect("a later asker creates it again"));
    assert!(root.path().join("busy").is_dir());
}

/// Two workers whose bulk folder claims cross: worker 1 creates the folders
/// of a packet (`p/a`, `p/q/x`, so it claims `p`), worker 2 holds `p/q` and
/// waits for `p`. Worker 1 must not wait for `p/q` while it holds `p`: it
/// gives `p/q/x` back, creates `p` and `p/a`, and both workers finish.
#[test]
fn transfer_engine_task_bulk_folder_claims_never_wait_crosswise() {
    let root = tempfile::tempdir().expect("temp dir");
    let target = fwd(root.path());
    let mut fake = Fake::new("crossing-folders");
    fake.bulk_dirs = true;
    let (fake, _) = fake.handle();
    let flow = crate::transfer::flow(unique("crossing-folders"), None);
    let register = FolderRegister::new(Side::Remote(&*fake), &target, flow);
    let cancel = AtomicBool::new(false);
    // Worker 2 claimed `p/q` (its own chain stopped below `p`).
    lock(&register.slots).insert("p/q".to_string(), Slot::Creating);
    std::thread::scope(|scope| {
        let (done, finished) = mpsc::channel();
        let worker1_done = done.clone();
        let (register, cancel) = (&register, &cancel);
        scope.spawn(move || {
            register.ensure_many(&["p/a", "p/q/x"], cancel);
            let _ = worker1_done.send(1);
        });
        scope.spawn(move || {
            // Worker 2 asks for `p` only once worker 1 holds or settled it:
            // the order that waited crosswise before.
            while !lock(&register.slots).contains_key("p") {
                std::thread::yield_now();
            }
            let parent = register.ensure("p", cancel);
            let created = root_dir_created(register, "p/q");
            register.settle_slot("p/q", &created);
            register.changed.notify_all();
            let _ = done.send(if parent.is_ok() { 2 } else { 0 });
        });
        // A bound only so a regression fails instead of hanging the suite.
        let mut seen = Vec::new();
        for _ in 0..2 {
            match finished.recv_timeout(Duration::from_secs(120)) {
                Ok(worker) => seen.push(worker),
                Err(_) => {
                    cancel.store(true, Ordering::Release);
                    panic!("the folder workers wait for each other ({seen:?})");
                }
            }
        }
        seen.sort_unstable();
        assert_eq!(seen, vec![1, 2], "both workers finish and find `p`");
    });
    assert!(matches!(
        lock(&register.slots).get("p/a"),
        Some(Slot::Ready { .. })
    ));
    assert!(!matches!(
        lock(&register.slots).get("p/q/x"),
        Some(Slot::Creating)
    ));
    register
        .ensure("p/q/x", &cancel)
        .expect("the folder given back is created when its file asks");
    for rel in ["p", "p/a", "p/q", "p/q/x"] {
        assert!(root.path().join(rel).is_dir(), "{rel}");
    }
}

/// Worker 2's own creation of `p/q`, once `p` exists.
fn root_dir_created(register: &FolderRegister<'_>, rel: &str) -> Result<bool, FolderError> {
    let path = register.path_of(rel);
    std::fs::create_dir(&path)
        .map(|()| true)
        .map_err(|error| FolderError::of(&path, &error))
}
