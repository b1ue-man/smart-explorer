//! „Größe“ und „Belegt“ in `analyze.node` (2026-11-18): jede Zeile trägt
//! `size` und `allocated` (belegter Platz), der Knoten `allocated`,
//! `allocatedKnown` und `sort`. Sortierung und Auswahl der größten Kinder
//! folgen `sort` (`allocated` | `size`); ohne Angabe gilt „Belegt“, wo das
//! Ergebnis den belegten Platz kennt, sonst „Größe“. Kennt das Ergebnis ihn
//! nicht (ältere Gegenstelle, Backend ohne Wert), ist `allocated` gleich
//! `size` und `allocatedKnown` false. Die Näherungszeilen laut Android
//! (Apps, fremde App-Daten, Rest) belegen, was Android meldet: `allocated`
//! = `size`.
use serde_json::{json, Value};

use crate::analytics::{ChildView, NodeKind, NodeView, SizeNode};
use crate::mobile::ApiError;

/// Was Sortierung und Auswahl der Kinder bestimmt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Measure {
    Allocated,
    Size,
}

impl Measure {
    /// `sort` der Anfrage; ohne bekannten belegten Platz immer „Größe“.
    pub(super) fn requested(args: &Value, known: bool) -> Result<Self, ApiError> {
        let chosen = match args.get("sort").and_then(Value::as_str) {
            None | Some("allocated") => Self::Allocated,
            Some("size") => Self::Size,
            Some(other) => {
                return Err(super::invalid(format!(
                    "Unbekannte Sortierung „{other}“ (allocated oder size)."
                )))
            }
        };
        Ok(if known { chosen } else { Self::Size })
    }

    fn key(self) -> &'static str {
        match self {
            Self::Allocated => "allocated",
            Self::Size => "size",
        }
    }

    fn of(self, row: &ChildView) -> u64 {
        match self {
            Self::Allocated => row.allocated,
            Self::Size => row.size,
        }
    }
}

/// The tree node at `segments` (exact names, as `node_view` follows them);
/// `None` for the app list, which is no node of the tree.
pub(super) fn tree_node<'t>(tree: &'t SizeNode, segments: &[String]) -> Option<&'t SizeNode> {
    let mut node = tree;
    for segment in segments {
        node = node
            .children
            .iter()
            .find(|child| &*child.name == segment.as_str())?;
    }
    Some(node)
}

/// The answer of `analyze.node`: `view` (taken with every child of a tree
/// node, or the app list as `node_view` folded it) sorted by `measure`, at
/// most `max` rows; the estimate rows always stay, as in `node_view`.
pub(super) fn node_json(
    mut view: NodeView,
    known: bool,
    measure: Measure,
    max: usize,
) -> Result<Value, ApiError> {
    let app_list = view.kind == NodeKind::Apps;
    let (mut rows, estimates): (Vec<ChildView>, Vec<ChildView>) =
        std::mem::take(&mut view.children)
            .into_iter()
            .partition(|row| {
                app_list
                    || matches!(
                        row.kind,
                        NodeKind::Dir | NodeKind::File | NodeKind::Aggregate
                    )
            });
    let order = |left: &ChildView, right: &ChildView| {
        measure
            .of(right)
            .cmp(&measure.of(left))
            .then_with(|| left.name.cmp(&right.name))
    };
    rows.sort_by(order);
    rows.truncate(max.saturating_sub(estimates.len()));
    rows.extend(estimates);
    rows.sort_by(order);
    view.children = rows;
    let mut value =
        serde_json::to_value(&view).map_err(|error| ApiError::internal(error.to_string()))?;
    value["allocatedKnown"] = json!(known);
    value["sort"] = json!(measure.key());
    // „NTFS-Systemdateien“ of the fast NTFS way: occupied by the file system
    // itself, nothing to open or delete.
    if let Some(rows) = value["children"].as_array_mut() {
        for row in rows {
            if row["kind"] == "file" && row["name"] == crate::analytics::SYSTEM_FILES_NAME {
                row["systemFiles"] = json!(true);
            }
        }
    }
    Ok(value)
}

/// The overview shared with the desktop and `se analyze` (storage core).
use crate::analytics::overview as tree_overview;

/// Largest files `analyze.overview` lists unless `top` says otherwise.
const OVERVIEW_TOP: usize = 100;

/// `analyze.overview {taskId, path, sort?, top?}` → `{sort, allocatedKnown,
/// largest:[{path, size, allocated, location (its folder)}], types:[{extension, files,
/// size, allocated}], aggregatedFiles, aggregatedSize, aggregatedAllocated,
/// aggregatedNote}`: „Größte Dateien“ and „Nach Dateityp“ of the folder at
/// `path` with all its subfolders. Files folded into „… N weitere
/// Eintraege“ count only in `aggregated*` (`aggregatedNote` says so).
pub(in super::super) fn overview(args: &Value) -> Result<Value, ApiError> {
    let task = super::str_arg(args, "taskId")?;
    let segments = super::string_list(args, "path")?;
    let top = super::super::args::opt_i64(args, "top")
        .map_or(OVERVIEW_TOP, |top| top.clamp(0, 10_000) as usize);
    super::with_stored(task, |stored| {
        let super::Stored::Analysis {
            outcome,
            base,
            root,
            ..
        } = stored
        else {
            return Err(super::invalid("Dieser Task ist keine Speicheranalyse."));
        };
        let tree = outcome
            .tree
            .as_ref()
            .ok_or_else(|| ApiError::new("not_found", "Kein Analyseergebnis."))?;
        let node = tree_node(tree, &segments)
            .ok_or_else(|| ApiError::new("not_found", "Eintrag nicht gefunden."))?;
        let measure = Measure::requested(args, outcome.allocated_known)?;
        let view = tree_overview::overview(node, measure == Measure::Allocated, top);
        let folder = super::join_segments(root, &segments);
        let largest: Vec<Value> = view
            .largest
            .iter()
            .map(|file| {
                // The folder holding the file, to open in "Dateien".
                let parent = match file.path.rsplit_once('/') {
                    Some((dir, _)) => format!("{}/{dir}", folder.trim_end_matches('/')),
                    None => folder.clone(),
                };
                json!({
                    "path": file.path,
                    "size": file.size,
                    "allocated": file.allocated,
                    // The file system's own space has no place to open.
                    "location": (file.path != crate::analytics::SYSTEM_FILES_NAME)
                        .then(|| super::location_for(base, &parent)),
                })
            })
            .collect();
        let types: Vec<Value> = view
            .types
            .iter()
            .map(|total| {
                json!({
                    "extension": total.extension,
                    "files": total.files,
                    "size": total.size,
                    "allocated": total.allocated,
                })
            })
            .collect();
        Ok(json!({
            "sort": measure.key(),
            "allocatedKnown": outcome.allocated_known,
            "largest": largest,
            "types": types,
            "aggregatedFiles": view.aggregated.files,
            "aggregatedSize": view.aggregated.size,
            "aggregatedAllocated": view.aggregated.allocated,
            "aggregatedNote": tree_overview::aggregated_note(&view.aggregated),
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::super::{insert_analysis_for_test, node};
    use crate::analytics::{ScanOutcome, SizeNode};
    use serde_json::{json, Value};

    fn sized(name: &str, size: u64, allocated: u64) -> SizeNode {
        SizeNode {
            name: name.into(),
            size,
            allocated,
            is_dir: true,
            children: Vec::new(),
        }
    }

    fn names(value: &Value) -> Vec<&str> {
        value["children"]
            .as_array()
            .map(|rows| rows.iter().filter_map(|row| row["name"].as_str()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn belegt_task_mobile_node_json_carries_both_values_and_sorts_by_allocated() {
        let tree = || {
            let mut root = sized("Daten", 7 << 30, (2 << 30) + (100 << 20));
            let mut system = sized(crate::analytics::SYSTEM_FILES_NAME, 1 << 20, 1 << 20);
            system.is_dir = false;
            root.children = vec![
                sized("geloescht", 5 << 30, 100 << 20),
                sized("fotos", 2 << 30, 2 << 30),
                system,
            ];
            root
        };
        let mut known = ScanOutcome::complete(tree());
        known.allocated_known = true;
        insert_analysis_for_test("belegt-task-known", known, "/sdcard/Daten", "/sdcard/Daten");
        let ask = |task: &str, sort: Option<&str>| {
            let mut args = json!({ "taskId": task, "path": [] });
            if let Some(sort) = sort {
                args["sort"] = json!(sort);
            }
            node(&args).expect("node")
        };
        let root = ask("belegt-task-known", None);
        assert_eq!(root["sort"], "allocated");
        assert_eq!(root["allocatedKnown"], true);
        assert_eq!(root["size"], 7u64 << 30);
        assert_eq!(root["allocated"], (2u64 << 30) + (100 << 20));
        assert_eq!(
            names(&root),
            ["fotos", "geloescht", crate::analytics::SYSTEM_FILES_NAME]
        );
        assert_eq!(root["children"][2]["systemFiles"], true);
        assert!(root["children"][0].get("systemFiles").is_none());
        assert_eq!(root["children"][1]["size"], 5u64 << 30);
        assert_eq!(root["children"][1]["allocated"], 100u64 << 20);
        let by_size = ask("belegt-task-known", Some("size"));
        assert_eq!(by_size["sort"], "size");
        assert_eq!(names(&by_size)[..2], ["geloescht", "fotos"]);
        assert!(
            node(&json!({ "taskId": "belegt-task-known", "path": [], "sort": "blocks" })).is_err()
        );

        // An older peer: allocated unknown, so „Größe“ decides and allocated = size.
        let mut unknown = ScanOutcome::complete(tree());
        unknown.allocated_known = false;
        if let Some(tree) = unknown.tree.as_mut() {
            tree.allocated = tree.size;
            for child in &mut tree.children {
                child.allocated = child.size;
            }
        }
        insert_analysis_for_test("belegt-task-unknown", unknown, "share://direct/pc/D", "/D");
        let root = ask("belegt-task-unknown", Some("allocated"));
        assert_eq!(root["sort"], "size");
        assert_eq!(root["allocatedKnown"], false);
        assert_eq!(names(&root)[..2], ["geloescht", "fotos"]);
        assert_eq!(root["children"][0]["allocated"], 5u64 << 30);

        // „Größte Dateien“ / „Nach Dateityp“ of the same result.
        let view = super::overview(&json!({ "taskId": "belegt-task-known", "path": [] }))
            .expect("overview");
        assert_eq!(view["sort"], "allocated");
        assert_eq!(view["aggregatedFiles"], 0);
        assert!(view["types"].is_array() && view["largest"].is_array());
        let system = view["largest"].as_array().and_then(|rows| {
            rows.iter()
                .find(|row| row["path"] == crate::analytics::SYSTEM_FILES_NAME)
        });
        assert!(system.is_some_and(|row| row["location"].is_null()));
    }
}
