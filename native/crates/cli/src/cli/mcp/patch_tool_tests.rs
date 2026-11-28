//! `apply_patch` on a local folder: every hunk kind, several files, moves,
//! a patch that does not match changes nothing, and a file that changed
//! since it was read is refused.
use super::patch_tool_publish::{publish, Expect};
use super::{apply_with, spelling};
use crate::cli::mcp::targets::Targets;
use crate::masks::MaskSet;
use serde_json::Value;

fn apply(base: &str, patch: &str) -> Result<(bool, Value), String> {
    let result = apply_with(&Targets::default(), &MaskSet::empty(), patch, base)?;
    Ok((
        result.is_error == Some(true),
        result.structured_content.unwrap_or(Value::Null),
    ))
}

#[test]
fn mcp_coding_task_apply_patch_changes_several_files() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().to_string_lossy().into_owned();
    std::fs::write(
        dir.path().join("main.rs"),
        "fn main() {\r\n    old();\r\n}\r\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("gone.txt"), "bye\n").unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn a() {}\npub fn b() {}\n",
    )
    .unwrap();
    let patch = [
        "*** Begin Patch",
        "*** Add File: docs/new.md",
        "+# Title",
        "+text",
        "*** Update File: main.rs",
        "@@ fn main() {",
        "-    old();",
        "+    new();",
        "*** Delete File: gone.txt",
        "*** Update File: src/lib.rs",
        "*** Move to: src/moved.rs",
        "@@",
        " pub fn a() {}",
        "-pub fn b() {}",
        "+pub fn c() {}",
        "*** Update File: docs/new.md",
        "@@",
        "-text",
        "+more text",
        "*** End Patch",
    ]
    .join("\n");
    let (failed, result) = apply(&base, &patch).unwrap();
    assert!(!failed, "{result}");
    let read = |rel: &str| std::fs::read_to_string(dir.path().join(rel)).unwrap();
    assert_eq!(
        read("docs/new.md"),
        "# Title\nmore text\n",
        "hunks on one file apply in order"
    );
    assert_eq!(
        read("main.rs"),
        "fn main() {\r\n    new();\r\n}\r\n",
        "line ends stay"
    );
    assert!(!dir.path().join("gone.txt").exists());
    assert!(!dir.path().join("src/lib.rs").exists());
    assert_eq!(read("src/moved.rs"), "pub fn a() {}\npub fn c() {}\n");
    let files = result["files"].as_array().unwrap();
    let action = |named: &str| {
        files
            .iter()
            .find(|file| file["named"] == named)
            .map(|file| {
                (
                    file["action"].as_str().unwrap().to_string(),
                    file.get("etag").is_some(),
                )
            })
    };
    assert_eq!(action("docs/new.md"), Some(("added".into(), true)));
    assert_eq!(action("main.rs"), Some(("updated".into(), true)));
    assert_eq!(action("gone.txt"), Some(("deleted".into(), false)));
    assert_eq!(action("src/lib.rs"), Some(("deleted".into(), false)));
    assert_eq!(action("src/moved.rs"), Some(("added".into(), true)));
    assert_eq!(result["moves"][0]["to"], "src/moved.rs");
    assert!(result["summary"].as_str().unwrap().starts_with("Success."));
}

#[test]
fn mcp_coding_task_apply_patch_that_does_not_match_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().to_string_lossy().into_owned();
    std::fs::write(dir.path().join("a.txt"), "one\ntwo\n").unwrap();
    let patch = "*** Begin Patch\n*** Add File: b.txt\n+new\n*** Update File: a.txt\n@@\n-three\n+3\n*** End Patch";
    let error = apply(&base, patch).unwrap_err();
    assert!(error.contains("Failed to find expected lines"), "{error}");
    assert!(!dir.path().join("b.txt").exists(), "nothing was published");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "one\ntwo\n"
    );
    let missing = "*** Begin Patch\n*** Delete File: nope.txt\n*** End Patch";
    assert!(apply(&base, missing)
        .unwrap_err()
        .contains("does not exist"));
    assert_eq!(
        apply(&base, "*** Begin Patch\n*** End Patch").unwrap_err(),
        "No files were modified."
    );
    assert!(apply(&base, "no patch")
        .unwrap_err()
        .starts_with("invalid patch"));
}

#[test]
fn mcp_coding_task_apply_patch_refuses_a_file_changed_since_it_was_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    std::fs::write(&path, "one\n").unwrap();
    let targets = Targets::default();
    let target = targets.resolve(&path.to_string_lossy()).unwrap();
    let stale = Expect::Etag("0000000000000000".into());
    assert!(publish(&target, b"two\n", &stale)
        .unwrap_err()
        .contains("changed since"));
    assert!(publish(&target, b"two\n", &Expect::Missing).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\n");
    // Paths of the patch are relative to the base unless they spell a target.
    assert_eq!(spelling("/w", "./src/a.rs"), "/w/src/a.rs");
    assert_eq!(spelling("/w/", "b"), "/w/b");
    assert_eq!(spelling("/w", "/abs/c"), "/abs/c");
    assert_eq!(spelling("", "rel"), "rel");
    assert_eq!(spelling("/w", "server:etc/x"), "server:etc/x");
}
