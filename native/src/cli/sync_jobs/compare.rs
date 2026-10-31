//! `se sync compare`: the desktop button "Vergleichen" — read both sides of
//! a saved setup with its own options and filters and print what a run
//! would do, without changing either side (`bisync::preview_with`, the same
//! call as `app/core/preview_core.rs`).
use crate::bisync::{Action, Preview};
use crate::syncjobs::SyncJob;
use serde_json::json;
use std::sync::atomic::AtomicBool;

/// Compares `job`'s two sides; `Err` when the setup cannot be read or
/// compared, so the exit status is non-zero.
pub(super) fn compare(job: &SyncJob, now_secs: i64, json_out: bool) -> Result<i32, String> {
    if job.is_vault() {
        return Err(format!(
            "{} is a vault sync; compare it with `se vault compare`",
            job.name
        ));
    }
    job.validate()
        .map_err(|error| format!("invalid setup {}: {error}", job.id))?;
    let globs = job.checked_glob_set()?;
    let (min_size, max_size, after, before) = job.checked_filter_bounds(now_secs)?;
    let options = job.checked_opts(true)?;
    let (a, root_a) =
        crate::connect::resolve_endpoint(&job.source).map_err(|error| error.to_string())?;
    let (b, root_b) =
        crate::connect::resolve_endpoint(&job.target).map_err(|error| error.to_string())?;
    let filter = crate::bisync::WalkFilter {
        include_hidden: job.include_hidden,
        ignore: &globs,
        min_size,
        max_size,
        after_mtime_ms: after,
        before_mtime_ms: before,
    };
    let a = crate::vfs::sync_backend(a);
    let b = crate::vfs::sync_backend(b);
    let cancel = AtomicBool::new(false);
    let started = std::time::Instant::now();
    let preview = crate::bisync::preview_with(
        &*a,
        &root_a,
        &*b,
        &root_b,
        options,
        &cancel,
        &filter,
        crate::bisync::RunSettings::for_job(&job.id),
    );
    let seconds = started.elapsed().as_secs_f64();
    if let Some(error) = &preview.error {
        return Err(format!("compare {}: {error}", job.name));
    }
    if json_out {
        let text = serde_json::to_string_pretty(&report(job, &preview, seconds))
            .map_err(|error| format!("JSON: {error}"))?;
        println!("{text}");
    } else {
        print_text(job, &preview, seconds);
    }
    Ok(0)
}

fn kind(action: &Action) -> (&'static str, &str) {
    match action {
        Action::CopyAtoB(rel) => ("copy_a_to_b", rel),
        Action::CopyBtoA(rel) => ("copy_b_to_a", rel),
        Action::FinalizeMoveAtoB(rel) => ("finish_move_a_to_b", rel),
        Action::FinalizeMoveBtoA(rel) => ("finish_move_b_to_a", rel),
        Action::DeleteA(rel) => ("delete_a", rel),
        Action::DeleteB(rel) => ("delete_b", rel),
        Action::KeepBothAtoB(rel) => ("keep_both_a_wins", rel),
        Action::KeepBothBtoA(rel) => ("keep_both_b_wins", rel),
    }
}

fn counts(preview: &Preview) -> std::collections::BTreeMap<&'static str, u64> {
    let mut counts = std::collections::BTreeMap::new();
    for action in &preview.actions {
        *counts.entry(kind(action).0).or_insert(0) += 1;
    }
    counts
}

fn report(job: &SyncJob, preview: &Preview, seconds: f64) -> serde_json::Value {
    let actions: Vec<_> = preview
        .actions
        .iter()
        .map(|action| {
            let (kind, rel) = kind(action);
            json!({ "action": kind, "path": rel })
        })
        .collect();
    let omissions: serde_json::Map<String, serde_json::Value> = preview
        .omissions
        .counts()
        .into_iter()
        .map(|(kind, count)| (kind.as_str().to_string(), json!(count)))
        .collect();
    json!({
        "id": job.id,
        "name": job.name,
        "source": job.source,
        "target": job.target,
        "seconds": seconds,
        "source_files": preview.a_files,
        "target_files": preview.b_files,
        "counts": counts(preview),
        "actions": actions,
        "conflicts": preview.conflicts.iter().map(|conflict| conflict.rel.as_str()).collect::<Vec<_>>(),
        "omissions": omissions,
        "duplicate_removals": preview.duplicate_removals,
        "blocked": preview.blocked.as_ref().map(|block| block.message()),
    })
}

fn print_text(job: &SyncJob, preview: &Preview, seconds: f64) {
    println!("Compared {}\t{} <-> {}", job.name, job.source, job.target);
    println!(
        "Files\tsource {}\ttarget {}\t{seconds:.1} s",
        preview.a_files, preview.b_files
    );
    let counts = counts(preview);
    if counts.is_empty() && preview.conflicts.is_empty() {
        println!("Nothing to do: both sides match");
    }
    for (kind, count) in &counts {
        println!("{kind}\t{count}");
    }
    if !preview.conflicts.is_empty() {
        println!("conflicts\t{}", preview.conflicts.len());
    }
    if let Some(summary) = preview.omissions.summary() {
        println!("Omitted\t{summary}");
    }
    if preview.duplicate_removals > 0 {
        println!("Duplicate removals\t{}", preview.duplicate_removals);
    }
    if let Some(block) = &preview.blocked {
        println!("Blocked\t{}", block.message());
    }
    for action in &preview.actions {
        let (kind, rel) = kind(action);
        println!("{kind}\t{rel}");
    }
    for conflict in &preview.conflicts {
        println!("conflict\t{}", conflict.rel);
    }
}
