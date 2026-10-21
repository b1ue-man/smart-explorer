//! `fs.transfer` with a connection involved becomes one engine job that
//! starts at once (no scan before), and the task shows the search.
use super::super::config::HostSettings;
use super::super::drive::{limiter_code, limiter_text_with, report_transfer, task_message};
use super::super::tasks::TaskTable;
use super::*;
use crate::transfer::{Limiter, TransferKind, TransferProgress};
use serde_json::json;
use std::time::{Duration, Instant};

/// A connection handle for building jobs; no I/O happens through it.
fn connection(root: &str) -> Endpoint {
    Endpoint::Remote(Arc::new(crate::vfs::LocalBackend::new(root)))
}

fn labels() -> (String, String) {
    ("Quelle".to_string(), "Ziel".to_string())
}

fn paths(plan: &Plan) -> Vec<String> {
    plan.sources
        .iter()
        .map(|source| source.path.clone())
        .collect()
}

#[test]
fn transfer_engine_task_android_filtered_upload_walks_below_the_base() {
    let plan = parse(&json!({
        "sources": ["/storage/emulated/0/DCIM/Camera", "/storage/emulated/0/DCIM/Neu"],
        "targetDir": "sftp://u@h:22/sicherung",
        "filter": { "extensions": "jpg" },
        "baseDir": "/storage/emulated/0/DCIM",
    }))
    .expect("plan");
    let job = plan_job(
        &plan,
        paths(&plan),
        Endpoint::Local,
        connection("/"),
        labels(),
    );
    assert_eq!(job.kind(), TransferKind::Upload);
    assert_eq!(job.target_dir, "/sicherung");
    assert_eq!(
        job.items,
        JobItems::Roots {
            paths: vec![
                "/storage/emulated/0/DCIM/Camera".to_string(),
                "/storage/emulated/0/DCIM/Neu".to_string(),
            ],
            base: Some("/storage/emulated/0/DCIM".to_string()),
        }
    );
    let (filter, base) = job.filter.clone().expect("filter travels with the job");
    assert_eq!(base, "/storage/emulated/0/DCIM");
    assert_eq!(filter.extensions, crate::filter::parse_extensions("jpg"));
    assert_eq!(job.conflict, Conflict::Rename);
    assert_eq!(job.layout, Layout::Tree);
    assert!(job.validate().is_ok());
}

#[test]
fn transfer_engine_task_android_connection_targets_never_replace() {
    let plan = parse(&json!({
        "sources": ["sftp://u@h:22/docs/bericht.pdf"],
        "targetDir": "/storage/emulated/0/Download",
        "conflict": "replace",
    }))
    .expect("plan");
    // `conflict` belongs to local→local copies (api.md); downloads number.
    assert_eq!(plan.conflict, Conflict::Overwrite);
    let job = plan_job(
        &plan,
        paths(&plan),
        connection("/"),
        Endpoint::Local,
        labels(),
    );
    assert_eq!(job.kind(), TransferKind::Download);
    assert_eq!(job.conflict, Conflict::Rename);
    assert_eq!(
        job.items,
        JobItems::Roots {
            paths: vec!["/docs/bericht.pdf".to_string()],
            base: None,
        }
    );
    assert!(job.filter.is_none());
    assert_eq!(job.source_label, "Quelle");
    assert_eq!(job.target_label, "Ziel");

    let between = parse(&json!({
        "sources": ["sftp://u@h:22/docs/a"],
        "targetDir": "webdav://w@dav:443/docs",
    }))
    .expect("plan");
    let job = plan_job(
        &between,
        paths(&between),
        connection("/"),
        connection("/"),
        labels(),
    );
    assert_eq!(job.kind(), TransferKind::RemoteCopy);
    assert_eq!(job.target_dir, "/docs");
}

#[test]
fn transfer_engine_task_android_transfer_shows_the_search_and_notes() {
    let mut progress = TransferProgress::new(TransferKind::Upload, "Upload", 0, 0);
    assert_eq!(task_message(&progress), None);
    progress.discovering = true;
    progress.files_total = 1234;
    assert_eq!(
        task_message(&progress).as_deref(),
        Some("Suche Dateien… 1234 gefunden")
    );
    progress.note = Some("wartet auf sftp://h".to_string());
    assert_eq!(
        task_message(&progress).as_deref(),
        Some("Suche Dateien… 1234 gefunden"),
        "the search comes first"
    );
    progress.discovering = false;
    assert_eq!(
        task_message(&progress).as_deref(),
        Some("wartet auf sftp://h")
    );
    progress.note = Some("  ".to_string());
    assert_eq!(task_message(&progress), None);
}

fn progress_with(limiter: Limiter, text: Option<&str>) -> TransferProgress {
    let mut progress = TransferProgress::new(TransferKind::Upload, "Upload", 0, 0);
    progress.limiter = limiter;
    progress.limiter_text = text.map(str::to_string);
    progress
}

/// Runs `report_transfer` once inside a real task and returns the final
/// snapshot of that task.
fn reported_task(private: bool, progress: TransferProgress) -> serde_json::Value {
    let dir = tempfile::tempdir().expect("temp dir");
    for name in ["files", "cache"] {
        std::fs::create_dir_all(dir.path().join(name)).expect("fixture dir");
    }
    let config = json!({
        "filesDir": dir.path().join("files"),
        "cacheDir": dir.path().join("cache"),
        "startDaemon": false,
    });
    let rt = Runtime::detached(HostSettings::parse(&config.to_string()).expect("settings"));
    let id = rt.spawn_task("transfer", "Kopieren".to_string(), move |ctx| {
        if private {
            ctx.keep_names_private();
        }
        report_transfer(ctx, &progress, &mut None);
        Ok(json!({}))
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let snapshot = rt
            .hub()
            .with(|state| state.tasks.get(&id).map(|record| record.snapshot()))
            .expect("task exists");
        if snapshot["state"] == "done" {
            return snapshot;
        }
        assert!(Instant::now() < deadline, "task did not finish: {snapshot}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn speed_task_android_task_json_carries_limiter_and_text() {
    let mut table = TaskTable::new(1);
    let id = table.create("transfer", "Kopieren".into(), Arc::default(), 0);
    let record = table.get_mut(&id).expect("task");
    let before = record.snapshot();
    assert_eq!(before["limiter"], "none");
    assert!(before["limiterText"].is_null());
    record.set_message("Suche Dateien… 3 gefunden");
    record.set_limiter(
        "remote",
        Some("Begrenzt durch Leitung oder Server (SFTP-Server)"),
    );
    let after = record.snapshot();
    assert_eq!(after["limiter"], "remote");
    assert_eq!(
        after["limiterText"],
        "Begrenzt durch Leitung oder Server (SFTP-Server)"
    );
    // The fields the app reads today stay as they were.
    for key in [
        "rateBps",
        "message",
        "errors",
        "result",
        "state",
        "doneBytes",
    ] {
        assert!(after.get(key).is_some(), "{key} missing: {after}");
    }
    assert_eq!(after["message"], "Suche Dateien… 3 gefunden");
    assert!(after["errors"]
        .as_array()
        .is_some_and(|list| list.is_empty()));
    // An empty text is no text; the code alone is kept.
    let record = table.get_mut(&id).expect("task");
    record.set_limiter("idle", Some(""));
    assert_eq!(record.snapshot()["limiter"], "idle");
    assert!(record.snapshot()["limiterText"].is_null());
}

#[test]
fn speed_task_android_private_task_text_has_no_names() {
    let drive = "Begrenzt durch das Laufwerk";
    let remote = "Begrenzt durch Leitung oder Server";
    for (limiter, named, plain) in [
        (
            Limiter::Source,
            "Begrenzt durch das Laufwerk (nvme0n1)",
            drive,
        ),
        (Limiter::Target, "Begrenzt durch das Laufwerk (C:)", drive),
        (
            Limiter::Remote,
            "Begrenzt durch Leitung oder Server (SFTP-Server)",
            remote,
        ),
    ] {
        let progress = progress_with(limiter, Some(named));
        assert_eq!(limiter_text_with(true, &progress).as_deref(), Some(plain));
        assert_eq!(limiter_text_with(false, &progress).as_deref(), Some(named));
    }
    // All other texts stay as the engine wrote them, private or not.
    for (limiter, text) in [
        (
            Limiter::Link,
            "Begrenzt durch die Leitung (12,5 MB/s von 125 MB/s)",
        ),
        (Limiter::SeCpu, "Begrenzt durch Smart Explorer (Rechenzeit)"),
        (Limiter::Memory, "Wartet auf Arbeitsspeicher (Puffer)"),
        (Limiter::Ramp, "Läuft an …"),
    ] {
        let progress = progress_with(limiter, Some(text));
        assert_eq!(limiter_text_with(true, &progress).as_deref(), Some(text));
    }
    // No text, no field.
    assert_eq!(
        limiter_text_with(true, &progress_with(Limiter::Source, None)),
        None
    );
    // Through a real private task the snapshot carries the plain text only.
    let task = reported_task(
        true,
        progress_with(
            Limiter::Source,
            Some("Begrenzt durch das Laufwerk (Tresor-Platte)"),
        ),
    );
    assert_eq!(task["limiterText"], drive);
    assert!(!task.to_string().contains("Tresor-Platte"), "{task}");
}

#[test]
fn speed_task_android_limiter_codes_cover_every_variant() {
    let all = [
        (Limiter::None, "none"),
        (Limiter::Ramp, "ramp"),
        (Limiter::Idle, "idle"),
        (Limiter::Source, "source"),
        (Limiter::Target, "target"),
        (Limiter::Remote, "remote"),
        (Limiter::Link, "link"),
        (Limiter::SeCpu, "se_cpu"),
        (Limiter::SeThread, "se_thread"),
        (Limiter::Memory, "memory"),
    ];
    for (limiter, code) in all {
        assert_eq!(limiter_code(limiter), code);
        // A new variant must be added to the list above.
        match limiter {
            Limiter::None
            | Limiter::Ramp
            | Limiter::Idle
            | Limiter::Source
            | Limiter::Target
            | Limiter::Remote
            | Limiter::Link
            | Limiter::SeCpu
            | Limiter::SeThread
            | Limiter::Memory => {}
        }
    }
    let mut codes: Vec<&str> = all.iter().map(|(_, code)| *code).collect();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), 10, "codes are unique");
}

#[test]
fn speed_task_android_report_transfer_sets_the_limiter() {
    let named = "Begrenzt durch Leitung oder Server (SFTP-Server)";
    let task = reported_task(false, progress_with(Limiter::Remote, Some(named)));
    assert_eq!(task["limiter"], "remote");
    assert_eq!(task["limiterText"], named);
    let task = reported_task(false, progress_with(Limiter::None, None));
    assert_eq!(task["limiter"], "none");
    assert!(task["limiterText"].is_null());
    let task = reported_task(true, progress_with(Limiter::Remote, Some(named)));
    assert_eq!(task["limiter"], "remote");
    assert_eq!(task["limiterText"], "Begrenzt durch Leitung oder Server");
}
