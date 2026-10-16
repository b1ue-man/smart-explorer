//! `se share trash`: the Windows host trash of the desktop tab „Papierkorb“
//! – files a device with write access moved into the Smart Explorer trash on
//! this host, listed page by page and restored without replacing anything.
//! Other hosts keep their OS or app trash and report that the catalog is not
//! available here.
use clap::{Args, Subcommand};

#[derive(Args)]
pub(super) struct TrashArgs {
    #[arg(long, global = true, help = "Print machine-readable JSON")]
    json: bool,
    #[command(subcommand)]
    command: Option<TrashCommand>,
}

#[derive(Subcommand)]
enum TrashCommand {
    #[command(about = "List one page of the host trash (default)")]
    List {
        #[arg(long, help = "Continue after this cursor (printed as `next`)")]
        cursor: Option<String>,
    },
    #[command(
        about = "Restore one entry to its original place; an existing file is never replaced"
    )]
    Restore {
        #[arg(help = "Entry id printed by `se share trash list`")]
        id: String,
    },
}

pub(super) fn run(args: TrashArgs) -> Result<(), String> {
    match args.command.unwrap_or(TrashCommand::List { cursor: None }) {
        TrashCommand::List { cursor } => list(cursor.as_deref(), args.json),
        TrashCommand::Restore { id } => {
            let outcome =
                crate::host_trash::restore(&id).map_err(|error| format!("host trash: {error}"))?;
            let state = match outcome {
                crate::host_trash::RestoreOutcome::Restored => "restored",
                crate::host_trash::RestoreOutcome::AlreadyAtOriginal => "already_at_original",
            };
            if args.json {
                println!("{}", serde_json::json!({ "id": id, "result": state }));
            } else {
                println!("trash\t{id}\t{state}");
            }
            Ok(())
        }
    }
}

fn list(cursor: Option<&str>, json: bool) -> Result<(), String> {
    let page = crate::host_trash::list(cursor).map_err(|error| format!("host trash: {error}"))?;
    if json {
        let entries = page
            .entries
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "id": entry.id,
                    "original": entry.original,
                    "size": entry.size,
                    "created_ms": entry.created_ms,
                    "state": entry.state.label(),
                    "restorable": entry.can_restore(),
                    "detail": entry.detail,
                })
            })
            .collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "entries": entries,
                "next": page.next,
                "issues": page.issues,
                "suppressed_issues": page.suppressed_issues,
            }))
            .map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    if page.entries.is_empty() {
        println!("trash\t0");
    }
    for entry in &page.entries {
        println!(
            "trash\t{}\t{}\tstate={}\trestorable={}\tsize={}",
            entry.id,
            clean(&entry.original),
            clean(entry.state.label()),
            entry.can_restore(),
            entry
                .size
                .map_or_else(|| "-".to_string(), |size| size.to_string()),
        );
    }
    for issue in &page.issues {
        println!("issue\t{}", clean(issue));
    }
    if page.suppressed_issues > 0 {
        println!("issues_suppressed\t{}", page.suppressed_issues);
    }
    if let Some(next) = &page.next {
        println!("next\t{next}");
    }
    Ok(())
}

fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}
