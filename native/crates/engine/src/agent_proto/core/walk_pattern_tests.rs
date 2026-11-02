//! The automaton of the walk filter reads exactly the regular expressions
//! globset writes and refuses everything else; its matches follow the byte
//! rules of globset's matcher (any byte for `.`, ASCII-only case folding,
//! folding before negation). The comparison with globset itself is
//! `syncjobs/core/abgleich_task_host_ignore_tests.rs`.
use super::{pattern_states, PatternSet};

fn set(regexes: &[&str]) -> PatternSet {
    let regexes: Vec<String> = regexes.iter().map(|regex| regex.to_string()).collect();
    PatternSet::new(&regexes).expect("pattern accepted")
}

#[test]
fn abgleich_task_walk_pattern_refuses_what_globset_never_writes() {
    for refused in [
        "^a$",
        "(?-u)a$",
        "(?-u)^a",
        "(?-u)^a+$",
        "(?-u)^a{2}$",
        "(?-u)^\\d$",
        "(?-u)^\\n$",
        "(?-u)^(a)$",
        "(?-u)^(?i:a)$",
        "(?-u)^[]a]$",
        "(?-u)^[a-]$",
        "(?-u)^[z-a]$",
        "(?-u)^[a&&b]$",
        "(?-u)^.*?$",
        "(?-u)^ä$",
        "(?-u)^a\\$",
        "(?-u)^\\x4$",
        "(?-u)^\\<a$",
    ] {
        assert!(
            pattern_states(refused).is_err(),
            "{refused} must be refused"
        );
    }
    for accepted in [
        "(?-u)^.*$",
        "(?-u)(?i)^.*$",
        "(?-u)^(?:/?|.*/)[^/]*\\.tmp$",
        "(?-u)^foo/.*$",
        "(?-u)^(?:/|/.*/)$",
        "(?-u)^a(?:b|c\\-d)$",
        "(?-u)^[!a-z\\]\\-]$",
        "(?-u)^[^/]$",
        "(?-u)^\\xc3\\xa4$",
        "(?-u)^[\\xc3\\xa4]$",
        "(?-u)^$",
        "(?-u)^ #x$",
    ] {
        assert!(pattern_states(accepted).is_ok(), "{accepted} must be read");
    }
}

#[test]
fn abgleich_task_walk_pattern_matches_bytes_as_globsets_matcher() {
    let star = set(&["(?-u)^(?:/?|.*/).*\\.tmp$"]);
    assert!(star.is_match(b"a.tmp"));
    assert!(star.is_match(b"x/y/a.tmp"));
    assert!(star.is_match(b"x/\n.tmp"), "`.` takes a line break too");
    assert!(!star.is_match(b"a.tmpx"));
    assert!(!star.is_match(b"a.TMP"), "case counts without (?i)");
    let folded = set(&["(?-u)(?i)^.*\\.tmp$"]);
    assert!(folded.is_match(b"A.TMP"));
    assert!(folded.is_match("Ä.tmp".as_bytes()));
    // Folding is ASCII only: a byte above 0x7F matches itself alone.
    let umlaut = set(&["(?-u)(?i)^\\xc3\\xa4$"]);
    assert!(umlaut.is_match("ä".as_bytes()));
    assert!(!umlaut.is_match("Ä".as_bytes()));
    // `.` is one byte: one non-ASCII letter takes two.
    assert!(!set(&["(?-u)^.$"]).is_match("ä".as_bytes()));
    assert!(set(&["(?-u)^..$"]).is_match("ä".as_bytes()));
    // A class is folded before it is negated.
    let negated = set(&["(?-u)(?i)^[^x]$"]);
    assert!(!negated.is_match(b"X"));
    assert!(!negated.is_match(b"x"));
    assert!(negated.is_match(b"y"));
    // A byte class of a two-byte letter takes either byte alone.
    let bytes = set(&["(?-u)^[\\xc3\\xa4]$"]);
    assert!(bytes.is_match(&[0xc3]));
    assert!(bytes.is_match(&[0xa4]));
    assert!(!bytes.is_match("ä".as_bytes()));
}

#[test]
fn abgleich_task_walk_pattern_set_matches_when_any_pattern_does() {
    let both = set(&["(?-u)^a$", "(?-u)^b/.*$"]);
    assert!(both.is_match(b"a"));
    assert!(both.is_match(b"b/c"));
    assert!(!both.is_match(b"c"));
    assert!(!both.is_match(b"b"));
    let none = PatternSet::new(&[]).unwrap();
    assert!(none.is_empty());
    assert!(!none.is_match(b""));
    // The empty expression matches the empty path only.
    let empty = set(&["(?-u)^$"]);
    assert!(empty.is_match(b""));
    assert!(!empty.is_match(b"a"));
}
