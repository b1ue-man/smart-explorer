//! Interactive presence of an Exec job with a terminal on the host (plan
//! 2026-11-14 C2 E3, package P2b). The client side registers its end in
//! `node_routes.rs`; the host registers its end here, from the start of the
//! job to every end of its session: result, error, cancellation, closed
//! transport, grant end and unwinding. The entry is the guard of
//! `serve_job`, so it ends when that future ends or is dropped.
use crate::share::exec_types::ExecStart;
use crate::share::keepalive::interactive_presence::{
    self, InteractiveGuard, InteractiveKind, InteractivePresence,
};

/// The entry of the job `start` in the process register: one for a job with
/// a terminal, none for a job without (its output is not latency sensitive).
pub(super) fn host_interactive(start: &ExecStart) -> Option<InteractiveGuard> {
    host_interactive_in(interactive_presence::global(), start)
}

fn host_interactive_in(
    presence: &'static InteractivePresence,
    start: &ExecStart,
) -> Option<InteractiveGuard> {
    start
        .terminal
        .is_some()
        .then(|| presence.enter(InteractiveKind::ExecTerminal))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::share::exec_terminal::{ExecTerminalRequest, ExecTerminalSize};
    use crate::share::exec_types::{ExecCommand, ExecId};

    fn register() -> &'static InteractivePresence {
        Box::leak(Box::new(InteractivePresence::new()))
    }

    fn start(terminal: bool) -> ExecStart {
        ExecStart {
            exec_id: ExecId::parse("ab".repeat(16)).unwrap(),
            command: if terminal {
                ExecCommand::Login
            } else {
                ExecCommand::Shell {
                    command: "printf x".into(),
                }
            },
            cwd: None,
            env: BTreeMap::new(),
            timeout_ms: None,
            max_output_bytes: None,
            terminal: terminal.then(|| ExecTerminalRequest {
                term: "xterm-256color".into(),
                size: ExecTerminalSize { cols: 80, rows: 24 },
            }),
        }
    }

    #[test]
    fn verbindungen_task_exec_host_terminal_job_counts_until_its_guard_drops() {
        let presence = register();
        assert!(host_interactive_in(presence, &start(false)).is_none());
        assert!(!presence.active());
        let first = host_interactive_in(presence, &start(true)).expect("terminal job");
        let second = host_interactive_in(presence, &start(true)).expect("terminal job");
        assert_eq!(first.kind(), InteractiveKind::ExecTerminal);
        assert_eq!(presence.count(InteractiveKind::ExecTerminal), 2);
        // Other kinds are not touched.
        assert_eq!(presence.count(InteractiveKind::TcpPump), 0);
        drop(first);
        assert!(presence.active());
        drop(second);
        assert!(!presence.active());
    }

    #[test]
    fn verbindungen_task_exec_host_entry_ends_on_error_abort_and_panic() {
        let presence = register();
        let terminal = start(true);
        // An error return of the serving future drops its entry.
        let failed = || -> Result<(), &'static str> {
            let _interactive = host_interactive_in(presence, &terminal);
            assert_eq!(presence.count(InteractiveKind::ExecTerminal), 1);
            Err("exec peer unresponsive")
        };
        assert!(failed().is_err());
        assert!(!presence.active());
        // A serving task that is aborted (the connection task ends) drops it.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let guard = host_interactive_in(presence, &terminal);
            let task = tokio::spawn(async move {
                let _interactive = guard;
                std::future::pending::<()>().await;
            });
            tokio::task::yield_now().await;
            assert_eq!(presence.count(InteractiveKind::ExecTerminal), 1);
            task.abort();
            let _ = task.await;
        });
        assert!(!presence.active());
        // Unwinding drops it as well.
        let unwound = std::panic::catch_unwind(|| {
            let _interactive = host_interactive_in(presence, &terminal);
            panic!("exec job failed");
        });
        assert!(unwound.is_err());
        assert!(!presence.active());
    }
}
