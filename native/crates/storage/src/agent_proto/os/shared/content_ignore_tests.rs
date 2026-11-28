//! Ignore files as ripgrep reads them: precedence of `.ignore`, `.gitignore`
//! and `.git/info/exclude`, whitelists, the work tree requirement, folders
//! above the search root, and the local content search with and without
//! `respect_ignore_files`.
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use super::{local_parents, IgnoreChain, IgnoreFiles, ParentIgnore};
use crate::agent_proto::{search_local_tree, ContentEvent, ContentQuery};

fn files(
    ignore: Option<&str>,
    gitignore: Option<&str>,
    exclude: Option<&str>,
    git: bool,
) -> IgnoreFiles {
    IgnoreFiles {
        ignore: ignore.map(|text| text.as_bytes().to_vec()),
        gitignore: gitignore.map(|text| text.as_bytes().to_vec()),
        exclude: exclude.map(|text| text.as_bytes().to_vec()),
        has_git: git,
    }
}

#[test]
fn mcp_coding_task_ignore_chain_follows_ripgrep_precedence() {
    let root = IgnoreChain::default().child(
        "",
        &files(
            None,
            Some("\u{feff}*.log\r\n!keep.log\n/build/\n# comment\n"),
            Some("local.tmp\n"),
            true,
        ),
    );
    assert!(root.ignored("a.log", false));
    assert!(!root.ignored("keep.log", false), "a whitelist keeps a file");
    assert!(root.ignored("build", true));
    assert!(!root.ignored("build", false), "`build/` names folders only");
    assert!(!root.ignored("sub/build", true), "`/build/` is anchored");
    assert!(root.ignored("local.tmp", false), ".git/info/exclude");
    assert!(!root.ignored("main.rs", false));

    // The deeper folder decides within one kind; `.ignore` wins over Git.
    let sub = root.child(
        "sub",
        &files(Some("keep.log\n"), Some("!sub.log\n"), None, false),
    );
    assert!(!sub.ignored("sub/sub.log", false));
    assert!(sub.ignored("sub/x.log", false));
    assert!(
        sub.ignored("sub/keep.log", false),
        "`.ignore` comes before the whitelist of `.gitignore`"
    );
    // A folder without ignore files shares its parent's chain.
    let plain = sub.child("sub/deeper", &IgnoreFiles::default());
    assert!(plain.ignored("sub/deeper/y.log", false));

    // Outside a work tree `.gitignore` does not apply, `.ignore` does.
    let loose =
        IgnoreChain::default().child("", &files(Some("*.md\n"), Some("*.log\n"), None, false));
    assert!(!loose.ignored("a.log", false));
    assert!(loose.ignored("a.md", false));
    assert!(IgnoreChain::default().is_empty() && !IgnoreChain::default().ignored("a", false));
}

#[test]
fn mcp_coding_task_ignore_chain_reads_folders_above_the_root() {
    // The root is `repo/src`; `repo` is the work tree root, `top` above it.
    let parents = vec![
        ParentIgnore {
            lead: "src".into(),
            files: files(
                None,
                Some("src/gen/\n*.bak\n"),
                Some("src/local.txt\n"),
                true,
            ),
        },
        ParentIgnore {
            lead: "repo/src".into(),
            files: files(Some("*.md\n"), Some("*.rs\n"), None, false),
        },
    ];
    let chain = IgnoreChain::above_root(&parents).child("", &IgnoreFiles::default());
    assert!(chain.ignored("gen", true));
    assert!(chain.ignored("x.bak", false));
    assert!(chain.ignored("local.txt", false));
    assert!(
        chain.ignored("notes.md", false),
        "`.ignore` of every folder above"
    );
    assert!(
        !chain.ignored("main.rs", false),
        "Git files above the work tree root do not apply"
    );
}

fn hit_files(root: &Path, ignore: bool) -> Vec<String> {
    let mut query = ContentQuery::text("needle");
    query.respect_ignore_files = ignore;
    let found = Mutex::new(Vec::new());
    let emit = |event: ContentEvent| {
        if let ContentEvent::Hits {
            rel, last: true, ..
        } = event
        {
            found.lock().unwrap().push(rel);
        }
        true
    };
    search_local_tree(root, &query, &emit, &AtomicBool::new(false)).expect("search");
    let mut found = found.into_inner().unwrap();
    found.sort();
    found
}

#[test]
fn mcp_coding_task_local_content_search_respects_ignore_files() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let write = |rel: &str, text: &str| {
        let path = repo.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write(".git/info/exclude", "excluded.txt\n");
    write(".git/HEAD", "ref: refs/heads/main\n");
    write(".gitignore", "target/\n*.log\nsrc/gen/\n");
    write(".ignore", "secret.txt\n");
    for rel in [
        "a.txt",
        "target/b.txt",
        "c.log",
        "secret.txt",
        "excluded.txt",
        "sub/d.txt",
        "sub/e.txt",
        "src/gen/f.txt",
        "src/g.txt",
    ] {
        write(rel, "a needle here\n");
    }
    write("sub/.gitignore", "d.txt\n");

    assert_eq!(hit_files(&repo, true), ["a.txt", "src/g.txt", "sub/e.txt"]);
    assert_eq!(
        hit_files(&repo, false).len(),
        9,
        "without the flag every file"
    );
    // A search below the work tree root applies the root's files.
    assert_eq!(hit_files(&repo.join("src"), true), ["g.txt"]);
    let parents = local_parents(&repo.join("src"));
    assert_eq!(
        parents.first().map(|parent| parent.lead.as_str()),
        Some("src")
    );
    assert!(parents[0].files.has_git);
}
