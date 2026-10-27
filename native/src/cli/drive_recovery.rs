//! CLI and MCP use the daemon's one offline ownership transaction.
use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Args)]
pub(super) struct SaveRemoveArgs {
    #[arg(help = "Mount ID from se drive list")]
    selector: String,
    #[arg(
        long,
        value_name = "FOLDER",
        help = "Existing folder for a private complete recovery package"
    )]
    destination: PathBuf,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
pub(super) struct RecoveryArgs {
    #[command(subcommand)]
    command: RecoveryCommand,
}

#[derive(Subcommand)]
enum RecoveryCommand {
    #[command(about = "List retained recovery packages; never mounts or contacts a provider")]
    List {
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Verify and privately import a versioned package without activating it")]
    Import {
        package: PathBuf,
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Delete an inactive recovery and the cached changes it retains; the saved package stays")]
    Discard {
        id: String,
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Restore an inactive package as a stopped mount; auto-remount remains off")]
    Reactivate {
        id: String,
        #[arg(long)]
        json: bool,
    },
}

pub(super) fn save_remove(args: SaveRemoveArgs) -> Result<i32, String> {
    let id = super::resolve_selector(&args.selector)?;
    let record = crate::daemon::save_remove_mount(id, absolute(args.destination)?)?;
    print_record(&record, args.json)?;
    Ok(0)
}

pub(super) fn run(args: RecoveryArgs) -> Result<i32, String> {
    match args.command {
        RecoveryCommand::List { json } => {
            let records = crate::daemon::list_mount_recoveries()?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&records).map_err(|error| error.to_string())?
                );
            } else if records.is_empty() {
                println!("recoveries\t0");
            } else {
                for record in records {
                    print_record(&record, false)?;
                }
            }
        }
        RecoveryCommand::Import { package, json } => {
            let record = crate::daemon::import_mount_recovery(absolute(package)?)?;
            print_record(&record, json)?;
        }
        RecoveryCommand::Discard { id, json } => {
            crate::daemon::discard_mount_recovery(id.clone())?;
            if json {
                println!("{}", serde_json::json!({ "discarded": id }));
            } else {
                println!("recovery\tgelöscht\t{id}");
            }
        }
        RecoveryCommand::Reactivate { id, json } => {
            let snapshot = crate::daemon::reactivate_mount_recovery(id)?;
            super::print_snapshot(&snapshot, json)?;
            if !json {
                println!("recovery\tbereit\tauto-remount=off\tAusdrücklich mit se drive retry erneut verbinden");
            }
        }
    }
    Ok(0)
}

fn absolute(path: PathBuf) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path)
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .map_err(|error| error.to_string())
    }
}

fn print_record(record: &crate::mount::InactiveMountRecovery, json: bool) -> Result<(), String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(record).map_err(|error| error.to_string())?
        );
    } else {
        println!(
            "recovery\t{}\t{}\t{}",
            record.id,
            record.config.id,
            record.package_path.display()
        );
        println!("manifest-sha256\t{}", record.manifest_sha256);
        println!(
            "retained-cache\t{}\treactivated\t{}",
            record.retained_cache.display(),
            record.reactivated
        );
        println!(
            "local-snapshot-complete\t{}\tsealed\t{}",
            record.local_snapshot_complete, record.sealed
        );
        let unknown = record
            .gaps
            .iter()
            .filter(|gap| gap.kind == crate::mount::RecoveryGapKind::JournalInventoryUnknown)
            .count();
        println!(
            "not-backed-up-count\t{}",
            record.gap_count.saturating_sub(unknown as u64)
        );
        println!("inventory-unknown-count\t{unknown}");
        if let Some(error) = &record.package_error {
            println!("package-error\t{error}");
        }
        for gap in &record.gaps {
            let label = if gap.kind == crate::mount::RecoveryGapKind::JournalInventoryUnknown {
                "inventory-unknown"
            } else {
                "not-backed-up"
            };
            println!("{label}\t{:?}\t{}\t{}", gap.kind, gap.subject, gap.detail);
        }
        println!("inactive\tkein automatisches Einbinden\tOriginalcache bleibt erhalten");
    }
    Ok(())
}
