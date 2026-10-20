//! `se vault sync add` and `se vault sync run` (vault plan C7.3, C5, U26-12).
//! A vault sync job keeps the encrypted storage of two sites alike without a
//! folder setup: the second side is a copy se creates (`--with replica`, the
//! default), an entered site of the same vault, or a new or empty folder at
//! any location. The fixed settings of a vault job (strict conflicts, no
//! move, no filters, versions in the app data, …) are not options here; the
//! selectable ones follow the names of `se sync add`. A run goes through the
//! vault sync with its rules and never resolves a conflict on its own.
use std::sync::atomic::AtomicBool;

use clap::{Args, Subcommand};
use clap_complete::ArgValueCandidates;
use serde_json::json;

use crate::bisync::{CompareMode, DeletePolicy, Direction, RunSettings};
use crate::syncjobs::editor::JobEditor;
use crate::syncjobs::{SyncJob, Trigger};
use crate::vault::{CopyTarget, JobPlace, RegistryStore, VaultRunRequest};

use super::complete::{job_candidates, site_candidates};
use super::output::{clean, pretty, site_of};

const DIRECTIONS: [&str; 3] = ["both", "a2b", "b2a"];
const DELETE_POLICIES: [&str; 3] = ["propagate", "mirror", "nodelete"];
const COMPARES: [&str; 2] = ["mtimesize", "checksum"];
const TRIGGERS: [&str; 6] = [
    "manual",
    "interval",
    "calendar",
    "realtime",
    "onstartup",
    "onconnect",
];
const WEEKDAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

#[derive(Args)]
pub(super) struct SyncArgs {
    #[command(subcommand)]
    command: SyncCommand,
}

#[derive(Subcommand)]
enum SyncCommand {
    #[command(about = "Create a vault sync job for a site")]
    Add(AddArgs),
    #[command(about = "Run a vault sync job now (conflicts stay for `se vault compare`)")]
    Run(RunArgs),
}

#[derive(Args)]
struct AddArgs {
    #[arg(
        value_name = "SITE",
        help = "The vault site to keep alike (side A)",
        add = ArgValueCandidates::new(site_candidates)
    )]
    site: String,
    #[arg(
        long,
        value_name = "replica|SITE|LOCATION",
        num_args = 0..=1,
        default_value = "replica",
        default_missing_value = "replica",
        help = "Side B: a new copy se keeps (replica), another site of the same vault, or a new or empty folder"
    )]
    with: String,
    #[arg(long, help = "Name of the job (default: Tresor <label>)")]
    name: Option<String>,
    #[arg(long, value_parser = DIRECTIONS, help = "both = two-way; a2b/b2a = one-way")]
    direction: Option<String>,
    #[arg(long, value_parser = DELETE_POLICIES, help = "One-way jobs: propagate, mirror or never delete (two-way jobs always carry deletions over)")]
    delete_policy: Option<String>,
    #[arg(long, value_parser = COMPARES, help = "How equal files are recognized")]
    compare: Option<String>,
    #[arg(long, value_parser = TRIGGERS, help = "What starts the job automatically")]
    trigger: Option<String>,
    #[arg(long, value_name = "MINUTES", help = "interval: run every N minutes")]
    interval_min: Option<u64>,
    #[arg(long, value_name = "HH:MM", help = "calendar: start time")]
    cal_time: Option<String>,
    #[arg(
        long,
        value_name = "DAYS",
        help = "calendar: comma-separated mon,tue,wed,thu,fri,sat,sun, or daily"
    )]
    cal_weekdays: Option<String>,
    #[arg(
        long,
        value_name = "DAY",
        help = "calendar: day of month 1-31 (0 = use the weekdays)"
    )]
    cal_monthday: Option<u8>,
    #[arg(
        long,
        value_name = "SECONDS",
        help = "realtime: wait after the last change"
    )]
    rt_debounce_secs: Option<u64>,
    #[arg(
        long,
        value_name = "SECONDS",
        help = "realtime: longest wait while changes keep coming (0 = automatic)"
    )]
    rt_max_latency_secs: Option<u64>,
    #[arg(
        long,
        value_name = "SECONDS",
        help = "realtime: poll sides without change events (0 = never)"
    )]
    rt_poll_secs: Option<u64>,
    #[arg(long, help = "Schedule active (true/false)")]
    enabled: Option<bool>,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Args)]
struct RunArgs {
    #[arg(
        value_name = "JOB",
        help = "ID, exact name or ID prefix of a vault job",
        add = ArgValueCandidates::new(job_candidates)
    )]
    job: String,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

pub(super) fn run(args: SyncArgs) -> Result<i32, String> {
    match args.command {
        SyncCommand::Add(args) => add(args),
        SyncCommand::Run(args) => run_job(args),
    }
}

/// `mon,wed` as the weekday mask of the editor (bit 0 = Monday); `daily`
/// is no day chosen.
pub(super) fn weekday_mask(text: &str) -> Result<u8, String> {
    let text = text.trim();
    if text.is_empty() || text.eq_ignore_ascii_case("daily") {
        return Ok(0);
    }
    let mut mask = 0u8;
    for day in text.split(',').map(str::trim).filter(|day| !day.is_empty()) {
        let index = WEEKDAYS
            .iter()
            .position(|known| known.eq_ignore_ascii_case(day))
            .ok_or_else(|| {
                format!("Unbekannter Wochentag „{day}“: mon,tue,wed,thu,fri,sat,sun oder daily")
            })?;
        mask |= 1 << index;
    }
    Ok(mask)
}

/// Lays the options over the draft of the new job (the checks of the GUI
/// editor and of the fixed vault values run when it is built).
fn apply(args: &AddArgs, editor: &mut JobEditor) -> Result<(), String> {
    let number = |slot: &mut String, value: Option<u64>| {
        if let Some(value) = value {
            *slot = value.to_string();
        }
    };
    if let Some(text) = &args.direction {
        editor.direction = Direction::parse(text).ok_or("Unbekannte Richtung")?;
    }
    if let Some(text) = &args.delete_policy {
        editor.delete_policy = DeletePolicy::parse(text).ok_or("Unbekannte Löschregel")?;
    }
    if let Some(text) = &args.compare {
        editor.compare = CompareMode::parse(text).ok_or("Unbekannter Vergleich")?;
    }
    if let Some(text) = &args.trigger {
        editor.trigger = Trigger::parse(text).ok_or("Unbekannter Auslöser")?;
    }
    number(&mut editor.interval_min, args.interval_min);
    if let Some(text) = &args.cal_time {
        editor.cal_time = text.clone();
    }
    if let Some(text) = &args.cal_weekdays {
        editor.cal_weekdays = weekday_mask(text)?;
    }
    number(&mut editor.cal_monthday, args.cal_monthday.map(u64::from));
    number(&mut editor.rt_debounce, args.rt_debounce_secs);
    number(&mut editor.rt_max_latency, args.rt_max_latency_secs);
    number(&mut editor.rt_poll, args.rt_poll_secs);
    if let Some(enabled) = args.enabled {
        editor.enabled = enabled;
    }
    Ok(())
}

/// Whether `text` names a place rather than a site: a locator, a path, or a
/// saved remote.
pub(super) fn looks_like_location(text: &str) -> bool {
    text.contains("://")
        || text.starts_with('@')
        || text.starts_with('/')
        || text.starts_with('\\')
        || text.starts_with('.')
        || text.get(1..2) == Some(":")
}

fn add(args: AddArgs) -> Result<i32, String> {
    let store = RegistryStore::open_default();
    let origin = site_of(&args.site)?;
    let with = args.with.trim().to_string();
    let location;
    let target = if with.eq_ignore_ascii_case("replica") {
        CopyTarget::Replica
    } else if let Ok(other) = store.resolve(&with) {
        // An entered site of the same vault (its ID: spelled exactly).
        location = other.id;
        CopyTarget::Site(&location)
    } else if looks_like_location(&with) {
        location = super::manage::location_of(&with)?;
        CopyTarget::Location(&location)
    } else {
        return Err(format!("Unbekannter Tresor oder Ort: {with}"));
    };
    let resolve = |location: &str| crate::connect::resolve_endpoint(location);
    let replicas = crate::vault::default_replicas_root();
    let taken = |id: &str| crate::syncjobs::job_id_taken(id);
    let save = |job: &SyncJob| -> Result<(), String> {
        let mut editor = JobEditor::from_job(job);
        apply(&args, &mut editor)?;
        let built = editor.build_sync_job(Some(job))?;
        crate::syncjobs::upsert(&built).map_err(|error| error.to_string())
    };
    let place = JobPlace {
        replicas_root: &replicas,
        resolve: &resolve,
        id_taken: &taken,
        save: &save,
    };
    let created =
        crate::vault::create_vault_job(&store, &origin.id, target, args.name.as_deref(), &place)?;
    if args.json {
        println!(
            "{}",
            pretty(&json!({
                "job": created.job.id,
                "site_a": created.site_a.id,
                "site_b": created.site_b.id,
            }))?
        );
    } else {
        println!(
            "created\t{}\t{}\t{}",
            created.job.id, created.site_a.id, created.site_b.id
        );
    }
    Ok(0)
}

fn run_job(args: RunArgs) -> Result<i32, String> {
    let job = crate::vault::find_vault_job(&args.job).map_err(|error| error.to_string())?;
    let cancel = AtomicBool::new(false);
    let run = crate::vault::run_vault_job(VaultRunRequest {
        job: &job,
        dry_run: false,
        cancel: &cancel,
        settings: RunSettings::for_job(&job.id),
        observer: None,
    });
    let stats = &run.outcome.stats;
    let errors = stats
        .errors
        .max(u64::try_from(run.outcome.errors.len()).unwrap_or(u64::MAX));
    let (conflicts, deferred) = (run.outcome.conflicts.len(), run.outcome.deferred.len());
    if args.json {
        println!(
            "{}",
            pretty(&json!({
                "copied_a_to_b": stats.a_to_b,
                "copied_b_to_a": stats.b_to_a,
                "deleted": stats.deleted,
                "conflicts": conflicts,
                "deferred": deferred,
                "errors": errors,
            }))?
        );
    } else {
        println!(
            "run\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            job.id, stats.a_to_b, stats.b_to_a, stats.deleted, conflicts, deferred, errors
        );
    }
    if let Some(reason) = &run.unavailable {
        eprintln!("se: {}", clean(reason));
        return Ok(1);
    }
    if let Some(block) = &run.outcome.blocked {
        eprintln!("se: {}", clean(&block.message()));
    }
    Ok(i32::from(errors > 0))
}
