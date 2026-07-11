use std::fs;

use crate::support::{
    assert_exit_code, assert_success, assert_success_for, os, run, stderr, stdout, Sandbox,
};

#[test]
fn help_succeeds_and_invalid_subcommand_uses_clap_exit_code() {
    let sandbox = Sandbox::new("parser");

    let help = run(sandbox.command().arg("--help"));
    assert_success(&help);
    assert!(stdout(&help).contains("Smart Explorer terminal access"));
    assert!(stdout(&help).contains("Usage: se"));
    assert!(stdout(&help).contains("search"));

    let invalid = run(sandbox.command().arg("not-a-real-command"));
    assert_exit_code(&invalid, 2);
    assert!(stderr(&invalid).contains("unrecognized subcommand"));
    assert!(stderr(&invalid).contains("Usage: se"));
}

#[test]
fn local_ls_stat_and_cat_report_real_files() {
    let sandbox = Sandbox::new("read");
    let file = sandbox.path("alpha note.txt");
    let payload = b"alpha body\n";
    fs::write(&file, payload).expect("create local fixture");
    fs::create_dir(sandbox.path("folder with spaces")).expect("create local directory fixture");

    let listing = run(sandbox.command().arg("ls").arg(&sandbox.root));
    assert_success(&listing);
    assert!(stdout(&listing)
        .lines()
        .any(|line| line.starts_with("--\t") && line.ends_with("\talpha note.txt")));
    assert!(stdout(&listing)
        .lines()
        .any(|line| line.starts_with("d-\t") && line.ends_with("\tfolder with spaces")));

    let stat = run(sandbox.command().arg("stat").arg(&file));
    assert_success(&stat);
    assert!(stdout(&stat).contains("name\talpha note.txt\n"));
    assert!(stdout(&stat).contains("type\tfile\n"));
    assert!(stdout(&stat).contains(&format!("size\t{}\n", payload.len())));

    let cat = run(sandbox.command().arg("cat").arg(&file));
    assert_success(&cat);
    assert_eq!(cat.stdout, payload);
    assert!(cat.stderr.is_empty());
}

#[test]
fn local_mkdir_copy_move_and_remove_are_end_to_end() {
    let sandbox = Sandbox::new("mutate");
    let source = sandbox.path("source file.txt");
    let copied = sandbox.path("created parent/copied file.txt");
    let moved = sandbox.path("created parent/moved file.txt");
    let payload = b"copy and move payload";
    fs::write(&source, payload).expect("create source fixture");

    let mkdir = run(sandbox
        .command()
        .arg("mkdir")
        .arg(sandbox.path("created parent")));
    assert_success_for("mkdir", &mkdir);
    assert!(sandbox.path("created parent").is_dir());

    let copy = run(sandbox.command().arg("cp").arg(&source).arg(&copied));
    assert_success_for("copy", &copy);
    assert_eq!(fs::read(&copied).expect("read copied fixture"), payload);
    assert_eq!(
        fs::read(&source).expect("source remains after copy"),
        payload
    );

    let move_output = run(sandbox.command().arg("mv").arg(&copied).arg(&moved));
    assert_success_for("move", &move_output);
    assert!(!copied.exists());
    assert_eq!(fs::read(&moved).expect("read moved fixture"), payload);

    let unforced_remove = run(sandbox.command().arg("rm").arg(&moved));
    assert_exit_code(&unforced_remove, 1);
    assert!(stderr(&unforced_remove).contains("rm requires --force"));
    assert!(moved.exists());

    let forced_remove = run(sandbox.command().arg("rm").arg("--force").arg(&moved));
    assert_success_for("forced remove", &forced_remove);
    assert!(!moved.exists());

    let recursive_remove = run(sandbox
        .command()
        .arg("rm")
        .arg("--recursive")
        .arg("--force")
        .arg(sandbox.path("created parent")));
    assert_success_for("recursive remove", &recursive_remove);
    assert!(!sandbox.path("created parent").exists());
}

#[test]
fn rm_preserve_root_guard_fails_before_recursive_delete_is_allowed() {
    let sandbox = Sandbox::new("preserve-root");
    let canonical_sandbox = fs::canonicalize(&sandbox.root).expect("canonicalize sandbox path");
    let filesystem_root = canonical_sandbox
        .ancestors()
        .last()
        .expect("an absolute temporary path has a filesystem root");
    assert!(filesystem_root.is_absolute());

    // Deliberately omit --recursive. Even if preserve-root regresses, the
    // independent directory guard prevents this test from deleting anything.
    let guarded = run(sandbox
        .command()
        .arg("rm")
        .arg("--force")
        .arg(filesystem_root));
    assert_exit_code(&guarded, 1);
    assert!(stderr(&guarded).contains("--no-preserve-root"));
    assert!(sandbox.root.exists());
}

#[test]
fn local_search_prints_matches_and_honors_result_limit() {
    let sandbox = Sandbox::new("search");
    let search_root = sandbox.path("search tree");
    fs::create_dir_all(search_root.join("nested")).expect("create search fixture tree");
    fs::create_dir(search_root.join("alpha folder")).expect("create directory-only match");
    fs::write(search_root.join("alpha.txt"), b"a").expect("create first search fixture");
    fs::write(search_root.join("beta.txt"), b"bb").expect("create second search fixture");
    fs::write(search_root.join("nested/alpha.log"), b"ccc").expect("create nested search fixture");
    fs::write(search_root.join("match-one.bin"), b"1").expect("create limited fixture");
    fs::write(search_root.join("match-two.bin"), b"2").expect("create limited fixture");

    let substring = run(sandbox
        .command()
        .arg("search")
        .arg(&search_root)
        .arg("alpha"));
    assert_success(&substring);
    let substring_lines: Vec<_> = stdout(&substring).lines().collect();
    assert_eq!(substring_lines.len(), 2);
    assert!(substring_lines
        .iter()
        .any(|line| line.ends_with("\talpha.txt")));
    assert!(substring_lines
        .iter()
        .any(|line| line.ends_with("\tnested/alpha.log")));
    assert!(!stdout(&substring).contains("alpha folder"));

    let glob = run(sandbox
        .command()
        .arg("search")
        .arg(&search_root)
        .arg("*.txt")
        .arg("--glob"));
    assert_success(&glob);
    let glob_lines: Vec<_> = stdout(&glob).lines().collect();
    assert_eq!(glob_lines.len(), 2);
    assert!(glob_lines.iter().any(|line| line.ends_with("\talpha.txt")));
    assert!(glob_lines.iter().any(|line| line.ends_with("\tbeta.txt")));

    let limited = run(sandbox
        .command()
        .arg("search")
        .arg(&search_root)
        .arg("match-")
        .arg("--max-results")
        .arg("1"));
    assert_success(&limited);
    let limited_lines: Vec<_> = stdout(&limited).lines().collect();
    assert_eq!(limited_lines.len(), 1);
    assert!(limited_lines[0].contains("match-"));

    let missing = run(sandbox
        .command()
        .arg("search")
        .arg(sandbox.path("missing"))
        .arg(os("anything")));
    assert_exit_code(&missing, 1);
    assert!(stderr(&missing).starts_with("se: "));
}
