//! Original-byte sessions and owned decisions for the recorded merge consumer.
use crate::{bisync::{Conflict, MergeFailure, MergeReport, StateKey}, linemerge::{Row, TextShape}, vfs::BackendHandle};

pub(in crate::app) struct MergeSession {
    pub a: BackendHandle, pub root_a: String, pub b: BackendHandle, pub root_b: String,
    pub key: StateKey, pub conflict: Conflict,
    pub original_a: Option<Vec<u8>>, pub original_b: Option<Vec<u8>>,
}
pub(in crate::app) enum MergeDecision { Write(Vec<u8>), Rows(TextShape), KeepBoth { keep_a: bool } }
pub(in crate::app) struct MergeUi {
    pub rel: String, pub rows: Vec<Row>, pub session: Option<MergeSession>,
    pub shape_a: TextShape, pub shape_b: TextShape, pub shape: TextShape,
    pub text_error: Option<String>, pub retry: Option<MergeDecision>, pub last_error: Option<String>,
}
impl MergeUi {
    pub fn loading(rel: String) -> Self {
        let shape = TextShape::of("");
        Self { rel, rows:Vec::new(), session:None, shape_a:shape, shape_b:shape, shape,
            text_error:None, retry:None, last_error:None }
    }
}
pub(in crate::app) struct MergeApplyResult {
    pub ui: MergeUi, pub result: Result<MergeReport, MergeFailure>,
}

/// Mixed separators cannot be represented by TextShape; keep-both remains raw.
pub(in crate::app) fn merge_text(bytes: Option<&[u8]>) -> Result<&str, String> {
    let text = std::str::from_utf8(bytes.unwrap_or_default())
        .map_err(|_| "Keine UTF-8-Textdatei; beide Originaldateien können getrennt erhalten bleiben.".to_string())?;
    if text.contains('\0') { return Err("Binärinhalt; bitte beide Originaldateien getrennt erhalten.".into()); }
    let lf = text.matches('\n').count();
    let crlf = text.matches("\r\n").count();
    if (crlf > 0 && crlf != lf) || text.replace("\r\n", "").contains('\r') {
        return Err("Gemischte Zeilenumbrüche; bitte beide Originaldateien getrennt erhalten.".into());
    }
    Ok(text)
}

/// Clip display work only; saved rows and original bytes are never truncated.
pub(in crate::app) fn line_display(text: &str) -> String {
    let mut chars = text.chars();
    let mut result: String = chars.by_ref().take(2048).collect();
    if chars.next().is_some() { result.push('…'); }
    result
}

pub(in crate::app) fn assemble_text(rows:&[Row], shape:TextShape) -> Vec<u8> {
    let joined = crate::linemerge::assemble_rows(rows);
    // TextShape normally adds the final separator only to nonempty strings.
    // One retained empty line is still content, unlike excluding every row.
    let text = if joined.is_empty() && rows.iter().any(|row| row.equal
        || (row.take_left && row.left.is_some()) || (row.take_right && row.right.is_some())) {
        shape.apply("x".into()).strip_prefix('x').unwrap_or_default().to_string()
    } else { shape.apply(joined) };
    text.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_merge_preserves_crlf_final_separator_and_empty_line() {
        for original in ["one\r\ntwo\r\n", "one\ntwo", "\r\n", "\n", ""] {
            let rows = crate::linemerge::rows(original,original).unwrap();
            assert_eq!(assemble_text(&rows,TextShape::of(original)),original.as_bytes());
        }
    }
    #[test]
    fn desktop_merge_excluding_all_lines_does_not_create_newline() {
        let row = Row { left:Some(String::new()),right:None,equal:false,take_left:false,take_right:false };
        assert!(assemble_text(&[row],TextShape::of("\n")).is_empty());
    }
    #[test]
    fn desktop_merge_rejects_lossy_and_mixed_text_without_changing_bytes() {
        for bytes in [vec![0xff,0xfe,0x00],b"a\r\nb\nc".to_vec(),b"a\rb".to_vec()] {
            let before = bytes.clone();
            assert!(merge_text(Some(&bytes)).is_err());
            assert_eq!(bytes,before);
        }
    }
}
