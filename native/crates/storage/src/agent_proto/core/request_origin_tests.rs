//! The background mark of one request (E4d). Remote suite only (AGENTS.md).
use super::*;
use crate::agent_proto::ServerFeatures;

/// The client marks a request only on a thread that runs background work
/// and only towards a server that reads the mark; the server's scope holds
/// for the one request it answers.
#[test]
fn verbindungen_task_metadata_presence_the_mark_belongs_to_one_request() {
    assert_eq!(request_id(5, true), 5, "a person's request");
    {
        let _work = crate::vfs::BackgroundScope::enter();
        let marked = request_id(5, true);
        assert_eq!(marked, 5 | BACKGROUND_REQUEST, "background work");
        assert!(is_background_request(marked));
        assert_eq!(request_id(5, false), 5, "a server without the label");
    }
    assert!(!is_background_request(5));
    assert!(request_scope(5).is_none(), "no mark: the handle's origin");
    assert!(!crate::vfs::in_background());
    let scope = request_scope(5 | BACKGROUND_REQUEST);
    assert!(scope.is_some());
    assert!(crate::vfs::in_background(), "while the request is answered");
    drop(scope);
    assert!(!crate::vfs::in_background(), "ends with that request");
}

/// The label in a version text: the service of this build announces it, the
/// service of v0.5.196 did not, and its absence reads as „no mark“.
#[test]
fn verbindungen_task_metadata_presence_only_a_server_with_the_label_reads_the_mark() {
    let old = "0.5.196+sync-links-v1.credit-v1.stage-v1.ext-v1.walk-meta-v1.walk-filter-v1.slots-64.capacity-v1.folder-reservation-v1.file-identity-v1.list-meta-v1.walk-keys-v1.meta-list-v1 worker";
    assert!(!ServerFeatures::parse(old).origin);
    let new = format!("{}.{LABEL_ORIGIN} worker", old.trim_end_matches(" worker"));
    let features = ServerFeatures::parse(&new);
    assert!(features.origin);
    assert!(features.service && features.credit, "the other labels stay");
}
