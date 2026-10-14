//! Delete-speed task tests for WebDAV folder deletion against the scripted
//! loopback HTTP server of the transfer-engine tests: a folder goes with one
//! Depth-0 PROPFIND and one DELETE through the generic delete entry point, a
//! 207 answer is a partial failure that is not repeated, a file is refused
//! and a canceled run sends nothing.
use super::transfer_engine_task_tests::{propfind, Answer, Http};
use crate::vfs::{
    remove_entry_controlled, remove_tree, DeleteTarget, RecursiveDeletePhase, RecursiveDeleteStatus,
};
use std::io;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

type Log = Arc<Mutex<Vec<(String, String)>>>;

/// A server whose DELETE answers `delete_status`; `/file` is a file, every
/// other path a collection. The log holds every request after the connect.
fn serve(delete_status: u16) -> (Http, Log) {
    let log: Log = Arc::default();
    let seen = log.clone();
    let http = Http::start(move |request| {
        seen.lock()
            .unwrap()
            .push((request.method.clone(), request.path.clone()));
        match (request.method.as_str(), request.path.as_str()) {
            ("PROPFIND", "/file") => propfind("/file", Some(3)),
            ("PROPFIND", path) => propfind(path, None),
            ("DELETE", _) => Answer::status(delete_status),
            _ => Answer::status(500),
        }
    });
    (http, log)
}

fn requests(log: &Log) -> Vec<(String, String)> {
    log.lock().unwrap().clone()
}

fn request(method: &str, path: &str) -> (String, String) {
    (method.to_string(), path.to_string())
}

#[test]
fn delete_speed_task_webdav_folder_goes_with_one_delete() {
    let (http, log) = serve(204);
    let backend = http.backend();
    log.lock().unwrap().clear();
    let folder = DeleteTarget {
        path: "/folder".into(),
        id: None,
        is_dir: true,
        is_symlink: false,
    };
    let mut updates = Vec::new();
    let report = remove_entry_controlled(&backend, &folder, &AtomicBool::new(false), |update| {
        updates.push(update)
    })
    .unwrap();
    assert_eq!(report.status, RecursiveDeleteStatus::Complete);
    assert_eq!((report.planned, report.removed), (1, 1));
    // No listing and no request per member: the server removes the members.
    assert_eq!(
        requests(&log),
        [request("PROPFIND", "/folder"), request("DELETE", "/folder")]
    );
    let last = updates.last().expect("progress was reported");
    assert_eq!(last.phase, RecursiveDeletePhase::Applying);
    assert_eq!((last.planned, last.removed), (1, 1));
}

#[test]
fn delete_speed_task_webdav_multistatus_is_a_partial_failure_sent_once() {
    let (http, log) = serve(207);
    let backend = http.backend();
    log.lock().unwrap().clear();
    let failure = remove_tree(&backend, "/folder", &AtomicBool::new(false), &mut |_| {})
        .expect_err("207 reports members the server could not delete");
    assert_eq!((failure.planned, failure.removed), (1, 0));
    assert!(
        failure.error.to_string().contains("may have removed part"),
        "{}",
        failure.error
    );
    let deletes = requests(&log)
        .into_iter()
        .filter(|(method, _)| method == "DELETE")
        .count();
    assert_eq!(deletes, 1, "a DELETE is never repeated");
}

#[test]
fn delete_speed_task_webdav_refuses_a_file_and_honors_cancel() {
    let (http, log) = serve(204);
    let backend = http.backend();
    log.lock().unwrap().clear();
    let failure = remove_tree(&backend, "/file", &AtomicBool::new(false), &mut |_| {})
        .expect_err("a file is no collection");
    assert_eq!(failure.error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(requests(&log), [request("PROPFIND", "/file")]);

    log.lock().unwrap().clear();
    let report = remove_tree(&backend, "/folder", &AtomicBool::new(true), &mut |_| {}).unwrap();
    assert_eq!(report.status, RecursiveDeleteStatus::Canceled);
    assert_eq!(report.removed, 0);
    assert!(requests(&log).is_empty());
}

#[test]
fn delete_speed_task_webdav_lost_answer_with_the_folder_gone_is_complete() {
    let log: Log = Arc::default();
    let seen = log.clone();
    let http = Http::start(move |request| {
        let mut seen = seen.lock().unwrap();
        seen.push((request.method.clone(), request.path.clone()));
        let deleted = seen.iter().any(|(method, _)| method == "DELETE");
        match request.method.as_str() {
            // The answer of the DELETE is lost; the server removed the folder.
            "DELETE" => Answer::status(502),
            "PROPFIND" if deleted => Answer::status(404),
            "PROPFIND" => propfind(&request.path, None),
            _ => Answer::status(500),
        }
    });
    let backend = http.backend();
    log.lock().unwrap().clear();
    let report = remove_tree(&backend, "/folder", &AtomicBool::new(false), &mut |_| {}).unwrap();
    assert_eq!(report.status, RecursiveDeleteStatus::Complete);
    assert_eq!(
        requests(&log),
        [
            request("PROPFIND", "/folder"),
            request("DELETE", "/folder"),
            request("PROPFIND", "/folder")
        ]
    );
}
