//! Models of the comparison window and the check page (U29-8): the group
//! filter, the rows with their A/B columns, the action buttons of an entry
//! with the questions that come first, the collective actions and the merge
//! of a text conflict. No `egui`, no input/output: the entries are the
//! values the worker delivers (`CompareItemWire`).
#![allow(dead_code)] // some accessors are the acceptance signals (tests) and the surfaces of later packages
use std::collections::HashSet;

use crate::daemon::{CompareItemWire, CompareSideWire};
use crate::linemerge::{Row, TextShape};
use crate::vault::{action_title, group_title, CompareGroup, ItemAction, ItemKind};

/// Which groups the list shows (U29-8): all but `structure` unless asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GroupFilter {
    on: [bool; 9],
}

impl Default for GroupFilter {
    fn default() -> Self {
        let mut on = [true; 9];
        on[Self::index(CompareGroup::Structure)] = false;
        Self { on }
    }
}

impl GroupFilter {
    fn index(group: CompareGroup) -> usize {
        CompareGroup::ALL
            .iter()
            .position(|known| *known == group)
            .unwrap_or(0)
    }

    pub(crate) fn is_on(&self, group: CompareGroup) -> bool {
        self.on[Self::index(group)]
    }

    pub(crate) fn set(&mut self, group: CompareGroup, on: bool) {
        self.on[Self::index(group)] = on;
    }

    /// Whether the entries of the group `key` show.
    pub(crate) fn shows_key(&self, key: &str) -> bool {
        CompareGroup::from_key(key).is_some_and(|group| self.is_on(group))
    }

    /// The group keys to ask the worker for: none (its default, every group
    /// but `structure`) unless `structure` is on, which only a request that
    /// names the groups delivers.
    pub(crate) fn requested(&self) -> Vec<&'static str> {
        if self.is_on(CompareGroup::Structure) {
            CompareGroup::ALL.iter().map(|group| group.key()).collect()
        } else {
            Vec::new()
        }
    }
}

/// `—` for a missing side, else size and time of the side.
pub(crate) fn side_text(side: &CompareSideWire) -> String {
    if !side.exists {
        return "—".to_string();
    }
    let size = side.size.map(crate::format::format_bytes);
    let time = side
        .mtime_ms
        .map(crate::format::format_date)
        .filter(|time| !time.is_empty());
    match (size, time) {
        (Some(size), Some(time)) => format!("{size}, {time}"),
        (Some(text), None) | (None, Some(text)) => text,
        (None, None) => "vorhanden".to_string(),
    }
}

/// The text of an entry in the list: its cleartext path; a key file or a
/// technical entry has none and names itself by note, finding or ID.
pub(crate) fn item_label(item: &CompareItemWire) -> String {
    [
        Some(item.path.as_str()),
        item.note.as_deref(),
        item.finding.as_deref(),
        Some(item.item.as_str()),
    ]
    .into_iter()
    .flatten()
    .find(|text| !text.trim().is_empty())
    .unwrap_or_default()
    .to_string()
}

/// One row of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RowModel {
    /// `{Titel} ({n})`.
    Header {
        group: CompareGroup,
        title: String,
        count: usize,
    },
    Item {
        /// Position in the entries the rows were made from.
        index: usize,
        label: String,
        a: String,
        b: String,
        /// The action on it succeeded; it leaves with the next comparison.
        done: bool,
    },
}

/// The rows: the groups in the order of the plan, each with its header, the
/// entries in the order of the worker. A group the filter hides, or without
/// an entry, has no rows.
pub(crate) fn compare_rows(
    items: &[CompareItemWire],
    filter: &GroupFilter,
    done: &HashSet<String>,
) -> Vec<RowModel> {
    let mut rows = Vec::new();
    for group in CompareGroup::ALL {
        if !filter.is_on(group) {
            continue;
        }
        let members: Vec<(usize, &CompareItemWire)> = items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.group == group.key())
            .collect();
        if members.is_empty() {
            continue;
        }
        rows.push(RowModel::Header {
            group,
            title: format!("{} ({})", group_title(group), members.len()),
            count: members.len(),
        });
        for (index, item) in members {
            rows.push(RowModel::Item {
                index,
                label: item_label(item),
                a: format!("A: {}", side_text(&item.a)),
                b: format!("B: {}", side_text(&item.b)),
                done: done.contains(&item.item),
            });
        }
    }
    rows
}

/// The `ItemKind` of a wire name.
pub(crate) fn kind_of(key: &str) -> ItemKind {
    match key {
        "folder" => ItemKind::Folder,
        "node" => ItemKind::Node,
        "key" => ItemKind::Key,
        _ => ItemKind::File,
    }
}

/// One action button of an entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ButtonModel {
    /// The wire name (`keep-a`, …).
    pub(crate) key: String,
    pub(crate) title: &'static str,
    /// The question that comes first, if the action removes something.
    pub(crate) question: Option<String>,
}

/// The question before an action that removes or replaces data (U29-8):
/// accepting a deletion, merging folders, removing a damaged or doubled
/// node. `path` is the entry; the removed cipher files are kept in a copy.
pub(crate) fn question_for(
    group: CompareGroup,
    action: ItemAction,
    path: &str,
    removed_root: &str,
) -> Option<String> {
    let kept = format!("Eine Kopie des Chiffrats bleibt in {removed_root}.");
    match action {
        ItemAction::ConfirmDelete if group == CompareGroup::Deferred => Some(
            "Die Löschung betrifft den Inhalt eines Ordners, dessen Verzeichnis-ID-Sicherung auf \
             einer Seite fehlt. Trotzdem übernehmen?"
                .to_string(),
        ),
        ItemAction::ConfirmDelete => Some(format!(
            "Die Löschung von „{path}“ wird auf der anderen Seite nachvollzogen. Übernehmen?"
        )),
        ItemAction::MergeFolders => Some(format!(
            "Der Inhalt des Ordners von Seite B wird in den Ordner von Seite A übernommen und \
             dort entfernt. {kept}"
        )),
        ItemAction::RemoveNode
        | ItemAction::RemoveDuplicate
        | ItemAction::RemoveStage
        | ItemAction::SeparateFolder => Some(format!(
            "Dieser Eintrag ist beschädigt oder doppelt. Er wird entfernt. {kept}"
        )),
        _ => None,
    }
}

/// The buttons of an entry: exactly the actions the worker offers, in its
/// order, with the titles of `vault::action_title`.
pub(crate) fn buttons_for(item: &CompareItemWire, removed_root: &str) -> Vec<ButtonModel> {
    let kind = kind_of(&item.kind);
    let group = CompareGroup::from_key(&item.group).unwrap_or(CompareGroup::Conflict);
    item.actions
        .iter()
        .filter_map(|key| {
            let action = ItemAction::from_key(key, kind)?;
            Some(ButtonModel {
                key: key.clone(),
                title: action_title(action),
                question: question_for(group, action, &item.path, removed_root),
            })
        })
        .collect()
}

/// The entries the collective actions „Alle Konflikte: A/B“ concern: a
/// conflict of a file with both sides present that offers both versions.
pub(crate) fn bulk_candidates(items: &[CompareItemWire]) -> usize {
    items
        .iter()
        .filter(|item| {
            item.group == CompareGroup::Conflict.key()
                && item.kind == ItemKind::File.key()
                && item.a.exists
                && item.b.exists
                && item.actions.iter().any(|key| key == "keep-a")
                && item.actions.iter().any(|key| key == "keep-b")
        })
        .count()
}

/// The question before a collective action.
pub(crate) fn bulk_question(count: usize, label: &str) -> String {
    format!("Alle {count} Konflikte zugunsten von {label} auflösen?")
}

/// The buttons „Öffnen (A)“ / „Öffnen (B)“: a side opens when it holds the
/// file; otherwise the button does not exist.
pub(crate) fn open_sides(item: &CompareItemWire) -> [bool; 2] {
    let file = item.kind == ItemKind::File.key() && !item.path.is_empty();
    [file && item.a.exists, file && item.b.exists]
}

/// What the detail shows besides the metadata: the line view of a text
/// conflict (the worker offers `merge` only for two texts within
/// `MERGE_MAX_BYTES`), else only the metadata and the open buttons.
pub(crate) fn offers_text(item: &CompareItemWire) -> bool {
    item.actions.iter().any(|key| key == "merge")
}

/// The line view of two versions and the choices for the merged text.
#[derive(Clone, Debug)]
pub(crate) struct MergeModel {
    pub(crate) rows: Vec<Row>,
    pub(crate) shape_a: TextShape,
    pub(crate) shape_b: TextShape,
    pub(crate) shape: TextShape,
}

/// Builds the view: both versions must have uniform line ends (the shape of
/// the result is one of them); the diff is bounded (`linemerge`).
pub(crate) fn merge_model(a: &str, b: &str) -> Result<MergeModel, String> {
    crate::app::sync_merge_types::merge_text(Some(a.as_bytes()))?;
    crate::app::sync_merge_types::merge_text(Some(b.as_bytes()))?;
    let rows = crate::linemerge::rows(a, b).map_err(|error| error.to_string())?;
    let (shape_a, shape_b) = (TextShape::of(a), TextShape::of(b));
    Ok(MergeModel {
        rows,
        shape_a,
        shape_b,
        shape: shape_a,
    })
}

impl MergeModel {
    /// „Alle A“ / „Alle B“.
    pub(crate) fn take_all(&mut self, side_a: bool) {
        for row in &mut self.rows {
            row.take_left = side_a && row.left.is_some();
            row.take_right = !side_a && row.right.is_some();
        }
        self.shape = if side_a { self.shape_a } else { self.shape_b };
    }

    /// The text the choices give.
    pub(crate) fn result(&self) -> String {
        let bytes = crate::app::sync_merge_types::assemble_text(&self.rows, self.shape);
        String::from_utf8(bytes).unwrap_or_default()
    }

    /// Whether the two versions differ at all.
    pub(crate) fn differs(&self) -> bool {
        self.rows.iter().any(|row| !row.equal)
    }
}

/// The mark of a row of the detail view: blank when equal, `~` changed on
/// both sides, `-` only on A, `+` only on B.
pub(crate) fn row_mark(row: &Row) -> &'static str {
    match (&row.left, &row.right, row.equal) {
        (_, _, true) => " ",
        (Some(_), Some(_), false) => "~",
        (Some(_), None, false) => "-",
        (None, Some(_), false) => "+",
        (None, None, false) => " ",
    }
}

/// The title of a finding of the check: the German description of its kind,
/// else the note of the entry.
pub(crate) fn finding_title(item: &CompareItemWire) -> String {
    item.finding
        .as_deref()
        .and_then(crate::vault::FindingKind::from_name)
        .map(|kind| kind.describe().to_string())
        .or_else(|| item.note.clone())
        .unwrap_or_else(|| "Befund".to_string())
}

/// The folder that keeps a copy of every cipher file a removal took away
/// (U26-7, `vault-removed/`).
pub(crate) fn removed_root() -> String {
    crate::support_dirs::app_data_dir()
        .join("vault-removed")
        .display()
        .to_string()
}
