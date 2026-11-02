//! The group script of digest_batch.rs through real login shells, the way
//! sshd runs it (`<shell> -c <command line>`): every installed shell hashes
//! the same files for names with `'`, `\`, `$()`, backquotes, quotes,
//! spaces, newlines and a leading `-`, a missing path is "cannot form" for
//! that path alone, no name runs anything, and the flow sends one exec per
//! group for thousands of files.
#![cfg(unix)]
use super::super::super::exec::CapturedExec;
use super::super::*;
use crate::vfs::VfsMeta;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

const SHELLS: &[&str] = &[
    "/bin/sh",
    "/bin/bash",
    "/usr/bin/bash",
    "/bin/dash",
    "/usr/bin/dash",
    "/bin/zsh",
    "/usr/bin/zsh",
    "/bin/ksh",
    "/usr/bin/ksh",
    "/bin/mksh",
    "/usr/bin/mksh",
    "/usr/bin/fish",
    "/bin/tcsh",
    "/usr/bin/tcsh",
    "/bin/csh",
    "/usr/bin/csh",
];

const NAMES: &[&str] = &[
    "it's.bin",
    "back\\slash",
    "a\\'; touch canary; #",
    "$HOME $(touch canary) `touch canary`",
    "with  two spaces ",
    "q\"uo\"te",
    "line\nbreak",
    "ends with newline\n",
    "-rf",
    "x",
    "%x%",
];

fn shells() -> Vec<&'static str> {
    let mut seen = Vec::new();
    let mut found = Vec::new();
    for shell in SHELLS {
        let Ok(real) = std::fs::canonicalize(shell) else {
            continue;
        };
        if !seen.contains(&real) {
            seen.push(real);
            found.push(*shell);
        }
    }
    assert!(found.contains(&"/bin/sh"), "every runner has /bin/sh");
    found
}

/// `shell -c <command line>` with `stdin`, the way sshd runs it.
pub(super) fn run(shell: &str, command: &str, stdin: &[u8], dir: &Path) -> CapturedExec {
    let mut child = Command::new(shell)
        .arg("-c")
        .arg(command)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the shell");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin)
        .expect("send the paths");
    let output = child.wait_with_output().expect("the shell ended");
    CapturedExec {
        stdout: output.stdout,
        stderr: output.stderr,
        exit_status: output
            .status
            .code()
            .and_then(|code| u32::try_from(code).ok()),
        ..CapturedExec::default()
    }
}

#[test]
fn abgleich_task_sftp_batch_command_hashes_many_paths_in_every_login_shell() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut paths = Vec::new();
    let mut expected = Vec::new();
    for (index, name) in NAMES.iter().enumerate() {
        let content = format!("content {index} of {name:?}");
        let path = dir.path().join(name);
        std::fs::write(&path, &content).expect("write");
        paths.push(path.to_str().expect("UTF-8 path").to_string());
        expected.push(Answer::Digest {
            hex: format!("{:x}", md5::compute(content.as_bytes())),
            size: content.len() as u64,
        });
        if index == 3 {
            // A missing path between the others: "cannot form" for it alone.
            paths.push(dir.path().join("missing").to_str().unwrap().to_string());
            expected.push(Answer::NotRegular);
        }
    }
    std::fs::create_dir(dir.path().join("folder")).expect("folder");
    std::os::unix::fs::symlink(&paths[0], dir.path().join("link")).expect("link");
    for name in ["folder", "link"] {
        paths.push(dir.path().join(name).to_str().unwrap().to_string());
        expected.push(Answer::NotRegular);
    }
    let files: Vec<(usize, &str, u64)> = paths
        .iter()
        .enumerate()
        .map(|(index, path)| (index, path.as_str(), 64))
        .collect();
    let found = groups(&files);
    assert_eq!(found.len(), 1, "one exec for all of them");
    for shell in shells() {
        let captured = run(shell, &batch_command_line(), &found[0].stdin, dir.path());
        assert_eq!(
            interpret_batch(&captured, paths.len()),
            BatchReply::Answers {
                answers: expected.clone(),
                aligned: true
            },
            "{shell}: {:?} {:?}",
            String::from_utf8_lossy(&captured.stdout),
            String::from_utf8_lossy(&captured.stderr)
        );
    }
    assert!(
        !dir.path().join("canary").exists(),
        "no file name ran a command"
    );
}

#[test]
fn abgleich_task_sftp_batch_flow_sends_one_exec_per_group_for_many_files() {
    use super::super::super::digest_batch_flow::{md5_many_by, Gates};
    use super::super::super::digest_command::DigestGate;
    use std::cell::Cell;
    use std::sync::atomic::AtomicBool;
    use std::time::UNIX_EPOCH;

    let dir = tempfile::tempdir().expect("temp dir");
    let count = 2_500;
    let mut paths = Vec::new();
    for index in 0..count {
        let name = if index % 500 == 7 {
            format!("odd name {index}\n")
        } else {
            format!("f {index}.txt")
        };
        let path = dir.path().join(name);
        std::fs::write(&path, format!("{index}")).expect("write");
        paths.push(path.to_str().unwrap().to_string());
    }
    let meta = |path: &str| -> Option<VfsMeta> {
        let found = std::fs::symlink_metadata(path).ok()?;
        let seconds = found
            .modified()
            .ok()?
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_secs();
        Some(VfsMeta {
            size: found.len(),
            mtime_ms: seconds as i64 * 1000,
            is_symlink: found.file_type().is_symlink(),
            is_dir: found.is_dir(),
            ..VfsMeta::default()
        })
    };
    let metas: Vec<VfsMeta> = paths.iter().map(|path| meta(path).unwrap()).collect();
    let files: Vec<(&str, &VfsMeta)> = paths.iter().map(String::as_str).zip(&metas).collect();
    let expected_groups = groups(
        &paths
            .iter()
            .zip(&metas)
            .enumerate()
            .map(|(index, (path, meta))| (index, path.as_str(), meta.size))
            .collect::<Vec<_>>(),
    )
    .len();
    let (md5, batch) = (DigestGate::default(), DigestGate::default());
    let runs = Cell::new(0);
    let found = md5_many_by(
        Gates {
            md5: &md5,
            batch: &batch,
        },
        &files,
        &AtomicBool::new(false),
        |asked| asked.iter().map(|path| meta(path)).collect(),
        |command, stdin, _| {
            runs.set(runs.get() + 1);
            Ok(Some(run("/bin/sh", command, stdin, dir.path())))
        },
    )
    .unwrap();
    for (index, digest) in found.iter().enumerate() {
        assert_eq!(
            digest.as_deref(),
            Some(format!("{:x}", md5::compute(format!("{index}"))).as_str()),
            "{:?}",
            paths[index]
        );
    }
    assert_eq!(
        runs.get(),
        expected_groups,
        "one exec per group, none per file"
    );
    assert!(
        runs.get() * 100 <= count,
        "{} execs for {count} files",
        runs.get()
    );
}
