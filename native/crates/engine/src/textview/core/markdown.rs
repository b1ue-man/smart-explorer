//! Markdown to [`MdDocument`] with pulldown-cmark (CommonMark plus tables,
//! task lists, strikethrough, footnotes and GitHub alerts). Code blocks come
//! highlighted by the shared [`Highlighter`]. Containers nested deeper than
//! [`MAX_NESTING`] are flattened into their parent, so hostile input cannot
//! exhaust the stack; inline styles are tracked without recursion.
use std::cell::RefCell;
use std::iter::Peekable;
use std::sync::Arc;

use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
};

use super::highlight::{Highlighter, PLAIN_TEXT};
use super::markdown_model::{
    MdAlignment, MdBlock, MdDocument, MdListItem, MdQuoteKind, MdRun, MdRunKind,
};
use super::style::ThemeKind;

/// Deepest nesting of quotes, lists and footnotes kept as structure.
pub const MAX_NESTING: usize = 32;

type Events<'a> = Peekable<Parser<'a>>;

struct Cx<'h> {
    highlighter: &'h Arc<Highlighter>,
    theme: ThemeKind,
    /// One slot per open list item for its task marker.
    task_slots: RefCell<Vec<Option<bool>>>,
}

/// Parses Markdown text into the rendering model.
pub fn parse_markdown(text: &str, highlighter: &Arc<Highlighter>, theme: ThemeKind) -> MdDocument {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_GFM;
    let mut events = Parser::new_ext(text, options).peekable();
    let cx = Cx {
        highlighter,
        theme,
        task_slots: RefCell::new(Vec::new()),
    };
    let mut blocks = Vec::new();
    // A stray end event at the top level cannot occur; keep going anyway.
    while events.peek().is_some() {
        blocks.extend(parse_blocks(&mut events, &cx, 0));
    }
    MdDocument { blocks }
}

fn is_inline_start(tag: &Tag<'_>) -> bool {
    matches!(
        tag,
        Tag::Emphasis
            | Tag::Strong
            | Tag::Strikethrough
            | Tag::Superscript
            | Tag::Subscript
            | Tag::Link { .. }
            | Tag::Image { .. }
    )
}

/// Blocks up to the end event of the enclosing container (consumed).
fn parse_blocks(events: &mut Events<'_>, cx: &Cx<'_>, depth: usize) -> Vec<MdBlock> {
    let mut blocks = Vec::new();
    let mut flattened = 0usize;
    loop {
        let Some(event) = events.peek() else {
            return blocks;
        };
        match event {
            Event::End(_) => {
                events.next();
                if flattened == 0 {
                    return blocks;
                }
                flattened -= 1;
            }
            Event::Rule => {
                events.next();
                blocks.push(MdBlock::Rule);
            }
            Event::Html(_) => {
                let mut text = String::new();
                while let Some(Event::Html(part)) = events.peek() {
                    text.push_str(part);
                    events.next();
                }
                blocks.push(MdBlock::Html { text });
            }
            Event::Start(tag) if !is_inline_start(tag) => {
                let Some(Event::Start(tag)) = events.next() else {
                    continue;
                };
                match tag {
                    Tag::Paragraph => {
                        let runs = parse_inlines(events, cx, true);
                        blocks.push(paragraph(runs));
                    }
                    Tag::Heading { level, .. } => {
                        let runs = parse_inlines(events, cx, true);
                        blocks.push(MdBlock::Heading {
                            level: heading_level(level),
                            runs,
                        });
                    }
                    Tag::CodeBlock(kind) => blocks.push(code_block(events, cx, kind)),
                    Tag::HtmlBlock => {
                        let text = collect_text(events);
                        blocks.push(MdBlock::Html { text });
                    }
                    Tag::Table(alignments) => blocks.push(table(events, cx, alignments)),
                    _ if depth >= MAX_NESTING => flattened += 1,
                    Tag::BlockQuote(kind) => {
                        let inner = parse_blocks(events, cx, depth + 1);
                        blocks.push(MdBlock::Quote {
                            kind: kind.map(quote_kind),
                            blocks: inner,
                        });
                    }
                    Tag::List(start) => {
                        let items = list_items(events, cx, depth + 1);
                        blocks.push(MdBlock::List { start, items });
                    }
                    Tag::Item => {
                        let item = list_item(events, cx, depth + 1);
                        blocks.push(MdBlock::List {
                            start: None,
                            items: vec![item],
                        });
                    }
                    Tag::FootnoteDefinition(label) => {
                        let inner = parse_blocks(events, cx, depth + 1);
                        blocks.push(MdBlock::FootnoteDefinition {
                            label: label.to_string(),
                            blocks: inner,
                        });
                    }
                    // Not enabled (definition lists, metadata) or unknown:
                    // its content stays in place.
                    _ => flattened += 1,
                }
            }
            // Inline content outside a paragraph (tight list items). Every
            // event reaching here is inline, so at least one is consumed.
            _ => {
                let runs = parse_inlines(events, cx, false);
                if !runs.is_empty() {
                    blocks.push(paragraph(runs));
                }
            }
        }
    }
}

fn list_items(events: &mut Events<'_>, cx: &Cx<'_>, depth: usize) -> Vec<MdListItem> {
    let mut items = Vec::new();
    while let Some(event) = events.next() {
        match event {
            Event::Start(Tag::Item) => items.push(list_item(events, cx, depth)),
            Event::End(_) => break,
            _ => {}
        }
    }
    items
}

fn list_item(events: &mut Events<'_>, cx: &Cx<'_>, depth: usize) -> MdListItem {
    cx.task_slots.borrow_mut().push(None);
    let blocks = parse_blocks(events, cx, depth);
    let task = cx.task_slots.borrow_mut().pop().flatten();
    MdListItem { task, blocks }
}

fn paragraph(runs: Vec<MdRun>) -> MdBlock {
    let mut visible = runs
        .iter()
        .filter(|r| !(matches!(r.kind, MdRunKind::Text) && r.text.trim().is_empty()));
    if let (Some(only), None) = (visible.next(), visible.next()) {
        if let MdRunKind::Image { url, title } = &only.kind {
            return MdBlock::Image {
                url: url.clone(),
                title: title.clone(),
                alt: only.text.clone(),
            };
        }
    }
    MdBlock::Paragraph { runs }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn quote_kind(kind: BlockQuoteKind) -> MdQuoteKind {
    match kind {
        BlockQuoteKind::Note => MdQuoteKind::Note,
        BlockQuoteKind::Tip => MdQuoteKind::Tip,
        BlockQuoteKind::Important => MdQuoteKind::Important,
        BlockQuoteKind::Warning => MdQuoteKind::Warning,
        BlockQuoteKind::Caution => MdQuoteKind::Caution,
    }
}

/// Text events up to the end of the current block (consumed).
fn collect_text(events: &mut Events<'_>) -> String {
    let mut text = String::new();
    for event in events.by_ref() {
        match event {
            Event::End(_) => break,
            Event::Text(part) | Event::Html(part) | Event::Code(part) => text.push_str(&part),
            _ => {}
        }
    }
    text
}

fn code_block(events: &mut Events<'_>, cx: &Cx<'_>, kind: CodeBlockKind<'_>) -> MdBlock {
    let language = match kind {
        CodeBlockKind::Fenced(info) => info
            .split(|c: char| c.is_whitespace() || c == ',' || c == '{')
            .next()
            .filter(|word| !word.is_empty())
            .map(str::to_string),
        CodeBlockKind::Indented => None,
    };
    let text = collect_text(events);
    let highlighted_as = language
        .as_deref()
        .and_then(|name| cx.highlighter.language_by_name(name))
        .unwrap_or_else(|| PLAIN_TEXT.to_string());
    let lines = cx
        .highlighter
        .highlight_text(&highlighted_as, &text, cx.theme);
    MdBlock::Code {
        language,
        highlighted_as,
        lines,
    }
}

fn table(events: &mut Events<'_>, cx: &Cx<'_>, alignments: Vec<Alignment>) -> MdBlock {
    let alignments = alignments
        .into_iter()
        .map(|a| match a {
            Alignment::None => MdAlignment::None,
            Alignment::Left => MdAlignment::Left,
            Alignment::Center => MdAlignment::Center,
            Alignment::Right => MdAlignment::Right,
        })
        .collect();
    let mut header = Vec::new();
    let mut rows: Vec<Vec<Vec<MdRun>>> = Vec::new();
    let mut in_head = false;
    while let Some(event) = events.next() {
        match event {
            Event::Start(Tag::TableHead) => in_head = true,
            Event::End(TagEnd::TableHead) => in_head = false,
            Event::Start(Tag::TableRow) if !in_head => rows.push(Vec::new()),
            Event::Start(Tag::TableCell) => {
                let cell = parse_inlines(events, cx, true);
                match (in_head, rows.last_mut()) {
                    (false, Some(row)) => row.push(cell),
                    _ => header.push(cell),
                }
            }
            Event::End(TagEnd::Table) => break,
            _ => {}
        }
    }
    MdBlock::Table {
        alignments,
        header,
        rows,
    }
}

#[derive(Default)]
struct InlineState {
    strong: u32,
    emphasis: u32,
    strike: u32,
    links: Vec<String>,
    /// Open images: url, title, alternative text so far.
    images: Vec<(String, String, String)>,
    /// Open inline tags (their end events belong to this run list).
    open: Vec<TagEnd>,
}

/// Inline runs. `explicit`: up to the end of the enclosing paragraph,
/// heading or cell (consumed); otherwise while inline events follow.
fn parse_inlines(events: &mut Events<'_>, cx: &Cx<'_>, explicit: bool) -> Vec<MdRun> {
    let mut runs: Vec<MdRun> = Vec::new();
    let mut state = InlineState::default();
    loop {
        let Some(event) = events.peek() else {
            return runs;
        };
        let stop = match event {
            Event::End(_) => state.open.is_empty(),
            Event::Start(tag) => !explicit && !is_inline_start(tag),
            Event::Rule | Event::Html(_) => !explicit,
            _ => false,
        };
        if stop {
            if explicit {
                events.next();
            }
            return runs;
        }
        let Some(event) = events.next() else {
            return runs;
        };
        match event {
            Event::Text(text) | Event::InlineMath(text) | Event::DisplayMath(text) => {
                if let Some(image) = state.images.last_mut() {
                    image.2.push_str(&text);
                } else {
                    push_run(&mut runs, &state, text.to_string(), MdRunKind::Text);
                }
            }
            Event::Code(text) => push_run(&mut runs, &state, text.to_string(), MdRunKind::Code),
            Event::Html(text) | Event::InlineHtml(text) => {
                push_run(&mut runs, &state, text.to_string(), MdRunKind::Html)
            }
            Event::SoftBreak => push_run(&mut runs, &state, " ".to_string(), MdRunKind::Text),
            Event::HardBreak => push_run(&mut runs, &state, "\n".to_string(), MdRunKind::LineBreak),
            Event::FootnoteReference(label) => {
                push_run(&mut runs, &state, label.to_string(), MdRunKind::FootnoteRef)
            }
            Event::TaskListMarker(checked) => {
                if let Some(slot) = cx.task_slots.borrow_mut().last_mut() {
                    if slot.is_none() {
                        *slot = Some(checked);
                    }
                }
            }
            Event::Start(tag) => {
                state.open.push(tag.to_end());
                match tag {
                    Tag::Strong => state.strong += 1,
                    Tag::Emphasis => state.emphasis += 1,
                    Tag::Strikethrough => state.strike += 1,
                    Tag::Link { dest_url, .. } => state.links.push(dest_url.to_string()),
                    Tag::Image {
                        dest_url, title, ..
                    } => {
                        state
                            .images
                            .push((dest_url.to_string(), title.to_string(), String::new()))
                    }
                    _ => {}
                }
            }
            Event::End(end) => {
                state.open.pop();
                match end {
                    TagEnd::Strong => state.strong = state.strong.saturating_sub(1),
                    TagEnd::Emphasis => state.emphasis = state.emphasis.saturating_sub(1),
                    TagEnd::Strikethrough => state.strike = state.strike.saturating_sub(1),
                    TagEnd::Link => {
                        state.links.pop();
                    }
                    TagEnd::Image => {
                        if let Some((url, title, alt)) = state.images.pop() {
                            if let Some(outer) = state.images.last_mut() {
                                outer.2.push_str(&alt);
                            } else {
                                push_run(&mut runs, &state, alt, MdRunKind::Image { url, title });
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::Rule => {}
        }
    }
}

fn push_run(runs: &mut Vec<MdRun>, state: &InlineState, text: String, kind: MdRunKind) {
    let run = MdRun {
        text,
        kind,
        strong: state.strong > 0,
        emphasis: state.emphasis > 0,
        strikethrough: state.strike > 0,
        link: state.links.last().cloned(),
    };
    if let Some(last) = runs.last_mut() {
        let same = matches!((&last.kind, &run.kind), (MdRunKind::Text, MdRunKind::Text))
            && last.strong == run.strong
            && last.emphasis == run.emphasis
            && last.strikethrough == run.strikethrough
            && last.link == run.link;
        if same {
            last.text.push_str(&run.text);
            return;
        }
    }
    runs.push(run);
}
