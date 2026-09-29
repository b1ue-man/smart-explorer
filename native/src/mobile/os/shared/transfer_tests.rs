//! `fs.transfer` with a connection involved becomes one engine job that
//! starts at once (no scan before), and the task shows the search.
use super::super::drive::task_message;
use super::*;
use crate::transfer::{TransferKind, TransferProgress};
use serde_json::json;

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
