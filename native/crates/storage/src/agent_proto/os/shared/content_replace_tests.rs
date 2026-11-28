//! The replacement of one file's content (`ReplaceProgram`): Notepad++'s
//! replacement syntax, the matches of the search, the selection of hits, the
//! encodings and line ends that stay, the previews of a dry run.
use super::content_query::{ContentQuery, TextMode};
use super::content_replace::{HitRef, Pick, ReplaceProgram, ReplaceSkip};

fn program(mode: TextMode, pattern: &str, replacement: &str) -> ReplaceProgram {
    let mut query = ContentQuery::text(pattern);
    query.mode = mode;
    query.match_case = true;
    ReplaceProgram::new(&query, replacement).unwrap()
}

/// The content after replacing every match (unchanged without one).
fn replaced(mode: TextMode, pattern: &str, replacement: &str, content: &str) -> String {
    let done = program(mode, pattern, replacement)
        .apply(content.as_bytes(), Pick::All, false)
        .unwrap();
    if done.replacements == 0 {
        return content.to_string();
    }
    String::from_utf8(done.content).unwrap()
}

fn regex(pattern: &str, replacement: &str, content: &str) -> String {
    replaced(TextMode::Regex, pattern, replacement, content)
}

/// Boost `format_all` as Notepad++ uses it (reference §1.1).
#[test]
fn replace_task_regex_replacement_follows_boost_format_all() {
    let words = r"(\w+) (\w+)";
    let cases: &[(&str, &str)] = &[
        ("$2 $1", "world hello"),
        (r"\2-\1", "world-hello"),
        ("${1}x", "hellox"),
        ("$&!", "hello world!"),
        ("$0!", "hello world!"),
        ("${0}.", "hello world."),
        ("$MATCH|${^MATCH}", "hello world|hello world"),
        ("$$", "$"),
        (r"\$1", "$1"),
        ("$+", "world"),
        ("$LAST_PAREN_MATCH", "world"),
        ("$^N", "world"),
        ("$12", ""),
        ("$a", "$a"),
        ("${name}", "${name}"),
        (r"\U$1\E $2", "HELLO world"),
        (r"\u$2 \l\U$1", "World HELLO"),
        (r"\L\u$1", "Hello"),
        (r"\(x\)\?\:", "(x)?:"),
        ("a)b", "a"),
        ("(a)b", "ab"),
        ("x?", "x?"),
        ("end$", "end$"),
        (r"\x41\x{263A}\t", "A\u{263A}\t"),
        (r"\n\r\e\a", "\n\r\x1b\x07"),
        (r"\cA", "\x01"),
        (r"\012", "\n"),
        (r"\q\\", r"q\"),
        (r"\xZZ", "xZZ"),
        (r"\x{41", "x{41"),
    ];
    for (replacement, expected) in cases {
        assert_eq!(
            regex(words, replacement, "hello world"),
            *expected,
            "replacement {replacement:?}"
        );
    }
    // Named groups: `$+{name}`, `?{name}`.
    let named = r"(?<first>\w+) (?<second>\w+)";
    assert_eq!(regex(named, "$+{second},$+{first}", "a b"), "b,a");
    assert_eq!(regex(named, "$+{none}.", "a b"), ".");
    assert_eq!(regex(named, "?{second}yes:no", "a b"), "yes");
    // Text before and after the match.
    assert_eq!(regex("-", "[$`|$']", "a-b"), "a[a|b]b");
    assert_eq!(regex("-", "[$PREMATCH|$POSTMATCH]", "a-b"), "a[a|b]b");
    // Conditionals with groups that took part or not.
    assert_eq!(regex("(a)|b", "?1X:Y", "ab"), "XY");
    assert_eq!(regex("(a)|b", "?{1}X:Y", "ab"), "XY");
    assert_eq!(regex("(a)|(b)", "(?1A:?2B)", "ab"), "AB");
    assert_eq!(regex("(a)|b", "<?1yes>", "ab"), "<yes><");
    // One-to-one case mapping: ß has no single upper case letter.
    assert_eq!(regex("straße", r"\U$0", "straße"), "STRAßE");
}

/// Normal is literal, Extended turns its escapes into characters in the
/// replacement too (Notepad++ `processRange`).
#[test]
fn replace_task_normal_and_extended_replacements() {
    assert_eq!(replaced(TextMode::Normal, "a", r"\n$1", "xay"), r"x\n$1y");
    assert_eq!(
        replaced(TextMode::Extended, "a", r"\t|\x41", "xay"),
        "x\t|Ay"
    );
    assert_eq!(replaced(TextMode::Extended, r"a\nb", "X", "a\nb\n"), "X\n");
    let mut query = ContentQuery::text("FOO");
    query.whole_word = true;
    let done = ReplaceProgram::new(&query, "bar")
        .unwrap()
        .apply(b"foo food Foo", Pick::All, false)
        .unwrap();
    assert_eq!(
        (done.replacements, done.content),
        (2, b"bar food bar".to_vec())
    );
}

/// Empty matches as `SCFIND_REGEXP_EMPTYMATCH_NOTAFTERMATCH`; matches across
/// lines; look-around through the backtracking engine with its groups.
#[test]
fn replace_task_matches_are_the_searchs() {
    assert_eq!(regex("x*", "-", "axb"), "-a-b-");
    assert_eq!(regex("^", ">", "a\nb"), ">a\n>b");
    assert_eq!(regex(r"a\Rb", "X", "a\r\nb a\nb"), "X X");
    assert_eq!(regex("(?<=a)(b)", "[$1]", "abcb"), "a[b]cb");
    assert_eq!(regex(r"(\w)\1", "<$1>", "aab"), "<a>b");
    assert_eq!(regex("é", "e", "café é"), "cafe e");
    let empty = program(TextMode::Normal, "fehlt", "x");
    let done = empty.apply(b"nichts", Pick::All, false).unwrap();
    assert_eq!((done.replacements, done.content.len()), (0, 0));
    let mut invalid = ContentQuery::text("(");
    invalid.mode = TextMode::Regex;
    assert!(ReplaceProgram::new(&invalid, "x").is_err());
}

/// BOM, UTF-16, ANSI and line ends stay; binary files and replacements an
/// ANSI file cannot hold are refused.
#[test]
fn replace_task_keeps_encoding_and_line_ends() {
    let apply = |pattern: &str, replacement: &str, content: &[u8]| {
        program(TextMode::Normal, pattern, replacement).apply(content, Pick::All, false)
    };
    let bom = apply("x", "z", b"\xEF\xBB\xBFx y").unwrap();
    assert_eq!(bom.content, b"\xEF\xBB\xBFz y");
    let utf16: Vec<u8> = [0xFF, 0xFE]
        .into_iter()
        .chain("a b".encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    let expected: Vec<u8> = [0xFF, 0xFE]
        .into_iter()
        .chain("ö b".encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    assert_eq!(apply("a", "ö", &utf16).unwrap().content, expected);
    let big: Vec<u8> = [0xFE, 0xFF]
        .into_iter()
        .chain("a".encode_utf16().flat_map(u16::to_be_bytes))
        .collect();
    assert_eq!(
        apply("a", "b", &big).unwrap().content,
        [0xFE, 0xFF, 0, b'b']
    );
    assert_eq!(
        apply("x", "€", b"caf\xE9 x").unwrap().content,
        b"caf\xE9 \x80"
    );
    assert_eq!(apply("é", "e", b"caf\xE9").unwrap().content, b"cafe");
    assert_eq!(apply("x", "Ω", b"caf\xE9 x"), Err(ReplaceSkip::NotAnsi));
    assert_eq!(apply("a", "b", b"a\0b"), Err(ReplaceSkip::Binary));
    assert_eq!(
        apply("a", "x", b"a\r\nb\r\n").unwrap().content,
        b"x\r\nb\r\n"
    );
    let lf = program(TextMode::Extended, "a", r"1\n2").apply(b"a\r\nb", Pick::All, false);
    assert_eq!(lf.unwrap().content, b"1\n2\r\nb");
    let end = program(TextMode::Regex, "b$", "c").apply(b"a\r\nb\r\n", Pick::All, false);
    assert_eq!(end.unwrap().content, b"a\r\nc\r\n");
}

fn hit(line: u64, ranges: &[(u32, u32)]) -> HitRef {
    HitRef {
        rel: "a.txt".into(),
        line,
        ranges: ranges.to_vec(),
        size: 0,
        mtime_ms: 0,
    }
}

/// Only the hits named: by line and range as the search showed them, a hit
/// without ranges for its whole line, a match across lines by any of its lines.
#[test]
fn replace_task_selected_hits_only() {
    let foo = program(TextMode::Normal, "foo", "X");
    let content = b"foo foo\nbar foo\nfoo\n";
    let refs = [hit(1, &[(4, 7)]), hit(2, &[])];
    let done = foo.apply(content, Pick::Hits(&refs), false).unwrap();
    assert_eq!(done.replacements, 2);
    assert_eq!(done.content, b"foo X\nbar X\nfoo\n");
    let none = foo.apply(content, Pick::Hits(&[hit(1, &[(1, 2)])]), false);
    assert_eq!(none.unwrap().replacements, 0);
    let across = program(TextMode::Extended, r"a\nb", "X");
    let done = across
        .apply(b"a\nb\n", Pick::Hits(&[hit(2, &[(0, 1)])]), false)
        .unwrap();
    assert_eq!(done.content, b"X\n");
    // Line 1 of a UTF-8 file with BOM, the BOM counted or not.
    let bom = b"\xEF\xBB\xBFfoo foo";
    for range in [(4, 7), (7, 10)] {
        let done = foo.apply(bom, Pick::Hits(&[hit(1, &[range])]), false);
        assert_eq!(done.unwrap().content, b"\xEF\xBB\xBFfoo X", "{range:?}");
    }
}

/// A dry run's previews: the lines before and after, one per group of lines.
#[test]
fn replace_task_previews_show_lines_before_and_after() {
    let two = program(TextMode::Normal, "two", "2");
    let done = two
        .apply(b"one two two\nthree\ntwo\n", Pick::All, true)
        .unwrap();
    let lines: Vec<(u64, &str, &str)> = done
        .previews
        .iter()
        .map(|preview| {
            (
                preview.line,
                preview.before.as_str(),
                preview.after.as_str(),
            )
        })
        .collect();
    assert_eq!(lines, [(1, "one two two", "one 2 2"), (3, "two", "2")]);
    let across = program(TextMode::Extended, r"a\nb", "X");
    let done = across.apply(b"za\nbz\n", Pick::All, true).unwrap();
    assert_eq!(done.previews[0].before, "za\nbz");
    assert_eq!(done.previews[0].after, "zXz");
    let mut long = ContentQuery::text("needle");
    long.max_line_bytes = 16;
    let content = format!("{}needle{}", "a".repeat(40), "b".repeat(40));
    let done = ReplaceProgram::new(&long, "N")
        .unwrap()
        .apply(content.as_bytes(), Pick::All, true)
        .unwrap();
    let preview = &done.previews[0];
    assert!(preview.truncated);
    assert!(preview.before.len() <= 16 && preview.before.contains("needle"));
    assert!(preview.after.contains('N'));
}
