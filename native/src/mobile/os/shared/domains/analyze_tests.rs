use super::*;
use crate::analytics::{Progress, ScanIssue, SizeNode};
use crate::mobile::HostSettings;
use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};
use std::io::{self, Read, Write};
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Instant;

#[test]
fn review_task_issues_keep_notes_apart_from_read_problems() {
    let mut outcome = ScanOutcome::complete(SizeNode {
        name: "root".into(),
        size: 0,
        is_dir: true,
        children: Vec::new(),
    });
    outcome
        .notes
        .push("2 Bereiche von Android geschützt".into());
    let value = issues_json(&outcome);
    assert_eq!(value["count"], 0);
    assert_eq!(value["text"], "");
    assert_eq!(value["notes"][0], "2 Bereiche von Android geschützt");

    outcome.status = ScanStatus::Partial;
    outcome.issues.push(ScanIssue {
        path: "/a".into(),
        detail: "Zugriff verweigert".into(),
    });
    outcome.suppressed_issues = 2;
    let value = issues_json(&outcome);
    assert_eq!(value["count"], 3);
    assert_eq!(value["text"], "/a: Zugriff verweigert\n… 2 weitere");
    assert_eq!(value["notes"].as_array().map(Vec::len), Some(1));
}

/// A Share peer as the app's pooled backend sees it: the host analyses
/// (`scan_storage`); a listing would mean a walk over the network.
#[derive(Default)]
struct HostWorker {
    analyses: AtomicUsize,
    listings: AtomicUsize,
}

impl Backend for HostWorker {
    fn scheme(&self) -> Scheme {
        Scheme::Peer
    }
    fn root_display(&self) -> String {
        "PC".into()
    }
    fn scan_storage(&self, root: &str, _progress: &Progress) -> VfsResult<Option<ScanOutcome>> {
        self.analyses.fetch_add(1, Ordering::Relaxed);
        let mut outcome = ScanOutcome::complete(SizeNode {
            name: root.trim_start_matches('/').into(),
            size: 42,
            is_dir: true,
            children: Vec::new(),
        });
        outcome.notes.push("1 Bereich vom Host geschützt".into());
        Ok(Some(outcome))
    }
    fn list_dir(&self, _path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.listings.fetch_add(1, Ordering::Relaxed);
        Err(io::Error::other("walked over the network"))
    }
    fn stat(&self, _path: &str) -> VfsResult<VfsMeta> {
        Err(io::Error::other("unused"))
    }
    fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
        Err(io::Error::other("unused"))
    }
    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Err(io::Error::other("unused"))
    }
    fn rename(&self, _src: &str, _dst: &str) -> VfsResult<()> {
        Err(io::Error::other("unused"))
    }
    fn remove_file(&self, _path: &str) -> VfsResult<()> {
        Err(io::Error::other("unused"))
    }
    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        Err(io::Error::other("unused"))
    }
    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Err(io::Error::other("unused"))
    }
}

#[test]
fn review_task_remote_analysis_runs_on_the_host_worker() {
    let host = Arc::new(HostWorker::default());
    let target = Target::Remote(host.clone(), "/Daten".into());
    let outcome = scan_target(&target, &Progress::default());
    assert_eq!(outcome.status, ScanStatus::Complete);
    assert_eq!(outcome.tree.as_ref().map(|tree| tree.size), Some(42));
    assert_eq!(outcome.notes, ["1 Bereich vom Host geschützt"]);
    assert_eq!(host.analyses.load(Ordering::Relaxed), 1);
    assert_eq!(host.listings.load(Ordering::Relaxed), 0, "no ListDir walk");
}

fn runtime(dir: &std::path::Path) -> Runtime {
    let config = json!({
        "filesDir": dir.join("files"),
        "cacheDir": dir.join("cache"),
        "startDaemon": false,
    });
    for name in ["files", "cache"] {
        std::fs::create_dir_all(dir.join(name)).expect("fixture dir");
    }
    Runtime::detached(HostSettings::parse(&config.to_string()).expect("settings"))
}

fn call(rt: &Runtime, method: &str, args: Value) -> Result<Value, ApiError> {
    crate::mobile::dispatch::dispatch(rt, method, &args)
}

fn wait_done(rt: &Runtime, id: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let snapshot = rt
            .hub()
            .with(|state| state.tasks.get(id).map(|record| record.snapshot()))
            .expect("task exists");
        if matches!(
            snapshot["state"].as_str(),
            Some("done" | "failed" | "canceled")
        ) {
            return snapshot;
        }
        assert!(Instant::now() < deadline, "task {id} did not finish");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn review_task_local_results_answer_remote_flag_notes_and_release() {
    let dir = tempfile::tempdir().expect("temp dir");
    let rt = runtime(dir.path());
    let data = dir.path().join("data");
    std::fs::create_dir_all(data.join("sub")).expect("data");
    std::fs::write(data.join("a.bin"), vec![1u8; 4096]).expect("a");
    std::fs::write(data.join("sub/b.bin"), vec![1u8; 4096]).expect("b");
    let location = data.to_string_lossy().into_owned();

    let started = call(&rt, "analyze.start", json!({ "location": location })).expect("start");
    assert_eq!(started["remote"], false);
    let task = started["taskId"].as_str().expect("task").to_string();
    assert_eq!(wait_done(&rt, &task)["state"], "done");
    let issues = call(&rt, "analyze.issues", json!({ "taskId": task })).expect("issues");
    assert!(issues["notes"].is_array(), "{issues}");
    let released = call(&rt, "analyze.release", json!({ "taskId": task })).expect("release");
    assert_eq!(released["released"], true);
    let gone = call(&rt, "analyze.node", json!({ "taskId": task, "path": [] }));
    assert_eq!(gone.map_err(|error| error.kind).unwrap_err(), "not_found");

    let started = call(
        &rt,
        "reclaim.start",
        json!({ "location": location, "minSize": 1024 }),
    )
    .expect("reclaim");
    assert_eq!(started["remote"], false);
    let task = started["taskId"].as_str().expect("task").to_string();
    assert_eq!(wait_done(&rt, &task)["state"], "done");
    let groups = call(&rt, "reclaim.groups", json!({ "taskId": task })).expect("groups");
    assert_eq!(groups.as_array().map(Vec::len), Some(1));
    let released = call(&rt, "reclaim.release", json!({ "taskId": task })).expect("release");
    assert_eq!(released["released"], true);
}
