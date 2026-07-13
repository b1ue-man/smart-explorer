use std::io::{self, Read};
use std::os::unix::process::ExitStatusExt;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use crate::share::exec_supervisor_protocol::{
    recv_command, send_event, SupervisorCommand, SupervisorEvent, SupervisorExit, SupervisorStart,
};
use crate::share::exec_types::ExecCommand;

pub(super) fn run(mut stream: std::os::unix::net::UnixStream) -> io::Result<()> {
    let start = match recv_command(&mut stream)? {
        SupervisorCommand::Start(start) => start,
        _ => return Err(invalid("supervisor expected a start frame")),
    };
    start.request.validate()?;
    let mut child = match spawn(&start) {
        Ok(child) => child,
        Err(error) => {
            let _ = send_event(&mut stream, &SupervisorEvent::Error(error.to_string()));
            return Ok(());
        }
    };
    send_event(&mut stream, &SupervisorEvent::Started { pid: child.id() })?;

    let writer = Arc::new(Mutex::new(stream.try_clone()?));
    let event_writer = writer.clone();
    let output = Arc::new(AtomicU64::new(0));
    let truncated = Arc::new(AtomicBool::new(false));
    let limit = start.request.effective_max_output_bytes();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| invalid("stdout pipe missing"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| invalid("stderr pipe missing"))?;
    let stdout_thread = spawn_output(
        stdout,
        writer.clone(),
        output.clone(),
        truncated.clone(),
        limit,
        false,
    )?;
    let stderr_thread = spawn_output(stderr, writer, output, truncated.clone(), limit, true)?;

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

    let mut stdin = child.stdin.take();
    loop {
        if let Some(status) = child.try_wait()? {
            let mut exit = SupervisorExit {
                code: status.code(),
                signal: status.signal(),
                output_truncated: false,
            };
            send_locked_event(&event_writer, SupervisorEvent::RootExited(exit.clone()))?;
            join_output(stdout_thread)?;
            join_output(stderr_thread)?;
            exit.output_truncated = truncated.load(Ordering::Acquire);
            send_locked_event(&event_writer, SupervisorEvent::Exited(exit))?;
            return Ok(());
        }
        match command_rx.recv_timeout(Duration::from_millis(20)) {
            Ok(Ok(SupervisorCommand::Stdin(data))) => write_stdin(&mut stdin, &data)?,
            Ok(Ok(SupervisorCommand::StdinEof)) => drop(stdin.take()),
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
                .get("SHELL")
                .filter(|value| value.starts_with('/'))
                .map(String::as_str)
                .unwrap_or("/bin/sh");
            let mut shell_command = Command::new(shell);
            shell_command.args(["-lc", command]);
            shell_command
        }
    };
    command
        .env_clear()
        .envs(&start.environment)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = start.request.cwd.as_deref().or_else(|| {
        start
            .environment
            .get("HOME")
            .filter(|home| !home.is_empty())
            .map(String::as_str)
    }) {
        command.current_dir(cwd);
    }
    command.spawn()
}

fn spawn_output<R: Read + Send + 'static>(
    mut reader: R,
    writer: Arc<Mutex<std::os::unix::net::UnixStream>>,
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

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
