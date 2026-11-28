//! `se replace`: Notepad++'s "Replace in Files" below a folder (plan
//! docs/plaene/2026-11-28-ersetzen-masken-shell, part C), with the search
//! options of `se grep` (`grep::SearchOptions`) plus the replacement. The
//! search runs where the files are; every changed file is published whole
//! through the backend of the place, only if unchanged since it was read,
//! after its original went encrypted into the replace journal
//! (`vfs::content_replace`). On a terminal it asks first, as Notepad++ does
//! (folder, filter) and with the number of places and files, default No;
//! without a terminal it needs `--yes` or `--dry-run`. `--history`, `--undo`
//! and `--discard` work on the journal.
//!
//! Output: per changed file `rel: N` (with `--dry-run` the lines before and
//! after as `rel:line:-text` / `rel:line:+text`), skipped files with their
//! reason and the totals on standard error; `--json` gives one object per
//! line. Exit status: 0 with replacements, 1 without (or answered No), 2 on
//! an error, 130 after Ctrl+C; `--undo`: 0 when every file has its original
//! again, 1 when files stayed as they were, 2 on an error.
use super::grep::{clean, SearchOptions};
use super::target;
use crate::agent_proto::{ReplaceEvent, ReplaceRequest, ReplaceSelection, ReplaceSummary};
use crate::share::OperationOrigin;
use crate::vfs::{BackendHandle, ReplaceJournal};
use serde_json::json;
use std::io::{self, BufRead, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const JOURNAL_ACTIONS: [&str; 3] = ["history", "undo", "discard"];

#[derive(clap::Args)]
pub(super) struct ReplaceArgs {
    #[arg(
        required_unless_present_any = JOURNAL_ACTIONS,
        help = "Folder to replace below: local path, @label:/path, endpoint, share://…, gdrive:///…"
    )]
    target: Option<String>,
    #[arg(
        required_unless_present_any = JOURNAL_ACTIONS,
        help = "Text to find; with --regex a regular expression, with --extended Notepad++ escapes"
    )]
    pattern: Option<String>,
    #[arg(
        required_unless_present_any = JOURNAL_ACTIONS,
        allow_hyphen_values = true,
        help = "Replacement: literal; with --extended Notepad++ escapes; with --regex Notepad++'s syntax ($1, \\1, $+{name}, \\U…\\E, ?1yes:no)"
    )]
    replacement: Option<String>,
    #[command(flatten)]
    search: SearchOptions,
    #[arg(
        long = "dry-run",
        help = "Show the lines before and after, change nothing"
    )]
    dry_run: bool,
    #[arg(long, help = "Replace without asking")]
    yes: bool,
    #[arg(
        long,
        value_name = "ID",
        conflicts_with_all = ["history", "discard"],
        help = "Restore the files of an earlier replacement that are unchanged since"
    )]
    undo: Option<String>,
    #[arg(
        long,
        conflicts_with = "discard",
        help = "List earlier replacements, newest first"
    )]
    history: bool,
    #[arg(
        long,
        value_name = "ID",
        help = "Delete an earlier replacement with its encrypted originals"
    )]
    discard: Option<String>,
    #[arg(long, help = "One JSON object per line")]
    json: bool,
}

/// Where the command writes and reads, and the journal it uses.
struct Io<'a> {
    out: &'a mut dyn Write,
    err: &'a mut dyn Write,
    input: &'a mut dyn BufRead,
    /// Standard input and error are a terminal: the command asks first.
    terminal: bool,
    /// `None` = the user's journal.
    journal: Option<&'a ReplaceJournal>,
}

pub(super) fn run(args: ReplaceArgs) -> Result<i32, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    let interrupted = cancel.clone();
    // Ctrl+C ends the run between two files; written files stay written.
    let _ = ctrlc::set_handler(move || interrupted.store(true, Ordering::SeqCst));
    let terminal = io::stdin().is_terminal() && io::stderr().is_terminal();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut err = io::stderr();
    let mut io = Io {
        out: &mut out,
        err: &mut err,
        input: &mut input,
        terminal,
        journal: None,
    };
    let code = run_with(&args, &mut io, &cancel);
    let _ = out.flush();
    Ok(code)
}

fn run_with(args: &ReplaceArgs, io: &mut Io<'_>, cancel: &Arc<AtomicBool>) -> i32 {
    let result = if args.history {
        history(args, io)
    } else if let Some(id) = &args.discard {
        with_journal(io, |journal| journal.discard(id)).map(|()| {
            let _ = writeln!(io.err, "Ersetzungslauf {id} gelöscht");
            0
        })
    } else if let Some(id) = &args.undo {
        undo(args, id, io)
    } else {
        replace(args, io, cancel)
    };
    result.unwrap_or_else(|error| {
        let _ = writeln!(io.err, "se replace: {error}");
        2
    })
}

fn with_journal<T>(
    io: &Io<'_>,
    act: impl FnOnce(&ReplaceJournal) -> io::Result<T>,
) -> Result<T, String> {
    let result = match io.journal {
        Some(journal) => act(journal),
        None => ReplaceJournal::open_default().and_then(|journal| act(&journal)),
    };
    result.map_err(|error| error.to_string())
}

fn history(args: &ReplaceArgs, io: &mut Io<'_>) -> Result<i32, String> {
    for run in with_journal(io, ReplaceJournal::history)? {
        let line = if args.json {
            json!({
                "id": run.id,
                "created_ms": run.created_ms,
                "state": run.state.key(),
                "location": run.location,
                "root": run.root,
                "pattern": run.pattern,
                "replacement": run.replacement,
                "mode": run.mode.key(),
                "files_changed": run.files_changed,
                "replacements": run.replacements,
            })
            .to_string()
        } else {
            format!(
                "{}\t{}\t{} in {}\t„{}“ → „{}“\t{}",
                run.id,
                run.state.key(),
                counted(run.replacements, "Stelle", "Stellen"),
                counted(run.files_changed, "Datei", "Dateien"),
                clean(&run.pattern),
                clean(&run.replacement),
                run.location
            )
        };
        let _ = writeln!(io.out, "{line}");
    }
    Ok(0)
}

fn undo(args: &ReplaceArgs, id: &str, io: &mut Io<'_>) -> Result<i32, String> {
    // The targets stay open until the undo ends (a mapped share's connection).
    let opened = std::cell::RefCell::new(Vec::new());
    let resolve = |location: &str| -> io::Result<BackendHandle> {
        let target = target::resolve(location, OperationOrigin::User).map_err(io::Error::other)?;
        let backend = target.backend.clone();
        opened.borrow_mut().push(target);
        Ok(backend)
    };
    let undone = with_journal(io, |journal| journal.undo(id, &resolve))?;
    let restored = undone.restored.iter().map(|rel| (rel, None));
    let skipped = undone.skipped.iter().map(|(rel, why)| (rel, Some(why)));
    for (rel, why) in restored.chain(skipped) {
        let _ = match (args.json, why) {
            (true, None) => writeln!(io.out, "{}", json!({ "rel": rel, "restored": true })),
            (true, Some(why)) => writeln!(io.out, "{}", json!({ "rel": rel, "skipped": why })),
            (false, None) => writeln!(io.out, "{}: wiederhergestellt", clean(rel)),
            (false, Some(why)) => writeln!(io.err, "se replace: {}: {why}", clean(rel)),
        };
    }
    let _ = writeln!(
        io.err,
        "Rückgängig {id}: {} wiederhergestellt, {} unverändert gelassen",
        undone.restored.len(),
        undone.skipped.len()
    );
    Ok(if undone.skipped.is_empty() { 0 } else { 1 })
}

fn replace(args: &ReplaceArgs, io: &mut Io<'_>, cancel: &Arc<AtomicBool>) -> Result<i32, String> {
    let (Some(spec), Some(pattern), Some(replacement)) =
        (&args.target, &args.pattern, &args.replacement)
    else {
        return Err("Ziel, Suchtext und Ersatztext fehlen".to_string());
    };
    let query = args.search.query(pattern);
    crate::agent_proto::prepare_content_search(&query)
        .map_err(|error| format!("ungültige Suche: {error}"))?;
    let target = target::resolve(spec, OperationOrigin::User)?;
    let location = location_of(spec, &target);
    let mut request = ReplaceRequest {
        query,
        replacement: replacement.clone(),
        selection: ReplaceSelection::All,
        dry_run: true,
    };
    let place = (target.backend.clone(), target.path.clone(), location);
    if !args.dry_run && !args.yes {
        if !io.terminal {
            return Err("ohne Terminal braucht der Befehl --yes oder --dry-run".to_string());
        }
        let counts = execute(&place, &request, io.journal, cancel, &mut |_| {})
            .map_err(|error| error.to_string())?;
        if cancel.load(Ordering::SeqCst) {
            let _ = writeln!(io.err, "se replace: abgebrochen, nichts geändert");
            return Ok(130);
        }
        if counts.replacements == 0 {
            let _ = writeln!(io.err, "{}", totals(pattern, replacement, &counts, true));
            return Ok(1);
        }
        let _ = write!(
            io.err,
            "Sollen wirklich alle Vorkommen ersetzt werden in:\n  {spec}\nFür Dateityp: {}\n{} in {}. Ersetzen? [j/N] ",
            args.search.filter_text(),
            counted(counts.replacements, "Stelle", "Stellen"),
            counted(counts.files_changed, "Datei", "Dateien")
        );
        let _ = io.err.flush();
        let mut answer = String::new();
        let _ = io.input.read_line(&mut answer);
        if !matches!(
            answer.trim().to_lowercase().as_str(),
            "j" | "ja" | "y" | "yes"
        ) {
            let _ = writeln!(io.err, "se replace: nichts geändert");
            return Ok(1);
        }
    }
    request.dry_run = args.dry_run;
    let (json_output, out, err) = (args.json, &mut *io.out, &mut *io.err);
    let summary = execute(&place, &request, io.journal, cancel, &mut |event| {
        print_event(&event, json_output, out, err)
    })
    .map_err(|error| error.to_string())?;
    if args.json {
        let value = json!({
            "summary": true, "files_searched": summary.files_searched,
            "files_changed": summary.files_changed, "replacements": summary.replacements,
            "skipped": summary.skipped, "dry_run": args.dry_run,
            "canceled": summary.canceled, "undo_id": summary.undo_id,
        });
        let _ = writeln!(io.out, "{value}");
    }
    let _ = writeln!(
        io.err,
        "{}",
        totals(pattern, replacement, &summary, args.dry_run)
    );
    if let Some(id) = &summary.undo_id {
        let _ = writeln!(io.err, "Rückgängig: se replace --undo {id}");
    }
    Ok(match (summary.canceled, summary.replacements) {
        (true, _) => 130,
        (false, 0) => 1,
        _ => 0,
    })
}

/// The place in the journal: the target as written, a relative local path
/// made absolute (undo may run in another folder).
fn location_of(spec: &str, target: &target::Target) -> String {
    let path = std::path::Path::new(spec);
    let local = target.backend.is_local() && !spec.contains("://") && !spec.starts_with('@');
    if local && path.is_relative() {
        if let Ok(absolute) = std::path::absolute(path) {
            return absolute.to_string_lossy().into_owned();
        }
    }
    spec.to_string()
}

/// Runs the replacement on its own thread; its events go to `on_event` here.
fn execute(
    (backend, root, location): &(BackendHandle, String, String),
    request: &ReplaceRequest,
    journal: Option<&ReplaceJournal>,
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(ReplaceEvent),
) -> io::Result<ReplaceSummary> {
    let (sender, events) = crossbeam_channel::bounded::<ReplaceEvent>(1024);
    std::thread::scope(|scope| {
        let worker = scope.spawn(move || {
            let emit = |event| sender.send(event).is_ok();
            match journal {
                Some(journal) => crate::vfs::content_replace_with(
                    journal, &**backend, root, request, location, &emit, cancel,
                ),
                None => {
                    crate::vfs::content_replace(&**backend, root, request, location, &emit, cancel)
                }
            }
        });
        for event in events.iter() {
            on_event(event);
        }
        worker
            .join()
            .map_err(|_| io::Error::other("das Ersetzen ist abgestürzt"))?
    })
}

fn print_event(event: &ReplaceEvent, json: bool, out: &mut dyn Write, err: &mut dyn Write) {
    let value = match event {
        ReplaceEvent::Progress { .. } => return,
        ReplaceEvent::Skipped { rel, reason } if !json => {
            let _ = writeln!(err, "se replace: {}: {reason}", clean(rel));
            return;
        }
        ReplaceEvent::Skipped { rel, reason } => {
            json!({ "rel": rel, "skipped": reason.key(), "reason": reason.message() })
        }
        ReplaceEvent::File {
            rel, replacements, ..
        } if !json => {
            let _ = writeln!(out, "{}: {replacements}", clean(rel));
            return;
        }
        ReplaceEvent::File {
            rel,
            replacements,
            size_before: old,
            size_after: new,
        } => {
            json!({ "rel": rel, "replacements": replacements, "size_before": old, "size_after": new })
        }
        ReplaceEvent::Preview {
            rel,
            line,
            before,
            after,
            truncated: cut,
        } if json => {
            json!({ "rel": rel, "line": line, "before": before, "after": after, "truncated": cut })
        }
        ReplaceEvent::Preview {
            rel,
            line,
            before,
            after,
            ..
        } => {
            let (rel, before, after) = (clean(rel), clean(before), clean(after));
            let _ = writeln!(out, "{rel}:{line}:-{before}\n{rel}:{line}:+{after}");
            return;
        }
    };
    let _ = writeln!(out, "{value}");
}

/// Notepad++'s totals, worded as the other surfaces word them.
fn totals(pattern: &str, replacement: &str, summary: &ReplaceSummary, dry_run: bool) -> String {
    let verb = if dry_run { "zu ersetzen" } else { "ersetzt" };
    let mut line = format!(
        "Ersetzen „{}“ → „{}“: {} in {} {verb} ({} durchsucht",
        clean(pattern),
        clean(replacement),
        counted(summary.replacements, "Stelle", "Stellen"),
        counted(summary.files_changed, "Datei", "Dateien"),
        summary.files_searched
    );
    if summary.skipped > 0 {
        line.push_str(&format!("; {} übersprungen", summary.skipped));
    }
    if summary.canceled {
        line.push_str("; abgebrochen");
    }
    line.push(')');
    line
}

/// `1 Stelle`, `2 Stellen`.
fn counted(count: u64, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use super::{run_with, Io, ReplaceArgs};
    use crate::agent_proto::TextMode;
    use crate::vfs::ReplaceJournal;
    use clap::Parser;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn parse(args: &[&str]) -> Result<ReplaceArgs, String> {
        let argv = ["se", "replace"].iter().chain(args).copied();
        match super::super::Cli::try_parse_from(argv).map_err(|error| error.to_string())? {
            super::super::Cli {
                command: super::super::Command::Replace(args),
            } => Ok(args),
            _ => Err("not replace".into()),
        }
    }

    /// The arguments, the exit status, the question on a terminal (default
    /// No), `--yes`, the journal's history, undo and discard.
    #[test]
    fn replace_task_cli_arguments_exit_codes_question_and_journal() {
        let args = parse(&["/srv", "(a)", "-$1-", "--regex", "--gitignore", "--dry-run"]).unwrap();
        let query = args.search.query(args.pattern.as_deref().unwrap());
        assert_eq!(
            (query.mode, query.respect_ignore_files),
            (TextMode::Regex, true)
        );
        assert_eq!(args.replacement.as_deref(), Some("-$1-"));
        assert!(args.dry_run && !args.yes && parse(&["/srv", "a", ""]).is_ok());
        assert!(parse(&["--history"]).unwrap().history);
        assert!(parse(&["/srv", "a"]).is_err() && parse(&["--undo", "x", "--history"]).is_err());
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "Hallo Welt\n").unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        let journal = ReplaceJournal::open_at(dir.path().join(".journal"), [3u8; 32]);
        let call = |args: &[&str], terminal: bool, answer: &str| {
            let args = parse(args).unwrap();
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let mut input = answer.as_bytes();
            let mut io = Io {
                out: &mut out,
                err: &mut err,
                input: &mut input,
                terminal,
                journal: Some(&journal),
            };
            let code = run_with(&args, &mut io, &Arc::new(AtomicBool::new(false)));
            (
                code,
                String::from_utf8(out).unwrap(),
                String::from_utf8(err).unwrap(),
            )
        };
        let (code, _, err) = call(&[&root, "Welt", "Erde"], false, "");
        assert_eq!(code, 2);
        assert!(err.contains("--yes oder --dry-run"), "{err}");
        let (code, out, _) = call(&[&root, "Welt", "Erde", "--dry-run"], false, "");
        assert_eq!(
            (code, out.as_str()),
            (0, "a.txt:1:-Hallo Welt\na.txt:1:+Hallo Erde\na.txt: 1\n")
        );
        let (code, _, err) = call(&[&root, "Welt", "Erde"], true, "\n");
        assert_eq!(code, 1);
        assert!(
            err.contains("1 Stelle in 1 Datei. Ersetzen? [j/N]") && err.contains("*.*"),
            "{err}"
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "Hallo Welt\n");
        let (code, out, err) = call(&[&root, "Welt", "Erde"], true, "j\n");
        assert_eq!((code, out.as_str()), (0, "a.txt: 1\n"), "{err}");
        assert!(err.contains("Rückgängig: se replace --undo "), "{err}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "Hallo Erde\n");
        assert_eq!(call(&[&root, "Welt", "Erde", "--yes"], false, "").0, 1);
        assert_eq!(call(&[&root, "(", "x", "--regex", "--yes"], false, "").0, 2);
        let (code, out, _) = call(&["--history", "--json"], false, "");
        assert_eq!(code, 0);
        let run: serde_json::Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
        assert_eq!(
            (run["files_changed"].as_u64(), run["state"].as_str()),
            (Some(1), Some("applied"))
        );
        let id = run["id"].as_str().unwrap().to_string();
        let (code, out, err) = call(&["--undo", &id], false, "");
        assert_eq!(
            (code, out.as_str()),
            (0, "a.txt: wiederhergestellt\n"),
            "{err}"
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "Hallo Welt\n");
        assert_eq!(call(&["--discard", &id], false, "").0, 0);
        assert_eq!(call(&["--history"], false, "").1, "");
        assert_eq!(call(&["--undo", &id], false, "").0, 2);
    }
}
