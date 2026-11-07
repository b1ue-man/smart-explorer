//! Upper bound of every update download (r-lokal-4; suite expectations
//! lok-4a and lok-4b of the review 2026-11-07), and the loopback HTTP feed
//! the updater's other tests share.
use std::io::{self, Cursor, Read};
use std::sync::atomic::AtomicBool;

use super::{copy_bounded, DownloadBound, SPACE_RESERVE};
use crate::updater::feed_trust::tests::fixture::{AppData, ReleaseFolder, VERSION};
use http_fixture::{HttpFeed, Route};

/// A feed served on `127.0.0.1` for the updater's tests.
#[allow(dead_code)]
pub(in crate::updater) mod http_fixture {
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use crate::updater::feed::Feed;
    use crate::updater::feed_trust::tests::fixture::ReleaseFolder;

    /// What the loopback server answers for one path.
    #[derive(Clone)]
    pub(in crate::updater) struct Route {
        pub(in crate::updater) body: Vec<u8>,
        /// `Content-Length` to announce instead of the body length.
        pub(in crate::updater) announce: Option<usize>,
        /// Send only this many bytes of the body, then close.
        pub(in crate::updater) cut_at: Option<usize>,
    }

    impl Route {
        pub(in crate::updater) fn plain(body: impl Into<Vec<u8>>) -> Self {
            Self {
                body: body.into(),
                announce: None,
                cut_at: None,
            }
        }
    }

    type Routes = Arc<Mutex<HashMap<String, Route>>>;

    /// A feed served on `127.0.0.1` (port chosen by the system) that logs the
    /// path of every request. Nothing leaves the machine.
    pub(in crate::updater) struct HttpFeed {
        addr: SocketAddr,
        routes: Routes,
        requests: Arc<Mutex<Vec<String>>>,
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl HttpFeed {
        pub(in crate::updater) fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("the feed listener");
            listener
                .set_nonblocking(true)
                .expect("nonblocking listener");
            let addr = listener.local_addr().expect("the feed address");
            let routes: Routes = Arc::default();
            let requests: Arc<Mutex<Vec<String>>> = Arc::default();
            let stop = Arc::new(AtomicBool::new(false));
            let thread = {
                let (routes, requests, stop) = (routes.clone(), requests.clone(), stop.clone());
                std::thread::spawn(move || accept_loop(&listener, &routes, &requests, &stop))
            };
            Self {
                addr,
                routes,
                requests,
                stop,
                thread: Some(thread),
            }
        }

        /// The feed's base URL without a trailing slash.
        pub(in crate::updater) fn base(&self) -> String {
            format!("http://{}", self.addr)
        }

        pub(in crate::updater) fn feed(&self) -> Feed {
            Feed::Http(self.base())
        }

        pub(in crate::updater) fn serve(&self, path: &str, route: Route) {
            self.routes
                .lock()
                .expect("the routes")
                .insert(path.to_string(), route);
        }

        /// Serves every file of `folder` under its own name (payloads under
        /// their HTTP names).
        pub(in crate::updater) fn serve_folder(&self, folder: &ReleaseFolder) {
            for entry in std::fs::read_dir(folder.path()).expect("the feed folder") {
                let name = entry
                    .expect("a directory entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned();
                let http_name = folder
                    .payloads()
                    .iter()
                    .find(|payload| payload.name == name)
                    .map_or(name.clone(), |payload| payload.http_name.to_string());
                self.serve(&http_name, Route::plain(folder.read(&name)));
            }
        }

        /// Paths requested so far, without the leading slash.
        pub(in crate::updater) fn requests(&self) -> Vec<String> {
            self.requests.lock().expect("the request log").clone()
        }
    }

    impl Drop for HttpFeed {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn accept_loop(
        listener: &TcpListener,
        routes: &Routes,
        requests: &Arc<Mutex<Vec<String>>>,
        stop: &AtomicBool,
    ) {
        while !stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let (routes, requests) = (routes.clone(), requests.clone());
                    std::thread::spawn(move || answer(stream, &routes, &requests));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => return,
            }
        }
    }

    fn answer(mut stream: TcpStream, routes: &Routes, requests: &Arc<Mutex<Vec<String>>>) {
        if stream.set_nonblocking(false).is_err()
            || stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .is_err()
        {
            return;
        }
        let mut head = Vec::new();
        let mut chunk = [0u8; 1024];
        let started = Instant::now();
        while !head.windows(4).any(|window| window == b"\r\n\r\n") {
            if head.len() > 16 * 1024 || started.elapsed() > Duration::from_secs(5) {
                return;
            }
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(count) => head.extend_from_slice(&chunk[..count]),
            }
        }
        let text = String::from_utf8_lossy(&head).into_owned();
        let path = text
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("/")
            .trim_start_matches('/')
            .to_string();
        requests.lock().expect("the request log").push(path.clone());
        let route = routes.lock().expect("the routes").get(&path).cloned();
        let Some(route) = route else {
            let _ = stream.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
            return;
        };
        let announced = route.announce.unwrap_or(route.body.len());
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {announced}\r\nConnection: close\r\n\r\n"
        );
        let body = match route.cut_at {
            Some(limit) => &route.body[..limit.min(route.body.len())],
            None => &route.body[..],
        };
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(body);
        let _ = stream.flush();
    }
}

fn run(
    bytes: &[u8],
    bound: DownloadBound,
    cancelled: bool,
) -> (io::Result<u64>, Vec<u8>, Vec<u64>) {
    let mut reader = Cursor::new(bytes.to_vec());
    let mut written = Vec::new();
    let mut seen = Vec::new();
    let result = copy_bounded(
        &mut reader,
        &mut written,
        bound,
        &mut |done| seen.push(done),
        &|| cancelled,
    );
    (result, written, seen)
}

fn exact(size: u64) -> DownloadBound {
    DownloadBound {
        exact: Some(size),
        ceiling: None,
    }
}

fn ceiling(size: u64) -> DownloadBound {
    DownloadBound {
        exact: None,
        ceiling: Some(size),
    }
}

/// A reader that fails once with `Interrupted`, then gives its bytes.
struct Interrupted {
    first: bool,
    rest: Cursor<Vec<u8>>,
}

impl Read for Interrupted {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if std::mem::take(&mut self.first) {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        self.rest.read(buffer)
    }
}

/// lok-4a: the copy ends with an error at the first byte beyond the bound,
/// or short of an announced length, or when cancelled, whatever the feed
/// sends, and never needs a fixed size.
#[test]
fn lokal_task_copy_bounded_enforces_the_announced_signed_and_free_space_bound() {
    let data = |size: usize| vec![0x5a; size];

    let (done, written, seen) = run(&data(100), exact(100), false);
    assert_eq!(done.expect("exactly the announced length"), 100);
    assert_eq!(written, data(100));
    assert_eq!(seen.last(), Some(&100));

    let (done, written, _) = run(&data(200), exact(100), false);
    let error = done.expect_err("more than announced");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("more bytes"), "{error}");
    assert!(written.len() <= 100, "{} bytes written", written.len());

    let (done, written, _) = run(&data(50), exact(100), false);
    let error = done.expect_err("less than announced");
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    assert_eq!(
        written.len(),
        50,
        "what arrived was written; the caller removes it"
    );

    let (done, _, _) = run(&data(10), ceiling(10), false);
    assert_eq!(done.expect("exactly at the ceiling"), 10);
    let (done, _, _) = run(&data(11), ceiling(10), false);
    let error = done.expect_err("beyond the ceiling");
    assert!(error.to_string().contains("1 GiB"), "{error}");

    let (done, written, _) = run(&data(100), exact(100), true);
    assert_eq!(
        done.expect_err("cancelled").kind(),
        io::ErrorKind::Interrupted
    );
    assert!(written.is_empty());

    // A feed that never ends fills nothing beyond the bound.
    for bound in [exact(1000), ceiling(1000)] {
        let mut sink = Vec::new();
        let result = copy_bounded(&mut io::repeat(0), &mut sink, bound, &mut |_| {}, &|| false);
        assert!(result.is_err(), "{bound:?}");
        assert!(sink.len() <= 1000, "{} bytes written", sink.len());
    }

    // An interrupted read is retried, not reported.
    let mut reader = Interrupted {
        first: true,
        rest: Cursor::new(data(30)),
    };
    let mut sink = Vec::new();
    let done = copy_bounded(&mut reader, &mut sink, exact(30), &mut |_| {}, &|| false);
    assert_eq!(done.expect("the retry"), 30);

    // The bound of a download: the announced length is exact; without one the
    // free space of the folder minus the reserve; a signed size is exact.
    let folder = std::env::temp_dir();
    let announced = DownloadBound::new(Some(100), &folder);
    assert_eq!((announced.exact, announced.ceiling), (Some(100), None));
    let unannounced = DownloadBound::new(None, &folder);
    assert_eq!(unannounced.exact, None);
    assert_eq!(
        unannounced.ceiling.is_some(),
        crate::updater::os::available_space(&folder).is_some()
    );
    assert_eq!(SPACE_RESERVE, 1 << 30);
    let signed = DownloadBound::exact(7);
    assert_eq!((signed.exact, signed.ceiling), (Some(7), None));
}

/// lok-4b: a feed that ends its body short of the announced length fails the
/// payload download and the file download; neither leaves a partial file
/// (`.download` in the app data, `.part` beside the destination). The
/// control cases still succeed.
#[test]
fn lokal_task_http_downloads_that_end_short_leave_no_partial_file() {
    let app = AppData::new();
    let release = ReleaseFolder::new(VERSION);
    let server = HttpFeed::start();
    server.serve_folder(&release);

    // Control: the whole payload is staged.
    let payload = server
        .feed()
        .fetch_exe(VERSION, None)
        .unwrap_or_else(|error| panic!("the complete payload: {error}"));
    assert!(payload.path().is_file());
    assert_eq!(payload.sha256(), release.payload("app").sha256());
    std::fs::remove_file(payload.path()).expect("the staged control payload");
    assert_eq!(app.staged_names(), Vec::<String>::new());

    // Every HTTP name of the app payload ends after half of its body.
    let announced = release.payload("app");
    for name in crate::updater::os::app_payload_spec().http_names {
        server.serve(
            name,
            Route {
                body: announced.bytes.clone(),
                announce: None,
                cut_at: Some(announced.bytes.len() / 2),
            },
        );
    }
    let error = server
        .feed()
        .fetch_exe(VERSION, None)
        .err()
        .expect("a short body must fail");
    assert!(error.contains("Download"), "{error}");
    assert_eq!(app.staged_names(), Vec::<String>::new(), "{error}");

    // The file download (Android APK, feed files).
    let target = tempfile::tempdir().expect("a target folder");
    let destination = target.path().join("update.apk");
    server.serve(
        "update.apk",
        Route {
            body: vec![7; 100],
            announce: None,
            cut_at: Some(50),
        },
    );
    let cancel = AtomicBool::new(false);
    let error = crate::updater::download_feed_file(
        &server.base(),
        "update.apk",
        &destination,
        &cancel,
        &mut |_, _| {},
    )
    .expect_err("a short body must fail");
    assert!(error.contains("update.apk"), "{error}");
    assert!(!destination.exists());
    assert!(!target.path().join("update.apk.part").exists());

    // Control: the whole file arrives, its length is reported, no partial file stays.
    server.serve("whole.apk", Route::plain(vec![9; 100]));
    let whole = target.path().join("whole.apk");
    let mut last = (0, None);
    crate::updater::download_feed_file(
        &server.base(),
        "whole.apk",
        &whole,
        &cancel,
        &mut |done, total| last = (done, total),
    )
    .expect("the complete file");
    assert_eq!(std::fs::read(&whole).expect("the file"), vec![9; 100]);
    assert_eq!(last, (100, Some(100)));
    assert!(!target.path().join("whole.apk.part").exists());

    // A cancelled download is reported as such and leaves nothing.
    let cancelled = AtomicBool::new(true);
    let aborted = target.path().join("aborted.apk");
    server.serve("aborted.apk", Route::plain(vec![1; 100]));
    let error = crate::updater::download_feed_file(
        &server.base(),
        "aborted.apk",
        &aborted,
        &cancelled,
        &mut |_, _| {},
    )
    .expect_err("a cancelled download");
    assert_eq!(error, "Download abgebrochen");
    assert!(!aborted.exists());
    assert!(!target.path().join("aborted.apk.part").exists());
}
