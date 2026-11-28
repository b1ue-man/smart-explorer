//! The replacement side of the result tree of „In Dateien suchen“ (plan
//! docs/plaene/2026-11-28-ersetzen-masken-shell, part C „Auswahl“): which
//! hits a replacement takes (the selected rows, or all that remain after
//! „Aus dem Ergebnis entfernen“, as VS Code's „Replace“/„Dismiss“), the
//! record of a finished run with its totals and its „Rückgängig“ state.
//! Free of `egui`.
use crate::agent_proto::ContentQuery;
use crate::app::content_search_results::{
    files_word, group_thousands, RecordState, Row, RowKind, SearchRecord, SearchResults,
};
use crate::app::text_tools::TextPlace;

/// One hit line a replacement takes, with what the search saw of its file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct SelectedHit {
    /// Path relative to the search root, `/`-separated.
    pub(in crate::app) rel: String,
    /// 1-based line number.
    pub(in crate::app) line: u64,
    /// Byte ranges of the matches within the line; empty for a cut line
    /// (its ranges count from the cut): then every match of that line.
    pub(in crate::app) ranges: Vec<(u32, u32)>,
    pub(in crate::app) size: u64,
    pub(in crate::app) mtime_ms: i64,
}

/// Which hits a run replaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum JobSelection {
    /// Every match of a new walk over the folder (the dialog's button).
    All,
    /// The chosen hits of a finished search, in files unchanged since.
    Hits(Vec<SelectedHit>),
}

/// One run of „In Dateien ersetzen“ as the surfaces hand it over.
#[derive(Clone)]
pub(in crate::app) struct ReplaceJob {
    pub(in crate::app) query: ContentQuery,
    /// The searched folder in the coordinates of `place`.
    pub(in crate::app) root: String,
    pub(in crate::app) place: TextPlace,
    pub(in crate::app) replacement: String,
    pub(in crate::app) selection: JobSelection,
    /// The search record the hits come from.
    pub(in crate::app) source: Option<u64>,
}

impl ReplaceJob {
    /// Stellen the selection names (for a cut line: at least one).
    pub(in crate::app) fn chosen_count(&self) -> Option<u64> {
        match &self.selection {
            JobSelection::All => None,
            JobSelection::Hits(hits) => {
                Some(hits.iter().map(|hit| hit.ranges.len().max(1) as u64).sum())
            }
        }
    }
}

/// „Rückgängig“ of a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum UndoState {
    /// No backup to restore (nothing changed, or the run named none).
    Unavailable,
    /// The run's backups can be restored.
    Available(String),
    Running,
    Done,
    /// Restored, except these many files changed since the run (they are
    /// listed with the skipped files).
    Partly(u64),
    Failed(String),
}

/// What a record of a run of „In Dateien ersetzen“ adds to a search record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct ReplaceInfo {
    pub(in crate::app) replacement: String,
    /// Only chosen hits (not a new walk over the folder).
    pub(in crate::app) chosen: bool,
    /// Replacements so far (the core's total at the end).
    pub(in crate::app) replacements: u64,
    /// Files searched (the core's total at the end).
    pub(in crate::app) files_searched: u64,
    pub(in crate::app) undo: UndoState,
    /// The search record the hits came from.
    pub(in crate::app) source: Option<u64>,
}

fn replacements_word(count: u64) -> &'static str {
    if count == 1 {
        "Ersetzung"
    } else {
        "Ersetzungen"
    }
}

impl ReplaceInfo {
    /// `Ersetzen „foo“ durch „bar“ (3 Ersetzungen in 2 Dateien von 10
    /// durchsuchten, 1 übersprungen) [Normal]`, with the state of „Rückgängig“.
    pub(in crate::app) fn header(&self, record: &SearchRecord) -> String {
        let what = if self.chosen {
            "Ersetzen (Auswahl)"
        } else {
            "Ersetzen"
        };
        let head = format!("{what} „{}“ durch „{}“", record.pattern, self.replacement);
        let files = record.files.len() as u64;
        let counts = format!(
            "{} {} in {} {}",
            group_thousands(self.replacements),
            replacements_word(self.replacements),
            group_thousands(files),
            files_word(files)
        );
        let skipped = record.skipped.len() as u64;
        let skipped = format!("{} übersprungen", group_thousands(skipped));
        let searched = group_thousands(self.files_searched.max(record.progress.0));
        let body = match &record.state {
            RecordState::Running => format!("(läuft: {counts}, {searched} durchsucht)"),
            RecordState::Done(_) => format!("({counts} von {searched} durchsuchten, {skipped})"),
            RecordState::Cancelled => format!("(abgebrochen: {counts}, {skipped})"),
            RecordState::Failed(error) => {
                format!("fehlgeschlagen nach {counts}, {skipped}: {error}")
            }
        };
        let undo = match &self.undo {
            UndoState::Unavailable | UndoState::Available(_) => String::new(),
            UndoState::Running => " – wird rückgängig gemacht …".to_string(),
            UndoState::Done => " – rückgängig gemacht".to_string(),
            UndoState::Partly(left) => format!(
                " – teilweise rückgängig gemacht, {} nicht zurückgesetzt",
                group_thousands(*left)
            ),
            UndoState::Failed(error) => format!(" – Rückgängig fehlgeschlagen: {error}"),
        };
        format!("{head} {body} {}{undo}", record.options)
    }
}

impl SearchRecord {
    /// The record of a run of `job` (put in front of the panel).
    pub(in crate::app) fn new_replace(id: u64, job: &ReplaceJob) -> Self {
        let mut record = Self::new(id, &job.query, job.root.clone(), job.place.clone());
        record.replace = Some(ReplaceInfo {
            replacement: job.replacement.clone(),
            chosen: job.selection != JobSelection::All,
            replacements: 0,
            files_searched: 0,
            undo: UndoState::Unavailable,
            source: job.source,
        });
        record
    }

    /// A search whose hits a replacement can take: a core search (not a
    /// filtered or replacement record) that ended or was cancelled.
    pub(in crate::app) fn replaceable(&self) -> Result<&ContentQuery, String> {
        if self.replace.is_some() {
            return Err("Ein Ergebnis von „Ersetzen“ lässt sich nicht erneut ersetzen.".into());
        }
        let Some(query) = &self.query else {
            return Err(
                "Treffer aus „In diesen Ergebnissen suchen“ lassen sich nicht ersetzen.".into(),
            );
        };
        match self.state {
            RecordState::Running => Err("Die Suche läuft noch.".into()),
            RecordState::Failed(_) => Err("Die Suche ist fehlgeschlagen.".into()),
            RecordState::Done(_) | RecordState::Cancelled => Ok(query),
        }
    }

    /// The hit lines of `file` (all when `line` is None) as replacement hits.
    fn selected_hits(&self, file: usize, line: Option<usize>, out: &mut Vec<SelectedHit>) {
        let Some(hits) = self.files.get(file) else {
            return;
        };
        for (index, hit) in hits.lines.iter().enumerate() {
            if hit.context || line.is_some_and(|wanted| wanted != index) {
                continue;
            }
            let selected = SelectedHit {
                rel: hits.rel.clone(),
                line: hit.line,
                ranges: if hit.truncated {
                    Vec::new()
                } else {
                    hit.ranges.clone()
                },
                size: hits.size,
                mtime_ms: hits.mtime_ms,
            };
            if !out.contains(&selected) {
                out.push(selected);
            }
        }
    }
}

impl SearchResults {
    /// The job that replaces the hits of `rows` (a header row: all of its
    /// search; a file row: all of its lines; context lines are passed over).
    /// All rows must belong to one search.
    pub(in crate::app) fn replace_selected(
        &self,
        rows: &[Row],
        replacement: &str,
    ) -> Result<ReplaceJob, String> {
        let Some(first) = rows.first() else {
            return Err("Bitte Treffer oder Dateien auswählen.".into());
        };
        if rows.iter().any(|row| row.search != first.search) {
            return Err("Bitte nur Treffer einer Suche auswählen.".into());
        }
        let record = self
            .record(first.search)
            .ok_or_else(|| "Die Suche ist nicht mehr vorhanden.".to_string())?;
        let query = record.replaceable()?;
        let mut hits = Vec::new();
        for row in rows {
            match row.kind {
                RowKind::Header => {
                    for file in 0..record.files.len() {
                        record.selected_hits(file, None, &mut hits);
                    }
                }
                RowKind::File(file) => record.selected_hits(file, None, &mut hits),
                RowKind::Line(file, line) => record.selected_hits(file, Some(line), &mut hits),
                RowKind::SkippedHeader | RowKind::Skipped(_) => {}
            }
        }
        if hits.is_empty() {
            return Err("Die Auswahl enthält keine Trefferzeile.".into());
        }
        Ok(ReplaceJob {
            query: query.clone(),
            root: record.root.clone(),
            place: record.place.clone(),
            replacement: replacement.to_string(),
            selection: JobSelection::Hits(hits),
            source: Some(record.id),
        })
    }

    /// „Alle verbleibenden ersetzen…“: every hit line still in `search`.
    pub(in crate::app) fn replace_remaining(
        &self,
        search: u64,
        replacement: &str,
    ) -> Result<ReplaceJob, String> {
        let header = Row {
            search,
            kind: RowKind::Header,
        };
        self.replace_selected(&[header], replacement)
    }

    /// „Aus dem Ergebnis entfernen“ (Entf, VS Code „Dismiss“): a header row
    /// drops its search, a file row its file, a hit line itself, a skipped
    /// row its entry; a file without hit lines goes as well. What is gone is
    /// never replaced. `true` when anything was taken out.
    pub(in crate::app) fn dismiss(&mut self, rows: &[Row]) -> bool {
        let mut changed = false;
        let searches: Vec<u64> = rows
            .iter()
            .filter(|row| row.kind == RowKind::Header)
            .map(|row| row.search)
            .collect();
        if !searches.is_empty() {
            self.records.retain(|record| !searches.contains(&record.id));
            changed = true;
        }
        for record in &mut self.records {
            let mine: Vec<RowKind> = rows
                .iter()
                .filter(|row| row.search == record.id)
                .map(|row| row.kind)
                .collect();
            if mine.is_empty() {
                continue;
            }
            changed |= dismiss_in(record, &mine);
        }
        if changed {
            self.touch();
        }
        changed
    }
}

fn dismiss_in(record: &mut SearchRecord, kinds: &[RowKind]) -> bool {
    let mut changed = false;
    let mut drop_files = vec![false; record.files.len()];
    let mut drop_lines: Vec<Vec<bool>> = record
        .files
        .iter()
        .map(|file| vec![false; file.lines.len()])
        .collect();
    let mut drop_skipped = vec![false; record.skipped.len()];
    for kind in kinds {
        match *kind {
            RowKind::File(file) if file < drop_files.len() => drop_files[file] = true,
            RowKind::Line(file, line) if drop_lines.get(file).is_some_and(|l| line < l.len()) => {
                drop_lines[file][line] = true
            }
            RowKind::SkippedHeader => drop_skipped.iter_mut().for_each(|drop| *drop = true),
            RowKind::Skipped(index) if index < drop_skipped.len() => drop_skipped[index] = true,
            _ => {}
        }
    }
    for (slot, file) in record.files.iter_mut().enumerate() {
        if drop_files[slot] || !drop_lines[slot].contains(&true) {
            continue;
        }
        let mut keep = drop_lines[slot].iter().map(|drop| !drop);
        file.lines.retain(|_| keep.next().unwrap_or(true));
        file.hits = file.lines.iter().filter(|line| !line.context).count() as u64;
        changed = true;
        if file.hits == 0 {
            drop_files[slot] = true;
        }
    }
    if drop_files.contains(&true) {
        let mut keep = drop_files.iter().map(|drop| !drop);
        record.files.retain(|_| keep.next().unwrap_or(true));
        record.reindex();
        changed = true;
    }
    if drop_skipped.contains(&true) {
        let mut keep = drop_skipped.iter().map(|drop| !drop);
        record.skipped.retain(|_| keep.next().unwrap_or(true));
        changed = true;
    }
    changed
}
