//! UTF-16 offsets and ARGB colors of the bridge's text JSON (api.md §4.14,
//! §4.15): umlauts take one UTF-16 unit but two bytes, emoji two units but
//! four bytes.
use super::*;
use crate::textview::{MdRun, MdRunKind, StyledSpan};

fn style(fg: [u8; 4]) -> TokenStyle {
    TokenStyle {
        fg,
        bg: None,
        bold: false,
        italic: false,
        underline: false,
    }
}

#[test]
fn textview_bridge_task_spans_are_utf16_offsets() {
    // "Grüße 😀 da": G r ü ß e = 7 bytes / 5 units, space, emoji 4 bytes / 2 units.
    let text = "Grüße 😀 da".to_string();
    let emoji = text.find('😀').expect("emoji");
    let after = emoji + '😀'.len_utf8();
    let line = StyledLine {
        text: text.clone(),
        spans: vec![
            StyledSpan {
                start: 0,
                end: emoji,
                style: style([0x11, 0x22, 0x33, 0xff]),
            },
            StyledSpan {
                start: emoji,
                end: after,
                style: TokenStyle {
                    bg: Some([1, 2, 3, 4]),
                    bold: true,
                    ..style([0, 0, 0, 0x80])
                },
            },
            StyledSpan {
                start: after,
                end: text.len(),
                style: style([0xff, 0, 0, 0xff]),
            },
        ],
    };
    let value = styled_line_json(Some(7), &line);
    assert_eq!(value["number"], 7);
    assert_eq!(value["text"], text);
    let spans = value["spans"].as_array().expect("spans");
    assert_eq!(
        (spans[0]["start"].as_u64(), spans[0]["end"].as_u64()),
        (Some(0), Some(6))
    );
    assert_eq!(
        (spans[1]["start"].as_u64(), spans[1]["end"].as_u64()),
        (Some(6), Some(8))
    );
    assert_eq!(
        (spans[2]["start"].as_u64(), spans[2]["end"].as_u64()),
        (Some(8), Some(11))
    );
    // The Kotlin length of the text is the end of the last span.
    assert_eq!(text.encode_utf16().count(), 11);
    assert_eq!(spans[0]["fg"].as_u64(), Some(0xff11_2233));
    assert!(spans[0]["bg"].is_null());
    assert_eq!(spans[1]["fg"].as_u64(), Some(0x8000_0000));
    assert_eq!(spans[1]["bg"].as_u64(), Some(0x0401_0203));
    assert_eq!(spans[1]["bold"], true);
    // Without a number (Markdown code lines) the field is absent.
    assert!(styled_line_json(None, &line).get("number").is_none());
}

#[test]
fn textview_bridge_task_utf16_cursor_and_back() {
    let text = "äb😀c";
    let mut cursor = Utf16Cursor::new(text);
    assert_eq!(cursor.at(0), 0);
    assert_eq!(cursor.at(2), 1); // after ä
    assert_eq!(cursor.at(3), 2); // after b
    assert_eq!(cursor.at(5), 2); // inside the emoji: its start
    assert_eq!(cursor.at(7), 4); // after the emoji
    assert_eq!(cursor.at(1), 0); // backwards, inside ä
    assert_eq!(cursor.at(99), 5); // clamped to the end
    assert_eq!(byte_at_utf16(text, 0), 0);
    assert_eq!(byte_at_utf16(text, 1), 2);
    assert_eq!(byte_at_utf16(text, 2), 3);
    assert_eq!(byte_at_utf16(text, 3), 3); // inside the pair: the emoji
    assert_eq!(byte_at_utf16(text, 4), 7);
    assert_eq!(byte_at_utf16(text, 5), 8);
    assert_eq!(byte_at_utf16(text, 50), 8);
    assert_eq!(floor_boundary(text, 1), 0);
}

#[test]
fn textview_bridge_task_hit_ranges_and_markdown_code_lines() {
    // "Größe" twice; the second hit starts after "Größe und ".
    let text = "Größe und Größe";
    let second = text.rfind("Größe").expect("second") as u32;
    let ranges = ranges_json(text, &[(0, 7), (second, second + 7)]);
    assert_eq!(ranges, json!([[0, 5], [10, 15]]));

    let code = StyledLine {
        text: "let ü = 1;".into(),
        spans: vec![StyledSpan {
            start: 0,
            end: "let ü = 1;".len(),
            style: style([0, 0, 0, 0xff]),
        }],
    };
    let document = MdDocument {
        blocks: vec![
            MdBlock::Heading {
                level: 1,
                runs: vec![MdRun {
                    text: "Titel".into(),
                    kind: MdRunKind::Text,
                    ..MdRun::default()
                }],
            },
            MdBlock::Quote {
                kind: None,
                blocks: vec![MdBlock::Code {
                    language: Some("rs".into()),
                    highlighted_as: "Rust".into(),
                    lines: vec![code],
                }],
            },
            MdBlock::Rule,
        ],
    };
    let value = markdown_json(&document);
    let blocks = value["blocks"].as_array().expect("blocks");
    assert_eq!(blocks[0]["type"], "heading");
    assert_eq!(blocks[0]["runs"][0]["text"], "Titel");
    assert_eq!(blocks[0]["runs"][0]["kind"]["type"], "text");
    let nested = &blocks[1]["blocks"][0];
    assert_eq!(nested["type"], "code");
    assert_eq!(nested["highlightedAs"], "Rust");
    assert_eq!(nested["lines"][0]["spans"][0]["end"], 10);
    assert_eq!(blocks[2]["type"], "rule");
}
