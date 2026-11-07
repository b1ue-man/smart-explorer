//! The worker proves that it holds the daemon token before a client sends
//! it (r-lokal-1, review 2026-11-07).
//!
//! A loopback port belongs to no user: after the worker ended, another
//! local user could bind the port still named in `daemon.ipc` and read the
//! token from the next request. So a client first sends a fresh nonce,
//! `{"t":"ipc_proof","nonce":"<64 hex>"}`, and the worker answers
//! `{"t":"ipc_proof","proof":"<64 hex>"}` with
//! `HMAC-SHA256(token, DOMAIN || nonce || "127.0.0.1:<port>")`, the address
//! the client dialed. Only a worker holding the token can answer; binding the
//! address keeps a squatter from relaying the nonce to the real worker on its
//! new port. The client sends its request (with the token) only after this
//! check. The worker still accepts requests without the proof (older
//! clients of the same token during an update); clients of this version
//! always ask for it.
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use ring::hmac;
use serde::{Deserialize, Serialize};

const DOMAIN: &[u8] = b"smart-explorer daemon ipc server proof v1\n";
const KIND: &str = "ipc_proof";
/// Longer than either proof line; a longer line is refused.
pub(super) const MAX_PROOF_LINE: usize = 256;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProofRequest {
    t: String,
    nonce: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProofReply {
    t: String,
    proof: String,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (index, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(text.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

fn message(nonce: &[u8; 32], server: &SocketAddr) -> Vec<u8> {
    let mut message = DOMAIN.to_vec();
    message.extend_from_slice(nonce);
    message.extend_from_slice(server.to_string().as_bytes());
    message
}

fn key(token: &str) -> hmac::Key {
    hmac::Key::new(hmac::HMAC_SHA256, token.as_bytes())
}

/// What a peeked first line is with respect to the proof.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ProofLine {
    /// Not a proof request: the ordinary capability prelude decides.
    NotProof,
    /// A proof request whose line has not fully arrived yet.
    Incomplete,
    /// A complete request: its length with newline, and the nonce.
    Complete(usize, [u8; 32]),
    /// A malformed or oversized proof request.
    Malformed,
}

pub(super) fn parse_request(bytes: &[u8]) -> ProofLine {
    let prefix = b"{\"t\":\"ipc_proof\"";
    let probe = &bytes[..bytes.len().min(prefix.len())];
    if !prefix.starts_with(probe) {
        return ProofLine::NotProof;
    }
    let Some(end) = bytes.iter().position(|byte| *byte == b'\n') else {
        if bytes.len() > MAX_PROOF_LINE {
            return ProofLine::Malformed;
        }
        return ProofLine::Incomplete;
    };
    if end > MAX_PROOF_LINE {
        return ProofLine::Malformed;
    }
    let Ok(request) = serde_json::from_slice::<ProofRequest>(&bytes[..end]) else {
        return ProofLine::Malformed;
    };
    match (request.t == KIND, unhex32(&request.nonce)) {
        (true, Some(nonce)) => ProofLine::Complete(end + 1, nonce),
        _ => ProofLine::Malformed,
    }
}

/// The worker's answer line for `nonce` on the connection it accepted at
/// `local` (its own listening address).
pub(super) fn reply_line(token: &str, nonce: &[u8; 32], local: &SocketAddr) -> io::Result<Vec<u8>> {
    let tag = hmac::sign(&key(token), &message(nonce, local));
    let mut line = serde_json::to_vec(&ProofReply {
        t: KIND.into(),
        proof: hex(tag.as_ref()),
    })
    .map_err(io::Error::other)?;
    line.push(b'\n');
    Ok(line)
}

/// Connects to the worker at `addr` and checks its proof before returning
/// the stream; the caller then writes its request as before. The stream
/// keeps no timeout of this check (like `TcpStream::connect_timeout`).
pub(crate) fn connect(addr: &SocketAddr, timeout: Duration) -> io::Result<TcpStream> {
    let token = super::ipc_storage::read_token()?;
    connect_token(addr, timeout, &token)
}

/// [`connect`] with a token the caller already read.
pub(super) fn connect_token(
    addr: &SocketAddr,
    timeout: Duration,
    token: &str,
) -> io::Result<TcpStream> {
    connect_with_token(addr, timeout, token).map_err(|failure| match failure {
        ProofFailure::Unreachable(error) => error,
        ProofFailure::Unproven => refused(),
    })
}

/// Test support for stand-in workers: reads one proof request from
/// `stream` and answers it for `token` (the suite's fake workers).
#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn answer_for_test(stream: &mut TcpStream, token: &str) -> io::Result<()> {
    let mut framed = read_line(stream)?;
    framed.push(b'\n');
    let ProofLine::Complete(_, nonce) = parse_request(&framed) else {
        return Err(refused());
    };
    let reply = reply_line(token, &nonce, &stream.local_addr()?)?;
    stream.write_all(&reply)
}

/// Why [`connect_with_token`] returned no stream.
pub(super) enum ProofFailure {
    /// No connection, or no answer in time.
    Unreachable(io::Error),
    /// The process at the address answered without a valid proof or closed
    /// the connection (also a worker of a version before this check).
    Unproven,
}

pub(super) fn connect_with_token(
    addr: &SocketAddr,
    timeout: Duration,
    token: &str,
) -> Result<TcpStream, ProofFailure> {
    let mut stream =
        TcpStream::connect_timeout(addr, timeout).map_err(ProofFailure::Unreachable)?;
    let budget = timeout.max(Duration::from_millis(500));
    let ready = (|| {
        stream.set_read_timeout(Some(budget))?;
        stream.set_write_timeout(Some(budget))
    })();
    ready.map_err(ProofFailure::Unreachable)?;
    match prove(&mut stream, addr, token) {
        Ok(()) => {}
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ) =>
        {
            return Err(ProofFailure::Unreachable(error));
        }
        Err(_) => return Err(ProofFailure::Unproven),
    }
    let cleared = (|| {
        stream.set_read_timeout(None)?;
        stream.set_write_timeout(None)
    })();
    cleared.map_err(ProofFailure::Unreachable)?;
    Ok(stream)
}

fn prove(stream: &mut TcpStream, addr: &SocketAddr, token: &str) -> io::Result<()> {
    let mut nonce = [0u8; 32];
    getrandom::getrandom(&mut nonce).map_err(io::Error::other)?;
    let mut request = serde_json::to_vec(&ProofRequest {
        t: KIND.into(),
        nonce: hex(&nonce),
    })
    .map_err(io::Error::other)?;
    request.push(b'\n');
    stream.write_all(&request)?;
    stream.flush()?;
    let line = read_line(stream)?;
    let reply: ProofReply = serde_json::from_slice(&line).map_err(|_| refused())?;
    let tag = unhex32(&reply.proof).ok_or_else(refused)?;
    if reply.t != KIND {
        return Err(refused());
    }
    hmac::verify(&key(token), &message(&nonce, addr), &tag).map_err(|_| refused())
}

/// One line, byte by byte, so nothing after it is consumed.
fn read_line(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte)? {
            0 => return Err(refused()),
            _ if byte[0] == b'\n' => return Ok(line),
            _ if line.len() >= MAX_PROOF_LINE => return Err(refused()),
            _ => line.push(byte[0]),
        }
    }
}

fn refused() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "the process at the published IPC address could not prove it is this user's background worker",
    )
}

#[cfg(test)]
#[path = "ipc_proof_tests.rs"]
mod tests;
