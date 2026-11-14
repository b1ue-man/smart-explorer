//! E46, S13: the channel of a window of a followed session, against a
//! stand-in for the worker's own IPC (`worker`), and the window's reading of
//! address and token.
use super::*;
use crate::daemon::ipc_protocol::screen::ScreenIpcReply;
use crate::daemon::ipc_storage::channel::{addr_from, token_from};
use serde_json::Value;
use std::io::{BufRead, Cursor};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;

const MAIN_TOKEN: &str = "6d61696e2d746f6b656e2d6f662d7468652d776f726b65722d6970632d313233";
const WAIT: Duration = Duration::from_secs(5);

fn active() -> Option<String> {
    Some("session-1".into())
}

fn switched() -> Option<String> {
    Some("session-2".into())
}

static KNOWN: AtomicBool = AtomicBool::new(false);

/// logind answers only while `KNOWN` is set.
fn flaky() -> Option<String> {
    KNOWN
        .load(Ordering::SeqCst)
        .then(|| "session-1".to_string())
}

/// Open requests: `a` of the laptop, `b` of the phone.
fn devices(id: &str) -> Option<String> {
    match id {
        "a" => Some("laptop".into()),
        "b" => Some("phone".into()),
        _ => None,
    }
}

fn answer(request: &Value) -> IpcResponse {
    let screen = request["req"]["kind"].as_str();
    match (
        request["t"].as_str(),
        screen,
        request["request"]["kind"].as_str(),
    ) {
        (Some("ping"), _, _) => IpcResponse::Pong {
            version: "test".into(),
            generation: "worker".into(),
            initialized: true,
        },
        (Some("screen"), Some("host_sessions" | "host_watch"), _) => {
            IpcResponse::Screen(ScreenIpcReply::Sessions {
                revision: 7,
                sessions: Vec::new(),
            })
        }
        (Some("screen"), Some("host_stop"), _) => {
            IpcResponse::Screen(ScreenIpcReply::Stopped { count: 2 })
        }
        (Some("access"), _, Some("decide")) => {
            IpcResponse::Access(AccessIpcReply::Decided { also_answered: 0 })
        }
        _ => IpcResponse::Err {
            msg: "unexpected request".into(),
            code: None,
        },
    }
}

/// The worker's own IPC: proves `MAIN_TOKEN`, hands every request it got to
/// the receiver and answers it.
fn worker() -> (Main, mpsc::Receiver<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (sender, received) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                return;
            };
            if ipc_proof::answer_for_test(&mut stream, MAIN_TOKEN).is_err() {
                continue;
            }
            let mut line = String::new();
            let deadline = Instant::now() + WAIT;
            if super::super::read_pre_auth_line(&mut stream, &mut line, deadline).is_err() {
                continue;
            }
            let Ok(request) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let response = answer(&request);
            let _ = sender.send(request);
            let _ = write_response(&mut stream, &response);
        }
    });
    let main = Main {
        addr,
        token: MAIN_TOKEN.into(),
    };
    (main, received)
}

fn channel(
    scope: WindowScope,
    current: fn() -> Option<String>,
) -> (WindowChannel, mpsc::Receiver<Value>) {
    let (main, received) = worker();
    let session = WindowSession {
        generation: "session-1".into(),
        current,
    };
    (open_with(scope, session, main, devices).unwrap(), received)
}

/// One request as the window's client sends it (proof, request, answer).
fn ask(addr: SocketAddr, token: &str, request: IpcRequest) -> io::Result<IpcResponse> {
    let mut stream = ipc_proof::connect_token(&addr, Duration::from_secs(2), token)?;
    set_stream_timeout(&stream, Some(WAIT));
    write_request(&mut stream, &request).map_err(|error| io::Error::other(error.to_string()))?;
    read_response(&mut stream).map_err(|error| io::Error::other(error.to_string()))
}

fn refused(response: &IpcResponse) -> bool {
    matches!(response, IpcResponse::Err { msg, code }
        if msg == super::super::admission::REFUSED
            && code.as_deref() == Some(crate::daemon::ipc::AUTH_REJECTED_CODE))
}

fn ping(token: &str) -> IpcRequest {
    IpcRequest::Ping {
        token: token.into(),
    }
}

fn screen(token: &str, req: ScreenIpcRequest) -> IpcRequest {
    IpcRequest::Screen {
        token: token.into(),
        req,
    }
}

fn access(token: &str, request: AccessIpcRequest) -> IpcRequest {
    IpcRequest::Access {
        token: token.into(),
        request,
    }
}

fn decide(token: &str, id: &str, decision: ApprovalDecision) -> IpcRequest {
    let approval_id = id.to_string();
    access(
        token,
        AccessIpcRequest::Decide {
            approval_id,
            decision,
        },
    )
}

#[test]
fn bildschirm_kanal_task_a_window_token_is_refused_outside_its_scope() {
    let (indicator, seen) = channel(WindowScope::Indicator, active);
    let token = indicator.token.clone();
    let outside = [
        IpcRequest::RefreshShare {
            token: token.clone(),
        },
        IpcRequest::ShareSnapshot {
            token: token.clone(),
        },
        IpcRequest::HashCacheClear {
            token: token.clone(),
        },
        IpcRequest::ExecJobs {
            token: token.clone(),
        },
        IpcRequest::ListMounts {
            token: token.clone(),
        },
        access(&token, AccessIpcRequest::Accounts),
        access(&token, AccessIpcRequest::Approvals),
        decide(&token, "a", ApprovalDecision::Once),
    ];
    for request in outside {
        let response = ask(indicator.addr, &token, request).unwrap();
        assert!(refused(&response), "{response:?}");
    }
    // A consent window's token reaches no screen session and no other device.
    let laptop = WindowScope::Consent {
        device: "laptop".into(),
    };
    let (consent, consent_seen) = channel(laptop, active);
    let other = &consent.token;
    let stop = ScreenIpcRequest::HostStop { session: None };
    for request in [
        screen(other, stop),
        screen(other, ScreenIpcRequest::HostSessions),
        decide(other, "b", ApprovalDecision::Deny),
        access(other, AccessIpcRequest::RightsPresets),
    ] {
        let response = ask(consent.addr, other, request).unwrap();
        assert!(refused(&response), "{response:?}");
    }
    // The other window's token is a wrong token here: same answer.
    let mut raw = TcpStream::connect(indicator.addr).unwrap();
    raw.set_read_timeout(Some(WAIT)).unwrap();
    write_request(&mut raw, &ping(other)).unwrap();
    let response = read_response(&mut raw).unwrap();
    assert!(refused(&response), "{response:?}");
    // The daemon token is not a window token, and a wrong token fails the proof.
    let response = ask(indicator.addr, &token, ping(MAIN_TOKEN)).unwrap();
    assert!(refused(&response), "{response:?}");
    assert!(ipc_proof::connect_token(&indicator.addr, WAIT, other).is_err());
    assert!(
        seen.try_recv().is_err(),
        "a refused request reached the worker"
    );
    assert!(consent_seen.try_recv().is_err());
}

#[test]
fn bildschirm_kanal_task_the_indicator_lists_watches_and_ends_the_worker_sessions() {
    let (indicator, seen) = channel(WindowScope::Indicator, active);
    let (addr, token) = (indicator.addr, indicator.token.clone());
    for req in [
        ScreenIpcRequest::HostSessions,
        ScreenIpcRequest::HostWatch { since: 6 },
    ] {
        let response = ask(addr, &token, screen(&token, req)).unwrap();
        assert!(
            matches!(
                response,
                IpcResponse::Screen(ScreenIpcReply::Sessions { revision: 7, .. })
            ),
            "{response:?}"
        );
    }
    for session in [Some("1".to_string()), None] {
        let response = ask(
            addr,
            &token,
            screen(&token, ScreenIpcRequest::HostStop { session }),
        )
        .unwrap();
        assert!(
            matches!(
                response,
                IpcResponse::Screen(ScreenIpcReply::Stopped { count: 2 })
            ),
            "{response:?}"
        );
    }
    let response = ask(addr, &token, ping(&token)).unwrap();
    assert!(matches!(response, IpcResponse::Pong { .. }), "{response:?}");
    let expected = ["host_sessions", "host_watch", "host_stop", "host_stop"];
    for kind in expected {
        let request = seen.recv_timeout(WAIT).unwrap();
        // The worker gets its own token, never the window token.
        assert_eq!(request["token"], MAIN_TOKEN);
        assert_eq!(request["req"]["kind"], kind);
    }
    let request = seen.recv_timeout(WAIT).unwrap();
    assert_eq!(request["t"], "ping");
    assert_eq!(request["token"], MAIN_TOKEN);
}

#[test]
fn bildschirm_kanal_task_a_decision_answers_exactly_the_requests_of_the_window() {
    let laptop = WindowScope::Consent {
        device: "laptop".into(),
    };
    let (consent, seen) = channel(laptop, active);
    let (addr, token) = (consent.addr, consent.token.clone());
    let decisions = [
        ApprovalDecision::Once,
        ApprovalDecision::Always,
        ApprovalDecision::Deny,
    ];
    for decision in decisions {
        let response = ask(addr, &token, decide(&token, "a", decision)).unwrap();
        assert!(
            matches!(
                response,
                IpcResponse::Access(AccessIpcReply::Decided { .. })
            ),
            "{response:?}"
        );
        let request = seen.recv_timeout(WAIT).unwrap();
        assert_eq!(request["token"], MAIN_TOKEN);
        assert_eq!(request["request"]["kind"], "decide");
        assert_eq!(request["request"]["approval_id"], "a");
    }
    // Another device's request and a request that is not open stay untouched.
    for id in ["b", "c", ""] {
        let response = ask(addr, &token, decide(&token, id, ApprovalDecision::Always)).unwrap();
        assert!(refused(&response), "{id}: {response:?}");
    }
    assert!(
        seen.try_recv().is_err(),
        "a refused decision reached the worker"
    );
}

#[test]
fn bildschirm_kanal_task_the_token_ends_with_the_window_and_with_a_session_switch() {
    let (indicator, _seen) = channel(WindowScope::Indicator, active);
    let (addr, token) = (indicator.addr, indicator.token.clone());
    let response = ask(addr, &token, ping(&token)).unwrap();
    assert!(matches!(response, IpcResponse::Pong { .. }), "{response:?}");
    // The launcher drops the guard when the window's process ended.
    drop(indicator.guard);
    // The listener is closed, or it answers like a wrong token.
    if let Ok(response) = ask(addr, &token, ping(&token)) {
        assert!(refused(&response), "{response:?}");
    }

    // Another active session ends the token and the window.
    let (moved, seen) = channel(WindowScope::Indicator, switched);
    let ended = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&ended);
    let end = move || flag.store(true, Ordering::SeqCst);
    moved.guard.window_started(Box::new(end));
    let response = ask(moved.addr, &moved.token, ping(&moved.token)).unwrap();
    assert!(refused(&response), "{response:?}");
    assert!(
        ended.load(Ordering::SeqCst),
        "the window of the old session runs on"
    );
    assert!(
        moved.guard.session_switched(),
        "the launcher starts it anew"
    );
    assert!(seen.try_recv().is_err());

    // An unknown state refuses only that request; the window stays.
    let (unknown, _seen) = channel(WindowScope::Indicator, flaky);
    let kept = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&kept);
    unknown
        .guard
        .window_started(Box::new(move || flag.store(true, Ordering::SeqCst)));
    let response = ask(unknown.addr, &unknown.token, ping(&unknown.token)).unwrap();
    assert!(refused(&response), "{response:?}");
    KNOWN.store(true, Ordering::SeqCst);
    let response = ask(unknown.addr, &unknown.token, ping(&unknown.token)).unwrap();
    assert!(matches!(response, IpcResponse::Pong { .. }), "{response:?}");
    assert!(!kept.load(Ordering::SeqCst));
    assert!(!unknown.guard.session_switched());
}

#[test]
fn bildschirm_kanal_task_the_window_reads_address_and_token_from_variable_and_stdin() {
    let channel: SocketAddr = "127.0.0.1:41000".parse().unwrap();
    assert_eq!(addr_from("127.0.0.1:41000"), Some(channel));
    assert_eq!(addr_from(" 127.0.0.1:41000\n"), Some(channel));
    assert!(addr_from("[::1]:41000").is_some());
    for value in [
        "192.168.1.2:41000",
        "0.0.0.0:41000",
        "127.0.0.1:0",
        "localhost:1",
        "",
    ] {
        assert_eq!(addr_from(value), None, "{value}");
    }
    let token = "ab".repeat(32);
    let mut stdin = Cursor::new(format!("{token}\nrest of the input"));
    assert_eq!(token_from(&mut stdin).unwrap(), token);
    // Only the first line is taken.
    let mut rest = String::new();
    stdin.read_line(&mut rest).unwrap();
    assert_eq!(rest, "rest of the input");
    let long = "a".repeat(200);
    let bad = [
        String::new(),
        "\n".to_string(),
        "abc\n".to_string(),
        format!("{}\n", "g".repeat(64)),
        format!("{token}0\n"),
        format!(" {token}\n"),
        long,
    ];
    for line in bad {
        let mut input = Cursor::new(line.clone());
        assert!(token_from(&mut input).is_err(), "{line:?}");
    }
}
