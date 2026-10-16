//! `se sync`: the saved sync setups of the desktop window "Sync-Setups" and
//! of the Android sync page — list, show, add, edit (every field of the
//! desktop editor), remove, enable/disable, the live log and its detail
//! level. Drafts go through the shared `JobEditor`, so defaults, validation
//! and messages are the GUI's; the files are the GUI's job files.
#[path = "sync_jobs/options.rs"]
mod options;
#[path = "sync_jobs/output.rs"]
mod output;

use clap::{Args, Subcommand};
use clap_complete::ArgValueCandidates;

use crate::syncjobs::editor::JobEditor;
use crate::syncjobs::SyncJob;

const SYNC_HELP: &str = "\
Manage the saved sync setups of the GUI (desktop \"Sync-Setups\", Android
\"Sync\"). The background worker runs them on their trigger. Every setting of
the desktop editor is an option of `add` and `edit`; option names are the keys
of `se sync show --json`. Values are checked exactly like in the GUI.

A job is selected by its id, its exact name, or an id prefix of at least four
characters (`se sync list` shows them).

Examples:
  se sync list
  se sync add --name Docs --source /home/alice/Docs --target @nas:/backup/docs --direction a2b
  se sync edit Docs --trigger calendar --cal-time 02:30 --cal-weekdays mon,thu
  se sync edit Docs --trigger realtime --rt-debounce-secs 10 --rt-poll-secs 300
  se sync edit Docs --ignore '**/*.tmp' --ignore 'node_modules/**'
  se sync show Docs --json
  se sync disable Docs
  se sync log Docs --verbose true
  se sync remove Docs";

#[derive(Args)]
#[command(long_about = SYNC_HELP)]
pub(super) struct SyncArgs {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "List the saved sync setups with schedule and state")]
    List {
        #[arg(long, help = "Print machine-readable JSON")]
        json: bool,
    },
    #[command(about = "Show every setting and the state of one setup")]
    Show(JobOutputArgs),
    #[command(about = "Save a new sync setup (defaults as in the GUI)")]
    Add(AddArgs),
    #[command(about = "Change settings of a sync setup; options not given stay")]
    Edit(EditArgs),
    #[command(about = "Remove a sync setup (its synchronization state and versions stay)")]
    Remove(JobArgs),
    #[command(about = "Activate the schedule of a setup")]
    Enable(JobArgs),
    #[command(about = "Deactivate the schedule of a setup (manual runs stay possible)")]
    Disable(JobArgs),
    #[command(about = "Print the live log of a setup and set its detail level")]
    Log(LogArgs),
}

#[derive(Args)]
struct JobArgs {
    #[arg(
        value_name = "JOB",
        help = "Id, exact name or id prefix from `se sync list`",
        add = ArgValueCandidates::new(job_candidates)
    )]
    job: String,
}

#[derive(Args)]
struct JobOutputArgs {
    #[command(flatten)]
    job: JobArgs,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Args)]
struct AddArgs {
    #[command(flatten)]
    options: options::JobOptions,
    #[arg(long, help = "Print the saved setup as JSON")]
    json: bool,
}

#[derive(Args)]
struct EditArgs {
    #[command(flatten)]
    job: JobArgs,
    #[command(flatten)]
    options: options::JobOptions,
    #[arg(long, help = "Print the saved setup as JSON")]
    json: bool,
}

#[derive(Args)]
struct LogArgs {
    #[command(flatten)]
    job: JobArgs,
    #[arg(
        long,
        value_name = "OFFSET",
        help = "Print complete lines from this byte offset (default: the last page)"
    )]
    from: Option<u64>,
    #[arg(
        long,
        help = "Log unchanged entries one by one from the next run on (true/false), like the log window's setting"
    )]
    verbose: Option<bool>,
    #[arg(long, help = "Print text, next offset and size as JSON")]
    json: bool,
}

pub(super) fn run(args: SyncArgs) -> Result<i32, String> {
    match args.command.unwrap_or(Command::List { json: false }) {
        Command::List { json } => list(json),
        Command::Show(args) => {
            let job = find_job(&args.job.job)?;
            print_job(&job, args.json)
        }
        Command::Add(args) => {
            let (source, target) = match args.options.sides() {
                (Some(source), Some(target)) => (source.to_string(), target.to_string()),
                _ => return Err("se sync add needs --source and --target".into()),
            };
            let mut editor = JobEditor::blank(source, target);
            args.options.apply(&mut editor)?;
            let job = editor.build_sync_job(None)?;
            save(&job)?;
            print_saved("Saved sync setup", &job, args.json)
        }
        Command::Edit(args) => {
            let existing = find_job(&args.job.job)?;
            refuse_while_running(&existing)?;
            let mut editor = JobEditor::from_job(&existing);
            args.options.apply(&mut editor)?;
            let job = editor.build_sync_job(Some(&existing))?;
            save(&job)?;
            print_saved("Saved sync setup", &job, args.json)
        }
        Command::Remove(args) => {
            let job = find_job(&args.job)?;
            refuse_while_running(&job)?;
            crate::syncjobs::remove(&job.id)
                .map_err(|error| format!("remove sync setup {}: {error}", job.id))?;
            println!("Removed sync setup {}\t{}", job.id, job.name);
            Ok(0)
        }
        Command::Enable(args) => set_enabled(&args.job, true),
        Command::Disable(args) => set_enabled(&args.job, false),
        Command::Log(args) => log(args),
    }
}

fn list(json: bool) -> Result<i32, String> {
    let report =
        crate::syncjobs::load_report().map_err(|error| format!("load sync setups: {error}"))?;
    let states = crate::syncjobs::load_job_states(&report.jobs);
    let now = now_secs();
    if json {
        let mut rows: Vec<serde_json::Value> = report
            .jobs
            .iter()
            .map(|job| output::job_value(job, states.get(&job.id), now))
            .collect();
        rows.extend(
            report.broken.iter().map(
                |broken| serde_json::json!({ "id": broken.id, "broken_config": broken.error }),
            ),
        );
        println!("{}", pretty(&serde_json::Value::Array(rows))?);
        return Ok(0);
    }
    for job in &report.jobs {
        println!("{}", output::list_line(job, states.get(&job.id), now));
    }
    for broken in &report.broken {
        println!(
            "{}\tbroken\t{}",
            broken.id,
            broken.error.replace(['\r', '\n'], " ")
        );
    }
    if report.jobs.is_empty() && report.broken.is_empty() {
        println!("sync setups\t0");
    }
    Ok(0)
}

fn print_job(job: &SyncJob, json: bool) -> Result<i32, String> {
    let state = crate::syncjobs::load_job_state(&job.id)
        .map_err(|error| format!("load the state of {}: {error}", job.id))?;
    let value = output::job_value(job, Some(&state), now_secs());
    if json {
        println!("{}", pretty(&value)?);
    } else {
        for line in output::show_lines(&value) {
            println!("{line}");
        }
    }
    Ok(0)
}

fn print_saved(headline: &str, job: &SyncJob, json: bool) -> Result<i32, String> {
    if json {
        return print_job(job, true);
    }
    println!(
        "{headline} {}\t{}\t{}",
        job.id,
        job.name,
        output::schedule_text(job)
    );
    Ok(0)
}

fn save(job: &SyncJob) -> Result<(), String> {
    crate::syncjobs::upsert(job).map_err(|error| format!("save sync setup {}: {error}", job.id))
}

fn set_enabled(selector: &str, enabled: bool) -> Result<i32, String> {
    let mut job = find_job(selector)?;
    job.enabled = enabled;
    save(&job)?;
    println!(
        "{} sync setup {}\t{}",
        if enabled { "Enabled" } else { "Disabled" },
        job.id,
        job.name
    );
    Ok(0)
}

fn log(args: LogArgs) -> Result<i32, String> {
    let job = find_job(&args.job.job)?;
    if let Some(verbose) = args.verbose {
        crate::bisync::set_job_log_verbose(&job.id, verbose)
            .map_err(|error| format!("save the log detail level: {error}"))?;
    }
    let chunk = crate::bisync::read_job_log(&job.id, args.from)
        .map_err(|error| format!("read the log of {}: {error}", job.id))?;
    if args.json {
        println!(
            "{}",
            pretty(&serde_json::json!({
                "text": chunk.text,
                "next": chunk.next,
                "size": chunk.size,
                "restarted": chunk.restarted,
                "verbose": crate::bisync::job_log_verbose(&job.id),
            }))?
        );
    } else {
        print!("{}", chunk.text);
        eprintln!(
            "se: log of {} up to offset {} (verbose {})",
            job.id,
            chunk.next,
            if crate::bisync::job_log_verbose(&job.id) {
                "on"
            } else {
                "off"
            }
        );
    }
    Ok(0)
}

/// The Android facade refuses changes to a job its worker is running; the
/// terminal does the same with the job's run mark.
fn refuse_while_running(job: &SyncJob) -> Result<(), String> {
    let state = crate::syncjobs::load_job_state(&job.id)
        .map_err(|error| format!("load the state of {}: {error}", job.id))?;
    if state.running_now(now_secs()).is_some() {
        return Err(format!(
            "sync setup {} is running right now; wait for it or cancel it first",
            job.id
        ));
    }
    Ok(())
}

fn find_job(selector: &str) -> Result<SyncJob, String> {
    let jobs = crate::syncjobs::load_report()
        .map_err(|error| format!("load sync setups: {error}"))?
        .jobs;
    select_job(jobs, selector)
}

/// Exact id, then exact name, then an id prefix of at least four characters;
/// several matches are refused with their ids.
fn select_job(jobs: Vec<SyncJob>, selector: &str) -> Result<SyncJob, String> {
    let selector = selector.trim();
    if selector.is_empty() {
        return Err("the sync setup selector must not be empty".into());
    }
    let rules: [&dyn Fn(&SyncJob) -> bool; 3] = [
        &|job: &SyncJob| job.id == selector,
        &|job: &SyncJob| job.name == selector,
        &|job: &SyncJob| selector.len() >= 4 && job.id.starts_with(selector),
    ];
    for rule in rules {
        let found: Vec<&SyncJob> = jobs.iter().filter(|job| rule(*job)).collect();
        match found.as_slice() {
            [] => continue,
            [job] => return Ok((*job).clone()),
            many => {
                return Err(format!(
                    "sync setup selector {selector:?} is ambiguous: {}",
                    many.iter()
                        .map(|job| job.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            }
        }
    }
    Err(format!(
        "sync setup not found: {selector}; `se sync list` shows the saved setups"
    ))
}

fn job_candidates() -> Vec<clap_complete::CompletionCandidate> {
    crate::syncjobs::load_report()
        .map(|report| report.jobs)
        .unwrap_or_default()
        .iter()
        .map(|job| {
            let help = format!("{} · {}", job.name, output::schedule_text(job))
                .replace(['\t', '\r', '\n'], " ");
            clap_complete::CompletionCandidate::new(&job.id)
                .help(Some(clap::builder::StyledStr::from(help)))
        })
        .collect()
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn pretty(value: &serde_json::Value) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::super::{Cli, Command as RootCommand};
    use super::{select_job, Command};
    use crate::syncjobs::SyncJob;

    fn parse(arguments: &[&str]) -> Result<Command, clap::Error> {
        let mut all = vec!["se", "sync"];
        all.extend_from_slice(arguments);
        Cli::try_parse_from(all).map(|cli| match cli.command {
            RootCommand::Sync(args) => args.command.expect("subcommand"),
            _ => panic!("wrong command"),
        })
    }

    #[test]
    fn luecken_task_sync_cli_parses_setup_commands() {
        assert!(Cli::try_parse_from(["se", "sync"]).is_ok());
        assert!(matches!(
            parse(&["list", "--json"]).unwrap(),
            Command::List { json: true }
        ));
        assert!(matches!(
            parse(&[
                "add",
                "--source",
                "/a",
                "--target",
                "/b",
                "--direction",
                "a2b",
                "--trigger",
                "realtime",
                "--rt-debounce-secs",
                "5",
                "--ignore",
                "*.tmp",
                "--ignore",
                "x/**",
                "--enabled",
                "false",
                "--json",
            ])
            .unwrap(),
            Command::Add(_)
        ));
        assert!(matches!(
            parse(&["edit", "Docs", "--verify", "true", "--max-delete-pct", "40"]).unwrap(),
            Command::Edit(_)
        ));
        let refused: [&[&str]; 4] = [
            &["add", "--direction", "sideways"],
            &["edit", "Docs", "--max-delete-pct", "300"],
            &["edit", "Docs", "--ignore", "a", "--clear-ignore"],
            &["edit", "Docs", "--verify", "maybe"],
        ];
        for bad in refused {
            assert!(parse(bad).is_err(), "{bad:?} must be refused");
        }
        match parse(&["log", "Docs", "--verbose", "true", "--from", "10"]).unwrap() {
            Command::Log(args) => {
                assert_eq!(args.verbose, Some(true));
                assert_eq!(args.from, Some(10));
            }
            _ => panic!("wrong subcommand"),
        }
        for command in ["remove", "enable", "disable", "show"] {
            assert!(parse(&[command, "Docs"]).is_ok(), "{command}");
        }
    }

    #[test]
    fn luecken_task_sync_cli_selects_by_id_name_or_prefix() {
        let mut first = SyncJob::new("Docs".into(), "/a".into(), "/b".into());
        first.id = "18a2f0c".into();
        let mut second = SyncJob::new("Photos".into(), "/c".into(), "/d".into());
        second.id = "18a2f9d".into();
        let jobs = vec![first, second];
        assert_eq!(select_job(jobs.clone(), "Photos").unwrap().id, "18a2f9d");
        assert_eq!(select_job(jobs.clone(), "18a2f0c").unwrap().name, "Docs");
        assert_eq!(select_job(jobs.clone(), "18a2f9").unwrap().name, "Photos");
        assert!(select_job(jobs.clone(), "18a2")
            .unwrap_err()
            .contains("ambiguous"));
        assert!(select_job(jobs.clone(), "18a").is_err());
        assert!(select_job(jobs, "").is_err());
    }
}
