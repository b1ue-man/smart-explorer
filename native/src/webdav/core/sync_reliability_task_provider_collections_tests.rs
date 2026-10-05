//! Collection creation uses one canonical mutation and fresh collision proof.
use super::transfer_engine_task_tests::{propfind, Answer, Http};
use crate::vfs::{congestion_of, Backend};
use std::io;

#[test]
fn sync_reliability_task_provider_dav_collection_literal_creation_and_existing_names() {
    let http = Http::start(|seen| match (seen.method.as_str(), seen.path.as_str()) {
        ("PROPFIND", path) => propfind(path, None),
        ("MKCOL", "/exists/") => Answer::status(405),
        ("MKCOL", "/literal%2520-folder/") | ("MKCOL", "/already-slash/") => Answer::status(201),
        _ => Answer::status(500),
    });
    let backend = http.backend();
    let identity = backend.state_identity();
    backend.create_dir("/literal%20-folder").unwrap();
    backend.create_dir_new("/already-slash/").unwrap();
    backend.create_dir("/exists").unwrap();
    assert_eq!(
        backend.create_dir_new("/exists").unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(backend.state_identity(), identity);
    drop(backend);
    assert_eq!(http.connections("MKCOL", "/literal%2520-folder/").len(), 1);
    assert!(http
        .connections("PROPFIND", "/literal%2520-folder")
        .is_empty());
    assert_eq!(http.connections("MKCOL", "/already-slash/").len(), 1);
    assert_eq!(http.connections("MKCOL", "/exists/").len(), 2);
    assert_eq!(http.connections("PROPFIND", "/exists").len(), 1);
}

#[test]
fn sync_reliability_task_provider_dav_collection_file_collision_needs_original_resource_proof() {
    for status in [404, 409] {
        let http = Http::start(
            move |seen| match (seen.method.as_str(), seen.path.as_str()) {
                ("PROPFIND", "/file") => propfind("/file", Some(7)),
                ("PROPFIND", path) => propfind(path, None),
                ("MKCOL", "/file/") => Answer::status(status),
                _ => Answer::status(500),
            },
        );
        let backend = http.backend();
        for exclusive in [false, true] {
            let error = if exclusive {
                backend.create_dir_new("/file").unwrap_err()
            } else {
                backend.create_dir("/file").unwrap_err()
            };
            assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        }
        drop(backend);
        assert_eq!(http.connections("MKCOL", "/file/").len(), 2);
        assert_eq!(http.connections("PROPFIND", "/file").len(), 2);
        assert!(http.connections("PROPFIND", "/file/").is_empty());
    }
}

#[test]
fn sync_reliability_task_provider_dav_collection_errors_remain_terminal() {
    for status in [301, 403, 404, 409, 503, 507] {
        for probe in [403, 404, 503] {
            let http = Http::start(
                move |seen| match (seen.method.as_str(), seen.path.as_str()) {
                    ("PROPFIND", "/") => propfind("/", None),
                    ("PROPFIND", "/denied") => Answer::status(probe),
                    ("MKCOL", "/denied/") => {
                        Answer::status(status).with("Location", "/unexpected/")
                    }
                    _ => Answer::status(500),
                },
            );
            let backend = http.backend();
            for exclusive in [false, true] {
                let result = if exclusive {
                    backend.create_dir_new("/denied")
                } else {
                    backend.create_dir("/denied")
                };
                let error = result.unwrap_err();
                assert_ne!(error.kind(), io::ErrorKind::AlreadyExists);
                if status == 403 || (matches!(status, 404 | 409) && probe == 403) {
                    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
                }
                if status == 503 || (matches!(status, 404 | 409) && probe == 503) {
                    assert!(congestion_of(&error).is_some());
                }
                if status == 507 {
                    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
                }
            }
            drop(backend);
            assert_eq!(http.connections("MKCOL", "/denied/").len(), 2);
            assert!(http.connections("MKCOL", "/unexpected/").is_empty());
            assert!(http.connections("GET", "/unexpected/").is_empty());
            let expected_probes = if matches!(status, 404 | 409) { 2 } else { 0 };
            assert_eq!(
                http.connections("PROPFIND", "/denied").len(),
                expected_probes
            );
        }
    }
}

#[test]
fn sync_reliability_task_provider_dav_collection_lost_ack_does_not_repeat_mutation() {
    let http = Http::start(|seen| match (seen.method.as_str(), seen.path.as_str()) {
        ("PROPFIND", "/") => propfind("/", None),
        ("MKCOL", "/lost/") => Answer::disconnect(),
        _ => Answer::status(500),
    });
    let backend = http.backend();
    assert!(backend.create_dir("/lost").is_err());
    drop(backend);
    assert_eq!(http.connections("MKCOL", "/lost/").len(), 1);
    assert!(http.connections("PROPFIND", "/lost").is_empty());
}
