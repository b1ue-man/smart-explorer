//! `content_digest` over WebDAV: the SHA-256 that ownCloud and Nextcloud
//! report in `oc:checksums` comes back from one PROPFIND, the file is not
//! read (no GET), and a server without a SHA-256 entry answers `Ok(None)`.
use super::super::content_digest::{from_propfind, md5_from_propfind, sha256_token};
use super::request_count_server::{lock, Server};
use crate::vfs::{content_digest, content_md5};
use std::sync::atomic::AtomicBool;

const HEX: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

fn put(server: &Server, path: &str, checksums: Option<&str>) {
    lock(&server.dav.files).insert(path.to_string(), (b"abcde".to_vec(), 1_700_000_000));
    if let Some(text) = checksums {
        lock(&server.dav.checksums).insert(path.to_string(), text.to_string());
    }
}

#[test]
fn abgleich_task_webdav_digest_comes_from_the_checksum_property_without_a_download() {
    let server = Server::start();
    let backend = server.backend();
    put(
        &server,
        "/a.bin",
        Some(&format!(
            "SHA1:da39a3ee5e6b4b0d3255bfef95601890afd80709 SHA256:{HEX}"
        )),
    );
    let found = content_digest(&backend, "/a.bin", &AtomicBool::new(false))
        .unwrap()
        .expect("the server stores a SHA-256");
    assert_eq!(found.sha256, HEX);
    assert_eq!(found.size, 5);
    assert_eq!(server.dav.count("GET"), 0, "the content was not read");
    assert_eq!(server.dav.count("PUT"), 0);
    // Case "MD5 property": an MD5 from the checksum property needs no download.
    {
        let server = Server::start();
        let backend = server.backend();
        put(
            &server,
            "/a.bin",
            Some(&format!(
                "SHA1:da39a3ee5e6b4b0d3255bfef95601890afd80709 MD5:{} SHA256:{HEX}",
                MD5.to_ascii_uppercase()
            )),
        );
        let asked = server.dav.count("PROPFIND");
        let found = content_md5(&backend, "/a.bin", &AtomicBool::new(false))
            .unwrap()
            .expect("the server stores an MD5");
        assert_eq!(found, MD5, "lowercase hex");
        assert_eq!(server.dav.count("GET"), 0, "the content was not read");
        assert_eq!(server.dav.count("PUT"), 0);
        assert_eq!(server.dav.count("PROPFIND"), asked + 1, "one question");
    }
}

#[test]
fn abgleich_task_webdav_digest_needs_a_sha256_entry() {
    let server = Server::start();
    let backend = server.backend();
    put(
        &server,
        "/md5.bin",
        Some("SHA1:da39a3ee5e6b4b0d3255bfef95601890afd80709 MD5:d41d8cd98f00b204e9800998ecf8427e"),
    );
    put(&server, "/none.bin", None);
    for path in ["/md5.bin", "/none.bin"] {
        let found = content_digest(&backend, path, &AtomicBool::new(false)).unwrap();
        assert_eq!(found, None, "{path}");
    }
    assert_eq!(server.dav.count("GET"), 0);
    // Case "MD5 needs an MD5 entry": a SHA-256 entry, a short MD5 and no property give no MD5.
    {
        let server = Server::start();
        let backend = server.backend();
        put(&server, "/sha.bin", Some(&format!("SHA256:{HEX}")));
        put(&server, "/short.bin", Some("MD5:abc"));
        put(&server, "/none.bin", None);
        for path in ["/sha.bin", "/short.bin", "/none.bin"] {
            let found = content_md5(&backend, path, &AtomicBool::new(false)).unwrap();
            assert_eq!(found, None, "{path}");
        }
        assert_eq!(server.dav.count("GET"), 0, "MD5 needs an MD5 entry");
    }
}

#[test]
fn abgleich_task_webdav_digest_tokens_parse_strictly() {
    assert_eq!(
        sha256_token(&format!("md5:aa SHA-256:{}", HEX.to_ascii_uppercase())).as_deref(),
        Some(HEX)
    );
    assert_eq!(sha256_token("SHA256:abc"), None, "too short");
    assert_eq!(
        sha256_token(&format!("SHA3-256:{HEX}")),
        None,
        "another algorithm"
    );
    assert_eq!(sha256_token(&format!("SHA256{HEX}")), None, "no separator");
}

#[test]
fn abgleich_task_webdav_digest_of_a_collection_is_refused() {
    let xml = r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:"><d:response><d:href>/x/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
    assert_eq!(
        from_propfind(xml).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
    // Case "MD5 of a collection": a collection gives no MD5, and a cancel asks nothing.
    {
        assert_eq!(
            md5_from_propfind(xml).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput,
            "MD5 of a collection"
        );
        let server = Server::start();
        let backend = server.backend();
        put(&server, "/a.bin", Some(&format!("MD5:{MD5}")));
        let before = server.dav.count("PROPFIND");
        let error = content_md5(&backend, "/a.bin", &AtomicBool::new(true)).unwrap_err();
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::Interrupted,
            "a cancel asks nothing"
        );
        assert_eq!(
            server.dav.count("PROPFIND"),
            before,
            "a cancel asks nothing"
        );
    }
}

const MD5: &str = "5d41402abc4b2a76b9719d911017c592";

/// A server without `oc:checksums` (Apache `mod_dav`, nginx, IIS): the first
/// answer shows none, so the connection asks no more PROPFIND for a MD5 or
/// a SHA-256 it cannot give; a server that carries them is asked each time.
#[test]
fn abgleich_task_webdav_without_checksums_asks_once_per_connection() {
    let server = Server::start();
    let backend = server.backend();
    for name in ["/a.bin", "/b.bin", "/c.bin"] {
        put(&server, name, None);
    }
    let cancel = AtomicBool::new(false);
    let before = server.dav.count("PROPFIND");
    for name in ["/a.bin", "/b.bin", "/c.bin"] {
        assert_eq!(content_md5(&backend, name, &cancel).unwrap(), None);
        assert_eq!(content_digest(&backend, name, &cancel).unwrap(), None);
    }
    assert_eq!(
        server.dav.count("PROPFIND"),
        before + 2,
        "one answer per algorithm, then no more questions"
    );
    assert_eq!(server.dav.count("GET"), 0);

    // Another connection to a server that carries them asks every file.
    let carrying = Server::start();
    let backend = carrying.backend();
    for name in ["/a.bin", "/b.bin"] {
        put(&carrying, name, Some(&format!("MD5:{MD5} SHA256:{HEX}")));
    }
    let before = carrying.dav.count("PROPFIND");
    for name in ["/a.bin", "/b.bin"] {
        assert_eq!(
            content_md5(&backend, name, &cancel).unwrap().as_deref(),
            Some(MD5)
        );
    }
    assert_eq!(carrying.dav.count("PROPFIND"), before + 2);
    // Case "listing without MD5": a listing without an MD5 spares the question; a folder says nothing.
    {
        use super::super::content_digest::ChecksumMemory;
        use crate::vfs::VfsMeta;
        let file = |md5: Option<&str>| VfsMeta {
            name: "f".into(),
            size: 5,
            content_md5: md5.map(str::to_string),
            ..VfsMeta::default()
        };
        let folder = VfsMeta {
            name: "d".into(),
            is_dir: true,
            ..VfsMeta::default()
        };
        let memory = ChecksumMemory::default();
        memory.note_listing(std::slice::from_ref(&folder));
        assert!(!memory.md5_absent(), "a listing of folders says nothing");
        memory.note_listing(&[folder.clone(), file(None)]);
        assert!(memory.md5_absent(), "listing without MD5");
        assert!(!memory.sha256_absent(), "each algorithm on its own");
        memory.note_listing(&[file(None), file(Some(MD5))]);
        assert!(!memory.md5_absent(), "a MD5 seen: ask again");
        memory.note_listing(&[file(None)]);
        memory.note_md5(false);
        assert!(!memory.md5_absent(), "once seen it stays");

        let server = Server::start();
        let backend = server.backend();
        put(&server, "/a.bin", Some(&format!("MD5:{MD5}")));
        backend.checksums.note_listing(&[file(None)]);
        let before = server.dav.count("PROPFIND");
        let cancel = AtomicBool::new(false);
        assert_eq!(
            content_md5(&backend, "/a.bin", &cancel).unwrap(),
            None,
            "listing without MD5"
        );
        assert_eq!(server.dav.count("PROPFIND"), before, "no request");
        backend.checksums.note_listing(&[file(Some(MD5))]);
        assert_eq!(
            content_md5(&backend, "/a.bin", &cancel).unwrap().as_deref(),
            Some(MD5),
            "listing with MD5"
        );
        assert_eq!(server.dav.count("PROPFIND"), before + 1, "listing with MD5");
    }
}
