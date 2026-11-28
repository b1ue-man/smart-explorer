//! Turns the colour model of the text view core (`StyledLine`, `TokenStyle`,
//! `ThemeColors`) into egui text layouts. The only place where the desktop
//! view meets highlighting: it paints spans, it knows no languages.
use eframe::egui::{self, text::LayoutJob, Color32, FontId, Stroke, TextFormat};

use crate::textview::{StyledLine, TokenStyle};

pub(in crate::app) fn rgba(color: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3])
}

fn format_for(style: &TokenStyle, font: &FontId) -> TextFormat {
    let color = rgba(style.fg);
    TextFormat {
        font_id: font.clone(),
        color,
        background: style.bg.map(rgba).unwrap_or(Color32::TRANSPARENT),
        italics: style.italic,
        underline: if style.underline {
            Stroke::new(1.0, color)
        } else {
            Stroke::NONE
        },
        ..Default::default()
    }
}

/// The layout of one styled line; `marks` (byte ranges) get `mark` as
/// background (search hits, the current match).
pub(in crate::app) fn styled_job(
    line: &StyledLine,
    font: &FontId,
    fallback: Color32,
    marks: &[(usize, usize)],
    mark: Color32,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    let text = line.text.as_str();
    if line.spans.is_empty() {
        let format = TextFormat::simple(font.clone(), fallback);
        append_marked(&mut job, text, 0, &format, marks, mark);
        return job;
    }
    let mut covered = 0;
    for span in &line.spans {
        let (start, end) = (span.start.min(text.len()), span.end.min(text.len()));
        if start < covered
            || start >= end
            || !text.is_char_boundary(start)
            || !text.is_char_boundary(end)
        {
            continue;
        }
        if start > covered {
            let format = TextFormat::simple(font.clone(), fallback);
            append_marked(
                &mut job,
                &text[covered..start],
                covered,
                &format,
                marks,
                mark,
            );
        }
        let format = format_for(&span.style, font);
        append_marked(&mut job, &text[start..end], start, &format, marks, mark);
        covered = end;
    }
    if covered < text.len() {
        let format = TextFormat::simple(font.clone(), fallback);
        append_marked(&mut job, &text[covered..], covered, &format, marks, mark);
    }
    job
}

/// Appends `piece` (which starts at byte `offset` of its line) split at the
/// borders of `marks`.
pub(in crate::app) fn append_marked(
    job: &mut LayoutJob,
    piece: &str,
    offset: usize,
    format: &TextFormat,
    marks: &[(usize, usize)],
    mark: Color32,
) {
    let end = offset + piece.len();
    let mut cursor = offset;
    for &(start, stop) in marks {
        let start = start.clamp(cursor, end);
        let stop = stop.clamp(start, end);
        if stop <= start {
            continue;
        }
        if !piece.is_char_boundary(start - offset) || !piece.is_char_boundary(stop - offset) {
            continue;
        }
        if start > cursor {
            job.append(&piece[cursor - offset..start - offset], 0.0, format.clone());
        }
        let mut marked = format.clone();
        marked.background = mark;
        job.append(&piece[start - offset..stop - offset], 0.0, marked);
        cursor = stop;
    }
    if cursor < end {
        job.append(&piece[cursor - offset..], 0.0, format.clone());
    }
}

/// A plain line in one colour with marks.
pub(in crate::app) fn plain_job(
    text: &str,
    font: &FontId,
    color: Color32,
    marks: &[(usize, usize)],
    mark: Color32,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    append_marked(
        &mut job,
        text,
        0,
        &TextFormat::simple(font.clone(), color),
        marks,
        mark,
    );
    job
}

/// Monospace font of the views.
pub(in crate::app) fn mono(ui: &egui::Ui) -> FontId {
    egui::TextStyle::Monospace.resolve(ui.style())
}
