//! The Markdown rendering model.
use std::sync::{Arc, OnceLock};

use crate::textview::{
    parse_markdown, Highlighter, MdAlignment, MdBlock, MdDocument, MdRun, MdRunKind, ThemeKind,
    MAX_NESTING,
};

fn builtin() -> Arc<Highlighter> {
    static SHARED: OnceLock<Arc<Highlighter>> = OnceLock::new();
    Arc::clone(SHARED.get_or_init(|| Arc::new(Highlighter::builtin())))
}

const SAMPLE: &str = "# Titel\n\
\n\
Text **fett** *kursiv* ~~weg~~ `code` [Link](https://x.example)\n\
\n\
- eins\n\
- [ ] offen\n\
- [x] fertig\n\
\x20\x201. tief\n\
\n\
| A | B |\n\
|:--|--:|\n\
| 1 | 2 |\n\
\n\
```rust\n\
fn main() {}\n\
```\n\
\n\
---\n\
\n\
![Bild](bild.png \"T\")\n\
\n\
> Zitat\n\
\n\
Fußnote[^1]\n\
\n\
[^1]: Anmerkung\n\
\n\
<div>roh</div>\n";

fn text_of(runs: &[MdRun]) -> String {
    runs.iter().map(|r| r.text.as_str()).collect()
}

fn find<'a>(doc: &'a MdDocument, pred: impl Fn(&MdBlock) -> bool) -> &'a MdBlock {
    doc.blocks.iter().find(|b| pred(b)).expect("block present")
}

#[test]
fn textview_task_markdown_model() {
    let doc = parse_markdown(SAMPLE, &builtin(), ThemeKind::Light);
    let MdBlock::Heading { level, runs } = &doc.blocks[0] else {
        panic!("heading first: {:?}", doc.blocks[0]);
    };
    assert_eq!((*level, text_of(runs).as_str()), (1, "Titel"));

    let MdBlock::Paragraph { runs } = &doc.blocks[1] else {
        panic!("paragraph: {:?}", doc.blocks[1]);
    };
    let run = |text: &str| runs.iter().find(|r| r.text == text).expect(text).clone();
    assert!(run("fett").strong);
    assert!(run("kursiv").emphasis);
    assert!(run("weg").strikethrough);
    assert_eq!(run("code").kind, MdRunKind::Code);
    assert_eq!(run("Link").link.as_deref(), Some("https://x.example"));

    let MdBlock::List { start, items } = find(&doc, |b| matches!(b, MdBlock::List { .. })) else {
        unreachable!()
    };
    assert_eq!(*start, None);
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].task, None);
    assert_eq!(items[1].task, Some(false));
    assert_eq!(items[2].task, Some(true));
    let nested = items[2]
        .blocks
        .iter()
        .find_map(|b| match b {
            MdBlock::List { start, items } => Some((*start, items.len())),
            _ => None,
        })
        .expect("nested ordered list");
    assert_eq!(nested, (Some(1), 1));

    let MdBlock::Table {
        alignments,
        header,
        rows,
    } = find(&doc, |b| matches!(b, MdBlock::Table { .. }))
    else {
        unreachable!()
    };
    assert_eq!(alignments, &vec![MdAlignment::Left, MdAlignment::Right]);
    assert_eq!(header.len(), 2);
    assert_eq!(text_of(&header[1]), "B");
    assert_eq!(rows.len(), 1);
    assert_eq!(text_of(&rows[0][1]), "2");

    let MdBlock::Code {
        language,
        highlighted_as,
        lines,
    } = find(&doc, |b| matches!(b, MdBlock::Code { .. }))
    else {
        unreachable!()
    };
    assert_eq!(language.as_deref(), Some("rust"));
    assert_eq!(highlighted_as, "Rust");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].text, "fn main() {}");
    assert!(lines[0].spans.len() >= 2, "highlighted: {:?}", lines[0]);

    assert!(doc.blocks.iter().any(|b| matches!(b, MdBlock::Rule)));
    assert!(doc.blocks.contains(&MdBlock::Image {
        url: "bild.png".into(),
        title: "T".into(),
        alt: "Bild".into(),
    }));
    let MdBlock::Quote { blocks, .. } = find(&doc, |b| matches!(b, MdBlock::Quote { .. })) else {
        unreachable!()
    };
    assert!(matches!(&blocks[0], MdBlock::Paragraph { runs } if text_of(runs) == "Zitat"));
    let reference = doc.blocks.iter().any(|b| match b {
        MdBlock::Paragraph { runs } => runs
            .iter()
            .any(|r| r.kind == MdRunKind::FootnoteRef && r.text == "1"),
        _ => false,
    });
    assert!(reference, "footnote reference");
    assert!(doc
        .blocks
        .iter()
        .any(|b| matches!(b, MdBlock::FootnoteDefinition { label, .. } if label == "1")));
    assert!(doc
        .blocks
        .iter()
        .any(|b| matches!(b, MdBlock::Html { text } if text.contains("<div>roh</div>"))));

    // JSON for the Android bridge and back.
    let json = serde_json::to_string(&doc).unwrap();
    assert!(json.contains("\"type\":\"heading\""));
    let back: MdDocument = serde_json::from_str(&json).unwrap();
    assert_eq!(back, doc);
}

fn depth(blocks: &[MdBlock]) -> usize {
    blocks
        .iter()
        .map(|b| match b {
            MdBlock::Quote { blocks, .. } | MdBlock::FootnoteDefinition { blocks, .. } => {
                1 + depth(blocks)
            }
            MdBlock::List { items, .. } => {
                1 + items.iter().map(|i| depth(&i.blocks)).max().unwrap_or(0)
            }
            _ => 0,
        })
        .max()
        .unwrap_or(0)
}

#[test]
fn textview_task_markdown_deep_nesting() {
    let text = format!("{} innen\n", ">".repeat(3000));
    let doc = parse_markdown(&text, &builtin(), ThemeKind::Dark);
    assert!(depth(&doc.blocks) <= MAX_NESTING + 1);
    let json = serde_json::to_string(&doc).unwrap();
    assert!(json.contains("innen"));
}
