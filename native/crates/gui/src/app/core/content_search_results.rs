//! The result tree of „In Dateien suchen“ (plan
//! docs/plaene/2026-11-27-dateien-suche-ansicht, B1 and E): one record per
//! search, newest first, as Notepad++'s „Search results“ — a header with the
//! totals, one foldable entry per file with its hit lines, and the skipped
//! files. Free of `egui`; the panel (`content_search_results_ui.rs`) draws
//! the flattened rows of this model.
use std::collections::HashMap;

use crate::agent_proto::{
    ContentEvent, ContentQuery, ContentSummary, LineHit, SkipReason, TextMode,
};

use super::text_tools::content_replace_results::ReplaceInfo;
use super::text_tools::TextPlace;

/// How a search record stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum RecordState {
    Running,
    Done(ContentSummary),
    Cancelled,
    Failed(String),
}

/// One file with hits.
#[derive(Clone, Debug)]
pub(in crate::app) struct FileHits {
    /// Path relative to the search root, `/`-separated.
    pub(in crate::app) rel: String,
    pub(in crate::app) size: u64,
    /// Modification time the search saw (ms since 1970); a replacement
    /// skips the file when it changed since.
    pub(in crate::app) mtime_ms: i64,
    /// Hit lines and context lines in file order.
    pub(in crate::app) lines: Vec<LineHit>,
    /// Hit lines (context lines not counted); replacements in a record of
    /// „In Dateien ersetzen“.
    pub(in crate::app) hits: u64,
    pub(in crate::app) expanded: bool,
}

/// One search and everything it found.
pub(in crate::app) struct SearchRecord {
    pub(in crate::app) id: u64,
    pub(in crate::app) pattern: String,
    /// `[Normal: Groß/Klein/Wort]`.
    pub(in crate::app) options: String,
    /// The searched folder in the coordinates of `place`.
    pub(in crate::app) root: String,
    pub(in crate::app) place: TextPlace,
    /// Set for „In diesen Ergebnissen suchen“: the pattern of the source search.
    pub(in crate::app) within: Option<String>,
    /// The query of the core search (None for „In diesen Ergebnissen
    /// suchen“, whose lines a replacement cannot name).
    pub(in crate::app) query: Option<ContentQuery>,
    /// Set for the record of a run of „In Dateien ersetzen“.
    pub(in crate::app) replace: Option<ReplaceInfo>,
    pub(in crate::app) files: Vec<FileHits>,
    index: HashMap<String, usize>,
    /// (relative path, reason) of files and folders that were not searched.
    pub(in crate::app) skipped: Vec<(String, String)>,
    pub(in crate::app) skipped_open: bool,
    pub(in crate::app) expanded: bool,
    /// Running totals (files searched, bytes read, hit lines).
    pub(in crate::app) progress: (u64, u64, u64),
    pub(in crate::app) state: RecordState,
}

/// What a row of the flattened tree shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::app) enum RowKind {
    Header,
    File(usize),
    Line(usize, usize),
    SkippedHeader,
    Skipped(usize),
}

/// One visible row; the search is named by its id, so rows stay valid when a
/// newer search is put in front.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::app) struct Row {
    pub(in crate::app) search: u64,
    pub(in crate::app) kind: RowKind,
}

/// Every search of the panel, newest first.
#[derive(Default)]
pub(in crate::app) struct SearchResults {
    pub(in crate::app) records: Vec<SearchRecord>,
    /// Changes whenever rows appear, vanish or fold (cache key of the panel).
    pub(in crate::app) generation: u64,
    next_id: u64,
}

/// `[Normal: Groß/Klein/Wort]` for the header (Notepad++: `[Normal: Case/Word]`).
pub(in crate::app) fn options_label(query: &ContentQuery) -> String {
    let mode = match query.mode {
        TextMode::Normal => "Normal",
        TextMode::Extended => "Erweitert",
        TextMode::Regex => "Regex",
    };
    let mut flags = Vec::new();
    if query.match_case {
        flags.push("Groß/Klein");
    }
    if query.whole_word && query.mode != TextMode::Regex {
        flags.push("Wort");
    }
    if query.dot_matches_newline && query.mode == TextMode::Regex {
        flags.push(". findet Zeilenumbruch");
    }
    if flags.is_empty() {
        format!("[{mode}]")
    } else {
        format!("[{mode}: {}]", flags.join("/"))
    }
}

/// `1234567` → `1.234.567` (German thousands separator).
pub(in crate::app) fn group_thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push('.');
        }
        out.push(ch);
    }
    out
}

pub(in crate::app) fn files_word(count: u64) -> &'static str {
    if count == 1 {
        "Datei"
    } else {
        "Dateien"
    }
}

/// German text of a skip reason.
pub(in crate::app) fn skip_text(reason: &SkipReason) -> String {
    match reason {
        SkipReason::Binary => "Binärdatei (nicht durchsucht)".to_string(),
        SkipReason::Unreadable(cause) => format!("nicht lesbar: {cause}"),
        SkipReason::UnlistableFolder(cause) => format!("Ordner nicht lesbar: {cause}"),
    }
}

/// `root` and `rel` joined with `/`.
pub(in crate::app) fn join_path(root: &str, rel: &str) -> String {
    if rel.is_empty() {
        return root.to_string();
    }
    if root.is_empty() {
        return rel.to_string();
    }
    if root.ends_with('/') {
        format!("{root}{rel}")
    } else {
        format!("{root}/{rel}")
    }
}

impl SearchRecord {
    pub(in crate::app) fn new(
        id: u64,
        query: &ContentQuery,
        root: String,
        place: TextPlace,
    ) -> Self {
        Self {
            id,
            pattern: query.pattern.clone(),
            options: options_label(query),
            root,
            place,
            within: None,
            query: Some(query.clone()),
            replace: None,
            files: Vec::new(),
            index: HashMap::new(),
            skipped: Vec::new(),
            skipped_open: false,
            expanded: true,
            progress: (0, 0, 0),
            state: RecordState::Running,
        }
    }

    /// Hit lines collected so far.
    pub(in crate::app) fn hit_count(&self) -> u64 {
        self.files.iter().map(|file| file.hits).sum()
    }

    pub(in crate::app) fn full_path(&self, file: usize) -> Option<String> {
        self.files
            .get(file)
            .map(|hits| join_path(&self.root, &hits.rel))
    }

    /// Takes one event of the running search.
    pub(in crate::app) fn apply(&mut self, event: ContentEvent) {
        match event {
            ContentEvent::Hits {
                rel,
                size,
                mtime_ms,
                lines,
                ..
            } => {
                let slot = match self.index.get(&rel) {
                    Some(slot) => *slot,
                    None => {
                        self.files.push(FileHits {
                            rel: rel.clone(),
                            size,
                            mtime_ms,
                            lines: Vec::new(),
                            hits: 0,
                            expanded: true,
                        });
                        self.index.insert(rel, self.files.len() - 1);
                        self.files.len() - 1
                    }
                };
                let file = &mut self.files[slot];
                file.hits += lines.iter().filter(|line| !line.context).count() as u64;
                file.lines.extend(lines);
            }
            ContentEvent::Skipped { rel, reason } => self.skipped.push((rel, skip_text(&reason))),
            ContentEvent::Progress { files, bytes, hits } => self.progress = (files, bytes, hits),
        }
    }

    /// Adds a file of a filtered record (no event stream).
    pub(in crate::app) fn push_file(&mut self, file: FileHits) {
        self.index.insert(file.rel.clone(), self.files.len());
        self.files.push(file);
    }

    /// The slot of the file `rel`, if the record has it.
    pub(in crate::app) fn file_slot(&self, rel: &str) -> Option<usize> {
        self.index.get(rel).copied()
    }

    /// Rebuilds the path index after files were taken out.
    pub(in crate::app) fn reindex(&mut self) {
        self.index = self
            .files
            .iter()
            .enumerate()
            .map(|(slot, file)| (file.rel.clone(), slot))
            .collect();
    }

    /// The header: `Suche „foo“ (3 Treffer in 2 Dateien von 10 durchsuchten) [Normal]`.
    pub(in crate::app) fn header(&self) -> String {
        if let Some(info) = &self.replace {
            return info.header(self);
        }
        let within = self
            .within
            .as_ref()
            .map(|source| format!(" in den Ergebnissen von „{source}“"))
            .unwrap_or_default();
        let head = format!("Suche „{}“{within}", self.pattern);
        let files = self.files.len() as u64;
        let hits = self.hit_count();
        match &self.state {
            RecordState::Running => format!(
                "{head} (läuft: {} Treffer in {} {}, {} durchsucht) {}",
                group_thousands(hits),
                group_thousands(files),
                files_word(files),
                group_thousands(self.progress.0),
                self.options
            ),
            RecordState::Done(summary) => format!(
                "{head} ({} Treffer in {} {} von {} durchsuchten) {}",
                group_thousands(hits),
                group_thousands(files),
                files_word(files),
                group_thousands(summary.files_searched.max(self.progress.0)),
                self.options
            ),
            RecordState::Cancelled => format!(
                "{head} (abgebrochen: {} Treffer in {} {} von {} durchsuchten) {}",
                group_thousands(hits),
                group_thousands(files),
                files_word(files),
                group_thousands(self.progress.0),
                self.options
            ),
            RecordState::Failed(error) => format!("{head} fehlgeschlagen: {error}"),
        }
    }

    /// `pfad (3 Treffer)` of a file row.
    pub(in crate::app) fn file_label(&self, file: usize) -> String {
        let Some(hits) = self.files.get(file) else {
            return String::new();
        };
        let unit = match (&self.replace, hits.hits) {
            (None, _) => "Treffer",
            (Some(_), 1) => "Ersetzung",
            (Some(_), _) => "Ersetzungen",
        };
        format!(
            "{} ({} {unit})",
            join_path(&self.root, &hits.rel),
            group_thousands(hits.hits)
        )
    }
}

impl SearchResults {
    pub(in crate::app) fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    pub(in crate::app) fn touch(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    /// Puts a new search in front; older searches fold, or go when `clear_each`.
    pub(in crate::app) fn begin(&mut self, record: SearchRecord, clear_each: bool) {
        if clear_each {
            self.records.clear();
        }
        for older in &mut self.records {
            older.expanded = false;
        }
        self.records.insert(0, record);
        self.touch();
    }

    pub(in crate::app) fn record(&self, id: u64) -> Option<&SearchRecord> {
        self.records.iter().find(|record| record.id == id)
    }

    pub(in crate::app) fn record_mut(&mut self, id: u64) -> Option<&mut SearchRecord> {
        self.records.iter_mut().find(|record| record.id == id)
    }

    pub(in crate::app) fn apply(&mut self, id: u64, event: ContentEvent) {
        let rows_change = !matches!(event, ContentEvent::Progress { .. });
        if let Some(record) = self.record_mut(id) {
            record.apply(event);
        }
        if rows_change {
            self.touch();
        }
    }

    pub(in crate::app) fn finish(&mut self, id: u64, state: RecordState) {
        if let Some(record) = self.record_mut(id) {
            if let RecordState::Done(summary) = &state {
                record.progress.0 = record.progress.0.max(summary.files_searched);
            }
            record.state = state;
        }
        self.touch();
    }

    pub(in crate::app) fn clear(&mut self) {
        self.records.clear();
        self.touch();
    }

    /// Folds or unfolds every search and every file.
    pub(in crate::app) fn set_all_expanded(&mut self, expanded: bool) {
        for record in &mut self.records {
            record.expanded = expanded;
            for file in &mut record.files {
                file.expanded = expanded;
            }
        }
        self.touch();
    }

    /// Folds or unfolds the entry of a header, file or skipped row.
    pub(in crate::app) fn toggle(&mut self, row: Row) {
        if let Some(record) = self.record_mut(row.search) {
            match row.kind {
                RowKind::Header => record.expanded = !record.expanded,
                RowKind::File(file) => {
                    if let Some(hits) = record.files.get_mut(file) {
                        hits.expanded = !hits.expanded;
                    }
                }
                RowKind::SkippedHeader => record.skipped_open = !record.skipped_open,
                RowKind::Line(..) | RowKind::Skipped(_) => {}
            }
        }
        self.touch();
    }

    /// Unfolds the search and file of `row`, so it is visible.
    pub(in crate::app) fn reveal(&mut self, row: Row) {
        if let Some(record) = self.record_mut(row.search) {
            record.expanded = true;
            match row.kind {
                RowKind::File(file) | RowKind::Line(file, _) => {
                    if let Some(hits) = record.files.get_mut(file) {
                        hits.expanded = true;
                    }
                }
                RowKind::Skipped(_) => record.skipped_open = true,
                RowKind::Header | RowKind::SkippedHeader => {}
            }
        }
        self.touch();
    }

    /// The visible rows, top to bottom.
    pub(in crate::app) fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for record in &self.records {
            let search = record.id;
            rows.push(Row {
                search,
                kind: RowKind::Header,
            });
            if !record.expanded {
                continue;
            }
            for (file, hits) in record.files.iter().enumerate() {
                rows.push(Row {
                    search,
                    kind: RowKind::File(file),
                });
                if hits.expanded {
                    rows.extend((0..hits.lines.len()).map(|line| Row {
                        search,
                        kind: RowKind::Line(file, line),
                    }));
                }
            }
            if !record.skipped.is_empty() {
                rows.push(Row {
                    search,
                    kind: RowKind::SkippedHeader,
                });
                if record.skipped_open {
                    rows.extend((0..record.skipped.len()).map(|index| Row {
                        search,
                        kind: RowKind::Skipped(index),
                    }));
                }
            }
        }
        rows
    }

    /// The line of a line row.
    pub(in crate::app) fn line(&self, row: Row) -> Option<&LineHit> {
        let RowKind::Line(file, line) = row.kind else {
            return None;
        };
        self.record(row.search)?.files.get(file)?.lines.get(line)
    }

    /// Text of a row as the panel shows it (without folding marks).
    pub(in crate::app) fn row_text(&self, row: Row) -> String {
        let Some(record) = self.record(row.search) else {
            return String::new();
        };
        match row.kind {
            RowKind::Header => record.header(),
            RowKind::File(file) => record.file_label(file),
            RowKind::Line(..) => self
                .line(row)
                .map(|line| format!("Zeile {}: {}", line.line, line.text))
                .unwrap_or_default(),
            RowKind::SkippedHeader => format!(
                "Ausgelassen ({})",
                group_thousands(record.skipped.len() as u64)
            ),
            RowKind::Skipped(index) => record
                .skipped
                .get(index)
                .map(|(rel, reason)| format!("{} – {reason}", join_path(&record.root, rel)))
                .unwrap_or_default(),
        }
    }
}
