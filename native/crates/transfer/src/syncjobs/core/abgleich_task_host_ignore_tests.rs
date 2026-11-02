//! The ignore patterns a walk next to the data applies (`JobIgnore::host`)
//! mean exactly what globset 0.4.18 means on this device: for every pattern
//! the walk takes, in both letter-case rules, its verdict on a file path and
//! on a folder path (the path, and the path with `/`) equals globset's, over
//! paths with letters of both cases, non-ASCII letters, a line break, a
//! backslash and the folder forms. A pattern it does not take is left to
//! the client (never applied next to the data).
use super::compile_with_host;
use crate::agent_proto::WalkFilterSpec;

const PATTERNS: &[&str] = &[
    "**/*.tmp",
    "*.tmp",
    "*.TMP",
    "node_modules/",
    "**/node_modules/**",
    "build",
    "/build",
    "build/**",
    "**/cache",
    "a/**/b",
    "a/*/b",
    "?.txt",
    "??",
    "[abc]*",
    "[!abc]*",
    "[a-z].log",
    "[!a-z]",
    "[]]x",
    "[-a]",
    "{a,b}/*.o",
    "{*.rs,*.toml}",
    "x{,y}",
    "{a,{b,c}}d",
    "**",
    "*",
    "Ä*",
    "*ä*",
    "ÄÖÜ/**",
    "[ä]",
    "\\*literal",
    "a\\b",
    "with space/*",
    "#hash",
    "a+b(c)",
    "$x^y",
    "~tilde&and",
    ".*",
    ".git/",
    "**/.DS_Store",
    "*.[oa]",
    "dir/",
    "**/dir/",
];

const PATHS: &[&str] = &[
    "",
    "a",
    "b",
    "x",
    "xy",
    "ab",
    "a.tmp",
    "A.TMP",
    "x/a.tmp",
    "x/A.Tmp",
    "a.tmpx",
    ".tmp",
    "node_modules",
    "x/node_modules",
    "x/node_modules/y",
    "build",
    "build/x",
    "x/build",
    "cache",
    "x/y/cache",
    "a/b",
    "a/x/b",
    "a/x/y/b",
    "a.txt",
    "ab.txt",
    "abc",
    "zz",
    "q.log",
    "Q.LOG",
    "]x",
    "-",
    "a/m.o",
    "b/m.o",
    "c/m.o",
    "lib.rs",
    "Cargo.toml",
    "x",
    "xy",
    "ad",
    "bd",
    "cd",
    "Äpfel",
    "äpfel",
    "Bär",
    "BÄR",
    "ÄÖÜ/x",
    "äöü/x",
    "ä",
    "Ä",
    "*literal",
    "xliteral",
    "a\\b",
    "a/b",
    "with space/f",
    "#hash",
    "a+b(c)",
    "$x^y",
    "~tilde&and",
    ".git",
    ".hidden",
    "x/.DS_Store",
    "m.o",
    "m.a",
    "line\nbreak.tmp",
    "dir",
    "x/dir",
    "dir/x",
];

fn with_slash(path: &str) -> String {
    format!("{path}/")
}

#[test]
fn abgleich_task_host_patterns_mean_what_globset_means() {
    let mut taken = 0usize;
    for case_insensitive in [false, true] {
        for pattern in PATTERNS {
            let patterns = vec![pattern.to_string()];
            let ignore = compile_with_host(&patterns, case_insensitive).unwrap();
            if ignore.host.is_empty() {
                continue;
            }
            taken += 1;
            let filter = WalkFilterSpec {
                patterns: ignore.host.clone(),
                // As globset's candidate path on this device.
                backslash_separates: std::path::is_separator('\\'),
                ..WalkFilterSpec::default()
            }
            .compile()
            .unwrap();
            for path in PATHS {
                let file = ignore.globs.is_match(path);
                let folder = file || ignore.globs.is_match(with_slash(path));
                let context =
                    format!("{pattern:?} (case-insensitive {case_insensitive}) on {path:?}");
                assert_eq!(
                    filter.leaves_out_file(path, false, 0, 0),
                    file,
                    "file {context}"
                );
                assert_eq!(
                    filter.leaves_out_dir(path, false),
                    folder,
                    "folder {context}"
                );
            }
        }
    }
    // Every pattern of the list is one globset writes in the accepted form.
    assert_eq!(taken, 2 * PATTERNS.len(), "patterns left to the client");
}

#[test]
fn abgleich_task_host_patterns_of_a_whole_job_mean_what_its_set_means() {
    let patterns: Vec<String> = PATTERNS.iter().map(|pattern| pattern.to_string()).collect();
    for case_insensitive in [false, true] {
        let ignore = compile_with_host(&patterns, case_insensitive).unwrap();
        assert_eq!(ignore.host.len(), patterns.len());
        let filter = WalkFilterSpec {
            patterns: ignore.host.clone(),
            backslash_separates: std::path::is_separator('\\'),
            ..WalkFilterSpec::default()
        }
        .compile()
        .unwrap();
        for path in PATHS {
            let file = ignore.globs.is_match(path);
            let folder = file || ignore.globs.is_match(with_slash(path));
            assert_eq!(filter.leaves_out_file(path, false, 0, 0), file, "{path:?}");
            assert_eq!(filter.leaves_out_dir(path, false), folder, "{path:?}");
        }
    }
}

#[test]
fn abgleich_task_job_ignore_keeps_the_set_and_the_case_rule() {
    let patterns = vec!["**/*.tmp".to_string()];
    let sensitive = compile_with_host(&patterns, false).unwrap();
    let insensitive = compile_with_host(&patterns, true).unwrap();
    assert!(!sensitive.globs.is_match("A.TMP"));
    assert!(insensitive.globs.is_match("A.TMP"));
    assert!(!sensitive.host[0].contains("(?i)"));
    assert!(insensitive.host[0].contains("(?i)"));
    // An invalid pattern fails the whole set, as before.
    assert!(compile_with_host(&["[".to_string()], false).is_err());
}
