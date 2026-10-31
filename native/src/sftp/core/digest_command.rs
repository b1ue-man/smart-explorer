//! `content_digest` of a plain SFTP server (no SSH agent next to the data):
//! the SSH server hashes the file itself and sends 64 hex digits, not the
//! content (`ssh host sh -c '…'`, RFC 4254 section 6.5 exec request on a
//! session channel of the connection that already carries the SFTP
//! subsystem). Only when the account may run commands and the host has
//! `sha256sum` (GNU coreutils, busybox), `shasum -a 256` (Perl, macOS) or
//! `sha256 -q` (BSD); otherwise the answer is `Ok(None)` and the caller moves
//! the content as before.
//!
//! The path reaches the remote command only as the positional parameter `$1`
//! of a fixed script, single-quoted (a quote inside the path becomes `'\''`),
//! and the file is read through a redirect, so neither the path nor its
//! first characters can be taken for an option or a command. A path with a
//! NUL, CR or LF is not sent. The SFTP attributes before and after the
//! command (size and time) must agree, or the file changed while it was
//! hashed and the answer is an error, never a digest.
//!
//! The sync's MD5 signature (`content_md5`) goes the same way with a second
//! fixed script (`MD5_SCRIPT`): `md5sum` (GNU coreutils, busybox) or
//! `md5 -q` (BSD, macOS). No other tool is tried (no `openssl`), and a host
//! with neither answers `Ok(None)`. Each algorithm has its own gate: a host
//! without `sha256sum` may still have `md5sum`.
use super::backend::SftpBackend;
use super::exec::CapturedExec;
use crate::vfs::{is_sha256_hex, ContentDigest, VfsMeta, VfsResult};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Exit status of the script: no regular file (or a link) at the path.
const EXIT_NOT_REGULAR: u32 = 3;
/// Exit status of the script: no SHA-256 tool on the host.
const EXIT_NO_TOOL: u32 = 127;
/// The shell could not run the script at all (POSIX `sh` exit status).
const EXIT_NOT_EXECUTABLE: u32 = 126;
/// Failures in a row (none of them proving that commands are unavailable)
/// after which the connection stops trying.
const FAILURES_BEFORE_GIVING_UP: u32 = 3;
/// A host that hashes slower than this is not worth waiting for: the
/// content would arrive sooner over the connection.
const SLOWEST_HASHING: u64 = 8 * 1024 * 1024;
/// Time to start the command and answer for a small file.
const BASE_WAIT: Duration = Duration::from_secs(60);

/// The fixed script; the path is `$1`. No `!` and no single quote, so no
/// login shell (sh, bash, zsh, fish, csh) reads anything into it.
const SCRIPT: &str = "f=$1; [ -f \"$f\" ] || exit 3; [ -h \"$f\" ] && exit 3; \
if command -v sha256sum >/dev/null 2>&1; then sha256sum <\"$f\"; \
elif command -v shasum >/dev/null 2>&1; then shasum -a 256 <\"$f\"; \
elif command -v sha256 >/dev/null 2>&1; then sha256 -q <\"$f\"; \
else exit 127; fi";

/// The fixed MD5 script; same rules as `SCRIPT`.
const MD5_SCRIPT: &str = "f=$1; [ -f \"$f\" ] || exit 3; [ -h \"$f\" ] && exit 3; \
if command -v md5sum >/dev/null 2>&1; then md5sum <\"$f\"; \
elif command -v md5 >/dev/null 2>&1; then md5 -q <\"$f\"; \
else exit 127; fi";

/// What the host is asked to form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Algorithm {
    Sha256,
    Md5,
}

impl Algorithm {
    fn script(self) -> &'static str {
        match self {
            Self::Sha256 => SCRIPT,
            Self::Md5 => MD5_SCRIPT,
        }
    }

    /// Whether `text` is a digest of this algorithm in lowercase hex.
    fn is_hex(self, text: &str) -> bool {
        match self {
            Self::Sha256 => is_sha256_hex(text),
            Self::Md5 => {
                text.len() == 32 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            }
        }
    }
}

/// What this connection learned about running the hash command.
#[derive(Default)]
pub(super) struct DigestGate {
    unavailable: AtomicBool,
    failures: AtomicU32,
}

impl DigestGate {
    pub(super) fn closed(&self) -> bool {
        self.unavailable.load(Ordering::Relaxed)
    }

    fn give_up(&self) {
        self.unavailable.store(true, Ordering::Relaxed);
    }

    fn failed(&self) {
        if self.failures.fetch_add(1, Ordering::Relaxed) + 1 >= FAILURES_BEFORE_GIVING_UP {
            self.give_up();
        }
    }

    fn worked(&self) {
        self.failures.store(0, Ordering::Relaxed);
    }
}

/// Single-quotes `text` for any POSIX shell.
pub(super) fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The command line for `path`: `sh -c '<script>' sh '<path>'`.
#[cfg(test)]
pub(super) fn command_for(path: &str) -> String {
    command_for_algorithm(Algorithm::Sha256, path)
}

/// The command line of `algorithm`'s script for `path`.
pub(super) fn command_for_algorithm(algorithm: Algorithm, path: &str) -> String {
    format!(
        "sh -c {} sh {}",
        sh_quote(algorithm.script()),
        sh_quote(path)
    )
}

/// Whether `path` can be sent as one shell word.
pub(super) fn sendable(path: &str) -> bool {
    !path.is_empty() && !path.contains(['\0', '\r', '\n'])
}

/// How long to wait for the hash of `size` bytes.
pub(super) fn wait_for(size: u64) -> Duration {
    BASE_WAIT + Duration::from_secs(size / SLOWEST_HASHING)
}

/// What the finished command means.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Reply {
    Digest(String),
    /// Not a regular file any more (or a link): the file changed.
    NotRegular,
    /// This connection will never answer a hash command.
    Unavailable,
    /// This time it did not work; commands may still work.
    Failed(String),
}

/// Reads the command's result: exit status, then the first token of the
/// first output line (`<hex>  -` of sha256sum/shasum, `<hex>` of sha256 -q).
#[cfg(test)]
pub(super) fn interpret(capture: &CapturedExec) -> Reply {
    interpret_for(capture, Algorithm::Sha256)
}

/// `interpret` for the digest of `algorithm`.
pub(super) fn interpret_for(capture: &CapturedExec, algorithm: Algorithm) -> Reply {
    if capture.exit_signal.is_some() {
        return Reply::Failed("the hash command was ended by a signal".into());
    }
    match capture.exit_status {
        Some(0) => {}
        Some(EXIT_NOT_REGULAR) => return Reply::NotRegular,
        Some(EXIT_NO_TOOL) | Some(EXIT_NOT_EXECUTABLE) => return Reply::Unavailable,
        Some(status) => {
            let detail = String::from_utf8_lossy(&capture.stderr);
            return Reply::Failed(format!(
                "the hash command exited with status {status}: {}",
                detail.trim()
            ));
        }
        None => return Reply::Failed("the hash command gave no exit status".into()),
    }
    if capture.stdout_truncated {
        return Reply::Unavailable;
    }
    let text = String::from_utf8_lossy(&capture.stdout);
    let token = text
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().next())
        .map(|token| token.trim_start_matches('\\').to_ascii_lowercase());
    match token {
        Some(hex) if algorithm.is_hex(&hex) => Reply::Digest(hex),
        // A shell that took the command for something else (a forced
        // command, a Windows shell) printed no digest although it succeeded.
        _ => Reply::Unavailable,
    }
}

/// The whole call, with the world behind two closures: `stat` (SFTP
/// attributes of the path) and `run` (the command line, how long to wait; the
/// connection-level result of the exec). `Ok(None)`: commands are not an
/// option here and the caller moves the content.
pub(super) fn digest_by(
    gate: &DigestGate,
    path: &str,
    stat: impl FnMut() -> VfsResult<VfsMeta>,
    run: impl FnOnce(&str, Duration) -> io::Result<Option<CapturedExec>>,
) -> VfsResult<Option<ContentDigest>> {
    Ok(hash_by(gate, Algorithm::Sha256, path, stat, run)?
        .map(|(sha256, size)| ContentDigest { sha256, size }))
}

/// `digest_by` for any `algorithm`: the lowercase hex digest and the size the
/// SFTP attributes showed.
pub(super) fn hash_by(
    gate: &DigestGate,
    algorithm: Algorithm,
    path: &str,
    mut stat: impl FnMut() -> VfsResult<VfsMeta>,
    run: impl FnOnce(&str, Duration) -> io::Result<Option<CapturedExec>>,
) -> VfsResult<Option<(String, u64)>> {
    if gate.closed() || !sendable(path) {
        return Ok(None);
    }
    let before = stat()?;
    if before.is_dir || before.is_symlink || before.special {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("no regular file: {path}"),
        ));
    }
    let captured = match run(
        &command_for_algorithm(algorithm, path),
        wait_for(before.size),
    ) {
        Ok(Some(captured)) => captured,
        // The server had no channel to spare right now.
        Ok(None) => return Ok(None),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            // The exec request itself was rejected (no shell for this
            // account, a subsystem-only login).
            gate.give_up();
            return Ok(None);
        }
        // Too slow: the content would arrive sooner than this hash.
        Err(error) if error.kind() == io::ErrorKind::TimedOut => {
            gate.failed();
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let after = stat()?;
    if after.size != before.size || after.mtime_ms != before.mtime_ms {
        return Err(changed());
    }
    match interpret_for(&captured, algorithm) {
        Reply::Digest(hex) => {
            gate.worked();
            Ok(Some((hex, before.size)))
        }
        Reply::NotRegular => Err(changed()),
        Reply::Unavailable => {
            gate.give_up();
            Ok(None)
        }
        Reply::Failed(_) => {
            gate.failed();
            Ok(None)
        }
    }
}

fn changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "file changed while its content digest was computed",
    )
}

impl SftpBackend {
    /// `BackendExtensions::content_digest` for this connection.
    pub(super) fn digest_by_command(
        &self,
        path: &str,
        cancel: &AtomicBool,
    ) -> VfsResult<Option<ContentDigest>> {
        use crate::vfs::Backend;
        let gate: &Arc<DigestGate> = &self.digest_gate;
        digest_by(
            gate,
            path,
            || self.stat(path),
            |command, wait| super::digest_exec::run(self, command, wait, cancel),
        )
    }

    /// `BackendExtensions::content_md5` for this connection.
    pub(super) fn md5_by_command(
        &self,
        path: &str,
        cancel: &AtomicBool,
    ) -> VfsResult<Option<String>> {
        use crate::vfs::Backend;
        let gate: &Arc<DigestGate> = &self.md5_gate;
        Ok(hash_by(
            gate,
            Algorithm::Md5,
            path,
            || self.stat(path),
            |command, wait| super::digest_exec::run(self, command, wait, cancel),
        )?
        .map(|(md5, _)| md5))
    }
}

#[cfg(test)]
#[path = "digest_command_tests.rs"]
mod tests;
