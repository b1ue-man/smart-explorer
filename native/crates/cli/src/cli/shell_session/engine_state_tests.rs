//! State-machine tests of `engine_state.rs`: order, routing, input, end.
use super::EngineState;
use crate::cli::shell_session::transports::TransportEnd;
use crate::cli::shell_session::types::{
    Activity, CommandRequest, Delivery, Dialect, OutcomeStatus, OwnEvent, StdinMode, Stream,
};
use std::collections::BTreeMap;

const NONCE: &str = "00ff";

fn request(command: &str) -> CommandRequest {
    CommandRequest {
        command: command.into(),
        cwd: None,
        env: BTreeMap::new(),
        stdin: StdinMode::Null,
    }
}

fn markers(seq: u64, exit: i32, cwd: &str) -> (Vec<u8>, Vec<u8>) {
    (
        format!("\x1eSE{NONCE}:{seq}:{exit}:{cwd}\x1e").into_bytes(),
        format!("\x1eSE{NONCE}:{seq}\x1e").into_bytes(),
    )
}

fn ready_state() -> EngineState {
    let (mut state, first) = EngineState::new(Dialect::Posix, NONCE.into(), &request(":")).unwrap();
    assert!(matches!(first.as_slice(), [Delivery::Write(_)]));
    state.output(Stream::Stderr, b"profile noise\n");
    let (out, err) = markers(0, 0, "/home/u");
    assert!(state.output(Stream::Stdout, &out).is_empty());
    state.output(Stream::Stderr, &err);
    assert!(state.ready());
    assert_eq!(state.startup_failure(), None);
    state
}

fn writes(deliveries: &[Delivery]) -> usize {
    deliveries
        .iter()
        .filter(|delivery| matches!(delivery, Delivery::Write(_)))
        .count()
}

#[test]
fn shell_session_task_state_runs_one_command_at_a_time_in_order() {
    let mut state = ready_state();
    let first = state.submit(1, "owner", 10, request("echo a")).unwrap();
    assert_eq!(writes(&first), 1);
    // The second command waits until the first one's markers arrived.
    assert_eq!(
        writes(&state.submit(2, "agent", 20, request("echo b")).unwrap()),
        0
    );
    let (out, err) = markers(1, 0, "/srv");
    assert_eq!(writes(&state.output(Stream::Stdout, &out)), 0);
    let done = state.output(Stream::Stderr, &err);
    assert_eq!(writes(&done), 1, "the queued command starts");
    assert!(done.iter().any(|delivery| matches!(delivery, Delivery::Own { to: 1, event: OwnEvent::Done { tag: 10, outcome } } if outcome.exit_code == Some(0) && outcome.cwd.as_deref() == Some("/srv"))));
    assert!(done.iter().any(|delivery| matches!(delivery, Delivery::Activity { except: Some(2), activity: Activity::Started { by, .. } } if by == "agent")));
    assert_eq!(state.cwd.as_deref(), Some("/srv"));
    assert!(
        state.input(1, b"x").is_err(),
        "input only for an own interactive command"
    );
}

#[test]
fn shell_session_task_state_routes_output_and_activity() {
    let mut state = ready_state();
    state.output(Stream::Stdout, b"background\n");
    let started = state.submit(1, "owner", 10, request("ls")).unwrap();
    // Output that arrived between commands belongs to the next command.
    assert!(started.iter().any(|delivery| matches!(delivery, Delivery::Own { to: 1, event: OwnEvent::Output { stream: Stream::Stdout, bytes, .. } } if bytes == b"background\n")));
    let routed = state.output(Stream::Stderr, b"warn");
    assert!(routed.iter().any(|delivery| matches!(
        delivery,
        Delivery::Own {
            to: 1,
            event: OwnEvent::Output {
                stream: Stream::Stderr,
                ..
            }
        }
    )));
    assert!(routed.iter().any(|delivery| matches!(
        delivery,
        Delivery::Activity {
            except: Some(1),
            activity: Activity::Output { .. }
        }
    )));
    let mut typed = request("read x");
    typed.stdin = StdinMode::Interactive;
    let (out, err) = markers(1, 2, "/");
    state.output(Stream::Stdout, &out);
    state.output(Stream::Stderr, &err);
    state.submit(3, "shell", 30, typed).unwrap();
    assert_eq!(writes(&state.input(3, b"line\n").unwrap()), 1);
    assert!(state.input(1, b"line\n").is_err());
    assert!(state.submit(1, "owner", 11, request("")).is_err());
}

#[test]
fn shell_session_task_state_ends_running_and_queued_commands() {
    let mut state = ready_state();
    state.submit(1, "owner", 10, request("sleep 9")).unwrap();
    state.submit(2, "agent", 20, request("echo later")).unwrap();
    state.cancelled("agent");
    let ended = state.ended(TransportEnd {
        cancelled: true,
        ..TransportEnd::default()
    });
    let outcomes: Vec<_> = ended
        .iter()
        .filter_map(|delivery| match delivery {
            Delivery::Own {
                to,
                event: OwnEvent::Done { outcome, .. },
            } => Some((*to, outcome.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes[0].1.status, OutcomeStatus::Cancelled);
    assert!(outcomes[0].1.session_ended);
    assert_eq!(outcomes[1].1.status, OutcomeStatus::Failed);
    assert!(ended.iter().any(|delivery| matches!(delivery, Delivery::Activity { except: None, activity: Activity::Ended { message, .. } } if message.contains("agent"))));
    assert!(state.submit(1, "owner", 12, request("ls")).is_err());
    // `exit 3` while running: the command completed with the shell's status.
    let mut state = ready_state();
    state.submit(1, "owner", 10, request("exit 3")).unwrap();
    let ended = state.ended(TransportEnd {
        exit_code: Some(3),
        ..TransportEnd::default()
    });
    assert!(ended.iter().any(|delivery| matches!(delivery, Delivery::Own { event: OwnEvent::Done { outcome, .. }, .. } if outcome.status == OutcomeStatus::Completed && outcome.exit_code == Some(3) && outcome.session_ended)));
    // A failed startup is reported with the shell's own error text.
    let (mut state, _) = EngineState::new(Dialect::Posix, NONCE.into(), &request(":")).unwrap();
    state.output(Stream::Stderr, b"cd: /nope: No such file or directory\n");
    let (out, err) = markers(0, 1, "/home/u");
    state.output(Stream::Stdout, &out);
    state.output(Stream::Stderr, &err);
    let failure = state.startup_failure().unwrap();
    assert!(failure.contains("No such file"), "{failure}");
    assert!(state.close_input().is_some());
}
