//! Unit tests of the terminal exec on a Linux host without systemd: the
//! supervisor runs on a thread of the test, serves a pseudo-terminal and is
//! driven over a socket pair with the supervisor protocol. Output is read
//! with a time limit, never unbounded.
use std::collections::BTreeMap;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::exec_pty;
use super::exec_supervisor::{self, SpawnPolicy};
use crate::share::exec_supervisor_protocol::{
    recv_event, send_command, SupervisorCommand, SupervisorEvent, SupervisorExit, SupervisorStart,
};
use crate::share::exec_terminal::{ExecTerminalRequest, ExecTerminalSize};
use crate::share::exec_types::{ExecCommand, ExecId, ExecStart};

/// No systemd and no intermediate; the shell is `/bin/sh` whatever `SHELL` says.
const POLICY: SpawnPolicy<'static> = SpawnPolicy {
    default_shell: "/bin/sh",
    shell_from_environment: false,
    shell_option: "-lc",
    own_process_group: false,
    intermediate: None,
};
const WAIT: Duration = Duration::from_secs(10);

fn terminal() -> Option<ExecTerminalRequest> {
    Some(ExecTerminalRequest {
        term: "xterm-256color".into(),
        size: ExecTerminalSize {
            cols: 100,
            rows: 30,
        },
    })
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

/// One supervisor on a thread and the controller's side of its socket.
struct Session {
    commands: UnixStream,
    events: mpsc::Receiver<io::Result<SupervisorEvent>>,
    supervisor: Option<JoinHandle<io::Result<()>>>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit: Option<SupervisorExit>,
}

impl Session {
    fn start(command: ExecCommand, terminal: Option<ExecTerminalRequest>) -> Self {
        let (mut commands, supervisor_end) = UnixStream::pair().expect("socket pair");
        let supervisor = std::thread::spawn(move || exec_supervisor::run(supervisor_end, &POLICY));
        let mut reader = commands.try_clone().expect("clone the command socket");
        let (event_tx, events) = mpsc::channel();
        std::thread::spawn(move || loop {
            let event = recv_event(&mut reader);
            let last = event.is_err();
            if event_tx.send(event).is_err() || last {
                break;
            }
        });
        let request = ExecStart {
            exec_id: ExecId::parse("00".repeat(16)).expect("exec id"),
            command,
            cwd: Some("/".into()),
            env: BTreeMap::new(),
            timeout_ms: None,
            max_output_bytes: None,
            terminal,
        };
        // Stale window variables must not reach a terminal job.
        let environment = BTreeMap::from([
            (
                "PATH".to_string(),
                std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into()),
            ),
            ("HOME".to_string(), "/".to_string()),
            ("COLUMNS".to_string(), "5".to_string()),
            ("LINES".to_string(), "5".to_string()),
        ]);
        let start = SupervisorStart {
            request,
            environment,
            account: None,
        };
        send_command(&mut commands, &SupervisorCommand::Start(start)).expect("send the start");
        Self {
            commands,
            events,
            supervisor: Some(supervisor),
            stdout: Vec::new(),
            stderr: Vec::new(),
            exit: None,
        }
    }

    fn send(&mut self, bytes: &[u8]) {
        let command = SupervisorCommand::Stdin(bytes.to_vec());
        send_command(&mut self.commands, &command).expect("send input");
    }

    /// A line as the terminal sends it: Enter is a carriage return.
    fn type_line(&mut self, line: &str) {
        self.send(format!("{line}\r").as_bytes());
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        let command = SupervisorCommand::Resize(ExecTerminalSize { cols, rows });
        send_command(&mut self.commands, &command).expect("send the window size");
    }

    fn pump(&mut self, deadline: Instant) {
        let left = deadline.saturating_duration_since(Instant::now());
        match self.events.recv_timeout(left) {
            Ok(Ok(SupervisorEvent::Stdout(bytes))) => self.stdout.extend(bytes),
            Ok(Ok(SupervisorEvent::Stderr(bytes))) => self.stderr.extend(bytes),
            Ok(Ok(SupervisorEvent::Exited(exit))) => self.exit = Some(exit),
            Ok(Ok(SupervisorEvent::Started { .. } | SupervisorEvent::RootExited(_))) => {}
            other => panic!(
                "no usable supervisor event before the deadline ({other:?}); output so far: {}",
                String::from_utf8_lossy(&self.stdout)
            ),
        }
    }

    fn wait_for_within(&mut self, needle: &str, within: Duration) {
        let deadline = Instant::now() + within;
        while !contains(&self.stdout, needle) {
            self.pump(deadline);
        }
    }

    fn wait_for(&mut self, needle: &str) {
        self.wait_for_within(needle, WAIT);
    }

    fn wait_exit(&mut self) -> SupervisorExit {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(exit) = &self.exit {
                return exit.clone();
            }
            self.pump(deadline);
        }
    }

    fn join_supervisor(&mut self) -> io::Result<()> {
        let handle = self.supervisor.take().expect("supervisor thread");
        let deadline = Instant::now() + WAIT;
        while !handle.is_finished() {
            assert!(Instant::now() < deadline, "the supervisor did not return");
            std::thread::sleep(Duration::from_millis(10));
        }
        handle.join().expect("the supervisor thread panicked")
    }
}

impl Drop for Session {
    /// A test that fails must not leave its shell behind.
    fn drop(&mut self) {
        let _ = send_command(&mut self.commands, &SupervisorCommand::Cancel);
        let _ = self.commands.shutdown(std::net::Shutdown::Both);
    }
}

#[test]
fn terminal_task_pty_pair_starts_with_the_requested_size_and_utf8_input() {
    let size = ExecTerminalSize {
        cols: 132,
        rows: 43,
    };
    let pty = exec_pty::open_pair(size).expect("open a pty pair");
    let mut window: libc::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: TIOCGWINSZ writes one winsize through the pointer.
    let read = unsafe { libc::ioctl(pty.slave.as_raw_fd(), libc::TIOCGWINSZ, &mut window) };
    assert_eq!(read, 0);
    assert_eq!((window.ws_col, window.ws_row), (132, 43));
    let mut attributes: libc::termios = unsafe { std::mem::zeroed() };
    // SAFETY: tcgetattr fills the termios the pointer names.
    assert_eq!(
        unsafe { libc::tcgetattr(pty.slave.as_raw_fd(), &mut attributes) },
        0
    );
    assert_ne!(attributes.c_iflag & libc::IUTF8, 0);
    assert!(exec_pty::terminal_supported());
}

#[test]
fn terminal_task_login_shell_runs_on_a_pty_of_the_requested_size() {
    let mut session = Session::start(ExecCommand::Login, terminal());
    session.type_line("tty");
    session.wait_for("/dev/pts/");
    session.type_line("stty size");
    session.wait_for("30 100");
    session.resize(120, 40);
    session.type_line("stty size");
    session.wait_for("40 120");
}

#[test]
fn terminal_task_login_shell_gets_term_a_login_name_and_no_window_variables() {
    let mut session = Session::start(ExecCommand::Login, terminal());
    session.type_line("echo T=$TERM S=$0 W=$(env | grep -c -e '^COLUMNS=' -e '^LINES=')");
    session.wait_for("T=xterm-256color S=-sh W=0");
}

#[test]
fn terminal_task_control_c_ends_the_foreground_command_and_keeps_the_shell() {
    let mut session = Session::start(ExecCommand::Login, terminal());
    session.type_line("echo started-$((6*7)); sleep 100");
    session.wait_for("started-42");
    // Let the shell finish making `sleep` the foreground job; a Ctrl+C also
    // flushes the terminal's pending input, so nothing is typed ahead of it.
    std::thread::sleep(Duration::from_millis(500));
    session.send(&[3]);
    std::thread::sleep(Duration::from_millis(500));
    session.type_line("echo ali''ve");
    session.wait_for_within("alive", Duration::from_secs(5));
}

#[test]
fn terminal_task_login_shell_exit_status_reaches_the_exit_event() {
    let mut session = Session::start(ExecCommand::Login, terminal());
    session.type_line("exit 7");
    let exit = session.wait_exit();
    assert_eq!((exit.code, exit.signal), (Some(7), None));
    session
        .join_supervisor()
        .expect("the supervisor ends cleanly");
}

#[test]
fn terminal_task_start_without_terminal_keeps_separate_pipes() {
    let command = "if [ -t 0 ] || [ -t 1 ]; then echo tty-yes; else echo tty-no; fi; \
                   echo out-$((1+1)); echo err-$((2+1)) >&2";
    let mut session = Session::start(
        ExecCommand::Shell {
            command: command.into(),
        },
        None,
    );
    let exit = session.wait_exit();
    assert_eq!(exit.code, Some(0));
    assert!(contains(&session.stdout, "tty-no"));
    assert!(contains(&session.stdout, "out-2"));
    assert!(!contains(&session.stdout, "err-3"));
    assert!(contains(&session.stderr, "err-3"));
    session
        .join_supervisor()
        .expect("the supervisor ends cleanly");
}

#[test]
fn terminal_task_start_fails_closed_unless_login_and_terminal_come_together() {
    let shell = ExecCommand::Shell {
        command: "true".into(),
    };
    for (command, request) in [(ExecCommand::Login, None), (shell, terminal())] {
        let mut session = Session::start(command, request);
        let error = session
            .join_supervisor()
            .expect_err("the start must be rejected");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
}
