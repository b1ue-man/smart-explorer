use std::fs::File;
use std::io::{self, Read};
use std::os::windows::process::CommandExt;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use windows_sys::Win32::Foundation::{BOOL, FALSE, TRUE};
use windows_sys::Win32::System::Console::{SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_C_EVENT};

use super::exec_conpty::{self, Conpty, Program};
use crate::share::exec_supervisor_protocol::{
    recv_command, send_event, SupervisorCommand, SupervisorEvent, SupervisorExit, SupervisorStart,
};
use crate::share::exec_types::ExecCommand;

/// Keyboard chunks (each at most 64 KiB) waiting for the terminal input pipe.
const TERMINAL_INPUT_QUEUE: usize = 64;

pub(super) fn run() -> io::Result<()> {
    let mut input = std::io::stdin();
    let start = match recv_command(&mut input)? {
        SupervisorCommand::Start(start) => start,
        _ => return Err(invalid("supervisor expected a start frame")),
    };
    start.request.validate()?;
    let output = Arc::new(Mutex::new(std::io::stdout()));
    if start.request.terminal.is_some() {
        return run_terminal(&start, input, output);
    }
    let mut child = match spawn(&start) {
        Ok(child) => child,
        Err(error) => {
            let _ = send_locked(&output, SupervisorEvent::Error(error.to_string()));
            return Ok(());
        }
    };
    send_locked(&output, SupervisorEvent::Started { pid: child.id() })?;

    let transferred = Arc::new(AtomicU64::new(0));
    let truncated = Arc::new(AtomicBool::new(false));
    let limit = start.request.effective_max_output_bytes();
    let stdout_thread = spawn_output(
        child
            .stdout
            .take()
            .ok_or_else(|| invalid("stdout pipe missing"))?,
        output.clone(),
        transferred.clone(),
        truncated.clone(),
        limit,
        false,
    )?;
    let stderr_thread = spawn_output(
        child
            .stderr
            .take()
            .ok_or_else(|| invalid("stderr pipe missing"))?,
        output.clone(),
        transferred,
        truncated.clone(),
        limit,
        true,
    )?;

    let command_rx = spawn_command_reader(input)?;

    let mut stdin = child.stdin.take();
    loop {
        if let Some(status) = child.try_wait()? {
            let mut exit = SupervisorExit {
                code: status.code(),
                signal: None,
                output_truncated: false,
            };
            send_locked(&output, SupervisorEvent::RootExited(exit.clone()))?;
            join_output(stdout_thread)?;
            join_output(stderr_thread)?;
            exit.output_truncated = truncated.load(Ordering::Acquire);
            send_locked(&output, SupervisorEvent::Exited(exit))?;
            return Ok(());
        }
        match command_rx.recv_timeout(Duration::from_millis(20)) {
            Ok(Ok(SupervisorCommand::Stdin(data))) => write_stdin(&mut stdin, &data)?,
            Ok(Ok(SupervisorCommand::StdinEof)) => drop(stdin.take()),
            // Only a terminal start is resized; the host never sends it otherwise.
            Ok(Ok(SupervisorCommand::Resize(_))) => {}
            Ok(Ok(SupervisorCommand::Cancel))
            | Ok(Err(_))
            | Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(());
            }
            Ok(Ok(SupervisorCommand::Start(_))) => {
                let _ = child.kill();
                return Err(invalid("duplicate supervisor start frame"));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

/// Terminal start: the login shell runs in a pseudoconsole (ConPTY). Its
/// output becomes `Stdout` events, `Stdin` commands become keyboard bytes and
/// `Resize` commands resize the pseudoconsole. `StdinEof` has no effect: a
/// terminal has no end of input. The host's Job Object ends whatever survives
/// this supervisor.
fn run_terminal(
    start: &SupervisorStart,
    input: std::io::Stdin,
    output: Arc<Mutex<std::io::Stdout>>,
) -> io::Result<()> {
    let launched = shield_from_interrupts().and_then(|()| launch_terminal(start));
    let (mut conpty, pty_output, pty_input) = match launched {
        Ok(launched) => launched,
        Err(error) => {
            let _ = send_locked(&output, SupervisorEvent::Error(error.to_string()));
            return Ok(());
        }
    };
    let result = serve_terminal(start, &mut conpty, input, pty_output, pty_input, &output);
    // However the session ended, nothing of it may stay: end the root process
    // (nothing to do once it exited) and close the pseudoconsole (nothing to
    // do once it is closed).
    let _ = conpty.terminate();
    let _ = conpty.close_console();
    result
}

/// `exec_conpty::start` gives the shell Ctrl+C by clearing this process's
/// inherited "ignore Ctrl+C" attribute. This handler keeps Ctrl+C and
/// Ctrl+Break of the supervisor's own windowless console from ending the
/// supervisor; handlers are not inherited, so the shell keeps the default.
fn shield_from_interrupts() -> io::Result<()> {
    // SAFETY: `ignore_interrupts` has the handler signature and lives for the
    // whole process.
    if unsafe { SetConsoleCtrlHandler(Some(ignore_interrupts), TRUE) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

unsafe extern "system" fn ignore_interrupts(kind: u32) -> BOOL {
    match kind {
        CTRL_C_EVENT | CTRL_BREAK_EVENT => TRUE,
        _ => FALSE,
    }
}

fn launch_terminal(start: &SupervisorStart) -> io::Result<(Conpty, File, File)> {
    let terminal = start
        .request
        .terminal
        .as_ref()
        .ok_or_else(|| invalid("terminal start without a terminal"))?;
    let program = match &start.request.command {
        ExecCommand::Login => Program::login_shell()?,
        ExecCommand::Argv { .. } | ExecCommand::Shell { .. } => {
            return Err(invalid("a terminal needs the login shell"));
        }
    };
    exec_conpty::start(
        &program,
        &start.environment,
        working_directory(start),
        terminal.size,
    )
}

fn serve_terminal(
    start: &SupervisorStart,
    conpty: &mut Conpty,
    input: std::io::Stdin,
    pty_output: File,
    pty_input: File,
    output: &Arc<Mutex<std::io::Stdout>>,
) -> io::Result<()> {
    send_locked(
        output,
        SupervisorEvent::Started {
            pid: conpty.process_id(),
        },
    )?;
    let truncated = Arc::new(AtomicBool::new(false));
    let reader = spawn_output(
        pty_output,
        output.clone(),
        Arc::new(AtomicU64::new(0)),
        truncated.clone(),
        start.request.effective_max_output_bytes(),
        false,
    )?;
    let keys = spawn_terminal_input(pty_input)?;
    let commands = spawn_command_reader(input)?;
    let code = loop {
        if let Some(code) = conpty.wait(Duration::ZERO)? {
            break code;
        }
        match commands.recv_timeout(Duration::from_millis(20)) {
            Ok(Ok(SupervisorCommand::Stdin(data))) => queue_terminal_input(conpty, &keys, data)?,
            Ok(Ok(SupervisorCommand::StdinEof)) => {}
            Ok(Ok(SupervisorCommand::Resize(size))) => {
                // A failed resize keeps the previous size; the next one replaces it.
                let _ = conpty.resize(size);
            }
            Ok(Ok(SupervisorCommand::Cancel))
            | Ok(Err(_))
            | Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            Ok(Ok(SupervisorCommand::Start(_))) => {
                return Err(invalid("duplicate supervisor start frame"));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    };
    let mut exit = SupervisorExit {
        code: Some(code as i32),
        signal: None,
        output_truncated: false,
    };
    send_locked(output, SupervisorEvent::RootExited(exit.clone()))?;
    // Closing the pseudoconsole flushes its last output and breaks the output
    // pipe. It runs on its own thread (before Windows 11 24H2 it waits for the
    // output to be drained) while this thread's reader keeps reading.
    let _ = conpty.close_console();
    join_output(reader)?;
    exit.output_truncated = truncated.load(Ordering::Acquire);
    send_locked(output, SupervisorEvent::Exited(exit))
}

/// Writes keyboard bytes to the pseudoconsole input on its own thread, so a
/// shell that does not read never blocks the command loop.
fn spawn_terminal_input(mut pty_input: File) -> io::Result<mpsc::SyncSender<Vec<u8>>> {
    use std::io::Write;
    let (keys, queue) = mpsc::sync_channel::<Vec<u8>>(TERMINAL_INPUT_QUEUE);
    std::thread::Builder::new()
        .name("exec-terminal-input".into())
        .spawn(move || {
            while let Ok(data) = queue.recv() {
                if pty_input.write_all(&data).is_err() {
                    break;
                }
            }
        })?;
    Ok(keys)
}

fn queue_terminal_input(
    conpty: &Conpty,
    keys: &mpsc::SyncSender<Vec<u8>>,
    mut data: Vec<u8>,
) -> io::Result<()> {
    loop {
        match keys.try_send(data) {
            Ok(()) => return Ok(()),
            Err(mpsc::TrySendError::Full(returned)) => {
                // The shell is not reading its input. Later commands wait
                // behind this one, but the end of the shell is still noticed.
                if conpty.wait(Duration::from_millis(5))?.is_some() {
                    return Ok(());
                }
                data = returned;
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                // The input pipe broke: the shell just ended (the command loop
                // sees that next) or the pseudoconsole is gone.
                return if conpty.wait(Duration::ZERO)?.is_some() {
                    Ok(())
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "terminal input closed",
                    ))
                };
            }
        }
    }
}

fn spawn(start: &SupervisorStart) -> io::Result<std::process::Child> {
    let mut command = match &start.request.command {
        ExecCommand::Argv { program, args } => {
            let mut command = Command::new(program);
            command.args(args);
            command
        }
        ExecCommand::Shell { command } => {
            let shell = start
                .environment
                .get("COMSPEC")
                .filter(|value| !value.is_empty())
                .map(String::as_str)
                .unwrap_or("cmd.exe");
            let mut shell_command = Command::new(shell);
            shell_command.args(["/D", "/S", "/C"]);
            shell_command.raw_arg(command);
            shell_command
        }
        ExecCommand::Login => return Err(invalid("an interactive login shell needs a terminal")),
    };
    command
        .env_clear()
        .envs(&start.environment)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    if let Some(cwd) = working_directory(start) {
        command.current_dir(cwd);
    }
    command.spawn()
}

/// The requested working directory, else the user's profile directory.
fn working_directory(start: &SupervisorStart) -> Option<&str> {
    start.request.cwd.as_deref().or_else(|| {
        start
            .environment
            .get("USERPROFILE")
            .filter(|home| !home.is_empty())
            .map(String::as_str)
    })
}

/// Reads the host's commands on a thread; it ends after an error or `Cancel`.
fn spawn_command_reader(
    mut input: std::io::Stdin,
) -> io::Result<mpsc::Receiver<io::Result<SupervisorCommand>>> {
    let (command_tx, command_rx) = mpsc::sync_channel(16);
    std::thread::Builder::new()
        .name("exec-supervisor-input".into())
        .spawn(move || loop {
            let result = recv_command(&mut input);
            let terminal = result.is_err() || matches!(result, Ok(SupervisorCommand::Cancel));
            if command_tx.send(result).is_err() || terminal {
                break;
            }
        })?;
    Ok(command_rx)
}

fn spawn_output<R: Read + Send + 'static>(
    mut reader: R,
    writer: Arc<Mutex<std::io::Stdout>>,
    transferred: Arc<AtomicU64>,
    truncated: Arc<AtomicBool>,
    limit: Option<u64>,
    stderr: bool,
) -> io::Result<std::thread::JoinHandle<io::Result<()>>> {
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
                if send_locked(&writer, event).is_err() {
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

fn send_locked(writer: &Arc<Mutex<std::io::Stdout>>, event: SupervisorEvent) -> io::Result<()> {
    let mut writer = writer
        .lock()
        .map_err(|_| io::Error::other("supervisor writer lock poisoned"))?;
    send_event(&mut *writer, &event)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
