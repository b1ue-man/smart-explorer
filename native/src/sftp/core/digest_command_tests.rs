use super::*;
use std::cell::{Cell, RefCell};

const HASH: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

fn captured(status: Option<u32>, stdout: &str, stderr: &str) -> CapturedExec {
    CapturedExec {
        stdout: stdout.as_bytes().to_vec(),
        stderr: stderr.as_bytes().to_vec(),
        exit_status: status,
        ..CapturedExec::default()
    }
}

/// What the script prints for a file of `size` bytes and `digest`.
fn ok_line(size: u64, digest: &str) -> String {
    format!("{MARKER} ok {size} {digest}\n")
}

fn file(size: u64, mtime_ms: i64) -> VfsMeta {
    VfsMeta {
        is_dir: false,
        is_symlink: false,
        special: false,
        size,
        mtime_ms,
        btime_ms: 0,
        hidden: false,
        system: false,
        name: "f".into(),
        id: None,
        content_md5: None,
    }
}

#[test]
fn abgleich_task_sftp_digest_command_line_is_fixed_text_without_any_path() {
    let command = command_for();
    assert_eq!(command, format!("sh -c '{SCRIPT}'"));
    for script in [SCRIPT, MD5_SCRIPT] {
        for forbidden in ['\'', '\\', '!', '\n', '\r', '\0'] {
            assert!(
                !script.contains(forbidden),
                "{forbidden:?}: no login shell may read anything into it"
            );
        }
        assert!(!script.contains(MARKER), "the marker is built at run time");
        assert!(script.contains("$(cat; echo x)"), "the path comes on stdin");
        assert!(
            script.contains("<\"$f\""),
            "the file is read through a redirect"
        );
    }
    // The same text for every path: nothing of the path is in it.
    assert_eq!(command_line(Algorithm::Sha256), command);
    assert_ne!(command_line(Algorithm::Md5), command);
}

#[test]
fn abgleich_task_sftp_digest_path_goes_to_stdin_byte_for_byte() {
    for path in [
        "/a/b",
        "/srv/it's.bin",
        "/srv/a\\'; id >/tmp/x; #",
        "/srv/$(id)`id`$HOME",
        "/srv/with space",
        "/srv/line\nbreak\n",
        "/srv/cr\rhere",
        "-rf /",
    ] {
        let mut expected = vec![STDIN_PREFIX];
        expected.extend_from_slice(path.as_bytes());
        assert_eq!(stdin_for(path), Some(expected), "{path:?}");
    }
    assert_eq!(stdin_for(""), None);
    assert_eq!(stdin_for("a\0b"), None, "no shell variable holds a NUL");
}

#[test]
fn abgleich_task_sftp_digest_reads_the_marker_line_of_each_tool() {
    for digest in [
        format!("{HASH} -"),
        format!("\\{HASH} -"),
        HASH.to_ascii_uppercase(),
        HASH.to_string(),
    ] {
        assert_eq!(
            interpret(&captured(Some(0), &ok_line(42, &digest), "")),
            Reply::Digest {
                hex: HASH.into(),
                size: 42
            },
            "{digest:?}"
        );
    }
    // A login shell that prints something first (a .bashrc greeting).
    let greeted = format!("Welcome\n{}", ok_line(7, HASH));
    assert_eq!(
        interpret(&captured(Some(0), &greeted, "")),
        Reply::Digest {
            hex: HASH.into(),
            size: 7
        }
    );
}

#[test]
fn abgleich_task_sftp_digest_answers_mean_what_the_script_says() {
    let line = |word: &str| format!("{MARKER} {word}\n");
    assert_eq!(
        interpret(&captured(Some(0), &line("none"), "")),
        Reply::NotRegular
    );
    assert_eq!(
        interpret(&captured(Some(0), &line("notool"), "")),
        Reply::Unavailable
    );
    assert!(matches!(
        interpret(&captured(Some(0), &line("fail"), "Permission denied")),
        Reply::Failed(text) if text.contains("Permission denied")
    ));
    // Marker present but no digest of this algorithm.
    assert_eq!(
        interpret(&captured(Some(0), &ok_line(1, "xyz"), "")),
        Reply::Unavailable
    );
    // Marker present, but the shell ended badly afterwards.
    assert!(matches!(
        interpret(&captured(Some(1), &ok_line(1, HASH), "")),
        Reply::Failed(_)
    ));
    let mut signaled = captured(None, &ok_line(1, HASH), "");
    signaled.exit_signal = Some("KILL".into());
    assert!(matches!(interpret(&signaled), Reply::Failed(_)));
    assert!(matches!(
        interpret(&captured(None, "", "")),
        Reply::Failed(_)
    ));
}

/// Shells that do not run the text as POSIX sh print no marker, whatever
/// their exit status: the connection cannot hash, nothing changed.
#[test]
fn abgleich_task_sftp_digest_without_marker_is_unavailable_never_changed() {
    let replies = [
        // cmd.exe without sh in PATH.
        captured(
            Some(9009),
            "",
            "'sh' is not recognized as an internal or external command,",
        ),
        // cmd.exe split at `||`, the rest ran `exit 3`-like fragments.
        captured(Some(3), "\"$m\" none\r\n", ""),
        captured(Some(1), "", "The system cannot find the path specified."),
        // PowerShell without sh.
        captured(
            Some(1),
            "",
            "sh : The term 'sh' is not recognized as the name of a cmdlet",
        ),
        // A forced command that echoes the original command.
        captured(Some(0), &format!("{}\n", command_for()), ""),
        // A banner, then success.
        captured(Some(0), "SSH-2.0 banner\n", ""),
        // An SFTP server that took the input for a packet (status 11) or
        // read EOF (status 0), and OpenSSH's "sftp connections only".
        captured(Some(11), "", ""),
        captured(Some(0), "", ""),
        captured(Some(1), "This service allows sftp connections only.\n", ""),
        // Exit statuses the old script used.
        captured(Some(127), "", ""),
        captured(Some(126), "", ""),
    ];
    for reply in replies {
        assert_eq!(interpret(&reply), Reply::Unavailable, "{:?}", reply.stdout);
        let gate = DigestGate::default();
        let result = digest_by(
            &gate,
            "/f",
            || Ok(file(1, 1)),
            |_, _, _| {
                Ok(Some(CapturedExec {
                    stdout: reply.stdout.clone(),
                    stderr: reply.stderr.clone(),
                    exit_status: reply.exit_status,
                    ..CapturedExec::default()
                }))
            },
        );
        assert_eq!(result.unwrap(), None, "the caller moves the content");
        assert!(gate.closed(), "not asked again on this connection");
    }
    let mut cut = captured(Some(0), "x", "");
    cut.stdout_truncated = true;
    assert_eq!(interpret(&cut), Reply::Unavailable);
}

#[test]
fn abgleich_task_sftp_digest_waits_longer_for_bigger_files() {
    assert_eq!(wait_for(0), BASE_WAIT);
    assert_eq!(
        wait_for(8 * 1024 * 1024 * 100),
        BASE_WAIT + Duration::from_secs(100)
    );
}

#[test]
fn abgleich_task_sftp_digest_flow_asks_the_server_not_the_content() {
    let gate = DigestGate::default();
    let sent = RefCell::new(Vec::new());
    let digest = digest_by(
        &gate,
        "/data/it's.bin",
        || Ok(file(42, 5_000)),
        |command, stdin, wait| {
            sent.borrow_mut()
                .push((command.to_string(), stdin.to_vec(), wait));
            Ok(Some(captured(
                Some(0),
                &ok_line(42, &format!("{HASH} -")),
                "",
            )))
        },
    )
    .unwrap()
    .expect("a digest");
    assert_eq!(digest.sha256, HASH);
    assert_eq!(digest.size, 42, "the size the SFTP attributes showed");
    let sent = sent.into_inner();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, command_for(), "the fixed command line");
    assert_eq!(sent[0].1, b"P/data/it's.bin".to_vec());
    assert_eq!(sent[0].2, wait_for(42));
}

#[test]
fn abgleich_task_sftp_digest_flow_detects_a_file_that_changed_while_hashed() {
    let gate = DigestGate::default();
    let reads = Cell::new(0);
    let result = digest_by(
        &gate,
        "/f",
        || {
            reads.set(reads.get() + 1);
            Ok(file(10, if reads.get() == 1 { 1_000 } else { 2_000 }))
        },
        |_, _, _| Ok(Some(captured(Some(0), &ok_line(10, HASH), ""))),
    );
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    let resized = Cell::new(0);
    let result = digest_by(
        &gate,
        "/f",
        || {
            resized.set(resized.get() + 1);
            Ok(file(10 + resized.get(), 1_000))
        },
        |_, _, _| Ok(Some(captured(Some(0), &ok_line(11, HASH), ""))),
    );
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    // The script saw no regular file and SFTP shows another time: changed.
    let moved_reads = Cell::new(0);
    let gone = digest_by(
        &gate,
        "/f",
        || {
            moved_reads.set(moved_reads.get() + 1);
            Ok(file(10, if moved_reads.get() == 1 { 1_000 } else { 9_000 }))
        },
        |_, _, _| Ok(Some(captured(Some(0), &format!("{MARKER} none\n"), ""))),
    );
    assert_eq!(gone.unwrap_err().kind(), io::ErrorKind::InvalidData);
    assert!(!gate.closed());
}

/// Only SFTP attributes that differ mean "changed": a shell that sees no
/// regular file or another size under the path while SFTP shows the same
/// file sees another file system (another root), so it cannot hash here.
#[test]
fn abgleich_task_sftp_digest_other_view_of_the_shell_is_unavailable() {
    let not_regular = DigestGate::default();
    let result = digest_by(
        &not_regular,
        "/f",
        || Ok(file(10, 1_000)),
        |_, _, _| Ok(Some(captured(Some(0), &format!("{MARKER} none\n"), ""))),
    );
    assert_eq!(result.unwrap(), None);
    assert!(not_regular.closed());

    let other_size = DigestGate::default();
    let result = digest_by(
        &other_size,
        "/f",
        || Ok(file(10, 1_000)),
        |_, _, _| Ok(Some(captured(Some(0), &ok_line(99, HASH), ""))),
    );
    assert_eq!(result.unwrap(), None, "no digest of another file");
    assert!(other_size.closed());
}

#[test]
fn abgleich_task_sftp_digest_flow_refuses_folders_and_links_before_any_command() {
    let gate = DigestGate::default();
    for (dir, link, special) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        let mut meta = file(0, 0);
        meta.is_dir = dir;
        meta.is_symlink = link;
        meta.special = special;
        let result = digest_by(
            &gate,
            "/x",
            || Ok(meta.clone()),
            |_, _, _| panic!("no command for a non-regular entry"),
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }
    let result = digest_by(
        &gate,
        "/a\0b",
        || panic!("a path with NUL is not looked at"),
        |_, _, _| panic!("a path with NUL is not sent"),
    );
    assert_eq!(result.unwrap(), None);
}

#[test]
fn abgleich_task_sftp_digest_flow_without_commands_leaves_the_content_to_the_caller() {
    let rejected = DigestGate::default();
    let first = digest_by(
        &rejected,
        "/f",
        || Ok(file(1, 1)),
        |_, _, _| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "exec rejected",
            ))
        },
    );
    assert_eq!(first.unwrap(), None);
    assert!(
        rejected.closed(),
        "an account without commands is not asked again"
    );
    let again = digest_by(
        &rejected,
        "/f",
        || panic!("a closed gate looks at nothing"),
        |_, _, _| panic!("a closed gate sends nothing"),
    );
    assert_eq!(again.unwrap(), None);

    let no_tool = DigestGate::default();
    let result = digest_by(
        &no_tool,
        "/f",
        || Ok(file(1, 1)),
        |_, _, _| Ok(Some(captured(Some(0), &format!("{MARKER} notool\n"), ""))),
    );
    assert_eq!(result.unwrap(), None);
    assert!(no_tool.closed());

    let busy = DigestGate::default();
    let result = digest_by(&busy, "/f", || Ok(file(1, 1)), |_, _, _| Ok(None));
    assert_eq!(result.unwrap(), None);
    assert!(!busy.closed(), "a full session limit is passing");
}

#[test]
fn abgleich_task_sftp_digest_flow_gives_up_after_three_failures_in_a_row() {
    let gate = DigestGate::default();
    let fail = |gate: &DigestGate| {
        digest_by(
            gate,
            "/f",
            || Ok(file(1, 1)),
            |_, _, _| {
                Ok(Some(captured(
                    Some(0),
                    &format!("{MARKER} fail\n"),
                    "Permission denied",
                )))
            },
        )
        .unwrap()
    };
    assert_eq!(fail(&gate), None);
    assert_eq!(fail(&gate), None);
    assert!(!gate.closed());
    // A success in between starts the count again.
    let ok = digest_by(
        &gate,
        "/f",
        || Ok(file(1, 1)),
        |_, _, _| Ok(Some(captured(Some(0), &ok_line(1, HASH), ""))),
    );
    assert!(ok.unwrap().is_some());
    assert_eq!(fail(&gate), None);
    assert_eq!(fail(&gate), None);
    assert!(!gate.closed());
    assert_eq!(fail(&gate), None);
    assert!(gate.closed());

    let slow = DigestGate::default();
    let timed_out = digest_by(
        &slow,
        "/f",
        || Ok(file(1, 1)),
        |_, _, _| Err(io::Error::new(io::ErrorKind::TimedOut, "slow")),
    );
    assert_eq!(
        timed_out.unwrap(),
        None,
        "the caller moves the content instead"
    );
}

#[cfg(unix)]
#[path = "abgleich_task_digest_shell_tests.rs"]
mod shell_tests;
