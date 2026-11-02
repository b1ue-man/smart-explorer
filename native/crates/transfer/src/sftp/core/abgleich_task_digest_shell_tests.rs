//! The fixed hash command through real login shells, the way sshd runs it
//! (`<shell> -c <command line>`, OpenSSH `session.c` `do_child`), with the
//! path on standard input: every shell installed on the runner (always
//! `/bin/sh`; bash, dash, zsh, ksh, mksh, fish, tcsh and csh where present)
//! must hash the same file for names with `'`, `\`, `$`, backquotes, spaces,
//! newlines and a leading `-`, and no name may run anything.
use super::super::*;
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
    "line\nbreak",
    "ends with newline\n",
    "-rf",
    "x",
    "%x%",
];

/// The installed shells, each once (a link counts as the shell it names).
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

/// Runs `command` with `shell -c`, `stdin` on its input, in `dir`.
fn run(shell: &str, command: &str, stdin: &[u8], dir: &Path) -> CapturedExec {
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
        .expect("send the path");
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

fn sha256_hex(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn abgleich_task_sftp_digest_fixed_command_hashes_the_same_in_every_login_shell() {
    let dir = tempfile::tempdir().expect("temp dir");
    for (index, name) in NAMES.iter().enumerate() {
        let content = format!("content {index} of {name:?}");
        std::fs::write(dir.path().join(name), &content).expect("write");
        let path = dir.path().join(name);
        let path = path.to_str().expect("UTF-8 path");
        let stdin = stdin_for(path).expect("sendable");
        for shell in shells() {
            for (algorithm, expected) in [
                (Algorithm::Sha256, sha256_hex(content.as_bytes())),
                (
                    Algorithm::Md5,
                    format!("{:x}", md5::compute(content.as_bytes())),
                ),
            ] {
                let captured = run(shell, &command_line(algorithm), &stdin, dir.path());
                assert_eq!(
                    interpret_for(&captured, algorithm),
                    Reply::Digest {
                        hex: expected,
                        size: content.len() as u64
                    },
                    "{shell} {algorithm:?} {name:?}: {:?} {:?}",
                    String::from_utf8_lossy(&captured.stdout),
                    String::from_utf8_lossy(&captured.stderr)
                );
            }
        }
    }
    assert!(
        !dir.path().join("canary").exists(),
        "no file name ran a command"
    );
}

#[test]
fn abgleich_task_sftp_digest_fixed_command_reports_folders_and_links_as_none() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir(dir.path().join("folder")).expect("folder");
    std::fs::write(dir.path().join("target"), b"t").expect("file");
    std::os::unix::fs::symlink(dir.path().join("target"), dir.path().join("link")).expect("link");
    for name in ["folder", "link", "missing"] {
        let path = dir.path().join(name);
        let stdin = stdin_for(path.to_str().expect("UTF-8")).expect("sendable");
        for shell in shells() {
            let captured = run(shell, &command_for(), &stdin, dir.path());
            assert_eq!(interpret(&captured), Reply::NotRegular, "{shell} {name}");
        }
    }
}
