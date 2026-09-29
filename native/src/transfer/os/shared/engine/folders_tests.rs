//! The folder register keeps a refusal of a busy peer only briefly: askers
//! right after it get the failure (counted once for the breaker), later ones
//! create the folder again.
use super::super::test_backend::{fwd, unique};
use super::{lock, FolderError, FolderRegister, Slot};
use crate::transfer::Side;
use std::io;
use std::sync::atomic::AtomicBool;
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
