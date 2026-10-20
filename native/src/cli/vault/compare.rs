//! `se vault compare`, `resolve` and `check` (vault plan C6, C7.3, U26-12):
//! the comparison mode of the vault sync jobs and „Tresor prüfen“ on the
//! command line. The comparison runs in the background worker (where the
//! keys are) and waits there for the next call, so `compare` and `resolve`
//! may be two processes. A vault job never resolves a conflict on its own;
//! every action here is called by the user. Text is read by `item` numbers
//! the comparison gave; nothing here prints a password.
use clap::Args;
use clap_complete::ArgValueCandidates;
use serde_json::json;

use crate::daemon::CompareViewFull;
use crate::vault::{CompareGroup, ItemAction, MERGE_MAX_BYTES};

use super::complete::{job_candidates, site_candidates};
use super::output::{clean, item_value, pretty, side_text, site_of, vault_error};

/// Group keys, and `all` for every group (also `structure`).
const GROUP_VALUES: [&str; 10] = [
    "conflict",
    "changed_a",
    "changed_b",
    "new_a",
    "new_b",
    "deleted_a",
    "deleted_b",
    "deferred",
    "structure",
    "all",
];

#[derive(Args)]
pub(super) struct CompareArgs {
    #[arg(
        value_name = "JOB",
        help = "ID, exact name or ID prefix of a vault job",
        add = ArgValueCandidates::new(job_candidates)
    )]
    job: String,
    #[arg(
        long,
        value_name = "GROUP",
        value_parser = GROUP_VALUES,
        help = "Group to list; repeat for several. Default: every group but structure; all = every group"
    )]
    group: Vec<String>,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Args)]
pub(super) struct ResolveArgs {
    #[arg(
        value_name = "JOB",
        help = "ID, exact name or ID prefix of a vault job (or a site after `se vault check`)",
        add = ArgValueCandidates::new(job_candidates)
    )]
    job: String,
    #[arg(
        value_name = "ITEM",
        required_unless_present = "all",
        conflicts_with = "all",
        help = "Entry number from `se vault compare`"
    )]
    item: Option<String>,
    #[arg(
        long,
        value_name = "ACTION",
        value_parser = ItemAction::KEYS,
        help = "What to do with the entry (the actions it lists in `compare`)"
    )]
    action: String,
    #[arg(
        long,
        value_name = "FILE",
        help = "merge: the merged text as a cleartext file of yours"
    )]
    merged_file: Option<std::path::PathBuf>,
    #[arg(
        long,
        help = "Every conflict with two files: keep-a or keep-b (no ITEM)"
    )]
    all: bool,
    #[arg(long, help = "Confirm the question a removal or a deletion asks")]
    yes: bool,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Args)]
pub(super) struct CheckArgs {
    #[arg(
        value_name = "SITE",
        help = "Vault ID, ID prefix (at least 4 digits) or label from `se vault list`",
        add = ArgValueCandidates::new(site_candidates)
    )]
    site: String,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

pub(super) fn groups_of(values: &[String]) -> Vec<&str> {
    if values.iter().any(|value| value == "all") {
        return CompareGroup::ALL.iter().map(|group| group.key()).collect();
    }
    values.iter().map(String::as_str).collect()
}

pub(super) fn compare_json(view: &CompareViewFull) -> serde_json::Value {
    let head = &view.head;
    json!({
        "job": head.job,
        "sides": {
            "a": { "site": head.site_a, "label": head.label_a },
            "b": { "site": head.site_b, "label": head.label_b },
        },
        "revision": head.revision,
        "counts": head.counts,
        "items": view.items.iter().map(item_value).collect::<Vec<_>>(),
    })
}

pub(super) fn compare(args: CompareArgs) -> Result<i32, String> {
    let view = crate::daemon::vault_compare(&args.job, &groups_of(&args.group))
        .map_err(|error| vault_error(&error, &args.job))?;
    if let Some(reason) = &view.head.unavailable {
        eprintln!("se: {}", clean(reason));
    }
    if args.json {
        println!("{}", pretty(&compare_json(&view))?);
        return Ok(0);
    }
    println!(
        "compare\t{}\t{}\t{}",
        clean(&view.head.job),
        view.head.revision,
        view.items.len()
    );
    for item in &view.items {
        println!(
            "item\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            item.item,
            item.group,
            item.kind,
            if item.path.is_empty() {
                "-".to_string()
            } else {
                clean(&item.path)
            },
            side_text(&item.a),
            side_text(&item.b),
            item.actions.join(","),
        );
    }
    Ok(0)
}

/// The hint behind a refusal that asks a question.
pub(super) fn with_hint(message: String, confirmed: bool) -> String {
    let asks = message.contains("Trotzdem übernehmen?") || message.contains("Kopie des Chiffrats");
    if asks && !confirmed {
        format!("{message} Mit --yes bestätigen.")
    } else {
        message
    }
}

pub(super) fn resolve(args: ResolveArgs) -> Result<i32, String> {
    let shown = |error: &crate::daemon::VaultIpcError| vault_error(error, &args.job);
    if args.all {
        if !matches!(args.action.as_str(), "keep-a" | "keep-b") {
            return Err("--all kennt nur --action keep-a oder keep-b".into());
        }
        let result = crate::daemon::vault_resolve(&args.job, None, &args.action, true, false)
            .map_err(|error| shown(&error))?;
        let failed = result.failed.len();
        if args.json {
            let rows: Vec<_> = result
                .failed
                .iter()
                .map(|row| json!({ "item": row.item, "error": row.error }))
                .collect();
            println!(
                "{}",
                pretty(&json!({ "resolved": result.resolved, "failed": rows }))?
            );
        } else {
            println!("resolved\t{}\t{failed}", result.resolved);
            for row in &result.failed {
                eprintln!("se: {}: {}", row.item, clean(&row.error));
            }
        }
        return Ok(i32::from(failed > 0));
    }
    let item = args.item.as_deref().ok_or("Eintragsnummer fehlt")?;
    if args.action == "merge" {
        let path = args
            .merged_file
            .as_ref()
            .ok_or("--action merge braucht --merged-file DATEI mit dem zusammengeführten Text")?;
        let text = read_merged(path)?;
        crate::daemon::vault_merge_apply(&args.job, item, &text).map_err(|error| shown(&error))?;
    } else {
        crate::daemon::vault_resolve(&args.job, Some(item), &args.action, false, args.yes)
            .map_err(|error| with_hint(shown(&error), args.yes))?;
    }
    if args.json {
        println!("{}", pretty(&json!({ "item": item, "result": "ok" }))?);
    } else {
        println!("resolved\t{item}\t{}", args.action);
    }
    Ok(0)
}

/// The merged text of the user's file (valid UTF-8, within the limit the
/// merge view takes).
fn read_merged(path: &std::path::Path) -> Result<String, String> {
    let size = std::fs::metadata(path)
        .map_err(|error| format!("{}: {error}", path.display()))?
        .len();
    if size > MERGE_MAX_BYTES {
        return Err(format!(
            "{}: der Text ist größer als {} KiB",
            path.display(),
            MERGE_MAX_BYTES / 1024
        ));
    }
    std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))
}

pub(super) fn check(args: CheckArgs) -> Result<i32, String> {
    let record = site_of(&args.site)?;
    let items =
        crate::daemon::vault_check(&record.id).map_err(|error| vault_error(&error, &record.id))?;
    if args.json {
        let rows: Vec<_> = items.iter().map(item_value).collect();
        println!("{}", pretty(&json!({ "site": record.id, "items": rows }))?);
    } else if items.is_empty() {
        println!("check\t{}\tok", record.id);
    }
    if !args.json {
        for item in &items {
            println!(
                "finding\t{}\t{}\t{}\t{}\t{}\t{}",
                item.item,
                item.finding.as_deref().unwrap_or("-"),
                item.kind,
                if item.path.is_empty() {
                    "-".to_string()
                } else {
                    clean(&item.path)
                },
                item.actions.join(","),
                item.note.as_deref().map_or_else(|| "-".to_string(), clean),
            );
        }
    }
    Ok(i32::from(!items.is_empty()))
}
