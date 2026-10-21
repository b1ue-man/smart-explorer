//! Acceptance signals of plan U46-7: how many requests a new file costs on
//! WebDAV, and that the safety of not replacing survives every shortening.
//! The engine path is a private stage (`stage_writer`) published by
//! `promote_copy_stage`; the sync path adds `finish_stage` and its time.
use super::request_count_server::{lock, Server};
use super::WebdavBackend;
use crate::vfs::{finish_stage, open_write_copy_stage_timed, Backend, StageFinish};
use std::io::{self, Write};
use std::sync::atomic::Ordering;

const MTIME: i64 = 1_700_000_000;

/// What the engine does for a new file in a folder it created: a private
/// stage (`stage_writer`), then `promote_copy_stage` without replacing.
fn upload_new_files(backend: &WebdavBackend, count: usize) {
    backend.create_dir("/dir").expect("MKCOL");
    for index in 0..count {
        let stage = format!("/dir/f{index}.bin.se-upload-{index:016x}");
        let mut writer = backend
            .open_write_copy_stage_unsynced(&stage, 4)
            .expect("stage");
        writer.write_all(b"data").expect("write");
        writer.flush().expect("PUT");
        drop(writer);
        backend
            .promote_copy_stage(&stage, &format!("/dir/f{index}.bin"))
            .expect("publish");
    }
}

#[test]
fn speed_task_webdav_requests_per_new_file_at_most_three_plus_folder() {
    let server = Server::start();
    let backend = server.backend();
    let dav = &server.dav;
    let connected = lock(&dav.log).len();
    upload_new_files(&backend, 20);
    assert_eq!((dav.count("PUT"), dav.count("MOVE")), (20, 20));
    assert_eq!(dav.count("MKCOL"), 1);
    let total = lock(&dav.log).len() - connected;
    let mean = total as f64 / 20.0;
    assert!(
        mean <= 3.2,
        "{mean} requests per file: {:?}",
        lock(&dav.log)
            .iter()
            .map(|r| r.method.clone())
            .collect::<Vec<_>>()
    );
    // No DELETE, GET or HEAD: nothing is read back or cleaned up.
    assert_eq!(
        dav.count("DELETE") + dav.count("GET") + dav.count("HEAD"),
        0
    );
    assert_eq!(lock(&dav.files).len(), 20);
}

#[test]
fn speed_task_webdav_put_keeps_if_none_match() {
    let server = Server::start();
    upload_new_files(&server.backend(), 3);
    let puts = server.dav.requests("PUT");
    assert_eq!(puts.len(), 3);
    for (path, headers) in puts {
        assert_eq!(
            headers.get("if-none-match").map(String::as_str),
            Some("*"),
            "{path}"
        );
    }
}

#[test]
fn speed_task_webdav_move_keeps_overwrite_false() {
    let server = Server::start();
    let backend = server.backend();
    upload_new_files(&backend, 3);
    for (path, headers) in server.dav.requests("MOVE") {
        assert_eq!(
            headers.get("overwrite").map(String::as_str),
            Some("F"),
            "{path}"
        );
    }
    // A taken name is refused by the server and nothing is replaced.
    let mut writer = backend
        .open_write_copy_stage_unsynced("/dir/again.se-upload-1", 3)
        .expect("stage");
    writer.write_all(b"new").expect("write");
    writer.flush().expect("PUT");
    drop(writer);
    let refused = backend
        .promote_copy_stage("/dir/again.se-upload-1", "/dir/f0.bin")
        .expect_err("taken");
    assert_eq!(refused.kind(), io::ErrorKind::AlreadyExists, "{refused}");
    assert_eq!(lock(&server.dav.files)["/dir/f0.bin"].0, b"data");
}

/// One sync-style publication: a timed stage, `finish_stage`, then the
/// no-replace rename. Returns the PROPFIND count of the destination and the
/// result of the rename.
fn publish_with_time(server: &Server) -> (usize, io::Result<()>) {
    let backend = server.backend();
    let mut writer = open_write_copy_stage_timed(&backend, "/s1", 4, MTIME * 1_000).expect("stage");
    writer.write_all(b"data").expect("write");
    writer.flush().expect("PUT");
    drop(writer);
    let finish = StageFinish {
        mtime_ms: Some(MTIME * 1_000),
        ..StageFinish::default()
    };
    assert!(
        finish_stage(&backend, "/s1", finish)
            .expect("finish")
            .mtime_applied
    );
    let renamed = backend.rename_no_replace("/s1", "/d1");
    let confirmations = server
        .dav
        .requests("PROPFIND")
        .iter()
        .filter(|(path, _)| path == "/d1")
        .count();
    (confirmations, renamed)
}

#[test]
fn speed_task_webdav_confirmation_skipped_only_when_response_proves_it() {
    // The MOVE answer does not prove the time: the confirmation stays.
    let server = Server::start();
    let (confirmations, renamed) = publish_with_time(&server);
    renamed.expect("the time was kept");
    assert_eq!(confirmations, 1);
    let (_, headers) = server.dav.requests("MOVE").remove(0);
    assert_eq!(
        headers.get("x-oc-mtime").map(String::as_str),
        Some("1700000000")
    );
    assert_eq!(headers.get("overwrite").map(String::as_str), Some("F"));
    assert!(headers.contains_key("if-match"));
    // Without proof a server that forgot the time is still caught.
    let server = Server::start();
    server.dav.move_forgets_time.store(true, Ordering::SeqCst);
    let (_, renamed) = publish_with_time(&server);
    assert_eq!(
        renamed.expect_err("time lost").kind(),
        io::ErrorKind::Unsupported
    );
    // The MOVE answer proves it (`X-OC-Mtime: accepted`): no extra request.
    let server = Server::start();
    server.dav.move_proof.store(true, Ordering::SeqCst);
    let (confirmations, renamed) = publish_with_time(&server);
    renamed.expect("accepted");
    assert_eq!(confirmations, 0);
}
