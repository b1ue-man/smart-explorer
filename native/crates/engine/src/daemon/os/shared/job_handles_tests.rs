//! A job of the app that runs in this process does not carry the metadata
//! presence of the handle a person browses with (plan 2026-11-14, E4).
use super::{job_sides_with, remember_origin, user_target};
use crate::share::{OperationOrigin, PeerOpenTarget};
use crate::transfer::{
    FolderConflict, FolderConflictPolicy, JobItems, JobView, Layout, RecoveryMode, Side,
};
use crate::types::{Conflict, CopyMode};
use crate::vfs::BackendHandle;
use std::cell::RefCell;
use std::sync::Arc;

fn device(contact: &str) -> PeerOpenTarget {
    PeerOpenTarget::Direct {
        contact_id: contact.to_string(),
    }
}

fn handle() -> BackendHandle {
    Arc::new(crate::vfs::LocalBackend::new("/"))
}

fn view<'a>(
    source: Side<'a>,
    target: Side<'a>,
    items: &'a JobItems,
    folders: &'a FolderConflictPolicy,
) -> JobView<'a> {
    JobView {
        id: Some("presence-task"),
        recoverable: true,
        recovery: RecoveryMode::New,
        source,
        target,
        target_dir: "/to",
        items,
        layout: Layout::Tree,
        filter: None,
        conflict: Conflict::Rename,
        folders,
        mode: CopyMode::Copy,
        source_label: "Quelle",
        target_label: "Ziel",
        resume: None,
    }
}

fn items() -> (JobItems, FolderConflictPolicy) {
    (
        JobItems::Roots {
            paths: vec!["/from/a".into()],
            base: None,
        },
        FolderConflictPolicy::new(FolderConflict::KeepBoth),
    )
}

#[test]
fn verbindungen_task_metadata_presence_user_handles_are_known_through_a_cache() {
    let browsing = handle();
    remember_origin(&browsing, &device("presence-known"), OperationOrigin::User);
    assert_eq!(user_target(&*browsing), Some(device("presence-known")));
    let cached: BackendHandle = Arc::new(crate::vfs::CachingBackend::new(browsing.clone()));
    assert_eq!(user_target(&*cached), Some(device("presence-known")));
    // A handle of the job engine is never listed, whatever device it reaches.
    let job = handle();
    remember_origin(&job, &device("presence-known"), OperationOrigin::Job);
    assert_eq!(user_target(&*job), None);
    drop(cached);
    drop(browsing);
    assert_eq!(user_target(&*job), None);
}

#[test]
fn verbindungen_task_metadata_presence_a_job_replaces_only_the_browsing_handle() {
    let (items, folders) = items();
    let browsing = handle();
    remember_origin(&browsing, &device("presence-source"), OperationOrigin::User);
    let own_job_handle = handle();
    remember_origin(
        &own_job_handle,
        &device("presence-target"),
        OperationOrigin::Job,
    );
    let fresh = handle();
    let opened = RefCell::new(Vec::new());
    let sides = job_sides_with(
        &view(
            Side::Remote(&*browsing),
            Side::Remote(&*own_job_handle),
            &items,
            &folders,
        ),
        &|target| {
            opened.borrow_mut().push(target.clone());
            Some(fresh.clone())
        },
    );
    assert!(Arc::ptr_eq(sides.source.as_ref().expect("source"), &fresh));
    assert!(sides.target.is_none(), "a handle of the origin Job stays");
    assert_eq!(*opened.borrow(), vec![device("presence-source")]);
    // Nothing a person browses with, nothing replaced and nothing opened.
    opened.borrow_mut().clear();
    let sides = job_sides_with(
        &view(
            Side::Local,
            Side::Remote(&*own_job_handle),
            &items,
            &folders,
        ),
        &|target| {
            opened.borrow_mut().push(target.clone());
            Some(fresh.clone())
        },
    );
    assert!(sides.is_empty());
    assert!(opened.borrow().is_empty());
}

#[test]
fn verbindungen_task_metadata_presence_a_job_on_one_device_opens_one_handle() {
    let (items, folders) = items();
    let source = handle();
    let target = handle();
    remember_origin(&source, &device("presence-same"), OperationOrigin::User);
    remember_origin(&target, &device("presence-same"), OperationOrigin::User);
    let fresh = handle();
    let opens = RefCell::new(0usize);
    let sides = job_sides_with(
        &view(
            Side::Remote(&*source),
            Side::Remote(&*target),
            &items,
            &folders,
        ),
        &|_| {
            *opens.borrow_mut() += 1;
            Some(fresh.clone())
        },
    );
    assert_eq!(*opens.borrow(), 1);
    assert!(Arc::ptr_eq(sides.source.as_ref().expect("source"), &fresh));
    assert!(Arc::ptr_eq(sides.target.as_ref().expect("target"), &fresh));
}

#[test]
fn verbindungen_task_metadata_presence_a_job_keeps_its_handle_when_none_opens() {
    let (items, folders) = items();
    let browsing = handle();
    remember_origin(&browsing, &device("presence-closed"), OperationOrigin::User);
    let sides = job_sides_with(
        &view(Side::Remote(&*browsing), Side::Local, &items, &folders),
        &|_| None,
    );
    assert!(
        sides.is_empty(),
        "the job runs over the handle it was given"
    );
}
