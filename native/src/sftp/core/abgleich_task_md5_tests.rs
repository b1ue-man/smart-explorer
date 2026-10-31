//! `content_md5` over SFTP without an agent: the SSH server forms the MD5
//! (`md5sum` or `md5 -q`) and sends 32 hex digits, the file content is not
//! read through SFTP. The script, the reading of its output and the flow are
//! checked without a network; one test runs the exec request over a real SSH
//! exchange against the in-process server of extra_link_tests.rs.
use super::digest_command::{
    command_for_algorithm, hash_by, interpret_for, Algorithm, DigestGate, Reply,
};
use super::digest_exec::capture;
use super::exec::CapturedExec;
use super::extra_link_tests::{ExecScript, Sshd};
use super::known_hosts;
use crate::vfs::VfsMeta;
use std::cell::RefCell;
use std::io;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

const MD5: &str = "5d41402abc4b2a76b9719d911017c592";
const SHA: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

fn captured(status: Option<u32>, stdout: &str) -> CapturedExec {
    CapturedExec {
        stdout: stdout.as_bytes().to_vec(),
        exit_status: status,
        ..CapturedExec::default()
    }
}

fn file(size: u64, mtime_ms: i64) -> VfsMeta {
    VfsMeta {
        size,
        mtime_ms,
        name: "f".into(),
        ..VfsMeta::default()
    }
}

#[test]
fn abgleich_task_sftp_md5_script_is_fixed_quoted_and_free_of_openssl() {
    let command = command_for_algorithm(Algorithm::Md5, "-rf /; echo $HOME 'x'");
    assert!(command.starts_with("sh -c '"), "{command}");
    assert!(
        command.ends_with(" sh '-rf /; echo $HOME '\\''x'\\'''"),
        "the path travels as one quoted word: {command}"
    );
    let script = command
        .strip_suffix(" sh '-rf /; echo $HOME '\\''x'\\'''")
        .unwrap();
    let script = script
        .strip_prefix("sh -c '")
        .unwrap()
        .strip_suffix('\'')
        .unwrap();
    assert!(!script.contains('\''), "no quote inside the quoted script");
    assert!(!script.contains('!'), "nothing a csh would read as history");
    assert!(script.contains("md5sum <\"$f\""), "{script}");
    assert!(script.contains("md5 -q <\"$f\""), "{script}");
    assert!(!script.contains("openssl"), "{script}");
    assert!(
        !script.contains("sha"),
        "the MD5 script asks for no SHA tool"
    );
    assert_ne!(
        command,
        command_for_algorithm(Algorithm::Sha256, "-rf /; echo $HOME 'x'")
    );
}

#[test]
fn abgleich_task_sftp_md5_reads_md5sum_and_bsd_output() {
    for text in [
        format!("{MD5}  -\n"),
        format!("\\{MD5}  -\n"),
        format!("{}  -\n", MD5.to_ascii_uppercase()),
        format!("{MD5}\n"),
    ] {
        assert_eq!(
            interpret_for(&captured(Some(0), &text), Algorithm::Md5),
            Reply::Digest(MD5.into()),
            "{text:?}"
        );
    }
    // A SHA-256 line is no MD5 and the other way round.
    assert_eq!(
        interpret_for(&captured(Some(0), &format!("{SHA}  -\n")), Algorithm::Md5),
        Reply::Unavailable
    );
    assert_eq!(
        interpret_for(
            &captured(Some(0), &format!("{MD5}  -\n")),
            Algorithm::Sha256
        ),
        Reply::Unavailable
    );
    assert_eq!(
        interpret_for(&captured(Some(127), ""), Algorithm::Md5),
        Reply::Unavailable
    );
    assert_eq!(
        interpret_for(&captured(Some(3), ""), Algorithm::Md5),
        Reply::NotRegular
    );
}

#[test]
fn abgleich_task_sftp_md5_flow_asks_the_server_and_checks_size_and_time() {
    let gate = DigestGate::default();
    let sent = RefCell::new(Vec::new());
    let found = hash_by(
        &gate,
        Algorithm::Md5,
        "/data/it's.bin",
        || Ok(file(42, 5_000)),
        |command, _| {
            sent.borrow_mut().push(command.to_string());
            Ok(Some(captured(Some(0), &format!("{MD5}  -\n"))))
        },
    )
    .unwrap();
    assert_eq!(found, Some((MD5.to_string(), 42)));
    let sent = sent.into_inner();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("md5sum"), "{}", sent[0]);
    assert!(sent[0].ends_with(" sh '/data/it'\\''s.bin'"), "{}", sent[0]);

    let reads = std::cell::Cell::new(0);
    let changed = hash_by(
        &gate,
        Algorithm::Md5,
        "/f",
        || {
            reads.set(reads.get() + 1);
            Ok(file(10, if reads.get() == 1 { 1_000 } else { 2_000 }))
        },
        |_, _| Ok(Some(captured(Some(0), MD5))),
    );
    assert_eq!(changed.unwrap_err().kind(), io::ErrorKind::InvalidData);
}

#[test]
fn abgleich_task_sftp_md5_without_a_tool_leaves_the_content_to_the_caller() {
    let md5_gate = DigestGate::default();
    let sha_gate = DigestGate::default();
    let none = hash_by(
        &md5_gate,
        Algorithm::Md5,
        "/f",
        || Ok(file(1, 1)),
        |_, _| Ok(Some(captured(Some(127), ""))),
    );
    assert_eq!(none.unwrap(), None);
    assert!(
        md5_gate.closed(),
        "a host without MD5 tools is not asked again"
    );
    assert!(!sha_gate.closed(), "the SHA-256 gate is its own");
    let again = hash_by(
        &md5_gate,
        Algorithm::Md5,
        "/f",
        || panic!("a closed gate looks at nothing"),
        |_, _| panic!("a closed gate sends nothing"),
    );
    assert_eq!(again.unwrap(), None);
    let busy = DigestGate::default();
    let result = hash_by(
        &busy,
        Algorithm::Md5,
        "/f",
        || Ok(file(1, 1)),
        |_, _| Ok(None),
    );
    assert_eq!(result.unwrap(), None);
    assert!(!busy.closed());
}

#[test]
fn abgleich_task_sftp_md5_comes_back_over_ssh_as_text_not_content() {
    let sshd = Sshd::start();
    known_hosts::pin("127.0.0.1", sshd.port, &sshd.fingerprint, true).expect("pin");
    *sshd.script.lock().unwrap() = ExecScript::Output {
        stdout: format!("{MD5}  -\n"),
        status: 0,
    };
    let (_handle, _method, mut channel) = sshd
        .extra(sshd.main_key(&sshd.fingerprint))
        .expect("logged in");
    sshd.runtime
        .block_on(channel.exec(
            true,
            command_for_algorithm(Algorithm::Md5, "/srv/it's here.bin").into_bytes(),
        ))
        .expect("exec request sent");
    let cancel = AtomicBool::new(false);
    let result = sshd
        .runtime
        .block_on(capture(&mut channel, Duration::from_secs(20), &cancel))
        .expect("the command ended");
    assert_eq!(
        interpret_for(&result, Algorithm::Md5),
        Reply::Digest(MD5.into())
    );
    let commands = sshd.log.commands.lock().unwrap().clone();
    assert_eq!(commands.len(), 1);
    assert!(commands[0].contains("md5sum"), "{}", commands[0]);
    assert!(
        commands[0].ends_with(" sh '/srv/it'\\''s here.bin'"),
        "{}",
        commands[0]
    );
}
