use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use super::core::{eio, now_secs};
use super::exec_platform::{ContainedExec, StopReason};
use super::exec_protocol::ServerFrame;
use super::exec_registry::{ExecCancelReason, ExecCancellation, ExecRegistry, ExecReservation};
use super::exec_supervisor_protocol::{SupervisorEvent, SupervisorExit};
use super::exec_types::{ExecStart, ExecTerminal, ExecTerminalKind};

const EVENT_POLL: Duration = Duration::from_millis(25);
const ROOT_OUTPUT_GRACE: Duration = Duration::from_millis(250);
const CONTAINMENT_STOP_TIMEOUT: Duration = Duration::from_secs(15);

pub(super) enum JobInput {
    Stdin(Vec<u8>),
    StdinEof,
}

struct StopOutcome {
    exit: Option<SupervisorExit>,
    output_complete: bool,
    message: Option<String>,
    stdout_bytes: u64,
    stderr_bytes: u64,
}

impl StopOutcome {
    fn error(message: String, stdout_bytes: u64, stderr_bytes: u64) -> Self {
        Self {
            exit: None,
            output_complete: false,
            message: Some(message),
            stdout_bytes,
            stderr_bytes,
        }
    }
}

pub(super) fn run_contained_job(
    registry: Arc<ExecRegistry>,
    start: ExecStart,
    reservation: ExecReservation,
    mut input: mpsc::Receiver<JobInput>,
    output: mpsc::Sender<ServerFrame>,
) -> io::Result<()> {
    let mut process = match ContainedExec::prepare(&start) {
        Ok(process) => process,
        Err(error) => return fail_preparation(&registry, &reservation, error, &output),
    };
    if let Err(error) = registry.commit_start(&reservation.lease, || {
        process
            .commit(start.clone())
            .map_err(|error| error.to_string())
    }) {
        let _ = process.terminate_all(StopReason::ProtocolError);
        let empty = process
            .confirm_empty(Instant::now() + CONTAINMENT_STOP_TIMEOUT)
            .is_ok();
        return finish_job(
            &registry,
            &reservation,
            terminal_for(&reservation, None, Some(error.to_string()), 0, 0, false),
            empty,
            &output,
        );
    }

    let timeout_at = start
        .effective_timeout_ms()
        .and_then(|milliseconds| Instant::now().checked_add(Duration::from_millis(milliseconds)));
    let mut stdout_bytes = 0u64;
    let mut stderr_bytes = 0u64;
    let mut root_exit: Option<SupervisorExit> = None;
    let mut root_grace = None;
    let mut output_complete = false;
    loop {
        if timeout_at.is_some_and(|deadline| Instant::now() >= deadline) {
            registry.cancel(&start.exec_id, ExecCancelReason::Timeout);
        }
        if root_grace.is_some_and(|deadline| Instant::now() >= deadline) {
            break;
        }
        if reservation.cancellation.reason().is_some() {
            break;
        }
        while let Ok(command) = input.try_recv() {
            match command {
                JobInput::Stdin(bytes) => {
                    if let Err(error) = process.write_stdin(&bytes) {
                        return stop_and_finish(
                            process,
                            &registry,
                            &reservation,
                            &start,
                            &output,
                            StopOutcome::error(error.to_string(), stdout_bytes, stderr_bytes),
                        );
                    }
                }
                JobInput::StdinEof => {
                    if let Err(error) = process.close_stdin() {
                        return stop_and_finish(
                            process,
                            &registry,
                            &reservation,
                            &start,
                            &output,
                            StopOutcome::error(error.to_string(), stdout_bytes, stderr_bytes),
                        );
                    }
                }
            }
        }
        match process.next_event(Some(Instant::now() + EVENT_POLL)) {
            Ok(SupervisorEvent::Started { .. }) => {
                if !emit(
                    &output,
                    ServerFrame::Started {
                        exec_id: start.exec_id.clone(),
                    },
                    &reservation.cancellation,
                ) {
                    registry.cancel(&start.exec_id, ExecCancelReason::Disconnected);
                }
            }
            Ok(SupervisorEvent::Stdout(data)) => {
                stdout_bytes = stdout_bytes.saturating_add(data.len() as u64);
                if !emit(
                    &output,
                    ServerFrame::Stdout {
                        exec_id: start.exec_id.clone(),
                        data,
                    },
                    &reservation.cancellation,
                ) {
                    registry.cancel(&start.exec_id, ExecCancelReason::Disconnected);
                }
            }
            Ok(SupervisorEvent::Stderr(data)) => {
                stderr_bytes = stderr_bytes.saturating_add(data.len() as u64);
                if !emit(
                    &output,
                    ServerFrame::Stderr {
                        exec_id: start.exec_id.clone(),
                        data,
                    },
                    &reservation.cancellation,
                ) {
                    registry.cancel(&start.exec_id, ExecCancelReason::Disconnected);
                }
            }
            Ok(SupervisorEvent::RootExited(exit)) => {
                root_exit = Some(exit);
                root_grace = Some(Instant::now() + ROOT_OUTPUT_GRACE);
            }
            Ok(SupervisorEvent::Exited(exit)) => {
                root_exit = Some(exit);
                output_complete = true;
                break;
            }
            Ok(SupervisorEvent::Error(message)) => {
                return stop_and_finish(
                    process,
                    &registry,
                    &reservation,
                    &start,
                    &output,
                    StopOutcome::error(message, stdout_bytes, stderr_bytes),
                );
            }
            Err(error) if error.kind() == io::ErrorKind::TimedOut => {}
            Err(error) => {
                return stop_and_finish(
                    process,
                    &registry,
                    &reservation,
                    &start,
                    &output,
                    StopOutcome::error(error.to_string(), stdout_bytes, stderr_bytes),
                );
            }
        }
    }
    stop_and_finish(
        process,
        &registry,
        &reservation,
        &start,
        &output,
        StopOutcome {
            exit: root_exit,
            output_complete,
            message: None,
            stdout_bytes,
            stderr_bytes,
        },
    )
}

fn stop_and_finish(
    mut process: ContainedExec,
    registry: &ExecRegistry,
    reservation: &ExecReservation,
    start: &ExecStart,
    output: &mpsc::Sender<ServerFrame>,
    outcome: StopOutcome,
) -> io::Result<()> {
    let reason = reservation.cancellation.reason();
    let _ = process.terminate_all(stop_reason(reason));
    let empty = process
        .confirm_empty(Instant::now() + CONTAINMENT_STOP_TIMEOUT)
        .is_ok();
    let truncated = outcome
        .exit
        .as_ref()
        .is_some_and(|status| status.output_truncated)
        || (!outcome.output_complete
            && start.effective_max_output_bytes().is_some_and(|limit| {
                outcome.stdout_bytes.saturating_add(outcome.stderr_bytes) >= limit
            }));
    finish_job(
        registry,
        reservation,
        terminal_for(
            reservation,
            outcome.exit,
            outcome.message,
            outcome.stdout_bytes,
            outcome.stderr_bytes,
            truncated,
        ),
        empty,
        output,
    )
}

fn terminal_for(
    reservation: &ExecReservation,
    exit: Option<SupervisorExit>,
    message: Option<String>,
    stdout_bytes: u64,
    stderr_bytes: u64,
    output_truncated: bool,
) -> ExecTerminal {
    let kind = match reservation.cancellation.reason() {
        Some(ExecCancelReason::Timeout) => ExecTerminalKind::TimedOut,
        Some(ExecCancelReason::Revoked) => ExecTerminalKind::Revoked,
        Some(ExecCancelReason::Disconnected) => ExecTerminalKind::Disconnected,
        Some(ExecCancelReason::User | ExecCancelReason::WorkerStopping) => {
            ExecTerminalKind::Cancelled
        }
        None if exit.is_some() => ExecTerminalKind::Exited,
        None => ExecTerminalKind::Failed,
    };
    ExecTerminal {
        exec_id: reservation.lease.exec_id.clone(),
        kind,
        exit_code: exit.as_ref().and_then(|exit| exit.code),
        signal: exit.as_ref().and_then(|exit| exit.signal),
        message,
        stdout_bytes,
        stderr_bytes,
        output_truncated,
    }
}

fn finish_job(
    registry: &ExecRegistry,
    reservation: &ExecReservation,
    terminal: ExecTerminal,
    containment_empty: bool,
    output: &mpsc::Sender<ServerFrame>,
) -> io::Result<()> {
    let view = registry
        .record_terminal(&reservation.lease, terminal, containment_empty, now_secs())
        .map_err(eio)?;
    let terminal = view
        .terminal
        .ok_or_else(|| eio("Exec-Registry verlor Terminalstatus"))?;
    output
        .blocking_send(ServerFrame::Terminal(terminal))
        .map_err(eio)
}

fn fail_preparation(
    registry: &ExecRegistry,
    reservation: &ExecReservation,
    error: io::Error,
    output: &mpsc::Sender<ServerFrame>,
) -> io::Result<()> {
    let view = registry
        .fail_preparation(&reservation.lease, error.to_string(), now_secs())
        .map_err(eio)?;
    output
        .blocking_send(ServerFrame::Terminal(
            view.terminal
                .ok_or_else(|| eio("Exec-Vorbereitung verlor Terminalstatus"))?,
        ))
        .map_err(eio)
}

fn emit(
    output: &mpsc::Sender<ServerFrame>,
    mut frame: ServerFrame,
    cancellation: &ExecCancellation,
) -> bool {
    loop {
        match output.try_send(frame) {
            Ok(()) => return true,
            Err(mpsc::error::TrySendError::Closed(_)) => return false,
            Err(mpsc::error::TrySendError::Full(returned)) => {
                frame = returned;
                if cancellation.reason().is_some() {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

fn stop_reason(reason: Option<ExecCancelReason>) -> StopReason {
    match reason {
        Some(ExecCancelReason::User) => StopReason::Cancelled,
        Some(ExecCancelReason::Timeout) => StopReason::TimedOut,
        Some(ExecCancelReason::Revoked) => StopReason::Revoked,
        Some(ExecCancelReason::Disconnected) => StopReason::Disconnected,
        Some(ExecCancelReason::WorkerStopping) => StopReason::WorkerStopping,
        None => StopReason::RootExited,
    }
}
