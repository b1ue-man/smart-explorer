//! Opening a Share peer through the background worker (the app, `se`, the
//! drives). The worker tries one target for up to 45 s (`ShareHost::open_share`)
//! and then names the cause, such as a refused login or an unreachable peer.
//! The client waits for that answer: a shorter socket timeout ended every
//! slower open with the operating system's text for a timed-out socket, on
//! Windows „…da die Gegenstelle nach einer bestimmten Zeitspanne nicht
//! richtig reagiert hat…“ (os error 10060), which blamed the peer for the
//! wait on the local worker.
use std::io;
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use super::super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse,
};
use super::super::ipc_storage::{read_ipc_addr, read_token};
use super::{ensure_worker_ready, restart_worker_for_client, share_backend_identity};

/// Longest wait for the worker's answer: its 45 s of attempts, one last peer
/// session handshake of up to 20 s, and a margin.
const OPEN_ANSWER_TIMEOUT: Duration = Duration::from_secs(75);

/// The outcome of one request to the worker.
enum OpenAttempt {
    /// The worker opened the target; the stream now carries its file protocol.
    Opened {
        stream: TcpStream,
        label: String,
        status: crate::share::ShareStatus,
    },
    /// The worker answered with an error, or gave no answer in time. Final: a
    /// new request would start the same attempts again.
    Failed(String),
    /// The connection to the worker broke before its answer.
    Broken(String),
    /// No worker accepted the connection.
    Unreachable,
}

pub fn open_share_backend(
    target: crate::share::PeerOpenTarget,
) -> Result<(String, crate::vfs::BackendHandle, crate::share::ShareStatus), String> {
    ensure_worker_ready()?;
    let token = read_token().map_err(|error| format!("Background-Worker Token: {error}"))?;
    let mut last = "Background-Worker nicht erreichbar".to_string();
    let mut restarted_after_agent_error = false;
    let mut restarted_after_missing_ipc = false;
    for _ in 0..8 {
        let attempt = match read_ipc_addr() {
            Some(addr) => open_attempt(addr, &token, &target, OPEN_ANSWER_TIMEOUT),
            None => OpenAttempt::Unreachable,
        };
        match attempt {
            OpenAttempt::Opened {
                stream,
                label,
                status,
            } => {
                let read = stream.try_clone().map_err(|error| error.to_string())?;
                let inner = share_backend_identity(label.clone(), target.clone());
                match crate::agent::AgentBackend::from_streams(
                    Box::new(read),
                    Box::new(stream),
                    inner,
                ) {
                    Ok(agent) => return Ok((label, Arc::new(agent), status)),
                    Err(error) => {
                        last = format!("Worker-Backend: {error}");
                        if restarted_after_agent_error {
                            return Err(last);
                        }
                        restarted_after_agent_error = true;
                        restart_worker_for_client()?;
                        std::thread::sleep(Duration::from_millis(750));
                    }
                }
            }
            OpenAttempt::Failed(message) => return Err(message),
            OpenAttempt::Broken(error) => {
                last = error;
                std::thread::sleep(Duration::from_millis(250));
            }
            OpenAttempt::Unreachable => {
                if !restarted_after_missing_ipc {
                    restarted_after_missing_ipc = true;
                    restart_worker_for_client()?;
                }
                std::thread::sleep(Duration::from_millis(750));
            }
        }
    }
    Err(last)
}

/// One `OpenShare` request to the worker at `addr`, waiting up to
/// `answer_timeout` for its answer.
fn open_attempt(
    addr: SocketAddr,
    token: &str,
    target: &crate::share::PeerOpenTarget,
    answer_timeout: Duration,
) -> OpenAttempt {
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(2)) else {
        return OpenAttempt::Unreachable;
    };
    set_stream_timeout(&stream, Some(answer_timeout));
    let request = IpcRequest::OpenShare {
        token: token.to_string(),
        target: target.clone(),
    };
    if let Err(error) = write_request(&mut stream, &request) {
        return OpenAttempt::Broken(error.to_string());
    }
    let response = match read_response(&mut stream) {
        Ok(response) => response,
        Err(error) if timed_out(&error) => {
            return OpenAttempt::Failed(format!(
                "Der Hintergrund-Worker hat {} s lang nicht gemeldet, ob {} erreichbar ist",
                answer_timeout.as_secs(),
                target.endpoint_prefix()
            ));
        }
        Err(error) => return OpenAttempt::Broken(error.to_string()),
    };
    match response {
        IpcResponse::OpenOk { label, status } => {
            set_stream_timeout(&stream, None);
            OpenAttempt::Opened {
                stream,
                label,
                status,
            }
        }
        IpcResponse::Err { msg, .. } => OpenAttempt::Failed(msg),
        _ => OpenAttempt::Failed("Unerwartete Worker-Antwort".into()),
    }
}

/// A socket read timeout: `WouldBlock` on Unix, `TimedOut` on Windows.
fn timed_out(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

#[cfg(test)]
mod tests {
    use super::{open_attempt, OpenAttempt};
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    fn target() -> crate::share::PeerOpenTarget {
        crate::share::PeerOpenTarget::Direct {
            contact_id: "win10".into(),
        }
    }

    /// A worker that reads one request, waits `delay` and then answers with
    /// `answer` (or closes without one).
    fn worker(delay: Duration, answer: Option<&'static str>) -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            assert!(request.contains("\"open_share\""), "{request}");
            assert!(request.contains("win10"), "{request}");
            std::thread::sleep(delay);
            if let Some(answer) = answer {
                let mut stream = stream;
                stream.write_all(answer.as_bytes()).unwrap();
                stream.write_all(b"\n").unwrap();
            }
        });
        addr
    }

    #[test]
    fn windows_direct_task_open_waits_for_the_worker_answer() {
        // The worker needs longer than the old 3 s socket timeout to name the
        // cause; the client reports the cause, not its own timeout.
        let addr = worker(
            Duration::from_secs(4),
            Some(r#"{"t":"err","msg":"Share-Anmeldung nicht autorisiert"}"#),
        );
        match open_attempt(addr, "token", &target(), Duration::from_secs(20)) {
            OpenAttempt::Failed(message) => {
                assert_eq!(message, "Share-Anmeldung nicht autorisiert")
            }
            _ => panic!("the worker's answer was not reported"),
        }
    }

    #[test]
    fn windows_direct_task_open_names_a_silent_worker() {
        let addr = worker(Duration::from_secs(3), None);
        let started = Instant::now();
        match open_attempt(addr, "token", &target(), Duration::from_secs(1)) {
            OpenAttempt::Failed(message) => {
                assert!(message.contains("Hintergrund-Worker"), "{message}");
                assert!(message.contains("share://direct/win10"), "{message}");
            }
            _ => panic!("a silent worker was not reported as such"),
        }
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn windows_direct_task_open_retries_a_broken_connection() {
        let addr = worker(Duration::ZERO, None);
        assert!(matches!(
            open_attempt(addr, "token", &target(), Duration::from_secs(5)),
            OpenAttempt::Broken(_)
        ));
    }
}
