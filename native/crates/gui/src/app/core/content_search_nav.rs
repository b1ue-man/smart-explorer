//! Moving through, copying from and searching within the result tree of „In
//! Dateien suchen“: F4/Umschalt+F4 (next/previous hit across files, with
//! wrap-around as in Notepad++), the texts of the copy commands and „In
//! diesen Ergebnissen suchen“, which makes a new record of the hit lines
//! that also match a second pattern.
use crate::agent_proto::{ContentSummary, LineHit};

use super::content_search_results::{
    join_path, FileHits, RecordState, Row, RowKind, SearchRecord, SearchResults,
};

/// How „In diesen Ergebnissen suchen“ reads its pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct WithinQuery {
    pub(in crate::app) pattern: String,
    pub(in crate::app) regex: bool,
    pub(in crate::app) match_case: bool,
}

/// The line matcher of a pattern: Normal = the literal text, otherwise a
/// regular expression (Rust syntax); the error names the mistake in German.
pub(in crate::app) fn line_regex(
    pattern: &str,
    regex: bool,
    match_case: bool,
) -> Result<regex::Regex, String> {
    if pattern.is_empty() {
        return Err("Der Suchtext ist leer.".to_string());
    }
    let source = if regex {
        pattern.to_string()
    } else {
        regex::escape(pattern)
    };
    regex::RegexBuilder::new(&source)
        .case_insensitive(!match_case)
        .build()
        .map_err(|error| format!("Ungültiger regulärer Ausdruck: {error}"))
}

impl SearchResults {
    /// The next (`forward`) or previous hit line after `from`, across the
    /// files of `from`'s search (the newest search without `from`); wraps
    /// around at the ends. Context lines are passed over.
    pub(in crate::app) fn next_hit(&self, from: Option<Row>, forward: bool) -> Option<Row> {
        let record = match from {
            Some(row) => self.record(row.search)?,
            None => self.records.first()?,
        };
        let hits: Vec<(usize, usize)> = record
            .files
            .iter()
            .enumerate()
            .flat_map(|(file, hits)| {
                hits.lines
                    .iter()
                    .enumerate()
                    .filter(|(_, line)| !line.context)
                    .map(move |(line, _)| (file, line))
            })
            .collect();
        let first = *hits.first()?;
        let last = *hits.last()?;
        // Position of `from` in (file, line) order; a file row sits before its
        // first line, a header or skipped row before (forward) or after
        // (backward) everything.
        let position: Option<(usize, isize)> = match from.map(|row| row.kind) {
            Some(RowKind::Line(file, line)) => Some((file, line as isize)),
            Some(RowKind::File(file)) => Some((file, -1)),
            _ => None,
        };
        let pick = |(file, line): (usize, usize)| Row {
            search: record.id,
            kind: RowKind::Line(file, line),
        };
        let Some(position) = position else {
            return Some(pick(if forward { first } else { last }));
        };
        let key = |(file, line): (usize, usize)| (file, line as isize);
        let found = if forward {
            hits.iter().copied().find(|hit| key(*hit) > position)
        } else {
            hits.iter().rev().copied().find(|hit| key(*hit) < position)
        };
        Some(pick(found.unwrap_or(if forward { first } else { last })))
    }

    /// Full paths of the files `rows` name (a header names all of its files),
    /// each once, in row order.
    pub(in crate::app) fn row_paths(&self, rows: &[Row]) -> Vec<String> {
        let mut paths: Vec<String> = Vec::new();
        let mut push = |path: String| {
            if !paths.contains(&path) {
                paths.push(path);
            }
        };
        for row in rows {
            let Some(record) = self.record(row.search) else {
                continue;
            };
            match row.kind {
                RowKind::Header => {
                    for file in &record.files {
                        push(join_path(&record.root, &file.rel));
                    }
                }
                RowKind::File(file) | RowKind::Line(file, _) => {
                    if let Some(path) = record.full_path(file) {
                        push(path);
                    }
                }
                RowKind::Skipped(index) => {
                    if let Some((rel, _)) = record.skipped.get(index) {
                        push(join_path(&record.root, rel));
                    }
                }
                RowKind::SkippedHeader => {}
            }
        }
        paths
    }

    /// „Kopieren“: the rows as the panel shows them, one per line.
    pub(in crate::app) fn copy_rows(&self, rows: &[Row]) -> String {
        rows.iter()
            .map(|row| self.row_text(*row))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// „Ausgewählte Zeilen kopieren“: only the text of the selected file
    /// lines, without „Zeile N:“.
    pub(in crate::app) fn copy_lines(&self, rows: &[Row]) -> String {
        rows.iter()
            .filter_map(|row| self.line(*row).map(|line| line.text.clone()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// „In diesen Ergebnissen suchen“: a new record (put in front) with the
    /// hit lines of `source` that also match `query`; its ranges mark the
    /// new pattern. Returns the id of the new record.
    pub(in crate::app) fn search_within(
        &mut self,
        source: u64,
        query: &WithinQuery,
        clear_each: bool,
    ) -> Result<u64, String> {
        let matcher = line_regex(&query.pattern, query.regex, query.match_case)?;
        let Some(record) = self.record(source) else {
            return Err("Die Suche ist nicht mehr vorhanden.".to_string());
        };
        let id = self.next_id_peek();
        let mut derived = derived_record(id, record, query);
        let mut hits_total = 0;
        for file in &record.files {
            let lines: Vec<LineHit> = file
                .lines
                .iter()
                .filter(|line| !line.context)
                .filter_map(|line| {
                    let ranges: Vec<(u32, u32)> = matcher
                        .find_iter(&line.text)
                        .filter(|found| found.end() > found.start())
                        .map(|found| (found.start() as u32, found.end() as u32))
                        .collect();
                    (!ranges.is_empty()).then(|| LineHit {
                        ranges,
                        context: false,
                        ..line.clone()
                    })
                })
                .collect();
            if lines.is_empty() {
                continue;
            }
            hits_total += lines.len() as u64;
            derived.push_file(FileHits {
                rel: file.rel.clone(),
                size: file.size,
                hits: lines.len() as u64,
                lines,
                expanded: true,
            });
        }
        let searched = record.files.len() as u64;
        derived.progress = (searched, 0, hits_total);
        derived.state = RecordState::Done(ContentSummary {
            files_searched: searched,
            files_matched: derived.files.len() as u64,
            hits: hits_total,
            ..ContentSummary::default()
        });
        let id = self.next_id();
        derived.id = id;
        self.begin(derived, clear_each);
        Ok(id)
    }

    fn next_id_peek(&self) -> u64 {
        self.records
            .iter()
            .map(|record| record.id)
            .max()
            .unwrap_or(0)
            + 1
    }
}

fn derived_record(id: u64, source: &SearchRecord, query: &WithinQuery) -> SearchRecord {
    let mut content = crate::agent_proto::ContentQuery::text(query.pattern.clone());
    content.match_case = query.match_case;
    if query.regex {
        content.mode = crate::agent_proto::TextMode::Regex;
    }
    let mut record = SearchRecord::new(id, &content, source.root.clone(), source.place.clone());
    record.within = Some(source.pattern.clone());
    record
}
