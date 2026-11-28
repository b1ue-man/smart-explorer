//! `se grep`: content search below a
//! folder (plan docs/plaene/2026-11-27-dateien-suche-ansicht section B), as
//! Notepad++'s "Find in Files" with grep's output. The search runs where the
//! files are (`vfs::content_search`: locally, in the SSH agent, on the Share
//! host, otherwise by reading through the backend). Output: `rel:line:text`
//! per hit (context lines `rel-line-text`, groups split by `--`), the file
//! names only, or `rel:count`; `--json` gives one JSON object per line
//! (`mcp/grep_records.rs`, shared with the MCP `grep` tool). The totals go to standard error as Notepad++ shows them. Exit status as grep:
//! 0 with hits, 1 without, 2 on an error (130 after Ctrl+C).
use super::mcp::grep_records::{run_search, totals_line, GrepOutput, Records};
use super::target;
use crate::agent_proto::{ContentEvent, ContentQuery, ContentSummary};
use crate::agent_proto::{SkipReason, TextMode};
use crate::share::OperationOrigin;
use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(clap::Args)]
pub(super) struct GrepArgs {
    #[arg(
        help = "Folder to search below: local path, @label:/path, endpoint, share://…, gdrive:///…"
    )]
    target: String,
    #[arg(
        help = "Text to find; with --regex a regular expression, with --extended Notepad++ escapes (\\n, \\t, \\x41, …)"
    )]
    pattern: String,
    #[arg(
        long,
        conflicts_with = "extended",
        help = "The pattern is a regular expression (Notepad++ \"Regular expression\")"
    )]
    regex: bool,
    #[arg(long, help = "The pattern holds Notepad++ escapes (\"Extended\")")]
    extended: bool,
    #[arg(long, help = "Match case")]
    case: bool,
    #[arg(long, help = "Match whole words only (not with --regex)")]
    word: bool,
    #[arg(
        long,
        requires = "regex",
        help = "`.` also matches line ends (\". matches newline\")"
    )]
    dotall: bool,
    #[arg(
        long,
        value_name = "FILTERS",
        default_value = "",
        help = "Notepad++ filters: `*.rs *.toml`, `!*.min.js` leaves files out, `!\\target` a first-level folder, `!+\\node_modules` folders on every level"
    )]
    filter: String,
    #[arg(
        long,
        value_name = "REGEX",
        help = "Search only in folders whose path relative to the target matches"
    )]
    folders: Option<String>,
    #[arg(
        long = "exclude-folders",
        value_name = "REGEX",
        help = "Leave out folders whose relative path matches, with everything below"
    )]
    exclude_folders: Option<String>,
    #[arg(long = "no-subfolders", help = "Only the files directly in the target")]
    no_subfolders: bool,
    #[arg(long, help = "Also enter hidden folders")]
    hidden: bool,
    #[arg(
        short = 'C',
        long = "context",
        value_name = "N",
        default_value_t = 0,
        help = "Lines before and after each hit"
    )]
    context: u32,
    #[arg(
        long = "files-only",
        conflicts_with = "count",
        help = "Print only the names of files with a hit"
    )]
    files_only: bool,
    #[arg(long, help = "Print the number of hit lines per file")]
    count: bool,
    #[arg(
        long = "max-results",
        value_name = "N",
        default_value_t = 0,
        help = "Stop after N hit lines; 0 = all"
    )]
    max_results: u64,
    #[arg(long, help = "One JSON object per line (rel, line, text, ranges)")]
    json: bool,
    #[arg(long, value_enum, default_value_t = ColorChoice::Auto, help = "Highlight hits: auto (on a terminal), always, never")]
    color: ColorChoice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum ColorChoice {
    Auto,
    Always,
    Never,
}

impl GrepArgs {
    fn output(&self) -> GrepOutput {
        if self.files_only {
            GrepOutput::Files
        } else if self.count {
            GrepOutput::Count
        } else {
            GrepOutput::Content
        }
    }

    fn query(&self) -> ContentQuery {
        let output = self.output();
        let mut query = ContentQuery::text(self.pattern.clone());
        query.mode = if self.regex {
            TextMode::Regex
        } else if self.extended {
            TextMode::Extended
        } else {
            TextMode::Normal
        };
        query.match_case = self.case;
        query.whole_word = self.word;
        query.dot_matches_newline = self.dotall;
        query.filters = self.filter.clone();
        query.folder_include = self.folders.clone().filter(|text| !text.is_empty());
        query.folder_exclude = self.exclude_folders.clone().filter(|text| !text.is_empty());
        query.subfolders = !self.no_subfolders;
        query.hidden = self.hidden;
        query.context_lines = if output == GrepOutput::Content {
            self.context
        } else {
            0
        };
        query.max_hits = self.max_results;
        query.files_only = output == GrepOutput::Files;
        query
    }
}

pub(super) fn run(args: GrepArgs) -> Result<i32, String> {
    let color = match args.color {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => io::stdout().is_terminal() && ansi_terminal(),
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let interrupted = cancel.clone();
    // Ctrl+C ends the search and still prints the totals.
    let _ = ctrlc::set_handler(move || interrupted.store(true, Ordering::SeqCst));
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    let code = run_into(&args, &mut out, &mut io::stderr(), color, &cancel);
    let _ = out.flush();
    Ok(code)
}

/// Whether the terminal shows ANSI colors: `NO_COLOR` unset, and a terminal
/// that announces itself (`TERM` other than `dumb`, Windows Terminal, ConEmu,
/// an editor's terminal). A Windows console host without them shows escapes
/// as text and gets none.
fn ansi_terminal() -> bool {
    let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    if set("NO_COLOR") {
        return false;
    }
    match std::env::var("TERM") {
        Ok(term) if term == "dumb" => false,
        Ok(term) if !term.is_empty() => true,
        _ => {
            set("WT_SESSION")
                || set("TERM_PROGRAM")
                || std::env::var("ConEmuANSI").is_ok_and(|value| value == "ON")
        }
    }
}

/// The search of `args` with its output in `out` and messages in `err`; the
/// exit status.
fn run_into(
    args: &GrepArgs,
    out: &mut dyn Write,
    err: &mut dyn Write,
    color: bool,
    cancel: &Arc<AtomicBool>,
) -> i32 {
    match search(args, out, err, color, cancel) {
        Ok(summary) => {
            let _ = writeln!(err, "{}", totals_line(&args.pattern, &summary));
            if cancel.load(Ordering::SeqCst) {
                let _ = writeln!(err, "se grep: abgebrochen");
                130
            } else if summary.files_matched > 0 || summary.hits > 0 {
                0
            } else {
                1
            }
        }
        Err(error) => {
            let _ = writeln!(err, "se grep: {error}");
            2
        }
    }
}

fn search(
    args: &GrepArgs,
    out: &mut dyn Write,
    err: &mut dyn Write,
    color: bool,
    cancel: &Arc<AtomicBool>,
) -> Result<ContentSummary, String> {
    let query = args.query();
    // Pattern, filters and folder regexes, before anything is opened.
    crate::agent_proto::prepare_content_search(&query)
        .map_err(|error| format!("ungültige Suche: {error}"))?;
    let target = target::resolve(&args.target, OperationOrigin::User)?;
    let mut records = Records::new(args.output(), args.json, color, args.context > 0);
    let mut buffer = Vec::new();
    let mut write_failed = false;
    let summary = run_search(
        target.backend.clone(),
        target.path.clone(),
        query,
        cancel.clone(),
        &mut |event| match event {
            ContentEvent::Hits {
                rel, lines, last, ..
            } => {
                if write_failed {
                    return;
                }
                buffer.clear();
                records.hits(&rel, &lines, last, &mut buffer);
                if out.write_all(&buffer).is_err() {
                    // A closed pipe (`| head`) ends the search.
                    write_failed = true;
                    cancel.store(true, Ordering::SeqCst);
                }
            }
            ContentEvent::Skipped { rel, reason } => match reason {
                SkipReason::Binary => {}
                SkipReason::Unreadable(detail) | SkipReason::UnlistableFolder(detail) => {
                    let _ = writeln!(err, "se grep: {rel}: {detail}");
                }
            },
            ContentEvent::Progress { .. } => {}
        },
    )
    .map_err(|error| error.to_string())?;
    let _ = out.flush();
    if write_failed {
        // grep ends quietly when its reader went away.
        cancel.store(false, Ordering::SeqCst);
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::{run_into, GrepArgs, GrepOutput};
    use crate::agent_proto::TextMode;
    use clap::Parser;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn parse(args: &[&str]) -> Result<GrepArgs, String> {
        let argv = ["se", "grep"].iter().chain(args).copied();
        match super::super::Cli::try_parse_from(argv).map_err(|error| error.to_string())? {
            super::super::Cli {
                command: super::super::Command::Grep(args),
            } => Ok(args),
            _ => Err("not grep".into()),
        }
    }

    #[test]
    fn grep_task_cli_arguments_build_the_query() {
        let args = parse(&[
            "/srv",
            "fo+",
            "--regex",
            "--case",
            "--dotall",
            "-C",
            "2",
            "--filter",
            "*.rs !*.min.rs",
            "--folders",
            "^src",
            "--exclude-folders",
            "target",
            "--no-subfolders",
            "--hidden",
            "--max-results",
            "5",
        ])
        .unwrap();
        let query = args.query();
        assert_eq!(query.pattern, "fo+");
        assert_eq!(query.mode, TextMode::Regex);
        assert!(query.match_case && query.dot_matches_newline && query.hidden);
        assert!(!query.whole_word && !query.subfolders && !query.files_only);
        assert_eq!(query.filters, "*.rs !*.min.rs");
        assert_eq!(query.folder_include.as_deref(), Some("^src"));
        assert_eq!(query.folder_exclude.as_deref(), Some("target"));
        assert_eq!((query.context_lines, query.max_hits), (2, 5));
        let plain = parse(&["/srv", "a b", "--word", "--files-only", "-C", "3"]).unwrap();
        assert_eq!(plain.output(), GrepOutput::Files);
        let query = plain.query();
        assert_eq!(query.mode, TextMode::Normal);
        assert!(query.files_only && query.whole_word && query.subfolders);
        // Context lines belong to the hit lines only.
        assert_eq!(query.context_lines, 0);
        assert_eq!(
            parse(&["/srv", "x", "--count"]).unwrap().output(),
            GrepOutput::Count
        );
        assert_eq!(
            parse(&["/srv", "\\t", "--extended"]).unwrap().query().mode,
            TextMode::Extended
        );
        for wrong in [
            &["/srv", "x", "--regex", "--extended"][..],
            &["/srv", "x", "--files-only", "--count"],
            &["/srv", "x", "--dotall"],
            &["/srv"],
        ] {
            assert!(parse(wrong).is_err(), "{wrong:?}");
        }
    }

    /// Exit status as grep: 0 with hits, 1 without, 2 on an error.
    #[test]
    fn grep_task_cli_exit_codes_follow_grep() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "Hallo Welt\nzweite Zeile\n").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub").join("b.rs"), "fn main() {}\n").unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        let run = |args: &[&str]| {
            let mut argv = vec![root.as_str()];
            argv.extend_from_slice(args);
            let args = parse(&argv).unwrap();
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let cancel = Arc::new(AtomicBool::new(false));
            let code = run_into(&args, &mut out, &mut err, false, &cancel);
            (
                code,
                String::from_utf8(out).unwrap(),
                String::from_utf8(err).unwrap(),
            )
        };
        let (code, out, err) = run(&["welt"]);
        assert_eq!((code, out.as_str()), (0, "a.txt:1:Hallo Welt\n"), "{err}");
        assert!(
            err.contains("Suche „welt“ (1 Treffer in 1 Datei von 2 durchsuchten"),
            "{err}"
        );
        assert_eq!(run(&["main", "--files-only"]).1, "sub/b.rs\n");
        assert_eq!(run(&["e", "--count", "--filter", "*.txt"]).1, "a.txt:2\n");
        assert_eq!(run(&["fehlt"]).0, 1);
        let (code, _, err) = run(&["(", "--regex"]);
        assert_eq!(code, 2);
        assert!(err.contains("ungültige Suche"), "{err}");
        assert_eq!(run(&["x", "--folders", "("]).0, 2);
        let missing = dir.path().join("fehlt").to_string_lossy().into_owned();
        let args = parse(&[missing.as_str(), "x"]).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        assert_eq!(
            run_into(&args, &mut Vec::new(), &mut Vec::new(), false, &cancel),
            2
        );
    }
}
