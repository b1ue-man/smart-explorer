//! The hash command over a real SSH exchange: an in-process SSH server
//! (extra_link_tests.rs) answers the exec request, so the request, the
//! output, the exit status, the cancel signal and the wait cross russh's wire
//! encoding. The file content never exists in these tests: the server only
//! prints what a `sha256sum` would.
use super::digest_command::{command_for, interpret, Reply};
use super::digest_exec::capture;
use super::extra_link_tests::{ExecScript, Sshd};
use super::known_hosts;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const HASH: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

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
    sshd.runtime
        .block_on(channel.exec(true, command_for(path).into_bytes()))
        .expect("exec request sent");
    (handle, channel)
}

#[test]
fn abgleich_task_sftp_exec_digest_comes_back_as_text_not_content() {
    let sshd = Sshd::start();
    let (_handle, mut channel) = started(
        &sshd,
        ExecScript::Output {
            stdout: format!("{HASH}  -\n"),
            status: 0,
        },
        "/srv/it's here.bin",
    );
    let cancel = AtomicBool::new(false);
    let captured = sshd
        .runtime
        .block_on(capture(&mut channel, Duration::from_secs(20), &cancel))
        .expect("the command ended");
    assert_eq!(interpret(&captured), Reply::Digest(HASH.into()));
    let commands = sshd.log.commands.lock().unwrap().clone();
    assert_eq!(commands.len(), 1);
    assert!(commands[0].starts_with("sh -c '"), "{}", commands[0]);
    assert!(
        commands[0].ends_with(" sh '/srv/it'\\''s here.bin'"),
        "the path travelled as one quoted word: {}",
        commands[0]
    );
}

#[test]
fn abgleich_task_sftp_exec_rejected_request_is_a_permission_error() {
    let sshd = Sshd::start();
    let (_handle, mut channel) = started(&sshd, ExecScript::Reject, "/f");
    let cancel = AtomicBool::new(false);
    let error = sshd
        .runtime
        .block_on(capture(&mut channel, Duration::from_secs(20), &cancel))
        .expect_err("the server refused to run commands");
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
}

#[test]
fn abgleich_task_sftp_exec_wait_ends_a_command_that_never_answers() {
    let sshd = Sshd::start();
    let (_handle, mut channel) = started(&sshd, ExecScript::Hang, "/f");
    let cancel = AtomicBool::new(false);
    let began = Instant::now();
    let error = sshd
        .runtime
        .block_on(capture(&mut channel, Duration::from_millis(600), &cancel))
        .expect_err("no answer in time");
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
