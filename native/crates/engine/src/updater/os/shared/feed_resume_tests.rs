//! A broken update download continues where it stopped (user rule
//! 2026-11-17): over the loopback feed, a cut answer is followed by a range
//! request from the bytes written; a server that ignores the range sends
//! everything again, which replaces the kept bytes; only a server that
//! never gets further ends the download.
use std::sync::atomic::AtomicBool;

use crate::updater::download_bound::tests::http_fixture::{HttpFeed, Route};
use crate::updater::feed_trust::tests::fixture::{AppData, ReleaseFolder, VERSION};

/// Downloads `name` of `server` into a fresh folder; the result and every
/// `(done, total)` the download reported.
fn download(server: &HttpFeed, name: &str) -> (Result<Vec<u8>, String>, Vec<(u64, Option<u64>)>) {
    let target = tempfile::tempdir().expect("a target folder");
    let destination = target.path().join(name);
    let mut seen = Vec::new();
    let result = crate::updater::download_feed_file(
        &server.base(),
        name,
        &destination,
        &AtomicBool::new(false),
        &mut |done, total| seen.push((done, total)),
    )
    .map(|()| std::fs::read(&destination).expect("the downloaded file"));
    assert!(
        !target.path().join(format!("{name}.part")).exists(),
        "no partial file stays"
    );
    (result, seen)
}

fn requests_of(server: &HttpFeed, name: &str) -> usize {
    server
        .requests()
        .iter()
        .filter(|path| path.as_str() == name)
        .count()
}

#[test]
fn zeitlimit_task_a_broken_update_download_continues_with_a_range_request() {
    let app = AppData::new();
    let release = ReleaseFolder::new(VERSION);
    let server = HttpFeed::start();
    server.serve_folder(&release);

    // The desktop payload: the first two answers break after a third of
    // their body; the third brings the rest.
    let payload = release.payload("app");
    let names = crate::updater::os::app_payload_spec().http_names;
    for name in names {
        server.serve(
            name,
            Route {
                cut_at: Some(payload.bytes.len() / 3),
                cut_requests: Some(2),
                ranges: true,
                ..Route::plain(payload.bytes.clone())
            },
        );
    }
    let staged = server
        .feed()
        .fetch_exe(VERSION, None)
        .unwrap_or_else(|error| panic!("the resumed payload: {error}"));
    assert_eq!(staged.sha256(), payload.sha256());
    assert_eq!(
        requests_of(&server, names[0]),
        3,
        "two cut answers, then the rest"
    );
    std::fs::remove_file(staged.path()).expect("the staged payload");
    assert_eq!(app.staged_names(), Vec::<String>::new());

    // The Android package: the rest arrives as a 206 from the kept bytes, so
    // the count of kept bytes never goes back.
    let body: Vec<u8> = (0..100u8).collect();
    server.serve(
        "resumed.apk",
        Route {
            cut_at: Some(30),
            cut_requests: Some(1),
            ranges: true,
            ..Route::plain(body.clone())
        },
    );
    let (result, seen) = download(&server, "resumed.apk");
    assert_eq!(result.expect("the resumed file"), body);
    assert_eq!(requests_of(&server, "resumed.apk"), 2);
    assert!(seen.contains(&(30, Some(100))), "{seen:?}");
    assert!(
        seen.windows(2).all(|pair| pair[0].0 <= pair[1].0),
        "the kept bytes stay: {seen:?}"
    );
    assert_eq!(seen.last(), Some(&(100, Some(100))));

    // A server that ignores the range sends the whole file again; it
    // replaces the kept bytes instead of being appended to them.
    server.serve(
        "restarted.apk",
        Route {
            cut_at: Some(40),
            cut_requests: Some(1),
            ..Route::plain(body.clone())
        },
    );
    let (result, seen) = download(&server, "restarted.apk");
    assert_eq!(result.expect("the restarted file"), body);
    assert_eq!(requests_of(&server, "restarted.apk"), 2);
    let back = seen.iter().position(|step| *step == (40, Some(100)));
    assert!(
        back.is_some_and(|at| seen[at + 1..].contains(&(0, Some(100)))),
        "the kept bytes went: {seen:?}"
    );

    // A server that always breaks at the same byte brings nothing new after
    // its first answer: five such attempts end the download (the tests have
    // no stall window, `http_resume::fetch`).
    server.serve(
        "stalled.apk",
        Route {
            cut_at: Some(50),
            ..Route::plain(body)
        },
    );
    let (result, _) = download(&server, "stalled.apk");
    let error = result.expect_err("a server that never gets further");
    assert!(error.contains("stalled.apk"), "{error}");
    assert!(error.contains("no new byte in 5 attempts"), "{error}");
    assert_eq!(requests_of(&server, "stalled.apk"), 6);
}
