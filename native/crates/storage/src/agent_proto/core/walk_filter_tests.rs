//! The walk filter applies the client's rules (hidden entries, patterns on
//! the path and, for a folder, on the path with `/`, size and time bounds)
//! and travels in the three walk requests; a run listing without a scope
//! keeps the frame of `walk-keys-v1`.
use super::{host_patterns, WalkFilterSpec, WalkScope};
use crate::agent_proto::Frame;

fn spec(patterns: &[&str]) -> WalkFilterSpec {
    WalkFilterSpec {
        patterns: patterns.iter().map(|regex| regex.to_string()).collect(),
        ..WalkFilterSpec::default()
    }
}

fn roundtrip(frame: Frame) -> Frame {
    let bytes = frame.encode(3).unwrap();
    assert_eq!(bytes.len(), frame.wire_len().unwrap());
    let (id, decoded) = Frame::decode(&bytes).unwrap();
    assert_eq!(id, 3);
    decoded
}

#[test]
fn abgleich_task_walk_filter_follows_the_client_rules() {
    // `**/*.tmp` and `cache/` as globset writes them.
    let filter = WalkFilterSpec {
        skip_hidden: true,
        min_size: 2,
        max_size: 100,
        after_mtime_ms: 10,
        before_mtime_ms: 1_000,
        ..spec(&["(?-u)^(?:/?|.*/)[^/]*\\.tmp$", "(?-u)^cache/$"])
    }
    .compile()
    .unwrap();
    assert!(filter.leaves_out_file("x/a.tmp", false, 5, 100));
    assert!(!filter.leaves_out_file("x/a.txt", false, 5, 100));
    assert!(filter.leaves_out_file("x/a.txt", true, 5, 100), "hidden");
    assert!(
        filter.leaves_out_file("x/a.txt", false, 1, 100),
        "too small"
    );
    assert!(
        filter.leaves_out_file("x/a.txt", false, 101, 100),
        "too large"
    );
    assert!(filter.leaves_out_file("x/a.txt", false, 5, 9), "too old");
    assert!(
        filter.leaves_out_file("x/a.txt", false, 5, 1_001),
        "too new"
    );
    // A folder matches with its trailing slash as well, a file never does.
    assert!(filter.leaves_out_dir("cache", false));
    assert!(!filter.leaves_out_file("cache", false, 5, 100));
    assert!(filter.leaves_out_dir("d", true), "hidden folder");
    assert!(!filter.leaves_out_dir("d", false));
    // Bounds never leave a folder out.
    let bounds = WalkFilterSpec {
        min_size: 1 << 40,
        ..WalkFilterSpec::default()
    }
    .compile()
    .unwrap();
    assert!(!bounds.leaves_out_dir("big", false));
}

#[test]
fn abgleich_task_walk_filter_matches_a_backslash_as_globset_on_the_client() {
    let mut filter = spec(&["(?-u)^a/b$"]);
    assert!(!filter
        .compile()
        .unwrap()
        .leaves_out_file("a\\b", false, 0, 0));
    filter.backslash_separates = true;
    assert!(filter
        .compile()
        .unwrap()
        .leaves_out_file("a\\b", false, 0, 0));
}

#[test]
fn abgleich_task_walk_filter_takes_only_patterns_it_evaluates() {
    let regexes = vec![
        "(?-u)^.*\\.tmp$".to_string(),
        "(?-u)^\\d$".to_string(),
        "(?-u)(?i)^.*\\.log$".to_string(),
    ];
    assert_eq!(
        host_patterns(regexes),
        vec![
            "(?-u)^.*\\.tmp$".to_string(),
            "(?-u)(?i)^.*\\.log$".to_string()
        ]
    );
    assert!(WalkFilterSpec::default().is_empty());
    assert!(!spec(&["(?-u)^a$"]).is_empty());
    assert!(spec(&["(?-u)^\\d$"]).compile().is_err());
}

#[test]
fn abgleich_task_walk_scope_travels_in_the_walk_requests() {
    let scope = WalkScope {
        fresh: true,
        filter: Some(WalkFilterSpec {
            skip_hidden: true,
            backslash_separates: true,
            min_size: 1,
            max_size: u64::MAX,
            after_mtime_ms: -5,
            before_mtime_ms: i64::MAX,
            ..spec(&["(?-u)^a$", "(?-u)(?i)^\\xc3\\xa4/.*$"])
        }),
    };
    let fresh_only = WalkScope {
        fresh: true,
        filter: None,
    };
    for scope in [scope, fresh_only, WalkScope::default()] {
        for frame in [
            Frame::WalkHashed3 {
                root: "/srv".into(),
                algorithm: 1,
                min_bytes: 0,
                meta: true,
                scope: scope.clone(),
            },
            Frame::WalkKeyed {
                root: "/srv".into(),
                algorithm: 1,
                min_bytes: 0,
                known: vec![7; 16],
                keys_only: true,
                scope: scope.clone(),
            },
            Frame::ListRuns {
                root: "/srv".into(),
                algorithm: 0,
                folders: vec!["a".into()],
                scope: scope.clone(),
            },
        ] {
            assert_eq!(roundtrip(frame.clone()), frame);
        }
    }
}

#[test]
fn abgleich_task_run_listing_without_scope_keeps_its_frame() {
    let plain = Frame::ListRuns {
        root: "/".into(),
        algorithm: 0,
        folders: vec!["a".into()],
        scope: WalkScope::default(),
    };
    let bytes = plain.encode(1).unwrap();
    // Request id, tag, root, algorithm, count, one folder: nothing after it.
    assert_eq!(bytes.len(), 8 + 1 + (4 + 1) + 1 + 4 + (4 + 1));
    // An unknown option bit is refused.
    let mut fresh = Frame::ListRuns {
        root: "/".into(),
        algorithm: 0,
        folders: vec!["a".into()],
        scope: WalkScope {
            fresh: true,
            filter: None,
        },
    }
    .encode(1)
    .unwrap();
    *fresh.last_mut().unwrap() = 16;
    assert!(Frame::decode(&fresh).is_err());
}

#[test]
fn abgleich_task_host_patterns_fit_the_text_every_host_takes() {
    // JSON text as serde writes it: `a`, `\"`, `b`, `\\`, `c`, `\n`,
    // `\u0001`, two bytes of `é` (17), quotes and a comma.
    assert_eq!(super::pattern_wire_cost("a\"b\\c\n\u{1}é"), 20);
    let regexes: Vec<String> = (0..3000)
        .map(|index| format!("(?-u)^(?:/?|.*/)name{index}\\.tmp$"))
        .collect();
    let host = host_patterns(regexes.clone());
    assert!(!host.is_empty() && host.len() < regexes.len());
    // In order, as far as the text every host takes reaches.
    assert_eq!(host[..], regexes[..host.len()]);
    let text: usize = host
        .iter()
        .map(|regex| super::pattern_wire_cost(regex.as_str()))
        .sum();
    assert!(text <= super::HOST_PATTERN_WIRE, "{text}");
    // The Share request's bound takes the list whole: the agent, the Share
    // host and the local side's keys apply the same patterns.
    let spec = WalkFilterSpec {
        patterns: host.clone(),
        ..WalkFilterSpec::default()
    };
    assert_eq!(spec.bounded().patterns, host);
    // A longer list is cut to a prefix that fits.
    let long = WalkFilterSpec {
        patterns: regexes.clone(),
        ..WalkFilterSpec::default()
    };
    assert_eq!(long.bounded().patterns, host);
}
