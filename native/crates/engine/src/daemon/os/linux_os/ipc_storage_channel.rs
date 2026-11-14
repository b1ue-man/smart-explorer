//! The worker IPC channel of a window started by a worker that follows a
//! session (E46, S13): `SE_WORKER_IPC_CHANNEL` names the loopback address of
//! the channel, and the token is the first line of the window's standard
//! input, read once and kept in memory. A process with the variable never
//! falls back to the publication or token file of its own account, so the
//! window talks only to the worker that started it. Without the variable
//! nothing changes.
use std::io::{self, BufRead, Read};
use std::net::SocketAddr;
use std::sync::OnceLock;

use crate::share::consent_launch::WORKER_IPC_CHANNEL_ENV;

/// Hex characters of a channel token (256 bits).
const TOKEN_HEX: usize = 64;
/// Longest first line read from stdin.
const MAX_TOKEN_LINE: u64 = 128;

/// `None` without a channel; otherwise its address, `Some(None)` when the
/// variable names no loopback address (also a value that is no text: the
/// variable alone decides, as for the token).
pub(in crate::daemon) fn channel_addr() -> Option<Option<SocketAddr>> {
    let value = std::env::var_os(WORKER_IPC_CHANNEL_ENV)?;
    Some(value.to_str().and_then(addr_from))
}

/// The channel token of this process; `None` without a channel.
pub(in crate::daemon) fn channel_token() -> Option<io::Result<String>> {
    std::env::var_os(WORKER_IPC_CHANNEL_ENV)?;
    static TOKEN: OnceLock<Option<String>> = OnceLock::new();
    let token = TOKEN.get_or_init(|| token_from(&mut io::stdin().lock()).ok());
    Some(token.clone().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the window got no worker IPC channel token on its standard input",
        )
    }))
}

/// A loopback address with a port.
pub(in crate::daemon) fn addr_from(value: &str) -> Option<SocketAddr> {
    value
        .trim()
        .parse::<SocketAddr>()
        .ok()
        .filter(|addr| addr.ip().is_loopback() && addr.port() != 0)
}

/// The first line of `input`: exactly 64 hex characters.
pub(in crate::daemon) fn token_from(input: &mut dyn BufRead) -> io::Result<String> {
    let mut line = String::new();
    input.take(MAX_TOKEN_LINE).read_line(&mut line)?;
    let token = line.strip_suffix('\n').unwrap_or(&line);
    if token.len() != TOKEN_HEX || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the worker IPC channel token is not 64 hex characters",
        ));
    }
    Ok(token.to_string())
}
