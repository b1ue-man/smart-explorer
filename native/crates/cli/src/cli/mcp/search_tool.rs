//! `search` as a job: the `se search` walk (server-side where available)
//! streams hits as JSON lines into the job's `hits` output. Links and special
//! files are hits of their own type (`symlink`, `special`) and never entered.
//! A glob with `/` or `**` matches the relative path, `gitignore` applies the
//! ignore files, `sort` orders by path or modification time (plan
//! docs/plaene/2026-11-28-ersetzen-masken-shell, part C). With an active
//! mask list `path` and `relative` of every hit are masked before they reach
//! the stream (a `PreMasked` stream, contract 4).
use super::args::Args;
use super::jobs::{spawn_worker, Job, JobControl, JobEnd, JobKind, JobStatus};
use super::session::Session;
use crate::cli::search::{SearchOptions, SearchSort};
use crate::masks::MaskSet;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

pub(super) const DESCRIPTION: &str = "Finds files, links and special files (plus directories with `directories: true`) below `target` (default: the working root) whose name contains `query` (case-insensitive) or matches it as a glob (`*`, `?`). A glob with `/` or `**` matches the path relative to `target` instead (`src/**/*.rs`, `**/*.{ts,tsx}`, `[a-z]`; `*` stays within one folder level). `gitignore: true` leaves out what `.gitignore` (inside a Git work tree), `.ignore` and `.git/info/exclude` exclude, as ripgrep does. `sort` is `name` (relative path) or `mtime` (newest first); `max_results` then counts after sorting. Hits are JSON lines with `path`, `relative`, `type`, `size`, `mtime_ms`. Links are reported with type `symlink` and never entered. Uses the server-side search of Share peers and SSH agents (those skip links), otherwise walks the tree. Hits page via `hits_offset`. Use it instead of Glob or `find` for names; `grep` searches file contents.";

pub(super) fn properties(target: &str, wait: &str, max_bytes: &str) -> Value {
    json!({
        "target": {"type": "string", "description": target},
        "query": {"type": "string", "description": "Text or glob; empty matches everything."},
        "glob": {"type": "boolean", "description": "Treat `query` as a glob (with `/` or `**`: a path glob)."},
        "directories": {"type": "boolean", "description": "Also return matching directories."},
        "gitignore": {"type": "boolean", "description": "Leave out what .gitignore, .ignore and .git/info/exclude exclude (default false)."},
        "sort": {"type": "string", "enum": ["none", "name", "mtime"], "description": "none (default, as found), name (relative path) or mtime (newest first)."},
        "max_results": {"type": "integer", "minimum": 0, "description": "Stop after this many hits; 0 or absent = all."},
        "wait_seconds": {"type": "integer", "minimum": 0, "description": wait},
        "max_bytes": {"type": "integer", "minimum": 1024, "description": max_bytes}
    })
}

struct SearchControl {
    cancel: Arc<AtomicBool>,
    hits: Arc<AtomicU64>,
}

impl JobControl for SearchControl {
    fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    fn progress(&self) -> Value {
        json!({ "hits": self.hits.load(Ordering::SeqCst) })
    }
}

/// The options of a call; an unknown `sort` or an invalid glob fails here.
pub(super) fn options(args: &Args<'_>) -> Result<SearchOptions, String> {
    let sort = match args.opt_string("sort")? {
        None => SearchSort::Found,
        Some(text) => SearchSort::parse(&text)
            .ok_or_else(|| format!("unknown sort `{text}`; use none, name or mtime"))?,
    };
    let options = SearchOptions {
        query: args.opt_string("query")?.unwrap_or_default(),
        glob: args.flag("glob")?,
        max_results: args.opt_u64("max_results")?.unwrap_or(0),
        want_dirs: args.flag("directories")?,
        sort,
        gitignore: args.flag("gitignore")?,
    };
    crate::cli::search::check_options(&options)?;
    Ok(options)
}

/// One hit as its JSON line, `path` and `relative` masked with `masks`.
fn hit_line(root: &str, hit: &crate::cli::search::Found, masks: &MaskSet) -> Vec<u8> {
    let mut value = crate::cli::search::hit_json(root, hit);
    if !masks.is_empty() {
        for key in ["path", "relative"] {
            if let Some(Value::String(text)) = value.get_mut(key) {
                *text = masks.mask_path(text);
            }
        }
    }
    let mut line = value.to_string().into_bytes();
    line.push(b'\n');
    line
}

pub(super) fn start(session: &Session, args: &Args<'_>) -> Result<Arc<Job>, String> {
    let target_spec = super::file_tools::target_spec(args)?;
    let options = options(args)?;
    let target = session.targets.resolve(&target_spec)?;
    let masks = session.masks.set()?;
    super::mask_layer_guard::guard(&masks, &target, super::mask_layer_guard::Reach::Tree)?;
    let targets = session.targets.clone();
    let job = session.jobs.create(
        JobKind::Search,
        format!("search {target_spec} for {:?}", options.query),
        &["hits"],
    )?;
    let cancel = Arc::new(AtomicBool::new(false));
    let hits = Arc::new(AtomicU64::new(0));
    job.set_control(Arc::new(SearchControl {
        cancel: cancel.clone(),
        hits: hits.clone(),
    }));
    spawn_worker(job.clone(), move |job| {
        let mut write_error = None;
        let outcome = crate::cli::search::search_found(&target, &options, &cancel, &mut |hit| {
            if write_error.is_some() {
                return;
            }
            match job.append("hits", &hit_line(&target.path, &hit, &masks)) {
                Ok(()) => {
                    hits.fetch_add(1, Ordering::SeqCst);
                }
                Err(error) => {
                    write_error = Some(error);
                    cancel.store(true, Ordering::SeqCst);
                }
            }
        });
        let mut result = json!({
            "root": target.path,
            "hits_total": hits.load(Ordering::SeqCst),
        });
        if let Value::Object(fields) = &mut result {
            if options.sort != SearchSort::Found {
                fields.insert("sort".into(), json!(options.sort.key()));
            }
            if options.gitignore {
                fields.insert("gitignore".into(), json!(true));
            }
        }
        match (outcome, write_error) {
            (_, Some(error)) => JobEnd {
                status: JobStatus::Failed,
                result,
                error: Some(error),
            },
            (Ok(()), None) => JobEnd {
                status: JobStatus::Completed,
                result,
                error: None,
            },
            (Err(error), None) => {
                let cancelled = job.cancel_requested();
                if !cancelled {
                    targets.after_failure(&target);
                }
                JobEnd {
                    status: if cancelled {
                        JobStatus::Cancelled
                    } else {
                        JobStatus::Failed
                    },
                    result,
                    error: Some(error),
                }
            }
        }
    })?;
    Ok(job)
}

#[cfg(test)]
mod tests {
    use super::options;
    use crate::cli::mcp::args::Args;
    use crate::cli::search::SearchSort;
    use serde_json::json;

    #[test]
    fn mcp_coding_task_search_options_are_checked_before_a_job_starts() {
        let parse = |value: serde_json::Value| {
            let map = value.as_object().cloned().unwrap_or_default();
            options(&Args::new(Some(&map)))
        };
        let full = parse(json!({"query": "src/**/*.rs", "glob": true, "sort": "mtime", "gitignore": true, "max_results": 3})).unwrap();
        assert_eq!(full.sort, SearchSort::Mtime);
        assert!(full.gitignore && full.glob && full.max_results == 3);
        let plain = parse(json!({"query": "x"})).unwrap();
        assert_eq!(plain.sort, SearchSort::Found);
        assert!(!plain.gitignore);
        assert!(parse(json!({"query": "x", "sort": "size"})).is_err());
        assert!(parse(json!({"query": "src/[a", "glob": true})).is_err());
        // Without `glob` the text is a name part, not a glob: never refused.
        assert!(parse(json!({"query": "src/[a"})).is_ok());
    }

    #[test]
    fn mcp_coding_task_search_path_globs_ignore_files_and_order() {
        use crate::cli::search::{search_found, SearchOptions};
        use std::time::{Duration, SystemTime};
        let dir = tempfile::tempdir().unwrap();
        let write = |rel: &str, age: u64| {
            let path = dir.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, rel).unwrap();
            let file = std::fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(SystemTime::now() - Duration::from_secs(age))
                .unwrap();
        };
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        write(".gitignore", 50);
        std::fs::write(dir.path().join(".gitignore"), "target/\n*.log\n").unwrap();
        write("src/main.rs", 30);
        write("src/a/lib.rs", 10);
        write("target/x.rs", 5);
        write("notes.log", 1);
        write("README.md", 40);
        let targets = crate::cli::mcp::targets::Targets::default();
        let target = targets.resolve(&dir.path().to_string_lossy()).unwrap();
        let found = |options: SearchOptions| {
            let mut rels = Vec::new();
            let cancel = std::sync::atomic::AtomicBool::new(false);
            search_found(&target, &options, &cancel, &mut |hit| rels.push(hit.rel)).unwrap();
            rels
        };
        let glob = |query: &str| SearchOptions {
            query: query.into(),
            glob: true,
            ..Default::default()
        };
        let mut rs = found(glob("src/**/*.rs"));
        rs.sort();
        assert_eq!(rs, ["src/a/lib.rs", "src/main.rs"]);
        let mut all = found(SearchOptions {
            gitignore: true,
            sort: SearchSort::Name,
            ..glob("**/*")
        });
        all.retain(|rel| !rel.starts_with(".git/"));
        assert_eq!(
            all,
            [".gitignore", "README.md", "src/a/lib.rs", "src/main.rs"]
        );
        assert_eq!(
            found(SearchOptions {
                sort: SearchSort::Mtime,
                ..glob("*.rs")
            }),
            ["target/x.rs", "src/a/lib.rs", "src/main.rs"],
            "newest first"
        );
        assert_eq!(
            found(SearchOptions {
                sort: SearchSort::Mtime,
                gitignore: true,
                max_results: 1,
                ..glob("*.rs")
            }),
            ["src/a/lib.rs"],
            "the limit counts after sorting and ignoring"
        );
        assert_eq!(
            found(SearchOptions {
                max_results: 1,
                ..glob("**/*.md")
            })
            .len(),
            1
        );
    }
}
