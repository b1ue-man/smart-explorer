//! Bounded line-level two-way merge used to resolve sync conflicts. Myers'
//! algorithm groups the versions into equal runs and change blocks ("hunks")
//! without allocating the old quadratic LCS table. Operational entry points
//! also enforce input limits and a deadline so the UI can fail closed instead
//! of presenting an approximate diff.

use similar::{Algorithm, ChangeTag, TextDiff};
use std::fmt;
use std::time::{Duration, Instant};

const MAX_INPUT_BYTES_PER_SIDE: usize = 16 * 1024 * 1024;
const MAX_TOTAL_LINES: usize = 500_000;
const DEFAULT_DIFF_TIMEOUT: Duration = Duration::from_secs(2);
const DEADLINE_CHECK_INTERVAL: usize = 1_024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LineMergeError {
    InputTooLarge {
        side: char,
        actual: usize,
        limit: usize,
    },
    TooManyLines {
        actual: usize,
        limit: usize,
    },
    TimedOut {
        limit: Duration,
    },
    InvalidTimeout,
}

impl fmt::Display for LineMergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InputTooLarge {
                side,
                actual,
                limit,
            } => write!(
                f,
                "Seite {side} ist mit {actual} Bytes zu groß (Limit: {limit} Bytes)."
            ),
            Self::TooManyLines { actual, limit } => write!(
                f,
                "Der Zeilenvergleich enthält {actual} Zeilen (Limit: {limit})."
            ),
            Self::TimedOut { limit } => write!(
                f,
                "Der Zeilenvergleich wurde nach {} ms abgebrochen; ein angenähertes Ergebnis wird nicht angezeigt.",
                limit.as_millis()
            ),
            Self::InvalidTimeout => write!(f, "Das Zeitlimit für den Zeilenvergleich ist ungültig."),
        }
    }
}

impl std::error::Error for LineMergeError {}

#[derive(Clone, Copy)]
struct MergeDeadline {
    at: Instant,
    limit: Duration,
}

impl MergeDeadline {
    fn new(limit: Duration) -> Result<Self, LineMergeError> {
        if limit.is_zero() {
            return Err(LineMergeError::TimedOut { limit });
        }
        let at = Instant::now()
            .checked_add(limit)
            .ok_or(LineMergeError::InvalidTimeout)?;
        Ok(Self { at, limit })
    }

    fn check(self) -> Result<(), LineMergeError> {
        if Instant::now() >= self.at {
            Err(LineMergeError::TimedOut { limit: self.limit })
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg(test)]
pub enum Choice {
    A,
    B,
    Both,
    Neither,
}

#[derive(Clone, Debug)]
pub struct Hunk {
    /// True when both sides are identical here (always kept as-is).
    pub equal: bool,
    pub a: Vec<String>,
    pub b: Vec<String>,
    /// For change hunks: which side(s) to keep. Ignored when `equal`.
    #[cfg(test)]
    pub choice: Choice,
}

/// Deadline-aware Myers line diff of `a` vs `b` → hunks (equal runs +
/// change blocks). The default choice for a change block is A (source/left).
pub fn diff(a: &str, b: &str) -> Result<Vec<Hunk>, LineMergeError> {
    let (hunks, _) = timed_hunks(a, b, DEFAULT_DIFF_TIMEOUT)?;
    Ok(hunks)
}

fn timed_hunks(
    a: &str,
    b: &str,
    timeout: Duration,
) -> Result<(Vec<Hunk>, MergeDeadline), LineMergeError> {
    let deadline = MergeDeadline::new(timeout)?;
    validate_input_size('A', a)?;
    validate_input_size('B', b)?;
    let a_line_count = a.lines().count();
    let b_line_count = b.lines().count();
    let total_lines =
        a_line_count
            .checked_add(b_line_count)
            .ok_or(LineMergeError::TooManyLines {
                actual: usize::MAX,
                limit: MAX_TOTAL_LINES,
            })?;
    if total_lines > MAX_TOTAL_LINES {
        return Err(LineMergeError::TooManyLines {
            actual: total_lines,
            limit: MAX_TOTAL_LINES,
        });
    }
    deadline.check()?;

    let al: Vec<&str> = a.lines().collect();
    let bl: Vec<&str> = b.lines().collect();
    let mut config = TextDiff::configure();
    config.algorithm(Algorithm::Myers).deadline(deadline.at);
    let line_diff = config.diff_slices(&al, &bl);
    // `similar` deliberately returns an approximation when its deadline is
    // reached. Reject it before exposing any choices to the user.
    deadline.check()?;

    let mut hunks: Vec<Hunk> = Vec::new();
    let mut eq: Vec<String> = Vec::new();
    let mut ca: Vec<String> = Vec::new();
    let mut cb: Vec<String> = Vec::new();

    fn flush_change(hunks: &mut Vec<Hunk>, ca: &mut Vec<String>, cb: &mut Vec<String>) {
        if !ca.is_empty() || !cb.is_empty() {
            hunks.push(Hunk {
                equal: false,
                a: std::mem::take(ca),
                b: std::mem::take(cb),
                #[cfg(test)]
                choice: Choice::A,
            });
        }
    }
    fn flush_eq(hunks: &mut Vec<Hunk>, eq: &mut Vec<String>) {
        if !eq.is_empty() {
            let v = std::mem::take(eq);
            hunks.push(Hunk {
                equal: true,
                a: v.clone(),
                b: v,
                #[cfg(test)]
                choice: Choice::Both,
            });
        }
    }

    for (index, change) in line_diff.iter_all_changes().enumerate() {
        if index % DEADLINE_CHECK_INTERVAL == 0 {
            deadline.check()?;
        }
        match change.tag() {
            ChangeTag::Equal => {
                flush_change(&mut hunks, &mut ca, &mut cb);
                eq.push(change.value().to_string());
            }
            ChangeTag::Delete => {
                flush_eq(&mut hunks, &mut eq);
                ca.push(change.value().to_string());
            }
            ChangeTag::Insert => {
                flush_eq(&mut hunks, &mut eq);
                cb.push(change.value().to_string());
            }
        }
    }
    flush_eq(&mut hunks, &mut eq);
    flush_change(&mut hunks, &mut ca, &mut cb);
    deadline.check()?;
    Ok((hunks, deadline))
}

fn validate_input_size(side: char, value: &str) -> Result<(), LineMergeError> {
    if value.len() > MAX_INPUT_BYTES_PER_SIDE {
        Err(LineMergeError::InputTooLarge {
            side,
            actual: value.len(),
            limit: MAX_INPUT_BYTES_PER_SIDE,
        })
    } else {
        Ok(())
    }
}

/// One aligned row of a side-by-side (git-style) diff: the A line and the B line
/// shown next to each other, with independent "include this side" toggles. A
/// `None` side is a gap (the line exists only on the other side).
#[derive(Clone, Debug)]
pub struct Row {
    pub left: Option<String>,
    pub right: Option<String>,
    /// Both sides identical at this row (always kept; toggles ignored).
    pub equal: bool,
    pub take_left: bool,
    pub take_right: bool,
}

/// Build an aligned side-by-side view of `a` vs `b`. Equal lines line up; within
/// a change block, A and B lines are paired by position (extra lines on the
/// longer side get a gap on the other). Defaults: keep the side(s) that have
/// content, preferring A when both differ.
pub fn rows(a: &str, b: &str) -> Result<Vec<Row>, LineMergeError> {
    rows_with_timeout(a, b, DEFAULT_DIFF_TIMEOUT)
}

/// Build rows with an explicit limit. This is public so non-GUI callers can
/// choose a tighter budget while retaining the same fail-closed semantics.
pub fn rows_with_timeout(a: &str, b: &str, timeout: Duration) -> Result<Vec<Row>, LineMergeError> {
    let (hunks, deadline) = timed_hunks(a, b, timeout)?;
    let mut rows = Vec::new();
    let mut row_count = 0usize;
    for h in hunks {
        if h.equal {
            for l in h.a {
                if row_count.is_multiple_of(DEADLINE_CHECK_INTERVAL) {
                    deadline.check()?;
                }
                rows.push(Row {
                    left: Some(l.clone()),
                    right: Some(l),
                    equal: true,
                    take_left: true,
                    take_right: false,
                });
                row_count += 1;
            }
        } else {
            let n = h.a.len().max(h.b.len());
            for i in 0..n {
                if row_count.is_multiple_of(DEADLINE_CHECK_INTERVAL) {
                    deadline.check()?;
                }
                let left = h.a.get(i).cloned();
                let right = h.b.get(i).cloned();
                let (tl, tr) = match (&left, &right) {
                    (Some(_), _) => (true, false),
                    (None, Some(_)) => (false, true),
                    _ => (false, false),
                };
                rows.push(Row {
                    left,
                    right,
                    equal: false,
                    take_left: tl,
                    take_right: tr,
                });
                row_count += 1;
            }
        }
    }
    deadline.check()?;
    Ok(rows)
}

/// The full A (left/source) version reconstructed from the aligned rows.
pub fn side_a(rows: &[Row]) -> String {
    rows.iter()
        .filter_map(|r| r.left.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The full B (right/target) version reconstructed from the aligned rows.
pub fn side_b(rows: &[Row]) -> String {
    rows.iter()
        .filter_map(|r| r.right.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Rebuild the merged text from per-row choices (equal rows always contribute).
pub fn assemble_rows(rows: &[Row]) -> String {
    let mut out: Vec<String> = Vec::new();
    for r in rows {
        if r.equal {
            if let Some(l) = &r.left {
                out.push(l.clone());
            }
            continue;
        }
        if r.take_left {
            if let Some(l) = &r.left {
                out.push(l.clone());
            }
        }
        if r.take_right {
            if let Some(l) = &r.right {
                out.push(l.clone());
            }
        }
    }
    out.join("\n")
}

/// Rebuild the merged text from the hunks' choices.
#[cfg(test)]
pub fn assemble(hunks: &[Hunk]) -> String {
    let mut out: Vec<String> = Vec::new();
    for h in hunks {
        if h.equal {
            out.extend(h.a.iter().cloned());
            continue;
        }
        match h.choice {
            Choice::A => out.extend(h.a.iter().cloned()),
            Choice::B => out.extend(h.b.iter().cloned()),
            Choice::Both => {
                out.extend(h.a.iter().cloned());
                out.extend(h.b.iter().cloned());
            }
            Choice::Neither => {}
        }
    }
    out.join("\n")
}

#[cfg(test)]
#[path = "linemerge_tests.rs"]
mod tests;
