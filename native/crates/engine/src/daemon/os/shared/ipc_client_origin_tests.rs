//! A reopened Share handle keeps the origin of the handle it stands for
//! (plan 2026-11-14, E4: metadata presence).
use super::UnavailableBackend;

#[test]
fn verbindungen_task_metadata_presence_a_reopened_handle_keeps_its_origin() {
    use crate::share::{OperationOrigin, PeerOpenTarget};
    let target = PeerOpenTarget::Direct {
        contact_id: "presence-reopen".into(),
    };
    for origin in [OperationOrigin::User, OperationOrigin::Job] {
        let backend = UnavailableBackend {
            label: "reopen".into(),
            target: target.clone(),
            origin,
        };
        let asked = std::cell::RefCell::new(Vec::new());
        let reopened = backend.reopen_with(&|target, origin| {
            asked.borrow_mut().push((target, origin));
            Err("not opened in this check".to_string())
        });
        assert!(reopened.is_err());
        assert_eq!(*asked.borrow(), vec![(target.clone(), origin)]);
    }
}
