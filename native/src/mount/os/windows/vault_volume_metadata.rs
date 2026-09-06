//! Recursive Windows/Node metadata demand through the real daemon-backed drive.
use super::MountedOptimization;
use std::{
    collections::BTreeMap,
    fs,
    io,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{mpsc, atomic::{AtomicBool, AtomicUsize, Ordering}},
    thread,
    time::{Duration, Instant},
};

const BRANCHES: usize = 512;
const LEVELS: usize = 8;
const WIDE_FILES: usize = 50_001;
const WORKERS: usize = 16;

pub(super) fn exercise(fixture: &mut MountedOptimization) -> io::Result<()> {
    let initial = counters(fixture);
    assert_eq!(initial.2, 0, "mount startup downloaded content");
    let cold = Instant::now();
    bounded_native(fixture, "cold recursive metadata", |root| {
        check_directory(&root.join("large"), (0..BRANCHES)
            .map(|branch| (format!("b{branch:03}"), true)).collect(), false)?;
        scan_branches(root)?;
        check_directory(&root.join("wide"), (0..WIDE_FILES)
            .map(|number| (format!("f{number:05}.md"), false)).collect(), false)
    })?;
    let after_cold = counters(fixture);
    report("native-cold", cold.elapsed(), initial, after_cold);
    assert_eq!(after_cold.2, 0, "recursive Windows metadata downloaded content");
    assert!(after_cold.0 > initial.0, "cold tree never reached daemon listing");
    assert!(after_cold.3 > 1, "independent daemon listings never overlapped");

    bounded_native(fixture, "native by-name and handle metadata", super::byname::exercise)?;
    // The complete pass can exceed TTL; prime a bounded exact subtree first,
    // then measure its immediate reuse, not an unrealistically immortal vault.
    bounded_native(fixture, "prime hot subtree", |root| check_branch(root, BRANCHES - 1))?;
    let before_hot = counters(fixture);
    let hot = Instant::now();
    bounded_native(fixture, "warm hot subtree", |root| check_branch(root, BRANCHES - 1))?;
    let hot_elapsed = hot.elapsed();
    let after_hot = counters(fixture);
    report("native-hot-subtree", hot_elapsed, before_hot, after_hot);
    assert!(hot_elapsed < Duration::from_secs(10), "hot subtree missed its within-TTL window");
    assert_eq!((after_hot.0, after_hot.1, after_hot.2),
        (before_hot.0, before_hot.1, before_hot.2), "hot subtree performed remote work");

    let node_before = counters(fixture);
    let node = Instant::now();
    run_node(fixture)?;
    let node_after = counters(fixture);
    report("node-recursive", node.elapsed(), node_before, node_after);
    assert_eq!(node_after.2, 0, "Node metadata traversal downloaded file content");
    eprintln!("[mount vault] exact manifest nested_dirs=4609 nested_files=16384 wide_files=50001");
    fixture.healthy()
}

fn counters(fixture: &MountedOptimization) -> (usize, usize, usize, usize) {
    let counts = fixture.bridge.as_ref().expect("layered vault bridge").counters();
    (counts.lists, counts.stats, counts.reads, counts.max_active)
}

fn report(phase: &str, elapsed: Duration, before: (usize, usize, usize, usize),
    after: (usize, usize, usize, usize)) {
    eprintln!("[mount vault] phase={phase} elapsed_ms={} raw_lists={} raw_stats={} raw_reads={} max_active={} provider_latency_ms=1",
        elapsed.as_millis(), after.0 - before.0, after.1 - before.1, after.2 - before.2, after.3);
}

fn bounded_native(
    fixture: &mut MountedOptimization,
    label: &'static str,
    work: impl FnOnce(&Path) -> io::Result<()> + Send + 'static,
) -> io::Result<()> {
    let root = fixture.root()?;
    let (send, receive) = mpsc::channel();
    let worker = thread::Builder::new().name("vault-native-metadata".into()).spawn(move || {
        let _ = send.send(work(&root));
    })?;
    let result = receive.recv_timeout(Duration::from_secs(240))
        .map_err(|error| io::Error::other(format!("{label}: {error}"))).and_then(|result| result);
    if result.is_err() { fixture.close(); }
    // Never detach a caller blocked inside the mounted filesystem. Unmount
    // first on error/timeout; the fixture's fatal deadline bounds even teardown.
    worker.join().map_err(|_| io::Error::other(format!("{label}: worker panicked")))?;
    result.map_err(|error| io::Error::other(format!("{label}: {error}")))
}

fn scan_branches(root: &Path) -> io::Result<()> {
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let completed = AtomicUsize::new(0);
    thread::scope(|scope| -> io::Result<()> {
        let mut workers = Vec::new();
        let mut first_error = None;
        for _ in 0..WORKERS {
            match thread::Builder::new().name("vault-recursive-metadata".into()).spawn_scoped(scope, || {
                while !stop.load(Ordering::Acquire) {
                    let branch = next.fetch_add(1, Ordering::Relaxed);
                    if branch >= BRANCHES { break; }
                    if let Err(error) = check_branch(root, branch) {
                        stop.store(true, Ordering::Release);
                        return Err(error);
                    }
                    completed.fetch_add(1, Ordering::Relaxed);
                }
                Ok(())
            }) {
                Ok(worker) => workers.push(worker),
                Err(error) => {
                    stop.store(true, Ordering::Release);
                    first_error = Some(error);
                    break;
                }
            }
        }
        for worker in workers {
            let result = worker.join().unwrap_or_else(|_| Err(io::Error::other("vault metadata worker panicked")));
            if let Err(error) = result {
                stop.store(true, Ordering::Release);
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    })?;
    assert_eq!(completed.load(Ordering::Relaxed), BRANCHES, "incomplete recursive manifest");
    Ok(())
}

fn check_branch(root: &Path, branch: usize) -> io::Result<()> {
    let mut directory = root.join("large").join(format!("b{branch:03}"));
    check_directory(&directory, vec![("d0".into(), true)], true)?;
    for level in 0..LEVELS {
        directory.push(format!("d{level}"));
        let mut entries = (0..4).map(|note| (format!("note{note}.md"), false)).collect::<Vec<_>>();
        if level + 1 < LEVELS { entries.push((format!("d{}", level + 1), true)); }
        check_directory(&directory, entries, true)?;
    }
    Ok(())
}

fn check_directory(path: &Path, expected: Vec<(String, bool)>, point_queries: bool) -> io::Result<()> {
    let mut expected = expected.into_iter().collect::<BTreeMap<_, _>>();
    for entry in fs::read_dir(path).map_err(|error| path_error("read_dir", path, error))? {
        let entry = entry.map_err(|error| path_error("read_dir entry", path, error))?;
        let name = entry.file_name().into_string().map_err(|_| io::Error::other("non-UTF8 fixture name"))?;
        let directory = expected.remove(&name)
            .ok_or_else(|| io::Error::other(format!("unexpected/duplicate entry {name} in {}", path.display())))?;
        let actual = entry.path();
        check_metadata(&actual, &entry.metadata()
            .map_err(|error| path_error("entry metadata", &actual, error))?, directory)?;
        if point_queries {
            check_metadata(&actual, &fs::metadata(&actual)
                .map_err(|error| path_error("stat", &actual, error))?, directory)?;
            check_metadata(&actual, &fs::symlink_metadata(&actual)
                .map_err(|error| path_error("lstat", &actual, error))?, directory)?;
        }
    }
    if !expected.is_empty() {
        return Err(io::Error::other(format!("{} missing {} expected entries; first={:?}",
            path.display(), expected.len(), expected.keys().next())));
    }
    Ok(())
}

fn check_metadata(path: &Path, metadata: &fs::Metadata, directory: bool) -> io::Result<()> {
    if metadata.is_dir() != directory || metadata.file_type().is_symlink()
        || (!directory && (!metadata.is_file() || metadata.len() != 4))
    {
        return Err(io::Error::other(format!("incorrect metadata for {}", path.display())));
    }
    Ok(())
}

fn path_error(operation: &str, path: &Path, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{operation} {}: {error}", path.display()))
}

struct NodeChild(Child);
impl Drop for NodeChild {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn run_node(fixture: &mut MountedOptimization) -> io::Result<()> {
    let executable = configured_file("SMART_EXPLORER_MOUNT_NODE")?;
    let script = configured_file("SMART_EXPLORER_MOUNT_VAULT_NODE_SCRIPT")?;
    let mut child = NodeChild(Command::new(executable).arg(script).arg(fixture.root()?)
        .current_dir(fixture.temporary.path()).env_remove("NODE_OPTIONS")
        // This affects only the checked-in child workload, never another app.
        .env("UV_THREADPOOL_SIZE", WORKERS.to_string())
        .stdin(Stdio::null()).stdout(Stdio::inherit()).stderr(Stdio::inherit()).spawn()?);
    let deadline = Instant::now() + Duration::from_secs(300);
    let result = (|| {
        loop {
            if let Some(status) = child.0.try_wait()? {
                return if status.success() { Ok(()) }
                    else { Err(io::Error::other(format!("Node vault workload failed: {status}"))) };
            }
            if Instant::now() >= deadline {
                return Err(io::Error::other("Node vault workload exceeded 300 seconds"));
            }
            thread::sleep(Duration::from_millis(25));
        }
    })();
    if result.is_err() { fixture.close(); }
    // Child cleanup follows unmount on failure, so no surviving Node operation
    // can hold this drive while the next runtime is being started.
    drop(child);
    result
}

fn configured_file(name: &str) -> io::Result<PathBuf> {
    let path = std::env::var_os(name).map(PathBuf::from)
        .ok_or_else(|| io::Error::other(format!("{name} is required")))?;
    if !path.is_absolute() || !path.is_file() {
        return Err(io::Error::other(format!("{name} must name an existing absolute file")));
    }
    Ok(path)
}
