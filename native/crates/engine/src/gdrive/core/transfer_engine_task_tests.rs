//! Transfer-engine task tests of the Drive backend (plan block C), part 1:
//! pooled connections, one write request per new file, streamed large files,
//! congestion and permanent quota errors — against a loopback Drive.
use super::task_drive::{
    count, drive_error, drive_server, job_backend, multipart, write_fresh, FakeDrive,
};
use super::task_http::{Answer, Request, Server};
use crate::vfs::{congestion_of, Backend};
use serde_json::json;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[test]
fn transfer_engine_task_drive_api_calls_share_one_pooled_connection() {
    let (drive, server) = drive_server();
    drive.insert("file-id", "a.txt", "root", "text/plain", b"abc");
    let backend = server.backend();
    for _ in 0..5 {
        let json = backend
            .get_json(&backend.api_url("files/file-id?fields=id,name"))
            .unwrap();
        assert_eq!(json["id"], "file-id");
    }
    backend.mkdir_all("Neu").unwrap();
    backend
        .remember_path("a.txt", "file-id", Some("text/plain"))
        .unwrap();
    backend.remove_file_id("a.txt", Some("file-id")).unwrap();
    // Five reads, the root alias lookup (once per backend), a folder create
    // (probe, IDs, POST), the exact-ID check before the trash and the trash
    // PATCH share one TCP connection: one TLS handshake in production
    // instead of eleven (root alias and exact-ID check since 1bee4b6a,
    // `identity.rs` `actual_parent_id`, `trash.rs` `validate_trash_path`).
    assert_eq!(server.requests().len(), 11);
    assert_eq!(server.connections(), 1);
}

#[test]
fn transfer_engine_task_drive_fresh_files_take_one_write_request_each() {
    let (drive, server) = drive_server();
    let backend = job_backend(&drive, &server);
    let large = vec![7u8; 70_000];
    let contents: [&[u8]; 3] = [b"", b"abc", &large];
    for (index, content) in contents.iter().enumerate() {
        write_fresh(&backend, &format!("Job/Datei {index}.bin"), content).unwrap();
    }
    let requests = server.requests();
    let writes: Vec<&Request> = requests
        .iter()
        .filter(|request| request.method != "GET")
        .collect();
    // One write request per new file: no probe, no stage, no promotion.
    assert_eq!(writes.len(), contents.len());
    for (index, (write, content)) in writes.iter().zip(contents).enumerate() {
        assert_eq!(write.method, "POST");
        assert_eq!(write.path(), "/upload/drive/v3/files");
        assert_eq!(write.query("uploadType").as_deref(), Some("multipart"));
        let (metadata, media) = multipart(write);
        assert_eq!(media, content);
        assert_eq!(metadata["name"], format!("Datei {index}.bin"));
        assert_eq!(metadata["parents"], json!(["job-folder"]));
        assert!(
            metadata.get("mimeType").is_none(),
            "new files keep Drive's type detection like open_write"
        );
        let id = metadata["id"].as_str().unwrap();
        assert_eq!(
            backend
                .cached_id(&format!("Job/Datei {index}.bin"))
                .unwrap()
                .as_deref(),
            Some(id)
        );
        assert_eq!(
            drive.object(id).unwrap()["md5Checksum"],
            format!("{:x}", md5::compute(content))
        );
    }
    // All three IDs came from one pooled generateIds call. The only other
    // requests are reads: the root alias and the exact-ID check of the job's
    // folder before each use (`valid_cached_id`, since 1bee4b6a); no probe
    // of the folder's names.
    assert_eq!(count(&requests, "GET", "/drive/v3/files/generateIds"), 1);
    let reads: Vec<&str> = requests
        .iter()
        .filter(|request| request.method == "GET")
        .map(|request| request.path())
        .filter(|path| *path != "/drive/v3/files/generateIds")
        .collect();
    assert!(
        reads
            .iter()
            .all(|path| matches!(*path, "/drive/v3/files/root" | "/drive/v3/files/job-folder")),
        "{reads:?}"
    );
    assert!(backend.pending_upload_ids_guard().unwrap().is_empty());
}

#[test]
fn transfer_engine_task_drive_large_fresh_files_stream_through_one_resumable_session() {
    let (drive, server) = drive_server();
    let backend = job_backend(&drive, &server);
    let chunk = super::resumable::CHUNK_SIZE;
    // Just above the multipart limit: one session, one PUT.
    let above = vec![3u8; 5_000_001];
    // Above one chunk: the first chunk goes out while the rest still arrives.
    let chunked: Vec<u8> = (0..chunk + 5).map(|index| index as u8).collect();
    for (name, content) in [("gross.bin", &above), ("groesser.bin", &chunked)] {
        let path = format!("Job/{name}");
        let mut writer = backend
            .open_write_fresh(&path, content.len() as u64)
            .unwrap()
            .unwrap();
        for piece in content.chunks(64 * 1024) {
            writer.write_all(piece).unwrap();
        }
        writer.flush().unwrap();
        let id = backend.cached_id(&path).unwrap().unwrap();
        assert_eq!(
            drive.object(&id).unwrap()["size"],
            content.len().to_string()
        );
    }
    let requests = server.requests();
    let posts: Vec<&Request> = requests
        .iter()
        .filter(|request| request.method == "POST")
        .collect();
    assert_eq!(posts.len(), 2);
    for post in posts {
        assert_eq!(post.query("uploadType").as_deref(), Some("resumable"));
        let metadata: serde_json::Value = serde_json::from_slice(&post.body).unwrap();
        assert!(metadata["id"].as_str().unwrap().starts_with("gen-"));
        assert_eq!(metadata["parents"], json!(["job-folder"]));
    }
    let ranges: Vec<String> = requests
        .iter()
        .filter(|request| request.method == "PUT")
        .map(|request| request.header("content-range").unwrap().to_string())
        .collect();
    assert_eq!(
        ranges,
        [
            "bytes 0-5000000/5000001".to_string(),
            format!("bytes 0-{}/{}", chunk - 1, chunk + 5),
            format!("bytes {}-{}/{}", chunk, chunk + 4, chunk + 5),
        ]
    );
    // Nothing probed the job's own folder.
    assert_eq!(count(&requests, "GET", "/drive/v3/files"), 0);
}

#[test]
fn transfer_engine_task_drive_rate_limits_reach_the_caller_as_congestion() {
    // A refused create comes back once, as congestion with the server's delay;
    // the retry of the same file meets the same reserved ID.
    let drive = Arc::new(FakeDrive::default());
    let refuse = Arc::new(AtomicBool::new(true));
    let server = {
        let (drive, refuse) = (Arc::clone(&drive), Arc::clone(&refuse));
        Server::start(move |request| {
            if request.method == "POST" && refuse.swap(false, Ordering::SeqCst) {
                return drive_error(429, "rateLimitExceeded", "Too Many Requests")
                    .header("Retry-After", "7");
            }
            drive.answer(request)
        })
    };
    let backend = job_backend(&drive, &server);
    let refused = write_fresh(&backend, "Job/a.txt", b"abc").unwrap_err();
    let congestion = congestion_of(&refused).expect("a rate limit arrives as congestion");
    assert_eq!(congestion.retry_after, Some(Duration::from_secs(7)));
    write_fresh(&backend, "Job/a.txt", b"abc").unwrap();
    let posts: Vec<Request> = server
        .requests()
        .into_iter()
        .filter(|request| request.method == "POST")
        .collect();
    assert_eq!(posts.len(), 2, "a mutation is never retried internally");
    assert_eq!(multipart(&posts[0]).0["id"], multipart(&posts[1]).0["id"]);
    assert_eq!(drive.named("job-folder", "a.txt").len(), 1);

    // Reads retry a rate limit only within the hide limit, then report
    // congestion; a server delay beyond it reaches the caller at once.
    let server = Server::start(|request| {
        let answer = drive_error(403, "userRateLimitExceeded", "Rate Limit Exceeded");
        if request.path().ends_with("/slow") {
            answer.header("Retry-After", "30")
        } else {
            answer
        }
    });
    let backend = server.backend();
    let error = backend
        .get_json(&backend.api_url("files/fast"))
        .unwrap_err();
    assert!(congestion_of(&error).is_some());
    let error = backend
        .get_json(&backend.api_url("files/slow"))
        .unwrap_err();
    assert_eq!(
        congestion_of(&error).unwrap().retry_after,
        Some(Duration::from_secs(30))
    );
    let requests = server.requests();
    assert_eq!(count(&requests, "GET", "/drive/v3/files/fast"), 6);
    assert_eq!(count(&requests, "GET", "/drive/v3/files/slow"), 1);

    // 503 on a create: the reserved ID proves nothing was created, so the
    // caller gets congestion instead of an ambiguous failure.
    let drive = Arc::new(FakeDrive::default());
    let server = {
        let drive = Arc::clone(&drive);
        Server::start(move |request| {
            if request.method == "POST" {
                return Answer::status(503, json!({"error": {"message": "Service Unavailable"}}));
            }
            drive.answer(request)
        })
    };
    let backend = job_backend(&drive, &server);
    let error = write_fresh(&backend, "Job/b.txt", b"abc").unwrap_err();
    assert!(congestion_of(&error).is_some());
    let requests = server.requests();
    let post = requests
        .iter()
        .find(|request| request.method == "POST")
        .unwrap();
    let id = multipart(post).0["id"].as_str().unwrap().to_string();
    assert_eq!(count(&requests, "POST", "/upload/drive/v3/files"), 1);
    assert_eq!(count(&requests, "GET", &format!("/drive/v3/files/{id}")), 1);
}

#[test]
fn transfer_engine_task_drive_full_storage_and_daily_quota_are_permanent() {
    let drive = Arc::new(FakeDrive::default());
    let server = {
        let drive = Arc::clone(&drive);
        Server::start(move |request| {
            if request.method == "POST" {
                return drive_error(
                    403,
                    "storageQuotaExceeded",
                    "The user's Drive storage quota has been exceeded.",
                );
            }
            if request.path().ends_with("/daily") {
                return drive_error(403, "dailyLimitExceeded", "Daily Limit Exceeded");
            }
            drive.answer(request)
        })
    };
    let backend = job_backend(&drive, &server);
    let full = write_fresh(&backend, "Job/a.txt", b"abc").unwrap_err();
    assert_eq!(full.kind(), io::ErrorKind::StorageFull);
    assert!(congestion_of(&full).is_none());
    let daily = backend
        .get_json(&backend.api_url("files/daily"))
        .unwrap_err();
    assert_eq!(daily.kind(), io::ErrorKind::QuotaExceeded);
    let requests = server.requests();
    assert_eq!(count(&requests, "POST", "/upload/drive/v3/files"), 1);
    assert_eq!(count(&requests, "GET", "/drive/v3/files/daily"), 1);
}

#[test]
fn speed_task_drive_large_upload_sends_while_the_next_chunk_fills_byte_exact() {
    // G3-08c: a full chunk goes out on its own thread while the writer fills
    // the next one. The fake Drive asserts every chunk starts at the
    // confirmed offset; the stored object must be the exact bytes.
    let (drive, server) = drive_server();
    let backend = job_backend(&drive, &server);
    let chunk = super::resumable::CHUNK_SIZE;
    let content: Vec<u8> = (0..2 * chunk + 7)
        .map(|index| (index % 251) as u8 ^ (index / 4096) as u8)
        .collect();
    let path = "Job/ueberlappt.bin";
    let mut writer = backend
        .open_write_fresh(path, content.len() as u64)
        .unwrap()
        .unwrap();
    for piece in content.chunks(64 * 1024 + 3) {
        writer.write_all(piece).unwrap();
    }
    writer.flush().unwrap();
    let id = backend.cached_id(path).unwrap().unwrap();
    let object = drive.object(&id).unwrap();
    assert_eq!(object["size"], content.len().to_string());
    assert_eq!(
        object["md5Checksum"],
        format!("{:x}", md5::compute(&content))
    );
    // The chunks went out in order and without a gap or an overlap.
    let mut next = 0usize;
    for request in server
        .requests()
        .iter()
        .filter(|request| request.method == "PUT")
    {
        let range = request.header("content-range").unwrap();
        let span = range.strip_prefix("bytes ").unwrap();
        let (from, rest) = span.split_once('-').unwrap();
        let (to, total) = rest.split_once('/').unwrap();
        assert_eq!(from.parse::<usize>().unwrap(), next, "{range}");
        assert_eq!(total.parse::<usize>().unwrap(), content.len());
        next = to.parse::<usize>().unwrap() + 1;
    }
    assert_eq!(next, content.len(), "every byte was sent once");
}
