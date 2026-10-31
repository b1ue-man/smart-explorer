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
fn abgleich_task_sftp_digest_path_is_one_single_quoted_word() {
    assert_eq!(sh_quote("/a/b"), "'/a/b'");
    assert_eq!(sh_quote("a'b; rm -rf /"), "'a'\\''b; rm -rf /'");
    assert_eq!(sh_quote("$(id)`id`\"x\""), "'$(id)`id`\"x\"'");
    let command = command_for("-rf /; echo $HOME 'x'");
    assert!(command.starts_with("sh -c '"), "{command}");
    assert!(
        command.ends_with(" sh '-rf /; echo $HOME '\\''x'\\'''"),
        "{command}"
    );
    assert!(!SCRIPT.contains('\''), "no quote inside the quoted script");
    assert!(!SCRIPT.contains('!'), "nothing a csh would read as history");
    assert!(
        SCRIPT.contains("<\"$f\""),
        "the file is read through a redirect"
    );
}

#[test]
fn abgleich_task_sftp_digest_refuses_paths_that_are_no_single_word() {
    assert!(sendable("/home/a b/c"));
    assert!(sendable("-x"));
    assert!(!sendable(""));
    assert!(!sendable("a\nb"));
    assert!(!sendable("a\rb"));
    assert!(!sendable("a\0b"));
}

#[test]
fn abgleich_task_sftp_digest_reads_sha256sum_shasum_and_bsd_output() {
    for text in [
        format!("{HASH}  -\n"),
        format!("\\{HASH}  -\n"),
        format!("{}  -\n", HASH.to_ascii_uppercase()),
        format!("{HASH}\n"),
    ] {
        assert_eq!(
            interpret(&captured(Some(0), &text, "")),
            Reply::Digest(HASH.into()),
            "{text:?}"
        );
    }
}

#[test]
fn abgleich_task_sftp_digest_exit_codes_mean_what_the_script_says() {
    assert_eq!(interpret(&captured(Some(3), "", "")), Reply::NotRegular);
    assert_eq!(interpret(&captured(Some(127), "", "")), Reply::Unavailable);
    assert_eq!(interpret(&captured(Some(126), "", "")), Reply::Unavailable);
    // Success without a digest: a shell that took the command for something
    // else (forced command, Windows shell).
    assert_eq!(
        interpret(&captured(Some(0), "SSH-2.0 banner\n", "")),
        Reply::Unavailable
    );
    assert_eq!(interpret(&captured(Some(0), "", "")), Reply::Unavailable);
    assert!(matches!(
        interpret(&captured(Some(1), "", "Permission denied")),
        Reply::Failed(text) if text.contains("Permission denied")
    ));
    assert!(matches!(
        interpret(&captured(None, "", "")),
        Reply::Failed(_)
    ));
    let mut signaled = captured(None, "", "");
    signaled.exit_signal = Some("KILL".into());
    assert!(matches!(interpret(&signaled), Reply::Failed(_)));
    let mut cut = captured(Some(0), HASH, "");
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
        |command, wait| {
            sent.borrow_mut().push((command.to_string(), wait));
            Ok(Some(captured(Some(0), &format!("{HASH}  -\n"), "")))
        },
    )
    .unwrap()
    .expect("a digest");
    assert_eq!(digest.sha256, HASH);
    assert_eq!(digest.size, 42, "the size the SFTP attributes showed");
    let sent = sent.into_inner();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0].0.ends_with(" sh '/data/it'\\''s.bin'"),
        "{}",
        sent[0].0
    );
    assert_eq!(sent[0].1, wait_for(42));
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
        |_, _| Ok(Some(captured(Some(0), HASH, ""))),
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
        |_, _| Ok(Some(captured(Some(0), HASH, ""))),
    );
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    let gone = digest_by(
        &gate,
        "/f",
        || Ok(file(10, 1_000)),
        |_, _| Ok(Some(captured(Some(3), "", ""))),
    );
    assert_eq!(gone.unwrap_err().kind(), io::ErrorKind::InvalidData);
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
            |_, _| panic!("no command for a non-regular entry"),
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }
}

#[test]
fn abgleich_task_sftp_digest_flow_without_commands_leaves_the_content_to_the_caller() {
    let rejected = DigestGate::default();
    let first = digest_by(
        &rejected,
        "/f",
        || Ok(file(1, 1)),
        |_, _| {
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
        |_, _| panic!("a closed gate sends nothing"),
    );
    assert_eq!(again.unwrap(), None);

    let no_tool = DigestGate::default();
    let result = digest_by(
        &no_tool,
        "/f",
        || Ok(file(1, 1)),
        |_, _| Ok(Some(captured(Some(127), "", ""))),
    );
    assert_eq!(result.unwrap(), None);
    assert!(no_tool.closed());

    let busy = DigestGate::default();
    let result = digest_by(&busy, "/f", || Ok(file(1, 1)), |_, _| Ok(None));
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
            |_, _| Ok(Some(captured(Some(1), "", "Permission denied"))),
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
        |_, _| Ok(Some(captured(Some(0), HASH, ""))),
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
        |_, _| Err(io::Error::new(io::ErrorKind::TimedOut, "slow")),
    );
    assert_eq!(
        timed_out.unwrap(),
        None,
        "the caller moves the content instead"
    );
}
