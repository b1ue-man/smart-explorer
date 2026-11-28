//! The parser against the cases of Codex's own parser tests
//! (`codex-rs/apply-patch/src/parser.rs`, `rust-v0.166.0`).
use super::{parse_patch, Chunk, Hunk};

fn chunk(context: Option<&str>, old: &[&str], new: &[&str]) -> Chunk {
    Chunk {
        change_context: context.map(str::to_string),
        old_lines: old.iter().map(|line| line.to_string()).collect(),
        new_lines: new.iter().map(|line| line.to_string()).collect(),
        context_line_indices: Vec::new(),
        is_end_of_file: false,
    }
}

#[test]
fn mcp_coding_task_patch_parser_follows_codex() {
    assert_eq!(
        parse_patch("bad"),
        Err("invalid patch: The first line of the patch must be '*** Begin Patch'".into())
    );
    assert_eq!(
        parse_patch("*** Begin Patch\nbad"),
        Err("invalid patch: The last line of the patch must be '*** End Patch'".into())
    );
    assert_eq!(
        parse_patch("*** Begin Patch \n*** Add File: foo\n+hi\n *** End Patch"),
        Ok(vec![Hunk::Add {
            path: "foo".into(),
            contents: "hi\n".into()
        }])
    );
    assert_eq!(
        parse_patch("*** Begin Patch\n*** Update File: test.py\n*** End Patch"),
        Err("invalid hunk at line 2, Update file hunk for path 'test.py' is empty".into())
    );
    assert_eq!(
        parse_patch("*** Begin Patch\n*** End Patch"),
        Ok(Vec::new())
    );
    assert_eq!(
        parse_patch(
            "*** Begin Patch\n*** Add File: path/add.py\n+abc\n+def\n*** Delete File: path/delete.py\n*** Update File: path/update.py\n*** Move to: path/update2.py\n@@ def f():\n-    pass\n+    return 123\n*** End Patch"
        ),
        Ok(vec![
            Hunk::Add {
                path: "path/add.py".into(),
                contents: "abc\ndef\n".into()
            },
            Hunk::Delete {
                path: "path/delete.py".into()
            },
            Hunk::Update {
                path: "path/update.py".into(),
                move_path: Some("path/update2.py".into()),
                chunks: vec![chunk(Some("def f():"), &["    pass"], &["    return 123"])]
            }
        ])
    );
    // An update followed by another hunk.
    assert_eq!(
        parse_patch(
            "*** Begin Patch\n*** Update File: file.py\n@@\n+line\n*** Add File: other.py\n+content\n*** End Patch"
        ),
        Ok(vec![
            Hunk::Update {
                path: "file.py".into(),
                move_path: None,
                chunks: vec![chunk(None, &[], &["line"])]
            },
            Hunk::Add {
                path: "other.py".into(),
                contents: "content\n".into()
            }
        ])
    );
    // An update without `@@` at its start, context lines and the file end.
    let parsed = parse_patch(
        "*** Begin Patch\n*** Update File: a.txt\n context\n-old\n+new\n*** End of File\n*** End Patch",
    )
    .unwrap();
    let Hunk::Update { chunks, .. } = &parsed[0] else {
        panic!("{parsed:?}");
    };
    assert_eq!(chunks[0].old_lines, ["context", "old"]);
    assert_eq!(chunks[0].new_lines, ["context", "new"]);
    assert_eq!(chunks[0].context_line_indices, [(0, 0)]);
    assert!(chunks[0].is_end_of_file);
    // A line that is no change line.
    assert_eq!(
        parse_patch("*** Begin Patch\n*** Update File: a\n@@\n-x\nbad\n*** End Patch"),
        Err("invalid hunk at line 5, Expected update hunk to start with a @@ context marker, got: 'bad'".into())
    );
    assert_eq!(
        parse_patch("*** Begin Patch\n*** Frobnicate: a\n*** End Patch"),
        Err("invalid hunk at line 2, '*** Frobnicate: a' is not a valid hunk header. Valid hunk headers: '*** Add File: {path}', '*** Delete File: {path}', '*** Update File: {path}'".into())
    );
    // Codex's lenient heredoc form.
    assert_eq!(
        parse_patch("<<'EOF'\n*** Begin Patch\n*** Delete File: x\n*** End Patch\nEOF\n"),
        Ok(vec![Hunk::Delete { path: "x".into() }])
    );
    // The environment id is read once.
    assert!(parse_patch(
        "*** Begin Patch\n*** Environment ID: dev\n*** Delete File: x\n*** End Patch"
    )
    .is_ok());
    assert!(parse_patch(
        "*** Begin Patch\n*** Environment ID: a\n*** Environment ID: b\n*** End Patch"
    )
    .is_err());
}
