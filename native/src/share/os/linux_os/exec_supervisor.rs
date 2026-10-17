use std::ffi::OsString;
use std::io::{self, Read};
use std::os::fd::OwnedFd;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use super::exec_pty;
use crate::share::exec_supervisor_protocol::{
    recv_command, send_event, RunAsAccount, SupervisorCommand, SupervisorEvent, SupervisorExit,
    SupervisorStart,
};
use crate::share::exec_terminal::ExecTerminalSize;
use crate::share::exec_types::ExecCommand;

/// How the supervisor starts a job's root process. Linux passes the values
/// it always used (its systemd unit's cgroup contains the job); Android runs
/// the supervisor in the app process and adds a per-job intermediate. A
/// terminal job (`ExecCommand::Login`) uses the same policy: its root is the
/// login shell below a pseudo-terminal and leads its own session instead of a
/// process group.
pub(crate) struct SpawnPolicy<'a> {
    /// Shell of `ExecCommand::Shell` and `ExecCommand::Login` unless an
    /// absolute `SHELL` is honoured.
    pub(crate) default_shell: &'static str,
    /// An absolute `SHELL` in the job environment replaces `default_shell`.
    pub(crate) shell_from_environment: bool,
    /// Option placed before the shell command string (`-lc`, `-c`).
    pub(crate) shell_option: &'static str,
    /// The root process leads its own process group (not set for a terminal
    /// job, whose `setsid` makes the root a group leader anyway).
    pub(crate) own_process_group: bool,
    /// Process spawned in place of the root; it forks the root itself.
    pub(crate) intermediate: Option<&'a dyn Intermediate>,
}

/// A process the supervisor spawns in place of the job's root where no OS
/// boundary contains the job (Android: a per-job subreaper). It forks the
/// root, reports the root's wait status and exits only once the job's whole
/// process tree is gone; the supervisor never signals the root directly.
pub(crate) trait Intermediate {
    /// Prepares `command` to start as the intermediate (pre-exec hook,
    /// status channel); `own_process_group` applies to the root.
    /// `controlling_terminal` makes the root, never the intermediate, lead a
    /// new session with the terminal on its stdin as controlling terminal,
    /// in place of `own_process_group`.
    fn arm(
        &self,
        command: &mut Command,
        own_process_group: bool,
        controlling_terminal: bool,
    ) -> io::Result<Box<dyn ArmedIntermediate>>;
}

/// One armed start of an [`Intermediate`].
pub(crate) trait ArmedIntermediate {
    /// The intermediate now runs with this pid.
    fn spawned(&mut self, pid: u32);
    /// The root's wait status once the intermediate reported it; never blocks.
    fn root_status(&mut self) -> io::Result<Option<ExitStatus>>;
    /// Ends every remaining process of the job and reaps the intermediate
    /// (`child`); returns only when it is gone.
    fn finish(&mut self, child: &mut Child);
}

/// The spawned process: the root itself, or the intermediate that forked it.
struct Root {
    child: Child,
    intermediate: Option<Box<dyn ArmedIntermediate>>,
}

impl Root {
    fn exit_status(&mut self) -> io::Result<Option<ExitStatus>> {
        match self.intermediate.as_mut() {
            Some(intermediate) => intermediate.root_status(),
            None => self.child.try_wait(),
        }
    }

    /// Cancel or a lost controller: a plain root is killed and reaped (the
    /// OS boundary ends the rest); an intermediate tree ends in `drop`.
    fn cancel(&mut self) {
        if self.intermediate.is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn abort(&mut self) {
        if self.intermediate.is_none() {
            let _ = self.child.kill();
        }
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        // Every way out of `run` after the spawn passes here: nothing of an
        // intermediate job outlives its supervisor.
        if let Some(intermediate) = self.intermediate.as_mut() {
            intermediate.finish(&mut self.child);
        }
    }
}

pub(super) fn run(
    mut stream: std::os::unix::net::UnixStream,
    policy: &SpawnPolicy<'_>,
) -> io::Result<()> {
    let start = match recv_command(&mut stream)? {
        SupervisorCommand::Start(start) => start,
        _ => return Err(invalid("supervisor expected a start frame")),
    };
    start.request.validate()?;
    let (mut root, master) = match spawn(&start, policy) {
        Ok(spawned) => spawned,
        Err(error) => {
            let _ = send_event(&mut stream, &SupervisorEvent::Error(error.to_string()));
            return Ok(());
        }
    };
    send_event(
        &mut stream,
        &SupervisorEvent::Started {
            pid: root.child.id(),
        },
    )?;

    let writer = Arc::new(Mutex::new(stream.try_clone()?));
    let event_writer = writer.clone();
    let output = Output {
        writer,
        transferred: Arc::new(AtomicU64::new(0)),
        truncated: Arc::new(AtomicBool::new(false)),
        limit: start.request.effective_max_output_bytes(),
    };
    let (mut input, output_threads) = start_streams(&mut root, master, &output)?;

    let (command_tx, command_rx) = mpsc::sync_channel(16);
    std::thread::Builder::new()
        .name("exec-supervisor-input".into())
        .spawn(move || loop {
            let result = recv_command(&mut stream);
            let terminal = result.is_err() || matches!(result, Ok(SupervisorCommand::Cancel));
            if command_tx.send(result).is_err() || terminal {
                break;
            }
        })?;

    loop {
        if let Some(status) = root.exit_status()? {
            let mut exit = SupervisorExit {
                code: status.code(),
                signal: status.signal(),
                output_truncated: false,
            };
            send_locked_event(&event_writer, SupervisorEvent::RootExited(exit.clone()))?;
            for thread in output_threads {
                join_output(thread)?;
            }
            exit.output_truncated = output.truncated.load(Ordering::Acquire);
            send_locked_event(&event_writer, SupervisorEvent::Exited(exit))?;
            return Ok(());
        }
        match command_rx.recv_timeout(Duration::from_millis(20)) {
            Ok(Ok(SupervisorCommand::Stdin(data))) => input.write(data)?,
            Ok(Ok(SupervisorCommand::StdinEof)) => input.close(),
            Ok(Ok(SupervisorCommand::Resize(size))) => input.resize(size),
            Ok(Ok(SupervisorCommand::Cancel))
            | Ok(Err(_))
            | Err(mpsc::RecvTimeoutError::Disconnected) => {
                root.cancel();
                return Ok(());
            }
            Ok(Ok(SupervisorCommand::Start(_))) => {
                root.abort();
                return Err(invalid("duplicate supervisor start frame"));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

/// What the output readers of one job share: the event writer, the byte count
/// against the output limit and the truncation flag.
struct Output {
    writer: Arc<Mutex<std::os::unix::net::UnixStream>>,
    transferred: Arc<AtomicU64>,
    truncated: Arc<AtomicBool>,
    limit: Option<u64>,
}

/// Where the controller's `Stdin`, `StdinEof` and `Resize` commands go.
enum ChildInput {
    Pipe(Option<ChildStdin>),
    Terminal(exec_pty::TerminalInput),
}

impl ChildInput {
    fn write(&mut self, data: Vec<u8>) -> io::Result<()> {
        match self {
            Self::Pipe(stdin) => write_stdin(stdin, &data),
            Self::Terminal(terminal) => terminal.write(data),
        }
    }

    /// End of input closes a pipe. A terminal stays open: closing its master
    /// would hang the session up.
    fn close(&mut self) {
        if let Self::Pipe(stdin) = self {
            drop(stdin.take());
        }
    }

    /// Only a terminal has a window. A failed change keeps the old size and
    /// the next change replaces it, so it never ends the session.
    fn resize(&self, size: ExecTerminalSize) {
        if let Self::Terminal(terminal) = self {
            let _ = terminal.resize(size);
        }
    }
}

/// Starts the output readers: the pseudo-terminal's master (one stdout
/// stream, no stderr) or the stdout and stderr pipes of the root.
fn start_streams(
    root: &mut Root,
    master: Option<OwnedFd>,
    output: &Output,
) -> io::Result<(ChildInput, Vec<std::thread::JoinHandle<io::Result<()>>>)> {
    if let Some(master) = master {
        let (terminal, reader) = exec_pty::start_io(master)?;
        let thread = spawn_output(reader, output, false)?;
        return Ok((ChildInput::Terminal(terminal), vec![thread]));
    }
    let stdout = root
        .child
        .stdout
        .take()
        .ok_or_else(|| invalid("stdout pipe missing"))?;
    let stderr = root
        .child
        .stderr
        .take()
        .ok_or_else(|| invalid("stderr pipe missing"))?;
    let threads = vec![
        spawn_output(stdout, output, false)?,
        spawn_output(stderr, output, true)?,
    ];
    Ok((ChildInput::Pipe(root.child.stdin.take()), threads))
}

/// The root and, for a terminal job, the master of its pseudo-terminal. The
/// supervisor keeps no descriptor of the slave once this returns: the output
/// ends when the job's processes have closed theirs.
fn spawn(start: &SupervisorStart, policy: &SpawnPolicy<'_>) -> io::Result<(Root, Option<OwnedFd>)> {
    if let Some(account) = &start.account {
        if policy.intermediate.is_some() {
            return Err(invalid("a run-as account needs the Linux systemd host"));
        }
        require_account_identity(account)?;
    }
    let terminal = start.request.terminal.as_ref();
    let mut command = match (&start.request.command, terminal) {
        (ExecCommand::Argv { program, args }, None) => {
            let mut command = Command::new(program);
            command.args(args);
            command
        }
        (ExecCommand::Shell { command }, None) => {
            let mut shell_command = Command::new(shell_program(start, policy));
            shell_command.args([policy.shell_option, command]);
            shell_command
        }
        (ExecCommand::Login, Some(_)) => login_command(start, policy)?,
        (ExecCommand::Login, None) => {
            return Err(invalid("an interactive login shell needs a terminal"));
        }
        (ExecCommand::Argv { .. } | ExecCommand::Shell { .. }, Some(_)) => {
            return Err(invalid("a terminal is only valid for the login shell"));
        }
    };
    command.env_clear().envs(&start.environment);
    let master = match terminal {
        Some(request) => {
            // In an account unit this process is the account, so the
            // terminal it opens belongs to the account already.
            let pty = exec_pty::open_pair(request.size)?;
            exec_pty::attach_slave(&mut command, pty.slave, request)?;
            Some(pty.master)
        }
        None => {
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            None
        }
    };
    let cwd = start.request.cwd.as_deref().or_else(|| {
        start
            .environment
            .get("HOME")
            .filter(|home| !home.is_empty())
            .map(String::as_str)
    });
    // An account unit has entered the working directory already, with the
    // account's own rights (`WorkingDirectory=`); a missing home leaves the
    // job in `/`, as with sshd.
    if let Some(cwd) = cwd.filter(|_| start.account.is_none()) {
        command.current_dir(cwd);
    }
    let intermediate = match policy.intermediate {
        Some(intermediate) => {
            Some(intermediate.arm(&mut command, policy.own_process_group, terminal.is_some())?)
        }
        None => {
            if terminal.is_some() {
                // SAFETY: the hook makes only async-signal-safe system calls
                // and allocates nothing.
                unsafe { command.pre_exec(exec_pty::become_session_leader) };
            } else if policy.own_process_group {
                command.process_group(0);
            }
            None
        }
    };
    let mut root = Root {
        child: command.spawn()?,
        intermediate,
    };
    let pid = root.child.id();
    if let Some(intermediate) = root.intermediate.as_mut() {
        intermediate.spawned(pid);
    }
    Ok((root, master))
}

/// An absolute `SHELL` of the job environment where the policy honours it,
/// otherwise the policy's shell.
fn shell_program<'a>(start: &'a SupervisorStart, policy: &SpawnPolicy<'_>) -> &'a str {
    start
        .environment
        .get("SHELL")
        .filter(|value| policy.shell_from_environment && value.starts_with('/'))
        .map(String::as_str)
        .unwrap_or(policy.default_shell)
}

/// The login shell of the host user. Its `argv[0]` is `-` and the shell's file
/// name, the login convention of `sshd`; it gets no arguments.
fn login_command(start: &SupervisorStart, policy: &SpawnPolicy<'_>) -> io::Result<Command> {
    let shell = shell_program(start, policy);
    let name = Path::new(shell)
        .file_name()
        .ok_or_else(|| invalid("the login shell path has no file name"))?;
    let mut arg0 = OsString::from("-");
    arg0.push(name);
    let mut command = Command::new(shell);
    command.arg0(arg0);
    Ok(command)
}

fn spawn_output<R: Read + Send + 'static>(
    mut reader: R,
    output: &Output,
    stderr: bool,
) -> io::Result<std::thread::JoinHandle<io::Result<()>>> {
    let writer = output.writer.clone();
    let transferred = output.transferred.clone();
    let truncated = output.truncated.clone();
    let limit = output.limit;
    std::thread::Builder::new()
        .name(if stderr { "exec-stderr" } else { "exec-stdout" }.into())
        .spawn(move || {
            let mut bytes = [0u8; 64 * 1024];
            loop {
                let read = reader.read(&mut bytes)?;
                if read == 0 {
                    return Ok(());
                }
                let allowed = reserve_output(&transferred, limit, read);
                if allowed < read {
                    truncated.store(true, Ordering::Release);
                }
                if allowed == 0 {
                    continue;
                }
                let event = if stderr {
                    SupervisorEvent::Stderr(bytes[..allowed].to_vec())
                } else {
                    SupervisorEvent::Stdout(bytes[..allowed].to_vec())
                };
                if send_locked_event(&writer, event).is_err() {
                    return Ok(());
                }
            }
        })
}

fn join_output(thread: std::thread::JoinHandle<io::Result<()>>) -> io::Result<()> {
    thread
        .join()
        .map_err(|_| io::Error::other("exec output reader panicked"))?
}

fn reserve_output(counter: &AtomicU64, limit: Option<u64>, requested: usize) -> usize {
    let Some(limit) = limit else {
        counter.fetch_add(requested as u64, Ordering::Relaxed);
        return requested;
    };
    let mut current = counter.load(Ordering::Relaxed);
    loop {
        let available = limit.saturating_sub(current).min(requested as u64);
        match counter.compare_exchange_weak(
            current,
            current.saturating_add(available),
            Ordering::AcqRel,
            Ordering::Relaxed,
        ) {
            Ok(_) => return available as usize,
            Err(actual) => current = actual,
        }
    }
}

fn write_stdin(stdin: &mut Option<ChildStdin>, data: &[u8]) -> io::Result<()> {
    use std::io::Write;
    stdin
        .as_mut()
        .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "remote stdin is closed"))?
        .write_all(data)
}

fn send_locked_event(
    writer: &Arc<Mutex<std::os::unix::net::UnixStream>>,
    event: SupervisorEvent,
) -> io::Result<()> {
    let mut writer = writer
        .lock()
        .map_err(|_| io::Error::other("supervisor writer lock poisoned"))?;
    send_event(&mut *writer, &event)
}

/// A job of another account runs only in its account unit, where systemd
/// made this supervisor the account (`User=`, PAM) before its first
/// instruction (PRM-M5). The supervisor never switches accounts itself: it
/// refuses a start whose account it is not, and root is never such an
/// account.
fn require_account_identity(account: &RunAsAccount) -> io::Result<()> {
    // SAFETY: the four calls only read this process's ids.
    let ids = unsafe {
        (
            libc::getuid(),
            libc::geteuid(),
            libc::getgid(),
            libc::getegid(),
        )
    };
    if account.uid != 0 && ids == (account.uid, account.uid, account.gid, account.gid) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "this supervisor does not run as the job's account; another account's job runs \
             only in its account unit",
        ))
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
