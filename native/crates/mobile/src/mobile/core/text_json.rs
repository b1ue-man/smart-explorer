//! JSON forms of the text view and the content search for the Kotlin side
//! (api.md §4.14–§4.16): every offset into a text is counted in UTF-16 code
//! units, the index unit of a Kotlin `String`, and every color is a 32-bit
//! ARGB number (`androidx.compose.ui.graphics.Color(Int)`). The core itself
//! counts bytes of UTF-8; the conversion happens only here.
use crate::textview::{MdBlock, MdDocument, StyledLine, ThemeColors, TokenStyle};
use serde_json::{json, Value};

/// Turns byte offsets of one text into UTF-16 offsets, walking forward from
/// the previous answer (offsets asked in ascending order cost one pass).
pub(crate) struct Utf16Cursor<'a> {
    text: &'a str,
    byte: usize,
    unit: usize,
}

impl<'a> Utf16Cursor<'a> {
    pub(crate) fn new(text: &'a str) -> Self {
        Self {
            text,
            byte: 0,
            unit: 0,
        }
    }

    /// The UTF-16 offset of the byte offset `byte`, clamped to the text and
    /// moved back to the start of a character that it falls into.
    pub(crate) fn at(&mut self, byte: usize) -> usize {
        let byte = floor_boundary(self.text, byte);
        if byte < self.byte {
            self.byte = 0;
            self.unit = 0;
        }
        self.unit += self.text[self.byte..byte].encode_utf16().count();
        self.byte = byte;
        self.unit
    }
}

/// `byte` clamped to `text` and moved back to a character boundary.
pub(crate) fn floor_boundary(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while byte > 0 && !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}

/// The byte offset of the UTF-16 offset `unit`; an offset inside a
/// surrogate pair names the character it belongs to, one past the end the end.
pub(crate) fn byte_at_utf16(text: &str, unit: usize) -> usize {
    let mut units = 0;
    for (index, ch) in text.char_indices() {
        if units >= unit {
            return index;
        }
        units += ch.len_utf16();
        if units > unit {
            return index;
        }
    }
    text.len()
}

/// RGBA bytes as one ARGB number.
pub(crate) fn argb(rgba: [u8; 4]) -> u32 {
    let [r, g, b, a] = rgba;
    (u32::from(a) << 24) | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
}

fn style_fields(start: usize, end: usize, style: &TokenStyle) -> Value {
    json!({
        "start": start,
        "end": end,
        "fg": argb(style.fg),
        "bg": style.bg.map(argb),
        "bold": style.bold,
        "italic": style.italic,
        "underline": style.underline,
    })
}

/// `StyledLine {number?, text, spans:[{start, end, fg, bg?, bold, italic,
/// underline}]}` with the spans in UTF-16 offsets of `text`.
pub(crate) fn styled_line_json(number: Option<u64>, line: &StyledLine) -> Value {
    let mut cursor = Utf16Cursor::new(&line.text);
    let spans: Vec<Value> = line
        .spans
        .iter()
        .map(|span| {
            let start = cursor.at(span.start);
            let end = cursor.at(span.end);
            style_fields(start, end, &span.style)
        })
        .collect();
    let mut value = json!({ "text": line.text, "spans": spans });
    if let Some(number) = number {
        value["number"] = json!(number);
    }
    value
}

/// The view colors around the text.
pub(crate) fn colors_json(colors: &ThemeColors) -> Value {
    json!({
        "theme": colors.name,
        "background": argb(colors.background),
        "foreground": argb(colors.foreground),
        "selection": argb(colors.selection),
        "lineHighlight": argb(colors.line_highlight),
        "gutter": argb(colors.gutter_foreground),
    })
}

/// Byte ranges of `text` as `[[start, end], …]` in UTF-16 offsets.
pub(crate) fn ranges_json(text: &str, ranges: &[(u32, u32)]) -> Value {
    let mut cursor = Utf16Cursor::new(text);
    Value::Array(
        ranges
            .iter()
            .map(|&(start, end)| {
                let start = cursor.at(start as usize);
                let end = cursor.at(end as usize);
                json!([start, end])
            })
            .collect(),
    )
}

/// `MdDocument` as the core's serde form (`type` tags in snake_case), except
/// that the lines of code blocks are `StyledLine`s in UTF-16 offsets and the
/// language a block was highlighted as is `highlightedAs`.
pub(crate) fn markdown_json(document: &MdDocument) -> Value {
    json!({ "blocks": blocks_json(&document.blocks) })
}

fn blocks_json(blocks: &[MdBlock]) -> Vec<Value> {
    blocks.iter().map(block_json).collect()
}

fn block_json(block: &MdBlock) -> Value {
    match block {
        MdBlock::Code {
            language,
            highlighted_as,
            lines,
        } => json!({
            "type": "code",
            "language": language,
            "highlightedAs": highlighted_as,
            "lines": lines.iter().map(|line| styled_line_json(None, line)).collect::<Vec<_>>(),
        }),
        MdBlock::List { start, items } => json!({
            "type": "list",
            "start": start,
            "items": items
                .iter()
                .map(|item| json!({ "task": item.task, "blocks": blocks_json(&item.blocks) }))
                .collect::<Vec<_>>(),
        }),
        MdBlock::Quote { kind, blocks } => json!({
            "type": "quote",
            "kind": kind.map(|kind| serde_json::to_value(kind).unwrap_or(Value::Null)),
            "blocks": blocks_json(blocks),
        }),
        MdBlock::FootnoteDefinition { label, blocks } => json!({
            "type": "footnote_definition",
            "label": label,
            "blocks": blocks_json(blocks),
        }),
        // Headings, paragraphs, tables, rules, images and HTML carry only
        // runs and text: their serde form is the bridge form.
        other => serde_json::to_value(other).unwrap_or(Value::Null),
    }
}

#[cfg(test)]
#[path = "text_json_tests.rs"]
mod tests;
