//! `se analyze`: the storage analysis of a target, as in the explorer's
//! „Speicher-Analyse“ (2026-11-18). A local folder is walked here; a remote
//! one is analysed by the device that holds it (`analytics::scan_remote`:
//! its own worker first). Every entry carries `size` (the sum of its file
//! lengths) and `allocated` (the space it occupies on the volume: less for
//! cloud placeholders, compressed and sparse files, a hard-linked file once).
//! `--sort allocated` (default where the result knows the occupied space)
//! or `--sort size` orders the entries; without known occupied space the
//! order is by size and the output says `allocated: unknown`. The MCP server
//! reaches it through its `se` tool (`["analyze", <target>, "--json"]`).
use super::target::Target;
use crate::analytics::{ScanOutcome, ScanStatus, SizeNode};
use crate::format::format_bytes;
use crate::share::OperationOrigin;
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(super) enum SortBy {
    /// Occupied space on the volume.
    Allocated,
    /// Sum of the file lengths.
    Size,
}

impl SortBy {
    /// What applies to a result: `size` while the occupied space is unknown.
    pub(super) fn effective(requested: Option<Self>, allocated_known: bool) -> Self {
        match requested {
            Some(Self::Allocated) | None if allocated_known => Self::Allocated,
            _ => Self::Size,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Allocated => "allocated",
            Self::Size => "size",
        }
    }

    fn of(self, node: &SizeNode) -> u64 {
        match self {
            Self::Allocated => node.allocated,
            Self::Size => node.size,
        }
    }
}

#[derive(clap::Args)]
pub(super) struct AnalyzeArgs {
    #[arg(help = "Folder to analyse: local path, endpoint, @label:/path or share:// target")]
    pub(super) target: String,
    #[arg(
        long,
        value_enum,
        help = "Order by occupied space (allocated, the default where the source reports it) or by size"
    )]
    pub(super) sort: Option<SortBy>,
    #[arg(
        long,
        default_value_t = 20,
        help = "Largest entries shown per folder; 0 = all"
    )]
    pub(super) top: usize,
    #[arg(
        long,
        default_value_t = 1,
        help = "Folder levels below the target that are listed"
    )]
    pub(super) depth: usize,
    #[arg(
        long,
        value_name = "N",
        help = "Also list the N largest files of the whole tree (\"Größte Dateien\")"
    )]
    pub(super) files: Option<usize>,
    #[arg(
        long,
        help = "Also total size and allocated per file extension (\"Nach Dateityp\")"
    )]
    pub(super) types: bool,
    #[arg(long, help = "Print machine-readable JSON")]
    pub(super) json: bool,
}

pub(super) fn run(args: AnalyzeArgs) -> Result<i32, String> {
    let target = super::target::resolve(&args.target, OperationOrigin::User)?;
    let progress = crate::analytics::Progress::default();
    let outcome = scan(&target, &progress);
    match outcome.status {
        ScanStatus::Canceled => return Err("analysis cancelled".to_string()),
        ScanStatus::Failed => {
            let detail = outcome
                .issues
                .first()
                .map(|issue| format!("{}: {}", issue.path, issue.detail))
                .unwrap_or_else(|| "analysis failed".to_string());
            return Err(detail);
        }
        ScanStatus::Complete | ScanStatus::Partial => {}
    }
    let sort = SortBy::effective(args.sort, outcome.allocated_known);
    let snapshot = progress.snapshot();
    let mut report = report_json(&outcome, sort, args.top, args.depth);
    if let Some(tree) = outcome.tree.as_ref() {
        add_overview(&mut report, tree, sort, args.files, args.types);
    }
    report["target"] = json!(args.target);
    report["files"] = json!(snapshot.files);
    report["dirs"] = json!(snapshot.dirs);
    if args.json {
        let text = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
        println!("{text}");
    } else {
        print!("{}", report_text(&report));
        for issue in &outcome.issues {
            eprintln!("se: not readable: {}: {}", issue.path, issue.detail);
        }
        if outcome.suppressed_issues > 0 {
            eprintln!("se: {} more paths not readable", outcome.suppressed_issues);
        }
    }
    Ok(0)
}

fn scan(target: &Target, progress: &crate::analytics::Progress) -> ScanOutcome {
    if target.backend.is_local() {
        let native = target.path.replace('/', std::path::MAIN_SEPARATOR_STR);
        crate::analytics::scan(std::path::Path::new(&native), progress)
    } else {
        crate::analytics::scan_remote(&*target.backend, &target.path, progress)
    }
}

/// The result as JSON: totals, `sort`, `allocated_known`, read problems and
/// the largest `top` entries (0 = all) of every folder down to `depth`.
pub(super) fn report_json(outcome: &ScanOutcome, sort: SortBy, top: usize, depth: usize) -> Value {
    let status = match outcome.status {
        ScanStatus::Complete => "complete",
        ScanStatus::Partial => "partial",
        ScanStatus::Failed => "failed",
        ScanStatus::Canceled => "canceled",
    };
    let issues: Vec<Value> = outcome
        .issues
        .iter()
        .map(|issue| json!({ "path": issue.path, "detail": issue.detail }))
        .collect();
    let tree = outcome.tree.as_ref();
    json!({
        "status": status,
        "sort": sort.key(),
        "allocated_known": outcome.allocated_known,
        "size": tree.map_or(0, |tree| tree.size),
        "allocated": tree.map_or(0, |tree| tree.allocated),
        "issues": issues,
        "suppressed_issues": outcome.suppressed_issues,
        "notes": outcome.notes,
        "entries": tree.map_or_else(Vec::new, |tree| entries(tree, sort, top, depth)),
    })
}

fn entries(node: &SizeNode, sort: SortBy, top: usize, depth: usize) -> Vec<Value> {
    if depth == 0 {
        return Vec::new();
    }
    let mut children: Vec<&SizeNode> = node.children.iter().collect();
    children.sort_by(|left, right| {
        sort.of(right)
            .cmp(&sort.of(left))
            .then_with(|| left.name.cmp(&right.name))
    });
    if top > 0 {
        children.truncate(top);
    }
    children
        .into_iter()
        .map(|child| {
            let mut entry = json!({
                "name": &*child.name,
                "type": if child.is_dir { "dir" } else { "file" },
                "size": child.size,
                "allocated": child.allocated,
            });
            if child.is_dir && depth > 1 {
                entry["entries"] = Value::Array(entries(child, sort, top, depth - 1));
            }
            if is_system_files(&child.name, child.is_dir) {
                entry["system_files"] = json!(true);
            }
            entry
        })
        .collect()
}

/// „Größte Dateien“ (`largest_files`, `files` > 0) and „Nach Dateityp“
/// (`types`) of the whole tree; files folded into „… N weitere Eintraege“
/// count only in `aggregated` (`aggregated_note` says so).
pub(super) fn add_overview(
    report: &mut Value,
    tree: &SizeNode,
    sort: SortBy,
    files: Option<usize>,
    types: bool,
) {
    if files.is_none() && !types {
        return;
    }
    let top = files.unwrap_or(0);
    let view = crate::analytics_overview::overview(tree, sort == SortBy::Allocated, top);
    if files.is_some() {
        report["largest_files"] = Value::Array(
            view.largest
                .iter()
                .map(|file| {
                    let mut row = json!({
                        "path": file.path,
                        "size": file.size,
                        "allocated": file.allocated,
                    });
                    if is_system_files(&file.path, false) {
                        row["system_files"] = json!(true);
                    }
                    row
                })
                .collect(),
        );
    }
    if types {
        report["types"] = Value::Array(
            view.types
                .iter()
                .map(|total| {
                    json!({
                        "extension": total.extension,
                        "files": total.files,
                        "size": total.size,
                        "allocated": total.allocated,
                    })
                })
                .collect(),
        );
    }
    report["aggregated"] = json!({
        "files": view.aggregated.files,
        "size": view.aggregated.size,
        "allocated": view.aggregated.allocated,
    });
    report["aggregated_note"] = json!(crate::analytics_overview::aggregated_note(&view.aggregated));
}

/// „NTFS-Systemdateien“ of the fast NTFS way: space the file system holds,
/// nothing to open or delete.
fn is_system_files(name: &str, is_dir: bool) -> bool {
    !is_dir && name == crate::analytics::SYSTEM_FILES_NAME
}

fn system_suffix(row: &Value) -> &'static str {
    if row["system_files"] == true {
        "  (vom Dateisystem belegt, nicht löschbar)"
    } else {
        ""
    }
}

/// `Size  Allocated  Path` lines of [`report_json`]: the total first, then
/// the entries (folders end in `/`), `unknown` without occupied space.
pub(super) fn report_text(report: &Value) -> String {
    let known = report["allocated_known"].as_bool().unwrap_or(false);
    let allocated = |value: &Value| {
        if known {
            format_bytes(value.as_u64().unwrap_or(0))
        } else {
            "unknown".to_string()
        }
    };
    let mut text = format!("{:>10}  {:>10}  Path\n", "Size", "Allocated");
    text.push_str(&format!(
        "{:>10}  {:>10}  (total)\n",
        format_bytes(report["size"].as_u64().unwrap_or(0)),
        allocated(&report["allocated"])
    ));
    let mut stack: Vec<(String, &Value)> = Vec::new();
    if let Some(rows) = report["entries"].as_array() {
        stack.extend(rows.iter().rev().map(|row| (String::new(), row)));
    }
    while let Some((prefix, row)) = stack.pop() {
        let dir = row["type"] == "dir";
        let name = format!(
            "{prefix}{}{}",
            row["name"].as_str().unwrap_or_default(),
            if dir { "/" } else { "" }
        );
        text.push_str(&format!(
            "{:>10}  {:>10}  {name}{}\n",
            format_bytes(row["size"].as_u64().unwrap_or(0)),
            allocated(&row["allocated"]),
            system_suffix(row)
        ));
        if let Some(rows) = row["entries"].as_array() {
            stack.extend(rows.iter().rev().map(|child| (name.clone(), child)));
        }
    }
    if let Some(files) = report["largest_files"].as_array() {
        text.push_str(&format!(
            "\nLargest files\n{:>10}  {:>10}  Path\n",
            "Size", "Allocated"
        ));
        for file in files {
            text.push_str(&format!(
                "{:>10}  {:>10}  {}{}\n",
                format_bytes(file["size"].as_u64().unwrap_or(0)),
                allocated(&file["allocated"]),
                file["path"].as_str().unwrap_or_default(),
                system_suffix(file)
            ));
        }
    }
    if let Some(types) = report["types"].as_array() {
        text.push_str(&format!(
            "\nBy type\n{:>10}  {:>10}  {:>8}  Extension\n",
            "Size", "Allocated", "Files"
        ));
        for total in types {
            text.push_str(&format!(
                "{:>10}  {:>10}  {:>8}  {}\n",
                format_bytes(total["size"].as_u64().unwrap_or(0)),
                allocated(&total["allocated"]),
                total["files"].as_u64().unwrap_or(0),
                total["extension"].as_str().unwrap_or_default()
            ));
        }
    }
    // How the result was made and what it did not count.
    for note in report["notes"].as_array().into_iter().flatten() {
        text.push_str(note.as_str().unwrap_or_default());
        text.push('\n');
    }
    if let Some(note) = report["aggregated_note"].as_str() {
        text.push_str(note);
        text.push('\n');
    }
    let sort = report["sort"].as_str().unwrap_or("size");
    text.push_str(&format!("Sorted by {sort}."));
    if !known {
        text.push_str(" The source does not report the occupied space (allocated: unknown).");
    }
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, size: u64, allocated: u64, children: Vec<SizeNode>) -> SizeNode {
        SizeNode {
            name: name.into(),
            size,
            allocated,
            is_dir: !children.is_empty() || name.ends_with("dir"),
            children,
        }
    }

    #[test]
    fn belegt_task_cli_json_carries_both_values_and_sorts_by_allocated() {
        let tree = node(
            "Daten",
            7 << 30,
            (2 << 30) + (100 << 20),
            vec![
                node(
                    "geloescht",
                    5 << 30,
                    100 << 20,
                    vec![node("a.bin", 5 << 30, 100 << 20, vec![])],
                ),
                node("fotos-dir", 2 << 30, 2 << 30, vec![]),
            ],
        );
        let mut outcome = ScanOutcome::complete(tree);
        outcome.allocated_known = true;
        assert_eq!(SortBy::effective(None, true), SortBy::Allocated);
        assert_eq!(SortBy::effective(Some(SortBy::Size), true), SortBy::Size);
        assert_eq!(
            SortBy::effective(Some(SortBy::Allocated), false),
            SortBy::Size
        );

        let report = report_json(&outcome, SortBy::Allocated, 0, 2);
        assert_eq!(report["sort"], "allocated");
        assert_eq!(report["allocated_known"], true);
        assert_eq!(report["size"], 7u64 << 30);
        assert_eq!(report["allocated"], (2u64 << 30) + (100 << 20));
        assert_eq!(report["entries"][0]["name"], "fotos-dir");
        assert_eq!(report["entries"][1]["name"], "geloescht");
        assert_eq!(report["entries"][1]["size"], 5u64 << 30);
        assert_eq!(report["entries"][1]["allocated"], 100u64 << 20);
        assert_eq!(report["entries"][1]["entries"][0]["name"], "a.bin");
        let by_size = report_json(&outcome, SortBy::Size, 1, 1);
        assert_eq!(by_size["entries"].as_array().map(Vec::len), Some(1));
        assert_eq!(by_size["entries"][0]["name"], "geloescht");
        assert!(by_size["entries"][0].get("entries").is_none());
        let text = report_text(&report);
        assert!(text.contains("geloescht/a.bin"), "{text}");
        assert!(text.contains(&format_bytes(100 << 20)), "{text}");

        outcome.allocated_known = false;
        let unknown = report_json(&outcome, SortBy::effective(None, false), 0, 1);
        assert_eq!(unknown["sort"], "size");
        assert_eq!(unknown["entries"][0]["name"], "geloescht");
        let system = node(crate::analytics::SYSTEM_FILES_NAME, 1, 1, vec![]);
        let mut with_system = ScanOutcome::complete(node("C:", 1, 1, vec![system]));
        with_system
            .notes
            .push("Analyseweg: schneller NTFS-Weg (MFT)".into());
        let report_system = report_json(&with_system, SortBy::Size, 0, 1);
        assert_eq!(report_system["entries"][0]["system_files"], true);
        let text = report_text(&report_system);
        assert!(
            text.contains("nicht löschbar") && text.contains("Analyseweg"),
            "{text}"
        );
        let mut with_views = report_json(&outcome, SortBy::Size, 0, 1);
        add_overview(
            &mut with_views,
            outcome.tree.as_ref().unwrap(),
            SortBy::Size,
            Some(1),
            true,
        );
        assert_eq!(with_views["largest_files"][0]["path"], "geloescht/a.bin");
        assert_eq!(with_views["types"][0]["extension"], "bin");
        assert_eq!(with_views["aggregated"]["files"], 0);
        assert!(report_text(&with_views).contains("Largest files"));
        let text = report_text(&unknown);
        assert!(
            text.contains("unknown") && !text.contains(&format_bytes(100 << 20)),
            "{text}"
        );
    }
}
