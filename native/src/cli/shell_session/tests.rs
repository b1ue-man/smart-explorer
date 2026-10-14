//! Sessions on a local shell (bash or sh on Unix, Windows PowerShell on
//! Windows) through the same engine, attach service and client as remote
//! sessions: state across commands, exit codes, separate streams, input,
//! both roles of attaching, cancel and close.
use super::client::SessionClient;
use super::host::{host, OpenOptions};
use super::local_transport::LocalTransport;
use super::registry;
#[cfg(unix)]
use super::types::Activity;
use super::types::{
    CommandOutcome, CommandRequest, CommandSink, Dialect, OutcomeStatus, StdinMode, Stream,
};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Collect {
    stdout: Mutex<Vec<u8>>,
    stderr: Mutex<Vec<u8>>,
    outcome: Mutex<Option<CommandOutcome>>,
    done: Condvar,
}

impl CommandSink for Collect {
    fn output(&self, stream: Stream, bytes: &[u8]) {
        let target = match stream {
            Stream::Stdout => &self.stdout,
            Stream::Stderr => &self.stderr,
        };
        target.lock().unwrap().extend_from_slice(bytes);
    }

    fn finished(&self, outcome: CommandOutcome) {
        *self.outcome.lock().unwrap() = Some(outcome);
        self.done.notify_all();
    }
}

struct Ran {
    stdout: String,
    stderr: String,
    outcome: CommandOutcome,
}

fn wait(sink: &Collect) -> Ran {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut outcome = sink.outcome.lock().unwrap();
    while outcome.is_none() {
        let left = deadline
            .checked_duration_since(Instant::now())
            .expect("the command did not finish within 60 s");
        outcome = sink.done.wait_timeout(outcome, left).unwrap().0;
    }
    Ran {
        stdout: String::from_utf8_lossy(&sink.stdout.lock().unwrap()).into_owned(),
        stderr: String::from_utf8_lossy(&sink.stderr.lock().unwrap()).into_owned(),
        outcome: outcome.clone().unwrap(),
    }
}

fn run(client: &SessionClient, request: CommandRequest) -> Ran {
    let sink = Arc::new(Collect::default());
    client.submit(request, sink.clone()).unwrap();
    wait(&sink)
}

fn command(text: &str) -> CommandRequest {
    CommandRequest {
        command: text.into(),
        cwd: None,
        env: BTreeMap::new(),
        stdin: StdinMode::Null,
    }
}

#[cfg(unix)]
fn local_shell() -> (LocalTransport, Dialect) {
    let start = super::dialect::start_line(Dialect::Posix);
    (LocalTransport::spawn("sh", &["-c", start]), Dialect::Posix)
}

#[cfg(windows)]
fn local_shell() -> (LocalTransport, Dialect) {
    let start = super::dialect::start_line(Dialect::PowerShell);
    let mut words = start.split_whitespace();
    let program = words.next().unwrap();
    let args: Vec<&str> = words.collect();
    (LocalTransport::spawn(program, &args), Dialect::PowerShell)
}

fn open(name: &str, attachable: bool, registry_dir: &Path) -> SessionClient {
    let (transport, dialect) = local_shell();
    host(
        Box::new(transport),
        dialect,
        "local",
        OpenOptions {
            name: name.into(),
            owner: "test owner".into(),
            attachable,
            cwd: None,
            env: BTreeMap::new(),
        },
        registry_dir,
    )
    .unwrap()
}

fn wait_unlisted(registry_dir: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !registry::list_in(registry_dir).is_empty() {
        assert!(Instant::now() < deadline, "the session entry stayed listed");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(unix)]
#[test]
fn shell_session_task_local_shell_keeps_state_across_commands() {
    let registry_dir = tempfile::tempdir().unwrap();
    let client = open("state", false, registry_dir.path());
    let first = run(
        &client,
        command("export SE_X=kept; cd /; MYVAR=v; f() { echo fn-$1; }"),
    );
    assert_eq!(first.outcome.status, OutcomeStatus::Completed);
    assert_eq!(first.outcome.exit_code, Some(0));
    assert_eq!(first.outcome.cwd.as_deref(), Some("/"));
    assert_eq!(client.info().cwd.as_deref(), Some("/"));
    let second = run(
        &client,
        command("printf '%s|%s|' \"$SE_X\" \"$MYVAR\"; f one; pwd"),
    );
    assert_eq!(second.stdout, "kept|v|fn-one\n/\n");
    let failed = run(&client, command("echo out; echo err >&2; false"));
    assert_eq!(failed.stdout, "out\n");
    assert_eq!(failed.stderr, "err\n");
    assert_eq!(failed.outcome.exit_code, Some(1));
    assert_eq!(
        run(&client, command("printf no-newline")).stdout,
        "no-newline"
    );
    let multi = run(&client, command("for i in 1 2; do\n  echo line$i\ndone"));
    assert_eq!(multi.stdout, "line1\nline2\n");
    let mut input = command("cat; echo after");
    input.stdin = StdinMode::Text("a\nb".into());
    assert_eq!(run(&client, input).stdout, "a\nb\nafter\n");
    let mut placed = command("pwd; printf %s \"$MODE\"");
    placed.cwd = Some("/tmp".into());
    placed.env.insert("MODE".into(), "x y".into());
    assert_eq!(run(&client, placed).stdout, "/tmp\nx y");
    let syntax = run(&client, command("echo 'unterminated"));
    assert_ne!(syntax.outcome.exit_code, Some(0));
    assert_eq!(run(&client, command("echo alive")).stdout, "alive\n");
    let ended = run(&client, command("exit 3"));
    assert_eq!(ended.outcome.status, OutcomeStatus::Completed);
    assert_eq!(ended.outcome.exit_code, Some(3));
    assert!(ended.outcome.session_ended);
    assert!(client.wait_ended(Duration::from_secs(10)));
    assert!(client
        .submit(command("true"), Arc::new(Collect::default()))
        .is_err());
}

#[cfg(unix)]
#[test]
fn shell_session_task_interactive_input_reaches_only_the_own_command() {
    let registry_dir = tempfile::tempdir().unwrap();
    let client = open("typed", false, registry_dir.path());
    let sink = Arc::new(Collect::default());
    let mut typed = command("read line; echo got-$line");
    typed.stdin = StdinMode::Interactive;
    client.submit(typed, sink.clone()).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    client.input(b"typed\n").unwrap();
    assert_eq!(wait(&sink).stdout, "got-typed\n");
    client.leave();
    assert!(client.wait_ended(Duration::from_secs(15)));
}

#[cfg(unix)]
#[test]
fn shell_session_task_attached_clients_share_state_and_activity() {
    let registry_dir = tempfile::tempdir().unwrap();
    let owner = open("shared", true, registry_dir.path());
    let entry = registry::find_in(registry_dir.path(), "shared").unwrap();
    assert_eq!(entry.dialect, Dialect::Posix);
    let attached = SessionClient::attach(&entry, "test attacher").unwrap();
    let watched = attached.activity();
    // The attached client changes the shell state; the owner sees it.
    run(&attached, command("cd /tmp && export FROM_ATTACHED=yes"));
    let seen = run(
        &owner,
        command("printf '%s %s' \"$PWD\" \"$FROM_ATTACHED\""),
    );
    assert_eq!(seen.stdout, "/tmp yes");
    // The attached client saw the owner's command and its output.
    let mut started = false;
    let mut output = Vec::new();
    while let Ok(activity) = watched.recv_timeout(Duration::from_secs(10)) {
        match activity {
            Activity::Started { by, .. } => started |= by == "test owner",
            Activity::Output { bytes, .. } => output.extend_from_slice(&bytes),
            Activity::Done { .. } => break,
            _ => {}
        }
    }
    assert!(started);
    assert_eq!(output, b"/tmp yes");
    // A forged token is refused.
    let mut forged = entry.clone();
    forged.token = "00".repeat(32);
    assert!(SessionClient::attach(&forged, "intruder").is_err());
    // Detaching keeps the session; the owner closing ends it for everyone.
    attached.leave();
    assert_eq!(run(&owner, command("echo still")).stdout, "still\n");
    let again = SessionClient::attach(&entry, "second").unwrap();
    owner.leave();
    assert!(again.wait_ended(Duration::from_secs(15)));
    assert!(owner.wait_ended(Duration::from_secs(15)));
    wait_unlisted(registry_dir.path());
}

#[cfg(unix)]
#[test]
fn shell_session_task_cancel_ends_the_session_for_every_participant() {
    let registry_dir = tempfile::tempdir().unwrap();
    let owner = open("cancel", true, registry_dir.path());
    let entry = registry::find_in(registry_dir.path(), "cancel").unwrap();
    let attached = SessionClient::attach(&entry, "watcher").unwrap();
    let sink = Arc::new(Collect::default());
    owner.submit(command("sleep 2"), sink.clone()).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    attached.cancel();
    let outcome = wait(&sink).outcome;
    assert_eq!(outcome.status, OutcomeStatus::Cancelled);
    assert!(outcome.session_ended);
    assert!(owner.wait_ended(Duration::from_secs(15)));
    assert!(attached.wait_ended(Duration::from_secs(15)));
    let (_, message) = attached.ended().unwrap();
    assert!(message.contains("watcher"), "{message}");
    wait_unlisted(registry_dir.path());
}

#[cfg(unix)]
#[test]
fn shell_session_task_attach_service_refuses_garbage_and_silence() {
    use std::io::{Read, Write};
    let registry_dir = tempfile::tempdir().unwrap();
    let owner = open("guarded", true, registry_dir.path());
    let entry = registry::find_in(registry_dir.path(), "guarded").unwrap();
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], entry.port));
    let mut byte = [0u8; 1];
    // A first frame other than hello is refused at once.
    let mut garbage = std::net::TcpStream::connect(address).unwrap();
    garbage.write_all(&[2, 0, 0, 0, 2, b'{', b'}']).unwrap();
    garbage
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    assert_eq!(garbage.read(&mut byte).unwrap_or(0), 0);
    // A silent client is dropped after the handshake deadline.
    let started = Instant::now();
    let mut silent = std::net::TcpStream::connect(address).unwrap();
    silent
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    assert_eq!(silent.read(&mut byte).unwrap_or(0), 0);
    assert!(started.elapsed() < Duration::from_secs(12));
    // A client trickling its hello one byte per second is dropped at the
    // same deadline: it holds for the whole first frame, not for each read.
    let started = Instant::now();
    let mut slow = std::net::TcpStream::connect(address).unwrap();
    slow.write_all(&[1, 0, 0, 0, 100]).unwrap();
    let mut trickle = slow.try_clone().unwrap();
    let trickling = std::thread::spawn(move || {
        for _ in 0..15 {
            std::thread::sleep(Duration::from_secs(1));
            if trickle.write_all(b" ").is_err() {
                break;
            }
        }
    });
    slow.set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    assert_eq!(slow.read(&mut byte).unwrap_or(0), 0);
    assert!(started.elapsed() < Duration::from_secs(10));
    drop(slow);
    trickling.join().unwrap();
    // The session is unaffected.
    assert_eq!(run(&owner, command("echo fine")).stdout, "fine\n");
    owner.leave();
    assert!(owner.wait_ended(Duration::from_secs(15)));
}

#[cfg(unix)]
#[test]
fn shell_session_task_sessions_without_consent_cannot_be_attached() {
    let registry_dir = tempfile::tempdir().unwrap();
    let owner = open("private", false, registry_dir.path());
    assert!(registry::find_in(registry_dir.path(), "private").is_err());
    assert!(registry::list_in(registry_dir.path()).is_empty());
    owner.leave();
    assert!(owner.wait_ended(Duration::from_secs(15)));
}

#[cfg(windows)]
#[test]
fn shell_session_task_local_powershell_keeps_state_across_commands() {
    let registry_dir = tempfile::tempdir().unwrap();
    let client = open("pwsh", true, registry_dir.path());
    let first = run(
        &client,
        command(
            "$env:SE_X = 'kept'; Set-Location C:\\; $myvar = 'v'; function f($a) { \"fn-$a\" }",
        ),
    );
    assert_eq!(first.outcome.status, OutcomeStatus::Completed);
    assert_eq!(first.outcome.exit_code, Some(0), "{}", first.stderr);
    assert_eq!(
        first.outcome.cwd.as_deref(),
        Some("C:\\"),
        "{}",
        first.stderr
    );
    let second = run(&client, command("\"$env:SE_X|$myvar|\" + (f one)"));
    assert_eq!(
        second.stdout.trim_end(),
        "kept|v|fn-one",
        "{}",
        second.stderr
    );
    assert_eq!(
        run(&client, command("cmd.exe /c exit 4")).outcome.exit_code,
        Some(4)
    );
    let error = run(&client, command("Write-Error 'boom'"));
    assert_eq!(error.outcome.exit_code, Some(1), "{}", error.stderr);
    assert!(error.stderr.contains("boom"), "{}", error.stderr);
    assert_eq!(
        run(&client, command("throw 'bad'")).outcome.exit_code,
        Some(1)
    );
    let handled = run(&client, command("try { throw 'x' } catch { 'handled' }"));
    assert_eq!(handled.outcome.exit_code, Some(0), "{}", handled.stderr);
    assert_eq!(handled.stdout.trim_end(), "handled", "{}", handled.stderr);
    let multi = run(&client, command("foreach ($i in 1..2) {\n  \"line$i\"\n}"));
    assert_eq!(
        multi.stdout.replace("\r\n", "\n"),
        "line1\nline2\n",
        "{}",
        multi.stderr
    );
    let mut piped = command("$input | ForEach-Object { \"got-$_\" }");
    piped.stdin = StdinMode::Text("data".into());
    assert_eq!(run(&client, piped).stdout.trim_end(), "got-data");
    let mut placed = command("$env:MODE");
    placed.cwd = Some(std::env::temp_dir().display().to_string());
    placed.env.insert("MODE".into(), "x y".into());
    let placed = run(&client, placed);
    assert_eq!(placed.stdout.trim_end(), "x y", "stderr: {}", placed.stderr);
    assert!(placed.outcome.cwd.is_some(), "stderr: {}", placed.stderr);
    let entry = registry::find_in(registry_dir.path(), "pwsh").unwrap();
    assert_eq!(entry.dialect, Dialect::PowerShell);
    let attached = SessionClient::attach(&entry, "attacher").unwrap();
    assert_eq!(
        run(&attached, command("$env:SE_X")).stdout.trim_end(),
        "kept"
    );
    attached.leave();
    let ended = run(&client, command("exit 3"));
    assert_eq!(ended.outcome.exit_code, Some(3), "{}", ended.stderr);
    assert!(ended.outcome.session_ended, "{}", ended.stderr);
    wait_unlisted(registry_dir.path());
}
