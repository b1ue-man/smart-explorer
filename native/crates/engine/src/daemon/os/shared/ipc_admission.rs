//! A bounded, nonblocking capability prelude. Unauthenticated connections
//! never reserve a client worker or a long-lived authentication permit.
//!
//! Capabilities: the daemon token for the GUI, `se` and `se mcp`; the
//! launch, backend and session tokens of a drive host for its `MountHost*`
//! requests. Two host requests carry the session token of their drive in
//! other fields: `MountHostExt` (`session_token` of its `id`) and
//! `Vault::SpoolKey` (`token`; the mount id sits in the payload behind the
//! prefix). The prelude admits `vault` with a session token of any running
//! drive host; `ipc.rs::require_request_auth` then admits exactly
//! `SpoolKey` for exactly the mount of that token and nothing else.
//!
//! A decoded capability the prelude refuses gets one explicit
//! `IpcResponse::Err` (no token, no request field) instead of a silently
//! dropped connection; malformed or empty prefixes are still dropped.
use std::collections::VecDeque;
use std::io::{self, Read};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde::Deserializer as _;

use super::ShareHost;

const MAX_PENDING: usize = 256;
const PREFIX_BYTES: usize = 16 * 1024;
const PRELUDE_TIMEOUT: Duration = Duration::from_millis(250);
const POLL_BUDGET: usize = 64;
pub(super) const PRELUDE_WAIT: Duration = Duration::from_millis(10);

struct Pending {
    stream: TcpStream,
    deadline: Instant,
}

#[derive(Default)]
pub(super) struct AdmissionQueue {
    pending: VecDeque<Pending>,
}

impl AdmissionQueue {
    pub(super) fn accept(
        &mut self,
        stream: TcpStream,
        peer: SocketAddr,
        host: &ShareHost,
        token: &str,
        mut serve: impl FnMut(TcpStream),
    ) {
        if !peer.ip().is_loopback() || stream.set_nonblocking(true).is_err() {
            return;
        }
        match inspect(&stream, |hint| authorized(hint, host, token)) {
            Ok(Some(Verdict::Admit)) => serve(stream),
            Ok(Some(Verdict::Refuse)) => refuse(stream),
            Ok(None) => {
                // Reclaim an unproven connection rather than reserve all
                // capacity against newly arriving legitimate clients.
                if self.pending.len() == MAX_PENDING {
                    self.pending.pop_front();
                }
                self.pending.push_back(Pending {
                    stream,
                    deadline: Instant::now() + PRELUDE_TIMEOUT,
                });
            }
            _ => {}
        }
    }

    pub(super) fn poll(
        &mut self,
        host: &ShareHost,
        token: &str,
        mut serve: impl FnMut(TcpStream),
    ) -> bool {
        let count = self.pending.len().min(POLL_BUDGET);
        for _ in 0..count {
            let Some(pending) = self.pending.pop_front() else {
                break;
            };
            if Instant::now() >= pending.deadline {
                continue;
            }
            match inspect(&pending.stream, |hint| authorized(hint, host, token)) {
                Ok(Some(Verdict::Admit)) => serve(pending.stream),
                Ok(Some(Verdict::Refuse)) => refuse(pending.stream),
                Ok(None) => self.pending.push_back(pending),
                _ => {}
            }
        }
        !self.pending.is_empty()
    }
}

/// Host requests whose capability belongs to the drive named by `id`.
const MOUNT_HOST_KINDS: [&str; 4] = [
    "mount_host_attach",
    "mount_host_backend",
    "mount_host_status",
    "mount_host_ext",
];

// No Debug implementation: the prelude contains a live IPC capability.
#[derive(Default)]
struct Hint {
    kind: Option<String>,
    id: Option<crate::mount::MountId>,
    token: Option<String>,
    token_field: Option<String>,
}

impl Hint {
    fn complete(&self) -> bool {
        match self.kind.as_deref() {
            Some(kind) if MOUNT_HOST_KINDS.contains(&kind) => {
                self.id.is_some() && self.token.is_some()
            }
            Some(_) => self.token.is_some(),
            None => false,
        }
    }
}

struct HintVisitor<'a> {
    captured: &'a mut Option<Hint>,
}

impl<'de> Visitor<'de> for HintVisitor<'_> {
    type Value = Hint;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("an IPC capability prefix")
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Hint, M::Error> {
        let mut hint = Hint::default();
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "t" => hint.kind = Some(map.next_value()?),
                "id" => hint.id = Some(map.next_value()?),
                "token" | "launch_token" | "backend_token" | "session_token" => {
                    hint.token = Some(map.next_value()?);
                    hint.token_field = Some(key);
                }
                _ => {
                    let _ = map.next_value::<IgnoredAny>()?;
                }
            }
            if hint.complete() {
                // serde_json ends the whole map even after an early Ok.
                // Capture only complete fields, then stop before the payload.
                *self.captured = Some(hint);
                return Err(serde::de::Error::custom("IPC capability prefix captured"));
            }
        }
        Err(serde::de::Error::custom("IPC capability is missing"))
    }
}

fn decode_hint(bytes: &[u8]) -> Result<Hint, serde_json::Error> {
    let mut captured = None;
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let result = decoder.deserialize_map(HintVisitor {
        captured: &mut captured,
    });
    captured.map(Ok).unwrap_or(result)
}

/// What the prelude decided about one connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    /// The capability holds: a client worker serves the request.
    Admit,
    /// A complete capability that does not hold: answered, then closed.
    Refuse,
    /// Empty or malformed prefix: dropped without an answer.
    Drop,
}

fn inspect(
    stream: &TcpStream,
    authorize: impl FnOnce(&Hint) -> bool,
) -> io::Result<Option<Verdict>> {
    let mut bytes = [0u8; PREFIX_BYTES];
    let count = match stream.peek(&mut bytes) {
        Ok(0) => return Ok(Some(Verdict::Drop)),
        Ok(count) => count,
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
        Err(error) => return Err(error),
    };
    match decode_hint(&bytes[..count]) {
        Ok(hint) if authorize(&hint) => Ok(Some(Verdict::Admit)),
        Ok(_) => Ok(Some(Verdict::Refuse)),
        Err(error) if error.is_eof() && count < PREFIX_BYTES => Ok(None),
        Err(_) => Ok(Some(Verdict::Drop)),
    }
}

/// Text of a refused capability. It names neither the token nor any field
/// of the request.
const REFUSED: &str =
    "Daemon-IPC-Anfrage abgelehnt: die Berechtigung ist ungueltig oder gehoert zu keinem laufenden Laufwerk-Host";

/// Answers a refused capability on the still nonblocking stream. The
/// request line already received is consumed first (up to its newline, at
/// most 16 KiB, never waiting), so closing the socket sends FIN and not RST:
/// an RST makes the client's read fail with WSAECONNRESET (10054) or
/// ECONNRESET before it sees the answer.
fn refuse(mut stream: TcpStream) {
    let mut sink = [0u8; 1024];
    let mut consumed = 0usize;
    loop {
        let want = sink.len().min(PREFIX_BYTES.saturating_sub(consumed));
        if want == 0 {
            break;
        }
        match stream.read(&mut sink[..want]) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                consumed += read;
                if sink[..read].contains(&b'\n') {
                    break;
                }
            }
        }
    }
    let _ = super::super::ipc_protocol::write_response(
        &mut stream,
        &super::super::ipc_protocol::IpcResponse::Err {
            msg: REFUSED.into(),
            code: Some(super::super::ipc::AUTH_REJECTED_CODE.into()),
        },
    );
    let _ = stream.shutdown(Shutdown::Write);
}

fn authorized(hint: &Hint, host: &ShareHost, expected_token: &str) -> bool {
    let (Some(kind), Some(token)) = (hint.kind.as_deref(), hint.token.as_deref()) else {
        return false;
    };
    match kind {
        "mount_host_attach" => {
            hint.token_field.as_deref() == Some("launch_token")
                && hint
                    .id
                    .as_ref()
                    .is_some_and(|id| host.mounts.check_launch_token(id, token).is_ok())
        }
        "mount_host_backend" => {
            hint.token_field.as_deref() == Some("backend_token")
                && hint
                    .id
                    .as_ref()
                    .is_some_and(|id| host.mounts.check_backend_token(id, token).is_ok())
        }
        "mount_host_status" | "mount_host_ext" => {
            hint.token_field.as_deref() == Some("session_token")
                && hint
                    .id
                    .as_ref()
                    .is_some_and(|id| host.mounts.check_session_token(id, token).is_ok())
        }
        // `Vault::SpoolKey` carries the session token of its drive host in
        // `token`; every other vault request carries the daemon token there.
        "vault" => {
            hint.token_field.as_deref() == Some("token")
                && (constant_time_eq(expected_token, token)
                    || host.mounts.is_host_session_token(token))
        }
        _ => {
            hint.token_field.as_deref() == Some("token") && constant_time_eq(expected_token, token)
        }
    }
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.bytes()
        .zip(right.bytes())
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;

    #[test]
    fn review_task_ipc_prelude_does_not_consume_the_original_request() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        server.set_nonblocking(true).unwrap();
        assert_eq!(inspect(&server, |_| true).unwrap(), None);
        client
            .write_all(b"{\"t\":\"ping\",\"token\":\"valid\"}\n")
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if let Some(verdict) =
                inspect(&server, |hint| hint.token.as_deref() == Some("valid")).unwrap()
            {
                assert_eq!(verdict, Verdict::Admit);
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        server.set_nonblocking(false).unwrap();
        server
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        use std::io::Read;
        let expected = b"{\"t\":\"ping\",\"token\":\"valid\"}\n";
        let mut bytes = vec![0u8; expected.len()];
        (&server).read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, expected);
    }

    #[test]
    fn review_task_ipc_prelude_allows_large_authenticated_payload_after_small_prefix() {
        let hint = decode_hint(b"{\"t\":\"share_command\",\"token\":\"valid\",\"cmd\":").unwrap();
        assert_eq!(hint.token.as_deref(), Some("valid"));
        assert!(decode_hint(b"{\"t\":\"share_command\",\"token\":\"val").is_err());
        assert!(decode_hint(b"{\"t\":\"share_command\",\"token\":123}").is_err());
        assert!(constant_time_eq("same", "same"));
        assert!(!constant_time_eq("same", "else"));
    }
}
