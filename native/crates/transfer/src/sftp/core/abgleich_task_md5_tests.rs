//! `content_md5` over SFTP without an agent: the SSH server forms the MD5
//! (`md5sum` or `md5 -q`) and sends 32 hex digits, the file content is not
//! read through SFTP. The script, the reading of its output and the flow are
//! checked without a network; one test runs the exec request over a real SSH
//! exchange against the in-process server of extra_link_tests.rs.
use super::digest_command::{
    command_line, hash_by, interpret_for, stdin_for, Algorithm, DigestGate, Reply, MARKER,
    MD5_SCRIPT,
};
use super::digest_exec::{capture, feed};
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

fn ok_line(size: u64, digest: &str) -> String {
    format!("{MARKER} ok {size} {digest}\n")
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
fn abgleich_task_sftp_md5_script_is_fixed_and_free_of_openssl() {
    let command = command_line(Algorithm::Md5);
    assert_eq!(command, format!("sh -c '{MD5_SCRIPT}'"));
    assert!(MD5_SCRIPT.contains("h=md5sum"), "{MD5_SCRIPT}");
    assert!(MD5_SCRIPT.contains("h=\"md5 -q\""), "{MD5_SCRIPT}");
    assert!(MD5_SCRIPT.contains("$($h <\"$f\")"), "{MD5_SCRIPT}");
    assert!(!MD5_SCRIPT.contains("openssl"), "{MD5_SCRIPT}");
    assert!(
        !MD5_SCRIPT.contains("sha"),
        "the MD5 script asks for no SHA tool"
    );
    assert_ne!(command, command_line(Algorithm::Sha256));
}

#[test]
fn abgleich_task_sftp_md5_reads_md5sum_and_bsd_output() {
    for digest in [
        format!("{MD5} -"),
        format!("\\{MD5} -"),
        MD5.to_ascii_uppercase(),
        MD5.to_string(),
    ] {
        assert_eq!(
            interpret_for(&captured(Some(0), &ok_line(5, &digest)), Algorithm::Md5),
            Reply::Digest {
                hex: MD5.into(),
                size: 5
            },
            "{digest:?}"
        );
    }
    // A SHA-256 line is no MD5 and the other way round.
    assert_eq!(
        interpret_for(&captured(Some(0), &ok_line(5, SHA)), Algorithm::Md5),
        Reply::Unavailable
    );
    assert_eq!(
        interpret_for(&captured(Some(0), &ok_line(5, MD5)), Algorithm::Sha256),
        Reply::Unavailable
    );
    assert_eq!(
        interpret_for(
            &captured(Some(0), &format!("{MARKER} notool\n")),
            Algorithm::Md5
        ),
        Reply::Unavailable
    );
    assert_eq!(
        interpret_for(
            &captured(Some(0), &format!("{MARKER} none\n")),
            Algorithm::Md5
        ),
        Reply::NotRegular
    );
    assert_eq!(
        interpret_for(&captured(Some(3), ""), Algorithm::Md5),
        Reply::Unavailable,
        "an exit status without the marker proves nothing"
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
        |command, stdin, _| {
            sent.borrow_mut()
                .push((command.to_string(), stdin.to_vec()));
            Ok(Some(captured(Some(0), &ok_line(42, &format!("{MD5} -")))))
        },
    )
    .unwrap();
    assert_eq!(found, Some((MD5.to_string(), 42)));
    let sent = sent.into_inner();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, command_line(Algorithm::Md5));
    assert_eq!(sent[0].1, b"P/data/it's.bin".to_vec());

    let reads = std::cell::Cell::new(0);
    let changed = hash_by(
        &gate,
        Algorithm::Md5,
        "/f",
        || {
            reads.set(reads.get() + 1);
            Ok(file(10, if reads.get() == 1 { 1_000 } else { 2_000 }))
        },
        |_, _, _| Ok(Some(captured(Some(0), &ok_line(10, MD5)))),
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
        |_, _, _| Ok(Some(captured(Some(0), &format!("{MARKER} notool\n")))),
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
        |_, _, _| panic!("a closed gate sends nothing"),
    );
    assert_eq!(again.unwrap(), None);
    let busy = DigestGate::default();
    let result = hash_by(
        &busy,
        Algorithm::Md5,
        "/f",
        || Ok(file(1, 1)),
        |_, _, _| Ok(None),
    );
    assert_eq!(result.unwrap(), None);
    assert!(!busy.closed());
}

#[test]
fn abgleich_task_sftp_md5_comes_back_over_ssh_as_text_not_content() {
    let sshd = Sshd::start();
    known_hosts::pin("127.0.0.1", sshd.port, &sshd.fingerprint, true).expect("pin");
    *sshd.script.lock().unwrap() = ExecScript::UntilEof {
        stdout: ok_line(9, &format!("{MD5} -")),
        status: 0,
    };
    let (_handle, _method, mut channel) = sshd
        .extra(sshd.main_key(&sshd.fingerprint))
        .expect("logged in");
    let stdin = stdin_for("/srv/it's here.bin").expect("sendable");
    sshd.runtime
        .block_on(channel.exec(true, command_line(Algorithm::Md5).into_bytes()))
        .expect("exec request sent");
    let cancel = AtomicBool::new(false);
    let result = sshd
        .runtime
        .block_on(async {
            feed(&channel, &stdin).await;
            capture(&mut channel, Duration::from_secs(20), &cancel).await
        })
        .expect("the command ended");
    assert_eq!(
        interpret_for(&result, Algorithm::Md5),
        Reply::Digest {
            hex: MD5.into(),
            size: 9
        }
    );
    let commands = sshd.log.commands.lock().unwrap().clone();
    assert_eq!(commands, vec![command_line(Algorithm::Md5)]);
    assert_eq!(*sshd.log.stdin.lock().unwrap(), stdin);
}
