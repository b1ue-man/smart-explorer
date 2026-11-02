//! Copying many small files over a real loopback Share (KP2): packets reach
//! the host through the background service's proxy (Z-P1), and the
//! `#[ignore]` benchmarks print files per second and requests per file (and,
//! for single files and one large file, MiB per second, KP5a):
//! `cargo test --release -- --ignored speed_task_share --nocapture --test-threads=1`.
use std::fs;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use super::transfer_engine_task_support::{get, put, RecordingSink};
use super::CopyPastePeerFixture;
use crate::share::node_sessions::OPENED_STREAMS;
use crate::transfer::{Endpoint, JobItems, Layout, TransferJob, TransferMsg};
use crate::vfs::{BackendHandle, BatchPutOutcome};

/// The fixture's peer behind the background service's proxy, as the GUI,
/// CLI, MCP and Android reach a Share.
fn proxied(fixture: &CopyPastePeerFixture) -> io::Result<BackendHandle> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let peer = fixture.backend.clone();
    std::thread::spawn(move || {
        if let Ok((socket, _)) = listener.accept() {
            if let Ok(read) = socket.try_clone() {
                let _ = crate::daemon::serve_sync_link_fixture(read, socket, peer);
            }
        }
    });
    let stream = TcpStream::connect(address)?;
    let read: Box<dyn Read + Send> = Box::new(stream.try_clone()?);
    let write: Box<dyn Write + Send> = Box::new(stream);
    let client = crate::agent::AgentBackend::from_streams(read, write, fixture.backend.clone())?;
    Ok(std::sync::Arc::new(client))
}

#[test]
fn speed_task_share_packets_pass_the_proxy() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let backend = proxied(&fixture)?;
    // The service announces the peer's packets although its root is the
    // list of exports; that list and the connections hold none.
    assert!(backend.batch_limits("/B").is_some());
    assert!(backend.batch_limits("/").is_none());
    assert!(backend.batch_limits("/Verbindungen").is_none());

    let outcomes = backend.put_batch(
        &[put("/B/one.txt", 3), put("/B/two.txt", 3)],
        &mut &b"onetwo"[..],
    )?;
    let published: Vec<String> = outcomes
        .iter()
        .map(|outcome| match outcome {
            BatchPutOutcome::Published(path) => path.clone(),
            BatchPutOutcome::Failed(error) => format!("failed {error}"),
        })
        .collect();
    assert_eq!(published, ["/B/one.txt", "/B/two.txt"]);
    assert_eq!(fs::read(fixture.root_b.join("two.txt"))?, b"two");

    let mut sink = RecordingSink::default();
    backend.get_batch(&[get("/B/one.txt", 3), get("/B/two.txt", 3)], &mut sink)?;
    assert_eq!(sink.bytes, [b"one".to_vec(), b"two".to_vec()]);

    // An engine job through the proxy moves the files in packets.
    let source = tempfile::tempdir()?;
    let local = source.path().join("small");
    write_tree(&local, 40, 2, 512)?;
    fs::create_dir(fixture.root_b.join("dst"))?;
    let done = run(&backend, Endpoint::Local, "/B/dst", &[fwd(&local)])?;
    assert_eq!(done, 40);
    assert_eq!(
        fs::read(fixture.root_b.join("dst/small/d1/f0001.bin"))?,
        content(1, 512)
    );
    Ok(())
}

fn fwd(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn content(index: usize, size: usize) -> Vec<u8> {
    let mut bytes = format!("file {index:06} ").into_bytes();
    bytes.resize(size, (index % 251) as u8);
    bytes
}

/// `files` files of `size` bytes in `folders` folders below `root`.
fn write_tree(root: &Path, files: usize, folders: usize, size: usize) -> io::Result<()> {
    for index in 0..files {
        let folder = root.join(format!("d{}", index % folders.max(1)));
        fs::create_dir_all(&folder)?;
        fs::write(
            folder.join(format!("f{index:04}.bin")),
            content(index, size),
        )?;
    }
    Ok(())
}

/// Runs a GUI-shaped job (crash log on, keep both) between `source` and the
/// Share `backend`; returns the files it moved.
fn run(
    backend: &BackendHandle,
    source: Endpoint,
    target_dir: &str,
    paths: &[String],
) -> io::Result<u64> {
    let remote_source = matches!(source, Endpoint::Remote(_));
    let job = TransferJob {
        id: crate::transfer::new_job_id(),
        recoverable: true,
        recovery: crate::transfer::RecoveryMode::New,
        folders: crate::transfer::FolderConflictPolicy::new(
            crate::transfer::FolderConflict::KeepBoth,
        ),
        target: if remote_source {
            Endpoint::Local
        } else {
            Endpoint::Remote(backend.clone())
        },
        source,
        target_dir: target_dir.to_string(),
        items: JobItems::Roots {
            paths: paths.to_vec(),
            base: None,
        },
        layout: Layout::Tree,
        filter: None,
        conflict: crate::types::Conflict::Rename,
        mode: crate::types::CopyMode::Copy,
        source_label: "Quelle".to_string(),
        target_label: "Ziel".to_string(),
        resume: None,
    };
    let finished = std::sync::Mutex::new(None);
    let cancel = AtomicBool::new(false);
    let report = |message: TransferMsg| {
        if let TransferMsg::Done {
            progress, errors, ..
        } = message
        {
            if let Ok(mut slot) = finished.lock() {
                *slot = Some((progress.files_done, errors));
            }
        }
    };
    crate::transfer::run_view(crate::transfer::JobView::of(&job), &report, &cancel);
    let finished = finished
        .into_inner()
        .map_err(|_| io::Error::other("report lock"))?;
    match finished {
        Some((done, errors)) if errors.is_empty() => Ok(done),
        Some((_, errors)) => Err(io::Error::other(errors.join("; "))),
        None => Err(io::Error::other("job ended without its terminal message")),
    }
}

/// Runs one case and prints its rate and requests per file.
fn bench(case: &str, files: usize, job: impl FnOnce() -> io::Result<u64>) -> io::Result<()> {
    let opened = OPENED_STREAMS.load(Ordering::Relaxed);
    let started = Instant::now();
    let done = job()?;
    let seconds = started.elapsed().as_secs_f64().max(f64::EPSILON);
    let requests = OPENED_STREAMS
        .load(Ordering::Relaxed)
        .saturating_sub(opened);
    assert_eq!(done, files as u64, "{case}");
    println!(
        "BENCH {case} files_per_s={:.0} requests_per_file={:.3}",
        files as f64 / seconds,
        requests as f64 / files as f64
    );
    Ok(())
}

/// Uploads `files` × `size` bytes in 50 folders to `/B/dst`, then downloads
/// `/A/src` and the export `/A` itself; directly and through the proxy.
fn bench_set(label: &str, files: usize, size: usize) -> io::Result<()> {
    for proxy in [false, true] {
        let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
        let backend = if proxy {
            proxied(&fixture)?
        } else {
            fixture.backend.clone()
        };
        let route = if proxy { "proxy" } else { "direct" };
        let local = tempfile::tempdir()?;
        let upload = local.path().join("src");
        write_tree(&upload, files, 50, size)?;
        fs::create_dir(fixture.root_b.join("dst"))?;
        bench(&format!("{label}_upload_{route}"), files, || {
            run(&backend, Endpoint::Local, "/B/dst", &[fwd(&upload)])
        })?;
        write_tree(&fixture.root_a.join("src"), files, 50, size)?;
        let into: PathBuf = local.path().join("down");
        fs::create_dir(&into)?;
        let remote = Endpoint::Remote(backend.clone());
        bench(&format!("{label}_download_{route}"), files, || {
            run(&backend, remote, &fwd(&into), &["/A/src".to_string()])
        })?;
        let export = local.path().join("export");
        fs::create_dir(&export)?;
        let remote = Endpoint::Remote(backend.clone());
        bench(&format!("{label}_download_export_{route}"), files, || {
            run(&backend, remote, &fwd(&export), &["/A".to_string()])
        })?;
    }
    Ok(())
}

#[test]
#[ignore = "benchmark: run explicitly with --ignored --nocapture"]
fn speed_task_share_small_files_4k() -> io::Result<()> {
    bench_set("small_5000x4k", 5000, 4 * 1024)
}

#[test]
#[ignore = "benchmark: run explicitly with --ignored --nocapture"]
fn speed_task_share_medium_files_64k() -> io::Result<()> {
    bench_set("medium_1000x64k", 1000, 64 * 1024)
}

/// Runs one case and prints its byte rate and requests per job.
fn bench_bytes(
    case: &str,
    bytes: u64,
    jobs: u64,
    job: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let opened = OPENED_STREAMS.load(Ordering::Relaxed);
    let started = Instant::now();
    job()?;
    let seconds = started.elapsed().as_secs_f64().max(f64::EPSILON);
    let requests = OPENED_STREAMS
        .load(Ordering::Relaxed)
        .saturating_sub(opened);
    println!(
        "BENCH {case} mib_per_s={:.1} requests_per_job={:.2}",
        bytes as f64 / seconds / (1024.0 * 1024.0),
        requests as f64 / jobs.max(1) as f64
    );
    Ok(())
}

/// `jobs` jobs of one file each (the single-file path, Z-P4), up and down,
/// directly and through the proxy.
fn bench_single_files(label: &str, jobs: usize, size: usize) -> io::Result<()> {
    for proxy in [false, true] {
        let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
        let backend = if proxy {
            proxied(&fixture)?
        } else {
            fixture.backend.clone()
        };
        let route = if proxy { "proxy" } else { "direct" };
        let local = tempfile::tempdir()?;
        let sources: Vec<PathBuf> = (0..jobs)
            .map(|index| local.path().join(format!("job{index:04}")))
            .collect();
        for (index, source) in sources.iter().enumerate() {
            write_tree(source, 1, 1, size)?;
            fs::create_dir_all(fixture.root_b.join(format!("dst{index:04}")))?;
        }
        let total = (jobs * size) as u64;
        bench_bytes(
            &format!("{label}_upload_{route}"),
            total,
            jobs as u64,
            || {
                for (index, source) in sources.iter().enumerate() {
                    let target = format!("/B/dst{index:04}");
                    let done = run(&backend, Endpoint::Local, &target, &[fwd(source)])?;
                    assert_eq!(done, 1);
                }
                Ok(())
            },
        )?;
    }
    Ok(())
}

/// One large file up and down, directly and through the proxy: the rate a
/// single transfer reaches (Z-P5, Z-P6, Z-P8).
fn bench_large_file(label: &str, size: usize) -> io::Result<()> {
    for proxy in [false, true] {
        let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
        let backend = if proxy {
            proxied(&fixture)?
        } else {
            fixture.backend.clone()
        };
        let route = if proxy { "proxy" } else { "direct" };
        let local = tempfile::tempdir()?;
        let upload = local.path().join("large");
        write_tree(&upload, 1, 1, size)?;
        fs::create_dir(fixture.root_b.join("dst"))?;
        bench_bytes(&format!("{label}_upload_{route}"), size as u64, 1, || {
            run(&backend, Endpoint::Local, "/B/dst", &[fwd(&upload)])?;
            Ok(())
        })?;
        write_tree(&fixture.root_a.join("src"), 1, 1, size)?;
        let into = local.path().join("down");
        fs::create_dir(&into)?;
        let remote = Endpoint::Remote(backend.clone());
        bench_bytes(&format!("{label}_download_{route}"), size as u64, 1, || {
            run(&backend, remote, &fwd(&into), &["/A/src".to_string()])?;
            Ok(())
        })?;
    }
    Ok(())
}

#[test]
#[ignore = "benchmark: run explicitly with --ignored --nocapture"]
fn speed_task_share_single_files_64k() -> io::Result<()> {
    bench_single_files("single_200x64k", 200, 64 * 1024)
}

#[test]
#[ignore = "benchmark: run explicitly with --ignored --nocapture"]
fn speed_task_share_large_file_512m() -> io::Result<()> {
    bench_large_file("large_1x512m", 512 * 1024 * 1024)
}
