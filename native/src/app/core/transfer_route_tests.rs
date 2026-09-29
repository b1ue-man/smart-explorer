//! Every copy entry point routes to one engine job with the same rules.
use super::super::transfer_test_support::remote;
use super::*;
use crate::transfer::TransferKind;
use std::sync::Arc;

fn local_roots(paths: &[&str]) -> TransferSelection {
    TransferSelection::roots(
        TransferPlace::local(),
        paths.iter().map(|path| path.to_string()).collect(),
        Some("/data".to_string()),
    )
}

#[test]
fn transfer_engine_task_paste_routes_every_side_combination() {
    let (one, two) = (remote("sftp://a@one:22"), remote("sftp://b@two:22"));
    let local = TransferPlace::local();
    let first = TransferPlace::remote(one.clone(), "Server A");
    let second = TransferPlace::remote(two.clone(), "Server B");
    let from_local = local_roots(&["/data/a.txt", "/data/folder"]);
    let from_remote = TransferSelection::roots(first.clone(), vec!["/docs/x".into()], None);
    let cases = [
        (&from_local, &local, "/copy", TransferKind::Local),
        (&from_local, &second, "/in", TransferKind::Upload),
        (&from_remote, &local, "/downloads", TransferKind::Download),
        (&from_remote, &second, "/docs", TransferKind::RemoteCopy),
        (&from_remote, &first, "/other", TransferKind::RemoteCopy),
    ];
    for (selection, target, dir, kind) in cases {
        let job = paste_job(selection, target, dir, CopyMode::Copy).expect("paste job");
        assert_eq!(job.kind(), kind);
        assert_eq!(job.target_dir, dir);
        assert_eq!(job.layout, Layout::Tree);
        assert_eq!(job.conflict, Conflict::Rename, "paste never replaces");
        assert!(job.resume.is_none());
        assert_eq!(job.items, selection.items);
    }
    let upload = paste_job(&from_local, &second, "/in", CopyMode::Copy).expect("upload");
    assert_eq!(upload.source_label, "/data");
    assert_eq!(upload.target_label, "Server B: /in");
    assert!(upload
        .target
        .backend()
        .is_some_and(|backend| Arc::ptr_eq(backend, &two)));
}

#[test]
fn transfer_engine_task_move_only_between_local_folders() {
    let selection = local_roots(&["/data/a.txt"]);
    let job = paste_job(
        &selection,
        &TransferPlace::local(),
        "/moved",
        CopyMode::Move,
    )
    .expect("local move");
    assert_eq!(job.kind(), TransferKind::Move);
    let target = TransferPlace::remote(remote("sftp://a@one:22"), "Server A");
    assert_eq!(
        paste_job(&selection, &target, "/in", CopyMode::Move).err(),
        Some(REMOTE_MOVE_REFUSED.to_string())
    );
    let from_remote = TransferSelection::roots(target, vec!["/docs/x".into()], None);
    assert!(paste_job(&from_remote, &TransferPlace::local(), "/d", CopyMode::Move).is_err());
}

#[test]
fn transfer_engine_task_folder_into_itself_is_refused_for_every_side() {
    let local = local_roots(&["/data/folder"]);
    let Err(error) = paste_job(
        &local,
        &TransferPlace::local(),
        "/data/folder/sub",
        CopyMode::Copy,
    ) else {
        panic!("a local folder copied into itself must be refused");
    };
    assert!(
        error.contains("Das Ziel liegt in einer der Quellen"),
        "{error}"
    );
    // Two handles to one account are one namespace.
    let source = TransferPlace::remote(remote("sftp://a@one:22"), "A");
    let same_account = TransferPlace::remote(remote("sftp://a@one:22"), "A (2)");
    let selection = TransferSelection::roots(source, vec!["/docs".into()], None);
    assert!(paste_job(&selection, &same_account, "/docs/inner", CopyMode::Copy).is_err());
    assert!(paste_job(&selection, &same_account, "/docs", CopyMode::Copy).is_err());
    assert!(paste_job(&selection, &same_account, "/other", CopyMode::Copy).is_ok());
    assert!(paste_job(&selection, &TransferPlace::local(), "", CopyMode::Copy).is_err());
}

#[test]
fn transfer_engine_task_picked_remote_target_keeps_its_connection() {
    // "Herunterladen nach…" into a folder of another remote with the same
    // relative path: two places, never a copy into itself (K28).
    let first = remote("sftp://a@one:22");
    let second = remote("webdav://b@two:443");
    let source = TransferPlace::remote(first.clone(), "Eins");
    let picked = TransferPlace::remote(second.clone(), "Zwei");
    assert!(!source.same_place(&picked));
    let selection = TransferSelection::roots(source.clone(), vec!["/docs/x".into()], None);
    let job = paste_job(&selection, &picked, "/docs/x", CopyMode::Copy).expect("other remote");
    assert_eq!(job.kind(), TransferKind::RemoteCopy);
    assert!(job
        .source
        .backend()
        .is_some_and(|backend| Arc::ptr_eq(backend, &first)));
    assert!(job
        .target
        .backend()
        .is_some_and(|backend| Arc::ptr_eq(backend, &second)));
    assert_eq!(job.target_label, "Zwei: /docs/x");
    assert!(paste_job(&selection, &source, "/docs/x", CopyMode::Copy).is_err());
}

#[test]
fn transfer_engine_task_dialog_job_keeps_conflict_policy_and_layout() {
    let selection =
        local_roots(&["/data/folder"]).with_filter(Some((FilterDef::new(), "/data".to_string())));
    let flat = dialog_job(
        &selection,
        "/out",
        false,
        Conflict::Overwrite,
        CopyMode::Copy,
    )
    .expect("dialog job");
    assert_eq!(flat.layout, Layout::Flatten);
    assert_eq!(flat.conflict, Conflict::Overwrite);
    assert!(flat.target.is_local());
    assert!(flat.filter.is_some());
    let tree =
        dialog_job(&selection, "/out", true, Conflict::Skip, CopyMode::Move).expect("dialog move");
    assert_eq!(tree.layout, Layout::Tree);
    assert_eq!(tree.kind(), TransferKind::Move);
}

#[test]
fn transfer_engine_task_resume_job_targets_the_resolved_roots() {
    let selection = local_roots(&["/data/a", "/data/b"]);
    let target = TransferPlace::remote(remote("sftp://a@one:22"), "A");
    let job = paste_job(&selection, &target, "/in", CopyMode::Copy).expect("job");
    let roots = vec![ResolvedRoot {
        source: "/data/a".into(),
        rel: "a (2)".into(),
    }];
    let resumed = resume_job(&job, &roots);
    assert_eq!(resumed.resume, Some(roots));
    assert_eq!(resumed.items, job.items);
    assert_eq!(resumed.target_dir, job.target_dir);
    assert_eq!(resumed.conflict, job.conflict);
    assert!(job.resume.is_none());
}

#[test]
fn transfer_engine_task_parent_dir_keeps_roots() {
    assert_eq!(parent_dir("/a/b"), "/a");
    assert_eq!(parent_dir("/a"), "/");
    assert_eq!(parent_dir("C:/x"), "C:/");
    assert_eq!(parent_dir("C:/x/y/"), "C:/x");
    assert_eq!(parent_dir("//server/share/x"), "//server/share");
    assert_eq!(parent_dir("name"), "");
}
