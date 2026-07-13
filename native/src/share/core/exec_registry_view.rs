use super::{ActiveJob, ExecJobView, ExecLifecycleState, ExecTerminal, ExecTerminalKind};

pub(super) fn view(job: &ActiveJob) -> ExecJobView {
    ExecJobView {
        exec_id: job.lease.exec_id.clone(),
        peer_device_id: job.lease.principal.device_id.clone(),
        peer_device_name: job.lease.principal.device_name.clone(),
        program: job.program.clone(),
        command_digest: job.lease.command_digest.clone(),
        state: job.state.clone(),
        policy_revision: job.lease.policy_revision,
        started_at: job.started_at,
        finished_at: None,
        terminal: None,
    }
}

pub(super) fn terminal_view(
    job: &ActiveJob,
    terminal: ExecTerminal,
    finished_at: i64,
) -> ExecJobView {
    let mut result = view(job);
    result.state = match terminal.kind {
        ExecTerminalKind::Exited => ExecLifecycleState::Exited,
        ExecTerminalKind::Failed => ExecLifecycleState::Failed,
        ExecTerminalKind::TimedOut => ExecLifecycleState::TimedOut,
        ExecTerminalKind::Cancelled => ExecLifecycleState::Cancelled,
        ExecTerminalKind::Revoked => ExecLifecycleState::Revoked,
        ExecTerminalKind::Disconnected => ExecLifecycleState::Disconnected,
    };
    result.finished_at = Some(finished_at);
    result.terminal = Some(terminal);
    result
}
