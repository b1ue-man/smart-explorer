//! The hash command over a real SSH exchange: an in-process SSH server
//! (extra_link_tests.rs) answers the exec request, so the request, the path
//! on standard input, the EOF, the output, the exit status, the cancel
//! signal and the wait cross russh's wire encoding. The file content never
//! exists in these tests: the server only prints what the script would.
use super::digest_command::{
    command_for, digest_by, interpret, stdin_for, DigestGate, Reply, MARKER,
};
use super::digest_exec::{capture, feed};
use super::exec::CapturedExec;
use super::extra_link_tests::{ExecScript, Sshd};
use super::known_hosts;
use crate::vfs::VfsMeta;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const HASH: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

/// Logs in, sends the fixed command and the path with EOF.
fn started(
    sshd: &Sshd,
    script: ExecScript,
    path: &str,
) -> (impl Sized, russh::Channel<russh::client::Msg>) {
    known_hosts::pin("127.0.0.1", sshd.port, &sshd.fingerprint, true).expect("pin");
    *sshd.script.lock().unwrap() = script;
    let (handle, _method, channel) = sshd
        .extra(sshd.main_key(&sshd.fingerprint))
        .expect("logged in");
    let stdin = stdin_for(path).expect("sendable");
    sshd.runtime.block_on(async {
        channel
            .exec(true, command_for().into_bytes())
            .await
            .expect("exec request sent");
        feed(&channel, &stdin).await;
    });
    (handle, channel)
}

fn waited(
    sshd: &Sshd,
    channel: &mut russh::Channel<russh::client::Msg>,
    wait: Duration,
) -> std::io::Result<CapturedExec> {
    let cancel = AtomicBool::new(false);
    sshd.runtime.block_on(capture(channel, wait, &cancel))
}

#[test]
fn abgleich_task_sftp_exec_digest_comes_back_as_text_not_content() {
    let sshd = Sshd::start();
    let path = "/srv/it's a\\'; id; # $(id)\nnext";
    let (_handle, mut channel) = started(
        &sshd,
        ExecScript::UntilEof {
            stdout: format!("{MARKER} ok 3 {HASH} -\n"),
            status: 0,
        },
        path,
    );
    let captured = waited(&sshd, &mut channel, Duration::from_secs(20)).expect("ended");
    assert_eq!(
        interpret(&captured),
        Reply::Digest {
            hex: HASH.into(),
            size: 3
        }
    );
    let commands = sshd.log.commands.lock().unwrap().clone();
    assert_eq!(commands, vec![command_for()], "the fixed text, no path");
    assert_eq!(
        *sshd.log.stdin.lock().unwrap(),
        stdin_for(path).unwrap(),
        "the path travelled on standard input, byte for byte"
    );
}

#[test]
fn abgleich_task_sftp_exec_rejected_request_is_a_permission_error() {
    let sshd = Sshd::start();
    let (_handle, mut channel) = started(&sshd, ExecScript::Reject, "/f");
    let error = waited(&sshd, &mut channel, Duration::from_secs(20))
        .expect_err("the server refused to run commands");
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
}

/// An account whose forced command is an SFTP server (`ForceCommand
/// internal-sftp`, an external `sftp-server`): it reads the input as SFTP
/// packets and ends at the EOF or the oversized length (status 11). The
/// attempt ends at once, not at the time limit, and closes the gate.
#[test]
fn abgleich_task_sftp_exec_sftp_only_account_ends_at_once_and_closes_the_gate() {
    let sshd = Sshd::start();
    for status in [11, 0, 1] {
        let (_handle, mut channel) = started(
            &sshd,
            ExecScript::UntilEof {
                stdout: String::new(),
                status,
            },
            "/f",
        );
        let began = Instant::now();
        let captured = waited(&sshd, &mut channel, Duration::from_secs(60)).expect("ended");
        assert!(began.elapsed() < Duration::from_secs(10), "no 60 s wait");
        assert_eq!(interpret(&captured), Reply::Unavailable, "{status}");
    }

    // The whole call: one attempt, then the connection asks no more.
    let gate = DigestGate::default();
    let began = Instant::now();
    let result = digest_by(
        &gate,
        "/srv/file",
        || {
            Ok(VfsMeta {
                size: 5,
                mtime_ms: 1,
                ..VfsMeta::default()
            })
        },
        |command, stdin, wait| {
            *sshd.script.lock().unwrap() = ExecScript::UntilEof {
                stdout: String::new(),
                status: 11,
            };
            let (_handle, _method, mut channel) = sshd
                .extra(sshd.main_key(&sshd.fingerprint))
                .expect("logged in");
            let cancel = AtomicBool::new(false);
            sshd.runtime
                .block_on(async {
                    channel
                        .exec(true, command.as_bytes().to_vec())
                        .await
                        .map_err(|error| std::io::Error::other(error.to_string()))?;
                    feed(&channel, stdin).await;
                    capture(&mut channel, wait, &cancel).await
                })
                .map(Some)
        },
    );
    assert_eq!(result.unwrap(), None, "the caller moves the content");
    assert!(gate.closed());
    assert!(began.elapsed() < Duration::from_secs(10));
}

#[test]
fn abgleich_task_sftp_exec_wait_ends_a_command_that_never_answers() {
    let sshd = Sshd::start();
    let (_handle, mut channel) = started(&sshd, ExecScript::Hang, "/f");
    let began = Instant::now();
    let error = waited(&sshd, &mut channel, Duration::from_millis(600)).expect_err("no answer");
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    assert!(began.elapsed() < Duration::from_secs(10));
}

#[test]
fn abgleich_task_sftp_exec_cancel_sends_term_and_ends_the_wait() {
    let sshd = Sshd::start();
    let (_handle, mut channel) = started(&sshd, ExecScript::Hang, "/f");
    let cancel = AtomicBool::new(false);
    let began = Instant::now();
    let error = std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(Duration::from_millis(300));
            cancel.store(true, Ordering::Relaxed);
        });
        sshd.runtime
            .block_on(capture(&mut channel, Duration::from_secs(60), &cancel))
            .expect_err("canceled")
    });
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    assert!(began.elapsed() < Duration::from_secs(10), "no 60 s wait");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(sshd.log.signals.lock().unwrap().as_slice(), ["TERM"]);
}
